use crate::sandbox::SandboxError;

/// Errors returned by Mutagen sync command building and parsing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SyncError {
    #[error("mutagen executable not found")]
    MutagenNotFound,
    #[error("command failed during {operation} (exit {exit_code:?}): {stderr}")]
    CommandFailed {
        operation: &'static str,
        exit_code: Option<i32>,
        stderr: String,
    },
    #[error("command timed out after {timeout_ms}ms: {program}")]
    Timeout { program: String, timeout_ms: u64 },
    #[error("invalid session id: {0}")]
    InvalidSessionId(String),
    #[error("invalid sync endpoint: {0}")]
    InvalidEndpoint(String),
    #[error("parse error during {operation}: {detail}")]
    ParseError {
        operation: &'static str,
        detail: String,
    },
    #[error(transparent)]
    Sandbox(#[from] SandboxError),
}
