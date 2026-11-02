use std::net::Ipv4Addr;

use pairproof::sandbox::preflight::{
    compute_preflight, CheckKind, CheckStatus, PreflightInputs, PreflightReport,
};
use pairproof::tui::expert::{can_proceed, expert_ssh_hint, preflight_view};

fn ip(a: u8, b: u8, c: u8, d: u8) -> Ipv4Addr {
    Ipv4Addr::new(a, b, c, d)
}

fn healthy() -> PreflightInputs {
    PreflightInputs {
        docker_version: Some("29.9.2".to_string()),
        daemon_reachable: true,
        image_present: true,
        port: 2222,
        port_free: true,
        local_ips: vec![ip(8, 8, 8, 8)],
        external_ip: Some(ip(8, 8, 8, 8)),
        mutagen_version: Some("0.18.1".to_string()),
    }
}

fn status(report: &PreflightReport, kind: CheckKind) -> CheckStatus {
    report
        .checks
        .iter()
        .find(|c| c.kind == kind)
        .expect("check exists")
        .status
}

#[test]
fn preflight_missing_docker_blocks_start() {
    let mut inputs = healthy();
    inputs.docker_version = None;
    let report = compute_preflight(&inputs);
    assert_eq!(
        status(&report, CheckKind::DockerInstalled),
        CheckStatus::Fail
    );
    assert!(!report.can_start());
    let view = preflight_view(&report);
    assert!(!view.can_start);
    assert!(!can_proceed(&report));
    assert_eq!(view.failures, report.failures());
    assert!(view.failures >= 1);
    assert!(view.lines.iter().any(|l| l.starts_with("[FAIL]")));
}

#[test]
fn preflight_warning_can_proceed_only_when_allowed() {
    // Warning only (missing image) still allows start.
    let mut inputs = healthy();
    inputs.image_present = false;
    let report = compute_preflight(&inputs);
    assert!(report.can_start());
    assert!(report.warnings() >= 1);
    assert!(can_proceed(&report));
    let view = preflight_view(&report);
    assert!(view.can_start);
    assert_eq!(view.warnings, report.warnings());
    assert!(view.lines.iter().any(|l| l.starts_with("[WARN]")));

    // Any failure blocks even when warnings also exist.
    let mut inputs = healthy();
    inputs.image_present = false;
    inputs.port_free = false;
    let report = compute_preflight(&inputs);
    assert!(!report.can_start());
    assert!(!can_proceed(&report));
    assert!(!preflight_view(&report).can_start);
}

#[test]
fn preflight_view_preserves_guidance() {
    let mut inputs = healthy();
    inputs.docker_version = None;
    let report = compute_preflight(&inputs);
    let view = preflight_view(&report);
    let joined = view.lines.join("\n");
    for check in &report.checks {
        assert!(joined.contains(&check.detail), "missing detail in {joined}");
        if let Some(guidance) = &check.guidance {
            assert!(
                joined.contains(guidance.as_str()),
                "missing guidance in {joined}"
            );
        }
    }
    // Guidance sits on its own indented line, labels are readable,
    // and no Debug enum names or over-long lines leak through.
    assert!(view.lines.iter().any(|l| l.trim_start().starts_with("-> ")));
    assert!(joined.contains("Connection"), "{joined}");
    assert!(!joined.contains("Public IP"), "{joined}");
    assert!(!joined.contains("CGNAT"), "{joined}");
    assert!(!joined.contains("ExternalIp"), "{joined}");
    assert!(!joined.contains("CgnatSuspicion"), "{joined}");
    assert!(!joined.contains("ask your ISP"), "{joined}");
    assert!(!joined.contains("port forwarding is required"), "{joined}");
    assert!(
        view.lines.iter().all(|l| l.chars().count() <= 84),
        "{joined}"
    );
    // The passed Connection line fits on one line, never wrapped.
    assert!(
        view.lines
            .iter()
            .any(|l| l.starts_with("[PASS] Connection:") && l.contains("direct connection ready")),
        "{joined}"
    );
    assert!(view
        .lines
        .iter()
        .any(|l| l.contains("install Docker and ensure it is on PATH")));
}

#[test]
fn preflight_view_prefixes_match_status() {
    let report = compute_preflight(&healthy());
    let view = preflight_view(&report);
    assert!(view.can_start);
    assert_eq!(view.failures, 0);
    for check in &report.checks {
        let expected = match check.status {
            CheckStatus::Pass => "[PASS]",
            CheckStatus::Warn => "[WARN]",
            CheckStatus::Fail => "[FAIL]",
        };
        assert!(
            view.lines.iter().any(|l| l.starts_with(expected)),
            "no {expected} line in {:?}",
            view.lines
        );
    }
}

#[test]
fn preflight_all_pass_can_start() {
    let report = compute_preflight(&healthy());
    assert!(report.can_start());
    let view = preflight_view(&report);
    assert!(view.can_start);
    assert_eq!(view.failures, 0);
    assert!(view
        .lines
        .iter()
        .all(|l| l.starts_with("[PASS]") || l.starts_with("[WARN]")));
}

#[test]
fn preflight_port_busy_blocks_start() {
    let mut inputs = healthy();
    inputs.port_free = false;
    let report = compute_preflight(&inputs);
    assert_eq!(status(&report, CheckKind::PortAvailable), CheckStatus::Fail);
    let view = preflight_view(&report);
    assert!(!view.can_start);
    assert!(view
        .lines
        .iter()
        .any(|l| l.starts_with("[FAIL]") && l.contains("Port:")));
}

#[test]
fn expert_ssh_hint_is_expert_guidance() {
    let hint = expert_ssh_hint("Permission denied (publickey).");
    assert!(hint.contains("ssh <target>"));
    assert!(!hint.contains("Permission denied (publickey)"));
    let unknown = expert_ssh_hint("");
    assert!(unknown.contains("ssh <target>"));
}
