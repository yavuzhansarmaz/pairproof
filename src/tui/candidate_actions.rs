//! Candidate coding-step actions and runner ports.

use std::fmt;

use crate::core::{ApproachChoice, Language, Session, TestSummary};
use crate::llm::coding::{CodingStepInput, CodingStepOutput};
use crate::scoring::ApproachGrade;
use crate::tui::candidate_work::{CandidateWork, CandidateWorkError, WorkPhase};

/// Typed runner failure; payload strings are never carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerError {
    Unavailable,
    Failed,
}

impl fmt::Display for RunnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunnerError::Unavailable => write!(f, "runner not available yet (Phase 10)"),
            RunnerError::Failed => write!(f, "runner failed"),
        }
    }
}

impl std::error::Error for RunnerError {}

/// Runs one coding step for the candidate controller.
pub trait CodingRunner {
    /// Returns the next complete code for one coding step.
    fn run_step(&mut self, input: &CodingStepInput) -> Result<CodingStepOutput, RunnerError>;
}

/// Runs tests for the candidate controller.
pub trait TestRunner {
    /// Returns passed and failed counts for `code`.
    fn run_tests(&mut self, code: &str, language: Language) -> Result<(u32, u32), RunnerError>;
}

/// Coding runner used until Phase 10 transport wiring.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableCodingRunner;

impl CodingRunner for UnavailableCodingRunner {
    fn run_step(&mut self, _input: &CodingStepInput) -> Result<CodingStepOutput, RunnerError> {
        Err(RunnerError::Unavailable)
    }
}

/// Test runner used until Phase 10 transport wiring.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableTestRunner;

impl TestRunner for UnavailableTestRunner {
    fn run_tests(&mut self, _code: &str, _language: Language) -> Result<(u32, u32), RunnerError> {
        Err(RunnerError::Unavailable)
    }
}

/// Grades a candidate-written Other approach at selection time.
pub trait OtherGrader {
    /// Returns the hidden grade for one Other approach text.
    fn grade(
        &mut self,
        question_statement: &str,
        other_text: &str,
    ) -> Result<ApproachGrade, RunnerError>;
}

/// Other grader used until Phase 10 transport wiring.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableOtherGrader;

impl OtherGrader for UnavailableOtherGrader {
    fn grade(
        &mut self,
        _question_statement: &str,
        _other_text: &str,
    ) -> Result<ApproachGrade, RunnerError> {
        Err(RunnerError::Unavailable)
    }
}

impl CandidateWork {
    /// Selects a preset approach, then runs one coding step.
    /// Runner failure only sets the phase; the core selection stands.
    pub fn select_preset(
        &mut self,
        session: &mut Session,
        preset: u8,
        runner: &mut dyn CodingRunner,
        at_ms: u64,
    ) -> Result<(), CandidateWorkError> {
        let approaches = self
            .approaches
            .as_ref()
            .ok_or(CandidateWorkError::NoApproaches)?;
        let option = approaches
            .options
            .get(preset as usize)
            .ok_or(CandidateWorkError::InvalidPreset(preset))?;
        let expected = option.expected_tokens;
        let title = option.title.clone();
        session.select_approach(ApproachChoice::Preset(preset), expected, at_ms)?;
        self.run_coding_step(session, title, None, runner);
        Ok(())
    }

    /// Selects the Other approach, then runs one coding step.
    /// Runner failure only sets the phase; the core selection stands.
    pub fn select_other(
        &mut self,
        session: &mut Session,
        grade: ApproachGrade,
        runner: &mut dyn CodingRunner,
        at_ms: u64,
    ) -> Result<(), CandidateWorkError> {
        let text = self.other_text.trim().to_string();
        session.select_approach(
            ApproachChoice::Other(text.clone()),
            grade.expected_tokens,
            at_ms,
        )?;
        self.other_text.clear();
        self.run_coding_step(session, text, None, runner);
        Ok(())
    }

    /// Grades the Other text, then selects it through `select_other`.
    /// Grader failure only sets the phase; no core selection is recorded.
    pub fn select_other_graded(
        &mut self,
        session: &mut Session,
        grader: &mut dyn OtherGrader,
        runner: &mut dyn CodingRunner,
        at_ms: u64,
    ) -> Result<(), CandidateWorkError> {
        if self.other_text.trim().is_empty() {
            ApproachChoice::Other(self.other_text.clone()).validated()?;
        }
        let statement = self.statement_or_title(session);
        let other_text = self.other_text.clone();
        let grade = match grader.grade(&statement, &other_text) {
            Ok(grade) => grade,
            Err(RunnerError::Unavailable) => {
                self.phase = WorkPhase::RunnerUnavailable;
                return Ok(());
            }
            Err(RunnerError::Failed) => {
                self.phase = WorkPhase::RunnerFailed;
                return Ok(());
            }
        };
        self.select_other(session, grade, runner, at_ms)
    }

    /// Records a continue action, then runs one coding step.
    /// Runner failure only sets the phase; the core action stands.
    pub fn continue_work(
        &mut self,
        session: &mut Session,
        runner: &mut dyn CodingRunner,
        at_ms: u64,
    ) -> Result<(), CandidateWorkError> {
        session.continue_work(at_ms)?;
        let approach_text = self.selected_approach_text(session);
        self.run_coding_step(session, approach_text, None, runner);
        Ok(())
    }

    /// Records a redirect instruction, then runs one coding step.
    /// Runner failure only sets the phase; the core action stands.
    pub fn redirect(
        &mut self,
        session: &mut Session,
        runner: &mut dyn CodingRunner,
        at_ms: u64,
    ) -> Result<(), CandidateWorkError> {
        let text = self.redirect_text.trim().to_string();
        session.redirect(&text, at_ms)?;
        self.redirect_text.clear();
        let approach_text = self.selected_approach_text(session);
        self.run_coding_step(session, approach_text, Some(text), runner);
        Ok(())
    }

    /// Clears the current approach selection through the core.
    pub fn change_approach(
        &mut self,
        session: &mut Session,
        at_ms: u64,
    ) -> Result<(), CandidateWorkError> {
        session.change_approach(at_ms)?;
        self.phase = WorkPhase::Idle;
        Ok(())
    }

    /// Requests tests and records only a real runner outcome.
    /// Runner failure records nothing and only updates the view flags.
    pub fn request_tests(
        &mut self,
        session: &mut Session,
        runner: &mut dyn TestRunner,
        at_ms: u64,
    ) -> Result<(), CandidateWorkError> {
        session.request_tests(at_ms)?;
        let code = self.current_code.clone();
        match runner.run_tests(&code, session.language()) {
            Ok((passed, failed)) => {
                session.record_test_result(passed, failed, at_ms)?;
                self.tests_unavailable = false;
                self.tests_failed = false;
            }
            Err(RunnerError::Unavailable) => {
                self.tests_unavailable = true;
                self.tests_failed = false;
            }
            Err(RunnerError::Failed) => {
                self.tests_unavailable = false;
                self.tests_failed = true;
            }
        }
        Ok(())
    }

    /// Returns the latest recorded test summary for the active slot.
    pub fn finish_preview(&self, session: &Session) -> Option<TestSummary> {
        crate::tui::candidate_work::finish_preview(session)
    }

    /// Finishes the active question and returns its test preview.
    pub fn finish_question(
        &mut self,
        session: &mut Session,
        at_ms: u64,
    ) -> Result<Option<TestSummary>, CandidateWorkError> {
        let preview = self.finish_preview(session);
        session.finish_question(at_ms)?;
        self.reset_for_next_question();
        Ok(preview)
    }

    /// Skips the active question through the core.
    pub fn skip_question(
        &mut self,
        session: &mut Session,
        at_ms: u64,
    ) -> Result<(), CandidateWorkError> {
        session.skip_question(at_ms)?;
        self.reset_for_next_question();
        Ok(())
    }

    /// Requests a re-question through the core; never resolves one.
    pub fn request_requestion(
        &mut self,
        session: &mut Session,
        at_ms: u64,
    ) -> Result<(), CandidateWorkError> {
        session.request_requestion(at_ms)?;
        Ok(())
    }

    /// Returns true when a re-question may be requested now.
    pub fn can_request_requestion(&self, session: &Session) -> bool {
        crate::tui::candidate_work::can_request_requestion(session)
    }

    fn reset_for_next_question(&mut self) {
        self.approaches = None;
        self.other_text.clear();
        self.redirect_text.clear();
        self.current_code.clear();
        self.phase = WorkPhase::Idle;
        self.tests_unavailable = false;
        self.tests_failed = false;
    }

    fn statement_or_title(&self, session: &Session) -> String {
        let Some(slot) = session.current_slot() else {
            return String::new();
        };
        self.statements
            .get(&slot.question.title_slug)
            .filter(|text| !text.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| slot.question.title.clone())
    }

    fn selected_approach_text(&self, session: &Session) -> String {
        let Some(slot) = session.current_slot() else {
            return String::new();
        };
        match &slot.approach {
            Some(ApproachChoice::Preset(n)) => self
                .approaches
                .as_ref()
                .and_then(|set| set.options.get(*n as usize))
                .map(|option| option.title.clone())
                .unwrap_or_else(|| format!("preset {n}")),
            Some(ApproachChoice::Other(text)) => text.clone(),
            None => String::new(),
        }
    }

    fn run_coding_step(
        &mut self,
        session: &Session,
        approach_text: String,
        instruction: Option<String>,
        runner: &mut dyn CodingRunner,
    ) {
        let input = CodingStepInput {
            question_statement: self.statement_or_title(session),
            approach_text,
            current_code: self.current_code.clone(),
            instruction,
            tests_requested: false,
            language: session.language(),
        };
        match runner.run_step(&input) {
            Ok(output) => {
                self.current_code = output.code;
                self.phase = WorkPhase::CodingReady;
            }
            Err(RunnerError::Unavailable) => self.phase = WorkPhase::RunnerUnavailable,
            Err(RunnerError::Failed) => self.phase = WorkPhase::RunnerFailed,
        }
    }
}
