/// Errors returned by sandbox validation, process, and parsing operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SandboxError {
    #[error("executable not found: {program}")]
    ExecutableNotFound { program: String },
    #[error("docker executable not found")]
    DockerNotFound,
    #[error("docker daemon unavailable: {stderr}")]
    DaemonUnavailable { stderr: String },
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
    #[error("invalid workspace path: {0}")]
    InvalidWorkspacePath(String),
    #[error("path escapes workspace: {0}")]
    PathEscape(String),
    #[error("invalid command: {0}")]
    InvalidCommand(String),
    #[error("parse error during {operation}: {detail}")]
    ParseError {
        operation: &'static str,
        detail: String,
    },
    #[error("file too large: {len} bytes exceeds {max} bytes")]
    FileTooLarge { len: usize, max: usize },
    #[error("container file not found: {0}")]
    ContainerFileNotFound(String),
    #[error("io error: {0}")]
    Io(String),
}

/// Converts raw stderr bytes to a lossy string truncated to 512 characters.
pub fn bounded_stderr(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).chars().take(512).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_stderr_is_lossy_and_truncated() {
        assert_eq!(bounded_stderr(b"oops"), "oops");
        let long = vec![b'a'; 900];
        assert_eq!(bounded_stderr(&long).chars().count(), 512);
        assert!(bounded_stderr(&[0xff, b'a']).contains('a'));
    }
}
