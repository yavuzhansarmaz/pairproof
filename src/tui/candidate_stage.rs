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

/// Staging for every question visited this session, by slot index.
#[derive(Debug, Clone, Default)]
pub struct ApproachStaging {
    stages: BTreeMap<usize, QuestionStage>,
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

    /// Stage of the current question, if one is being staged.
    pub fn current_stage(&self) -> Option<&QuestionStage> {
        self.current.and_then(|index| self.stages.get(&index))
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
