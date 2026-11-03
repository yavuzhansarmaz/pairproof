use crate::core::events::EventKind;
use crate::core::events::EventKind::*;
use crate::core::session::{Session, R};
use crate::core::types::CoreError::*;
use crate::core::types::QuestionStatus::*;
use crate::core::types::SessionStatus::*;
use crate::core::types::{ensure, validate_question, ApproachChoice, QuestionRef, QuestionSource};

impl Session {
    pub fn select_approach(
        &mut self,
        choice: ApproachChoice,
        expected_tokens: u64,
        at_ms: u64,
    ) -> R {
        let p = self.active()?;
        self.ensure_no_requestion_pending(p)?;
        self.ensure_connected()?;
        ensure(self.slots[p].approach.is_none(), ApproachAlreadySelected)?;
        ensure(expected_tokens > 0, InvalidExpectedTokens)?;
        let choice = choice.validated()?;
        let index = self.slots[p].index;
        self.slots[p].approach = Some(choice.clone());
        self.slots[p].locked_expected_tokens = Some(expected_tokens);
        self.push(at_ms, ApproachSelected { index, choice });
        self.push(
            at_ms,
            TokensLocked {
                index,
                expected: expected_tokens,
            },
        );
        Ok(())
    }

    pub fn continue_work(&mut self, at_ms: u64) -> R {
        self.emit_active(at_ms, |index| Continued { index })
    }

    pub fn redirect(&mut self, instruction: &str, at_ms: u64) -> R {
        let instruction = instruction.trim().to_string();
        ensure(!instruction.is_empty(), EmptyInstruction)?;
        self.emit_active(at_ms, move |index| Redirected {
            index,
            instruction: instruction.clone(),
        })
    }

    pub fn request_tests(&mut self, at_ms: u64) -> R {
        self.emit_active(at_ms, |index| TestsRequested { index })
    }

    pub fn change_approach(&mut self, at_ms: u64) -> R {
        let p = self.active()?;
        self.ensure_no_requestion_pending(p)?;
        self.ensure_connected()?;
        ensure(self.slots[p].approach.is_some(), NoApproachSelected)?;
        let index = self.slots[p].index;
        self.slots[p].approach = None;
        self.slots[p].locked_expected_tokens = None;
        self.push(at_ms, ApproachChanged { index });
        Ok(())
    }

    /// Adds spent tokens to the active slot; emits no event.
    pub fn add_tokens(&mut self, n: u64) -> R {
        let p = self.active()?;
        self.ensure_connected()?;
        self.slots[p].actual_tokens = self.slots[p].actual_tokens.saturating_add(n);
        Ok(())
    }

    pub fn request_requestion(&mut self, at_ms: u64) -> R {
        let p = self.active()?;
        ensure(!self.slots[p].re_question_pending, ReQuestionAlreadyUsed)?;
        self.ensure_connected()?;
        let s = &self.slots[p];
        ensure(s.source == QuestionSource::Expert, ReQuestionNotAllowed)?;
        ensure(s.approach.is_none(), ReQuestionNotAllowed)?;
        ensure(
            !s.re_question_used && !s.re_question_pending,
            ReQuestionAlreadyUsed,
        )?;
        let index = s.index;
        self.slots[p].re_question_pending = true;
        self.push(at_ms, ReQuestionRequested { index });
        Ok(())
    }

    pub fn resolve_requestion(
        &mut self,
        approved: bool,
        replacement: Option<QuestionRef>,
        at_ms: u64,
    ) -> R {
        let p = self.active()?;
        ensure(self.slots[p].re_question_pending, ReQuestionNotPending)?;
        if approved {
            let repl = replacement.ok_or(ReplacementRequired)?;
            validate_question(&repl, None)?;
            ensure(
                repl.difficulty == self.slots[p].question.difficulty,
                ReplacementDifficultyMismatch,
            )?;
            ensure(
                !self
                    .slots
                    .iter()
                    .enumerate()
                    .any(|(i, s)| i != p && s.question.title_slug == repl.title_slug),
                DuplicateSlug(repl.title_slug.clone()),
            )?;
            self.slots[p].question = repl;
        }
        let index = self.slots[p].index;
        self.slots[p].re_question_pending = false;
        self.slots[p].re_question_used = true;
        self.push(at_ms, ReQuestionResolved { index, approved });
        Ok(())
    }

    pub fn finish_question(&mut self, at_ms: u64) -> R {
        self.close(false, at_ms)
    }

    pub fn skip_question(&mut self, at_ms: u64) -> R {
        self.close(true, at_ms)
    }

    fn emit_active(&mut self, at_ms: u64, kind: impl Fn(usize) -> EventKind) -> R {
        let p = self.active()?;
        self.ensure_no_requestion_pending(p)?;
        self.ensure_connected()?;
        let index = self.slots[p].index;
        self.push(at_ms, kind(index));
        Ok(())
    }

    fn close(&mut self, skipped: bool, at_ms: u64) -> R {
        let p = self.active()?;
        self.ensure_no_requestion_pending(p)?;
        self.ensure_connected()?;
        let index = self.slots[p].index;
        self.slots[p].status = if skipped { Skipped } else { Finished };
        self.slots[p].finished_at_ms = Some(at_ms);
        self.push(
            at_ms,
            if skipped {
                QuestionSkipped { index }
            } else {
                QuestionFinished { index }
            },
        );
        if !self.activate_next(at_ms) {
            self.status = if self.expert_set {
                Scoring
            } else {
                WaitingForExpert
            };
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::events::EventKind;
    use crate::core::test_support::*;
    use crate::core::types::{CoreError, Difficulty, QuestionStatus};

    #[test]
    fn actions_require_active_and_redirect_needs_text() {
        let mut s = new_session();
        assert!(s.continue_work(1).is_err());
        assert!(s.add_tokens(1).is_err());
        let mut s = started();
        assert!(s.redirect("  ", 5).is_err());
        s.continue_work(5).expect("continue");
        s.redirect("use a heap", 6).expect("redirect");
        s.request_tests(7).expect("tests");
        s.select_approach(ApproachChoice::Preset(0), 10, 8)
            .expect("select");
        let n = s.events().len();
        s.add_tokens(3).expect("tokens");
        assert_eq!(s.events().len(), n);
    }

    #[test]
    fn skip_advances_and_closed_slots_stay_closed() {
        let mut s = started();
        s.skip_question(5).expect("skip");
        assert_eq!(s.slot(0).expect("slot").status, QuestionStatus::Skipped);
        assert_eq!(s.current_index(), Some(1));
        s.finish_question(6).expect("finish");
        assert_eq!(s.slot(0).expect("slot").status, QuestionStatus::Skipped);
        assert_eq!(s.slot(1).expect("slot").status, QuestionStatus::Finished);
        assert_eq!(s.current_index(), Some(2));
    }

    #[test]
    fn approach_locks_then_change_frees_reselection_keeping_tokens() {
        let mut s = started();
        assert!(s
            .select_approach(ApproachChoice::Preset(4), 100, 5)
            .is_err());
        assert!(s.select_approach(ApproachChoice::Preset(0), 0, 5).is_err());
        s.select_approach(ApproachChoice::Preset(1), 100, 5)
            .expect("select");
        assert!(s.select_approach(ApproachChoice::Preset(2), 50, 6).is_err());
        s.add_tokens(40).expect("tokens");
        s.change_approach(7).expect("change");
        let slot = s.current_slot().expect("slot");
        assert_eq!(slot.actual_tokens, 40);
        assert_eq!(slot.locked_expected_tokens, None);
        s.select_approach(ApproachChoice::Preset(2), 200, 8)
            .expect("reselect");
        assert_eq!(
            s.current_slot().expect("slot").locked_expected_tokens,
            Some(200)
        );
    }

    #[test]
    fn re_question_blocked_on_easy_extra_and_after_approach() {
        assert!(started().request_requestion(5).is_err());
        let mut s = at_expert();
        s.select_approach(ApproachChoice::Preset(0), 10, 20)
            .expect("select");
        assert!(s.request_requestion(21).is_err());
        let mut s = at_expert();
        s.add_extra_question(q("x0", Difficulty::Medium), 20)
            .expect("extra");
        for i in 0..5u64 {
            s.finish_question(30 + i).expect("finish expert");
        }
        assert_eq!(s.current_index(), Some(10));
        assert!(s.request_requestion(40).is_err());
    }

    #[test]
    fn re_question_approve_swaps_same_difficulty_and_reject_burns() {
        let mut s = at_expert();
        s.request_requestion(20).expect("request");
        assert!(s
            .resolve_requestion(true, Some(q("r0", Difficulty::Hard)), 21)
            .is_err());
        assert!(s.current_slot().expect("slot").re_question_pending);
        s.resolve_requestion(true, Some(q("r0", Difficulty::Medium)), 22)
            .expect("approve");
        assert_eq!(s.current_slot().expect("slot").question.title_slug, "r0");
        assert!(s.request_requestion(23).is_err());
        let mut s = at_expert();
        s.request_requestion(20).expect("request");
        s.resolve_requestion(false, None, 21).expect("reject");
        assert!(s.current_slot().expect("slot").re_question_used);
        assert!(s.request_requestion(22).is_err());
        assert!(s.resolve_requestion(false, None, 23).is_err());
    }

    #[test]
    fn pending_requestion_blocks_question_flow() {
        let mut s = at_expert();
        s.request_requestion(20).expect("request");
        assert_eq!(s.finish_question(21), Err(CoreError::ReQuestionPending));
        assert_eq!(s.skip_question(21), Err(CoreError::ReQuestionPending));
        assert_eq!(
            s.select_approach(ApproachChoice::Preset(0), 10, 21),
            Err(CoreError::ReQuestionPending)
        );
        assert_eq!(s.continue_work(21), Err(CoreError::ReQuestionPending));
        assert_eq!(
            s.redirect("use a heap", 21),
            Err(CoreError::ReQuestionPending)
        );
        assert_eq!(s.request_tests(21), Err(CoreError::ReQuestionPending));
        assert_eq!(s.change_approach(21), Err(CoreError::ReQuestionPending));
        let slot = s.current_slot().expect("active slot stays open");
        assert_eq!(slot.status, QuestionStatus::Active);
        assert!(slot.re_question_pending);
        assert_eq!(s.current_index(), Some(5));
        s.resolve_requestion(true, Some(q("r0", Difficulty::Medium)), 22)
            .expect("resolve on active");
        assert_eq!(s.current_slot().expect("slot").question.title_slug, "r0");
        s.finish_question(23).expect("finish after resolve");
        assert_eq!(s.slot(5).expect("slot").status, QuestionStatus::Finished);
        assert!(!s.slot(5).expect("slot").re_question_pending);
        assert!(s
            .resolve_requestion(true, Some(q("r1", Difficulty::Medium)), 24)
            .is_err());
        assert_eq!(s.slot(5).expect("slot").question.title_slug, "r0");
    }

    #[test]
    fn events_record_approach_choice_and_redirect_text() {
        let mut s = started();
        s.select_approach(ApproachChoice::Other("  sliding window  ".into()), 100, 5)
            .expect("select");
        s.redirect("  use a heap instead  ", 6).expect("redirect");
        assert!(s.events().iter().any(|e| e.kind
            == EventKind::ApproachSelected {
                index: 0,
                choice: ApproachChoice::Other("sliding window".into())
            }));
        assert!(s.events().iter().any(|e| e.kind
            == EventKind::Redirected {
                index: 0,
                instruction: "use a heap instead".into()
            }));
        s.change_approach(7).expect("change");
        s.select_approach(ApproachChoice::Preset(2), 50, 8)
            .expect("reselect");
        assert!(s.events().iter().any(|e| e.kind
            == EventKind::ApproachSelected {
                index: 0,
                choice: ApproachChoice::Preset(2)
            }));
    }

    #[test]
    fn event_seq_is_strictly_increasing_from_zero() {
        let mut s = started();
        s.select_approach(ApproachChoice::Preset(3), 77, 5)
            .expect("select");
        s.finish_question(6).expect("finish");
        let seqs: Vec<u64> = s.events().iter().map(|e| e.seq).collect();
        let expected: Vec<u64> = (0..seqs.len() as u64).collect();
        assert_eq!(seqs, expected);
        assert_eq!(s.events()[0].kind, EventKind::SessionCreated);
        assert!(s.events().iter().any(|e| e.kind
            == EventKind::TokensLocked {
                index: 0,
                expected: 77
            }));
    }

    #[test]
    fn finish_advances_in_order() {
        let mut s = started();
        s.finish_question(9).expect("finish");
        assert_eq!(s.current_index(), Some(1));
        assert_eq!(s.slot(0).expect("slot").status, Finished);
    }
}
