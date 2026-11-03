//! Candidate live screen composition: centring, single-column
//! lines, and the two-pane solving lines.

use super::*;

/// Default split and the draggable range for the two-pane solving
/// screen: the question pane starts at 60% and either pane may
/// shrink to 30% (so the other grows to 70%).
pub const DEFAULT_QUESTION_PCT: u16 = 60;
pub(crate) const MIN_PANE_PCT: u16 = 30;
pub(crate) const MAX_QUESTION_PCT: u16 = 70;

/// Column of the pane divider for a terminal `width` at `pct`.
pub(crate) fn divider_column(width: u16, pct: u16) -> u16 {
    ((u32::from(width) * u32::from(pct)) / 100) as u16
}

/// Width of the question pane, matching the horizontal layout split.
pub fn question_pane_width(term_width: u16, pct: u16) -> u16 {
    divider_column(term_width, pct)
}

/// Width of the coach pane, the remainder after the question pane.
pub fn coach_pane_width(term_width: u16, pct: u16) -> u16 {
    term_width.saturating_sub(question_pane_width(term_width, pct))
}

/// Text columns inside a pane: outer border plus two columns of
/// padding on each side. Must match `render_candidate`.
pub fn pane_content_width(pane_width: u16) -> usize {
    usize::from(pane_width).saturating_sub(6)
}

/// Scrollable rows in the question pane: full height, minus the
/// border and the pinned key hint.
pub fn question_scroll_rows(term_height: usize) -> usize {
    term_height.saturating_sub(3).max(1)
}

/// Scrollable rows in the coach pane. `status_rows` is the number
/// of status text lines (0 hides the box). The border, the rule
/// above the input, and the input line stay pinned.
pub fn coach_scroll_rows(term_height: usize, status_rows: usize) -> usize {
    let status_box = if status_rows == 0 {
        0
    } else {
        status_rows.saturating_add(2)
    };
    // border (2) + separator (1) + input (1)
    term_height.saturating_sub(status_box + 4).max(1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneSide {
    Question,
    Coach,
}

pub fn pane_side_for_column(column: u16, width: u16, pct: u16) -> PaneSide {
    if column < divider_column(width, pct) {
        PaneSide::Question
    } else {
        PaneSide::Coach
    }
}

/// Pane clicked at `column`: `Question` left of the divider,
/// `Coach` at/right of it. Row-independent: a click anywhere on
/// a pane targets that pane.
pub fn pane_click_target(column: u16, width: u16, pct: u16) -> PaneSide {
    pane_side_for_column(column, width, pct)
}

/// Focus after clicking a panel. A click anywhere on a pane
/// selects it; the divider column still starts a resize drag
/// instead (handled by the caller checking `divider_hit` first).
/// Coach selects Chat; Question selects Work only when leaving
/// Chat, otherwise the current sub-focus (Work/Other/Redirect)
/// is kept so typed text and its pane stay intact. This is the
/// single panel-switch transition used by mouse; Tab keeps the
/// existing `next_focus` cycle. Both routes cross panels through
/// the same Work<->Chat boundary: Tab from Chat lands on Work and
/// the cycle reaches Chat from the question sub-focuses, while a
/// panel click jumps directly across that same boundary.
pub fn focus_for_pane(focus: CandidateFocus, side: PaneSide) -> CandidateFocus {
    match side {
        PaneSide::Coach => CandidateFocus::Chat,
        PaneSide::Question => {
            if focus == CandidateFocus::Chat {
                CandidateFocus::Work
            } else {
                focus
            }
        }
    }
}

pub fn max_scroll(total_lines: usize, visible_rows: usize) -> usize {
    total_lines.saturating_sub(visible_rows)
}

pub fn clamp_scroll(scroll: usize, total_lines: usize, visible_rows: usize) -> usize {
    if visible_rows == 0 {
        return 0;
    }
    scroll.min(max_scroll(total_lines, visible_rows))
}

pub fn scroll_after_growth(
    scroll: usize,
    following: bool,
    total_lines: usize,
    visible_rows: usize,
) -> usize {
    if following {
        max_scroll(total_lines, visible_rows)
    } else {
        clamp_scroll(scroll, total_lines, visible_rows)
    }
}

/// True when `column` grabs the divider (within one column).
pub fn divider_hit(column: u16, width: u16, pct: u16) -> bool {
    column.abs_diff(divider_column(width, pct)) <= 1
}

/// True when pressing Enter on Connect should first show the
/// loading screen and only then run the connection submit.
pub fn connect_submit_shows_loading(
    screen: CandidateScreen,
    input_len: usize,
    show_help: bool,
) -> bool {
    screen == CandidateScreen::Connect && input_len > 0 && !show_help
}

/// Question-pane percent for a drag to `column`, clamped so both
/// panes keep at least the minimum share.
pub fn split_pct_for_column(column: u16, width: u16) -> u16 {
    if width == 0 {
        return DEFAULT_QUESTION_PCT;
    }
    ((u32::from(column) * 100) / u32::from(width))
        .clamp(u32::from(MIN_PANE_PCT), u32::from(MAX_QUESTION_PCT)) as u16
}

/// Centred screens: help always, plus Connect and Rules before the
/// live session starts; later screens use the full area.
pub fn screen_centered(show_help: bool, live: bool, screen: CandidateScreen) -> bool {
    show_help || (!live && matches!(screen, CandidateScreen::Connect | CandidateScreen::Rules))
}

pub(crate) fn live_lines(
    state: &CandidateRunState,
    view: &CandidateView,
    connection_lost: bool,
    status: &Option<String>,
) -> Vec<String> {
    let mut lines = view.lines();
    if connection_lost {
        lines.push(CONNECTION_LOST.to_string());
        lines.push("press R to reconnect with a fresh string.".to_string());
    }
    // `view.lines()` is already sanitized by its producer; only the
    // raw typed texts appended here are sanitized, at this boundary.
    lines.extend(composer_lines(state));
    lines.push(sanitize_display(&format!(
        "message: {}",
        state.chat.input_text()
    )));
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
        question_hint(state, view.options.len())
    });
    lines
}

/// Two-pane live lines: question side (with the typed work inputs
/// and the focus hint) and coach side (replies plus the message input).
pub(crate) fn live_panes(
    state: &CandidateRunState,
    view: &CandidateView,
    connection_lost: bool,
    status: &Option<String>,
) -> (Vec<String>, Vec<String>) {
    let mut left = view.question_lines_staged(state.staging.current_stage());
    if connection_lost {
        left.push(CONNECTION_LOST.to_string());
        left.push("press R to reconnect with a fresh string.".to_string());
    }
    left.extend(composer_lines(state));
    if state.paste_notice || state.chat.paste_notice() {
        left.push("pasting is blocked here; type instead.".to_string());
    }
    if let Some(text) = status {
        left.push(sanitize_display(text));
    }
    if let Some(banner) = work_banner(state, view) {
        left.push(banner);
    }
    left.push(question_hint(state, view.options.len()));
    let mut right = view.chat_transcript(&state.chat.sent_messages());
    right.push(sanitize_display(&format!(
        "message: {}",
        state.chat.input_text()
    )));
    (left, right)
}

/// The input line for the field that is actually receiving keys.
/// `other` and `redirect` are separate actions; only the active one
/// is shown, so typed text cannot appear in the other field.
fn composer_lines(state: &CandidateRunState) -> Vec<String> {
    match state.focus {
        CandidateFocus::Other => {
            vec![sanitize_display(&format!(
                "other: {}",
                state.work.other_text()
            ))]
        }
        CandidateFocus::Redirect => {
            vec![sanitize_display(&format!(
                "additional notes: {}",
                state.work.redirect_text()
            ))]
        }
        CandidateFocus::Work | CandidateFocus::Chat => {
            let mut lines = Vec::new();
            if !state.work.other_text().is_empty() {
                lines.push(sanitize_display(&format!(
                    "other: {}",
                    state.work.other_text()
                )));
            }
            if !state.work.redirect_text().is_empty() {
                lines.push(sanitize_display(&format!(
                    "additional notes: {}",
                    state.work.redirect_text()
                )));
            }
            lines
        }
    }
}

/// Lines pinned under the question body: the optional work banner,
/// then the key hint. Scroll math must skip both.
pub fn question_pinned_count(lines: &[String]) -> usize {
    let mut count = 0usize;
    if lines.last().is_some_and(|line| line.starts_with("focus:")) {
        count += 1;
        if lines.len() >= 2 {
            let previous = &lines[lines.len() - 2];
            if previous.starts_with("working:") || previous.starts_with("ready:") {
                count += 1;
            }
        }
    }
    count
}

/// What the question pane says while code is being written, and
/// once that code has landed in the question folder.
fn work_banner(state: &CandidateRunState, view: &CandidateView) -> Option<String> {
    if let Some(pending) = &state.pending_approach {
        if view.question_index == Some(pending.index)
            && view.selected.as_ref() != Some(&pending.selection)
        {
            return Some(format!(
                "working: writing {}. this takes a moment",
                pending.label()
            ));
        }
    }
    if let Some((index, text)) = &state.after_code {
        if view.question_index == Some(*index) {
            return Some(format!("ready: {text}"));
        }
    }
    None
}

fn cursor_on_other(state: &CandidateRunState, option_count: usize) -> bool {
    option_count > 0
        && state
            .staging
            .current_stage()
            .is_some_and(|stage| stage.cursor == option_count)
}

/// Bottom hint. On the other row it describes the approach field,
/// because that row is where the candidate types.
fn question_hint(state: &CandidateRunState, option_count: usize) -> String {
    // The `focus:` prefix is a pin marker. The question pane strips
    // it and colours the key names like the other screens.
    match state.focus {
        CandidateFocus::Other => {
            "focus: enter submit, backspace delete, tab next, esc quit".to_string()
        }
        CandidateFocus::Redirect => {
            "focus: enter send, backspace delete, tab next, esc quit".to_string()
        }
        CandidateFocus::Chat => {
            "focus: enter send, backspace delete, up/down scroll, tab panel, esc quit".to_string()
        }
        _ if cursor_on_other(state, option_count) => {
            "focus: enter submit, backspace delete, tab next, esc quit".to_string()
        }
        _ => "focus: up/down move, enter submit, left/right question, tab panel, esc quit"
            .to_string(),
    }
}
