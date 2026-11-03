use serde::{Deserialize, Serialize};

/// Canonical session-id rule for every boundary (core, sandbox, sync, artifacts):
/// `[A-Za-z0-9_-]{1,40}`, rejecting leading `-` and `net-` (network-name collision).
/// Production ids `e{pid}x{ms}` conform. This is the single implementation;
/// other boundaries delegate here and map to their own error type.
pub fn validate_session_id(raw: &str) -> bool {
    !raw.is_empty()
        && !raw.starts_with('-')
        && !raw.starts_with("net-")
        && raw.len() <= 40
        && raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Errors returned by core session operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CoreError {
    #[error("session id must not be empty")]
    EmptySessionId,
    #[error("expected {expected} questions, got {got}")]
    InvalidQuestionCount { expected: usize, got: usize },
    #[error("question '{slug}' has wrong difficulty")]
    WrongDifficulty { slug: String },
    #[error("duplicate question slug: {0}")]
    DuplicateSlug(String),
    #[error("invalid question: {0}")]
    InvalidQuestion(String),
    #[error("operation not allowed in current state: {0}")]
    InvalidState(String),
    #[error("session is in a terminal state")]
    Terminal,
    #[error("expert questions are already set")]
    ExpertAlreadySet,
    #[error("expert questions are not set")]
    ExpertNotSet,
    #[error("extra question already added")]
    ExtraAlreadyAdded,
    #[error("question slot limit exceeded")]
    SlotLimitExceeded,
    #[error("no active question")]
    NoActiveQuestion,
    #[error("approach already selected")]
    ApproachAlreadySelected,
    #[error("no approach selected")]
    NoApproachSelected,
    #[error("invalid approach: {0}")]
    InvalidApproach(String),
    #[error("expected tokens must be greater than zero")]
    InvalidExpectedTokens,
    #[error("instruction must not be empty")]
    EmptyInstruction,
    #[error("re-question is not allowed for this question")]
    ReQuestionNotAllowed,
    #[error("re-question already used for this question")]
    ReQuestionAlreadyUsed,
    #[error("no re-question is pending")]
    ReQuestionNotPending,
    #[error("approved re-question requires a replacement question")]
    ReplacementRequired,
    #[error("replacement question difficulty must match the original")]
    ReplacementDifficultyMismatch,
    #[error("candidate has not connected")]
    CandidateNotConnected,
    #[error("candidate has already connected")]
    CandidateAlreadyConnected,
    #[error("a re-question is pending on the active question")]
    ReQuestionPending,
    #[error("candidate is disconnected")]
    CandidateDisconnected,
    #[error("paste flag size must be greater than zero")]
    InvalidPasteFlag,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Difficulty {
    Easy,
    Medium,
    Hard,
}

impl Difficulty {
    /// Returns the GraphQL filter value for this difficulty.
    pub fn as_filter(self) -> &'static str {
        match self {
            Difficulty::Easy => "EASY",
            Difficulty::Medium => "MEDIUM",
            Difficulty::Hard => "HARD",
        }
    }
}

/// Supported solution language for a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    Python,
    JavaScript,
    Sql,
}

impl Language {
    /// Returns the solution file extension without a dot.
    pub fn file_extension(self) -> &'static str {
        match self {
            Language::Python => "py",
            Language::JavaScript => "js",
            Language::Sql => "sql",
        }
    }

    /// Returns the LeetCode language slug.
    pub fn leetcode_slug(self) -> &'static str {
        match self {
            Language::Python => "python3",
            Language::JavaScript => "javascript",
            Language::Sql => "mysql",
        }
    }
}

/// Reference to a live LeetCode question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionRef {
    pub title_slug: String,
    pub title: String,
    pub difficulty: Difficulty,
    pub acceptance_pct: Option<u32>,
}

/// Origin of a question slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuestionSource {
    AutoEasy,
    Expert,
    ExpertExtra,
}

/// Lifecycle state of one question slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuestionStatus {
    Pending,
    Active,
    Finished,
    Skipped,
}

/// Candidate approach choice for the active question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApproachChoice {
    Preset(u8),
    Other(String),
}

impl ApproachChoice {
    /// Validates and normalizes this choice.
    pub fn validated(self) -> Result<Self, CoreError> {
        match self {
            ApproachChoice::Preset(n) if n <= 3 => Ok(ApproachChoice::Preset(n)),
            ApproachChoice::Preset(n) => Err(CoreError::InvalidApproach(format!("preset {n}"))),
            ApproachChoice::Other(text) => {
                let trimmed = text.trim().to_string();
                if trimmed.is_empty() {
                    Err(CoreError::InvalidApproach("empty other text".to_string()))
                } else {
                    Ok(ApproachChoice::Other(trimmed))
                }
            }
        }
    }
}

/// Latest test-run summary for one question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestSummary {
    pub passed: u32,
    pub failed: u32,
    pub at_ms: u64,
}

/// One ordered question in a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionSlot {
    pub index: usize,
    pub source: QuestionSource,
    pub question: QuestionRef,
    pub status: QuestionStatus,
    pub approach: Option<ApproachChoice>,
    pub locked_expected_tokens: Option<u64>,
    pub actual_tokens: u64,
    pub re_question_used: bool,
    pub re_question_pending: bool,
    pub started_at_ms: Option<u64>,
    pub finished_at_ms: Option<u64>,
    pub latest_test: Option<TestSummary>,
    pub paste_flags: u32,
}

impl QuestionSlot {
    /// Creates a pending slot for `question`.
    pub fn new(index: usize, source: QuestionSource, question: QuestionRef) -> Self {
        Self {
            index,
            source,
            question,
            status: QuestionStatus::Pending,
            approach: None,
            locked_expected_tokens: None,
            actual_tokens: 0,
            re_question_used: false,
            re_question_pending: false,
            started_at_ms: None,
            finished_at_ms: None,
            latest_test: None,
            paste_flags: 0,
        }
    }
}

/// Lifecycle state of a whole session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    Setup,
    WaitingForCandidate,
    InProgress,
    WaitingForExpert,
    Scoring,
    Completed,
    EndedByExpert,
    TechnicalFailure,
}

impl SessionStatus {
    /// Returns true when no further mutations are allowed.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            SessionStatus::Completed
                | SessionStatus::EndedByExpert
                | SessionStatus::TechnicalFailure
        )
    }
}

pub(crate) fn ensure(cond: bool, err: CoreError) -> Result<(), CoreError> {
    if cond {
        Ok(())
    } else {
        Err(err)
    }
}

pub(crate) fn expect_count(got: usize, expected: usize) -> Result<(), CoreError> {
    ensure(
        got == expected,
        CoreError::InvalidQuestionCount { expected, got },
    )
}

/// Checks non-empty fields and optional expected difficulty.
pub(crate) fn validate_question(
    q: &QuestionRef,
    diff: Option<Difficulty>,
) -> Result<(), CoreError> {
    ensure(
        !q.title_slug.trim().is_empty() && !q.title.trim().is_empty(),
        CoreError::InvalidQuestion(q.title_slug.clone()),
    )?;
    match diff {
        Some(d) if q.difficulty != d => Err(CoreError::WrongDifficulty {
            slug: q.title_slug.clone(),
        }),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::session::Session;
    use crate::core::test_support::*;

    #[test]
    fn language_maps_to_extension_and_slug() {
        assert_eq!(Language::Python.file_extension(), "py");
        assert_eq!(Language::Python.leetcode_slug(), "python3");
        assert_eq!(Language::JavaScript.file_extension(), "js");
        assert_eq!(Language::JavaScript.leetcode_slug(), "javascript");
        assert_eq!(Language::Sql.file_extension(), "sql");
        assert_eq!(Language::Sql.leetcode_slug(), "mysql");
    }

    #[test]
    fn approach_validation_rejects_bad_preset_and_blank_other() {
        assert!(ApproachChoice::Preset(3).validated().is_ok());
        assert!(ApproachChoice::Preset(4).validated().is_err());
        assert!(ApproachChoice::Other("  ".to_string()).validated().is_err());
        assert_eq!(
            ApproachChoice::Other("  dp  ".to_string()).validated(),
            Ok(ApproachChoice::Other("dp".to_string()))
        );
    }

    #[test]
    fn new_requires_five_unique_easy_and_non_empty_id() {
        let mk = |id: &str, qs: Vec<QuestionRef>| Session::new(id.into(), Language::Python, qs, 0);
        assert!(mk("s", qs("e", 4, Difficulty::Easy)).is_err());
        assert!(mk("  ", qs("e", 5, Difficulty::Easy)).is_err());
        let mut wrong = qs("e", 5, Difficulty::Easy);
        wrong[0] = q("e0", Difficulty::Medium);
        assert!(mk("s", wrong).is_err());
        let mut dup = qs("e", 5, Difficulty::Easy);
        dup[1] = q("e0", Difficulty::Easy);
        assert_eq!(mk("s", dup), Err(CoreError::DuplicateSlug("e0".into())));
    }

    #[test]
    fn expert_set_validates_count_difficulty_duplicates_and_once() {
        let mut s = new_session();
        assert!(s
            .set_expert_questions(
                qs("m", 4, Difficulty::Medium),
                qs("h", 2, Difficulty::Hard),
                1
            )
            .is_err());
        let mut bad = qs("m", 3, Difficulty::Medium);
        bad[0] = q("m0", Difficulty::Easy);
        assert!(s
            .set_expert_questions(bad, qs("h", 2, Difficulty::Hard), 1)
            .is_err());
        let mut dup = qs("m", 3, Difficulty::Medium);
        dup[0] = q("e0", Difficulty::Medium);
        assert!(s
            .set_expert_questions(dup, qs("h", 2, Difficulty::Hard), 1)
            .is_err());
        s.set_expert_questions(
            qs("m", 3, Difficulty::Medium),
            qs("h", 2, Difficulty::Hard),
            1,
        )
        .expect("expert");
        assert_eq!(s.slots().len(), 10);
        assert_eq!(
            s.set_expert_questions(
                qs("m", 3, Difficulty::Medium),
                qs("h", 2, Difficulty::Hard),
                2
            ),
            Err(CoreError::ExpertAlreadySet)
        );
    }

    #[test]
    fn extra_requires_expert_set_and_at_most_one() {
        let mut s = new_session();
        assert_eq!(
            s.add_extra_question(q("x0", Difficulty::Hard), 1),
            Err(CoreError::ExpertNotSet)
        );
        let mut s = with_expert();
        s.add_extra_question(q("x0", Difficulty::Easy), 5)
            .expect("extra");
        assert_eq!(s.slots().len(), 11);
        assert_eq!(
            s.add_extra_question(q("x1", Difficulty::Easy), 6),
            Err(CoreError::ExtraAlreadyAdded)
        );
    }
}
