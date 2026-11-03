//! Candidate live screen composition: centring, single-column
//! lines, and the two-pane solving lines.

use super::*;

/// Centred screens: help always, plus Connect and Rules before the
/// live session starts; later screens use the full area.
pub(crate) fn screen_centered(show_help: bool, live: bool, screen: CandidateScreen) -> bool {
    show_help || (!live && matches!(screen, CandidateScreen::Connect | CandidateScreen::Rules))
}

pub(crate) fn live_lines(
    state: &CandidateRunState,
    runtime: &CandidateRuntime<SshCandidateTransport>,
    status: &Option<String>,
) -> Vec<String> {
    let view = runtime.view();
    let mut lines = view.lines();
    if runtime.connection_lost() {
        lines.push(CONNECTION_LOST.to_string());
        lines.push("press R to reconnect with a fresh string.".to_string());
    }
    // `view.lines()` is already sanitized by its producer; only the
    // raw typed texts appended here are sanitized, at this boundary.
    lines.extend([
        sanitize_display(&format!("other: {}", state.work.other_text())),
        sanitize_display(&format!("redirect: {}", state.work.redirect_text())),
        sanitize_display(&format!("message: {}", state.chat.input_text())),
    ]);
    if state.paste_notice || state.chat.paste_notice() {
        lines.push("pasting is blocked here; type instead.".to_string());
    }
    if let Some(text) = status {
        lines.push(sanitize_display(text));
    }
    // The key-hint line always stays at the very bottom.
    lines.push(if view.is_terminal() {
        "enter quit".to_string()
    } else {
        "focus: work/other/redirect/chat (tab switches)".to_string()
    });
    lines
}

/// Two-pane live lines: question side (with the typed work inputs
/// and the focus hint) and coach side (replies plus the message input).
pub(crate) fn live_panes(
    state: &CandidateRunState,
    runtime: &CandidateRuntime<SshCandidateTransport>,
    status: &Option<String>,
) -> (Vec<String>, Vec<String>) {
    let view = runtime.view();
    let mut left = view.question_lines();
    if runtime.connection_lost() {
        left.push(CONNECTION_LOST.to_string());
        left.push("press R to reconnect with a fresh string.".to_string());
    }
    left.extend([
        sanitize_display(&format!("other: {}", state.work.other_text())),
        sanitize_display(&format!("redirect: {}", state.work.redirect_text())),
    ]);
    if state.paste_notice || state.chat.paste_notice() {
        left.push("pasting is blocked here; type instead.".to_string());
    }
    if let Some(text) = status {
        left.push(sanitize_display(text));
    }
    left.push("focus: work/other/redirect/chat (tab switches)".to_string());
    let mut right = view.chat_lines();
    right.push(sanitize_display(&format!(
        "message: {}",
        state.chat.input_text()
    )));
    (left, right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_and_rules_centre_before_live() {
        assert!(screen_centered(false, false, CandidateScreen::Connect));
        assert!(screen_centered(false, false, CandidateScreen::Rules));
        assert!(!screen_centered(false, false, CandidateScreen::Waiting));
    }

    #[test]
    fn live_screens_use_full_area_unless_help_is_open() {
        assert!(!screen_centered(false, true, CandidateScreen::Rules));
        assert!(!screen_centered(false, true, CandidateScreen::Solving));
        assert!(screen_centered(true, true, CandidateScreen::Solving));
    }
}
