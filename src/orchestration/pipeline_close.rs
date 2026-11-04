//! Question close-out, execution records, and navigation for the pipeline:
//! finishing, skipping, changing approach, running solutions, and switching
//! the active question while enforcing final submission boundaries.

use crate::core::CoreError;
use crate::orchestration::coding::SandboxIo;
use crate::orchestration::control_snapshot::SnapshotRun;
use crate::orchestration::pipeline::{PipelineError, QuestionPipeline, TestRunRecord};
use crate::orchestration::session::Orchestrator;
use crate::sandbox::{parse_test_counts, RunnabilityReport};

const NO_TESTS_HINT: &str = "no tests ran. press t to write tests.";

impl QuestionPipeline {
    /// Returns every raw test-run record stored verbatim, in run order.
    pub fn test_records(&self) -> &[TestRunRecord] {
        &self.test_records
    }

    /// Returns every solution-run record stored verbatim, in run order.
    pub fn run_records(&self) -> &[TestRunRecord] {
        &self.run_records
    }

    /// Returns the latest test record for `index`, if any.
    pub fn latest_test_record(&self, index: usize) -> Option<&TestRunRecord> {
        self.test_records.iter().rev().find(|r| r.index == index)
    }

    /// Returns the latest coding or execution summary for `index`, if any.
    pub fn latest_run(&self, index: usize) -> Option<&SnapshotRun> {
        self.latest_runs.get(&index)
    }

    /// Returns the current code revision for `index` (0 before generation).
    pub fn code_revision(&self, index: usize) -> u64 {
        self.code_revisions.get(&index).copied().unwrap_or(0)
    }

    /// Returns true when `index` has been permanently submitted.
    pub fn is_submitted(&self, index: usize) -> bool {
        self.submitted_slots.contains(&index)
    }

    pub(crate) fn record_coding_run(&mut self, index: usize, report: &RunnabilityReport) {
        let rev = *self
            .code_revisions
            .entry(index)
            .and_modify(|r| *r += 1)
            .or_insert(1);
        let seq = self.latest_runs.get(&index).map_or(1, |r| r.seq + 1);
        self.latest_runs.insert(
            index,
            SnapshotRun {
                kind: "code".to_string(),
                seq,
                code_revision: rev,
                success: report.runnable,
                exit_code: report.exit_code,
                stdout: report.stdout.clone(),
                stderr: report.stderr.clone(),
                at_ms: 0,
            },
        );
    }

    pub(crate) fn record_exec_run(&mut self, kind: &str, record: &TestRunRecord) {
        let rev = self.code_revision(record.index);
        let seq = self.latest_runs.get(&record.index).map_or(1, |r| r.seq + 1);
        self.latest_runs.insert(
            record.index,
            SnapshotRun {
                kind: kind.to_string(),
                seq,
                code_revision: rev,
                success: record.outcome.success,
                exit_code: record.outcome.exit_code,
                stdout: record.outcome.stdout.clone(),
                stderr: record.outcome.stderr.clone(),
                at_ms: record.at_ms,
            },
        );
    }

    /// Runs the solution's tests in the sandbox and compares actual vs
    /// expected. Does not emit a test-request event or advance the question.
    pub async fn run_solution(
        &mut self,
        orch: &mut Orchestrator,
        sandbox: &dyn SandboxIo,
        now_ms: u64,
    ) -> Result<TestRunRecord, PipelineError> {
        if orch.is_terminal() {
            return Err(CoreError::Terminal.into());
        }
        let index = orch.current_index().ok_or(CoreError::NoActiveQuestion)?;
        if self.submitted_slots.contains(&index) {
            return Err(CoreError::InvalidState("question already submitted".into()).into());
        }
        if !orch.is_connected() {
            return Err(CoreError::CandidateDisconnected.into());
        }
        if orch
            .session()
            .current_slot()
            .is_some_and(|s| s.re_question_pending)
        {
            return Err(CoreError::ReQuestionPending.into());
        }
        let language = orch.session().language();
        if let Ok(disk_code) = sandbox.read_solution(index, language).await {
            if !disk_code.trim().is_empty() {
                self.codes.insert(index, disk_code);
            }
        }
        let mut outcome = sandbox.run_requested_tests(index, language).await?;
        let (passed, failed) = parse_test_counts(&outcome.stdout, &outcome.stderr, outcome.success);
        if passed == 0 && failed == 0 {
            let already = outcome.stdout.to_ascii_lowercase().contains("no tests ran");
            if outcome.stdout.trim().is_empty() {
                outcome.stdout = NO_TESTS_HINT.to_string();
            } else if !already {
                outcome.stdout = format!("{}\n{NO_TESTS_HINT}", outcome.stdout.trim_end());
            }
        }
        let record = TestRunRecord {
            index,
            at_ms: now_ms,
            outcome,
        };
        self.run_records.push(record.clone());
        self.test_records.push(record.clone());
        self.record_exec_run("test", &record);
        if passed + failed > 0 {
            self.record_counted_test_result(orch, passed, failed, now_ms)?;
        }
        Ok(record)
    }

    pub(crate) fn clear_approaches(&mut self) {
        self.approaches = None;
        self.approaches_index = None;
    }

    /// Drops the cached approach set for `index` so the next generate
    /// is a fresh Meta call. Used after change-approach and after an
    /// approved re-question replacement, which keep the slot index.
    pub(crate) fn invalidate_approaches(&mut self, index: usize) {
        self.approach_sets.remove(&index);
        if self.approaches_index == Some(index) {
            self.approaches = None;
            self.approaches_index = None;
        }
    }

    /// Puts the cached approach set for the active question back on
    /// the live slot. Finish, skip, and navigate clear the live set
    /// so a stale selection cannot be reused, but a question the
    /// candidate returns to must still show its options.
    fn restore_cached_approaches(&mut self, orch: &Orchestrator) {
        let Some(index) = orch.current_index() else {
            return;
        };
        if let Some(set) = self.approach_sets.get(&index).cloned() {
            self.approaches = Some(set);
            self.approaches_index = Some(index);
        }
    }

    /// Captures the latest workspace solution from the sandbox, then
    /// permanently submits the active question via [`Self::finish_question`].
    pub async fn finish_question_with_sandbox(
        &mut self,
        orch: &mut Orchestrator,
        sandbox: &dyn SandboxIo,
        now_ms: u64,
    ) -> Result<(), PipelineError> {
        if let Some(index) = orch.current_index() {
            if !self.submitted_slots.contains(&index) {
                let language = orch.session().language();
                if let Ok(disk_code) = sandbox.read_solution(index, language).await {
                    if !disk_code.trim().is_empty() {
                        self.codes.insert(index, disk_code);
                    }
                }
            }
        }
        self.finish_question(orch, now_ms)
    }

    /// Finishes the active question via the orchestrator, permanently locks
    /// the slot, and clears stored approaches so a stale set cannot be reused.
    pub fn finish_question(
        &mut self,
        orch: &mut Orchestrator,
        now_ms: u64,
    ) -> Result<(), PipelineError> {
        let index = orch.current_index().ok_or(CoreError::NoActiveQuestion)?;
        if self.submitted_slots.contains(&index) {
            return Err(CoreError::InvalidState("question already submitted".into()).into());
        }
        let selected = self.current_selection_quality(orch);
        orch.finish_question(now_ms)?;
        self.submitted_slots.insert(index);
        self.record_current_selection(selected);
        self.clear_approaches();
        self.restore_cached_approaches(orch);
        Ok(())
    }

    /// Switches the active question via the orchestrator and clears
    /// the live approach set; permanently submitted questions are rejected.
    pub fn navigate_question(
        &mut self,
        orch: &mut Orchestrator,
        index: usize,
        now_ms: u64,
    ) -> Result<(), PipelineError> {
        if self.submitted_slots.contains(&index) {
            return Err(CoreError::InvalidState("question already submitted".into()).into());
        }
        let selected = self.current_selection_quality(orch);
        orch.navigate_to(index, now_ms)?;
        self.record_current_selection(selected);
        self.clear_approaches();
        self.restore_cached_approaches(orch);
        Ok(())
    }

    /// Skips the active question via the orchestrator and clears approaches.
    pub fn skip_question(
        &mut self,
        orch: &mut Orchestrator,
        now_ms: u64,
    ) -> Result<(), PipelineError> {
        let selected = self.current_selection_quality(orch);
        orch.skip_question(now_ms)?;
        self.record_current_selection(selected);
        self.clear_approaches();
        self.restore_cached_approaches(orch);
        Ok(())
    }

    /// Changes the selected approach via the orchestrator and clears the
    /// stored approaches; a fresh generation is required to reselect.
    pub fn change_approach(
        &mut self,
        orch: &mut Orchestrator,
        now_ms: u64,
    ) -> Result<(), PipelineError> {
        if let Some(index) = orch.current_index() {
            if self.submitted_slots.contains(&index) {
                return Err(CoreError::InvalidState("question already submitted".into()).into());
            }
        }
        let selected = self.current_selection_quality(orch);
        orch.change_approach(now_ms)?;
        self.record_current_selection(selected);
        if let Some(index) = orch.current_index() {
            self.invalidate_approaches(index);
        } else {
            self.clear_approaches();
        }
        Ok(())
    }
}
