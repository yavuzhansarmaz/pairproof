//! Pure expert-app navigation state machine beneath rendering.

/// Screen shown by the expert interactive loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppScreen {
    #[default]
    RoleSelect,
    Preflight,
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
    /// No Meta API key is configured yet.
    #[error("meta api key is not configured")]
    ApiKeyMissing,
}

fn is_irreversible(screen: AppScreen) -> bool {
    matches!(
        screen,
        AppScreen::ConnectionWait
            | AppScreen::SelectMedium
            | AppScreen::SelectHard
            | AppScreen::Monitor
            | AppScreen::Final
    )
}

fn event_name(event: AppEvent) -> &'static str {
    match event {
        AppEvent::SelectExpert => "SelectExpert",
        AppEvent::SelectCandidate => "SelectCandidate",
        AppEvent::PreflightCompleted => "PreflightCompleted",
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
    pub preflight_lines: Vec<String>,
    pub proposal_lines: Vec<String>,
    pub final_lines: Vec<String>,
    pub api_key_status: Option<String>,
    pub api_key_ready: bool,
    pub api_key_entry: bool,
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

    /// Records the compact per-check lines shown on Expert Setup.
    pub fn set_preflight_lines(&mut self, lines: Vec<String>) {
        self.preflight_lines = lines;
    }

    /// Records the proposal lines shown on the Proposal screen.
    pub fn set_proposal_lines(&mut self, lines: Vec<String>) {
        self.proposal_lines = lines;
    }

    /// Records the final breakdown lines shown on the Final screen.
    pub fn set_final_lines(&mut self, lines: Vec<String>) {
        self.final_lines = lines;
    }

    /// Records the API-key status line shown on Expert Setup.
    pub fn set_api_key_status(&mut self, status: impl Into<String>) {
        self.api_key_status = Some(status.into());
    }

    /// Records whether a usable Meta API key is configured.
    pub fn set_api_key_ready(&mut self, ready: bool) {
        self.api_key_ready = ready;
    }

    /// Opens or closes the masked API-key entry on Expert Setup.
    pub fn set_api_key_entry(&mut self, active: bool) {
        self.api_key_entry = active;
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

    /// Clears the transient status message (e.g. a consumed reissue
    /// string must not stay on screen after the candidate reconnects).
    pub fn clear_status_message(&mut self) {
        self.status_message = None;
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
    /// Screens past Proposal commit external state (a locked
    /// connection, a committed selection, a live or ended session),
    /// so Back is not offered there. ConnectionWait is the exception:
    /// the runtime stays built and re-confirming returns here.
    pub fn can_go_back(&self) -> bool {
        self.screen != AppScreen::Exited
            && !self.history.is_empty()
            && (!is_irreversible(self.screen) || self.screen == AppScreen::ConnectionWait)
    }

    /// Pops to the previous screen; RoleSelect/Exited and screens
    /// past the runtime commit point (except ConnectionWait) are
    /// typed errors.
    pub fn back(&mut self) -> Result<AppScreen, AppError> {
        if self.screen == AppScreen::Exited
            || (is_irreversible(self.screen) && self.screen != AppScreen::ConnectionWait)
        {
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
            (S::Preflight, E::DisclosureAcknowledged) => {
                self.disclosure_acknowledged = true;
                Ok(self.screen)
            }
            (S::Preflight, E::PreflightCompleted) => {
                if !self.preflight_passed {
                    Err(AppError::PreflightBlocked)
                } else if !self.disclosure_acknowledged {
                    Err(AppError::DisclosureRequired)
                } else if !self.api_key_ready {
                    Err(AppError::ApiKeyMissing)
                } else {
                    Ok(self.go(S::Setup))
                }
            }
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

pub use crate::tui::expert_view::{footer, screen_lines, screen_title};
