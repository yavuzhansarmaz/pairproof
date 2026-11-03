//! Candidate snapshot view: HTML statements become plain text, the
//! waiting state is detectable, and panes split question from chat.

use pairproof::orchestration::control::{CandidateSnapshot, SnapshotOption, SnapshotQuestion};
use pairproof::tui::candidate_runtime::CandidateView;
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

#[test]
fn candidate_message_draws_on_the_right_of_the_coach_pane() {
    let view = CandidateView::from_snapshot(None);
    let mut right = view.chat_transcript(&["hello".to_string()]);
    right.push("message: ".to_string());
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| draw_candidate_screen(frame, &view.question_lines(), &right, false, 0, 0, 0))
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let mut found = None;
    for y in 0..buffer.area.height {
        let row: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol().to_string())
            .collect();
        if let Some(col) = row.find("hello") {
            found = Some((col, y, row));
            break;
        }
    }
    let (col, _y, row) = found.expect("hello on screen");
    assert!(col > 40, "hello should sit in the right pane: {row}");
    assert!(!row.contains("< hello"), "{row}");
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
