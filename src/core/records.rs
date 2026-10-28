use crate::core::events::EventKind;
use crate::core::session::{Session, R};
use crate::core::types::{ensure, CoreError, TestSummary};

impl Session {
    /// Records a completed test run on the active question.
    pub fn record_test_result(&mut self, passed: u32, failed: u32, at_ms: u64) -> R {
        let p = self.active()?;
        let index = self.slots[p].index;
        self.slots[p].latest_test = Some(TestSummary {
            passed,
            failed,
            at_ms,
        });
        self.push(
            at_ms,
            EventKind::TestRunCompleted {
                index,
                passed,
                failed,
            },
        );
        Ok(())
    }

    /// Records a bulk-insert paste flag on the active question.
    pub fn record_paste_flag(&mut self, inserted_bytes: u64, at_ms: u64) -> R {
        let p = self.active()?;
        ensure(inserted_bytes > 0, CoreError::InvalidPasteFlag)?;
        let index = self.slots[p].index;
        self.slots[p].paste_flags = self.slots[p].paste_flags.saturating_add(1);
        self.push(
            at_ms,
            EventKind::PasteFlagged {
                index,
                inserted_bytes,
            },
        );
        Ok(())
    }

    /// Logs a chat boundary violation with no other state change.
    pub fn record_chat_boundary_violation(&mut self, at_ms: u64) -> R {
        self.ensure_not_terminal()?;
        let index = self.current_index();
        self.push(at_ms, EventKind::ChatBoundaryViolated { index });
        Ok(())
    }

    /// Returns the total paste flags recorded on all question slots.
    pub fn paste_flag_count(&self) -> u32 {
        self.slots
            .iter()
            .fold(0u32, |acc, s| acc.saturating_add(s.paste_flags))
    }

    /// Returns how many chat boundary violations are in the event log.
    pub fn chat_boundary_violations(&self) -> usize {
        self.events
            .iter()
            .filter(|e| matches!(e.kind, EventKind::ChatBoundaryViolated { .. }))
            .count()
    }

    /// Returns every locked token expectation for one slot, in event order.
    pub fn token_locks(&self, slot_index: usize) -> Vec<u64> {
        self.events
            .iter()
            .filter_map(|e| match e.kind {
                EventKind::TokensLocked { index, expected } if index == slot_index => {
                    Some(expected)
                }
                _ => None,
            })
            .collect()
    }
}
