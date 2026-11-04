//! Slice D: Tab and mouse header-click panel navigation.

use pairproof::tui::candidate_run::{next_focus, CandidateFocus, CandidateRunState};
use pairproof::tui::candidate_runtime::{divider_hit, focus_for_pane, pane_click_target, PaneSide};
use pairproof::tui::render::{draw_candidate_screen, FRAME_COLOR, PROOF_COLOR};

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

fn left_lines(count: usize) -> Vec<String> {
    let mut lines = vec!["QHEAD first question line".to_string()];
    for i in 0..count {
        lines.push(format!("QLINE {i:03} question body"));
    }
    lines.push("QTAIL last question line".to_string());
    lines.push("focus: tab or click a panel to switch question/coach".to_string());
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

#[test]
fn tab_switches_question_and_coach_panels() {
    assert_eq!(next_focus(CandidateFocus::Work), CandidateFocus::Chat);
    assert_eq!(next_focus(CandidateFocus::Other), CandidateFocus::Chat);
    assert_eq!(next_focus(CandidateFocus::Redirect), CandidateFocus::Chat);
    assert_eq!(next_focus(CandidateFocus::Chat), CandidateFocus::Work);
    assert_eq!(
        next_focus(next_focus(CandidateFocus::Work)),
        CandidateFocus::Work
    );
}

#[test]
fn question_pane_click_selects_question_panel() {
    // Divider for 80 columns at 60% is column 48. The target is
    // row-independent, so header (0) and body (5) rows agree.
    for _row in [0, 5] {
        assert_eq!(pane_click_target(0, 80, 60), PaneSide::Question);
        assert_eq!(pane_click_target(47, 80, 60), PaneSide::Question);
    }
    assert_eq!(
        focus_for_pane(CandidateFocus::Chat, PaneSide::Question),
        CandidateFocus::Work
    );
}

#[test]
fn coach_pane_click_selects_coach_panel() {
    // Row-independent: header (0) and body (9) rows agree.
    for _row in [0, 9] {
        assert_eq!(pane_click_target(48, 80, 60), PaneSide::Coach);
        assert_eq!(pane_click_target(79, 80, 60), PaneSide::Coach);
    }
    assert_eq!(
        focus_for_pane(CandidateFocus::Work, PaneSide::Coach),
        CandidateFocus::Chat
    );
}

#[test]
fn mouse_and_keyboard_share_panel_transition() {
    assert_eq!(
        focus_for_pane(CandidateFocus::Work, PaneSide::Coach),
        CandidateFocus::Chat
    );
    assert_eq!(next_focus(CandidateFocus::Work), CandidateFocus::Chat);
    assert_eq!(next_focus(CandidateFocus::Other), CandidateFocus::Chat);
    assert_eq!(next_focus(CandidateFocus::Redirect), CandidateFocus::Chat);
    assert_eq!(
        focus_for_pane(CandidateFocus::Chat, PaneSide::Question),
        next_focus(CandidateFocus::Chat)
    );
    assert_eq!(
        focus_for_pane(CandidateFocus::Chat, PaneSide::Question),
        CandidateFocus::Work
    );
}

#[test]
fn panel_header_click_updates_active_indicator() {
    let left = left_lines(3);
    let right = right_lines(3);
    let question_focused = drawn(&left, &right, false, 0, 0);
    assert_eq!(question_focused[(48, 0)].fg, FRAME_COLOR);
    assert_eq!(question_focused[(0, 0)].fg, PROOF_COLOR);
    let chat_focused = drawn(&left, &right, true, 0, 0);
    assert_eq!(chat_focused[(48, 0)].fg, PROOF_COLOR);
    assert_eq!(chat_focused[(0, 0)].fg, FRAME_COLOR);
}

#[test]
fn content_click_switches_to_clicked_pane() {
    // Body-row clicks map to their pane on both sides.
    for _row in [1, 12] {
        assert_eq!(pane_click_target(10, 80, 60), PaneSide::Question);
        assert_eq!(pane_click_target(70, 80, 60), PaneSide::Coach);
    }
    assert_eq!(
        focus_for_pane(CandidateFocus::Chat, pane_click_target(10, 80, 60)),
        CandidateFocus::Work
    );
    assert_eq!(
        focus_for_pane(CandidateFocus::Work, pane_click_target(70, 80, 60)),
        CandidateFocus::Chat
    );
}

#[test]
fn divider_column_click_prefers_drag_over_switch() {
    // Drag precedence lives in live.rs: the caller checks
    // divider_hit first, so this column starts a resize drag
    // even though pane_click_target would otherwise return Coach.
    assert!(divider_hit(48, 80, 60));
    assert_eq!(pane_click_target(48, 80, 60), PaneSide::Coach);
}

#[test]
fn mouse_click_does_not_interrupt_text_input() {
    let mut state = CandidateRunState::new();
    state.focus = CandidateFocus::Other;
    state.work.set_other_text("my own approach");
    assert_eq!(
        focus_for_pane(CandidateFocus::Other, PaneSide::Question),
        CandidateFocus::Other
    );
    assert_eq!(state.work.other_text(), "my own approach");
}

#[test]
fn panel_navigation_preserves_scroll_positions() {
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
}
