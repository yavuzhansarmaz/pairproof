//! Per-question staged approach selection for the candidate
//! solving screen: the cursor over the approach list and the choice
//! marked with Space, held locally until Enter submits it. Staging
//! is kept per question index, so switching questions with the
//! arrow keys never loses a marked choice or a typed `Other` text.

use std::collections::BTreeMap;

use crate::orchestration::control::SnapshotSelection;

/// A choice marked with Space but not yet submitted with Enter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StagedChoice {
    Preset(u8),
    Other,
}

/// An approach Enter already sent, still waiting for the snapshot
/// to record it. The status box shows this immediately, because the
/// expert only publishes the selection after the coding step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingApproach {
    pub index: usize,
    pub selection: SnapshotSelection,
}

impl PendingApproach {
    /// Same wording as the status box once the snapshot lands.
    pub fn label(&self) -> String {
        match &self.selection {
            SnapshotSelection::Preset(n) => format!("preset {}", n + 1),
            SnapshotSelection::Other(text) if text.is_empty() => "other (blank)".to_string(),
            SnapshotSelection::Other(text) => format!("other: {text}"),
        }
    }
}

/// Staged state for one question: cursor row (0..=option count,
/// where the row at `option count` is `Other`), the marked choice,
/// and the `Other` text typed for this question.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QuestionStage {
    pub cursor: usize,
    pub choice: Option<StagedChoice>,
    pub other_text: String,
}

impl QuestionStage {
    /// Initial stage derived from the server-recorded selection, so
    /// a question the candidate already submitted shows its choice.
    pub fn from_selected(selected: &Option<SnapshotSelection>, option_count: usize) -> Self {
        match selected {
            Some(SnapshotSelection::Preset(n)) => Self {
                cursor: (*n as usize).min(option_count),
                choice: Some(StagedChoice::Preset(*n)),
                other_text: String::new(),
            },
            Some(SnapshotSelection::Other(text)) => Self {
                cursor: option_count,
                choice: Some(StagedChoice::Other),
                other_text: text.clone(),
            },
            None => Self::default(),
        }
    }
}

/// Explicit lifecycle state of one question on the candidate side.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum QuestionFlowState {
    #[default]
    AwaitingApproach,
    GeneratingInitialSolution {
        selection: SnapshotSelection,
        base_revision: u64,
    },
    SyncingWorkspace {
        selection: SnapshotSelection,
        target_revision: u64,
    },
    SolutionReady,
    ContinuingInteraction,
    UpdatingSolution {
        instruction: Option<String>,
        base_revision: u64,
    },
    UpdatingTests {
        instruction: String,
        base_revision: u64,
    },
    RunningSolution {
        base_seq: u64,
    },
    RunningTests {
        base_seq: u64,
    },
    Submitting,
    Submitted,
}

impl QuestionFlowState {
    /// True while an async coding, sync, execution, or submit step is running.
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            Self::GeneratingInitialSolution { .. }
                | Self::SyncingWorkspace { .. }
                | Self::UpdatingSolution { .. }
                | Self::UpdatingTests { .. }
                | Self::RunningSolution { .. }
                | Self::RunningTests { .. }
                | Self::Submitting
        )
    }

    /// True while the question is submitting or already permanently submitted.
    pub fn is_submitted_or_submitting(&self) -> bool {
        matches!(self, Self::Submitting | Self::Submitted)
    }

    /// Candidate-safe phase description for the status box, if active.
    pub fn status_phase_label(&self) -> Option<&'static str> {
        match self {
            Self::AwaitingApproach => None,
            Self::GeneratingInitialSolution { .. } => Some("generating initial solution"),
            Self::SyncingWorkspace { .. } => Some("syncing solution to workspace"),
            Self::SolutionReady => Some("solution ready"),
            Self::ContinuingInteraction => Some("continuing with agent"),
            Self::UpdatingSolution { .. } => Some("updating solution"),
            Self::UpdatingTests { .. } => Some("updating tests"),
            Self::RunningSolution { .. } => Some("running solution"),
            Self::RunningTests { .. } => Some("running tests"),
            Self::Submitting => Some("submitting question"),
            Self::Submitted => Some("submitted"),
        }
    }

    /// Pinned banner line for the question pane while work is in progress.
    pub fn banner_text(&self, index: usize) -> Option<String> {
        let n = index + 1;
        match self {
            Self::GeneratingInitialSolution { selection, .. } => {
                let pending = PendingApproach {
                    index,
                    selection: selection.clone(),
                };
                Some(format!(
                    "working: writing {}. this takes a moment",
                    pending.label()
                ))
            }
            Self::SyncingWorkspace { .. } => Some(format!(
                "working: syncing q{n}/solution to workspace. this takes a moment"
            )),
            Self::ContinuingInteraction => Some(
                "ready: type instructions to update code or generate/update tests, then press enter."
                    .to_string(),
            ),
            Self::UpdatingSolution { .. } => Some(format!(
                "working: updating solution in q{n}/. this takes a moment"
            )),
            Self::UpdatingTests { .. } => Some(format!(
                "working: generating/updating tests in q{n}/. this takes a moment"
            )),
            Self::RunningSolution { .. } => Some(format!("working: running solution in q{n}/")),
            Self::RunningTests { .. } => Some(format!("working: running tests in q{n}/")),
            Self::Submitting => Some(format!("working: submitting q{n}/")),
            Self::Submitted => Some(format!("ready: q{n}/ submitted.")),
            _ => None,
        }
    }
}

/// Staging for every question visited this session, by slot index.
#[derive(Debug, Clone, Default)]
pub struct ApproachStaging {
    stages: BTreeMap<usize, QuestionStage>,
    flows: BTreeMap<usize, QuestionFlowState>,
    current: Option<usize>,
}

impl ApproachStaging {
    /// Makes `index` the current question, restoring its saved stage
    /// or deriving a fresh one from the server selection. The
    /// outgoing question's stage (including its `Other` text) is
    /// saved first, so nothing typed or marked is lost on a switch.
    pub fn switch_to(
        &mut self,
        index: usize,
        selected: &Option<SnapshotSelection>,
        option_count: usize,
        outgoing_other_text: &str,
    ) -> QuestionStage {
        if let Some(previous) = self.current {
            if previous != index {
                if let Some(stage) = self.stages.get_mut(&previous) {
                    stage.other_text = outgoing_other_text.to_string();
                }
            }
        }
        self.current = Some(index);
        self.stages
            .entry(index)
            .or_insert_with(|| QuestionStage::from_selected(selected, option_count))
            .clone()
    }

    /// Saves the `Other` text of the current question.
    pub fn save_other_text(&mut self, text: &str) {
        if let Some(stage) = self.current_stage_mut() {
            stage.other_text = text.to_string();
        }
    }

    /// Index of the question currently on screen, if one is staged.
    pub fn current_index(&self) -> Option<usize> {
        self.current
    }

    /// Stage of the current question, if one is being staged.
    pub fn current_stage(&self) -> Option<&QuestionStage> {
        self.current.and_then(|index| self.stages.get(&index))
    }

    /// Lifecycle flow state of `index`.
    pub fn flow_for(&self, index: usize) -> QuestionFlowState {
        self.flows.get(&index).cloned().unwrap_or_default()
    }

    /// Lifecycle flow state of the current question on screen.
    pub fn current_flow(&self) -> QuestionFlowState {
        self.current
            .map(|index| self.flow_for(index))
            .unwrap_or_default()
    }

    /// Updates the lifecycle flow state for `index`.
    pub fn set_flow(&mut self, index: usize, flow: QuestionFlowState) {
        self.flows.insert(index, flow);
    }

    /// True when `index` is submitting or permanently submitted.
    pub fn is_submitted(&self, index: usize) -> bool {
        self.flow_for(index).is_submitted_or_submitting()
    }

    /// Moves the cursor by `delta` rows, clamped to the option rows
    /// plus the `Other` row at `option_count`.
    pub fn move_cursor(&mut self, delta: i32, option_count: usize) {
        if let Some(stage) = self.current_stage_mut() {
            let next = stage.cursor as i32 + delta;
            stage.cursor = next.clamp(0, option_count as i32) as usize;
        }
    }

    /// Puts the cursor on the `Other` row.
    pub fn cursor_to_other(&mut self, option_count: usize) {
        if let Some(stage) = self.current_stage_mut() {
            stage.cursor = option_count;
        }
    }

    /// Marks the row under the cursor with Space: a preset row marks
    /// that preset, the last row marks `Other`.
    pub fn mark_cursor(&mut self, option_count: usize) {
        if let Some(stage) = self.current_stage_mut() {
            stage.choice = if stage.cursor < option_count {
                Some(StagedChoice::Preset(stage.cursor as u8))
            } else {
                Some(StagedChoice::Other)
            };
        }
    }

    fn current_stage_mut(&mut self) -> Option<&mut QuestionStage> {
        let index = self.current?;
        self.stages.get_mut(&index)
    }
}

/// Settles pending question lifecycle states against the latest snapshot view and sync status.
pub(crate) fn settle_with_sync(
    state: &mut crate::tui::candidate_run::CandidateRunState,
    view: &crate::tui::candidate_runtime::CandidateView,
    solution_synced: bool,
    sync_failed: bool,
) {
    for nav in &view.nav {
        if nav.state == "Finished" {
            state
                .staging
                .set_flow(nav.index, QuestionFlowState::Submitted);
        }
    }
    if let Some(current_idx) = state.staging.current_index() {
        if state.staging.flow_for(current_idx) == QuestionFlowState::Submitting
            && view.question_index != Some(current_idx)
        {
            state
                .staging
                .set_flow(current_idx, QuestionFlowState::Submitted);
        }
    }
    let Some(index) = view.question_index else {
        return;
    };
    let n = index + 1;
    let rev = view.code_revision();
    let run_seq = view.run_seq();
    let ready_msg = format!("code is in q{n}/. c continue, r run, t run tests, or f submit.");
    let flow = state.staging.flow_for(index);
    let was_updating_tests = matches!(flow, QuestionFlowState::UpdatingTests { .. });
    match flow {
        QuestionFlowState::ContinuingInteraction
            if state.focus != crate::tui::candidate_run::CandidateFocus::Redirect =>
        {
            state
                .staging
                .set_flow(index, QuestionFlowState::SolutionReady);
        }
        QuestionFlowState::GeneratingInitialSolution {
            selection,
            base_revision,
        } => {
            if view.selected.as_ref() == Some(&selection) && (rev > base_revision || rev == 0) {
                if sync_failed || !solution_synced {
                    state.staging.set_flow(
                        index,
                        QuestionFlowState::SyncingWorkspace {
                            selection,
                            target_revision: rev,
                        },
                    );
                } else {
                    state.pending_approach = None;
                    state
                        .staging
                        .set_flow(index, QuestionFlowState::SolutionReady);
                    state.after_code = Some((index, ready_msg.clone()));
                }
            }
        }
        QuestionFlowState::SyncingWorkspace {
            target_revision, ..
        } => {
            if !sync_failed && solution_synced && rev >= target_revision {
                state.pending_approach = None;
                state
                    .staging
                    .set_flow(index, QuestionFlowState::SolutionReady);
                state.after_code = Some((index, ready_msg.clone()));
            }
        }
        QuestionFlowState::UpdatingSolution { base_revision, .. }
        | QuestionFlowState::UpdatingTests { base_revision, .. } => {
            if rev > base_revision {
                if sync_failed || !solution_synced {
                    if let Some(selection) = view.selected.clone() {
                        state.staging.set_flow(
                            index,
                            QuestionFlowState::SyncingWorkspace {
                                selection,
                                target_revision: rev,
                            },
                        );
                    }
                } else {
                    state
                        .staging
                        .set_flow(index, QuestionFlowState::SolutionReady);
                    let target = if was_updating_tests {
                        "tests"
                    } else {
                        "solution"
                    };
                    state.after_code = Some((
                        index,
                        format!("updated {target} in q{n}/. c continue, r run, t run tests, or f submit."),
                    ));
                }
            }
        }
        QuestionFlowState::RunningSolution { base_seq } => {
            if run_seq > base_seq {
                state
                    .staging
                    .set_flow(index, QuestionFlowState::SolutionReady);
                let label = match view.latest_run() {
                    Some(r) if r.success => "passed",
                    Some(_) => "failed",
                    None => "completed",
                };
                state.after_code = Some((
                    index,
                    format!("run {label} in q{n}/. c continue, r run, t run tests, or f submit."),
                ));
            }
        }
        QuestionFlowState::RunningTests { base_seq } => {
            if run_seq > base_seq || (base_seq == 0 && view.latest_test.is_some()) {
                state
                    .staging
                    .set_flow(index, QuestionFlowState::SolutionReady);
                let label = match view.latest_run() {
                    Some(r) if r.success => "passed",
                    Some(_) => "failed",
                    None => match view.latest_test {
                        Some(t) if t.failed == 0 => "passed",
                        Some(_) => "failed",
                        None => "completed",
                    },
                };
                state.after_code = Some((
                    index,
                    format!("tests {label} in q{n}/. c continue, r run, t run tests, or f submit."),
                ));
            }
        }
        QuestionFlowState::AwaitingApproach
            if view.selected.is_some()
                && state.pending_approach.is_none()
                && !sync_failed
                && solution_synced =>
        {
            state
                .staging
                .set_flow(index, QuestionFlowState::SolutionReady);
            if state.after_code.is_none() {
                state.after_code = Some((index, ready_msg.clone()));
            }
        }
        _ => {}
    }
    let landed = state.pending_approach.as_ref().is_some_and(|pending| {
        view.question_index == Some(pending.index)
            && view.selected.as_ref() == Some(&pending.selection)
    });
    if landed && !sync_failed && solution_synced {
        if let Some(pending) = state.pending_approach.take() {
            let n = pending.index + 1;
            state
                .staging
                .set_flow(pending.index, QuestionFlowState::SolutionReady);
            state.after_code = Some((
                pending.index,
                format!("code is in q{n}/. c continue, r run, t run tests, or f submit."),
            ));
        }
    }
}
