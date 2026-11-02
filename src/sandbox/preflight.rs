use std::net::{IpAddr, Ipv4Addr, TcpListener, UdpSocket};
use std::time::Duration;

use super::command::{docker_info, docker_version, image_inspect};
use super::image::effective_image_tag;
use super::parse::{parse_docker_server_version, parse_docker_version};
use super::process::{run_bounded, CommandSpec, DEFAULT_MAX_OUTPUT};

const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// Classification of an IPv4 address for direct-mode reachability checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpClass {
    Loopback,
    Private,
    Cgnat,
    LinkLocal,
    Public,
    Unspecified,
}

/// Classifies an IPv4 address into a reachability bucket.
pub fn classify_ipv4(ip: Ipv4Addr) -> IpClass {
    if ip.is_unspecified() {
        IpClass::Unspecified
    } else if ip.is_loopback() {
        IpClass::Loopback
    } else if ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]) {
        IpClass::Cgnat
    } else if ip.is_private() {
        IpClass::Private
    } else if ip.is_link_local() {
        IpClass::LinkLocal
    } else {
        IpClass::Public
    }
}

/// Which preflight condition a check describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckKind {
    DockerInstalled,
    DockerDaemon,
    SandboxImage,
    PortAvailable,
    ExternalIp,
    CgnatSuspicion,
    MutagenInstalled,
}

/// Outcome severity of one preflight check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

/// One preflight result with an optional remediation hint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightCheck {
    pub kind: CheckKind,
    pub status: CheckStatus,
    pub detail: String,
    pub guidance: Option<String>,
}

/// Pure inputs for `compute_preflight`, gathered by `gather_preflight`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightInputs {
    pub docker_version: Option<String>,
    pub daemon_reachable: bool,
    pub image_present: bool,
    pub port: u16,
    pub port_free: bool,
    pub local_ips: Vec<Ipv4Addr>,
    pub mutagen_version: Option<String>,
}

/// Outcome of all preflight checks in a fixed order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightReport {
    pub checks: Vec<PreflightCheck>,
}

impl PreflightReport {
    /// Returns true when no check failed.
    pub fn can_start(&self) -> bool {
        self.failures() == 0
    }

    pub fn failures(&self) -> usize {
        self.count(CheckStatus::Fail)
    }

    pub fn warnings(&self) -> usize {
        self.count(CheckStatus::Warn)
    }

    fn count(&self, status: CheckStatus) -> usize {
        self.checks.iter().filter(|c| c.status == status).count()
    }
}

fn check(
    kind: CheckKind,
    status: CheckStatus,
    detail: impl Into<String>,
    guidance: Option<&str>,
) -> PreflightCheck {
    PreflightCheck {
        kind,
        status,
        detail: detail.into(),
        guidance: guidance.map(str::to_string),
    }
}

/// Computes the seven preflight checks from pure inputs.
pub fn compute_preflight(inputs: &PreflightInputs) -> PreflightReport {
    let docker_missing = inputs.docker_version.is_none();
    let docker = match &inputs.docker_version {
        Some(version) => check(CheckKind::DockerInstalled, CheckStatus::Pass, version, None),
        None => check(
            CheckKind::DockerInstalled,
            CheckStatus::Fail,
            "docker not found; refusing to start",
            Some("install Docker and ensure it is on PATH"),
        ),
    };
    let daemon = if docker_missing {
        check(
            CheckKind::DockerDaemon,
            CheckStatus::Fail,
            "daemon not checked: docker missing",
            Some("install Docker, then start the Docker daemon"),
        )
    } else if inputs.daemon_reachable {
        check(
            CheckKind::DockerDaemon,
            CheckStatus::Pass,
            "daemon reachable",
            None,
        )
    } else {
        check(
            CheckKind::DockerDaemon,
            CheckStatus::Fail,
            "docker daemon unreachable",
            Some("start the Docker daemon"),
        )
    };
    let image = if docker_missing {
        check(
            CheckKind::SandboxImage,
            CheckStatus::Warn,
            "sandbox image presence unknown: docker missing",
            None,
        )
    } else if inputs.image_present {
        check(
            CheckKind::SandboxImage,
            CheckStatus::Pass,
            "sandbox image present",
            None,
        )
    } else {
        check(
            CheckKind::SandboxImage,
            CheckStatus::Warn,
            "sandbox image missing; it will be built on first use",
            None,
        )
    };
    let port = if inputs.port_free {
        check(
            CheckKind::PortAvailable,
            CheckStatus::Pass,
            format!("port {} is free", inputs.port),
            None,
        )
    } else {
        check(
            CheckKind::PortAvailable,
            CheckStatus::Fail,
            format!("port {} is busy", inputs.port),
            Some("choose another port or stop the process using this one"),
        )
    };
    let classes: Vec<IpClass> = inputs
        .local_ips
        .iter()
        .map(|ip| classify_ipv4(*ip))
        .collect();
    let public: Vec<String> = inputs
        .local_ips
        .iter()
        .filter(|ip| classify_ipv4(**ip) == IpClass::Public)
        .map(ToString::to_string)
        .collect();
    let has_non_loopback = classes
        .iter()
        .any(|c| !matches!(c, IpClass::Loopback | IpClass::Unspecified));
    let external_ip = if !public.is_empty() {
        check(
            CheckKind::ExternalIp,
            CheckStatus::Pass,
            format!("public interface address: {}", public.join(", ")),
            None,
        )
    } else if has_non_loopback {
        check(
            CheckKind::ExternalIp,
            CheckStatus::Warn,
            "no public interface address; behind NAT, port forwarding is required",
            Some("direct mode needs a public IP and a forwarded port"),
        )
    } else {
        check(
            CheckKind::ExternalIp,
            CheckStatus::Warn,
            "no external interface found",
            Some("direct mode needs a public IP and a forwarded port"),
        )
    };
    let cgnat = if classes.contains(&IpClass::Cgnat) {
        check(
            CheckKind::CgnatSuspicion,
            CheckStatus::Warn,
            "CGNAT address range detected; direct mode likely fails",
            Some("ask your ISP for a public IP or use another network"),
        )
    } else if public.is_empty() {
        check(
            CheckKind::CgnatSuspicion,
            CheckStatus::Warn,
            "all interfaces are private; CGNAT behind the router is possible",
            Some("verify the router WAN address is a public IP"),
        )
    } else {
        check(
            CheckKind::CgnatSuspicion,
            CheckStatus::Pass,
            "no CGNAT signs on local interfaces",
            None,
        )
    };
    let mutagen = match &inputs.mutagen_version {
        Some(version) => check(
            CheckKind::MutagenInstalled,
            CheckStatus::Pass,
            version,
            None,
        ),
        None => check(
            CheckKind::MutagenInstalled,
            CheckStatus::Warn,
            "mutagen not found; it is needed on the candidate machine for sync",
            Some("install Mutagen on the candidate machine"),
        ),
    };
    PreflightReport {
        checks: vec![docker, daemon, image, port, external_ip, cgnat, mutagen],
    }
}

/// Returns true when `port` can be bound on all interfaces. Port 0 is never free.
pub fn port_is_free(port: u16) -> bool {
    port != 0 && TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).is_ok()
}

/// Returns the first port in `range` for which `is_free` holds, or `None`
/// when every port in the range is busy. Iteration follows the range's
/// own bounds, so a `start..end` range never probes `end`.
pub fn first_free_port(
    range: impl IntoIterator<Item = u16>,
    is_free: impl Fn(u16) -> bool,
) -> Option<u16> {
    range.into_iter().find(|port| is_free(*port))
}

/// Finds the primary outbound IPv4 via a UDP connect, which sends no packets.
pub fn primary_local_ipv4() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(8, 8, 8, 8), 80)).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

/// Runs one probe and returns its stdout text when it succeeds.
async fn probe_stdout(spec: &CommandSpec) -> Option<String> {
    let out = run_bounded(spec, None, PROBE_TIMEOUT, DEFAULT_MAX_OUTPUT)
        .await
        .ok()?;
    out.success().then(|| out.stdout_text())
}

/// Gathers local facts with bounded probes, then computes the report.
pub async fn gather_preflight(port: u16) -> PreflightReport {
    let docker_version = match probe_stdout(&docker_version()).await {
        Some(text) => parse_docker_version(&text)
            .ok()
            .or_else(|| text.lines().next().map(|l| l.trim().to_string()))
            .filter(|v| !v.is_empty()),
        None => None,
    };
    let daemon_reachable = match probe_stdout(&docker_info()).await {
        Some(text) => parse_docker_server_version(&text).is_ok(),
        None => false,
    };
    let image_present = match image_inspect(&effective_image_tag()) {
        Ok(spec) => probe_stdout(&spec).await.is_some(),
        Err(_) => false,
    };
    let mutagen_spec = CommandSpec::new("mutagen", ["version"]);
    let mutagen_version = probe_stdout(&mutagen_spec)
        .await
        .and_then(|text| text.lines().next().map(|l| l.trim().to_string()))
        .filter(|v| !v.is_empty());
    let local_ips = primary_local_ipv4()
        .filter(|ip| classify_ipv4(*ip) != IpClass::Loopback)
        .into_iter()
        .collect();
    compute_preflight(&PreflightInputs {
        docker_version,
        daemon_reachable,
        image_present,
        port,
        port_free: port_is_free(port),
        local_ips,
        mutagen_version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_free_port_all_busy_is_none() {
        assert_eq!(first_free_port(2222..2322, |_| false), None);
    }

    #[test]
    fn first_free_port_skips_busy_first() {
        assert_eq!(first_free_port(2222..2322, |p| p != 2222), Some(2223));
    }

    #[test]
    fn first_free_port_first_free_wins_and_end_is_exclusive() {
        assert_eq!(first_free_port(2222..2322, |_| true), Some(2222));
        assert_eq!(first_free_port(2222..2322, |p| p == 2321), Some(2321));
        assert_eq!(first_free_port(2222..2322, |p| p == 2322), None);
    }
}
