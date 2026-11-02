//! Role-aware help lines and secret-safe error presentation.

use crate::tui::app::AppScreen;
use crate::tui::candidate::{CandidateConnectError, CandidateScreen};
use crate::tui::candidate_run::CandidateFocus;
use crate::tui::candidate_runtime::CandidateRuntimeError;
use crate::tui::shared::sanitize_display;

fn clean(lines: Vec<&str>) -> Vec<String> {
    lines.iter().map(|line| sanitize_display(line)).collect()
}

/// Returns expert help lines listing only keys wired for `screen`.
pub fn expert_help_lines(screen: AppScreen) -> Vec<String> {
    let mut lines = vec!["Help", "q quit, Esc quit, b back, r retry"];
    match screen {
        AppScreen::RoleSelect => lines.extend(["e expert, c candidate"]),
        AppScreen::Preflight => lines.extend(["Enter continue when checks pass"]),
        AppScreen::ApiKey => lines.extend(["Enter continue when the key is saved"]),
        AppScreen::Setup => lines.extend([
            "Paste the job description, then additional notes",
            "Enter continue",
        ]),
        AppScreen::Proposal => lines.extend(["Enter confirm and wait for the candidate"]),
        AppScreen::ConnectionWait => {
            lines.extend(["Waiting for the candidate to connect", "Enter check again"])
        }
        AppScreen::SelectMedium => lines.extend(["Enter fetch and select medium questions"]),
        AppScreen::SelectHard => lines.extend(["Enter fetch and complete the selection"]),
        AppScreen::Monitor => lines.extend([
            "R reissue a fresh connection string after a disconnect",
            "e end the session",
        ]),
        AppScreen::Final => lines.extend(["Enter or q close"]),
        AppScreen::Exited => lines.extend(["Goodbye."]),
    }
    lines.push("Press ? to close help.");
    clean(lines)
}

/// Returns candidate help lines for `screen` with the work pane focused.
pub fn candidate_help_lines(screen: CandidateScreen) -> Vec<String> {
    candidate_help_lines_for(screen, CandidateFocus::Work)
}

/// Returns candidate help lines for a screen and solving pane focus.
pub fn candidate_help_lines_for(screen: CandidateScreen, focus: CandidateFocus) -> Vec<String> {
    let mut lines = vec!["Help"];
    match (screen, focus) {
        (CandidateScreen::Connect, _) => lines.extend([
            "Paste the connection string from the expert.",
            "Enter connect",
            "Backspace delete a character",
            "Esc quit",
        ]),
        (CandidateScreen::Rules, _) => lines.extend(["Enter accept the rules", "q quit"]),
        (CandidateScreen::Waiting, _) => lines.extend(["Waiting for the expert.", "q quit"]),
        (CandidateScreen::Solving, CandidateFocus::Work) => lines.extend([
            "1-4 choose an approach",
            "5 or o describe your own approach",
            "c continue, d redirect, a change approach",
            "t run tests, f finish the question, s skip the question",
            "r request a new question",
            "Tab switch pane",
            "q quit",
        ]),
        (CandidateScreen::Solving, CandidateFocus::Other) => lines.extend([
            "Type your own approach.",
            "Enter confirm the approach",
            "Backspace delete a character",
            "Tab next pane",
        ]),
        (CandidateScreen::Solving, CandidateFocus::Redirect) => lines.extend([
            "Type a redirect instruction.",
            "Enter send the redirect",
            "Backspace delete a character",
            "Tab next pane",
        ]),
        (CandidateScreen::Solving, CandidateFocus::Chat) => lines.extend([
            "Type a message.",
            "Enter send the message",
            "Backspace delete a character",
            "Tab next pane",
        ]),
        (CandidateScreen::Final, _) => lines.extend(["Enter or q close"]),
        (CandidateScreen::Exited, _) => lines.extend(["Goodbye."]),
    }
    if !candidate_is_typing(screen, focus) {
        lines.push("Press ? to close help.");
    }
    clean(lines)
}

/// Returns true when printable keys type into a candidate text field.
pub fn candidate_is_typing(screen: CandidateScreen, focus: CandidateFocus) -> bool {
    match screen {
        CandidateScreen::Connect => true,
        CandidateScreen::Solving => matches!(
            focus,
            CandidateFocus::Other | CandidateFocus::Redirect | CandidateFocus::Chat
        ),
        _ => false,
    }
}

/// Maps a candidate connection failure to a fixed generic message.
pub fn candidate_connect_message(err: &CandidateConnectError) -> &'static str {
    match err {
        CandidateConnectError::Malformed => {
            "Connection string is malformed; check it and try again."
        }
        CandidateConnectError::Expired => {
            "Connection string expired; ask the expert for the current string"
        }
        CandidateConnectError::Rejected => {
            "Connection was rejected; check the string and try again."
        }
        CandidateConnectError::Unavailable => {
            "Connection is not available; ask the expert for help."
        }
    }
}

/// Maps any candidate runtime failure to a fixed generic message.
pub fn candidate_error_message(err: &CandidateRuntimeError) -> &'static str {
    match err {
        CandidateRuntimeError::Auth(inner) => candidate_connect_message(inner),
        CandidateRuntimeError::SyncFailed => "Sync problem \u{2014} ask the expert",
        CandidateRuntimeError::WrongSession | CandidateRuntimeError::Transport => {
            "Connection problem \u{2014} ask the expert"
        }
        CandidateRuntimeError::NotConnected => {
            "Not connected; paste the connection string and try again."
        }
        CandidateRuntimeError::Ended => "The session has ended; ask the expert what to do next.",
    }
}

/// Builds a sanitized expert error line: problem plus next step.
pub fn expert_error_message(context: &str, detail: &str) -> String {
    let detail = sanitize_display(detail);
    if detail.trim().is_empty() {
        return format!(
            "{}: check the session state and try again.",
            sanitize_display(context)
        );
    }
    format!(
        "{}: {} Check the session state and try again.",
        sanitize_display(context),
        detail
    )
}

/// Formats a countdown that came from the live runtime state.
pub fn countdown_line(secs: u64) -> String {
    format!("Expires in {secs}s.")
}

/// How the expert loop routes one key while help may be open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpertKeyAction {
    ToggleHelp,
    Ignore,
    Dispatch,
}

/// Routes a key: `?` toggles help; help open swallows non-quit keys.
pub fn expert_key_action(code: crossterm::event::KeyCode, show_help: bool) -> ExpertKeyAction {
    use crossterm::event::KeyCode as K;
    match code {
        K::Char('?') => ExpertKeyAction::ToggleHelp,
        _ if show_help && !matches!(code, K::Char('q') | K::Esc) => ExpertKeyAction::Ignore,
        _ => ExpertKeyAction::Dispatch,
    }
}
