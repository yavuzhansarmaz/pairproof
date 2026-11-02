use super::process::CommandOutcome;

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

/// Maps a missing docker binary to the docker-specific error.
pub(crate) fn map_docker_not_found(err: SandboxError) -> SandboxError {
    match err {
        SandboxError::ExecutableNotFound { program } if program == "docker" => {
            SandboxError::DockerNotFound
        }
        other => other,
    }
}

/// Builds a bounded command-failure error from an outcome.
pub(crate) fn command_failed(operation: &'static str, outcome: &CommandOutcome) -> SandboxError {
    SandboxError::CommandFailed {
        operation,
        exit_code: outcome.exit_code,
        stderr: bounded_stderr(&outcome.stderr),
    }
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

    #[test]
    fn docker_missing_maps_to_docker_not_found() {
        let e = SandboxError::ExecutableNotFound {
            program: "docker".into(),
        };
        assert_eq!(map_docker_not_found(e), SandboxError::DockerNotFound);
    }

    #[test]
    fn other_program_passes_through() {
        let e = SandboxError::ExecutableNotFound {
            program: "mutagen".into(),
        };
        assert_eq!(map_docker_not_found(e.clone()), e);
    }

    #[test]
    fn other_variant_passes_through() {
        let timeout = SandboxError::Timeout {
            program: "docker".into(),
            timeout_ms: 1,
        };
        for e in [SandboxError::DockerNotFound, timeout] {
            assert_eq!(map_docker_not_found(e.clone()), e);
        }
    }

    #[test]
    fn command_failed_bounds_stderr_and_keeps_exit_code() {
        let outcome = CommandOutcome {
            exit_code: Some(3),
            stdout: Vec::new(),
            stderr: vec![b'x'; 900],
            stdout_truncated: false,
            stderr_truncated: false,
        };
        assert_eq!(
            command_failed("docker_version", &outcome),
            SandboxError::CommandFailed {
                operation: "docker_version",
                exit_code: Some(3),
                stderr: "x".repeat(512),
            }
        );
    }
}
