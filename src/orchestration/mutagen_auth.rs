//! Isolated Mutagen SSH authentication over the FIFO askpass session.
//! Uses std::process with an explicit deadline because
//! `CommandSpec`/`run_bounded` has no environment support.

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::connection::Secret;
use crate::orchestration::ssh_auth::{AskpassSession, SshAuthError};

const MAX_OUTPUT: usize = 64 * 1024;
const O_NONBLOCK: i32 = 0o4000;

fn set_mode(path: &Path, mode: u32) -> Result<(), SshAuthError> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

fn resolve_program(name: &str) -> Result<PathBuf, SshAuthError> {
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(name);
            if candidate.is_file()
                && fs::metadata(&candidate).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
            {
                return Ok(candidate);
            }
        }
    }
    let fallback = PathBuf::from(format!("/usr/bin/{name}"));
    if fallback.is_file() {
        return Ok(fallback);
    }
    Err(SshAuthError::SpawnFailed(format!("{name} not found")))
}

/// Builds the ssh wrapper text; contains pinned options, never a secret.
pub fn wrapper_script(session: &AskpassSession, real_ssh: &Path) -> String {
    let known_hosts = session.known_hosts_path().to_string_lossy();
    let helper = session.helper_path().to_string_lossy();
    format!(
        "#!/bin/sh\nSSH_ASKPASS='{helper}'\nSSH_ASKPASS_REQUIRE=force\nexport SSH_ASKPASS SSH_ASKPASS_REQUIRE\nexec {} -o StrictHostKeyChecking=yes -o UserKnownHostsFile={} -o PreferredAuthentications=password -o PubkeyAuthentication=no -o NumberOfPasswordPrompts=1 \"$@\"\n",
        real_ssh.display(),
        known_hosts
    )
}

/// Builds the scp wrapper text; forces the pinned ssh wrapper via `-S`.
/// `-O` selects legacy SCP because the sandbox sshd has no SFTP subsystem.
pub fn scp_wrapper_script(session: &AskpassSession, real_scp: &Path, ssh_wrapper: &Path) -> String {
    let known_hosts = session.known_hosts_path().to_string_lossy();
    let helper = session.helper_path().to_string_lossy();
    format!(
        "#!/bin/sh\nSSH_ASKPASS='{helper}'\nSSH_ASKPASS_REQUIRE=force\nexport SSH_ASKPASS SSH_ASKPASS_REQUIRE\nexec {} -O -o StrictHostKeyChecking=yes -o UserKnownHostsFile={} -o PreferredAuthentications=password -o PubkeyAuthentication=no -S {} \"$@\"\n",
        real_scp.display(),
        known_hosts,
        ssh_wrapper.display()
    )
}

/// Isolated environment for one Mutagen invocation set.
pub struct MutagenAuthEnv {
    data_dir: PathBuf,
    bin_dir: PathBuf,
    wrapper: PathBuf,
    scp_wrapper: PathBuf,
    fifo: PathBuf,
    env: Vec<(String, String)>,
}

impl MutagenAuthEnv {
    /// Prepares `<session>/mutagen` and `<session>/bin/ssh` (both private).
    /// Writes known_hosts only from the already-verified key line.
    pub fn prepare(
        session: &AskpassSession,
        verified_key_line: &str,
    ) -> Result<Self, SshAuthError> {
        session.write_known_hosts(verified_key_line)?;
        let base = session.dir_path();
        let data_dir = base.join("mutagen");
        let bin_dir = base.join("bin");
        for dir in [&data_dir, &bin_dir] {
            match fs::create_dir(dir) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(err) => return Err(err.into()),
            }
            set_mode(dir, 0o700)?;
        }
        let real_ssh = resolve_program("ssh")?;
        let wrapper = bin_dir.join("ssh");
        fs::write(&wrapper, wrapper_script(session, &real_ssh))?;
        set_mode(&wrapper, 0o700)?;
        let scp_wrapper = bin_dir.join("scp");
        if let Ok(real_scp) = resolve_program("scp") {
            fs::write(
                &scp_wrapper,
                scp_wrapper_script(session, &real_scp, &wrapper),
            )?;
            set_mode(&scp_wrapper, 0o700)?;
        }
        let old_path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into());
        let env = vec![
            (
                "MUTAGEN_DATA_DIRECTORY".into(),
                data_dir.to_string_lossy().into_owned(),
            ),
            ("PATH".into(), format!("{}:{old_path}", bin_dir.display())),
            (
                "SSH_ASKPASS".into(),
                session.helper_path().to_string_lossy().into_owned(),
            ),
            ("SSH_ASKPASS_REQUIRE".into(), "force".into()),
        ];
        Ok(Self {
            data_dir,
            bin_dir,
            wrapper,
            scp_wrapper,
            fifo: session.fifo_path().to_path_buf(),
            env,
        })
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }
    pub fn bin_dir(&self) -> &Path {
        &self.bin_dir
    }
    pub fn wrapper_path(&self) -> &Path {
        &self.wrapper
    }
    pub fn scp_wrapper_path(&self) -> &Path {
        &self.scp_wrapper
    }
    pub fn fifo_path(&self) -> &Path {
        &self.fifo
    }
    pub fn env_pairs(&self) -> &[(String, String)] {
        &self.env
    }

    /// Starts the looping password feeder bound to this environment's FIFO.
    pub fn start_password_feeder(&self, secret: &Secret) -> PasswordFeeder {
        start_password_feeder_at(&self.fifo, secret, Duration::from_secs(600))
    }

    /// Starts the looping feeder with an explicit overall deadline.
    pub fn start_password_feeder_with_timeout(
        &self,
        secret: &Secret,
        ttl: Duration,
    ) -> PasswordFeeder {
        start_password_feeder_at(&self.fifo, secret, ttl)
    }
}

impl fmt::Debug for MutagenAuthEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MutagenAuthEnv")
            .field("data_dir", &self.data_dir)
            .field("bin_dir", &self.bin_dir)
            .field("wrapper", &self.wrapper)
            .finish()
    }
}

/// Looping feeder: serves every new FIFO reader until stopped or deadline.
pub struct PasswordFeeder {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
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

/// Starts a looping feeder for `fifo`; the secret stays in the thread only.
pub fn start_password_feeder_at(fifo: &Path, secret: &Secret, ttl: Duration) -> PasswordFeeder {
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let fifo = fifo.to_path_buf();
    let hex = secret.expose_hex();
    let join = std::thread::spawn(move || feeder_loop(&fifo, &hex, &thread_stop, ttl));
    PasswordFeeder {
        stop,
        join: Some(join),
    }
}

/// Starts a looping feeder for an askpass session's FIFO.
pub fn start_password_feeder(
    session: &AskpassSession,
    secret: &Secret,
    ttl: Duration,
) -> PasswordFeeder {
    start_password_feeder_at(session.fifo_path(), secret, ttl)
}

impl PasswordFeeder {
    /// Stops the feeder; does not block indefinitely on a stuck thread.
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            drop(join);
        }
    }

    /// Waits up to `timeout` for the feeder thread to finish.
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
}

impl fmt::Debug for PasswordFeeder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PasswordFeeder([REDACTED])")
    }
}

impl Drop for PasswordFeeder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            drop(join);
        }
    }
}

/// Bounded result of one mutagen invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutagenOutcome {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl MutagenOutcome {
    pub fn success(&self) -> bool {
        self.exit_code == Some(0)
    }
}

fn read_capped(mut pipe: impl Read + Send + 'static) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            match pipe.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let room = MAX_OUTPUT.saturating_sub(kept.len());
                    kept.extend_from_slice(&buf[..n][..room.min(n)]);
                }
            }
        }
        kept
    })
}

/// Runs one `mutagen <argv>` with the isolated env and a hard deadline.
/// std::process is used because `CommandSpec` carries no env support.
pub fn run_mutagen(
    env: &MutagenAuthEnv,
    argv: &[&str],
    timeout: Duration,
) -> Result<MutagenOutcome, SshAuthError> {
    let mut child = Command::new("mutagen")
        .args(argv)
        .envs(env.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| SshAuthError::SpawnFailed(e.kind().to_string()))?;
    let out_reader = read_capped(child.stdout.take().expect("stdout piped"));
    let err_reader = read_capped(child.stderr.take().expect("stderr piped"));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(SshAuthError::Timeout);
            }
            Err(err) => return Err(err.into()),
        }
    };
    let stdout = out_reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();
    Ok(MutagenOutcome {
        exit_code: status.code(),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

/// Stops the isolated daemon (never the user's global daemon).
pub fn stop_isolated_daemon(env: &MutagenAuthEnv) -> Result<(), SshAuthError> {
    let _ = run_mutagen(env, &["daemon", "stop"], Duration::from_secs(15))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapper_script_pins_host_key_and_has_no_secret_slot() {
        let session = AskpassSession::create("unitwrapper").expect("session");
        let script = wrapper_script(&session, Path::new("/usr/bin/ssh"));
        assert!(script.contains("StrictHostKeyChecking=yes"));
        assert!(script.contains("UserKnownHostsFile="));
        assert!(script.contains("SSH_ASKPASS_REQUIRE=force"));
        assert!(!script.contains("pairproof:v1"));
    }
}
