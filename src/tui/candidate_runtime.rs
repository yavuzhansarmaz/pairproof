//! Candidate runtime: SSH auth, hello, isolated Mutagen sync, intents,
//! and snapshot rendering. Candidate failures never carry secrets.
#[rustfmt::skip] use crate::connection::ConnectionString;
#[rustfmt::skip] use crate::orchestration::control::{final_ack_bytes, heartbeat_bytes, parse_session_binding, CandidateSnapshot, Intent, IntentKind, SnapshotFinal, SnapshotOption, SnapshotTest, FINAL_ACK_PATH, HEARTBEAT_PATH, SESSION_PATH};
#[rustfmt::skip] use crate::orchestration::control_client::{CandidateControlClient, SshControlClient};
#[rustfmt::skip] use crate::orchestration::mutagen_auth::{run_mutagen, MutagenAuthEnv, PasswordFeeder};
#[rustfmt::skip] use crate::orchestration::ssh_auth::{run_ssh_command, run_ssh_command_with_stdin, verify_and_key_line, AskpassSession, SshAuthError};
#[rustfmt::skip] use crate::tui::candidate::{screen_title, CandidateConnectError, CandidateScreen, ConnectionVerifier};
#[rustfmt::skip] use crate::tui::candidate_chat::other_insert;
#[rustfmt::skip] use crate::tui::candidate_run::{candidate_screen_lines, handle_candidate_key, handle_candidate_paste, now_ms, CandidateFocus, CandidateRunState, CandidateTtyGuard};
#[rustfmt::skip] use crate::tui::shared::sanitize_display;
#[rustfmt::skip] use crossterm::event::{Event, KeyCode as K, KeyEvent, KeyEventKind, KeyModifiers};
#[rustfmt::skip] use std::error::Error;
#[rustfmt::skip] use std::path::{Path, PathBuf};
#[rustfmt::skip] use std::sync::{Arc, Mutex};
#[rustfmt::skip] use std::time::Duration;
const HELLO_PATH: &str = "/pairproof-control/inbox/hello.json";
const SSH_TIMEOUT: Duration = Duration::from_secs(20);
const HEARTBEAT_THROTTLE_MS: u64 = 5_000;
pub const CONNECTION_LOST: &str = "Connection lost \u{2014} retrying";
/// Explicit runtime phase; work is reachable only when Connected.
#[rustfmt::skip]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateRuntimePhase { Disconnected, Authenticating, Syncing, Connected, Ended }
/// Candidate-safe runtime failure; no variant carries transport detail.
#[rustfmt::skip]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CandidateRuntimeError {
    #[error("{0}")] Auth(#[from] CandidateConnectError),
    #[error("Sync problem \u{2014} ask the expert")] SyncFailed,
    #[error("control message bound to another session")] WrongSession,
    #[error("candidate is not connected")] NotConnected,
    #[error("session has ended")] Ended,
    #[error("Connection problem \u{2014} ask the expert")] Transport,
}
/// Maps any SSH-auth failure to generic rejection; payloads are dropped.
#[rustfmt::skip] pub fn candidate_connect_error_for_ssh(_err: &SshAuthError) -> CandidateConnectError { CandidateConnectError::Rejected }
/// Control-channel boundary; production is [`SshCandidateTransport`].
#[rustfmt::skip]
pub trait CandidateTransport {
    fn authenticate(&mut self, connection: &ConnectionString) -> Result<(), CandidateRuntimeError>;
    fn discover_session_id(&mut self) -> Result<String, CandidateRuntimeError> { Err(CandidateRuntimeError::Transport) }
    fn write_hello(&mut self, session_id: &str) -> Result<(), CandidateRuntimeError>;
    fn start_sync(&mut self, session_id: &str, alpha: &Path) -> Result<(), CandidateRuntimeError>;
    fn send_intent(&mut self, intent: &Intent) -> Result<(), CandidateRuntimeError>;
    fn write_heartbeat(&mut self, _session_id: &str, _at_ms: u64) -> Result<(), CandidateRuntimeError> { Ok(()) }
    fn read_snapshot(&mut self) -> Result<Option<CandidateSnapshot>, CandidateRuntimeError>;
    fn write_final_ack(&mut self, _session_id: &str) -> Result<(), CandidateRuntimeError> { Ok(()) }
    fn flush_sync(&mut self, session_id: &str) -> Result<(), CandidateRuntimeError>;
    fn terminate_sync(&mut self, session_id: &str) -> Result<(), CandidateRuntimeError>;
}
/// Snapshot-only render data; the snapshot type has no hidden fields.
#[rustfmt::skip]
#[derive(Debug, Clone, PartialEq)]
pub struct CandidateView {
    pub phase: String, pub sync_phase: String, pub question_title: Option<String>, pub question_index: Option<usize>,
    pub statement: Option<String>, pub options: Vec<SnapshotOption>, pub selected: Option<String>,
    pub latest_test: Option<SnapshotTest>, pub chat_replies: Vec<String>, pub final_breakdown: Option<SnapshotFinal>,
}
#[rustfmt::skip]
impl CandidateView {
    pub fn from_snapshot(snapshot: Option<&CandidateSnapshot>) -> Self {
        let Some(s) = snapshot else { return Self { phase: String::new(), sync_phase: String::new(), question_title: None, question_index: None, statement: None, options: Vec::new(), selected: None, latest_test: None, chat_replies: Vec::new(), final_breakdown: None }; };
        Self { phase: s.phase.clone(), sync_phase: s.sync_phase.clone(), question_title: s.current.as_ref().map(|q| q.title.clone()), question_index: s.current.as_ref().map(|q| q.index), statement: s.current.as_ref().and_then(|q| q.statement.clone()), options: s.options.clone(), selected: s.selected.clone(), latest_test: s.latest_test, chat_replies: s.chat_replies.clone(), final_breakdown: s.final_breakdown.clone() }
    }
    pub fn is_terminal(&self) -> bool { matches!(self.phase.as_str(), "Completed" | "EndedByExpert" | "TechnicalFailure") }
    /// Sanitized lines; a rating renders only from a carried breakdown.
    pub fn lines(&self) -> Vec<String> {
        if self.phase.is_empty() { return vec!["Waiting for the expert.".to_string()]; }
        let mut lines = Vec::new();
        match (&self.question_index, &self.question_title) { (Some(i), Some(t)) => lines.push(format!("Q{}: {}", i + 1, t)), _ => lines.push("No active question.".to_string()) }
        match &self.statement { Some(s) => lines.push(s.clone()), None => lines.push("Statement is not available yet.".to_string()) }
        if self.options.is_empty() { lines.push("Approaches are not available yet.".to_string()); } else { for (i, o) in self.options.iter().enumerate() { lines.push(format!("{}: {} - {}", i, o.title, o.summary)); } lines.push("Other: describe your own approach".to_string()); }
        lines.push(format!("Selected approach: {}", self.selected.as_deref().unwrap_or("none")));
        match self.latest_test { Some(t) => lines.push(format!("Latest test: {} passed, {} failed", t.passed, t.failed)), None => lines.push("Latest test: no test run yet".to_string()) }
        if !self.sync_phase.is_empty() { lines.push(self.sync_phase.clone()); }
        lines.push(format!("Phase: {}", self.phase)); for reply in &self.chat_replies { lines.push(format!("Coach: {reply}")); }
        if let Some(f) = &self.final_breakdown { lines.push("Final breakdown".to_string()); lines.push(format!("Final rating: {:.2}", f.rating)); for q in &f.per_question { let grade = q.grade.map_or("not scored".to_string(), |g| format!("{g:.1}")); let mut line = format!("Q{}: grade {}", q.slot_index + 1, grade); if let Some(r) = &q.rationale { line.push_str(&format!(": {r}")); } lines.push(line); } }
        else if self.phase == "TechnicalFailure" { lines.push("The session ended as a technical failure.".to_string()); lines.push("No evaluation was produced; the interview can be rescheduled.".to_string()); }
        else if self.is_terminal() { lines.push("Session ended.".to_string()); }
        lines.iter().map(|line| sanitize_display(line)).collect()
    }
}
/// Snapshot-driven runtime; intent seq and ids are managed internally.
#[rustfmt::skip]
pub struct CandidateRuntime<T: CandidateTransport> {
    transport: T, session_id: String, workspace: PathBuf, sequencer: CandidateControlClient,
    phase: CandidateRuntimePhase, last_snapshot: Option<CandidateSnapshot>, chat_seq: u64, ack_sent: bool, lost: bool, last_hb_ms: u64,
}
#[rustfmt::skip]
impl<T: CandidateTransport> CandidateRuntime<T> {
    pub fn new(session_id: impl Into<String>, workspace: PathBuf, transport: T) -> Self {
        let session_id = session_id.into();
        Self { sequencer: CandidateControlClient::new(&session_id), transport, session_id, workspace, phase: CandidateRuntimePhase::Disconnected, last_snapshot: None, chat_seq: 0, ack_sent: false, lost: false, last_hb_ms: 0 }
    }
    pub fn new_discovering(workspace: PathBuf, transport: T) -> Self { Self::new("", workspace, transport) }
    pub fn phase(&self) -> CandidateRuntimePhase { self.phase }
    pub fn session_id(&self) -> &str { &self.session_id }
    pub fn workspace(&self) -> &Path { &self.workspace }
    pub fn transport(&self) -> &T { &self.transport }
    pub fn transport_mut(&mut self) -> &mut T { &mut self.transport }
    pub fn last_snapshot(&self) -> Option<&CandidateSnapshot> { self.last_snapshot.as_ref() }
    pub fn final_ack_sent(&self) -> bool { self.ack_sent }
    pub fn connection_lost(&self) -> bool { self.lost }
    pub fn status_text(&self) -> Option<String> { self.lost.then(|| CONNECTION_LOST.to_string()) }
    /// Authenticates, writes hello, then starts sync, in that order.
    pub fn connect(&mut self, connection: &ConnectionString) -> Result<(), CandidateRuntimeError> {
        if self.phase == CandidateRuntimePhase::Ended { return Err(CandidateRuntimeError::Ended); }
        if self.phase == CandidateRuntimePhase::Connected { return Ok(()); }
        self.phase = CandidateRuntimePhase::Authenticating;
        if let Err(err) = self.transport.authenticate(connection) { self.phase = CandidateRuntimePhase::Disconnected; return Err(err); }
        match self.transport.discover_session_id() { Ok(id) if !id.trim().is_empty() && crate::sandbox::validate::validate_session_id(&id).is_ok() && (self.session_id.is_empty() || self.session_id == id) => { self.session_id = id.clone(); self.sequencer = CandidateControlClient::new(&id); } _ => { self.phase = CandidateRuntimePhase::Disconnected; return Err(CandidateRuntimeError::Transport); } }
        if let Err(err) = self.transport.write_hello(&self.session_id) { self.phase = CandidateRuntimePhase::Disconnected; return Err(err); }
        self.phase = CandidateRuntimePhase::Syncing; self.transport.start_sync(&self.session_id, &self.workspace)?; self.phase = CandidateRuntimePhase::Connected; Ok(())
    }
    pub fn send_intent(&mut self, kind: IntentKind) -> Result<Intent, CandidateRuntimeError> {
        if self.phase == CandidateRuntimePhase::Ended { return Err(CandidateRuntimeError::Ended); }
        if self.phase != CandidateRuntimePhase::Connected { return Err(CandidateRuntimeError::NotConnected); }
        let intent = self.sequencer.prepare(kind);
        intent.validate(&self.session_id).map_err(|_| CandidateRuntimeError::Transport)?;
        match self.transport.send_intent(&intent) { Ok(()) => Ok(intent), Err(err) => { if err == CandidateRuntimeError::Transport { self.lost = true; } Err(err) } }
    }
    pub fn send_chat(&mut self, text: &str) -> Result<Intent, CandidateRuntimeError> {
        self.chat_seq += 1;
        self.send_intent(IntentKind::ChatRequest { chat_id: format!("chat-{}", self.chat_seq), text: text.to_string() })
    }
    /// Reads one snapshot; wrong-session keeps the last good snapshot.
    pub fn poll_snapshot(&mut self) -> Result<Option<CandidateSnapshot>, CandidateRuntimeError> { self.poll_snapshot_at(now_ms()) }
    /// Polls with an explicit clock: throttled heartbeat, then one snapshot read.
    /// A transport failure marks the connection lost and keeps the last snapshot.
    pub fn poll_snapshot_at(&mut self, now_ms: u64) -> Result<Option<CandidateSnapshot>, CandidateRuntimeError> {
        if self.phase == CandidateRuntimePhase::Ended { return Err(CandidateRuntimeError::Ended); }
        if self.phase != CandidateRuntimePhase::Connected { return Err(CandidateRuntimeError::NotConnected); }
        if now_ms.saturating_sub(self.last_hb_ms) > HEARTBEAT_THROTTLE_MS && self.transport.write_heartbeat(&self.session_id, now_ms).is_ok() { self.last_hb_ms = now_ms; }
        match self.transport.read_snapshot() {
            Err(CandidateRuntimeError::Transport) => { self.lost = true; Err(CandidateRuntimeError::Transport) }
            Err(err) => Err(err),
            Ok(None) => Ok(None),
            Ok(Some(snapshot)) => {
                if snapshot.session_id != self.session_id { return Err(CandidateRuntimeError::WrongSession); }
                self.lost = false;
                if snapshot.final_breakdown.is_some() && !self.ack_sent { self.ack_sent = true; let _ = self.transport.write_final_ack(&self.session_id); }
                self.last_snapshot = Some(snapshot.clone());
                Ok(Some(snapshot))
            }
        }
    }
    /// Reconnects with a fresh string: drops stale sync, then full connect.
    pub fn reconnect(&mut self, connection: &ConnectionString) -> Result<(), CandidateRuntimeError> {
        if self.phase == CandidateRuntimePhase::Ended { return Err(CandidateRuntimeError::Ended); }
        let _ = self.transport.terminate_sync(&self.session_id);
        self.phase = CandidateRuntimePhase::Disconnected; self.lost = false;
        self.connect(connection)
    }
    pub fn flush(&mut self) -> Result<(), CandidateRuntimeError> { if self.phase != CandidateRuntimePhase::Connected { return Err(CandidateRuntimeError::NotConnected); } self.transport.flush_sync(&self.session_id) }
    /// Terminates sync and ends; local files are preserved.
    pub fn end(&mut self) -> Result<(), CandidateRuntimeError> { if self.phase == CandidateRuntimePhase::Ended { return Ok(()); } let result = self.transport.terminate_sync(&self.session_id); self.phase = CandidateRuntimePhase::Ended; result }
    pub fn view(&self) -> CandidateView { CandidateView::from_snapshot(self.last_snapshot.as_ref()) }
}
/// Production transport: FIFO askpass SSH plus isolated Mutagen sync.
#[rustfmt::skip]
pub struct SshCandidateTransport {
    connection: Option<ConnectionString>, key_line: Option<String>, client: Option<SshControlClient>,
    askpass: Option<AskpassSession>, sync_env: Option<MutagenAuthEnv>, feeder: Option<PasswordFeeder>,
}
#[rustfmt::skip]
impl SshCandidateTransport {
    pub fn new() -> Self { Self { connection: None, key_line: None, client: None, askpass: None, sync_env: None, feeder: None } }
    pub fn from_verified(connection: &ConnectionString, key_line: String) -> Self { Self { connection: Some(connection.clone()), key_line: Some(key_line), client: None, askpass: None, sync_env: None, feeder: None } }
    fn stored(&self) -> Result<(&ConnectionString, &str), CandidateRuntimeError> { match (&self.connection, &self.key_line) { (Some(c), Some(k)) => Ok((c, k.as_str())), _ => Err(CandidateRuntimeError::Transport) } }
}
#[rustfmt::skip] impl Default for SshCandidateTransport { fn default() -> Self { Self::new() } }
#[rustfmt::skip]
impl std::fmt::Debug for SshCandidateTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshCandidateTransport").field("connected", &self.connection.is_some()).field("key_line", &"[REDACTED]").field("sync_active", &self.sync_env.is_some()).finish()
    }
}
#[rustfmt::skip] fn sync_name(session_id: &str) -> Result<String, CandidateRuntimeError> { crate::sync::sync_session_name(session_id).map_err(|_| CandidateRuntimeError::SyncFailed) }
#[rustfmt::skip]
impl CandidateTransport for SshCandidateTransport {
    fn authenticate(&mut self, connection: &ConnectionString) -> Result<(), CandidateRuntimeError> {
        self.connection = Some(connection.clone());
        if self.key_line.is_none() { let line = verify_and_key_line(connection.endpoint(), connection.fingerprint()).map_err(|e| CandidateRuntimeError::Auth(candidate_connect_error_for_ssh(&e)))?; self.key_line = Some(line); }
        let (conn, key_line) = self.stored()?; let key_line = key_line.to_string();
        run_ssh_command(conn.endpoint(), conn.username(), conn.secret(), &key_line, &["true"], SSH_TIMEOUT).map_err(|e| CandidateRuntimeError::Auth(candidate_connect_error_for_ssh(&e)))?; Ok(())
    }
    fn discover_session_id(&mut self) -> Result<String, CandidateRuntimeError> {
        let (conn, key_line) = self.stored()?; let conn = conn.clone(); let key_line = key_line.to_string();
        let text = run_ssh_command(conn.endpoint(), conn.username(), conn.secret(), &key_line, &["cat", SESSION_PATH], SSH_TIMEOUT).map_err(|_| CandidateRuntimeError::Transport)?;
        parse_session_binding(text.as_bytes()).map_err(|_| CandidateRuntimeError::Transport)
    }
    fn write_hello(&mut self, session_id: &str) -> Result<(), CandidateRuntimeError> {
        let (conn, key_line) = self.stored()?;
        let key_line = key_line.to_string();
        let client = SshControlClient::new(session_id, conn.endpoint().clone(), conn.username().clone(), conn.secret().clone(), key_line.clone());
        let body = serde_json::json!({ "v": 1, "session_id": session_id }).to_string();
        run_ssh_command_with_stdin(conn.endpoint(), conn.username(), conn.secret(), &key_line, &["tee", HELLO_PATH], Some(body.as_bytes()), SSH_TIMEOUT).map_err(|_| CandidateRuntimeError::Transport)?;
        self.client = Some(client);
        Ok(())
    }
    fn write_final_ack(&mut self, session_id: &str) -> Result<(), CandidateRuntimeError> { let (conn, key_line) = self.stored()?; let key_line = key_line.to_string(); let body = final_ack_bytes(session_id); run_ssh_command_with_stdin(conn.endpoint(), conn.username(), conn.secret(), &key_line, &["tee", FINAL_ACK_PATH], Some(body.as_slice()), SSH_TIMEOUT).map_err(|_| CandidateRuntimeError::Transport)?; Ok(()) }
    fn write_heartbeat(&mut self, session_id: &str, at_ms: u64) -> Result<(), CandidateRuntimeError> { let (conn, key_line) = self.stored()?; let key_line = key_line.to_string(); let body = heartbeat_bytes(session_id, at_ms); run_ssh_command_with_stdin(conn.endpoint(), conn.username(), conn.secret(), &key_line, &["tee", HEARTBEAT_PATH], Some(body.as_slice()), SSH_TIMEOUT).map_err(|_| CandidateRuntimeError::Transport)?; Ok(()) }
    fn start_sync(&mut self, session_id: &str, alpha: &Path) -> Result<(), CandidateRuntimeError> {
        let (conn, key_line) = self.stored()?;
        let conn = conn.clone();
        let key_line = key_line.to_string();
        let name = sync_name(session_id)?;
        let alpha_text = alpha.to_string_lossy().into_owned();
        crate::sync::validate_alpha_dir(&alpha_text).map_err(|_| CandidateRuntimeError::SyncFailed)?;
        std::fs::create_dir_all(alpha).map_err(|_| CandidateRuntimeError::SyncFailed)?;
        if self.askpass.is_none() { self.askpass = Some(AskpassSession::create("candidate").map_err(|_| CandidateRuntimeError::SyncFailed)?); }
        let askpass = self.askpass.as_ref().ok_or(CandidateRuntimeError::SyncFailed)?;
        let env = MutagenAuthEnv::prepare(askpass, &key_line).map_err(|_| CandidateRuntimeError::SyncFailed)?;
        let feeder = env.start_password_feeder(conn.secret());
        let started = run_mutagen(&env, &["daemon", "start"], Duration::from_secs(20)).map(|o| o.success()).unwrap_or(false);
        let beta = crate::sync::beta_url(conn.endpoint(), conn.username());
        let created = if started { run_mutagen(&env, &["sync", "create", "--name", &name, "--mode", "two-way-safe", "--symlink-mode", "ignore", &alpha_text, &beta], Duration::from_secs(60)).map(|o| o.success()).unwrap_or(false) } else { false };
        if !created { feeder.stop(); return Err(CandidateRuntimeError::SyncFailed); }
        self.sync_env = Some(env);
        self.feeder = Some(feeder);
        Ok(())
    }
    fn send_intent(&mut self, intent: &Intent) -> Result<(), CandidateRuntimeError> {
        let client = self.client.as_ref().ok_or(CandidateRuntimeError::Transport)?;
        client.send_intent(intent).map_err(|_| CandidateRuntimeError::Transport)
    }
    fn read_snapshot(&mut self) -> Result<Option<CandidateSnapshot>, CandidateRuntimeError> {
        let Some(client) = self.client.as_ref() else { return Ok(None); };
        match client.fetch_snapshot() {
            Ok(snapshot) => Ok(Some(snapshot)),
            Err(crate::orchestration::control::ControlError::WrongSession) => Err(CandidateRuntimeError::WrongSession),
            Err(_) => Ok(None),
        }
    }
    fn flush_sync(&mut self, session_id: &str) -> Result<(), CandidateRuntimeError> {
        let Some(env) = self.sync_env.as_ref() else { return Ok(()); };
        let name = sync_name(session_id)?;
        match run_mutagen(env, &["sync", "flush", &name], Duration::from_secs(30)) { Ok(o) if o.success() => Ok(()), _ => Err(CandidateRuntimeError::SyncFailed) }
    }
    fn terminate_sync(&mut self, session_id: &str) -> Result<(), CandidateRuntimeError> {
        let mut result = Ok(());
        if let Some(env) = self.sync_env.as_ref() {
            if let Ok(name) = sync_name(session_id) {
                if !matches!(run_mutagen(env, &["sync", "terminate", &name], Duration::from_secs(30)), Ok(o) if o.success()) { result = Err(CandidateRuntimeError::SyncFailed); }
            }
            let _ = run_mutagen(env, &["daemon", "stop"], Duration::from_secs(15));
        }
        if let Some(feeder) = self.feeder.take() { feeder.stop(); }
        self.sync_env = None;
        result
    }
}
#[rustfmt::skip] pub struct SshAuthVerifier { stash: Arc<Mutex<Option<(ConnectionString, String)>>> }
#[rustfmt::skip]
impl SshAuthVerifier {
    pub fn new() -> Self { Self { stash: Arc::new(Mutex::new(None)) } }
    pub fn stash(&self) -> Arc<Mutex<Option<(ConnectionString, String)>>> { Arc::clone(&self.stash) }
}
#[rustfmt::skip] impl Default for SshAuthVerifier { fn default() -> Self { Self::new() } }
#[rustfmt::skip]
impl ConnectionVerifier for SshAuthVerifier {
    fn verify(&self, c: &ConnectionString, _now_ms: u64) -> Result<(), CandidateConnectError> {
        let line = verify_and_key_line(c.endpoint(), c.fingerprint()).map_err(|e| candidate_connect_error_for_ssh(&e))?;
        run_ssh_command(c.endpoint(), c.username(), c.secret(), &line, &["true"], SSH_TIMEOUT).map_err(|e| candidate_connect_error_for_ssh(&e))?;
        if let Ok(mut slot) = self.stash.lock() { *slot = Some((c.clone(), line)); }
        Ok(())
    }
}
/// Local workspace synced to the container `/workspace` root.
#[rustfmt::skip] pub fn default_workspace() -> PathBuf { std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir()).join("pairproof-workspace") }
/// Explicit test-only binding override; production discovery never uses it.
#[rustfmt::skip]
pub fn production_session_id(connection: &ConnectionString) -> String {
    if let Ok(value) = std::env::var("PAIRPROOF_SESSION_ID") {
        let value = value.trim().to_string();
        if crate::sandbox::validate::validate_session_id(&value).is_ok() { return value; }
    }
    format!("cand-{}", connection.username().as_str().trim_start_matches("pp-"))
}
#[rustfmt::skip] fn pop_last(text: &str) -> String { let mut owned = text.to_string(); owned.pop(); owned }
#[rustfmt::skip]
fn apply(status: &mut Option<String>, result: Result<Intent, CandidateRuntimeError>) -> bool {
    match result { Ok(_) => { *status = None; true }, Err(err) => { *status = Some(err.to_string()); false } }
}
#[rustfmt::skip]
fn live_lines(state: &CandidateRunState, runtime: &CandidateRuntime<SshCandidateTransport>, status: &Option<String>) -> Vec<String> {
    let view = runtime.view();
    let mut lines = view.lines();
    if runtime.connection_lost() { lines.push(CONNECTION_LOST.to_string()); lines.push("Press R to reconnect with a fresh string.".to_string()); }
    lines.extend([format!("Other: {}", state.work.other_text()), format!("Redirect: {}", state.work.redirect_text()), format!("Message: {}", state.chat.input_text())]);
    if state.paste_notice || state.chat.paste_notice() { lines.push("Pasting is blocked here; type instead.".to_string()); }
    lines.push(if view.is_terminal() { "Enter quit".to_string() } else { "Focus: work/other/redirect/chat (Tab switches)".to_string() });
    if let Some(text) = status { lines.push(text.clone()); }
    lines.iter().map(|line| sanitize_display(line)).collect()
}
#[rustfmt::skip]
fn handle_live_key(state: &mut CandidateRunState, runtime: &mut CandidateRuntime<SshCandidateTransport>, key: KeyEvent, status: &mut Option<String>) {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code { K::Esc => { state.app.quit(); return; } K::Char('c') if control => { state.app.quit(); return; } _ => {} }
    if runtime.view().is_terminal() { match key.code { K::Enter => { state.app.quit(); }, K::Char('q') if !control => { state.app.quit(); }, _ => {} } return; }
    match state.focus {
        CandidateFocus::Work => match key.code {
            K::Tab => state.focus = CandidateFocus::Other,
            K::Char(c) if !control => match c {
                '1'..='4' => { apply(status, runtime.send_intent(IntentKind::SelectPreset { preset: c as u8 - b'1' })); } '5' | 'o' => state.focus = CandidateFocus::Other,
                'c' => { apply(status, runtime.send_intent(IntentKind::Continue)); } 'd' => state.focus = CandidateFocus::Redirect,
                't' => { apply(status, runtime.send_intent(IntentKind::RequestTests)); } 's' => { apply(status, runtime.send_intent(IntentKind::Skip)); }
                'f' => { if apply(status, runtime.send_intent(IntentKind::Finish)) { if let Err(err) = runtime.flush() { *status = Some(err.to_string()); } } }
                'r' => { apply(status, runtime.send_intent(IntentKind::RequestRequestion)); } 'q' => { state.app.quit(); }, _ => {}
            }, _ => {}
        },
        CandidateFocus::Other => match key.code {
            K::Tab => state.focus = CandidateFocus::Redirect,
            K::Enter => { let text = state.work.other_text().to_string(); if apply(status, runtime.send_intent(IntentKind::SelectOther { text })) { state.work.set_other_text(""); state.focus = CandidateFocus::Work; } }
            K::Backspace => { let text = pop_last(state.work.other_text()); state.work.set_other_text(&text); } K::Char(c) if !control => { let _ = other_insert(&mut state.work, &c.to_string(), false); } _ => {}
        },
        CandidateFocus::Redirect => match key.code {
            K::Tab => state.focus = CandidateFocus::Chat,
            K::Enter => { let text = state.work.redirect_text().to_string(); if apply(status, runtime.send_intent(IntentKind::Redirect { text })) { state.work.set_redirect_text(""); state.focus = CandidateFocus::Work; } }
            K::Backspace => { let text = pop_last(state.work.redirect_text()); state.work.set_redirect_text(&text); } K::Char(c) if !control => state.work.push_redirect_char(c), _ => {}
        },
        CandidateFocus::Chat => match key.code {
            K::Tab => state.focus = CandidateFocus::Work,
            K::Enter => { let text = state.chat.input_text().to_string(); if apply(status, runtime.send_chat(&text)) { state.chat.clear_input(); } }
            K::Backspace => state.chat.backspace_input(), K::Char(c) if !control => state.chat.type_char(c), _ => {}
        },
    }
}
#[rustfmt::skip]
fn fresh_verifier(state: &mut CandidateRunState) -> Arc<Mutex<Option<(ConnectionString, String)>>> {
    let verifier = SshAuthVerifier::new(); let stash = verifier.stash(); state.app.set_verifier(verifier); stash
}
/// Production TTY loop; each tick polls at most one bounded op.
#[rustfmt::skip]
pub fn candidate_loop(state: &mut CandidateRunState) -> Result<(), Box<dyn Error>> {
    let _guard = CandidateTtyGuard;
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen, crossterm::event::EnableBracketedPaste)?;
    let backend = ratatui::backend::CrosstermBackend::new(std::io::stdout());
    let mut terminal = ratatui::Terminal::new(backend)?;
    let mut stash = fresh_verifier(state);
    let mut runtime: Option<CandidateRuntime<SshCandidateTransport>> = None;
    let mut live = false;
    let mut status: Option<String> = None;
    while state.app.screen() != CandidateScreen::Exited {
        if live { if let Some(active) = runtime.as_mut() { if let Err(err) = active.poll_snapshot() { status = Some(err.to_string()); } } }
        let (title, lines) = if live {
            match runtime.as_ref() { Some(active) => (screen_title(CandidateScreen::Solving), live_lines(state, active, &status)), None => (screen_title(state.app.screen()), candidate_screen_lines(state)) }
        } else { let mut lines = candidate_screen_lines(state); if let Some(text) = &status { lines.push(text.clone()); } (screen_title(state.app.screen()), lines) };
        let _ = terminal.draw(|frame| {
            let widget = ratatui::widgets::Paragraph::new(lines.join("\n")).block(ratatui::widgets::Block::default().title(title).borders(ratatui::widgets::Borders::ALL));
            frame.render_widget(widget, frame.area());
        });
        if crossterm::event::poll(Duration::from_millis(250)).unwrap_or(false) {
            match crossterm::event::read() {
                Ok(Event::Key(key)) if key.kind != KeyEventKind::Release => {
                    if live {
                        let lost = runtime.as_ref().map(|active| active.connection_lost()).unwrap_or(false);
                        if lost && key.code == K::Char('R') && !key.modifiers.contains(KeyModifiers::CONTROL) {
                            live = false; *state = CandidateRunState::new(); stash = fresh_verifier(state);
                            status = Some("Paste the fresh connection string, then press Enter.".to_string());
                        } else if let Some(active) = runtime.as_mut() { handle_live_key(state, active, key, &mut status); }
                    } else {
                        let connect_enter = state.app.screen() == CandidateScreen::Connect && key.code == K::Enter;
                        handle_candidate_key(state, key);
                        if connect_enter && state.app.screen() == CandidateScreen::Rules {
                            let material = stash.lock().ok().and_then(|slot| slot.clone());
                            match material {
                                Some((conn, key_line)) => {
                                    if let Some(existing) = runtime.as_mut() {
                                        match existing.reconnect(&conn) { Ok(()) => { status = None; }, Err(err) => { status = Some(err.to_string()); *state = CandidateRunState::new(); stash = fresh_verifier(state); } }
                                    } else {
                                        let mut built = CandidateRuntime::new_discovering(default_workspace(), SshCandidateTransport::from_verified(&conn, key_line));
                                        match built.connect(&conn) { Ok(()) => { runtime = Some(built); status = None; }, Err(err) => { status = Some(err.to_string()); *state = CandidateRunState::new(); stash = fresh_verifier(state); } }
                                    }
                                }
                                None => { status = Some(CandidateRuntimeError::Transport.to_string()); *state = CandidateRunState::new(); stash = fresh_verifier(state); }
                            }
                        }
                        if runtime.is_some() && state.app.rules_accepted() { live = true; }
                    }
                }
                Ok(Event::Paste(text)) => handle_candidate_paste(state, &text),
                _ => {}
            }
        }
    }
    if let Some(active) = runtime.as_mut() { let _ = active.end(); }
    Ok(())
}
