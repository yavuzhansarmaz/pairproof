//! Expert connection controller beneath rendering; delegates to rotator.

use crate::connection::{
    ConnectionError, ConnectionState, ConnectionString, Endpoint, EphemeralUsername, Fingerprint,
    SecretRotator,
};
use crate::tui::shared::sanitize_display;

/// Explicit phase of the expert connection flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionPhase {
    Idle,
    AwaitingCandidate,
    Locked,
    Ended,
    ReissueNeeded,
}

/// Returns true when `err` is the expired-window error.
pub fn error_is_expired(err: &ConnectionError) -> bool {
    matches!(err, ConnectionError::Expired)
}

/// Expert connection controller owning the rotating secret.
#[derive(Debug)]
pub struct ConnectionController {
    rotator: SecretRotator,
    fingerprint: Fingerprint,
    current: Option<ConnectionString>,
    phase: ConnectionPhase,
}

impl ConnectionController {
    /// Creates a controller and primes the first connection string.
    pub fn new(
        endpoint: Endpoint,
        username: EphemeralUsername,
        fingerprint: Fingerprint,
        now_ms: u64,
    ) -> Self {
        let mut rotator = SecretRotator::new(endpoint, username, fingerprint.clone(), now_ms);
        let current = rotator.current_string(now_ms).ok();
        Self {
            rotator,
            fingerprint,
            current,
            phase: ConnectionPhase::AwaitingCandidate,
        }
    }

    /// Returns the current string, rotating first if its window elapsed.
    pub fn current_string(&mut self, now_ms: u64) -> Result<ConnectionString, ConnectionError> {
        match self.rotator.current_string(now_ms) {
            Ok(value) => {
                self.current = Some(value.clone());
                if self.phase != ConnectionPhase::Locked && self.phase != ConnectionPhase::Ended {
                    self.phase = ConnectionPhase::AwaitingCandidate;
                }
                Ok(value)
            }
            Err(err) => {
                self.apply_error_phase(&err);
                Err(err)
            }
        }
    }

    /// Returns the sanitized canonical string for expert display.
    pub fn display_string(&mut self, now_ms: u64) -> Result<String, ConnectionError> {
        let value = self.current_string(now_ms)?;
        Ok(sanitize_display(&value.encode()))
    }

    /// Returns seconds left in the current window via the rotator.
    pub fn countdown_secs(&self, now_ms: u64) -> u64 {
        self.rotator.countdown_secs(now_ms)
    }

    /// Returns true when the window elapsed while still rotating.
    pub fn is_expired(&self, now_ms: u64) -> bool {
        self.rotator.state() == ConnectionState::Rotating
            && self.rotator.countdown_secs(now_ms) == 0
    }

    /// Verifies a presented string; only success locks the connection.
    pub fn on_candidate_connected(
        &mut self,
        presented: &ConnectionString,
        now_ms: u64,
    ) -> Result<(), ConnectionError> {
        match self.rotator.verify(presented, now_ms) {
            Ok(()) => {
                self.phase = ConnectionPhase::Locked;
                self.current = None;
                Ok(())
            }
            Err(err) => {
                if matches!(err, ConnectionError::Expired) {
                    self.phase = ConnectionPhase::ReissueNeeded;
                } else {
                    self.apply_error_phase(&err);
                }
                Err(err)
            }
        }
    }

    /// Reissues after an accidental disconnect, invalidating the old secret.
    pub fn on_accidental_disconnect(
        &mut self,
        now_ms: u64,
    ) -> Result<ConnectionString, ConnectionError> {
        self.phase = ConnectionPhase::ReissueNeeded;
        self.reissue(now_ms)
    }

    /// Issues a fresh string with a full window; earlier secrets die.
    pub fn reissue(&mut self, now_ms: u64) -> Result<ConnectionString, ConnectionError> {
        match self.rotator.reissue(now_ms) {
            Ok(value) => {
                self.current = Some(value.clone());
                self.phase = ConnectionPhase::AwaitingCandidate;
                Ok(value)
            }
            Err(err) => {
                self.apply_error_phase(&err);
                Err(err)
            }
        }
    }

    /// Ends the session permanently; safe to call more than once.
    pub fn terminate(&mut self) {
        self.rotator.terminate();
        self.current = None;
        self.phase = ConnectionPhase::Ended;
    }

    /// Returns the explicit controller phase.
    pub fn phase(&self) -> ConnectionPhase {
        self.phase
    }

    /// Returns the underlying rotator state.
    pub fn state(&self) -> ConnectionState {
        self.rotator.state()
    }

    /// Returns the fingerprint this controller was built with.
    pub fn fingerprint(&self) -> &Fingerprint {
        &self.fingerprint
    }

    /// Returns the cached current string, if one is held.
    pub fn cached_string(&self) -> Option<&ConnectionString> {
        self.current.as_ref()
    }

    /// Returns true when `err` is the expired-window error.
    pub fn error_is_expired(err: &ConnectionError) -> bool {
        matches!(err, ConnectionError::Expired)
    }

    fn apply_error_phase(&mut self, err: &ConnectionError) {
        match err {
            ConnectionError::SessionEnded => {
                self.phase = ConnectionPhase::Ended;
                self.current = None;
            }
            ConnectionError::AlreadyLocked => {
                self.phase = ConnectionPhase::Locked;
            }
            _ => {}
        }
    }
}
