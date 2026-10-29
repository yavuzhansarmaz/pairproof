use std::time::{Duration, Instant};

use crate::connection::{Endpoint, EphemeralUsername, Fingerprint, Secret};

use super::command::{container_remove, container_run, exec_root, network_create, network_remove};
use super::error::{bounded_stderr, SandboxError};
use super::image::{check_docker_available, effective_image_tag, ensure_image};
use super::parse::parse_host_key_fingerprint;
use super::process::{run_bounded, CommandOutcome, CommandSpec, DEFAULT_MAX_OUTPUT};
use super::validate::{container_name, network_name};

/// Timeout for short docker and provisioning operations.
pub const OP_TIMEOUT: Duration = Duration::from_secs(30);

const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Names and endpoint of one session sandbox, derived from a validated session id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxHandle {
    session_id: String,
    container: String,
    network: String,
    endpoint_port: u16,
}

impl SandboxHandle {
    /// Derives sandbox names for a session id and endpoint without calling docker.
    pub fn ids(session_id: &str, endpoint: &Endpoint) -> Result<Self, SandboxError> {
        Ok(Self {
            session_id: session_id.to_string(),
            container: container_name(session_id)?,
            network: network_name(session_id)?,
            endpoint_port: endpoint.port(),
        })
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn container_name(&self) -> &str {
        &self.container
    }

    pub fn network_name(&self) -> &str {
        &self.network
    }

    pub fn endpoint_port(&self) -> u16 {
        self.endpoint_port
    }
}

/// Builds a `Vec<String>` argv from string slices.
fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| (*p).to_string()).collect()
}

/// Maps a docker spawn failure to the docker-specific error.
fn map_docker_not_found(err: SandboxError) -> SandboxError {
    match err {
        SandboxError::ExecutableNotFound { program } if program == "docker" => {
            SandboxError::DockerNotFound
        }
        other => other,
    }
}

/// Builds a bounded command-failure error from an outcome.
fn failed(operation: &'static str, outcome: &CommandOutcome) -> SandboxError {
    SandboxError::CommandFailed {
        operation,
        exit_code: outcome.exit_code,
        stderr: bounded_stderr(&outcome.stderr),
    }
}

/// Runs a command and returns its outcome, mapping a missing docker binary.
async fn run_outcome(
    spec: &CommandSpec,
    stdin: Option<&[u8]>,
) -> Result<CommandOutcome, SandboxError> {
    run_bounded(spec, stdin, OP_TIMEOUT, DEFAULT_MAX_OUTPUT)
        .await
        .map_err(map_docker_not_found)
}

/// Runs a command and requires a zero exit code.
async fn run_ok(
    spec: &CommandSpec,
    operation: &'static str,
    stdin: Option<&[u8]>,
) -> Result<CommandOutcome, SandboxError> {
    let outcome = run_outcome(spec, stdin).await?;
    if outcome.success() {
        Ok(outcome)
    } else {
        Err(failed(operation, &outcome))
    }
}

/// Polls a root `test` command until it succeeds or 30 seconds elapse.
async fn poll_root_test(
    handle: &SandboxHandle,
    test_argv: &[String],
    operation: &'static str,
    message: &str,
) -> Result<(), SandboxError> {
    let deadline = Instant::now() + OP_TIMEOUT;
    loop {
        let spec = exec_root(&handle.container, test_argv, false)?;
        let outcome = run_outcome(&spec, None).await?;
        if outcome.success() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(SandboxError::CommandFailed {
                operation,
                exit_code: None,
                stderr: message.to_string(),
            });
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Creates the per-session plain bridge network, treating an existing one as success.
pub async fn create_network(handle: &SandboxHandle) -> Result<(), SandboxError> {
    let spec = network_create(&handle.session_id)?;
    let outcome = run_outcome(&spec, None).await?;
    if outcome.success() || outcome.stderr_text().contains("already exists") {
        Ok(())
    } else {
        Err(failed("network_create", &outcome))
    }
}

/// Starts the session container from the sandbox image.
pub async fn start_container(handle: &SandboxHandle, tag: &str) -> Result<(), SandboxError> {
    let endpoint = Endpoint::new("127.0.0.1", handle.endpoint_port)
        .map_err(|e| SandboxError::InvalidCommand(e.to_string()))?;
    let spec = container_run(&handle.session_id, &endpoint, tag)?;
    run_ok(&spec, "container_run", None).await?;
    Ok(())
}

/// Waits until the per-container SSH host key has been generated.
pub async fn wait_for_host_key(handle: &SandboxHandle) -> Result<(), SandboxError> {
    poll_root_test(
        handle,
        &argv(&["test", "-f", "/etc/ssh/ssh_host_ed25519_key.pub"]),
        "wait_for_host_key",
        "host key did not appear within 30s",
    )
    .await
}

/// Installs the iptables lockdown: only loopback, established, and new TCP/22 pass.
#[rustfmt::skip]
pub async fn apply_network_lockdown(handle: &SandboxHandle) -> Result<(), SandboxError> {
    let rules: [&[&str]; 8] = [
        &["iptables", "-A", "INPUT", "-i", "lo", "-j", "ACCEPT"],
        &["iptables", "-A", "OUTPUT", "-o", "lo", "-j", "ACCEPT"],
        &["iptables", "-A", "INPUT", "-m", "conntrack", "--ctstate", "ESTABLISHED,RELATED", "-j", "ACCEPT"],
        &["iptables", "-A", "OUTPUT", "-m", "conntrack", "--ctstate", "ESTABLISHED,RELATED", "-j", "ACCEPT"],
        &["iptables", "-A", "INPUT", "-p", "tcp", "--dport", "22", "-m", "conntrack", "--ctstate", "NEW", "-j", "ACCEPT"],
        &["iptables", "-P", "INPUT", "DROP"],
        &["iptables", "-P", "OUTPUT", "DROP"],
        &["iptables", "-P", "FORWARD", "DROP"],
    ];
    for rule in rules {
        let spec = exec_root(&handle.container, &argv(rule), false)?;
        run_ok(&spec, "network_lockdown", None).await?;
    }
    Ok(())
}

/// Sets the ephemeral user password. The secret travels only over stdin.
async fn set_password(
    handle: &SandboxHandle,
    user: &EphemeralUsername,
    secret: &Secret,
) -> Result<(), SandboxError> {
    let stdin = format!("{}:{}\n", user.as_str(), secret.expose_hex());
    let spec = exec_root(&handle.container, &argv(&["chpasswd"]), true)?;
    run_ok(&spec, "chpasswd", Some(stdin.as_bytes())).await?;
    Ok(())
}

/// Rotates the ephemeral user password while the container keeps running.
pub async fn rotate_secret(
    handle: &SandboxHandle,
    user: &EphemeralUsername,
    new_secret: &Secret,
) -> Result<(), SandboxError> {
    set_password(handle, user, new_secret).await
}

/// Creates the ephemeral user, sets its password, and restricts sshd to it.
pub async fn provision_user(
    handle: &SandboxHandle,
    user: &EphemeralUsername,
    secret: &Secret,
) -> Result<(), SandboxError> {
    let name = user.as_str();
    let useradd = exec_root(
        &handle.container,
        &argv(&["useradd", "-m", "-d", "/workspace", "-s", "/bin/bash", name]),
        false,
    )?;
    let outcome = run_outcome(&useradd, None).await?;
    if !outcome.success() && !outcome.stderr_text().contains("already exists") {
        return Err(failed("useradd", &outcome));
    }
    let owner = format!("{name}:{name}");
    let chown = exec_root(
        &handle.container,
        &argv(&["chown", "-R", &owner, "/workspace"]),
        false,
    )?;
    run_ok(&chown, "chown_workspace", None).await?;
    set_password(handle, user, secret).await?;
    let mkdir = exec_root(
        &handle.container,
        &argv(&["mkdir", "-p", "/etc/ssh/sshd_config.d"]),
        false,
    )?;
    run_ok(&mkdir, "write_allow_users", None).await?;
    let tee = exec_root(
        &handle.container,
        &argv(&["tee", "/etc/ssh/sshd_config.d/10-pairproof.conf"]),
        true,
    )?;
    let allow = format!("AllowUsers {name}\n");
    run_ok(&tee, "write_allow_users", Some(allow.as_bytes())).await?;
    let reload = exec_root(&handle.container, &argv(&["kill", "-HUP", "1"]), false)?;
    run_ok(&reload, "sshd_reload", None).await?;
    Ok(())
}

/// Creates the session database and a socket-authenticated user for it.
#[rustfmt::skip]
pub async fn provision_database_user(handle: &SandboxHandle, user: &EphemeralUsername) -> Result<(), SandboxError> {
    poll_root_test(handle, &argv(&["test", "-S", "/run/mysqld/mysqld.sock"]), "provision_database_user", "mariadb socket did not appear within 30s").await?;
    let add = exec_root(&handle.container, &argv(&["useradd", "-M", "-s", "/usr/sbin/nologin", "ppcheck"]), false)?;
    let added = run_outcome(&add, None).await?;
    if !added.success() && !added.stderr_text().contains("already exists") { return Err(failed("useradd_ppcheck", &added)); }
    let name = user.as_str();
    let sql = format!("CREATE DATABASE IF NOT EXISTS pairproof;\nCREATE USER IF NOT EXISTS '{name}'@'localhost' IDENTIFIED VIA unix_socket;\nGRANT ALL PRIVILEGES ON pairproof.* TO '{name}'@'localhost';\nCREATE USER IF NOT EXISTS 'ppcheck'@'localhost' IDENTIFIED VIA unix_socket;\nGRANT ALL PRIVILEGES ON pp_runnable.* TO 'ppcheck'@'localhost';\n");
    let spec = exec_root(&handle.container, &argv(&["mariadb", "-u", "root"]), true)?;
    run_ok(&spec, "provision_database_user", Some(sql.as_bytes())).await?;
    Ok(())
}

/// Reads and validates the container SSH host key fingerprint.
pub async fn read_fingerprint(handle: &SandboxHandle) -> Result<Fingerprint, SandboxError> {
    let spec = exec_root(
        &handle.container,
        &argv(&["cat", "/etc/ssh/ssh_host_ed25519_key.pub"]),
        false,
    )?;
    let outcome = run_ok(&spec, "read_fingerprint", None).await?;
    parse_host_key_fingerprint(&outcome.stdout_text())
}

/// Runs the provisioning steps after the network exists.
async fn provision_steps(
    handle: &SandboxHandle,
    tag: &str,
    user: &EphemeralUsername,
    secret: &Secret,
) -> Result<Fingerprint, SandboxError> {
    start_container(handle, tag).await?;
    wait_for_host_key(handle).await?;
    apply_network_lockdown(handle).await?;
    provision_user(handle, user, secret).await?;
    provision_database_user(handle, user).await?;
    read_fingerprint(handle).await
}

/// Creates and fully provisions a session sandbox, cleaning up on failure.
pub async fn create_and_provision(
    session_id: &str,
    endpoint: &Endpoint,
    user: &EphemeralUsername,
    secret: &Secret,
) -> Result<(SandboxHandle, Fingerprint), SandboxError> {
    check_docker_available().await?;
    let tag = effective_image_tag();
    ensure_image(&tag).await?;
    let handle = SandboxHandle::ids(session_id, endpoint)?;
    create_network(&handle).await?;
    match provision_steps(&handle, &tag, user, secret).await {
        Ok(fingerprint) => Ok((handle, fingerprint)),
        Err(err) => {
            let _ = destroy(&handle).await;
            Err(err)
        }
    }
}

/// Removes the session container and network. Missing objects are success.
pub async fn destroy(handle: &SandboxHandle) -> Result<(), SandboxError> {
    let mut first_error: Option<SandboxError> = None;
    let container_spec = container_remove(&handle.session_id)?;
    match run_outcome(&container_spec, None).await {
        Ok(outcome) => {
            if !outcome.success() && !outcome.stderr_text().contains("No such container") {
                first_error = Some(failed("container_remove", &outcome));
            }
        }
        Err(err) => first_error = Some(err),
    }
    let network_spec = network_remove(&handle.session_id)?;
    match run_outcome(&network_spec, None).await {
        Ok(outcome) => {
            let stderr = outcome.stderr_text();
            let missing = stderr.contains("not found") || stderr.contains("No such network");
            if !outcome.success() && !missing && first_error.is_none() {
                first_error = Some(failed("network_remove", &outcome));
            }
        }
        Err(err) => {
            if first_error.is_none() {
                first_error = Some(err);
            }
        }
    }
    match first_error {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

/// Destroys a sandbox by session id. Port 0 marks that no endpoint is needed.
pub async fn destroy_by_session_id(session_id: &str) -> Result<(), SandboxError> {
    let handle = SandboxHandle {
        session_id: session_id.to_string(),
        container: container_name(session_id)?,
        network: network_name(session_id)?,
        endpoint_port: 0,
    };
    destroy(&handle).await
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
