//! Coding loop: one core action, one coding step, workspace write, and a
//! runnability check per candidate action. `t` writes tests; `r` runs them.
//! Transports run on the real session, so the Phase 4 layer books usage
//! and logs session events on it exactly once.

use std::time::Duration;

use crate::connection::EphemeralUsername;
use crate::core::{ApproachChoice, CoreError, Language, Session};
use crate::llm::coding::{run_coding_step, CodingStepInput, CodingStepOutput};
use crate::llm::MetaClient;
use crate::orchestration::pipeline::{
    ApproachGenerator, PipelineError, PipelineFuture, QuestionPipeline,
};
use crate::orchestration::session::Orchestrator;
use crate::sandbox::validate::question_layout;
use crate::sandbox::{
    check_runnability, read_file, run_tests, write_file, RunnabilityReport, SandboxHandle,
    TestOutcome, WorkspacePath, RUN_TIMEOUT,
};

const SANDBOX_TIMEOUT: Duration = Duration::from_secs(30);
const WRITE_TESTS_INSTRUCTION: &str = concat!(
    "generate unit tests for the question examples and edge cases. ",
    "Print one line per case as `Test N: PASSED` or `Test N: FAILED`, ",
    "and finish with exactly: TESTS: <passed> passed, <failed> failed"
);

/// Outcome of one coding step plus its runnability check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingStepOutcome {
    pub code: String,
    pub ready: bool,
    pub summary: String,
    pub runnable: bool,
    pub runnability: RunnabilityReport,
}

/// Narrow boundary for one Phase 4 coding step. The production
/// implementation books API usage on the session before parsing, exactly
/// once; callers must never add those tokens again.
pub trait CodingTransport: Send + Sync {
    /// Runs one coding step against the session's active question.
    fn coding_step<'a>(
        &'a self,
        session: &'a mut Session,
        input: &'a CodingStepInput,
    ) -> PipelineFuture<'a, CodingStepOutput>;
}

/// Production coding transport wrapping the real Phase 4 coding step.
#[derive(Debug)]
pub struct MetaCodingTransport {
    client: MetaClient,
}

impl MetaCodingTransport {
    /// Creates a transport wrapping `client`.
    pub fn new(client: MetaClient) -> Self {
        Self { client }
    }
}

impl CodingTransport for MetaCodingTransport {
    fn coding_step<'a>(
        &'a self,
        session: &'a mut Session,
        input: &'a CodingStepInput,
    ) -> PipelineFuture<'a, CodingStepOutput> {
        Box::pin(async move {
            let outcome = run_coding_step(&self.client, session, input)
                .await
                .map_err(|e| PipelineError::Llm(e.to_string()))?;
            Ok(outcome.value)
        })
    }
}

/// Narrow sandbox boundary over solution files in the workspace.
pub trait SandboxIo: Send + Sync {
    /// Writes solution code as workspace file data, never as shell input.
    fn write_solution<'a>(
        &'a self,
        index: usize,
        language: Language,
        code: &'a str,
    ) -> PipelineFuture<'a, ()>;
    /// Reads back the stored solution code for a question.
    fn read_solution<'a>(&'a self, index: usize, language: Language) -> PipelineFuture<'a, String>;
    /// Checks runnability only; never runs candidate-requested tests.
    fn check_runnable<'a>(
        &'a self,
        index: usize,
        language: Language,
    ) -> PipelineFuture<'a, RunnabilityReport>;
    /// Runs the solution file and returns the raw outcome verbatim.
    fn run_requested_tests<'a>(
        &'a self,
        index: usize,
        language: Language,
    ) -> PipelineFuture<'a, TestOutcome>;
}

/// True when a redirect asks for tests, so the coding step may
/// include them. A plain solution step never does.
pub(crate) fn instruction_wants_tests(instruction: &str) -> bool {
    instruction
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| word.eq_ignore_ascii_case("test") || word.eq_ignore_ascii_case("tests"))
}

fn sandbox_err(err: impl std::fmt::Display) -> PipelineError {
    PipelineError::Sandbox(err.to_string())
}

fn solution_path(index: usize, language: Language) -> Result<WorkspacePath, PipelineError> {
    question_layout(index, language)
        .map(|(_, solution)| solution)
        .map_err(sandbox_err)
}

/// Production sandbox I/O wrapping the real Docker workspace functions.
#[derive(Debug)]
pub struct DockerSandboxIo {
    handle: SandboxHandle,
    user: EphemeralUsername,
}

impl DockerSandboxIo {
    /// Creates sandbox I/O for one session container and ephemeral user.
    pub fn new(handle: SandboxHandle, user: EphemeralUsername) -> Self {
        Self { handle, user }
    }
}

impl SandboxIo for DockerSandboxIo {
    fn write_solution<'a>(
        &'a self,
        index: usize,
        language: Language,
        code: &'a str,
    ) -> PipelineFuture<'a, ()> {
        Box::pin(async move {
            let path = solution_path(index, language)?;
            write_file(&self.handle, &self.user, &path, code.as_bytes())
                .await
                .map_err(sandbox_err)
        })
    }

    fn read_solution<'a>(&'a self, index: usize, language: Language) -> PipelineFuture<'a, String> {
        Box::pin(async move {
            let path = solution_path(index, language)?;
            let bytes = read_file(&self.handle, &self.user, &path)
                .await
                .map_err(sandbox_err)?;
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        })
    }

    fn check_runnable<'a>(
        &'a self,
        index: usize,
        language: Language,
    ) -> PipelineFuture<'a, RunnabilityReport> {
        Box::pin(async move {
            let path = solution_path(index, language)?;
            check_runnability(&self.handle, &self.user, &path, language, SANDBOX_TIMEOUT)
                .await
                .map_err(sandbox_err)
        })
    }

    fn run_requested_tests<'a>(
        &'a self,
        index: usize,
        language: Language,
    ) -> PipelineFuture<'a, TestOutcome> {
        Box::pin(async move {
            let path = solution_path(index, language)?;
            run_tests(&self.handle, &self.user, &path, language, RUN_TIMEOUT)
                .await
                .map_err(sandbox_err)
        })
    }
}

impl QuestionPipeline {
    /// Selects a preset approach with its hidden locked tokens, then runs
    /// exactly one coding step, writes the code, and checks runnability.
    pub async fn select_preset(
        &mut self,
        orch: &mut Orchestrator,
        preset: u8,
        coding: &dyn CodingTransport,
        sandbox: &dyn SandboxIo,
        now_ms: u64,
    ) -> Result<CodingStepOutcome, PipelineError> {
        if !self.has_current_approaches(orch) {
            return Err(PipelineError::NoApproaches);
        }
        let expected = self
            .approaches
            .as_ref()
            .and_then(|set| set.options.get(preset as usize))
            .map(|option| option.expected_tokens)
            .ok_or(PipelineError::InvalidPreset)?;
        // Submitting over an existing selection is a change of
        // approach: free the slot first so the new choice can lock.
        if orch
            .session()
            .current_slot()
            .is_some_and(|slot| slot.approach.is_some())
        {
            orch.change_approach(now_ms)?;
        }
        orch.select_approach(ApproachChoice::Preset(preset), expected, now_ms)?;
        self.run_one_coding_step(orch, coding, sandbox, None, false)
            .await
    }

    /// Grades a candidate `Other` approach first; a grading failure makes
    /// no core selection. On success selects it and runs one coding step.
    pub async fn select_other(
        &mut self,
        orch: &mut Orchestrator,
        text: &str,
        generator: &dyn ApproachGenerator,
        coding: &dyn CodingTransport,
        sandbox: &dyn SandboxIo,
        now_ms: u64,
    ) -> Result<CodingStepOutcome, PipelineError> {
        let statement = self.statement_text(orch)?;
        let grade = generator.grade_other(&statement, text).await?;
        let index = orch.current_index();
        if orch
            .session()
            .current_slot()
            .is_some_and(|slot| slot.approach.is_some())
        {
            orch.change_approach(now_ms)?;
        }
        orch.select_approach(
            ApproachChoice::Other(text.to_string()),
            grade.expected_tokens,
            now_ms,
        )?;
        if let Some(index) = index {
            self.other_grades.insert(index, grade);
        }
        self.run_one_coding_step(orch, coding, sandbox, None, false)
            .await
    }

    /// Records a continue action exactly once, then runs one coding step.
    pub async fn continue_coding(
        &mut self,
        orch: &mut Orchestrator,
        coding: &dyn CodingTransport,
        sandbox: &dyn SandboxIo,
        now_ms: u64,
    ) -> Result<CodingStepOutcome, PipelineError> {
        orch.continue_work(now_ms)?;
        self.run_one_coding_step(orch, coding, sandbox, None, false)
            .await
    }

    /// Records a redirect exactly once, then runs one coding step with
    /// the redirect instruction.
    pub async fn redirect_coding(
        &mut self,
        orch: &mut Orchestrator,
        instruction: &str,
        coding: &dyn CodingTransport,
        sandbox: &dyn SandboxIo,
        now_ms: u64,
    ) -> Result<CodingStepOutcome, PipelineError> {
        orch.redirect(instruction, now_ms)?;
        self.run_one_coding_step(
            orch,
            coding,
            sandbox,
            Some(instruction.to_string()),
            instruction_wants_tests(instruction),
        )
        .await
    }

    /// Records a test request (extra score), then asks the coding agent
    /// to write tests into the solution file. Does not execute them.
    pub async fn request_tests(
        &mut self,
        orch: &mut Orchestrator,
        coding: &dyn CodingTransport,
        sandbox: &dyn SandboxIo,
        now_ms: u64,
    ) -> Result<CodingStepOutcome, PipelineError> {
        let index = orch.current_index().ok_or(CoreError::NoActiveQuestion)?;
        if self.submitted_slots.contains(&index) {
            return Err(CoreError::InvalidState("question already submitted".into()).into());
        }
        orch.request_tests(now_ms)?;
        self.run_one_coding_step(
            orch,
            coding,
            sandbox,
            Some(WRITE_TESTS_INSTRUCTION.to_string()),
            true,
        )
        .await
    }

    pub(crate) async fn refresh_code_from_sandbox(
        &mut self,
        index: usize,
        language: Language,
        sandbox: &dyn SandboxIo,
    ) {
        if let Ok(disk_code) = sandbox.read_solution(index, language).await {
            if !disk_code.trim().is_empty() {
                self.codes.insert(index, disk_code);
            }
        }
    }

    fn approach_text(&self, orch: &Orchestrator) -> String {
        let Some(slot) = orch.session().current_slot() else {
            return String::new();
        };
        match &slot.approach {
            Some(ApproachChoice::Preset(n)) => self
                .approaches
                .as_ref()
                .and_then(|set| set.options.get(*n as usize))
                .map(|o| format!("{}: {}", o.title, o.summary))
                .unwrap_or_else(|| format!("preset option {n}")),
            Some(ApproachChoice::Other(text)) => text.clone(),
            None => String::new(),
        }
    }

    async fn run_one_coding_step(
        &mut self,
        orch: &mut Orchestrator,
        coding: &dyn CodingTransport,
        sandbox: &dyn SandboxIo,
        instruction: Option<String>,
        tests_requested: bool,
    ) -> Result<CodingStepOutcome, PipelineError> {
        if orch.is_terminal() {
            return Err(CoreError::Terminal.into());
        }
        let index = orch.current_index().ok_or(CoreError::NoActiveQuestion)?;
        if self.submitted_slots.contains(&index) {
            return Err(CoreError::InvalidState("question already submitted".into()).into());
        }
        let language = orch.session().language();
        self.refresh_code_from_sandbox(index, language, sandbox)
            .await;
        if tests_requested {
            self.tests_generated.insert(index);
        }
        let current_code = self
            .codes
            .get(&index)
            .cloned()
            .or_else(|| self.current_statement(orch).map(|f| f.starter_code))
            .unwrap_or_default();
        let input = CodingStepInput {
            question_statement: self.statement_text(orch)?,
            approach_text: self.approach_text(orch),
            current_code,
            instruction,
            tests_requested: tests_requested || self.tests_generated.contains(&index),
            language,
        };
        let result = coding.coding_step(orch.session_mut(), &input).await;
        if orch.is_terminal() {
            return Err(CoreError::Terminal.into());
        }
        if orch.current_index() != Some(index) || self.submitted_slots.contains(&index) {
            return Err(
                CoreError::InvalidState("session changed during coding step".to_string()).into(),
            );
        }
        let output = result?;
        sandbox
            .write_solution(index, language, &output.code)
            .await?;
        let report = sandbox.check_runnable(index, language).await?;
        self.codes.insert(index, output.code.clone());
        self.record_coding_run(index, &report);
        Ok(CodingStepOutcome {
            runnable: report.runnable,
            code: output.code,
            ready: output.ready,
            summary: output.summary,
            runnability: report,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::instruction_wants_tests;

    #[test]
    fn a_redirect_that_asks_for_tests_is_allowed_to_write_them() {
        assert!(instruction_wants_tests("write tests and run them"));
        assert!(instruction_wants_tests("Add a Test for the empty list."));
        assert!(!instruction_wants_tests("rename the helper"));
        assert!(!instruction_wants_tests("latest edge case"));
    }
}
