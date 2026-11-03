//! Candidate connection verification seam and the Connect-screen
//! input/submit behaviour of `CandidateApp`.

use crate::connection::ConnectionString;
use crate::tui::candidate::{CandidateApp, CandidateConnectError, CandidateScreen};

/// Verifies a parsed connection string without leaking diagnostics.
pub trait ConnectionVerifier {
    /// Returns Ok only when the connection may proceed.
    fn verify(&self, c: &ConnectionString, now_ms: u64) -> Result<(), CandidateConnectError>;
}

impl<F> ConnectionVerifier for F
where
    F: Fn(&ConnectionString, u64) -> Result<(), CandidateConnectError>,
{
    fn verify(&self, c: &ConnectionString, now_ms: u64) -> Result<(), CandidateConnectError> {
        self(c, now_ms)
    }
}

/// Legacy Phase-9 seam: always `Unavailable`, never fakes verification.
/// Live Phase-10 production replaces it via `fresh_verifier`/
/// `SshAuthVerifier` in `candidate_runtime`; crate-internal only, tests
/// exercise it through `CandidateApp::new_unavailable`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct UnavailableVerifier;

impl ConnectionVerifier for UnavailableVerifier {
    fn verify(&self, _c: &ConnectionString, _now_ms: u64) -> Result<(), CandidateConnectError> {
        Err(CandidateConnectError::Unavailable)
    }
}

impl CandidateApp {
    /// Appends one typed character to the connection input on Connect.
    /// After a failed submit, the first typed character starts fresh:
    /// the rejected string is cleared instead of appended to.
    pub fn push_char(&mut self, c: char) {
        if self.screen() != CandidateScreen::Connect || c.is_control() || c.is_whitespace() {
            return;
        }
        if self.last_error.is_some() {
            self.input.clear();
        }
        self.input.push(c);
        self.last_error = None;
    }

    /// Replaces the connection input with pasted text on Connect, so
    /// pasting a fresh string after a malformed one corrects it
    /// instead of appending to the rejected text.
    pub fn replace_text(&mut self, text: &str) {
        if self.screen() != CandidateScreen::Connect {
            return;
        }
        if !text.chars().any(|c| !c.is_control() && !c.is_whitespace()) {
            return;
        }
        self.input.clear();
        self.push_text(text);
    }

    /// Appends pasted text to the connection input on Connect.
    /// After a failed submit, appended text also starts fresh.
    pub fn push_text(&mut self, text: &str) {
        if self.screen() != CandidateScreen::Connect {
            return;
        }
        if self.last_error.is_some() {
            self.input.clear();
        }
        for c in text.chars() {
            if !c.is_control() && !c.is_whitespace() {
                self.input.push(c);
            }
        }
        self.last_error = None;
    }

    /// Removes the last connection-input character, if any.
    pub fn backspace(&mut self) {
        if self.screen() == CandidateScreen::Connect {
            self.input.pop();
            self.last_error = None;
        }
    }

    /// Clears the whole connection input.
    pub fn clear_input(&mut self) {
        if self.screen() == CandidateScreen::Connect {
            self.input.clear();
            self.last_error = None;
        }
    }

    /// Returns the number of characters in the connection input.
    pub fn input_len(&self) -> usize {
        self.input.chars().count()
    }

    /// Returns the masked connect line; raw input is never included.
    pub fn connect_line(&self) -> String {
        if self.input.is_empty() {
            "connection string: (empty)".to_string()
        } else {
            format!(
                "connection string: {} ({} characters)",
                "\u{2022}".repeat(self.input_len()),
                self.input_len()
            )
        }
    }

    /// Parses, expiry-checks, and verifies the current input.
    pub fn submit_connection(
        &mut self,
        now_ms: u64,
    ) -> Result<CandidateScreen, CandidateConnectError> {
        if self.screen() != CandidateScreen::Connect {
            return Err(self.record_error(CandidateConnectError::Rejected));
        }
        let parsed = match ConnectionString::parse(self.input.trim()) {
            Ok(value) => value,
            Err(_) => return Err(self.record_error(CandidateConnectError::Malformed)),
        };
        if now_ms >= parsed.expires_at_ms() {
            return Err(self.record_error(CandidateConnectError::Expired));
        }
        match self.verifier.verify(&parsed, now_ms) {
            Ok(()) => {
                self.mark_connection_verified();
                Ok(self.screen())
            }
            Err(CandidateConnectError::Unavailable) => {
                Err(self.record_error(CandidateConnectError::Unavailable))
            }
            Err(_) => Err(self.record_error(CandidateConnectError::Rejected)),
        }
    }

    /// Raw connection input for a worker thread to verify off the
    /// TTY thread; only ever handed to the connection verifier.
    pub(crate) fn connect_input(&self) -> String {
        self.input.clone()
    }

    /// Marks a connection verified on a worker thread as connected.
    pub(crate) fn mark_connection_verified(&mut self) {
        self.connected = true;
        self.input.clear();
        self.last_error = None;
    }

    /// Records a connection failure reported by a worker thread.
    pub(crate) fn record_connect_error(&mut self, error: CandidateConnectError) {
        self.last_error = Some(error);
    }

    fn record_error(&mut self, error: CandidateConnectError) -> CandidateConnectError {
        self.last_error = Some(error);
        error
    }
}
