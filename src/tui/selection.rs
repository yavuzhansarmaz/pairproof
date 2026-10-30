//! Expert question-selection and re-question controller beneath rendering.

use std::collections::HashSet;

use thiserror::Error;

use crate::core::{CoreError, QuestionRef, Session};
use crate::leetcode::selection::to_question_ref;
use crate::leetcode::{CatalogEntry, Difficulty};

/// Errors from the selection controller; transitions fail as values.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SelectionError {
    /// A selection did not contain the required number of questions.
    #[error("expected {expected} questions, got {got}")]
    WrongCount { expected: usize, got: usize },
    /// A selected question had the wrong difficulty for its screen.
    #[error("question '{slug}' has wrong difficulty")]
    WrongDifficulty { slug: String },
    /// Two selected questions shared a slug or clashed with the session.
    #[error("duplicate question slug: {0}")]
    DuplicateSlug(String),
    /// Hard selection was attempted before medium selection.
    #[error("medium questions must be selected before hard questions")]
    MediumRequired,
    /// A selected question had blank identifying fields.
    #[error("invalid question: {0}")]
    InvalidQuestion(String),
    /// A core session operation failed.
    #[error(transparent)]
    Core(#[from] CoreError),
}

/// Filters a question-ref pool to one difficulty, preserving order.
pub fn difficulty_filter(pool: &[QuestionRef], difficulty: Difficulty) -> Vec<QuestionRef> {
    pool.iter()
        .filter(|q| q.difficulty == difficulty)
        .cloned()
        .collect()
}

/// Converts catalog entries to question refs, dropping paid or unknown entries.
pub fn pool_to_refs(entries: &[CatalogEntry]) -> Vec<QuestionRef> {
    entries.iter().filter_map(to_question_ref).collect()
}

fn validate_selection(
    chosen: &[QuestionRef],
    expected: Difficulty,
    existing: &HashSet<String>,
) -> Result<(), SelectionError> {
    let mut seen: HashSet<&str> = HashSet::new();
    for question in chosen {
        if question.title_slug.trim().is_empty() || question.title.trim().is_empty() {
            return Err(SelectionError::InvalidQuestion(question.title_slug.clone()));
        }
        if question.difficulty != expected {
            return Err(SelectionError::WrongDifficulty {
                slug: question.title_slug.clone(),
            });
        }
        if !seen.insert(question.title_slug.as_str()) || existing.contains(&question.title_slug) {
            return Err(SelectionError::DuplicateSlug(question.title_slug.clone()));
        }
    }
    Ok(())
}

/// Expert selection controller owning a session and pending medium picks.
#[derive(Debug)]
pub struct SelectionController {
    session: Session,
    topics: Vec<String>,
    pending_medium: Option<Vec<QuestionRef>>,
}

impl SelectionController {
    /// Creates a controller for `session` with the proposal topics.
    pub fn new(session: Session, topics: Vec<String>) -> Self {
        Self {
            session,
            topics,
            pending_medium: None,
        }
    }

    /// Returns the owned session.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Returns the owned session mutably for lifecycle driving.
    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    /// Returns the proposal topics for pool fetching.
    pub fn topics(&self) -> &[String] {
        &self.topics
    }

    /// Returns the pending medium picks, if the medium screen is done.
    pub fn pending_medium(&self) -> Option<&[QuestionRef]> {
        self.pending_medium.as_deref()
    }

    /// Returns true once three medium questions are pending.
    pub fn has_pending_medium(&self) -> bool {
        self.pending_medium.is_some()
    }

    /// Captures three medium questions pending the hard screen.
    pub fn select_medium(
        &mut self,
        chosen: Vec<QuestionRef>,
        _at_ms: u64,
    ) -> Result<(), SelectionError> {
        if chosen.len() != 3 {
            return Err(SelectionError::WrongCount {
                expected: 3,
                got: chosen.len(),
            });
        }
        let existing: HashSet<String> = self
            .session
            .slots()
            .iter()
            .map(|slot| slot.question.title_slug.clone())
            .collect();
        validate_selection(&chosen, Difficulty::Medium, &existing)?;
        self.pending_medium = Some(chosen);
        Ok(())
    }

    /// Validates two hard questions and commits the expert set to core.
    pub fn select_hard(
        &mut self,
        chosen: Vec<QuestionRef>,
        at_ms: u64,
    ) -> Result<(), SelectionError> {
        let medium = self
            .pending_medium
            .clone()
            .ok_or(SelectionError::MediumRequired)?;
        if chosen.len() != 2 {
            return Err(SelectionError::WrongCount {
                expected: 2,
                got: chosen.len(),
            });
        }
        let mut existing: HashSet<String> = self
            .session
            .slots()
            .iter()
            .map(|slot| slot.question.title_slug.clone())
            .collect();
        for question in &medium {
            existing.insert(question.title_slug.clone());
        }
        validate_selection(&chosen, Difficulty::Hard, &existing)?;
        self.session.set_expert_questions(medium, chosen, at_ms)?;
        self.pending_medium = None;
        Ok(())
    }

    /// Convenience two-screen submit: medium first, then hard.
    pub fn submit(
        &mut self,
        medium: Vec<QuestionRef>,
        hard: Vec<QuestionRef>,
        at_ms: u64,
    ) -> Result<(), SelectionError> {
        self.select_medium(medium, at_ms)?;
        self.select_hard(hard, at_ms)
    }

    /// Returns the current slot index when a re-question is pending.
    pub fn pending_requestion(&self) -> Option<usize> {
        self.session
            .current_slot()
            .filter(|slot| slot.re_question_pending)
            .map(|slot| slot.index)
    }

    /// Approves a pending re-question with a same-difficulty replacement.
    pub fn approve_requestion(
        &mut self,
        replacement: QuestionRef,
        at_ms: u64,
    ) -> Result<(), SelectionError> {
        self.session
            .resolve_requestion(true, Some(replacement), at_ms)?;
        Ok(())
    }

    /// Rejects a pending re-question, burning that question's right.
    pub fn reject_requestion(&mut self, at_ms: u64) -> Result<(), SelectionError> {
        self.session.resolve_requestion(false, None, at_ms)?;
        Ok(())
    }

    /// Adds the single expert extra question via core.
    pub fn add_extra(&mut self, question: QuestionRef, at_ms: u64) -> Result<(), SelectionError> {
        self.session.add_extra_question(question, at_ms)?;
        Ok(())
    }
}
