use crate::core::CoreError;

/// Errors surfaced by the orchestration layer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OrchestratorError {
    /// Wraps a core session error unchanged.
    #[error(transparent)]
    Core(#[from] CoreError),
    /// Candidate is already attached to the session.
    #[error("candidate already attached")]
    AlreadyAttached,
}
