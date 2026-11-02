//! Pure expert-app navigation state machine beneath rendering.

use crate::tui::candidate::placeholder_message;
use crate::tui::shared::sanitize_display;

/// Screen shown by the expert interactive loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppScreen {
    #[default]
    RoleSelect,
    Preflight,
    ApiKey,
    Setup,
    Proposal,
    ConnectionWait,
    SelectMedium,
    SelectHard,
    Monitor,
    Final,
    Exited,
}

/// Event driving the expert navigation state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEvent {
    SelectExpert,
    SelectCandidate,
    PreflightCompleted,
    ApiKeySaved,
    DisclosureAcknowledged,
    ProposalReady,
    ProposalConfirmed,
    ConnectionLocked,
    MediumSelected,
    ExpertSelectionDone,
    EndSession,
    AccidentalDisconnect,
    CandidateReconnected,
    Back,
    Quit,
    Retry,
}

/// Typed navigation failures; illegal events never panic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AppError {
    /// Event is not legal on the current screen.
    #[error("invalid transition from {from:?} on event {event}")]
    InvalidTransition { from: AppScreen, event: String },
    /// Preflight has not passed, so advancing is blocked.
    #[error("preflight has not passed")]
    PreflightBlocked,
    /// Proposal was requested before disclosure acknowledgement.
    #[error("acknowledge the disclosure before continuing")]
    DisclosureRequired,
    /// Connection advance was attempted before the candidate connected.
    #[error("candidate is not connected")]
    NotConnected,
    /// Session was already ended by the expert.
    #[error("session already ended")]
    AlreadyEnded,
}

fn event_name(event: AppEvent) -> &'static str {
    match event {
        AppEvent::SelectExpert => "SelectExpert",
        AppEvent::SelectCandidate => "SelectCandidate",
        AppEvent::PreflightCompleted => "PreflightCompleted",
        AppEvent::ApiKeySaved => "ApiKeySaved",
        AppEvent::DisclosureAcknowledged => "DisclosureAcknowledged",
        AppEvent::ProposalReady => "ProposalReady",
        AppEvent::ProposalConfirmed => "ProposalConfirmed",
        AppEvent::ConnectionLocked => "ConnectionLocked",
        AppEvent::MediumSelected => "MediumSelected",
        AppEvent::ExpertSelectionDone => "ExpertSelectionDone",
        AppEvent::EndSession => "EndSession",
        AppEvent::AccidentalDisconnect => "AccidentalDisconnect",
        AppEvent::CandidateReconnected => "CandidateReconnected",
        AppEvent::Back => "Back",
        AppEvent::Quit => "Quit",
        AppEvent::Retry => "Retry",
    }
}

/// Expert navigation state; flags are fed by controller results.
#[derive(Debug, Clone, Default)]
pub struct ExpertApp {
    pub screen: AppScreen,
    pub history: Vec<AppScreen>,
    pub preflight_passed: bool,
    pub disclosure_acknowledged: bool,
    pub has_proposal: bool,
    pub connected: bool,
    pub expert_selection_done: bool,
    pub ended: bool,
    pub disconnected: bool,
    pub candidate_placeholder: bool,
    status_message: Option<String>,
}

impl ExpertApp {
    /// Creates an app starting on the role-selection screen.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the current screen.
    pub fn screen(&self) -> AppScreen {
        self.screen
    }

    /// Records the preflight outcome from `can_proceed(report)`.
    pub fn set_preflight_passed(&mut self, passed: bool) {
        self.preflight_passed = passed;
    }

    /// Records whether a proposal exists in the setup controller.
    pub fn set_has_proposal(&mut self, has_proposal: bool) {
        self.has_proposal = has_proposal;
    }

    /// Records disclosure acknowledgement from the setup controller.
    pub fn set_disclosure_acknowledged(&mut self, acknowledged: bool) {
        self.disclosure_acknowledged = acknowledged;
    }

    /// Records a locked connection from the connection controller.
    pub fn set_connected(&mut self, connected: bool) {
        self.connected = connected;
    }

    /// Records completed expert selection from the selection controller.
    pub fn set_expert_selection_done(&mut self, done: bool) {
        self.expert_selection_done = done;
    }

    /// Sets a transient status message rendered sanitized.
    pub fn set_status_message(&mut self, message: impl Into<String>) {
        self.status_message = Some(message.into());
    }

    /// Returns the transient status message, if any.
    pub fn status_message(&self) -> Option<&str> {
        self.status_message.as_deref()
    }

    /// Returns true when candidate placeholder mode was selected.
    pub fn is_candidate_placeholder(&self) -> bool {
        self.candidate_placeholder
    }

    /// Returns true after an accidental disconnect on the monitor.
    pub fn is_disconnected(&self) -> bool {
        self.disconnected
    }

    /// Returns true when Back can return to a previous screen.
    pub fn can_go_back(&self) -> bool {
        self.screen != AppScreen::Exited && !self.history.is_empty()
    }

    /// Pops to the previous screen; RoleSelect/Exited are typed errors.
    pub fn back(&mut self) -> Result<AppScreen, AppError> {
        if self.screen == AppScreen::Exited {
            return Err(self.invalid(AppEvent::Back));
        }
        match self.history.pop() {
            Some(previous) => {
                self.screen = previous;
                Ok(previous)
            }
            None => Err(self.invalid(AppEvent::Back)),
        }
    }

    /// Applies one event, returning the resulting screen.
    pub fn handle(&mut self, event: AppEvent) -> Result<AppScreen, AppError> {
        match event {
            AppEvent::Quit => {
                if self.screen != AppScreen::Exited {
                    self.go(AppScreen::Exited);
                }
                Ok(self.screen)
            }
            AppEvent::Back => self.back(),
            AppEvent::Retry => Ok(self.screen),
            _ => self.handle_forward(event),
        }
    }

    fn handle_forward(&mut self, event: AppEvent) -> Result<AppScreen, AppError> {
        use AppEvent as E;
        use AppScreen as S;
        match (self.screen, event) {
            (S::RoleSelect, E::SelectExpert) => Ok(self.go(S::Preflight)),
            (S::RoleSelect, E::SelectCandidate) => {
                self.candidate_placeholder = true;
                Ok(self.go(S::Exited))
            }
            (S::Preflight, E::PreflightCompleted) => {
                if self.preflight_passed {
                    Ok(self.go(S::ApiKey))
                } else {
                    Err(AppError::PreflightBlocked)
                }
            }
            (S::ApiKey, E::ApiKeySaved) => Ok(self.go(S::Setup)),
            (S::Setup, E::DisclosureAcknowledged) => {
                self.disclosure_acknowledged = true;
                Ok(self.screen)
            }
            (S::Setup, E::ProposalReady) => {
                if !self.disclosure_acknowledged {
                    Err(AppError::DisclosureRequired)
                } else if !self.has_proposal {
                    Err(self.invalid(event))
                } else {
                    Ok(self.go(S::Proposal))
                }
            }
            (S::Proposal, E::ProposalConfirmed)
            | (S::Proposal, E::ProposalReady)
            | (S::Proposal, E::ConnectionLocked) => Ok(self.go(S::ConnectionWait)),
            (S::ConnectionWait, E::ConnectionLocked) => {
                if self.connected {
                    Ok(self.go(S::SelectMedium))
                } else {
                    Err(AppError::NotConnected)
                }
            }
            (S::SelectMedium, E::MediumSelected) | (S::SelectMedium, E::ExpertSelectionDone) => {
                Ok(self.go(S::SelectHard))
            }
            (S::SelectHard, E::ExpertSelectionDone) => {
                self.expert_selection_done = true;
                Ok(self.go(S::Monitor))
            }
            (S::Monitor, E::EndSession) => {
                self.ended = true;
                Ok(self.go(S::Final))
            }
            (S::Monitor, E::AccidentalDisconnect) => {
                self.disconnected = true;
                Ok(self.screen)
            }
            (S::Monitor, E::CandidateReconnected) => {
                self.disconnected = false;
                Ok(self.screen)
            }
            _ => {
                if event == E::EndSession && self.ended {
                    Err(AppError::AlreadyEnded)
                } else {
                    Err(self.invalid(event))
                }
            }
        }
    }

    fn go(&mut self, next: AppScreen) -> AppScreen {
        self.history.push(self.screen);
        self.screen = next;
        next
    }

    fn invalid(&self, event: AppEvent) -> AppError {
        AppError::InvalidTransition {
            from: self.screen,
            event: event_name(event).to_string(),
        }
    }
}

/// Returns the static title for a screen.
pub fn screen_title(screen: AppScreen) -> &'static str {
    match screen {
        AppScreen::RoleSelect => "Select role",
        AppScreen::Preflight => "Preflight checks",
        AppScreen::ApiKey => "API key",
        AppScreen::Setup => "Session setup",
        AppScreen::Proposal => "Proposed context",
        AppScreen::ConnectionWait => "Waiting for candidate",
        AppScreen::SelectMedium => "Select medium questions",
        AppScreen::SelectHard => "Select hard questions",
        AppScreen::Monitor => "Live monitor",
        AppScreen::Final => "Final breakdown",
        AppScreen::Exited => "Exited",
    }
}

fn footer(app: &ExpertApp) -> String {
    match app.screen() {
        AppScreen::Exited => "q quit".to_string(),
        _ if app.can_go_back() => "q quit, b back, r retry, Enter advance".to_string(),
        _ => "q quit, r retry, Enter advance".to_string(),
    }
}

/// Builds deterministic sanitized lines for rendering one screen.
pub fn screen_lines(app: &ExpertApp) -> Vec<String> {
    use AppScreen as S;
    let screen = app.screen();
    let mut lines = vec![screen_title(screen).to_string()];
    if app.candidate_placeholder && screen == S::Exited {
        lines.push(placeholder_message().to_string());
    } else {
        match screen {
            S::RoleSelect => lines.push("Press e for expert, c for candidate.".into()),
            S::Preflight => lines.push(if app.preflight_passed {
                "Preflight passed. Press Enter to continue.".into()
            } else {
                "Preflight has not passed yet. Press r to retry.".into()
            }),
            S::ApiKey => {
                lines.push("API key is never shown on screen. Press Enter when saved.".into())
            }
            S::Setup => {
                lines.push(if app.disclosure_acknowledged {
                    "Disclosure acknowledged.".into()
                } else {
                    "Acknowledge the Meta API disclosure to continue.".into()
                });
                lines.push(if app.has_proposal {
                    "Proposal ready. Press Enter to review it.".into()
                } else {
                    "No proposal yet.".into()
                });
            }
            S::Proposal => {
                lines.push("Review the proposal. Press Enter to wait for the candidate.".into())
            }
            S::ConnectionWait => lines.push(if app.connected {
                "Candidate connected. Press Enter to select questions.".into()
            } else {
                "Waiting for candidate to connect.".into()
            }),
            S::SelectMedium => {
                lines.push("Select 3 medium questions. Press Enter when done.".into())
            }
            S::SelectHard => lines.push("Select 2 hard questions. Press Enter when done.".into()),
            S::Monitor => {
                lines.push(if app.disconnected {
                    "Candidate disconnected. Session is still live.".into()
                } else {
                    "Session live.".into()
                });
                lines.push("Press Enter to end the session.".into());
            }
            S::Final => lines
                .push("Session ended. Report generation is Phase 10 (not claimed here).".into()),
            S::Exited => lines.push("Goodbye.".into()),
        }
    }
    if let Some(message) = app.status_message() {
        lines.push(format!("Status: {}", sanitize_display(message)));
    }
    lines.push(footer(app));
    lines.iter().map(|line| sanitize_display(line)).collect()
}
