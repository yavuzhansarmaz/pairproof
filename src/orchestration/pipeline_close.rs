//! Question close-out and navigation for the pipeline: finishing,
//! skipping, changing approach, and switching the active question.
//! Each records the current selection quality before the live
//! approach set is cleared; the per-index cache in the pipeline
//! keeps a question's options stable across switches.

use crate::orchestration::pipeline::{PipelineError, QuestionPipeline, TestRunRecord};
use crate::orchestration::session::Orchestrator;

impl QuestionPipeline {
    /// Returns every raw test-run record stored verbatim, in run order.
    pub fn test_records(&self) -> &[TestRunRecord] {
        &self.test_records
    }

    pub(crate) fn clear_approaches(&mut self) {
        self.approaches = None;
        self.approaches_index = None;
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

    /// Finishes the active question via the orchestrator and clears the
    /// stored approaches so a stale set can never be selected again.
    pub fn finish_question(
        &mut self,
        orch: &mut Orchestrator,
        now_ms: u64,
    ) -> Result<(), PipelineError> {
        let selected = self.current_selection_quality(orch);
        orch.finish_question(now_ms)?;
        self.record_current_selection(selected);
        self.clear_approaches();
        self.restore_cached_approaches(orch);
        Ok(())
    }

    /// Switches the active question via the orchestrator and clears
    /// the live approach set (the per-index cache keeps the options
    /// stable when the candidate comes back to a question).
    pub fn navigate_question(
        &mut self,
        orch: &mut Orchestrator,
        index: usize,
        now_ms: u64,
    ) -> Result<(), PipelineError> {
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
        let selected = self.current_selection_quality(orch);
        orch.change_approach(now_ms)?;
        self.record_current_selection(selected);
        self.clear_approaches();
        Ok(())
    }
}
