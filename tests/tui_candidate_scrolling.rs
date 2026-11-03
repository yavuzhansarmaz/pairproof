//! Independent Question/Coach scrolling for the candidate panes.

use pairproof::tui::candidate_runtime::{
    clamp_scroll, max_scroll, pane_side_for_column, scroll_after_growth, PaneSide,
};
use pairproof::tui::render::draw_candidate_screen;
use pairproof::tui::viewport::{clamp_wrapped_scroll, wrapped_row_count};

fn left_lines(count: usize) -> Vec<String> {
    let mut lines = vec!["QHEAD first question line".to_string()];
    for i in 0..count {
        lines.push(format!("QLINE {i:03} question body"));
    }
    lines.push("QTAIL last question line".to_string());
    lines.push("focus: work/other/redirect/chat (tab switches)".to_string());
    lines
}

fn right_lines(count: usize) -> Vec<String> {
    let mut lines = vec!["COACHHEAD first coach line".to_string()];
    for i in 0..count {
        lines.push(format!("CLINE {i:03} coach body"));
    }
    lines.push("CTAIL last coach line".to_string());
    lines.push("message: ".to_string());
    lines
}

fn drawn(
    left: &[String],
    right: &[String],
    chat_focused: bool,
    question_scroll: usize,
    coach_scroll: usize,
) -> ratatui::buffer::Buffer {
    let backend = ratatui::backend::TestBackend::new(80, 24);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| {
            draw_candidate_screen(
                frame,
                left,
                right,
                chat_focused,
                60,
                question_scroll,
                coach_scroll,
            )
        })
        .expect("draw");
    terminal.backend().buffer().clone()
}

fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
    let area = *buffer.area();
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn question_panel_scrolls_independently() {
    let left = left_lines(30);
    let right = right_lines(30);
    let buffer = drawn(&left, &right, false, 15, 0);
    let text = buffer_text(&buffer);
    assert!(text.contains("QTAIL"), "{text}");
    assert!(!text.contains("QHEAD"), "{text}");
    assert!(text.contains("COACHHEAD"), "{text}");
}

#[test]
fn coach_panel_scrolls_independently() {
    let left = left_lines(30);
    let right = right_lines(30);
    let buffer = drawn(&left, &right, true, 0, 15);
    let text = buffer_text(&buffer);
    assert!(text.contains("CTAIL"), "{text}");
    assert!(!text.contains("COACHHEAD"), "{text}");
    assert!(text.contains("QHEAD"), "{text}");
}

#[test]
fn panel_switch_preserves_scroll_positions() {
    let left = left_lines(30);
    let right = right_lines(30);
    let first = buffer_text(&drawn(&left, &right, false, 15, 15));
    let second = buffer_text(&drawn(&left, &right, true, 15, 15));
    for text in [&first, &second] {
        assert!(text.contains("QTAIL"), "{text}");
        assert!(text.contains("CTAIL"), "{text}");
        assert!(!text.contains("QHEAD"), "{text}");
        assert!(!text.contains("COACHHEAD"), "{text}");
    }
    assert_eq!(first, second);
}

#[test]
fn coach_scroll_handles_long_conversation() {
    let left = left_lines(5);
    let right: Vec<String> = (0..200)
        .map(|i| format!("coach line {i:03}"))
        .chain([("message: ".to_string())])
        .collect();
    let scroll = max_scroll(200, 20);
    let buffer = drawn(&left, &right, true, 0, scroll);
    let text = buffer_text(&buffer);
    assert!(text.contains("coach line 199"), "{text}");
    assert!(!text.contains("coach line 000"), "{text}");
}

#[test]
fn question_scroll_handles_long_statement() {
    let left: Vec<String> = (0..200)
        .map(|i| format!("question line {i:03}"))
        .chain([("focus: work/other/redirect/chat (tab switches)".to_string())])
        .collect();
    let right = right_lines(5);
    let scroll = max_scroll(200, 20);
    let buffer = drawn(&left, &right, false, scroll, 0);
    let text = buffer_text(&buffer);
    assert!(text.contains("question line 199"), "{text}");
    assert!(!text.contains("question line 000"), "{text}");
}

#[test]
fn streaming_does_not_force_scroll_when_reading_history() {
    assert_eq!(scroll_after_growth(5, false, 100, 20), 5);
    assert_eq!(scroll_after_growth(5, false, 140, 20), 5);
}

#[test]
fn streaming_keeps_bottom_when_user_follows_latest_message() {
    assert_eq!(scroll_after_growth(0, true, 120, 20), max_scroll(120, 20));
    assert_eq!(scroll_after_growth(0, true, 120, 20), 100);
}

#[test]
fn scroll_position_is_clamped_to_content() {
    assert_eq!(clamp_scroll(usize::MAX / 2, 50, 20), 30);
    assert_eq!(clamp_scroll(10, 50, 0), 0);
    let left = left_lines(30);
    let right = right_lines(30);
    let buffer = drawn(&left, &right, true, usize::MAX / 2, usize::MAX / 2);
    let text = buffer_text(&buffer);
    assert!(text.contains("QTAIL"), "{text}");
    assert!(text.contains("CTAIL"), "{text}");
    assert!(!text.contains("QHEAD"), "{text}");
    assert!(!text.contains("COACHHEAD"), "{text}");
}

#[test]
fn coach_scroll_uses_wrapped_rows_for_long_replies() {
    let coach: Vec<String> = (0..8)
        .map(|i| {
            format!("coach{i} {}", "fillerword ".repeat(7))
                .trim()
                .to_string()
        })
        .collect();
    let visible_estimate = 21usize;
    let logical_max = max_scroll(coach.len(), visible_estimate);
    let wrapped_total = wrapped_row_count(&coach, 26);
    let wrapped_max = max_scroll(wrapped_total, visible_estimate);
    assert_eq!(logical_max, 0);
    assert!(wrapped_total > coach.len() * 2, "{wrapped_total}");
    assert!(wrapped_max > 0, "{wrapped_total}");
    let mut right = coach;
    right.push("message: ".to_string());
    let left = left_lines(5);
    let buffer = drawn(&left, &right, true, 0, wrapped_max);
    let text = buffer_text(&buffer);
    assert!(text.contains("coach7"), "{text}");
    assert!(!text.contains("coach0"), "{text}");
}

#[test]
fn wrapped_scroll_clamp_matches_content_bounds() {
    let lines = vec![
        "alpha beta gamma delta".to_string(),
        "epsilon zeta eta theta".to_string(),
    ];
    let total = wrapped_row_count(&lines, 10);
    assert!(total > 2, "{total}");
    assert_eq!(clamp_wrapped_scroll(&lines, 10, 1, 2), 1);
    assert_eq!(
        clamp_wrapped_scroll(&lines, 10, 999, 2),
        total.saturating_sub(2)
    );
    assert_eq!(clamp_wrapped_scroll(&[], 10, 10, 2), 0);
}

#[test]
fn mouse_wheel_scrolls_active_panel() {
    assert_eq!(pane_side_for_column(10, 80, 60), PaneSide::Question);
    assert_eq!(pane_side_for_column(47, 80, 60), PaneSide::Question);
    assert_eq!(pane_side_for_column(70, 80, 60), PaneSide::Coach);
    assert_eq!(pane_side_for_column(48, 80, 60), PaneSide::Coach);
}
