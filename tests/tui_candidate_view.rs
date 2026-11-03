//! Candidate snapshot view: HTML statements become plain text, the
//! waiting state is detectable, and panes split question from chat.

use pairproof::orchestration::control::{
    CandidateSnapshot, SnapshotOption, SnapshotQuestion, SnapshotSelection,
};
use pairproof::tui::candidate_runtime::CandidateView;
use pairproof::tui::candidate_stage::PendingApproach;
use pairproof::tui::render::draw_candidate_screen;
use pairproof::tui::shared::html_to_text;
use ratatui::{backend::TestBackend, Terminal};

fn snapshot(statement: Option<String>) -> CandidateSnapshot {
    CandidateSnapshot {
        v: 1,
        session_id: "s1".to_string(),
        phase: "InProgress".to_string(),
        sync_phase: "Sync: active".to_string(),
        current: Some(SnapshotQuestion {
            index: 0,
            title: "Increasing Decreasing String".to_string(),
            slug: "increasing-decreasing-string".to_string(),
            statement,
        }),
        options: vec![SnapshotOption {
            title: "Sort first".to_string(),
            summary: "Sort then sweep".to_string(),
        }],
        nav: Vec::new(),
        selected: None,
        latest_test: None,
        chat_replies: vec!["Try sorting first.".to_string()],
        final_breakdown: None,
        chat_partial: None,
        recovery: None,
    }
}

#[test]
fn status_phase_is_written_as_in_progress() {
    let view = CandidateView::from_snapshot(Some(&snapshot(Some("S.".to_string()))));
    assert!(
        view.status_lines()
            .iter()
            .any(|line| line == "phase: in progress"),
        "{:?}",
        view.status_lines()
    );
}

#[test]
fn html_statement_arrives_as_plain_text_lines() {
    let html = "<p>You are given a string <code>s</code>.</p>\
                <ol><li>Remove the <strong>smallest</strong> character.</li>\
                <li>Repeat until done.</li></ol>\
                <p><strong>Input:</strong> s = &quot;abc&quot;&nbsp;done</p>";
    let view = CandidateView::from_snapshot(Some(&snapshot(Some(html.to_string()))));
    let statement = view.statement.clone().expect("statement");
    assert!(!statement.contains('<'), "{statement}");
    assert!(
        statement.contains("You are given a string s."),
        "{statement}"
    );
    assert!(
        statement.contains("- Remove the smallest character."),
        "{statement}"
    );
    assert!(statement.contains("s = \"abc\" done"), "{statement}");
    let lines = view.question_lines();
    assert_eq!(lines[0], "Q1: Increasing Decreasing String");
    assert!(lines.iter().all(|line| !line.contains('<')), "{lines:?}");
    assert!(!view.waiting_for_statement());
}

#[test]
fn missing_statement_is_waiting_and_panes_split() {
    let waiting = CandidateView::from_snapshot(Some(&snapshot(None)));
    assert!(waiting.waiting_for_statement());
    let view =
        CandidateView::from_snapshot(Some(&snapshot(Some("<p>Plain statement.</p>".to_string()))));
    let left = view.question_lines().join("\n");
    assert!(left.contains("Plain statement."), "{left}");
    assert!(left.contains("1: Sort first - Sort then sweep"), "{left}");
    assert!(!left.contains("Talking with me"), "{left}");
    let right = view.chat_lines();
    assert_eq!(
        right[0],
        "> Talking with me won't affect your score. \
         You can ask me anything about the question on the left, in any language you like."
    );
    assert_eq!(right[1], "");
    assert_eq!(right[2], "> Try sorting first.");
}

#[test]
fn coach_transcript_puts_the_candidate_between_replies() {
    let mut view = CandidateView::from_snapshot(Some(&snapshot(Some("S.".to_string()))));
    view.chat_replies = vec!["Think about the bounds.".to_string()];
    view.chat_partial = None;
    let lines = view.chat_transcript(&["where do I start?".to_string()]);
    assert!(lines[0].starts_with("> "), "{lines:?}");
    assert!(
        lines.iter().any(|line| line == "< where do I start?"),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|line| line == "> Think about the bounds."),
        "{lines:?}"
    );
    let you = lines
        .iter()
        .position(|line| line.starts_with("< "))
        .expect("you");
    let reply = lines
        .iter()
        .position(|line| line == "> Think about the bounds.")
        .expect("reply");
    assert!(you < reply, "{lines:?}");
}

#[test]
fn coach_pane_title_is_coach_not_ai_coach() {
    let view =
        CandidateView::from_snapshot(Some(&snapshot(Some("<p>Plain statement.</p>".to_string()))));
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| {
            draw_candidate_screen(
                frame,
                &view.question_lines(),
                &view.chat_lines(),
                true,
                60,
                0,
                0,
            )
        })
        .expect("draw");
    let rendered: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(rendered.contains("Coach"), "{rendered}");
    assert!(!rendered.contains("AI coach"), "{rendered}");
    assert!(!rendered.contains("AI Coach"), "{rendered}");
}

/// Column of `needle` on row `y`. Box-drawing symbols are more than
/// one byte, so a byte index into the joined row is not a column.
fn column_of(buffer: &ratatui::buffer::Buffer, y: u16, needle: &str) -> Option<u16> {
    let mut text = String::new();
    let mut byte_to_col = Vec::new();
    for x in 0..buffer.area.width {
        let symbol = buffer[(x, y)].symbol();
        byte_to_col.extend(std::iter::repeat_n(x, symbol.len()));
        text.push_str(symbol);
    }
    text.find(needle).map(|byte| byte_to_col[byte])
}

#[test]
fn candidate_message_draws_on_the_right_with_a_navy_marker() {
    use pairproof::tui::render::{ENTER_KEY_COLOR, HEADING_COLOR, TEXT_COLOR};
    let view = CandidateView::from_snapshot(None);
    let mut right = view.chat_transcript(&["hello".to_string()]);
    right.push("message: typed".to_string());
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| draw_candidate_screen(frame, &view.question_lines(), &right, true, 60, 0, 0))
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let mut found = None;
    for y in 0..buffer.area.height {
        if column_of(&buffer, y, "< hello").is_some() {
            found = Some(y);
            break;
        }
    }
    let y = found.expect("candidate line on screen");
    let col = column_of(&buffer, y, "< hello").expect("column");
    // Coach pane starts at 60% of 80. A right-aligned `< hello`
    // sits against the right padding, well past the pane's left edge.
    assert!(col > 65, "hello should stick to the right, col {col}");
    assert_eq!(buffer[(col, y)].symbol(), "<");
    assert_eq!(buffer[(col, y)].fg, HEADING_COLOR);
    assert_eq!(buffer[(col + 2, y)].fg, TEXT_COLOR);
    assert_eq!(buffer[(col - 1, y)].symbol(), " ");
    let mut coach_at = None;
    for row_y in 0..buffer.area.height {
        if let Some(marker_col) = column_of(&buffer, row_y, "> Talking") {
            coach_at = Some((marker_col, row_y));
            break;
        }
    }
    let (marker_col, marker_y) = coach_at.expect("coach welcome");
    assert!(marker_col < 55, "coach stays on the left, col {marker_col}");
    assert_eq!(buffer[(marker_col, marker_y)].fg, HEADING_COLOR);
    assert_eq!(buffer[(marker_col + 2, marker_y)].fg, TEXT_COLOR);
    let mut input_at = None;
    for row_y in 0..buffer.area.height {
        if let Some(marker_col) = column_of(&buffer, row_y, "> typed") {
            input_at = Some((marker_col, row_y));
            break;
        }
    }
    let (marker_col, marker_y) = input_at.expect("pinned input");
    assert_eq!(buffer[(marker_col, marker_y)].symbol(), ">");
    assert_eq!(buffer[(marker_col, marker_y)].fg, ENTER_KEY_COLOR);
    assert_eq!(marker_y, buffer.area.height - 2);
    let rule: String = (0..buffer.area.width)
        .map(|x| buffer[(x, marker_y - 1)].symbol().to_string())
        .collect();
    assert!(rule.contains('\u{2500}'), "{rule}");
    assert!(!rule.contains("message:"), "{rule}");
}

#[test]
fn coach_welcome_is_first_even_before_any_reply() {
    let view = CandidateView::from_snapshot(None);
    let lines = view.chat_lines();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].starts_with("> Talking with me"), "{lines:?}");
    assert!(lines[0].contains("won't affect your score"), "{lines:?}");
    assert!(lines[0].contains("in any language you like"), "{lines:?}");
}

#[test]
fn html_to_text_decodes_entities_and_bullets() {
    let text = html_to_text("<p>a &amp; b</p><ul><li>one</li><li>two &#65;</li></ul>");
    assert!(text.contains("a & b"), "{text}");
    assert!(text.contains("- one"), "{text}");
    assert!(text.contains("- two A"), "{text}");
    assert!(!text.contains('<'));
}

#[test]
fn pending_approach_shows_until_the_snapshot_records_it() {
    let view = CandidateView::from_snapshot(Some(&snapshot(Some("S.".to_string()))));
    let pending = PendingApproach {
        index: 0,
        selection: SnapshotSelection::Preset(0),
    };
    let lines = view.status_lines_with_pending(Some(&pending));
    assert_eq!(
        lines[0], "selected approach: preset 1, writing the code",
        "{lines:?}"
    );
    let mut caught_up = view.clone();
    caught_up.selected = Some(SnapshotSelection::Preset(0));
    let landed = caught_up.status_lines_with_pending(Some(&pending));
    assert_eq!(landed[0], "selected approach: preset 1", "{landed:?}");
}

#[test]
fn final_exit_line_draws_enter_in_green() {
    use pairproof::tui::render::{draw_screen, ENTER_KEY_COLOR};
    use pairproof::tui::viewport::final_frame;
    let source = vec![
        "Final breakdown".to_string(),
        "question detail".to_string(),
        "final rating: 7.50 / 10".to_string(),
        "solved: 1".to_string(),
        "enter to exit".to_string(),
    ];
    let frame = final_frame(&source, 20, 0);
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|ui| draw_screen(ui, &frame.lines, true))
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let mut found = None;
    for y in 0..buffer.area.height {
        if column_of(&buffer, y, "enter to exit").is_some() {
            found = Some(y);
            break;
        }
    }
    let y = found.expect("exit line");
    let col = column_of(&buffer, y, "enter to exit").expect("column");
    assert_eq!(buffer[(col, y)].symbol(), "e");
    assert_eq!(buffer[(col, y)].fg, ENTER_KEY_COLOR);
}

#[test]
fn expert_quit_without_a_breakdown_tells_the_candidate_the_client_closed() {
    let mut view = CandidateView::from_snapshot(None);
    view.phase = "EndedByExpert".to_string();
    let lines = view.final_screen_lines();
    let joined = lines.join("\n");
    assert!(joined.contains("the expert's client closed."), "{joined}");
    assert!(joined.contains("the session is over."), "{joined}");
    assert_eq!(lines.last().map(String::as_str), Some("enter to close"));
    assert!(!joined.contains("technical failure"), "{joined}");
    view.phase = "TechnicalFailure".to_string();
    let failure = view.final_screen_lines().join("\n");
    assert!(
        failure.contains("the session ended as a technical failure."),
        "{failure}"
    );
}
