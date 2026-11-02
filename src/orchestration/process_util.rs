//! Shared synchronous process helpers for SSH/Mutagen authentication.
//! The async capped reader in `crate::sandbox::process` is intentionally
//! not merged with the synchronous [`read_capped`] here.

use std::fs::{self, OpenOptions};
use std::io::Read;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::connection::{Endpoint, Fingerprint};
use crate::sandbox::parse::parse_host_key_fingerprint;

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
impl std::fmt::Debug for SshAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

/// Sets exact Unix permissions on `path`.
pub(crate) fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

/// Spawns a thread reading `pipe` to EOF, keeping at most `max` bytes.
pub(crate) fn read_capped(mut pipe: impl Read + Send + 'static, max: usize) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            match pipe.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let room = max.saturating_sub(kept.len());
                    kept.extend_from_slice(&buf[..n][..room.min(n)]);
                }
            }
        }
        kept
    })
}

/// Polls `child` until exit or `timeout` (15 ms poll interval).
pub(crate) fn poll_child(child: &mut Child, timeout: Duration) -> Result<ExitStatus, SshAuthError> {
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

fn feeder_loop(fifo: &Path, hex: &str, stop: &AtomicBool, ttl: Duration) {
    use std::io::Write;
    let deadline = Instant::now() + ttl;
    while !stop.load(Ordering::SeqCst) && Instant::now() < deadline {
        match OpenOptions::new()
            .write(true)
            .custom_flags(O_NONBLOCK)
            .open(fifo)
        {
            Ok(mut file) => {
                let _ = file.write_all(hex.as_bytes());
                let _ = file.write_all(b"\n");
                drop(file);
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(err) if err.raw_os_error() == Some(6) => {
                std::thread::sleep(Duration::from_millis(15));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(25)),
        }
    }
}

/// Shared core behind the one-shot and looping FIFO password feeders.
/// Dropping it signals the thread to stop without blocking.
pub(crate) struct FeederCore {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl FeederCore {
    /// One-shot feeder: serves a single FIFO reader, then exits.
    pub(crate) fn one_shot(fifo: PathBuf, hex: String) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        Self {
            stop,
            join: Some(std::thread::spawn(move || {
                feeder_once(&fifo, &hex, &thread_stop)
            })),
        }
    }

    /// Looping feeder: serves every new FIFO reader until stopped or `ttl`.
    pub(crate) fn looping(fifo: PathBuf, hex: String, ttl: Duration) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        Self {
            stop,
            join: Some(std::thread::spawn(move || {
                feeder_loop(&fifo, &hex, &thread_stop, ttl)
            })),
        }
    }

    /// Signals the feeder to stop without blocking indefinitely.
    pub(crate) fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            drop(join);
        }
    }

    /// Waits up to `timeout` for the feeder thread to finish.
    pub(crate) fn join_timeout(mut self, timeout: Duration) -> Result<(), SshAuthError> {
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
}

impl Drop for FeederCore {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            drop(join);
        }
    }
}

/// Single host-key scan shared by the SSH-auth and connection-gate paths.
/// Runs `ssh-keyscan -T 5` under the caller-chosen outer `timeout`
/// (8 s for SSH auth, 6 s for the connection gate; preserved per caller),
/// verifies the ed25519 key against `expected`, and returns the verified
/// keyscan line for use as known_hosts.
pub(crate) fn keyscan_verified_line(
    endpoint: &Endpoint,
    expected: &Fingerprint,
    timeout: Duration,
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
    poll_child(&mut child, timeout).map_err(|err| match err {
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
mod tests {
    use super::SshAuthError;

    #[test]
    fn error_debug_and_display_redact_payloads() {
        let canary = "0123456789abcdeffedcba9876543210";
        for err in [
            SshAuthError::Io(canary.to_string()),
            SshAuthError::HostKeyUnavailable(canary.to_string()),
            SshAuthError::SpawnFailed(canary.to_string()),
        ] {
            assert!(!format!("{err:?}").contains(canary), "{err:?}");
            assert!(!format!("{err}").contains(canary), "{err}");
        }
        assert_eq!(
            format!("{}", SshAuthError::Timeout),
            "ssh operation timed out"
        );
        assert_eq!(format!("{:?}", SshAuthError::Timeout), "Timeout");
        assert_eq!(
            format!("{}", SshAuthError::HostKeyMismatch),
            "host key mismatch"
        );
    }

    #[test]
    fn error_from_io_keeps_only_kind() {
        let err = SshAuthError::from(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "secret-canary-detail",
        ));
        assert_eq!(err, SshAuthError::Io("entity not found".to_string()));
        assert!(!format!("{err:?}").contains("secret-canary-detail"));
    }
}
