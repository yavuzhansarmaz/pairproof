use std::collections::HashSet;

use crate::core::events::EventKind::*;
use crate::core::events::{EventKind, SessionEvent};
use crate::core::types::CoreError::*;
use crate::core::types::QuestionStatus::*;
use crate::core::types::SessionStatus::*;
use crate::core::types::{
    ensure, expect_count, validate_question, CoreError, Language, QuestionRef, QuestionSlot,
    QuestionSource, SessionStatus,
};
use crate::leetcode::Difficulty;

pub(crate) type R<T = ()> = Result<T, CoreError>;

/// Pure in-memory interview session state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub(crate) id: String,
    pub(crate) language: Language,
    pub(crate) status: SessionStatus,
    pub(crate) slots: Vec<QuestionSlot>,
    pub(crate) events: Vec<SessionEvent>,
    pub(crate) expert_set: bool,
    pub(crate) extra_added: bool,
    pub(crate) connected: bool,
}

impl Session {
    /// Creates a session with exactly 5 unique easy questions.
    pub fn new(id: String, language: Language, easy: Vec<QuestionRef>, at_ms: u64) -> R<Self> {
        ensure(!id.trim().is_empty(), EmptySessionId)?;
        expect_count(easy.len(), 5)?;
        let mut seen = HashSet::new();
        for q in &easy {
            validate_question(q, Some(Difficulty::Easy))?;
            ensure(
                seen.insert(q.title_slug.as_str()),
                DuplicateSlug(q.title_slug.clone()),
            )?;
        }
        let slots = easy
            .into_iter()
            .enumerate()
            .map(|(i, q)| QuestionSlot::new(i, QuestionSource::AutoEasy, q))
            .collect();
        let mut s = Self {
            id,
            language,
            status: Setup,
            slots,
            events: Vec::new(),
            expert_set: false,
            extra_added: false,
            connected: false,
        };
        s.push(at_ms, SessionCreated);
        Ok(s)
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn language(&self) -> Language {
        self.language
    }
    pub fn status(&self) -> SessionStatus {
        self.status
    }
    pub fn slots(&self) -> &[QuestionSlot] {
        &self.slots
    }
    pub fn slot(&self, index: usize) -> Option<&QuestionSlot> {
        self.slots.get(index)
    }
    pub fn events(&self) -> &[SessionEvent] {
        &self.events
    }
    pub fn current_index(&self) -> Option<usize> {
        self.active_pos().map(|p| self.slots[p].index)
    }
    pub fn current_slot(&self) -> Option<&QuestionSlot> {
        self.active_pos().map(|p| &self.slots[p])
    }

    pub fn mark_waiting_for_candidate(&mut self, at_ms: u64) -> R {
        self.ensure_not_terminal()?;
        ensure(
            self.status == Setup,
            self.bad_state("mark_waiting_for_candidate"),
        )?;
        self.status = WaitingForCandidate;
        self.push(at_ms, WaitingForCandidateStarted);
        Ok(())
    }

    pub fn candidate_connected(&mut self, at_ms: u64) -> R {
        self.ensure_not_terminal()?;
        ensure(
            self.status == WaitingForCandidate,
            self.bad_state("candidate_connected"),
        )?;
        ensure(!self.connected, CandidateAlreadyConnected)?;
        self.connected = true;
        self.push(at_ms, CandidateConnected);
        Ok(())
    }

    pub fn start(&mut self, at_ms: u64) -> R {
        self.ensure_not_terminal()?;
        ensure(self.status == WaitingForCandidate, self.bad_state("start"))?;
        ensure(self.connected, CandidateNotConnected)?;
        self.status = InProgress;
        ensure(self.activate_next(at_ms), NoActiveQuestion)
    }

    pub fn set_expert_questions(
        &mut self,
        medium: Vec<QuestionRef>,
        hard: Vec<QuestionRef>,
        at_ms: u64,
    ) -> R {
        self.ensure_not_terminal()?;
        ensure(
            self.status != Scoring,
            self.bad_state("set_expert_questions"),
        )?;
        ensure(!self.expert_set, ExpertAlreadySet)?;
        expect_count(medium.len(), 3)?;
        expect_count(hard.len(), 2)?;
        let mut seen: HashSet<String> = self
            .slots
            .iter()
            .map(|s| s.question.title_slug.clone())
            .collect();
        for (list, diff) in [(&medium, Difficulty::Medium), (&hard, Difficulty::Hard)] {
            for q in list {
                validate_question(q, Some(diff))?;
                ensure(
                    seen.insert(q.title_slug.clone()),
                    DuplicateSlug(q.title_slug.clone()),
                )?;
            }
        }
        for q in medium.into_iter().chain(hard) {
            let idx = self.slots.len();
            self.slots
                .push(QuestionSlot::new(idx, QuestionSource::Expert, q));
        }
        self.expert_set = true;
        self.push(at_ms, ExpertQuestionsSet);
        if self.status == WaitingForExpert {
            self.status = InProgress;
            self.activate_next(at_ms);
        }
        Ok(())
    }

    pub fn add_extra_question(&mut self, q: QuestionRef, at_ms: u64) -> R {
        self.ensure_not_terminal()?;
        ensure(self.status != Scoring, self.bad_state("add_extra_question"))?;
        ensure(self.expert_set, ExpertNotSet)?;
        ensure(!self.extra_added, ExtraAlreadyAdded)?;
        ensure(self.slots.len() < 11, SlotLimitExceeded)?;
        validate_question(&q, None)?;
        ensure(
            !self
                .slots
                .iter()
                .any(|s| s.question.title_slug == q.title_slug),
            DuplicateSlug(q.title_slug.clone()),
        )?;
        let idx = self.slots.len();
        self.slots
            .push(QuestionSlot::new(idx, QuestionSource::ExpertExtra, q));
        self.extra_added = true;
        self.push(at_ms, ExtraQuestionAdded);
        Ok(())
    }

    pub fn mark_completed(&mut self, at_ms: u64) -> R {
        self.ensure_not_terminal()?;
        ensure(self.status == Scoring, self.bad_state("mark_completed"))?;
        self.end_session(SessionStatus::Completed, EventKind::Completed, at_ms)
    }

    pub fn end_by_expert(&mut self, at_ms: u64) -> R {
        self.end_session(EndedByExpert, ExpertEnded, at_ms)
    }

    pub fn mark_technical_failure(&mut self, at_ms: u64) -> R {
        self.end_session(
            SessionStatus::TechnicalFailure,
            EventKind::TechnicalFailure,
            at_ms,
        )
    }

    pub(crate) fn push(&mut self, at_ms: u64, kind: EventKind) {
        let seq = self.events.len() as u64;
        self.events.push(SessionEvent { seq, at_ms, kind });
    }

    pub(crate) fn ensure_not_terminal(&self) -> R {
        ensure(!self.status.is_terminal(), Terminal)
    }

    pub(crate) fn bad_state(&self, op: &str) -> CoreError {
        InvalidState(format!("{op} in {:?}", self.status))
    }

    pub(crate) fn active_pos(&self) -> Option<usize> {
        self.slots.iter().position(|s| s.status == Active)
    }

    pub(crate) fn active(&self) -> R<usize> {
        self.ensure_not_terminal()?;
        self.active_pos().ok_or(NoActiveQuestion)
    }

    pub(crate) fn ensure_no_requestion_pending(&self, p: usize) -> R {
        ensure(!self.slots[p].re_question_pending, ReQuestionPending)
    }

    pub(crate) fn activate_next(&mut self, at_ms: u64) -> bool {
        let Some(p) = self.slots.iter().position(|s| s.status == Pending) else {
            return false;
        };
        self.slots[p].status = Active;
        self.slots[p].started_at_ms = Some(at_ms);
        let index = self.slots[p].index;
        self.push(at_ms, QuestionStarted { index });
        true
    }

    fn end_session(&mut self, status: SessionStatus, kind: EventKind, at_ms: u64) -> R {
        self.ensure_not_terminal()?;
        self.status = status;
        self.push(at_ms, kind);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::test_support::*;
    use crate::core::types::SessionStatus;

    #[test]
    fn start_requires_waiting_and_activates_first_slot() {
        let mut s = new_session();
        assert!(s.start(1).is_err());
        assert!(s.candidate_connected(1).is_err());
        s.mark_waiting_for_candidate(1).expect("waiting");
        assert!(s.mark_waiting_for_candidate(2).is_err());
        s.candidate_connected(2).expect("connected");
        s.start(3).expect("start");
        assert_eq!(s.status(), SessionStatus::InProgress);
        assert_eq!(s.current_index(), Some(0));
        assert!(s.start(4).is_err());
    }

    #[test]
    fn start_requires_candidate_connection() {
        let mut s = new_session();
        s.mark_waiting_for_candidate(1).expect("waiting");
        assert_eq!(s.start(2), Err(CoreError::CandidateNotConnected));
        assert_eq!(s.status(), SessionStatus::WaitingForCandidate);
        assert_eq!(s.current_index(), None);
        s.candidate_connected(3).expect("connected");
        s.start(4).expect("start after connect");
        assert_eq!(s.current_index(), Some(0));
    }

    #[test]
    fn second_candidate_connection_is_rejected() {
        let mut s = new_session();
        s.mark_waiting_for_candidate(1).expect("waiting");
        s.candidate_connected(2).expect("first connect");
        let n = s.events().len();
        assert_eq!(
            s.candidate_connected(3),
            Err(CoreError::CandidateAlreadyConnected)
        );
        assert_eq!(s.events().len(), n);
        assert_eq!(
            s.events()
                .iter()
                .filter(|e| e.kind == EventKind::CandidateConnected)
                .count(),
            1
        );
        s.start(4).expect("start");
        assert!(s.candidate_connected(5).is_err());
    }

    #[test]
    fn waiting_for_expert_after_fifth_then_expert_set_resumes() {
        let mut s = started();
        for i in 0..5u64 {
            s.finish_question(10 + i).expect("finish");
        }
        assert_eq!(s.status(), SessionStatus::WaitingForExpert);
        assert_eq!(s.current_index(), None);
        s.set_expert_questions(
            qs("m", 3, Difficulty::Medium),
            qs("h", 2, Difficulty::Hard),
            20,
        )
        .expect("expert set");
        assert_eq!(s.status(), SessionStatus::InProgress);
        assert_eq!(s.current_index(), Some(5));
    }

    #[test]
    fn finishing_all_reaches_scoring_then_completed_terminal() {
        let mut s = with_expert();
        for i in 0..10u64 {
            s.finish_question(10 + i).expect("finish");
        }
        assert_eq!(s.status(), SessionStatus::Scoring);
        assert!(s.finish_question(30).is_err());
        s.mark_completed(31).expect("completed");
        assert_eq!(s.status(), SessionStatus::Completed);
        assert!(s.end_by_expert(32).is_err());
        assert!(s.finish_question(32).is_err());
        assert!(s.add_tokens(1).is_err());
    }

    #[test]
    fn expert_end_and_technical_failure_are_terminal() {
        let mut s = started();
        s.end_by_expert(9).expect("end");
        assert_eq!(s.status(), SessionStatus::EndedByExpert);
        assert!(s.continue_work(10).is_err());
        let mut s = started();
        s.mark_technical_failure(9).expect("failure");
        assert_eq!(s.status(), SessionStatus::TechnicalFailure);
        assert!(s.mark_completed(10).is_err());
    }
}
