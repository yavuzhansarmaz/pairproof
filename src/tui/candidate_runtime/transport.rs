//! Candidate production transport: FIFO askpass SSH plus
//! isolated Mutagen sync, and the SSH auth verifier.

use super::*;

/// Production transport: FIFO askpass SSH plus isolated Mutagen sync.
pub struct SshCandidateTransport {
    connection: Option<ConnectionString>,
    key_line: Option<String>,
    verified: bool,
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
            verified: false,
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
            verified: true,
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
    pub(crate) fn poll_parts(
        &self,
    ) -> Option<(
        ConnectionString,
        String,
        SshControlClient,
        Option<MutagenAuthEnv>,
    )> {
        Some((
            self.connection.clone()?,
            self.key_line.clone()?,
            self.client.clone()?,
            self.sync_env.clone(),
        ))
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
/// Single SSH auth sequence shared by the verifier and the transport:
/// verifies the host key into `key_line` when it is not already
/// populated (a verified line is cached in the slot even when the
/// probe then fails, so a retry does not rescan), then probes with
/// the ssh `true` command. Failures map to generic rejection only.
fn authenticate_ssh(
    connection: &ConnectionString,
    key_line: &mut Option<String>,
) -> Result<(), CandidateConnectError> {
    if key_line.is_none() {
        let line = verify_and_key_line(connection.endpoint(), connection.fingerprint())
            .map_err(|e| candidate_connect_error_for_ssh(&e))?;
        *key_line = Some(line);
    }
    let Some(line) = key_line.as_deref() else {
        return Err(CandidateConnectError::Rejected);
    };
    run_ssh_command(
        connection.endpoint(),
        connection.username(),
        connection.secret(),
        line,
        &["true"],
        SSH_TIMEOUT,
    )
    .map_err(|e| candidate_connect_error_for_ssh(&e))?;
    Ok(())
}
impl CandidateTransport for SshCandidateTransport {
    fn authenticate(&mut self, connection: &ConnectionString) -> Result<(), CandidateRuntimeError> {
        if self.verified && self.connection.as_ref() == Some(connection) && self.key_line.is_some()
        {
            return Ok(());
        }
        self.connection = Some(connection.clone());
        let res =
            authenticate_ssh(connection, &mut self.key_line).map_err(CandidateRuntimeError::Auth);
        self.verified = res.is_ok();
        res
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
        let reused_askpass = self.askpass.is_some();
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
        let body = heartbeat_bytes(session_id, now_ms());
        let _ = run_ssh_command_with_stdin(
            conn.endpoint(),
            conn.username(),
            conn.secret(),
            &key_line,
            &["tee", HEARTBEAT_PATH],
            Some(body.as_slice()),
            SSH_TIMEOUT,
        );
        let started = run_mutagen(&env, &["daemon", "start"], Duration::from_secs(20))
            .map(|o| o.success())
            .unwrap_or(false);
        if started && reused_askpass {
            let _ = run_mutagen(&env, &["sync", "terminate", &name], Duration::from_secs(15));
        }
        let beta = crate::sync::beta_url(conn.endpoint(), conn.username());
        let create_args = crate::sync::sync_create_args(&name, &alpha_text, &beta);
        let create_argv: Vec<&str> = create_args.iter().map(String::as_str).collect();
        let created = if started {
            run_mutagen(&env, &create_argv, Duration::from_secs(60))
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
    fn sync_tracks_workspace(&self) -> bool {
        true
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
        flush_sync_with_env(env, session_id)
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

pub(crate) fn flush_sync_with_env(
    env: &MutagenAuthEnv,
    session_id: &str,
) -> Result<(), CandidateRuntimeError> {
    let name = sync_name(session_id)?;
    match run_mutagen(env, &["sync", "flush", &name], Duration::from_secs(30)) {
        Ok(o) if o.success() => {}
        _ => return Err(CandidateRuntimeError::SyncFailed),
    }
    if let Ok(list) = run_mutagen(
        env,
        &["sync", "list", "--template", crate::sync::STATUS_TEMPLATE],
        Duration::from_secs(10),
    ) {
        if list.success() {
            if let Some(info) = crate::sync::parse_sync_template(&list.stdout)
                .into_iter()
                .find(|s| s.name == name)
            {
                if info.conflicts > 0 || info.paused {
                    return Err(CandidateRuntimeError::SyncFailed);
                }
            }
        }
    }
    Ok(())
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
/// Verifies `c` over SSH and stashes the verified material for the
/// runtime build; shared by the verifier and the worker thread that
/// verifies off the TTY thread.
pub(crate) fn verify_and_stash(
    c: &ConnectionString,
    stash: &Arc<Mutex<Option<(ConnectionString, String)>>>,
) -> Result<(), CandidateConnectError> {
    let mut key_line = None;
    authenticate_ssh(c, &mut key_line)?;
    if let (Ok(mut slot), Some(line)) = (stash.lock(), key_line) {
        *slot = Some((c.clone(), line));
    }
    Ok(())
}

impl ConnectionVerifier for SshAuthVerifier {
    fn verify(&self, c: &ConnectionString, _now_ms: u64) -> Result<(), CandidateConnectError> {
        verify_and_stash(c, &self.stash)
    }
}
