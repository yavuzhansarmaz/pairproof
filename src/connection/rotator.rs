use super::error::ConnectionError;
use super::secret::Secret;
use super::string::{ConnectionString, Endpoint, EphemeralUsername, Fingerprint};

/// Lifetime of one rotating secret window, in milliseconds.
pub const ROTATION_WINDOW_MS: u64 = 60_000;

/// Lifecycle state of a connection string's rotating secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Rotating,
    Locked,
    Ended,
}

/// Issues 60-second rotating connection strings and locks on first use.
#[derive(Debug)]
pub struct SecretRotator {
    endpoint: Endpoint,
    username: EphemeralUsername,
    fingerprint: Fingerprint,
    state: ConnectionState,
    current: Option<(Secret, u64)>,
}

impl SecretRotator {
    /// Starts rotating with a fresh secret expiring one window after `now_ms`.
    pub fn new(
        endpoint: Endpoint,
        username: EphemeralUsername,
        fingerprint: Fingerprint,
        now_ms: u64,
    ) -> Self {
        Self {
            endpoint,
            username,
            fingerprint,
            state: ConnectionState::Rotating,
            current: Some((
                Secret::generate(),
                now_ms.saturating_add(ROTATION_WINDOW_MS),
            )),
        }
    }

    pub fn state(&self) -> ConnectionState {
        self.state
    }

    /// Returns the current string, rotating the secret first if its window elapsed.
    pub fn current_string(&mut self, now_ms: u64) -> Result<ConnectionString, ConnectionError> {
        match self.state {
            ConnectionState::Locked => return Err(ConnectionError::AlreadyLocked),
            ConnectionState::Ended => return Err(ConnectionError::SessionEnded),
            ConnectionState::Rotating => {}
        }
        if let Some((_, expires_at_ms)) = &self.current {
            if now_ms >= *expires_at_ms {
                self.current = Some((
                    Secret::generate(),
                    now_ms.saturating_add(ROTATION_WINDOW_MS),
                ));
            }
        }
        let (secret, expires_at_ms) = match &self.current {
            Some((secret, expires_at_ms)) => (secret.clone(), *expires_at_ms),
            None => return Err(ConnectionError::SessionEnded),
        };
        Ok(ConnectionString::new(
            self.endpoint.clone(),
            self.username.clone(),
            secret,
            expires_at_ms,
            self.fingerprint.clone(),
        ))
    }

    /// Seconds left in the current window, ceiling-rounded and capped at 60.
    pub fn countdown_secs(&self, now_ms: u64) -> u64 {
        if self.state != ConnectionState::Rotating {
            return 0;
        }
        let expires_at_ms = match &self.current {
            Some((_, expires_at_ms)) => *expires_at_ms,
            None => return 0,
        };
        let remaining_ms = expires_at_ms.saturating_sub(now_ms);
        remaining_ms
            .saturating_add(999)
            .checked_div(1000)
            .unwrap_or(0)
            .min(60)
    }

    /// Checks a presented string against the held window; success locks the rotator.
    pub fn verify(
        &mut self,
        presented: &ConnectionString,
        now_ms: u64,
    ) -> Result<(), ConnectionError> {
        match self.state {
            ConnectionState::Ended => return Err(ConnectionError::SessionEnded),
            ConnectionState::Locked => return Err(ConnectionError::AlreadyLocked),
            ConnectionState::Rotating => {}
        }
        if presented.fingerprint() != &self.fingerprint {
            return Err(ConnectionError::FingerprintMismatch);
        }
        if presented.endpoint() != &self.endpoint {
            return Err(ConnectionError::EndpointMismatch);
        }
        if presented.username() != &self.username {
            return Err(ConnectionError::UsernameMismatch);
        }
        let expires_at_ms = match &self.current {
            Some((_, expires_at_ms)) => *expires_at_ms,
            None => return Err(ConnectionError::SessionEnded),
        };
        if now_ms >= expires_at_ms || presented.expires_at_ms() != expires_at_ms {
            return Err(ConnectionError::Expired);
        }
        let secret_matches = match &self.current {
            Some((secret, _)) => presented.secret() == secret,
            None => return Err(ConnectionError::SessionEnded),
        };
        if !secret_matches {
            return Err(ConnectionError::WrongSecret);
        }
        self.current = None;
        self.state = ConnectionState::Locked;
        Ok(())
    }

    /// Issues a fresh secret with a full window; every earlier secret is dead.
    pub fn reissue(&mut self, now_ms: u64) -> Result<ConnectionString, ConnectionError> {
        if self.state == ConnectionState::Ended {
            return Err(ConnectionError::SessionEnded);
        }
        let secret = Secret::generate();
        let expires_at_ms = now_ms.saturating_add(ROTATION_WINDOW_MS);
        self.current = Some((secret.clone(), expires_at_ms));
        self.state = ConnectionState::Rotating;
        Ok(ConnectionString::new(
            self.endpoint.clone(),
            self.username.clone(),
            secret,
            expires_at_ms,
            self.fingerprint.clone(),
        ))
    }

    /// Ends the session permanently and drops the held secret.
    pub fn terminate(&mut self) {
        self.current = None;
        self.state = ConnectionState::Ended;
    }
}
