//! Candidate screens: connection, rules, waiting, solving, and final.

use std::fmt;

use crate::core::{Session, SessionStatus};
use crate::tui::help::candidate_connect_message;
use crate::tui::shared::sanitize_display;

/// Returns the one-line hint shown when no terminal is available.
pub fn placeholder_message() -> &'static str {
    "candidate: run pairproof --role candidate in a terminal, then paste the connection string from the expert."
}

/// Returns the generic candidate-safe sync message.
pub fn candidate_sync_message() -> &'static str {
    "sync problem \u{2014} ask the expert"
}

/// Screen shown by the candidate flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CandidateScreen {
    #[default]
    Connect,
    Rules,
    Waiting,
    Solving,
    Final,
    Exited,
}

/// Candidate-safe connection failure; messages never carry input data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CandidateConnectError {
    /// The pasted string could not be parsed.
    #[error("connection string is malformed; check it and try again.")]
    Malformed,
    /// The pasted string is past its expiry time.
    #[error("connection string expired; ask the expert for the current string")]
    Expired,
    /// The verifier rejected the pasted string.
    #[error("connection was rejected; check the string and try again.")]
    Rejected,
    /// No verifier is wired up yet in this slice.
    #[error("connection verification is not available yet.")]
    Unavailable,
}

/// Typed candidate navigation failures; illegal actions never panic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CandidateError {
    /// Action is not legal on the current screen.
    #[error("invalid transition from {from:?} on {action}")]
    InvalidTransition {
        from: CandidateScreen,
        action: String,
    },
    /// Action needs a verified connection first.
    #[error("candidate is not connected")]
    NotConnected,
    /// Connection submission failed with a candidate-safe error.
    #[error("{0}")]
    Connect(#[from] CandidateConnectError),
}

pub use crate::tui::candidate_connect::ConnectionVerifier;
pub(crate) use crate::tui::candidate_connect::UnavailableVerifier;

/// Pure candidate state machine; no terminal or network I/O here.
pub struct CandidateApp {
    pub(crate) input: String,
    pub(crate) verifier: Box<dyn ConnectionVerifier>,
    pub(crate) connected: bool,
    rules_accepted: bool,
    exited: bool,
    session: Option<Session>,
    pub(crate) last_error: Option<CandidateConnectError>,
}

impl CandidateApp {
    /// Legacy alias for [`Self::new_unavailable`]; not the live verifier.
    pub fn new() -> Self {
        Self::new_unavailable()
    }

    /// Creates an app on Connect with the legacy unavailable verifier.
    pub fn new_unavailable() -> Self {
        Self::with_verifier(UnavailableVerifier)
    }

    /// Creates an app on Connect with an injected verifier.
    pub fn with_verifier(verifier: impl ConnectionVerifier + 'static) -> Self {
        Self {
            input: String::new(),
            verifier: Box::new(verifier),
            connected: false,
            rules_accepted: false,
            exited: false,
            session: None,
            last_error: None,
        }
    }

    /// Replaces the verifier, mainly for tests and Phase 10 wiring.
    pub fn set_verifier(&mut self, verifier: impl ConnectionVerifier + 'static) {
        self.verifier = Box::new(verifier);
    }

    /// Returns the derived current screen.
    pub fn screen(&self) -> CandidateScreen {
        if self.exited {
            return CandidateScreen::Exited;
        }
        if !self.connected {
            return CandidateScreen::Connect;
        }
        if !self.rules_accepted {
            return CandidateScreen::Rules;
        }
        match self.session_status() {
            Some(SessionStatus::InProgress) => CandidateScreen::Solving,
            Some(
                SessionStatus::Completed
                | SessionStatus::EndedByExpert
                | SessionStatus::TechnicalFailure,
            ) => CandidateScreen::Final,
            _ => CandidateScreen::Waiting,
        }
    }

    /// Returns true after the verifier accepted a connection string.
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// Returns true after the candidate explicitly accepted the rules.
    pub fn rules_accepted(&self) -> bool {
        self.rules_accepted
    }

    /// Returns true only on Solving after connection and consent.
    pub fn can_solve(&self) -> bool {
        self.connected && self.rules_accepted && self.screen() == CandidateScreen::Solving
    }

    /// Returns the attached session status, if any.
    pub fn session_status(&self) -> Option<SessionStatus> {
        self.session.as_ref().map(Session::status)
    }

    /// Returns the attached session, if any.
    pub fn session(&self) -> Option<&Session> {
        self.session.as_ref()
    }

    /// Returns the attached session mutably, if any.
    pub(crate) fn session_mut(&mut self) -> Option<&mut Session> {
        self.session.as_mut()
    }

    /// Returns the rules lines, including workspace and paste notes.
    pub fn rules_lines(&self) -> Vec<String> {
        [
            "work in a single workspace only; it is the only workspace.",
            "this folder is that workspace: each question appears here as q1, q2, and so on.",
            "you need no external resources for this session.",
            "please write your own work: bulk pastes are flagged.",
        ]
        .iter()
        .map(|line| sanitize_display(line))
        .collect()
    }

    /// Accepts the rules; only legal on Rules after connecting.
    pub fn accept_rules(&mut self) -> Result<CandidateScreen, CandidateError> {
        if self.screen() != CandidateScreen::Rules {
            return Err(CandidateError::InvalidTransition {
                from: self.screen(),
                action: "AcceptRules".to_string(),
            });
        }
        if !self.connected {
            return Err(CandidateError::NotConnected);
        }
        self.rules_accepted = true;
        Ok(self.screen())
    }

    /// Attaches a session; the screen is derived from its status.
    pub fn attach_session(&mut self, session: Session) -> CandidateScreen {
        self.session = Some(session);
        self.screen()
    }

    /// Quits from any screen into Exited.
    pub fn quit(&mut self) -> CandidateScreen {
        self.exited = true;
        CandidateScreen::Exited
    }

    /// Returns the waiting lines for the current session status.
    pub fn waiting_lines(&self) -> Vec<String> {
        if self.session_status() == Some(SessionStatus::WaitingForExpert) {
            vec!["expert is preparing questions".to_string()]
        } else {
            vec!["waiting for the expert.".to_string()]
        }
    }

    /// Builds deterministic sanitized lines for rendering one screen.
    pub fn screen_lines(&self) -> Vec<String> {
        let lines = match self.screen() {
            CandidateScreen::Connect => {
                let mut lines = vec![
                    "Candidate connection".to_string(),
                    "start in an empty folder. questions appear here as q1, q2.".to_string(),
                    "paste the connection string from the expert.".to_string(),
                    self.connect_line(),
                ];
                if let Some(error) = self.last_error {
                    lines.push(format!("error: {}", candidate_connect_message(&error)));
                }
                lines.push("enter connect, esc quit".to_string());
                lines
            }
            CandidateScreen::Rules => {
                let mut lines = vec!["Candidate rules".to_string()];
                lines.extend(self.rules_lines());
                lines.push("enter accept rules, esc quit".to_string());
                lines
            }
            CandidateScreen::Waiting => {
                let mut lines = vec!["Candidate waiting".to_string()];
                lines.extend(self.waiting_lines());
                lines.push("esc quit".to_string());
                lines
            }
            CandidateScreen::Solving => vec![
                "Candidate session".to_string(),
                "session in progress.".to_string(),
                "esc quit".to_string(),
            ],
            CandidateScreen::Final => vec![
                "Candidate session".to_string(),
                "session ended.".to_string(),
                "esc quit".to_string(),
            ],
            CandidateScreen::Exited => {
                vec!["Exited".to_string(), "goodbye.".to_string()]
            }
        };
        lines.iter().map(|line| sanitize_display(line)).collect()
    }
}

impl Default for CandidateApp {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for CandidateApp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CandidateApp")
            .field("screen", &self.screen())
            .field("connected", &self.connected)
            .field("rules_accepted", &self.rules_accepted)
            .field("input", &"[REDACTED]")
            .field("input_len", &self.input_len())
            .field("session_present", &self.session.is_some())
            .field("session_status", &self.session_status())
            .field("last_error", &self.last_error)
            .finish()
    }
}

/// Returns the static title for a candidate screen.
pub fn screen_title(screen: CandidateScreen) -> &'static str {
    match screen {
        CandidateScreen::Connect => "Candidate connection",
        CandidateScreen::Rules => "Candidate rules",
        CandidateScreen::Waiting => "Candidate waiting",
        CandidateScreen::Solving | CandidateScreen::Final => "Candidate session",
        CandidateScreen::Exited => "Exited",
    }
}
