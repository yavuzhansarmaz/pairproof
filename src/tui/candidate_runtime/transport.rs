//! Candidate production transport: FIFO askpass SSH plus
//! isolated Mutagen sync, and the SSH auth verifier.

use super::*;

/// Production transport: FIFO askpass SSH plus isolated Mutagen sync.
pub struct SshCandidateTransport {
    connection: Option<ConnectionString>,
    key_line: Option<String>,
    client: Option<SshControlClient>,
    askpass: Option<AskpassSession>,
    sync_env: Option<MutagenAuthEnv>,
    feeder: Option<PasswordFeeder>,
    expert_version: Option<String>,
}
impl SshCandidateTransport {
    pub fn new() -> Self {
        Self {
            connection: None,
            key_line: None,
            client: None,
            askpass: None,
            sync_env: None,
            feeder: None,
            expert_version: None,
        }
    }
    pub fn from_verified(connection: &ConnectionString, key_line: String) -> Self {
        Self {
            connection: Some(connection.clone()),
            key_line: Some(key_line),
            client: None,
            askpass: None,
            sync_env: None,
            feeder: None,
            expert_version: None,
        }
    }
    fn stored(&self) -> Result<(&ConnectionString, &str), CandidateRuntimeError> {
        match (&self.connection, &self.key_line) {
            (Some(c), Some(k)) => Ok((c, k.as_str())),
            _ => Err(CandidateRuntimeError::Transport),
        }
    }
}
impl Default for SshCandidateTransport {
    fn default() -> Self {
        Self::new()
    }
}
impl std::fmt::Debug for SshCandidateTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshCandidateTransport")
            .field("connected", &self.connection.is_some())
            .field("key_line", &"[REDACTED]")
            .field("sync_active", &self.sync_env.is_some())
            .finish()
    }
}
fn sync_name(session_id: &str) -> Result<String, CandidateRuntimeError> {
    crate::sync::sync_session_name(session_id).map_err(|_| CandidateRuntimeError::SyncFailed)
}
impl CandidateTransport for SshCandidateTransport {
    fn authenticate(&mut self, connection: &ConnectionString) -> Result<(), CandidateRuntimeError> {
        self.connection = Some(connection.clone());
        if self.key_line.is_none() {
            let line = verify_and_key_line(connection.endpoint(), connection.fingerprint())
                .map_err(|e| CandidateRuntimeError::Auth(candidate_connect_error_for_ssh(&e)))?;
            self.key_line = Some(line);
        }
        let (conn, key_line) = self.stored()?;
        let key_line = key_line.to_string();
        run_ssh_command(
            conn.endpoint(),
            conn.username(),
            conn.secret(),
            &key_line,
            &["true"],
            SSH_TIMEOUT,
        )
        .map_err(|e| CandidateRuntimeError::Auth(candidate_connect_error_for_ssh(&e)))?;
        Ok(())
    }
    fn discover_session_id(&mut self) -> Result<String, CandidateRuntimeError> {
        let (conn, key_line) = self.stored()?;
        let conn = conn.clone();
        let key_line = key_line.to_string();
        let text = run_ssh_command(
            conn.endpoint(),
            conn.username(),
            conn.secret(),
            &key_line,
            &["cat", SESSION_PATH],
            SSH_TIMEOUT,
        )
        .map_err(|_| CandidateRuntimeError::Transport)?;
        let binding = parse_session_binding_full(text.as_bytes())
            .map_err(|_| CandidateRuntimeError::Transport)?;
        self.expert_version = binding.app_version.clone();
        Ok(binding.session_id)
    }
    fn expert_version(&self) -> Option<String> {
        self.expert_version.clone()
    }
    fn write_hello(&mut self, session_id: &str) -> Result<(), CandidateRuntimeError> {
        let (conn, key_line) = self.stored()?;
        let key_line = key_line.to_string();
        let client = SshControlClient::new(
            session_id,
            conn.endpoint().clone(),
            conn.username().clone(),
            conn.secret().clone(),
            key_line.clone(),
        );
        let body = hello_bytes(session_id, env!("CARGO_PKG_VERSION"));
        run_ssh_command_with_stdin(
            conn.endpoint(),
            conn.username(),
            conn.secret(),
            &key_line,
            &["tee", HELLO_PATH],
            Some(&body),
            SSH_TIMEOUT,
        )
        .map_err(|_| CandidateRuntimeError::Transport)?;
        self.client = Some(client);
        Ok(())
    }
    fn write_final_ack(&mut self, session_id: &str) -> Result<(), CandidateRuntimeError> {
        let (conn, key_line) = self.stored()?;
        let key_line = key_line.to_string();
        let body = final_ack_bytes(session_id);
        run_ssh_command_with_stdin(
            conn.endpoint(),
            conn.username(),
            conn.secret(),
            &key_line,
            &["tee", FINAL_ACK_PATH],
            Some(body.as_slice()),
            SSH_TIMEOUT,
        )
        .map_err(|_| CandidateRuntimeError::Transport)?;
        Ok(())
    }
    fn write_heartbeat(
        &mut self,
        session_id: &str,
        at_ms: u64,
    ) -> Result<(), CandidateRuntimeError> {
        let (conn, key_line) = self.stored()?;
        let key_line = key_line.to_string();
        let body = heartbeat_bytes(session_id, at_ms);
        run_ssh_command_with_stdin(
            conn.endpoint(),
            conn.username(),
            conn.secret(),
            &key_line,
            &["tee", HEARTBEAT_PATH],
            Some(body.as_slice()),
            SSH_TIMEOUT,
        )
        .map_err(|_| CandidateRuntimeError::Transport)?;
        Ok(())
    }
    fn start_sync(&mut self, session_id: &str, alpha: &Path) -> Result<(), CandidateRuntimeError> {
        let (conn, key_line) = self.stored()?;
        let conn = conn.clone();
        let key_line = key_line.to_string();
        let name = sync_name(session_id)?;
        let alpha_text = alpha.to_string_lossy().into_owned();
        crate::sync::validate_alpha_dir(&alpha_text)
            .map_err(|_| CandidateRuntimeError::SyncFailed)?;
        std::fs::create_dir_all(alpha).map_err(|_| CandidateRuntimeError::SyncFailed)?;
        if self.askpass.is_none() {
            self.askpass = Some(
                AskpassSession::create("candidate")
                    .map_err(|_| CandidateRuntimeError::SyncFailed)?,
            );
        }
        let askpass = self
            .askpass
            .as_ref()
            .ok_or(CandidateRuntimeError::SyncFailed)?;
        let env = MutagenAuthEnv::prepare(askpass, &key_line)
            .map_err(|_| CandidateRuntimeError::SyncFailed)?;
        let feeder = env.start_password_feeder(conn.secret());
        let started = run_mutagen(&env, &["daemon", "start"], Duration::from_secs(20))
            .map(|o| o.success())
            .unwrap_or(false);
        let beta = crate::sync::beta_url(conn.endpoint(), conn.username());
        let created = if started {
            run_mutagen(
                &env,
                &[
                    "sync",
                    "create",
                    "--name",
                    &name,
                    "--mode",
                    "two-way-safe",
                    "--symlink-mode",
                    "ignore",
                    &alpha_text,
                    &beta,
                ],
                Duration::from_secs(60),
            )
            .map(|o| o.success())
            .unwrap_or(false)
        } else {
            false
        };
        if !created {
            feeder.stop();
            return Err(CandidateRuntimeError::SyncFailed);
        }
        self.sync_env = Some(env);
        self.feeder = Some(feeder);
        Ok(())
    }
    fn send_intent(&mut self, intent: &Intent) -> Result<(), CandidateRuntimeError> {
        let client = self
            .client
            .as_ref()
            .ok_or(CandidateRuntimeError::Transport)?;
        client
            .send_intent(intent)
            .map_err(|_| CandidateRuntimeError::Transport)
    }
    fn read_snapshot(&mut self) -> Result<Option<CandidateSnapshot>, CandidateRuntimeError> {
        let Some(client) = self.client.as_ref() else {
            return Ok(None);
        };
        match client.fetch_snapshot() {
            Ok(snapshot) => Ok(Some(snapshot)),
            Err(crate::orchestration::control::ControlError::WrongSession) => {
                Err(CandidateRuntimeError::WrongSession)
            }
            Err(_) => Ok(None),
        }
    }
    fn flush_sync(&mut self, session_id: &str) -> Result<(), CandidateRuntimeError> {
        let Some(env) = self.sync_env.as_ref() else {
            return Ok(());
        };
        let name = sync_name(session_id)?;
        match run_mutagen(env, &["sync", "flush", &name], Duration::from_secs(30)) {
            Ok(o) if o.success() => Ok(()),
            _ => Err(CandidateRuntimeError::SyncFailed),
        }
    }
    fn terminate_sync(&mut self, session_id: &str) -> Result<(), CandidateRuntimeError> {
        let mut result = Ok(());
        if let Some(env) = self.sync_env.as_ref() {
            if let Ok(name) = sync_name(session_id) {
                if !matches!(run_mutagen(env, &["sync", "terminate", &name], Duration::from_secs(30)), Ok(o) if o.success())
                {
                    result = Err(CandidateRuntimeError::SyncFailed);
                }
            }
            let _ = run_mutagen(env, &["daemon", "stop"], Duration::from_secs(15));
        }
        if let Some(feeder) = self.feeder.take() {
            feeder.stop();
        }
        self.sync_env = None;
        result
    }
}
pub struct SshAuthVerifier {
    stash: Arc<Mutex<Option<(ConnectionString, String)>>>,
}
impl SshAuthVerifier {
    pub fn new() -> Self {
        Self {
            stash: Arc::new(Mutex::new(None)),
        }
    }
    pub fn stash(&self) -> Arc<Mutex<Option<(ConnectionString, String)>>> {
        Arc::clone(&self.stash)
    }
}
impl Default for SshAuthVerifier {
    fn default() -> Self {
        Self::new()
    }
}
impl ConnectionVerifier for SshAuthVerifier {
    fn verify(&self, c: &ConnectionString, _now_ms: u64) -> Result<(), CandidateConnectError> {
        let line = verify_and_key_line(c.endpoint(), c.fingerprint())
            .map_err(|e| candidate_connect_error_for_ssh(&e))?;
        run_ssh_command(
            c.endpoint(),
            c.username(),
            c.secret(),
            &line,
            &["true"],
            SSH_TIMEOUT,
        )
        .map_err(|e| candidate_connect_error_for_ssh(&e))?;
        if let Ok(mut slot) = self.stash.lock() {
            *slot = Some((c.clone(), line));
        }
        Ok(())
    }
}
