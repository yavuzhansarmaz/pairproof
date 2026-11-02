//! Candidate problem, approach, and coding-step controller.

use std::collections::HashMap;

use crate::core::{ApproachChoice, CoreError, QuestionSource, Session, TestSummary};
use crate::scoring::{ApproachOptionView, ApproachSet};
use crate::tui::shared::sanitize_display;

pub use crate::tui::candidate_actions::{
    CodingRunner, RunnerError, TestRunner, UnavailableCodingRunner, UnavailableTestRunner,
};

/// Phase of candidate coding work for the active question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkPhase {
    #[default]
    Idle,
    CodingReady,
    RunnerUnavailable,
    RunnerFailed,
}

impl WorkPhase {
    /// Returns the candidate-safe phase line for this phase.
    pub fn label(self) -> &'static str {
        match self {
            WorkPhase::Idle => "phase: waiting for an approach choice",
            WorkPhase::CodingReady => "phase: code ready",
            WorkPhase::RunnerUnavailable => "phase: coding step not available yet",
            WorkPhase::RunnerFailed => {
                "phase: the coding step failed; ask the expert if it persists."
            }
        }
    }
}

/// Typed candidate-work failures; runner payloads stay generic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CandidateWorkError {
    /// Core session failure.
    #[error(transparent)]
    Core(#[from] CoreError),
    /// No approach set has been provided for this question.
    #[error("approaches are not available yet.")]
    NoApproaches,
    /// Preset index is outside the available approach set.
    #[error("invalid approach preset {0}")]
    InvalidPreset(u8),
    /// Coding runner is unavailable; the payload is never shown.
    #[error("The coding step is not available yet; ask the expert if it persists.")]
    RunnerUnavailable(String),
    /// Coding runner failed; the payload is never shown.
    #[error("The coding step failed; ask the expert if it persists.")]
    RunnerFailed(String),
}

/// Pure candidate-work state; the caller keeps owning the session.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CandidateWork {
    pub(crate) approaches: Option<ApproachSet>,
    pub(crate) statements: HashMap<String, String>,
    pub(crate) other_text: String,
    pub(crate) redirect_text: String,
    pub(crate) current_code: String,
    pub(crate) phase: WorkPhase,
    pub(crate) tests_unavailable: bool,
    pub(crate) tests_failed: bool,
}

impl CandidateWork {
    /// Creates empty candidate-work state in the idle phase.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the current coding phase.
    pub fn phase(&self) -> WorkPhase {
        self.phase
    }

    /// Returns the latest code produced by a coding step.
    pub fn current_code(&self) -> &str {
        &self.current_code
    }

    /// Replaces the approach set for the current question.
    pub fn set_approaches(&mut self, approaches: ApproachSet) {
        self.approaches = Some(approaches);
    }

    /// Stores the statement text for one question slug.
    pub fn set_statement(&mut self, slug: impl Into<String>, text: impl Into<String>) {
        self.statements.insert(slug.into(), text.into());
    }

    /// Returns the stored statement for one question slug.
    pub fn statement_for(&self, slug: &str) -> Option<&str> {
        self.statements.get(slug).map(String::as_str)
    }

    /// Returns candidate-visible approach options, if any are set.
    pub fn approach_options(&self) -> Vec<ApproachOptionView> {
        self.approaches
            .as_ref()
            .map(ApproachSet::candidate_options)
            .unwrap_or_default()
    }

    /// Replaces the candidate-written Other approach text.
    pub fn set_other_text(&mut self, text: &str) {
        self.other_text = text.to_string();
    }

    /// Returns the candidate-written Other approach text.
    pub fn other_text(&self) -> &str {
        &self.other_text
    }

    /// Appends one character to the Other approach text.
    pub fn push_other_char(&mut self, c: char) {
        if !c.is_control() {
            self.other_text.push(c);
        }
    }

    /// Appends text to the Other approach text.
    pub fn push_other_text(&mut self, text: &str) {
        for c in text.chars() {
            self.push_other_char(c);
        }
    }

    /// Replaces the redirect instruction text.
    pub fn set_redirect_text(&mut self, text: &str) {
        self.redirect_text = text.to_string();
    }

    /// Returns the redirect instruction text.
    pub fn redirect_text(&self) -> &str {
        &self.redirect_text
    }

    /// Appends one character to the redirect instruction text.
    pub fn push_redirect_char(&mut self, c: char) {
        if !c.is_control() {
            self.redirect_text.push(c);
        }
    }
}

/// Returns true when core re-question rules allow a request.
pub fn can_request_requestion(session: &Session) -> bool {
    session.current_slot().is_some_and(|slot| {
        slot.source == QuestionSource::Expert
            && slot.approach.is_none()
            && !slot.re_question_used
            && !slot.re_question_pending
    })
}

/// Returns the latest recorded test summary for the active slot.
pub fn finish_preview(session: &Session) -> Option<TestSummary> {
    session.current_slot().and_then(|slot| slot.latest_test)
}

/// Render-ready candidate work data with permitted fields only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkView {
    pub question_label: Option<String>,
    pub question_index: Option<usize>,
    pub question_title: Option<String>,
    pub statement: Option<String>,
    pub approaches: Vec<ApproachOptionView>,
    pub approaches_available: bool,
    pub selected_approach: Option<String>,
    pub latest_test: Option<String>,
    pub latest_test_summary: Option<TestSummary>,
    pub can_request_requestion: bool,
    pub re_question_pending: bool,
    pub tests_unavailable: bool,
    pub tests_failed: bool,
    pub phase: WorkPhase,
    pub phase_line: String,
    pub has_code: bool,
}

fn truncated_statement(text: &str) -> String {
    let sanitized = sanitize_display(text);
    if sanitized.chars().count() > 4000 {
        let kept: String = sanitized.chars().take(4000).collect();
        format!("{kept} [truncated]")
    } else {
        sanitized
    }
}

/// Builds the candidate work view from permitted data only.
pub fn work_view(session: &Session, work: &CandidateWork) -> WorkView {
    let slot = session.current_slot();
    let approaches: Vec<ApproachOptionView> = work
        .approach_options()
        .into_iter()
        .map(|option| ApproachOptionView {
            title: sanitize_display(&option.title),
            summary: sanitize_display(&option.summary),
        })
        .collect();
    WorkView {
        question_label: slot
            .map(|s| sanitize_display(&format!("Q{}: {}", s.index + 1, s.question.title))),
        question_index: slot.map(|s| s.index),
        question_title: slot.map(|s| sanitize_display(&s.question.title)),
        statement: slot.and_then(|s| {
            work.statements
                .get(&s.question.title_slug)
                .map(|text| truncated_statement(text))
        }),
        approaches_available: work.approaches.is_some(),
        approaches,
        selected_approach: slot
            .and_then(|s| s.approach.as_ref())
            .map(|choice| match choice {
                ApproachChoice::Preset(n) => format!("preset {n}"),
                ApproachChoice::Other(text) => sanitize_display(text),
            }),
        latest_test: slot
            .and_then(|s| s.latest_test)
            .map(|t| format!("{} passed, {} failed", t.passed, t.failed)),
        latest_test_summary: slot.and_then(|s| s.latest_test),
        can_request_requestion: can_request_requestion(session),
        re_question_pending: slot.is_some_and(|s| s.re_question_pending),
        tests_unavailable: work.tests_unavailable,
        tests_failed: work.tests_failed,
        phase: work.phase,
        phase_line: work.phase.label().to_string(),
        has_code: !work.current_code.trim().is_empty(),
    }
}

/// Renders candidate work lines from permitted data only.
pub fn work_lines(session: &Session, work: &CandidateWork) -> Vec<String> {
    let view = work_view(session, work);
    let Some(label) = view.question_label.clone() else {
        return vec!["no active question.".to_string()];
    };
    let mut lines = vec![label];
    match &view.statement {
        Some(statement) => lines.push(statement.clone()),
        None => lines.push("statement is not available yet.".to_string()),
    }
    if view.approaches_available {
        for (index, option) in view.approaches.iter().enumerate() {
            lines.push(format!(
                "{}: {} - {}",
                index + 1,
                option.title,
                option.summary
            ));
        }
        lines.push("other: describe your own approach".to_string());
    } else {
        lines.push("approaches are not available yet.".to_string());
    }
    lines.push(format!(
        "selected approach: {}",
        view.selected_approach.as_deref().unwrap_or("none")
    ));
    lines.push(format!(
        "latest test: {}",
        view.latest_test.as_deref().unwrap_or("no test run yet")
    ));
    if view.tests_unavailable {
        lines.push("tests are not available yet.".to_string());
    }
    if view.tests_failed {
        lines.push("tests could not be run; ask the expert if it persists.".to_string());
    }
    if view.re_question_pending {
        lines.push("re-question requested; waiting for the expert.".to_string());
    } else if view.can_request_requestion {
        lines.push("re-question: available".to_string());
    } else {
        lines.push("re-question: not available".to_string());
    }
    // Every dynamic field arrives sanitized via `work_view`; the
    // remaining lines are literals, so no second pass is needed.
    lines.push(view.phase_line.clone());
    lines
}
