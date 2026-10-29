//! Typed classification of SSH and sync stderr into safe guidance.

/// Category of an SSH failure inferred from stderr text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SshFailureKind {
    Unreachable,
    AuthFailed,
    HostKeyMismatch,
    Timeout,
    Refused,
    Unknown,
}

/// Classification result with expert-only and candidate-safe messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureClassification {
    pub kind: SshFailureKind,
    pub expert_guidance: String,
    pub candidate_message: String,
}

const CANDIDATE_MESSAGE: &str = "Sync problem \u{2014} ask the expert";

fn classification(kind: SshFailureKind, expert_guidance: &str) -> FailureClassification {
    FailureClassification {
        kind,
        expert_guidance: expert_guidance.to_string(),
        candidate_message: CANDIDATE_MESSAGE.to_string(),
    }
}

/// Classifies SSH stderr case-insensitively into a failure kind.
pub fn classify_ssh_failure(stderr: &str) -> FailureClassification {
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("host key verification failed")
        || lower.contains("remote host identification")
    {
        classification(
            SshFailureKind::HostKeyMismatch,
            "Host key verification failed; verify the remote host key, \
             update known_hosts after confirming the host, and check that \
             `ssh <target>` works non-interactively.",
        )
    } else if lower.contains("permission denied") || lower.contains("authentication") {
        classification(
            SshFailureKind::AuthFailed,
            "Authentication failed; check your SSH key or credentials; \
             check that `ssh <target>` works non-interactively.",
        )
    } else if lower.contains("timed out") || lower.contains("timeout") {
        classification(
            SshFailureKind::Timeout,
            "Connection timed out; check network and firewall rules; \
             check that `ssh <target>` works non-interactively.",
        )
    } else if lower.contains("connection refused") {
        classification(
            SshFailureKind::Refused,
            "Connection refused; check that the SSH service is running \
             on the remote; check that `ssh <target>` works \
             non-interactively.",
        )
    } else if lower.contains("unreachable")
        || lower.contains("no route to host")
        || lower.contains("network is unreachable")
    {
        classification(
            SshFailureKind::Unreachable,
            "cannot reach the remote; check that `ssh <target>` works \
             non-interactively and verify network reachability.",
        )
    } else {
        classification(
            SshFailureKind::Unknown,
            "Sync failed; check that `ssh <target>` works \
             non-interactively and review the sync status.",
        )
    }
}
