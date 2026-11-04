//! Question navigation for a live session: switching the active
//! question among unsubmitted slots while keeping finished slots locked.

use crate::core::events::EventKind;
use crate::core::session::{Session, R};
use crate::core::types::QuestionStatus::{Active, Finished, Pending};
use crate::core::types::SessionStatus;
use crate::core::types::{ensure, CoreError};

impl Session {
    /// Switches the active question to `target` if it has not been finished.
    /// Finished questions are permanently submitted and cannot be reopened.
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
        ensure(
            self.slots[tp].status != Finished,
            self.bad_state("navigate_to finished question"),
        )?;
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
