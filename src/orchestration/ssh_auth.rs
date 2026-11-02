//! FIFO askpass SSH password authentication.
//! The secret moves only through a private FIFO; never argv, env, or files.
use crate::connection::{Endpoint, EphemeralUsername, Fingerprint, Secret};
use crate::sandbox::parse::parse_host_key_fingerprint;
use rand::rngs::OsRng;
use rand::RngCore;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
const MAX_STDOUT: usize = 64 * 1024;
const O_NONBLOCK: i32 = 0o4000;

/// Typed SSH-auth failure. Payloads are never shown by Debug or Display.
#[derive(Clone, PartialEq, Eq, thiserror::Error)]
pub enum SshAuthError {
    #[error("ssh auth io error")]
    Io(String),
    #[error("ssh operation timed out")]
    Timeout,
    #[error("ssh authentication failed")]
    AuthFailed,
    #[error("host key mismatch")]
    HostKeyMismatch,
    #[error("host key unavailable")]
    HostKeyUnavailable(String),
    #[error("ssh could not be started")]
    SpawnFailed(String),
}
impl fmt::Debug for SshAuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(_) => write!(f, "Io(\"[REDACTED]\")"),
            Self::Timeout => write!(f, "Timeout"),
            Self::AuthFailed => write!(f, "AuthFailed"),
            Self::HostKeyMismatch => write!(f, "HostKeyMismatch"),
            Self::HostKeyUnavailable(_) => write!(f, "HostKeyUnavailable(\"[REDACTED]\")"),
            Self::SpawnFailed(_) => write!(f, "SpawnFailed(\"[REDACTED]\")"),
        }
    }
}
impl From<std::io::Error> for SshAuthError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err.kind().to_string())
    }
}
fn set_mode(path: &Path, mode: u32) -> Result<(), SshAuthError> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}
fn random_suffix() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
#[rustfmt::skip]
fn sanitize_tag(tag: &str) -> String { let cleaned: String = tag.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).take(24).collect(); if cleaned.is_empty() { "session".to_string() } else { cleaned } }
/// Builds the askpass helper text; contains the FIFO path, never a secret.
pub fn helper_script(fifo: &Path) -> String {
    let fifo = fifo.to_string_lossy().replace('\'', "'\\''");
    format!("#!/bin/sh\ncase \"$1\" in\n  *assword*) exec cat '{fifo}' ;;\n  *) exit 1 ;;\nesac\n")
}

/// RAII private askpass directory: helper, FIFO and known_hosts.
pub struct AskpassSession {
    dir: PathBuf,
    helper: PathBuf,
    fifo: PathBuf,
    known_hosts: PathBuf,
}
impl AskpassSession {
    /// Creates a private (0700) session directory under the OS temp dir.
    pub fn create(session_tag: &str) -> Result<Self, SshAuthError> {
        let dir = std::env::temp_dir().join(format!(
            "pairproof-askpass-{}-{}",
            sanitize_tag(session_tag),
            random_suffix()
        ));
        fs::create_dir(&dir)?;
        let session = Self {
            helper: dir.join("askpass.sh"),
            fifo: dir.join("password.fifo"),
            known_hosts: dir.join("known_hosts"),
            dir,
        };
        if let Err(err) = session.setup() {
            let _ = fs::remove_dir_all(&session.dir);
            return Err(err);
        }
        Ok(session)
    }
    fn setup(&self) -> Result<(), SshAuthError> {
        set_mode(&self.dir, 0o700)?;
        fs::write(&self.helper, helper_script(&self.fifo))?;
        set_mode(&self.helper, 0o700)?;
        let status = Command::new("mkfifo")
            .args(["-m", "600"])
            .arg(&self.fifo)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| SshAuthError::SpawnFailed(e.kind().to_string()))?;
        if !status.success() {
            return Err(SshAuthError::Io("mkfifo failed".into()));
        }
        set_mode(&self.fifo, 0o600)?;
        Ok(())
    }
    #[rustfmt::skip]    pub fn helper_path(&self) -> &Path { &self.helper }
    #[rustfmt::skip]    pub fn fifo_path(&self) -> &Path { &self.fifo }
    #[rustfmt::skip]    pub fn dir_path(&self) -> &Path { &self.dir }
    #[rustfmt::skip]    pub fn known_hosts_path(&self) -> &Path { &self.known_hosts }
    /// Writes exactly one verified key line as the session known_hosts (0600).
    pub fn write_known_hosts(&self, verified_key_line: &str) -> Result<(), SshAuthError> {
        let line = verified_key_line.trim();
        if line.is_empty() || line.contains('\n') || line.contains('\r') {
            return Err(SshAuthError::Io("invalid known_hosts line".into()));
        }
        fs::write(&self.known_hosts, format!("{line}\n"))?;
        set_mode(&self.known_hosts, 0o600)?;
        Ok(())
    }
    /// Starts a one-shot feeder thread that writes the secret to the FIFO.
    pub fn feed_password(&self, secret: &Secret) -> FeederHandle {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let fifo = self.fifo.clone();
        let hex = secret.expose_hex();
        FeederHandle {
            stop,
            join: Some(std::thread::spawn(move || {
                feeder_once(&fifo, &hex, &thread_stop)
            })),
        }
    }
}
impl fmt::Debug for AskpassSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AskpassSession")
            .field("dir", &self.dir)
            .finish()
    }
}
impl Drop for AskpassSession {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}
fn feeder_once(fifo: &Path, hex: &str, stop: &AtomicBool) {
    use std::io::Write;
    while !stop.load(Ordering::SeqCst) {
        match OpenOptions::new()
            .write(true)
            .custom_flags(O_NONBLOCK)
            .open(fifo)
        {
            Ok(mut file) => {
                let _ = file.write_all(hex.as_bytes());
                let _ = file.write_all(b"\n");
                return;
            }
            Err(err) if err.raw_os_error() == Some(6) => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(_) => return,
        }
    }
}

/// Handle for a one-shot feeder thread. Dropping it stops the thread.
pub struct FeederHandle {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}
impl FeederHandle {
    /// Waits up to `timeout` for the feed to complete.
    pub fn join_timeout(mut self, timeout: Duration) -> Result<(), SshAuthError> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.join.as_ref().is_some_and(JoinHandle::is_finished) {
                if let Some(join) = self.join.take() {
                    let _ = join.join();
                }
                return Ok(());
            }
            if Instant::now() >= deadline {
                self.stop.store(true, Ordering::SeqCst);
                return Err(SshAuthError::Timeout);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    /// Signals the feeder to stop without blocking indefinitely.
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            drop(join);
        }
    }
}
impl fmt::Debug for FeederHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FeederHandle([REDACTED])")
    }
}
impl Drop for FeederHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            drop(join);
        }
    }
}

/// Auditable SSH invocation: program, argv and env pairs (never a secret).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshInvocation {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}
/// Builds the exact ssh argv/env used by [`run_ssh_command`].
#[rustfmt::skip]
pub fn ssh_invocation(session: &AskpassSession, endpoint: &Endpoint, user: &EphemeralUsername, remote_argv: &[&str], timeout: Duration) -> SshInvocation {
    let connect_secs = (timeout.as_millis().div_ceil(1000)).max(1);
    let mut args: Vec<String> = Vec::new();
    for opt in ["StrictHostKeyChecking=yes".to_string(), format!("UserKnownHostsFile={}", session.known_hosts_path().display()), "PreferredAuthentications=password".into(), "PubkeyAuthentication=no".into(), "NumberOfPasswordPrompts=1".into(), format!("ConnectTimeout={connect_secs}")] { args.push("-o".into()); args.push(opt); }
    args.push("-p".into()); args.push(endpoint.port().to_string()); args.push(format!("{}@{}", user.as_str(), endpoint.host()));
    args.extend(remote_argv.iter().map(|arg| (*arg).to_string()));
    SshInvocation { program: "ssh".into(), args, env: vec![("SSH_ASKPASS".into(), session.helper_path().to_string_lossy().into_owned()), ("SSH_ASKPASS_REQUIRE".into(), "force".into())] }
}
fn read_capped(mut pipe: impl Read + Send + 'static) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            match pipe.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let room = MAX_STDOUT.saturating_sub(kept.len());
                    kept.extend_from_slice(&buf[..n][..room.min(n)]);
                }
            }
        }
        kept
    })
}
fn poll_child(
    child: &mut Child,
    timeout: Duration,
) -> Result<std::process::ExitStatus, SshAuthError> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(15)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(SshAuthError::Timeout);
            }
            Err(err) => return Err(err.into()),
        }
    }
}
/// Runs one remote command over password SSH with a pinned host key.
/// Uses std::process with an explicit deadline (no shell, argv only).
pub fn run_ssh_command(
    endpoint: &Endpoint,
    user: &EphemeralUsername,
    secret: &Secret,
    verified_key_line: &str,
    remote_argv: &[&str],
    timeout: Duration,
) -> Result<String, SshAuthError> {
    run_ssh_command_with_stdin(
        endpoint,
        user,
        secret,
        verified_key_line,
        remote_argv,
        None,
        timeout,
    )
}
/// Like [`run_ssh_command`], feeding `stdin` bytes to the remote process.
/// Payload bytes travel only as stdin data, never in argv or env.
pub fn run_ssh_command_with_stdin(
    endpoint: &Endpoint,
    user: &EphemeralUsername,
    secret: &Secret,
    verified_key_line: &str,
    remote_argv: &[&str],
    stdin: Option<&[u8]>,
    timeout: Duration,
) -> Result<String, SshAuthError> {
    if remote_argv.is_empty() {
        return Err(SshAuthError::Io("empty remote argv".into()));
    }
    let session = AskpassSession::create("ssh")?;
    session.write_known_hosts(verified_key_line)?;
    let invocation = ssh_invocation(&session, endpoint, user, remote_argv, timeout);
    let feeder = session.feed_password(secret);
    let mut child = Command::new(&invocation.program)
        .args(&invocation.args)
        .envs(invocation.env.iter().map(|(k, v)| (k, v)))
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| SshAuthError::SpawnFailed(e.kind().to_string()))?;
    if let (Some(mut pipe), Some(data)) = (child.stdin.take(), stdin) {
        let data = data.to_vec();
        std::thread::spawn(move || {
            let _ = pipe.write_all(&data);
        });
    }
    let out_reader = read_capped(child.stdout.take().expect("stdout piped"));
    let err_reader = read_capped(child.stderr.take().expect("stderr piped"));
    let status = match poll_child(&mut child, timeout) {
        Ok(status) => status,
        Err(err) => {
            feeder.stop();
            return Err(err);
        }
    };
    let stdout = out_reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();
    let _ = feeder.join_timeout(Duration::from_secs(2));
    if status.success() {
        return Ok(String::from_utf8_lossy(&stdout).into_owned());
    }
    let stderr_text = String::from_utf8_lossy(&stderr).to_lowercase();
    if stderr_text.contains("host key verification") {
        Err(SshAuthError::HostKeyMismatch)
    } else if stderr_text.contains("permission denied") {
        Err(SshAuthError::AuthFailed)
    } else {
        Err(SshAuthError::Io("ssh exited non-zero".into()))
    }
}
/// Keyscans `endpoint`, verifies the ed25519 key against `expected`,
/// and returns the verified keyscan line for use as known_hosts.
pub fn verify_and_key_line(
    endpoint: &Endpoint,
    expected: &Fingerprint,
) -> Result<String, SshAuthError> {
    let unavailable = |detail: &str| SshAuthError::HostKeyUnavailable(detail.to_string());
    let mut child = Command::new("ssh-keyscan")
        .args([
            "-T",
            "5",
            "-p",
            &endpoint.port().to_string(),
            endpoint.host(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| unavailable("ssh-keyscan could not be started"))?;
    poll_child(&mut child, Duration::from_secs(8)).map_err(|err| match err {
        SshAuthError::Timeout => unavailable("ssh-keyscan timed out"),
        _ => unavailable("ssh-keyscan could not be started"),
    })?;
    let output = child
        .wait_with_output()
        .map_err(|_| unavailable("ssh-keyscan output could not be read"))?;
    if output.stdout.is_empty() {
        return Err(unavailable("ssh-keyscan returned no host key"));
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let line = text
        .lines()
        .find(|line| line.split_whitespace().any(|field| field == "ssh-ed25519"))
        .ok_or_else(|| unavailable("no ed25519 host key was returned"))?;
    let found = parse_host_key_fingerprint(line)
        .map_err(|_| unavailable("the returned host key could not be parsed"))?;
    if found == *expected {
        Ok(line.trim().to_string())
    } else {
        Err(SshAuthError::HostKeyMismatch)
    }
}
#[cfg(test)]
#[rustfmt::skip]
mod tests { use super::*; #[test] fn helper_script_gates_on_password_prompt() { let s = helper_script(Path::new("/tmp/x/password.fifo")); assert!(s.contains("*assword*") && s.contains("exit 1") && s.contains("exec cat")); assert!(!s.contains("0123456789abcdeffedcba9876543210")); } }
