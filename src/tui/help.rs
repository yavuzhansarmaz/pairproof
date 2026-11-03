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
        AppScreen::Preflight => lines.extend(["  m change the Meta API key"]),
        AppScreen::Setup => lines.extend([
            "  paste into the focused box, enter moves on",
            "  tab switches box, backspace deletes the last character",
            "  up/down scroll, drag a box border to resize",
            "  m change the Meta API key (back on Expert Setup)",
            "  b back a step, keeping your pasted text",
        ]),
        AppScreen::Proposal => lines.extend(["  enter wait for the candidate"]),
        AppScreen::ConnectionWait => lines.extend([
            "  waiting for the candidate to connect",
            "  share the connection string shown on screen",
            "  the [COPY] button copies the connection string on click",
            "  b back to the proposal",
        ]),
        AppScreen::SelectMedium => lines.extend([
            "  up/down move, left/right change page",
            "  space choose a question, enter continue with 3 chosen",
            "  tab preview the question under the cursor (left/right switch question, b returns)",
            "  r pick at random instead (enter confirms, b returns)",
        ]),
        AppScreen::SelectHard => lines.extend([
            "  up/down move, left/right change page",
            "  space choose a question, enter finish with 2 chosen",
            "  tab preview the question under the cursor (left/right switch question, b returns)",
            "  r pick at random instead (enter confirms, b returns)",
        ]),
        AppScreen::Monitor => lines.extend([
            "  c reissue a fresh connection string after a disconnect",
            "  e end the session and build the final report",
        ]),
        AppScreen::Final => lines.extend(["  enter close"]),
        AppScreen::Exited => lines.extend(["  goodbye."]),
    }
    lines.push("  d read the disclosure");
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
        "b close help.",
    ]);
    clean(lines)
}

/// Returns the full Meta disclosure text for the help reader.
pub fn disclosure_lines() -> Vec<String> {
    clean(vec![
        "Meta disclosure",
        "your job description and additional notes (each",
        "up to 50,000 characters) will be sent to the Meta",
        "API to propose interview topics, level, and",
        "language for the session.",
        "",
        "nothing is sent before you acknowledge this on",
        "the Expert Setup screen. the acknowledgement is",
        "remembered on this machine, so you will not be",
        "asked again. the raw pastes are deleted when",
        "the session ends.",
        "",
        "b back to help, esc quit.",
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
            "paste the connection string from the expert.",
            "enter connect",
            "backspace delete a character",
            "esc quit",
        ]),
        (CandidateScreen::Rules, _) => lines.extend(["enter accept the rules", "esc quit"]),
        (CandidateScreen::Waiting, _) => lines.extend(["waiting for the expert.", "esc quit"]),
        (CandidateScreen::Solving, CandidateFocus::Work) => lines.extend([
            "1-4 choose an approach",
            "on other: type your approach, enter submits it",
            "5 or o opens that approach field",
            "c continue, d additional notes for the code, a change approach",
            "t run tests, f finish the question, s skip the question",
            "r request a new question",
            "tab or click a panel to switch pane",
            "esc quit",
        ]),
        (CandidateScreen::Solving, CandidateFocus::Other) => lines.extend([
            "type your own approach.",
            "enter confirm the approach",
            "backspace delete a character",
            "tab or click a panel to switch pane",
        ]),
        (CandidateScreen::Solving, CandidateFocus::Redirect) => lines.extend([
            "type additional notes for the code being written.",
            "enter send the notes",
            "backspace delete a character",
            "tab or click a panel to switch pane",
        ]),
        (CandidateScreen::Solving, CandidateFocus::Chat) => lines.extend([
            "type a message.",
            "enter send the message",
            "backspace delete a character",
            "tab or click a panel to switch pane",
        ]),
        (CandidateScreen::Final, _) => lines.extend(["enter close", "esc quit"]),
        (CandidateScreen::Exited, _) => lines.extend(["goodbye."]),
    }
    if !candidate_is_typing(screen, focus) {
        lines.push("press b to close help.");
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
            "connection string is malformed; check it and try again."
        }
        CandidateConnectError::Expired => {
            "connection string expired; ask the expert for the current string"
        }
        CandidateConnectError::Rejected => {
            "connection was rejected; check the string and try again."
        }
        CandidateConnectError::Unavailable => {
            "connection is not available; ask the expert for help."
        }
    }
}

/// Maps any candidate runtime failure to a fixed generic message.
pub fn candidate_error_message(err: &CandidateRuntimeError) -> &'static str {
    match err {
        CandidateRuntimeError::Auth(inner) => candidate_connect_message(inner),
        CandidateRuntimeError::SyncFailed => "sync problem \u{2014} ask the expert",
        CandidateRuntimeError::WrongSession | CandidateRuntimeError::Transport => {
            "connection problem \u{2014} ask the expert"
        }
        CandidateRuntimeError::NotConnected => {
            "not connected; paste the connection string and try again."
        }
        CandidateRuntimeError::Ended => "the session has ended; ask the expert what to do next.",
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
        "{}: {} check the session state and try again.",
        sanitize_display(context),
        detail
    )
}

/// Formats a countdown that came from the live runtime state.
pub fn countdown_line(secs: u64) -> String {
    format!("expires in {secs}s.")
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
/// while help is open: role selection on the main menu and the
/// API-key change on Expert Setup / Session Setup are safe to
/// dispatch without closing help first.
pub fn help_overlay_dispatches(code: crossterm::event::KeyCode, screen: AppScreen) -> bool {
    (screen == AppScreen::RoleSelect
        && matches!(
            code,
            crossterm::event::KeyCode::Char('e') | crossterm::event::KeyCode::Char('c')
        ))
        || (matches!(screen, AppScreen::Preflight | AppScreen::Setup)
            && code == crossterm::event::KeyCode::Char('m'))
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
