//! Candidate live screen composition: centring, single-column
//! lines, and the two-pane solving lines.

use super::*;

/// Default split and the draggable range for the two-pane solving
/// screen: the question pane starts at 60% and either pane may
/// shrink to 30% (so the other grows to 70%).
pub(crate) const DEFAULT_QUESTION_PCT: u16 = 60;
pub(crate) const MIN_PANE_PCT: u16 = 30;
pub(crate) const MAX_QUESTION_PCT: u16 = 70;

/// Column of the pane divider for a terminal `width` at `pct`.
pub(crate) fn divider_column(width: u16, pct: u16) -> u16 {
    ((u32::from(width) * u32::from(pct)) / 100) as u16
}

/// True when `column` grabs the divider (within one column).
pub(crate) fn divider_hit(column: u16, width: u16, pct: u16) -> bool {
    column.abs_diff(divider_column(width, pct)) <= 1
}

/// True when pressing Enter on Connect should first show the
/// loading screen and only then run the connection submit.
pub(crate) fn connect_submit_shows_loading(
    screen: CandidateScreen,
    input_len: usize,
    show_help: bool,
) -> bool {
    screen == CandidateScreen::Connect && input_len > 0 && !show_help
}

/// Question-pane percent for a drag to `column`, clamped so both
/// panes keep at least the minimum share.
pub(crate) fn split_pct_for_column(column: u16, width: u16) -> u16 {
    if width == 0 {
        return DEFAULT_QUESTION_PCT;
    }
    ((u32::from(column) * 100) / u32::from(width))
        .clamp(u32::from(MIN_PANE_PCT), u32::from(MAX_QUESTION_PCT)) as u16
}

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
mod split_tests {
    use super::*;

    #[test]
    fn split_defaults_sixty_forty_and_clamps_both_panes_to_thirty() {
        assert_eq!(DEFAULT_QUESTION_PCT, 60);
        assert_eq!(split_pct_for_column(60, 100), 60);
        assert_eq!(split_pct_for_column(10, 100), 30);
        assert_eq!(split_pct_for_column(95, 100), 70);
        assert_eq!(split_pct_for_column(0, 0), 60);
    }

    #[test]
    fn connect_submit_shows_loading_only_for_nonempty_connect_enter() {
        assert!(connect_submit_shows_loading(
            CandidateScreen::Connect,
            12,
            false
        ));
        assert!(!connect_submit_shows_loading(
            CandidateScreen::Connect,
            0,
            false
        ));
        assert!(!connect_submit_shows_loading(
            CandidateScreen::Connect,
            12,
            true
        ));
        assert!(!connect_submit_shows_loading(
            CandidateScreen::Rules,
            12,
            false
        ));
    }

    #[test]
    fn divider_hit_covers_the_border_column() {
        assert!(divider_hit(60, 100, 60));
        assert!(divider_hit(59, 100, 60));
        assert!(divider_hit(61, 100, 60));
        assert!(!divider_hit(50, 100, 60));
    }
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
