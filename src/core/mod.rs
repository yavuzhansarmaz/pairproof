//! Interview flow state, session lifecycle, event log, and artifacts.

pub mod actions;
pub mod artifacts;
pub mod event_log;
pub mod events;
pub mod navigation;
pub mod records;
pub mod session;
#[cfg(test)]
pub(crate) mod test_support;
pub mod types;

pub use artifacts::{ArtifactError, ArtifactLayout, ContextSummary, ModelProvenance, TokenRecord};
pub use event_log::{read_events, EventLogError, EventLogWriter};
pub use events::{EventKind, SessionEvent};
pub use session::Session;
pub use types::{
    validate_session_id, ApproachChoice, CoreError, Difficulty, Language, QuestionRef,
    QuestionSlot, QuestionSource, QuestionStatus, SessionStatus, TestSummary,
};
