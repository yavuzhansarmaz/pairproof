//! Candidate-side control client: sends intents and reads snapshots.
//! Intent text travels only as JSON data on SSH stdin, never in argv.

use std::time::Duration;

use crate::connection::{Endpoint, EphemeralUsername, Secret};
use crate::orchestration::control::{
    parse_snapshot, CandidateSnapshot, ControlError, Intent, IntentKind,
};
use crate::orchestration::ssh_auth::{run_ssh_command, run_ssh_command_with_stdin};

/// Container path of the candidate-writable intent log.
pub const REMOTE_INTENT_PATH: &str = "/pairproof-control/inbox/intents.jsonl";
/// Container path of the expert-published snapshot.
pub const REMOTE_SNAPSHOT_PATH: &str = "/pairproof-control/outbox/snapshot.json";
const SSH_TIMEOUT: Duration = Duration::from_secs(20);

/// Session-bound intent sequencer held by the candidate.
#[derive(Debug, Clone)]
pub struct CandidateControlClient {
    session_id: String,
    next_seq: u64,
}
impl CandidateControlClient {
    /// Creates a client whose first intent uses sequence 1.
    pub fn new(session_id: &str) -> Self {
        Self {
            session_id: session_id.into(),
            next_seq: 1,
        }
    }
    /// Returns the bound session id.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    /// Returns the sequence the next prepared intent will use.
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }
    /// Builds the next sequenced intent without sending it.
    pub fn prepare(&mut self, kind: IntentKind) -> Intent {
        let intent = Intent::new(&self.session_id, self.next_seq, kind);
        self.next_seq += 1;
        intent
    }
    /// Serializes one intent as an append-ready JSONL line with newline.
    pub fn intent_line(intent: &Intent) -> Result<String, ControlError> {
        Ok(format!("{}\n", intent.to_line()?))
    }
    /// Parses snapshot bytes, rejecting a wrong session or version.
    pub fn read_snapshot(&self, bytes: &[u8]) -> Result<CandidateSnapshot, ControlError> {
        parse_snapshot(bytes, &self.session_id)
    }
}

/// SSH-backed candidate client using the 10G FIFO askpass session.
#[derive(Debug)]
pub struct SshControlClient {
    inner: CandidateControlClient,
    endpoint: Endpoint,
    user: EphemeralUsername,
    secret: Secret,
    verified_key_line: String,
}
impl SshControlClient {
    /// Creates an SSH client bound to one session and pinned host key.
    pub fn new(
        session_id: &str,
        endpoint: Endpoint,
        user: EphemeralUsername,
        secret: Secret,
        verified_key_line: String,
    ) -> Self {
        Self {
            inner: CandidateControlClient::new(session_id),
            endpoint,
            user,
            secret,
            verified_key_line,
        }
    }
    /// Returns the inner sequencer.
    pub fn inner(&self) -> &CandidateControlClient {
        &self.inner
    }
    /// Returns the inner sequencer mutably.
    pub fn inner_mut(&mut self) -> &mut CandidateControlClient {
        &mut self.inner
    }
    /// Appends one intent line through SSH stdin (`tee -a`, argv only).
    pub fn send_intent(&self, intent: &Intent) -> Result<(), ControlError> {
        intent.validate(self.inner.session_id())?;
        let line = CandidateControlClient::intent_line(intent)?;
        run_ssh_command_with_stdin(
            &self.endpoint,
            &self.user,
            &self.secret,
            &self.verified_key_line,
            &["tee", "-a", REMOTE_INTENT_PATH],
            Some(line.as_bytes()),
            SSH_TIMEOUT,
        )
        .map_err(|e| ControlError::Io(format!("{e:?}")))?;
        Ok(())
    }
    /// Prepares the next intent and appends it through SSH stdin.
    pub fn send_next(&mut self, kind: IntentKind) -> Result<Intent, ControlError> {
        let intent = self.inner.prepare(kind);
        self.send_intent(&intent)?;
        Ok(intent)
    }
    /// Reads and validates the expert snapshot through SSH.
    pub fn fetch_snapshot(&self) -> Result<CandidateSnapshot, ControlError> {
        let text = run_ssh_command(
            &self.endpoint,
            &self.user,
            &self.secret,
            &self.verified_key_line,
            &["cat", REMOTE_SNAPSHOT_PATH],
            SSH_TIMEOUT,
        )
        .map_err(|e| ControlError::Io(format!("{e:?}")))?;
        self.inner.read_snapshot(text.as_bytes())
    }
}
