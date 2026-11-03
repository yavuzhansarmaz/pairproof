//! Free question navigation for a live session: switching the
//! active question in either direction, reopening finished or
//! skipped questions with their recorded work intact.

use crate::core::events::EventKind;
use crate::core::session::{Session, R};
use crate::core::types::QuestionStatus::{Active, Pending};
use crate::core::types::SessionStatus;
use crate::core::types::{ensure, CoreError};

impl Session {
    /// Switches the active question to the slot with `target` index.
    /// The question left behind returns to Pending with its approach,
    /// tokens, and code progress untouched; a finished or skipped
    /// target reopens (its finish time is cleared, so it is graded
    /// only if finished again). Switching is free in both directions.
    pub fn navigate_to(&mut self, target: usize, at_ms: u64) -> R {
        self.ensure_not_terminal()?;
        self.ensure_connected()?;
        ensure(
            matches!(
                self.status,
                SessionStatus::InProgress
                    | SessionStatus::Scoring
                    | SessionStatus::WaitingForExpert
            ),
            self.bad_state("navigate_to"),
        )?;
        let Some(tp) = self.slots.iter().position(|s| s.index == target) else {
            return Err(CoreError::NoActiveQuestion);
        };
        if self.active_pos() == Some(tp) {
            return Ok(());
        }
        if let Some(ap) = self.active_pos() {
            ensure(
                !self.slots[ap].re_question_pending,
                CoreError::ReQuestionPending,
            )?;
            self.slots[ap].status = Pending;
        }
        let first_start = self.slots[tp].started_at_ms.is_none();
        self.slots[tp].status = Active;
        self.slots[tp].finished_at_ms = None;
        if first_start {
            self.slots[tp].started_at_ms = Some(at_ms);
        }
        self.status = SessionStatus::InProgress;
        self.push(
            at_ms,
            if first_start {
                EventKind::QuestionStarted { index: target }
            } else {
                EventKind::QuestionReopened { index: target }
            },
        );
        Ok(())
    }
}
