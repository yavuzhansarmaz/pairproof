use pairproof::sync::classify_ssh_failure as classify_via_mod;
use pairproof::sync::failure::{classify_ssh_failure, SshFailureKind};

#[test]
fn ssh_failure_classification_has_safe_unknown_fallback() {
    for input in ["", "   ", "some unexpected ssh noise"] {
        let c = classify_ssh_failure(input);
        assert_eq!(c.kind, SshFailureKind::Unknown, "input: {input:?}");
        assert_eq!(c.candidate_message, "Sync problem \u{2014} ask the expert");
        assert!(c.expert_guidance.contains("ssh <target>"));
        assert!(c.expert_guidance.contains("works non-interactively"));
        assert!(!c.expert_guidance.is_empty());
    }
}

#[test]
fn ssh_unreachable_classified() {
    for input in [
        "ssh: connect to host 10.0.0.1: Network is unreachable",
        "No route to host",
        "host unreachable",
    ] {
        assert_eq!(
            classify_ssh_failure(input).kind,
            SshFailureKind::Unreachable,
            "input: {input}"
        );
    }
}

#[test]
fn ssh_auth_failed_classified() {
    for input in [
        "Permission denied (publickey).",
        "Authentication failed.",
        "permission denied, please try again",
    ] {
        assert_eq!(
            classify_ssh_failure(input).kind,
            SshFailureKind::AuthFailed,
            "input: {input}"
        );
    }
}

#[test]
fn ssh_host_key_mismatch_classified() {
    for input in [
        "Host key verification failed.",
        "REMOTE HOST IDENTIFICATION HAS CHANGED!",
    ] {
        assert_eq!(
            classify_ssh_failure(input).kind,
            SshFailureKind::HostKeyMismatch,
            "input: {input}"
        );
    }
}

#[test]
fn ssh_timeout_classified() {
    for input in [
        "Connection timed out",
        "ssh: connect timeout expired",
        "Operation timed out",
    ] {
        assert_eq!(
            classify_ssh_failure(input).kind,
            SshFailureKind::Timeout,
            "input: {input}"
        );
    }
}

#[test]
fn ssh_refused_classified() {
    assert_eq!(
        classify_ssh_failure("ssh: connect: Connection refused").kind,
        SshFailureKind::Refused
    );
}

#[test]
fn ssh_classification_case_insensitive() {
    assert_eq!(
        classify_ssh_failure("PERMISSION DENIED").kind,
        SshFailureKind::AuthFailed
    );
    assert_eq!(
        classify_ssh_failure("CONNECTION REFUSED").kind,
        SshFailureKind::Refused
    );
    assert_eq!(
        classify_ssh_failure("Host Is Unreachable").kind,
        SshFailureKind::Unreachable
    );
}

#[test]
fn ssh_candidate_message_never_leaks_details() {
    for input in [
        "Permission denied for user@203.0.113.5",
        "Connection refused 192.168.1.10:22 /home/user/.ssh",
        "Host key verification failed for example-host",
        "",
        "totally unknown failure at /srv/session-123",
    ] {
        let c = classify_ssh_failure(input);
        assert_eq!(c.candidate_message, "Sync problem \u{2014} ask the expert");
        assert!(!c.candidate_message.contains("203.0.113"));
        assert!(!c.candidate_message.contains("/home"));
        assert!(!c.candidate_message.contains("session"));
        assert!(!c.candidate_message.contains("ssh"));
        assert!(!c.candidate_message.contains("192.168"));
    }
}

#[test]
fn ssh_expert_guidance_has_no_secret() {
    let c = classify_ssh_failure("Permission denied (password= hunter2 key= abc)");
    let dbg = format!("{:?}", c);
    assert!(!dbg.contains("hunter2"));
    assert!(!c.expert_guidance.contains("hunter2"));
    assert!(c.expert_guidance.contains("ssh <target>"));
}

#[test]
fn ssh_reexport_via_sync_mod() {
    let c = classify_via_mod("Connection refused");
    assert_eq!(c.kind, SshFailureKind::Refused);
}
