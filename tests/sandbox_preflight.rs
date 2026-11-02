use std::net::{Ipv4Addr, TcpListener};

use pairproof::sandbox::preflight::{
    classify_ipv4, compute_preflight, first_free_port, gather_preflight, port_is_free,
    primary_local_ipv4, CheckKind, CheckStatus, IpClass, PreflightInputs, PreflightReport,
};

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
        local_ips: vec![ip(203, 0, 113, 10)],
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
fn classify_ipv4_all_classes() {
    assert_eq!(classify_ipv4(ip(127, 0, 0, 1)), IpClass::Loopback);
    assert_eq!(classify_ipv4(ip(0, 0, 0, 0)), IpClass::Unspecified);
    assert_eq!(classify_ipv4(ip(10, 1, 2, 3)), IpClass::Private);
    assert_eq!(classify_ipv4(ip(172, 16, 0, 1)), IpClass::Private);
    assert_eq!(classify_ipv4(ip(172, 31, 255, 255)), IpClass::Private);
    assert_eq!(classify_ipv4(ip(172, 32, 0, 1)), IpClass::Public);
    assert_eq!(classify_ipv4(ip(192, 168, 1, 1)), IpClass::Private);
    assert_eq!(classify_ipv4(ip(100, 64, 0, 1)), IpClass::Cgnat);
    assert_eq!(classify_ipv4(ip(100, 127, 255, 255)), IpClass::Cgnat);
    assert_eq!(classify_ipv4(ip(100, 128, 0, 1)), IpClass::Public);
    assert_eq!(classify_ipv4(ip(100, 63, 0, 1)), IpClass::Public);
    assert_eq!(classify_ipv4(ip(169, 254, 1, 1)), IpClass::LinkLocal);
    assert_eq!(classify_ipv4(ip(8, 8, 8, 8)), IpClass::Public);
}

#[test]
fn preflight_missing_docker_hard_fails() {
    let mut inputs = healthy();
    inputs.docker_version = None;
    let report = compute_preflight(&inputs);
    assert_eq!(
        status(&report, CheckKind::DockerInstalled),
        CheckStatus::Fail
    );
    assert_eq!(status(&report, CheckKind::DockerDaemon), CheckStatus::Fail);
    assert_eq!(status(&report, CheckKind::SandboxImage), CheckStatus::Warn);
    assert!(!report.can_start());
    assert_eq!(report.failures(), 2);
    assert_eq!(report.checks.len(), 7);
}

#[test]
fn preflight_daemon_down_fails_but_image_missing_warns() {
    let mut inputs = healthy();
    inputs.daemon_reachable = false;
    inputs.image_present = false;
    let report = compute_preflight(&inputs);
    assert_eq!(
        status(&report, CheckKind::DockerInstalled),
        CheckStatus::Pass
    );
    assert_eq!(status(&report, CheckKind::DockerDaemon), CheckStatus::Fail);
    assert_eq!(status(&report, CheckKind::SandboxImage), CheckStatus::Warn);
    assert!(!report.can_start());
    let mut inputs = healthy();
    inputs.image_present = false;
    let report = compute_preflight(&inputs);
    assert_eq!(status(&report, CheckKind::SandboxImage), CheckStatus::Warn);
    assert!(report.can_start());
}

#[test]
fn preflight_port_busy_fails() {
    let mut inputs = healthy();
    inputs.port_free = false;
    let report = compute_preflight(&inputs);
    assert_eq!(status(&report, CheckKind::PortAvailable), CheckStatus::Fail);
    assert!(!report.can_start());
}

#[test]
fn preflight_cgnat_ip_warns_suspicion() {
    let mut inputs = healthy();
    inputs.local_ips = vec![ip(100, 64, 0, 1)];
    let report = compute_preflight(&inputs);
    assert_eq!(
        status(&report, CheckKind::CgnatSuspicion),
        CheckStatus::Warn
    );
    assert_eq!(status(&report, CheckKind::ExternalIp), CheckStatus::Warn);
    assert!(report.can_start());
    assert!(report.warnings() >= 2);
}

#[test]
fn preflight_all_private_warns_cgnat_possible() {
    let mut inputs = healthy();
    inputs.local_ips = vec![ip(192, 168, 1, 5), ip(10, 0, 0, 2)];
    let report = compute_preflight(&inputs);
    assert_eq!(
        status(&report, CheckKind::CgnatSuspicion),
        CheckStatus::Warn
    );
    assert_eq!(status(&report, CheckKind::ExternalIp), CheckStatus::Warn);
}

#[test]
fn preflight_public_ip_passes_network_checks() {
    let report = compute_preflight(&healthy());
    assert_eq!(status(&report, CheckKind::ExternalIp), CheckStatus::Pass);
    assert_eq!(
        status(&report, CheckKind::CgnatSuspicion),
        CheckStatus::Pass
    );
    assert!(report.can_start());
    assert_eq!(report.failures(), 0);
    assert_eq!(report.warnings(), 0);
}

#[test]
fn preflight_no_interface_and_no_mutagen_warn() {
    let mut inputs = healthy();
    inputs.local_ips = vec![];
    inputs.mutagen_version = None;
    let report = compute_preflight(&inputs);
    assert_eq!(status(&report, CheckKind::ExternalIp), CheckStatus::Warn);
    assert_eq!(
        status(&report, CheckKind::CgnatSuspicion),
        CheckStatus::Warn
    );
    assert_eq!(
        status(&report, CheckKind::MutagenInstalled),
        CheckStatus::Warn
    );
    assert!(report.can_start());
}

#[test]
fn port_is_free_detects_bound_port() {
    let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).expect("bind ephemeral");
    let bound = listener.local_addr().expect("local addr").port();
    assert!(!port_is_free(bound));
    assert!(!port_is_free(0));
    drop(listener);
    assert!(port_is_free(bound));
}

#[test]
fn first_free_port_all_busy_none_first_busy_next() {
    assert_eq!(first_free_port(2222..2322, |_| false), None);
    assert_eq!(first_free_port(2222..2322, |p| p != 2222), Some(2223));
    assert_eq!(first_free_port(2222..2322, |_| true), Some(2222));
    assert_eq!(first_free_port(2222..2322, |p| p == 2322), None);
}

#[test]
fn primary_local_ipv4_never_unspecified_when_present() {
    if let Some(found) = primary_local_ipv4() {
        assert_ne!(found, Ipv4Addr::UNSPECIFIED);
    }
}

#[tokio::test]
async fn gather_preflight_returns_seven_checks() {
    let report = gather_preflight(0).await;
    assert_eq!(report.checks.len(), 7);
    let kinds: Vec<CheckKind> = report.checks.iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        vec![
            CheckKind::DockerInstalled,
            CheckKind::DockerDaemon,
            CheckKind::SandboxImage,
            CheckKind::PortAvailable,
            CheckKind::ExternalIp,
            CheckKind::CgnatSuspicion,
            CheckKind::MutagenInstalled,
        ]
    );
    assert_eq!(status(&report, CheckKind::PortAvailable), CheckStatus::Fail);
    assert!(!report.can_start());
}
