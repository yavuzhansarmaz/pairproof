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
    let mut lines = vec![
        "Help",
        "PairProof runs AI-native interviews: the AI writes",
        "the code, the candidate directs it, and you watch",
        "judgment, not typing speed.",
        "",
        "On this screen:",
    ];
    match screen {
        AppScreen::RoleSelect => lines.extend([
            "  e expert — set up and run a live interview",
            "  c candidate — join with a connection string",
        ]),
        AppScreen::Preflight => lines.extend([
            "  Enter follows the step shown on screen:",
            "  acknowledge, add the API key, then continue",
            "  r re-run the environment checks",
            "  b back to the main menu",
        ]),
        AppScreen::Setup => lines.extend([
            "  Paste into the focused box, Enter moves on",
            "  Up/Down scroll, drag a box border to resize",
            "  b back a step, keeping your pasted text",
        ]),
        AppScreen::Proposal => lines.extend(["  Enter confirm and wait for the candidate"]),
        AppScreen::ConnectionWait => lines.extend([
            "  Waiting for the candidate to connect",
            "  Share the connection string shown on screen",
        ]),
        AppScreen::SelectMedium => lines.extend(["  Enter load the medium questions"]),
        AppScreen::SelectHard => lines.extend(["  Enter load the hard questions"]),
        AppScreen::Monitor => lines.extend([
            "  R reissue a fresh connection string after a disconnect",
            "  e end the session and build the final report",
        ]),
        AppScreen::Final => lines.extend(["  Enter close"]),
        AppScreen::Exited => lines.extend(["  Goodbye."]),
    }
    lines.extend([
        "",
        "Everywhere:",
        "  esc quit    ? this help",
        "",
        "Tool commands, from a separate terminal:",
        "  pairproof update                update PairProof",
        "  pairproof update --check        check only, no download",
        "  pairproof uninstall             remove binary and data",
        "  pairproof uninstall --keep-data remove binary, keep data",
        "",
        "b close help, d read the disclosure.",
    ]);
    clean(lines)
}

/// Returns the full Meta disclosure text for the help reader.
pub fn disclosure_lines() -> Vec<String> {
    clean(vec![
        "Meta disclosure",
        "Your job description and additional notes (each",
        "up to 50,000 characters) will be sent to the Meta",
        "API to propose interview topics, level, and",
        "language for the session.",
        "",
        "Nothing is sent before you acknowledge this on",
        "the Expert Setup screen. The acknowledgement is",
        "remembered on this machine, so you will not be",
        "asked again. The raw pastes are deleted when",
        "the session ends.",
        "",
        "esc quit, b back to help.",
    ])
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
            "esc quit",
        ]),
        (CandidateScreen::Rules, _) => lines.extend(["Enter accept the rules", "esc quit"]),
        (CandidateScreen::Waiting, _) => lines.extend(["Waiting for the expert.", "esc quit"]),
        (CandidateScreen::Solving, CandidateFocus::Work) => lines.extend([
            "1-4 choose an approach",
            "5 or o describe your own approach",
            "c continue, d redirect, a change approach",
            "t run tests, f finish the question, s skip the question",
            "r request a new question",
            "Tab switch pane",
            "esc quit",
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
        (CandidateScreen::Final, _) => lines.extend(["Enter close", "esc quit"]),
        (CandidateScreen::Exited, _) => lines.extend(["Goodbye."]),
    }
    if !candidate_is_typing(screen, focus) {
        lines.push("Press b to close help.");
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
        CandidateRuntimeError::VersionMismatch { .. } => {
            "PairProof version mismatch; update/restart PairProof and reconnect."
        }
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
    Quit,
    ShowDisclosure,
    ReturnToHelp,
}

/// Routes a full key event: Ctrl-C quits, other Ctrl/Alt-modified keys
/// never trigger plain-letter actions (Ctrl-E must not end a session).
pub fn expert_event_action(key: crossterm::event::KeyEvent, show_help: bool) -> ExpertKeyAction {
    use crossterm::event::KeyModifiers;
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        if key.code == crossterm::event::KeyCode::Char('c') {
            return ExpertKeyAction::Quit;
        }
        return ExpertKeyAction::Ignore;
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        return ExpertKeyAction::Ignore;
    }
    expert_key_action(key.code, show_help)
}

/// Routes a key while the disclosure reader is open: esc quits,
/// `b` returns to help, everything else is swallowed.
pub fn expert_disclosure_action(key: crossterm::event::KeyEvent) -> ExpertKeyAction {
    use crossterm::event::KeyModifiers;
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        if key.code == crossterm::event::KeyCode::Char('c') {
            return ExpertKeyAction::Quit;
        }
        return ExpertKeyAction::Ignore;
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        return ExpertKeyAction::Ignore;
    }
    match key.code {
        crossterm::event::KeyCode::Esc => ExpertKeyAction::Quit,
        crossterm::event::KeyCode::Char('b') => ExpertKeyAction::ReturnToHelp,
        _ => ExpertKeyAction::Ignore,
    }
}

/// Returns true when a key shown on the help overlay still works
/// while help is open: role selection on the main menu is safe to
/// dispatch without closing help first.
pub fn help_overlay_dispatches(code: crossterm::event::KeyCode, screen: AppScreen) -> bool {
    screen == AppScreen::RoleSelect
        && matches!(
            code,
            crossterm::event::KeyCode::Char('e') | crossterm::event::KeyCode::Char('c')
        )
}

/// Routes a key: `?` toggles help; help open swallows non-quit keys.
pub fn expert_key_action(code: crossterm::event::KeyCode, show_help: bool) -> ExpertKeyAction {
    use crossterm::event::KeyCode as K;
    match code {
        K::Char('?') if !show_help => ExpertKeyAction::ToggleHelp,
        K::Char('b') if show_help => ExpertKeyAction::ToggleHelp,
        K::Char('d') if show_help => ExpertKeyAction::ShowDisclosure,
        _ if show_help && code != K::Esc => ExpertKeyAction::Ignore,
        _ => ExpertKeyAction::Dispatch,
    }
}
