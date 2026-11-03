//! Candidate runtime: SSH auth, hello, isolated Mutagen sync, intents,
//! and snapshot rendering. Candidate failures never carry secrets.
use crate::connection::ConnectionString;
use crate::orchestration::control::{
    final_ack_bytes, heartbeat_bytes, hello_bytes, parse_session_binding_full, CandidateSnapshot,
    Intent, IntentKind, SnapshotFinal, SnapshotOption, SnapshotRecovery, SnapshotTest,
    FINAL_ACK_PATH, HEARTBEAT_PATH, SESSION_PATH,
};
use crate::orchestration::control_client::{CandidateControlClient, SshControlClient};
use crate::orchestration::mutagen_auth::{run_mutagen, MutagenAuthEnv, PasswordFeeder};
use crate::orchestration::ssh_auth::{
    run_ssh_command, run_ssh_command_with_stdin, verify_and_key_line, AskpassSession, SshAuthError,
};
use crate::tui::candidate::{CandidateConnectError, CandidateScreen, ConnectionVerifier};
use crate::tui::candidate_chat::other_insert;
use crate::tui::candidate_run::{
    candidate_screen_lines, handle_candidate_key, handle_candidate_paste, next_focus, now_ms,
    work_key_action, CandidateFocus, CandidateRunState, CandidateTtyGuard, WorkKeyAction,
};
use crate::tui::shared::sanitize_display;
use crossterm::event::{Event, KeyCode as K, KeyEvent, KeyEventKind, KeyModifiers};
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
const HELLO_PATH: &str = "/pairproof-control/inbox/hello.json";
const SSH_TIMEOUT: Duration = Duration::from_secs(20);
const HEARTBEAT_THROTTLE_MS: u64 = 5_000;
pub const CONNECTION_LOST: &str = "connection lost \u{2014} retrying";
mod live;
mod panes;
mod transport;
mod view;
pub use self::live::{candidate_loop, default_workspace, handle_live_key};
pub use self::transport::{SshAuthVerifier, SshCandidateTransport};
pub use self::view::CandidateView;
/// Builds or reconnects the runtime once a verified string lands
/// on Rules; failures reset to a fresh Connect state.
fn establish_runtime_after_rules(
    state: &mut CandidateRunState,
    runtime: &mut Option<CandidateRuntime<SshCandidateTransport>>,
    stash: &mut std::sync::Arc<std::sync::Mutex<Option<(ConnectionString, String)>>>,
    status: &mut Option<String>,
) {
    let material = stash.lock().ok().and_then(|slot| slot.clone());
    match material {
        Some((conn, key_line)) => {
            if let Some(existing) = runtime.as_mut() {
                match existing.reconnect(&conn) {
                    Ok(()) => {
                        *status = None;
                    }
                    Err(err) => {
                        *status = Some(crate::tui::help::candidate_error_message(&err).to_string());
                        *state = CandidateRunState::new();
                        *stash = self::live::fresh_verifier(state);
                    }
                }
            } else {
                let mut built = CandidateRuntime::new_discovering(
                    default_workspace(),
                    SshCandidateTransport::from_verified(&conn, key_line),
                );
                match built.connect(&conn) {
                    Ok(()) => {
                        *runtime = Some(built);
                        *status = None;
                    }
                    Err(err) => {
                        *status = Some(crate::tui::help::candidate_error_message(&err).to_string());
                        *state = CandidateRunState::new();
                        *stash = self::live::fresh_verifier(state);
                    }
                }
            }
        }
        None => {
            *status = Some(
                crate::tui::help::candidate_error_message(&CandidateRuntimeError::Transport)
                    .to_string(),
            );
            *state = CandidateRunState::new();
            *stash = self::live::fresh_verifier(state);
        }
    }
}

/// Explicit runtime phase; work is reachable only when Connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateRuntimePhase {
    Disconnected,
    Authenticating,
    Syncing,
    Connected,
    Ended,
}
/// Candidate-safe runtime failure; no variant carries transport detail.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CandidateRuntimeError {
    #[error("{0}")]
    Auth(#[from] CandidateConnectError),
    #[error("sync problem \u{2014} ask the expert")]
    SyncFailed,
    #[error("control message bound to another session")]
    WrongSession,
    #[error("candidate is not connected")]
    NotConnected,
    #[error("session has ended")]
    Ended,
    #[error("connection problem \u{2014} ask the expert")]
    Transport,
    #[error("PairProof version mismatch: candidate is running {candidate}, expert is running {expert}; both must run the same version - update/restart PairProof and reconnect")]
    VersionMismatch { candidate: String, expert: String },
}
/// Maps any SSH-auth failure to generic rejection; payloads are dropped.
pub fn candidate_connect_error_for_ssh(_err: &SshAuthError) -> CandidateConnectError {
    CandidateConnectError::Rejected
}
/// Control-channel boundary; production is [`SshCandidateTransport`].
pub trait CandidateTransport {
    fn authenticate(&mut self, connection: &ConnectionString) -> Result<(), CandidateRuntimeError>;
    fn discover_session_id(&mut self) -> Result<String, CandidateRuntimeError> {
        Err(CandidateRuntimeError::Transport)
    }
    fn expert_version(&self) -> Option<String> {
        None
    }
    fn write_hello(&mut self, session_id: &str) -> Result<(), CandidateRuntimeError>;
    fn start_sync(&mut self, session_id: &str, alpha: &Path) -> Result<(), CandidateRuntimeError>;
    fn send_intent(&mut self, intent: &Intent) -> Result<(), CandidateRuntimeError>;
    fn write_heartbeat(
        &mut self,
        _session_id: &str,
        _at_ms: u64,
    ) -> Result<(), CandidateRuntimeError> {
        Ok(())
    }
    fn read_snapshot(&mut self) -> Result<Option<CandidateSnapshot>, CandidateRuntimeError>;
    fn write_final_ack(&mut self, _session_id: &str) -> Result<(), CandidateRuntimeError> {
        Ok(())
    }
    fn flush_sync(&mut self, session_id: &str) -> Result<(), CandidateRuntimeError>;
    fn terminate_sync(&mut self, session_id: &str) -> Result<(), CandidateRuntimeError>;
}
/// Snapshot-only render data; the snapshot type has no hidden fields.
/// Snapshot-driven runtime; intent seq and ids are managed internally.
pub struct CandidateRuntime<T: CandidateTransport> {
    transport: T,
    session_id: String,
    workspace: PathBuf,
    sequencer: CandidateControlClient,
    phase: CandidateRuntimePhase,
    last_snapshot: Option<CandidateSnapshot>,
    chat_seq: u64,
    ack_sent: bool,
    lost: bool,
    last_hb_ms: u64,
    recovery_view: Option<SnapshotRecovery>,
    last_recovery_generation: u64,
}
impl<T: CandidateTransport> CandidateRuntime<T> {
    pub fn new(session_id: impl Into<String>, workspace: PathBuf, transport: T) -> Self {
        let session_id = session_id.into();
        Self {
            sequencer: CandidateControlClient::new(&session_id),
            transport,
            session_id,
            workspace,
            phase: CandidateRuntimePhase::Disconnected,
            last_snapshot: None,
            chat_seq: 0,
            ack_sent: false,
            lost: false,
            last_hb_ms: 0,
            recovery_view: None,
            last_recovery_generation: 0,
        }
    }
    pub fn new_discovering(workspace: PathBuf, transport: T) -> Self {
        Self::new("", workspace, transport)
    }
    pub fn phase(&self) -> CandidateRuntimePhase {
        self.phase
    }
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }
    pub fn transport(&self) -> &T {
        &self.transport
    }
    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }
    pub fn last_snapshot(&self) -> Option<&CandidateSnapshot> {
        self.last_snapshot.as_ref()
    }
    pub fn recovery_view(&self) -> Option<&SnapshotRecovery> {
        self.recovery_view.as_ref()
    }
    pub fn last_recovery_generation(&self) -> u64 {
        self.last_recovery_generation
    }
    pub fn final_ack_sent(&self) -> bool {
        self.ack_sent
    }
    pub fn connection_lost(&self) -> bool {
        self.lost
    }
    pub fn status_text(&self) -> Option<String> {
        self.lost.then(|| CONNECTION_LOST.to_string())
    }
    /// Authenticates, writes hello, then starts sync, in that order.
    pub fn connect(&mut self, connection: &ConnectionString) -> Result<(), CandidateRuntimeError> {
        if self.phase == CandidateRuntimePhase::Ended {
            return Err(CandidateRuntimeError::Ended);
        }
        if self.phase == CandidateRuntimePhase::Connected {
            return Ok(());
        }
        self.phase = CandidateRuntimePhase::Authenticating;
        if let Err(err) = self.transport.authenticate(connection) {
            self.phase = CandidateRuntimePhase::Disconnected;
            return Err(err);
        }
        match self.transport.discover_session_id() {
            Ok(id)
                if !id.trim().is_empty()
                    && crate::sandbox::validate::validate_session_id(&id).is_ok()
                    && (self.session_id.is_empty() || self.session_id == id) =>
            {
                self.session_id = id.clone();
                self.sequencer = CandidateControlClient::new(&id);
            }
            _ => {
                self.phase = CandidateRuntimePhase::Disconnected;
                return Err(CandidateRuntimeError::Transport);
            }
        }
        if let Some(ev) = self.transport.expert_version() {
            if ev.trim() != env!("CARGO_PKG_VERSION") {
                self.phase = CandidateRuntimePhase::Disconnected;
                return Err(CandidateRuntimeError::VersionMismatch {
                    candidate: env!("CARGO_PKG_VERSION").to_string(),
                    expert: ev.trim().to_string(),
                });
            }
        }
        if let Err(err) = self.transport.write_hello(&self.session_id) {
            self.phase = CandidateRuntimePhase::Disconnected;
            return Err(err);
        }
        self.phase = CandidateRuntimePhase::Syncing;
        self.transport
            .start_sync(&self.session_id, &self.workspace)?;
        self.phase = CandidateRuntimePhase::Connected;
        Ok(())
    }
    pub fn send_intent(&mut self, kind: IntentKind) -> Result<Intent, CandidateRuntimeError> {
        if self.phase == CandidateRuntimePhase::Ended {
            return Err(CandidateRuntimeError::Ended);
        }
        if self.phase != CandidateRuntimePhase::Connected {
            return Err(CandidateRuntimeError::NotConnected);
        }
        let intent = self.sequencer.prepare(kind);
        intent
            .validate(&self.session_id)
            .map_err(|_| CandidateRuntimeError::Transport)?;
        match self.transport.send_intent(&intent) {
            Ok(()) => Ok(intent),
            Err(err) => {
                if err == CandidateRuntimeError::Transport {
                    self.lost = true;
                }
                Err(err)
            }
        }
    }
    pub fn send_chat(&mut self, text: &str) -> Result<Intent, CandidateRuntimeError> {
        self.chat_seq += 1;
        self.send_intent(IntentKind::ChatRequest {
            chat_id: format!("chat-{}", self.chat_seq),
            text: text.to_string(),
        })
    }
    /// Reads one snapshot; wrong-session keeps the last good snapshot.
    pub fn poll_snapshot(&mut self) -> Result<Option<CandidateSnapshot>, CandidateRuntimeError> {
        self.poll_snapshot_at(now_ms())
    }
    /// Polls with an explicit clock: one snapshot read, then a throttled
    /// heartbeat. The attempt time is recorded even when the heartbeat
    /// fails, so a dead transport is not retried on every poll tick.
    /// A transport failure marks the connection lost and keeps the last snapshot.
    pub fn poll_snapshot_at(
        &mut self,
        now_ms: u64,
    ) -> Result<Option<CandidateSnapshot>, CandidateRuntimeError> {
        if self.phase == CandidateRuntimePhase::Ended {
            return Err(CandidateRuntimeError::Ended);
        }
        if self.phase != CandidateRuntimePhase::Connected {
            return Err(CandidateRuntimeError::NotConnected);
        }
        let snapshot = self.transport.read_snapshot();
        if now_ms.saturating_sub(self.last_hb_ms) > HEARTBEAT_THROTTLE_MS {
            self.last_hb_ms = now_ms;
            let _ = self.transport.write_heartbeat(&self.session_id, now_ms);
        }
        match snapshot {
            Err(CandidateRuntimeError::Transport) => {
                self.lost = true;
                Err(CandidateRuntimeError::Transport)
            }
            Err(err) => Err(err),
            Ok(None) => Ok(None),
            Ok(Some(snapshot)) => {
                if snapshot.session_id != self.session_id {
                    return Err(CandidateRuntimeError::WrongSession);
                }
                self.lost = false;
                match &snapshot.recovery {
                    None => self.recovery_view = None,
                    Some(r) if r.generation >= self.last_recovery_generation => {
                        self.last_recovery_generation = r.generation;
                        self.recovery_view = Some(r.clone());
                    }
                    Some(_) => {}
                }
                if snapshot.final_breakdown.is_some() && !self.ack_sent {
                    self.ack_sent = true;
                    let _ = self.transport.write_final_ack(&self.session_id);
                }
                self.last_snapshot = Some(snapshot.clone());
                Ok(Some(snapshot))
            }
        }
    }
    /// Reconnects with a fresh string: drops stale sync, then full connect.
    pub fn reconnect(
        &mut self,
        connection: &ConnectionString,
    ) -> Result<(), CandidateRuntimeError> {
        if self.phase == CandidateRuntimePhase::Ended {
            return Err(CandidateRuntimeError::Ended);
        }
        let _ = self.transport.terminate_sync(&self.session_id);
        self.phase = CandidateRuntimePhase::Disconnected;
        self.lost = false;
        self.connect(connection)
    }
    pub fn flush(&mut self) -> Result<(), CandidateRuntimeError> {
        if self.phase != CandidateRuntimePhase::Connected {
            return Err(CandidateRuntimeError::NotConnected);
        }
        self.transport.flush_sync(&self.session_id)
    }
    /// Terminates sync and ends; local files are preserved.
    pub fn end(&mut self) -> Result<(), CandidateRuntimeError> {
        if self.phase == CandidateRuntimePhase::Ended {
            return Ok(());
        }
        let result = self.transport.terminate_sync(&self.session_id);
        self.phase = CandidateRuntimePhase::Ended;
        result
    }
    pub fn view(&self) -> CandidateView {
        let mut view = CandidateView::from_snapshot(self.last_snapshot.as_ref());
        view.recovery = self.recovery_view.clone();
        view
    }
}
