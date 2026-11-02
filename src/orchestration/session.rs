use crate::core::{ApproachChoice, Language, QuestionRef, Session, SessionStatus};
use crate::orchestration::error::OrchestratorError;

type Result<T = ()> = std::result::Result<T, OrchestratorError>;

/// Owns a core session and sequences lifecycle and candidate actions.
#[derive(Debug, Clone)]
pub struct Orchestrator {
    session: Session,
}

impl Orchestrator {
    /// Creates an orchestrator owning a new core session.
    pub fn create(
        session_id: String,
        language: Language,
        easy: Vec<QuestionRef>,
        now_ms: u64,
    ) -> Result<Self> {
        let session = Session::new(session_id, language, easy, now_ms)?;
        Ok(Self { session })
    }

    /// Returns the owned core session.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Returns the owned core session mutably.
    pub(crate) fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    /// Returns the session identifier.
    pub fn session_id(&self) -> &str {
        self.session.id()
    }

    /// Returns the current session status.
    pub fn status(&self) -> SessionStatus {
        self.session.status()
    }

    /// Returns the active question index, if any.
    pub fn current_index(&self) -> Option<usize> {
        self.session.current_index()
    }

    /// Returns whether a candidate is currently connected.
    pub fn is_connected(&self) -> bool {
        self.session.is_connected()
    }

    /// Returns whether the session is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        self.session.status().is_terminal()
    }

    /// Returns the active question reference, if any.
    pub fn current_question(&self) -> Option<&QuestionRef> {
        self.session.current_slot().map(|s| &s.question)
    }

    /// Returns elapsed time from the earliest question start.
    pub fn elapsed_ms(&self, now_ms: u64) -> u64 {
        let earliest = self
            .session
            .slots()
            .iter()
            .filter_map(|s| s.started_at_ms)
            .min();
        match earliest {
            Some(start) => now_ms.saturating_sub(start),
            None => 0,
        }
    }

    /// Returns elapsed time on the active question, if any.
    pub fn per_question_ms(&self, now_ms: u64) -> Option<u64> {
        let slot = self.session.current_slot()?;
        let started = slot.started_at_ms?;
        Some(now_ms.saturating_sub(started))
    }

    /// Moves the session to waiting for a candidate.
    pub fn mark_waiting_for_candidate(&mut self, now_ms: u64) -> Result {
        self.session.mark_waiting_for_candidate(now_ms)?;
        Ok(())
    }

    fn attach_inner(&mut self, now_ms: u64) -> Result {
        if self.session.is_connected() {
            if self.session.status().is_terminal() {
                self.session.candidate_connected(now_ms)?;
                return Ok(());
            }
            return Err(OrchestratorError::AlreadyAttached);
        }
        self.session.candidate_connected(now_ms)?;
        Ok(())
    }

    /// Attaches a candidate to the session.
    pub fn attach_candidate(&mut self, now_ms: u64) -> Result {
        self.attach_inner(now_ms)
    }

    /// Disconnects the currently attached candidate.
    pub fn disconnect_candidate(&mut self, now_ms: u64) -> Result {
        self.session.candidate_disconnected(now_ms)?;
        Ok(())
    }

    /// Reconnects a previously disconnected candidate.
    pub fn reconnect_candidate(&mut self, now_ms: u64) -> Result {
        self.attach_inner(now_ms)
    }

    /// Starts the interview on the first question.
    pub fn start(&mut self, now_ms: u64) -> Result {
        self.session.start(now_ms)?;
        Ok(())
    }

    /// Sets the expert-selected medium and hard questions.
    pub fn set_expert_questions(
        &mut self,
        medium: Vec<QuestionRef>,
        hard: Vec<QuestionRef>,
        now_ms: u64,
    ) -> Result {
        self.session.set_expert_questions(medium, hard, now_ms)?;
        Ok(())
    }

    /// Adds the single optional extra question.
    pub fn add_extra_question(&mut self, q: QuestionRef, now_ms: u64) -> Result {
        self.session.add_extra_question(q, now_ms)?;
        Ok(())
    }

    /// Ends the session by expert decision.
    pub fn end_by_expert(&mut self, now_ms: u64) -> Result {
        self.session.end_by_expert(now_ms)?;
        Ok(())
    }

    /// Marks the session as a technical failure.
    pub fn mark_technical_failure(&mut self, now_ms: u64) -> Result {
        self.session.mark_technical_failure(now_ms)?;
        Ok(())
    }

    /// Marks a scoring session as completed.
    pub fn mark_completed(&mut self, now_ms: u64) -> Result {
        self.session.mark_completed(now_ms)?;
        Ok(())
    }

    /// Selects an approach and locks expected tokens.
    pub fn select_approach(
        &mut self,
        choice: ApproachChoice,
        expected_tokens: u64,
        now_ms: u64,
    ) -> Result {
        self.session
            .select_approach(choice, expected_tokens, now_ms)?;
        Ok(())
    }

    /// Records that the candidate continues working.
    pub fn continue_work(&mut self, now_ms: u64) -> Result {
        self.session.continue_work(now_ms)?;
        Ok(())
    }

    /// Redirects the candidate with an instruction.
    pub fn redirect(&mut self, instruction: &str, now_ms: u64) -> Result {
        self.session.redirect(instruction, now_ms)?;
        Ok(())
    }

    /// Changes the selected approach on the active question.
    pub fn change_approach(&mut self, now_ms: u64) -> Result {
        self.session.change_approach(now_ms)?;
        Ok(())
    }

    /// Requests tests for the active question.
    pub fn request_tests(&mut self, now_ms: u64) -> Result {
        self.session.request_tests(now_ms)?;
        Ok(())
    }

    /// Records a test run result on the active question.
    pub fn record_test_result(&mut self, passed: u32, failed: u32, now_ms: u64) -> Result {
        self.session.record_test_result(passed, failed, now_ms)?;
        Ok(())
    }

    /// Finishes the active question and advances.
    pub fn finish_question(&mut self, now_ms: u64) -> Result {
        self.session.finish_question(now_ms)?;
        Ok(())
    }

    /// Skips the active question and advances.
    pub fn skip_question(&mut self, now_ms: u64) -> Result {
        self.session.skip_question(now_ms)?;
        Ok(())
    }

    /// Requests a replacement for the active expert question.
    pub fn request_requestion(&mut self, now_ms: u64) -> Result {
        self.session.request_requestion(now_ms)?;
        Ok(())
    }

    /// Resolves a pending re-question request.
    pub fn resolve_requestion(
        &mut self,
        approved: bool,
        replacement: Option<QuestionRef>,
        now_ms: u64,
    ) -> Result {
        self.session
            .resolve_requestion(approved, replacement, now_ms)?;
        Ok(())
    }

    /// Adds spent tokens to the active question without an event.
    pub fn add_tokens(&mut self, n: u64) -> Result {
        self.session.add_tokens(n)?;
        Ok(())
    }
}
