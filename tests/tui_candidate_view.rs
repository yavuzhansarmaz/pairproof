//! Candidate snapshot view: HTML statements become plain text, the
//! waiting state is detectable, and panes split question from chat.

use pairproof::orchestration::control::{CandidateSnapshot, SnapshotOption, SnapshotQuestion};
use pairproof::tui::candidate_runtime::CandidateView;
use pairproof::tui::shared::html_to_text;

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
        selected: None,
        latest_test: None,
        chat_replies: vec!["Try sorting first.".to_string()],
        final_breakdown: None,
        recovery: None,
    }
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
    assert!(!left.contains("coach:"), "{left}");
    let right = view.chat_lines().join("\n");
    assert_eq!(right, "coach: Try sorting first.");
}

#[test]
fn html_to_text_decodes_entities_and_bullets() {
    let text = html_to_text("<p>a &amp; b</p><ul><li>one</li><li>two &#65;</li></ul>");
    assert!(text.contains("a & b"), "{text}");
    assert!(text.contains("- one"), "{text}");
    assert!(text.contains("- two A"), "{text}");
    assert!(!text.contains('<'));
}
