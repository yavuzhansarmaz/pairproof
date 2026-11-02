//! Per-question grading integration and the final breakdown.
//! Skipped questions are never sent for grading, missing inputs are
//! passed as empty strings, and the final rating is delegated to
//! `crate::scoring::final_rating` exactly once per call.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;

use crate::core::{ArtifactError, CoreError, EventLogError, Session, SessionStatus};
use crate::llm::grading::{grade_questions, scorable_slots, QuestionScoringInput};
use crate::llm::MetaClient;
use crate::orchestration::error::OrchestratorError;
use crate::orchestration::session::Orchestrator;
use crate::scoring::{final_rating, ApproachQuality, FinalBreakdown, GradeOutcome, ScoringError};

/// Errors from finalization. Payloads are redacted summaries and
/// never carry an API key, connection secret, or raw paste text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FinalizeError {
    /// Wraps a core session error unchanged.
    #[error("core error: {0}")]
    Core(#[from] CoreError),
    /// Redacted large-language-model failure summary.
    #[error("llm error: {0}")]
    Llm(String),
    /// Wraps a scoring validation or merge error unchanged.
    #[error("scoring error: {0}")]
    Scoring(#[from] ScoringError),
    /// Redacted artifact persistence failure summary.
    #[error("artifact error: {0}")]
    Artifact(String),
    /// Redacted I/O failure summary.
    #[error("io error: {0}")]
    Io(String),
    /// The session was already finalized; nothing was rewritten.
    #[error("session already finalized")]
    AlreadyFinalized,
}

impl From<OrchestratorError> for FinalizeError {
    fn from(err: OrchestratorError) -> Self {
        match err {
            OrchestratorError::Core(e) => Self::Core(e),
            OrchestratorError::AlreadyAttached => {
                Self::Core(CoreError::InvalidState("candidate already attached".into()))
            }
        }
    }
}

impl From<ArtifactError> for FinalizeError {
    fn from(err: ArtifactError) -> Self {
        match err {
            ArtifactError::Io(e) => Self::Io(e.to_string()),
            other => Self::Artifact(other.to_string()),
        }
    }
}

impl From<EventLogError> for FinalizeError {
    fn from(err: EventLogError) -> Self {
        Self::Artifact(err.to_string())
    }
}

impl From<std::io::Error> for FinalizeError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err.to_string())
    }
}

/// Boxed future returned by the narrow question-grader boundary.
pub type GradeFuture<'a> = Pin<Box<dyn Future<Output = Vec<GradeOutcome>> + Send + 'a>>;

/// Narrow boundary for grading every scorable question. Production
/// wraps `grade_questions`, whose per-question retry turns exhausted
/// failures into `GradeOutcome::NotScored`; fakes return per-slot
/// success or failure directly.
pub trait QuestionGrader: Send + Sync {
    /// Grades every supplied input, one outcome per input.
    fn grade_all<'a>(&'a self, inputs: &'a [QuestionScoringInput]) -> GradeFuture<'a>;
}

/// Production grader wrapping the real per-question grading calls.
#[derive(Debug)]
pub struct MetaQuestionGrader {
    client: MetaClient,
}

impl MetaQuestionGrader {
    /// Creates a grader wrapping `client`.
    pub fn new(client: MetaClient) -> Self {
        Self { client }
    }
}

impl QuestionGrader for MetaQuestionGrader {
    fn grade_all<'a>(&'a self, inputs: &'a [QuestionScoringInput]) -> GradeFuture<'a> {
        let inputs = inputs.to_vec();
        Box::pin(async move { grade_questions(&self.client, &inputs).await })
    }
}

/// Grades only the session's scorable (finished) slots. Skipped and
/// unfinished slots are never sent. A missing statement (looked up by
/// title slug) or missing code is passed as an empty string.
pub async fn grade_session(
    orch: &Orchestrator,
    grader: &dyn QuestionGrader,
    statements: &HashMap<String, String>,
    codes: &HashMap<usize, String>,
) -> Result<Vec<GradeOutcome>, FinalizeError> {
    let session = orch.session();
    let mut inputs = Vec::new();
    for slot_index in scorable_slots(session) {
        let Some(slot) = session.slots().iter().find(|s| s.index == slot_index) else {
            continue;
        };
        let statement = statements
            .get(&slot.question.title_slug)
            .cloned()
            .unwrap_or_default();
        let code = codes.get(&slot_index).cloned().unwrap_or_default();
        if let Some(input) =
            QuestionScoringInput::from_session(session, slot_index, &statement, &code)
        {
            inputs.push(input);
        }
    }
    Ok(grader.grade_all(&inputs).await)
}

/// Computes the final breakdown by delegating to `final_rating`
/// exactly once. Approach qualities are supplied by the caller.
pub fn compute_final(
    orch: &Orchestrator,
    qualities: &BTreeMap<usize, Vec<ApproachQuality>>,
    grades: &[GradeOutcome],
) -> Result<FinalBreakdown, FinalizeError> {
    Ok(final_rating(orch.session(), qualities, grades)?)
}

/// Returns whether the candidate may see the final breakdown.
/// Only the two evaluation-producing terminal states qualify; a
/// technical failure produces no candidate evaluation.
pub fn final_visible_to_candidate(status: SessionStatus) -> bool {
    matches!(
        status,
        SessionStatus::Completed | SessionStatus::EndedByExpert
    )
}

/// Returns the expert breakdown and the candidate breakdown. Both
/// roles get the same breakdown when it is visible to the candidate;
/// before a terminal-final state the candidate gets `None`.
pub fn both_roles_final(
    orch: &Orchestrator,
    qualities: &BTreeMap<usize, Vec<ApproachQuality>>,
    grades: &[GradeOutcome],
) -> Result<(FinalBreakdown, Option<FinalBreakdown>), FinalizeError> {
    let expert = compute_final(orch, qualities, grades)?;
    let candidate = if final_visible_to_candidate(orch.status()) {
        Some(expert.clone())
    } else {
        None
    };
    Ok((expert, candidate))
}

/// Elapsed wall time from the real lifecycle boundaries: the first
/// recorded event (session creation) to the terminal event, or to
/// `now_ms` while the session is still running.
pub fn session_elapsed_ms(session: &Session, now_ms: u64) -> u64 {
    let Some(first) = session.events().first() else {
        return 0;
    };
    let end = if session.status().is_terminal() {
        session.events().last().map_or(now_ms, |e| e.at_ms)
    } else {
        now_ms
    };
    end.saturating_sub(first.at_ms)
}
