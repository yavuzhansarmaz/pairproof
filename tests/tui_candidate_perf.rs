//! Slice E: bounded viewport rendering and resize behaviour.

use pairproof::tui::candidate_runtime::{clamp_scroll, scroll_after_growth, CandidateView};
use pairproof::tui::render::{draw_candidate_screen, FRAME_COLOR, PROOF_COLOR};
use pairproof::tui::viewport::{wrapped_rows, wrapped_window};

fn taken_count(lines: &[String], width: usize, scroll: usize, visible: usize) -> usize {
    let (start, local) = wrapped_window(lines, width, scroll, visible);
    let need = local + visible + 1;
    let mut covered = 0usize;
    let mut count = 0usize;
    for line in lines.iter().skip(start) {
        if covered >= need && count > 0 {
            break;
        }
        covered += wrapped_rows(line, width);
        count += 1;
    }
    count
}

fn drawn(
    left: &[String],
    right: &[String],
    chat_focused: bool,
    q_scroll: usize,
    c_scroll: usize,
    width: u16,
    height: u16,
) -> ratatui::buffer::Buffer {
    let backend = ratatui::backend::TestBackend::new(width, height);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| {
            draw_candidate_screen(frame, left, right, chat_focused, 60, q_scroll, c_scroll)
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

fn base_view() -> CandidateView {
    CandidateView {
        phase: "InProgress".to_string(),
        sync_phase: String::new(),
        question_title: Some("Two Sum".to_string()),
        question_index: Some(0),
        statement: Some("distinctive statement line".to_string()),
        options: Vec::new(),
        selected: None,
        latest_test: None,
        chat_replies: vec!["distinctive coach reply".to_string()],
        chat_partial: None,
        final_breakdown: None,
        recovery: None,
    }
}

#[test]
fn large_coach_transcript_uses_bounded_viewport_rendering() {
    let lines: Vec<String> = (0..500).map(|i| format!("coach line {i:03}")).collect();
    let visible = 20usize;
    let width = 30usize;
    let scroll = 480usize;
    let (start, _local) = wrapped_window(&lines, width, scroll, visible);
    assert!(start > 0, "start {start}");
    assert!(taken_count(&lines, width, scroll, visible) <= visible + 2);
    // Draw scrolled to bottom shows the last line.
    let left = vec!["Q1: Two Sum".to_string(), "focus: x".to_string()];
    let mut right = lines.clone();
    right.push("COACHLAST final".to_string());
    right.push("message: ".to_string());
    let buffer = drawn(&left, &right, true, 0, usize::MAX / 2, 80, 24);
    let text = buffer_text(&buffer);
    assert!(text.contains("COACHLAST"), "{text}");
}

#[test]
fn large_question_statement_uses_bounded_viewport_rendering() {
    let lines: Vec<String> = (0..500).map(|i| format!("question line {i:03}")).collect();
    let visible = 20usize;
    let width = 40usize;
    let scroll = 480usize;
    let (start, _local) = wrapped_window(&lines, width, scroll, visible);
    assert!(start > 0, "start {start}");
    assert!(taken_count(&lines, width, scroll, visible) <= visible + 2);
    let mut left = lines.clone();
    left.push("QUESTIONLAST final".to_string());
    left.push("focus: x".to_string());
    let right = vec!["coach: hi".to_string(), "message: ".to_string()];
    let buffer = drawn(&left, &right, false, usize::MAX / 2, 0, 80, 24);
    let text = buffer_text(&buffer);
    assert!(text.contains("QUESTIONLAST"), "{text}");
}

#[test]
fn scroll_events_do_not_rebuild_unchanged_chat_history() {
    let lines500: Vec<String> = (0..500).map(|i| format!("line {i:03}")).collect();
    let lines50: Vec<String> = (0..50).map(|i| format!("line {i:03}")).collect();
    let first = wrapped_window(&lines500, 30, 40, 20);
    let second = wrapped_window(&lines500, 30, 40, 20);
    assert_eq!(first, second);
    assert_eq!(
        taken_count(&lines500, 30, 20, 20),
        taken_count(&lines50, 30, 20, 20)
    );
    assert!(taken_count(&lines500, 30, 40, 20) <= 22);
    assert!(taken_count(&lines50, 30, 20, 20) <= 22);
}

#[test]
fn panel_resize_preserves_content_and_focus() {
    let view = base_view();
    let mut left = view.question_lines();
    left.push("focus: x".to_string());
    let mut right = view.chat_lines();
    right.push("message: ".to_string());
    for (width, height) in [(80u16, 24u16), (100, 30)] {
        for focused in [false, true] {
            let buffer = drawn(&left, &right, focused, 0, 0, width, height);
            let text = buffer_text(&buffer);
            assert!(text.contains("distinctive statement"), "{text}");
            assert!(text.contains("distinctive coach"), "{text}");
            // Left pane top-left border and right pane top-left border.
            let left_fg = buffer[(0, 0)].fg;
            let divider_x = ((u32::from(width) * 60) / 100) as u16;
            let right_fg = buffer[(divider_x, 0)].fg;
            if focused {
                assert_eq!(right_fg, PROOF_COLOR);
                assert_eq!(left_fg, FRAME_COLOR);
            } else {
                assert_eq!(left_fg, PROOF_COLOR);
                assert_eq!(right_fg, FRAME_COLOR);
            }
        }
    }
}

#[test]
fn panel_resize_preserves_scroll_state() {
    // Still fits after resize: preserved.
    assert_eq!(clamp_scroll(10, 100, 20), 10);
    assert_eq!(scroll_after_growth(10, false, 100, 30), 10);
    // Terminal shrinks (larger visible claim / smaller content fit):
    // clamped to the new maximum.
    assert_eq!(clamp_scroll(90, 100, 50), 50);
    assert_eq!(scroll_after_growth(90, false, 100, 50), 50);
    assert_eq!(clamp_scroll(90, 100, 95), 5);
}

#[test]
fn panel_resize_does_not_interrupt_active_stream() {
    let mut view = base_view();
    view.chat_partial = Some("STREAMINGPARTIALXYZ token".to_string());
    let chat = view.chat_lines();
    assert!(
        chat.iter().any(|line| line.contains("STREAMINGPARTIALXYZ")),
        "{chat:?}"
    );
    let mut left = view.question_lines();
    left.push("focus: x".to_string());
    let mut right = chat;
    right.push("message: ".to_string());
    for (width, height) in [(80u16, 24u16), (100, 30)] {
        let buffer = drawn(&left, &right, true, 0, 0, width, height);
        let text = buffer_text(&buffer);
        assert!(text.contains("STREAMINGPARTIALXYZ"), "{text}");
    }
}
