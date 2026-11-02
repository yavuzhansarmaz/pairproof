//! Connection acceptance gate: host-key pinning before secret verification.

use std::fmt;
use std::time::Duration;

use crate::connection::{
    ConnectionError, ConnectionState, ConnectionString, Endpoint, EphemeralUsername, Fingerprint,
    SecretRotator,
};
use crate::core::{CoreError, EventKind, SessionStatus};
use crate::orchestration::error::OrchestratorError;
use crate::orchestration::process_util::keyscan_verified_line;
use crate::orchestration::session::Orchestrator;
use crate::orchestration::ssh_auth::SshAuthError;

/// Errors from the connection gate; payloads never carry secrets.
#[derive(Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConnectionGateError {
    /// The scanned host key does not match the pinned fingerprint.
    #[error("host key mismatch")]
    HostKeyMismatch,
    /// No usable host key could be obtained; detail is static text.
    #[error("host key unavailable")]
    HostKeyUnavailable(String),
    /// Password login has no safe non-interactive channel here.
    #[error("password authentication is not supported")]
    PasswordAuthUnsupported,
    /// Wraps a connection-string or rotating-secret error unchanged.
    #[error(transparent)]
    Connection(#[from] ConnectionError),
    /// Wraps an orchestrator error unchanged.
    #[error(transparent)]
    Orchestrator(#[from] OrchestratorError),
}

impl fmt::Debug for ConnectionGateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HostKeyMismatch => write!(f, "HostKeyMismatch"),
            Self::HostKeyUnavailable(_) => write!(f, "HostKeyUnavailable(\"[REDACTED]\")"),
            Self::PasswordAuthUnsupported => write!(f, "PasswordAuthUnsupported"),
            Self::Connection(err) => f.debug_tuple("Connection").field(err).finish(),
            Self::Orchestrator(err) => f.debug_tuple("Orchestrator").field(err).finish(),
        }
    }
}

/// Explicit phase of the connection gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatePhase {
    Awaiting,
    Locked,
    Ended,
    ReissueNeeded,
}

/// Narrow host-key boundary so gate logic is deterministic in tests.
pub trait HostKeyVerifier: fmt::Debug {
    /// Verifies the live host key at `endpoint` against `expected`.
    fn verify_host_key(
        &self,
        endpoint: &Endpoint,
        expected: &Fingerprint,
    ) -> Result<(), ConnectionGateError>;
}

/// Production verifier using `ssh-keyscan` over an argv array only.
/// Uses std::process with an explicit deadline because the trait is
/// synchronous, while the bounded async runner needs a runtime.
#[derive(Debug, Clone, Copy, Default)]
pub struct SshKeyscanVerifier;

fn unavailable(detail: &str) -> ConnectionGateError {
    ConnectionGateError::HostKeyUnavailable(detail.to_string())
}

/// Read-only mirror of core `candidate_connected` attach preconditions.
fn attach_precheck(orchestrator: &Orchestrator) -> Result<(), OrchestratorError> {
    if orchestrator.is_terminal() {
        return Err(CoreError::Terminal.into());
    }
    if orchestrator.is_connected() {
        return Err(OrchestratorError::AlreadyAttached);
    }
    if orchestrator.status() == SessionStatus::WaitingForCandidate {
        return Ok(());
    }
    let ever_connected = orchestrator.session().events().iter().any(|e| {
        matches!(
            e.kind,
            EventKind::CandidateConnected | EventKind::CandidateReconnected { .. }
        )
    });
    if ever_connected {
        Ok(())
    } else {
        Err(CoreError::InvalidState(format!(
            "candidate_connected in {:?}",
            orchestrator.status()
        ))
        .into())
    }
}

/// Maps the shared keyscan error onto the gate error type, preserving
/// the mismatch/unavailable distinction and the static detail text.
fn map_keyscan_error(err: SshAuthError) -> ConnectionGateError {
    match err {
        SshAuthError::HostKeyMismatch => ConnectionGateError::HostKeyMismatch,
        SshAuthError::HostKeyUnavailable(detail) => ConnectionGateError::HostKeyUnavailable(detail),
        _ => unavailable("ssh-keyscan could not be started"),
    }
}

impl HostKeyVerifier for SshKeyscanVerifier {
    fn verify_host_key(
        &self,
        endpoint: &Endpoint,
        expected: &Fingerprint,
    ) -> Result<(), ConnectionGateError> {
        // Shared keyscan with a 6 s outer timeout (SSH auth uses the
        // same scan with 8 s; see `verify_and_key_line`).
        keyscan_verified_line(endpoint, expected, Duration::from_secs(6))
            .map(|_| ())
            .map_err(map_keyscan_error)
    }
}

/// Owns the rotating secret and binds acceptance to the orchestrator.
#[derive(Debug)]
pub struct ConnectionGate {
    rotator: SecretRotator,
    verifier: Box<dyn HostKeyVerifier>,
    endpoint: Endpoint,
    fingerprint: Fingerprint,
    phase: GatePhase,
}

impl ConnectionGate {
    /// Creates a gate using the production `ssh-keyscan` verifier.
    pub fn new(
        endpoint: Endpoint,
        username: EphemeralUsername,
        fingerprint: Fingerprint,
        now_ms: u64,
    ) -> Self {
        Self::with_verifier(endpoint, username, fingerprint, SshKeyscanVerifier, now_ms)
    }

    /// Creates a gate with an explicit host-key verifier.
    pub fn with_verifier<V>(
        endpoint: Endpoint,
        username: EphemeralUsername,
        fingerprint: Fingerprint,
        verifier: V,
        now_ms: u64,
    ) -> Self
    where
        V: HostKeyVerifier + 'static,
    {
        let rotator = SecretRotator::new(endpoint.clone(), username, fingerprint.clone(), now_ms);
        Self {
            rotator,
            verifier: Box::new(verifier),
            endpoint,
            fingerprint,
            phase: GatePhase::Awaiting,
        }
    }

    /// Returns the current string, rotating first if its window elapsed.
    pub fn current_string(&mut self, now_ms: u64) -> Result<ConnectionString, ConnectionGateError> {
        match self.rotator.current_string(now_ms) {
            Ok(value) => {
                if self.phase != GatePhase::Locked && self.phase != GatePhase::Ended {
                    self.phase = GatePhase::Awaiting;
                }
                Ok(value)
            }
            Err(err) => {
                self.apply_error_phase(&err);
                Err(err.into())
            }
        }
    }

    /// Returns seconds left in the current rotation window.
    pub fn countdown_secs(&self, now_ms: u64) -> u64 {
        self.rotator.countdown_secs(now_ms)
    }

    /// Returns the explicit gate phase.
    pub fn phase(&self) -> GatePhase {
        self.phase
    }

    /// Returns the underlying rotator state.
    pub fn state(&self) -> ConnectionState {
        self.rotator.state()
    }

    /// Returns the pinned host-key fingerprint.
    pub fn fingerprint(&self) -> &Fingerprint {
        &self.fingerprint
    }

    /// Accepts a presented string: precheck, host key, secret, attach.
    pub fn accept(
        &mut self,
        orchestrator: &mut Orchestrator,
        presented: &ConnectionString,
        now_ms: u64,
    ) -> Result<(), ConnectionGateError> {
        if self.rotator.state() == ConnectionState::Ended {
            self.phase = GatePhase::Ended;
            return Err(ConnectionError::SessionEnded.into());
        }
        if self.rotator.state() == ConnectionState::Locked {
            return Err(ConnectionError::AlreadyLocked.into());
        }
        attach_precheck(orchestrator)?;
        self.verifier
            .verify_host_key(&self.endpoint, &self.fingerprint)?;
        if let Err(err) = self.rotator.verify(presented, now_ms) {
            self.apply_error_phase(&err);
            return Err(err.into());
        }
        if let Err(err) = orchestrator.attach_candidate(now_ms) {
            let _ = self.rotator.reissue(now_ms);
            self.phase = GatePhase::Awaiting;
            return Err(err.into());
        }
        self.phase = GatePhase::Locked;
        Ok(())
    }

    /// Accepts a candidate whose SSH password authentication already
    /// proved possession of the current secret (expert-side auth log /
    /// control hello in 10I-2). No host-key scan runs here: the candidate
    /// authenticated to the sandbox whose fingerprint the expert pinned
    /// in the displayed string. The gate verifies its own current string
    /// to lock the rotator, then attaches exactly once.
    pub fn accept_after_ssh_auth(
        &mut self,
        orchestrator: &mut Orchestrator,
        now_ms: u64,
    ) -> Result<(), ConnectionGateError> {
        if self.rotator.state() == ConnectionState::Ended {
            self.phase = GatePhase::Ended;
            return Err(ConnectionError::SessionEnded.into());
        }
        if self.rotator.state() == ConnectionState::Locked {
            return Err(ConnectionError::AlreadyLocked.into());
        }
        attach_precheck(orchestrator)?;
        let current = self.current_string(now_ms)?;
        if let Err(err) = self.rotator.verify(&current, now_ms) {
            self.apply_error_phase(&err);
            return Err(err.into());
        }
        if let Err(err) = orchestrator.attach_candidate(now_ms) {
            let _ = self.rotator.reissue(now_ms);
            self.phase = GatePhase::Awaiting;
            return Err(err.into());
        }
        self.phase = GatePhase::Locked;
        Ok(())
    }

    /// Disconnects the candidate; the rotator stays locked for reissue.
    pub fn handle_disconnect(
        &mut self,
        orchestrator: &mut Orchestrator,
        now_ms: u64,
    ) -> Result<(), ConnectionGateError> {
        orchestrator.disconnect_candidate(now_ms)?;
        self.phase = GatePhase::ReissueNeeded;
        Ok(())
    }

    /// Issues a fresh string as an explicit expert action after disconnect.
    pub fn reissue_after_disconnect(
        &mut self,
        now_ms: u64,
    ) -> Result<ConnectionString, ConnectionGateError> {
        match self.rotator.reissue(now_ms) {
            Ok(value) => {
                self.phase = GatePhase::Awaiting;
                Ok(value)
            }
            Err(err) => {
                self.apply_error_phase(&err);
                Err(err.into())
            }
        }
    }

    /// Ends the session permanently in both the gate and the orchestrator.
    pub fn terminate(
        &mut self,
        orchestrator: &mut Orchestrator,
        now_ms: u64,
    ) -> Result<(), ConnectionGateError> {
        self.rotator.terminate();
        self.phase = GatePhase::Ended;
        orchestrator.end_by_expert(now_ms)?;
        Ok(())
    }

    /// Password login has no safe non-interactive channel in this build.
    /// It never succeeds instead of placing the secret in argv or the environment.
    pub fn authenticate_password(&self) -> Result<(), ConnectionGateError> {
        Err(ConnectionGateError::PasswordAuthUnsupported)
    }

    fn apply_error_phase(&mut self, err: &ConnectionError) {
        match err {
            ConnectionError::SessionEnded => self.phase = GatePhase::Ended,
            ConnectionError::Expired => self.phase = GatePhase::ReissueNeeded,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_error_debug_and_display_redact_canary() {
        let canary = "0123456789abcdeffedcba9876543210";
        let err = ConnectionGateError::HostKeyUnavailable(canary.to_string());
        assert!(!format!("{err:?}").contains(canary));
        assert!(!format!("{err}").contains(canary));
        let endpoint = Endpoint::new("203.0.113.10", 22022).expect("endpoint");
        let username = EphemeralUsername::parse("pp-a1b2c3d4").expect("username");
        let fingerprint = Fingerprint::parse(&"a".repeat(64)).expect("fingerprint");
        let mut gate = ConnectionGate::new(endpoint, username, fingerprint, 0);
        let live = gate.current_string(0).expect("current string");
        let secret_hex = live.secret().expose_hex();
        assert!(!format!("{gate:?}").contains(&secret_hex));
        assert!(!format!("{live:?}").contains(&secret_hex));
    }
}
