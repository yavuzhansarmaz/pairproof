use std::collections::BTreeMap;

use pairproof::core::{Language, QuestionRef, Session};
use pairproof::leetcode::Difficulty;
use pairproof::orchestration::control::{SnapshotFinal, SnapshotFinalQuestion};
use pairproof::scoring::{GradeOutcome, QuestionGrade};
use pairproof::tui::candidate_final::{
    candidate_final, candidate_final_lines, candidate_final_lines_for_height,
    candidate_final_pinned_lines, CandidateFinal,
};
use pairproof::tui::candidate_runtime::CandidateView;
use pairproof::tui::monitor::FinalView;
use pairproof::tui::viewport::{
    is_pinned_summary, split_pinned, truncation_marker, visible_lines, visible_with_pinned,
};

fn q(slug: &str, difficulty: Difficulty) -> QuestionRef {
    QuestionRef {
        title_slug: slug.into(),
        title: format!("Title {slug}"),
        difficulty,
        acceptance_pct: None,
    }
}

fn qs(prefix: &str, n: usize, difficulty: Difficulty) -> Vec<QuestionRef> {
    (0..n)
        .map(|i| q(&format!("{prefix}{i}"), difficulty))
        .collect()
}

fn begun() -> Session {
    let mut s = Session::new(
        "s1".into(),
        Language::Python,
        qs("e", 5, Difficulty::Easy),
        0,
    )
    .expect("session");
    s.mark_waiting_for_candidate(2).expect("waiting");
    s.candidate_connected(3).expect("connected");
    s.start(4).expect("start");
    s
}

fn completed() -> Session {
    let mut session = begun();
    session
        .set_expert_questions(
            qs("m", 3, Difficulty::Medium),
            qs("h", 2, Difficulty::Hard),
            5,
        )
        .expect("expert questions");
    for i in 0..10u64 {
        session.finish_question(10 + i).expect("finish");
    }
    session.mark_completed(100).expect("completed");
    session
}

fn graded(slot: usize, grade: f64) -> GradeOutcome {
    GradeOutcome::Graded(
        QuestionGrade::new(slot, grade, format!("rationale {slot}"), 0.2).expect("grade"),
    )
}

#[test]
fn final_rating_remains_visible_on_short_terminal() {
    let session = completed();
    let result =
        candidate_final(&session, &BTreeMap::new(), &[graded(0, 9.0)]).expect("candidate final");
    let CandidateFinal::Breakdown(view) = &result else {
        panic!("expected breakdown");
    };
    let rating_line = format!("Final rating: {:.2}", view.rating);
    for height in [12_usize, 8, 5] {
        let visible = candidate_final_lines_for_height(&result, height);
        assert!(visible.len() <= height, "{visible:?}");
        assert!(visible.contains(&rating_line), "h={height}: {visible:?}");
        assert!(
            visible.iter().any(|l| l.contains("more lines")),
            "h={height}: {visible:?}"
        );
    }
    let full = candidate_final_lines(&result);
    assert!(full.len() > 12, "{full:?}");
    let visible = visible_with_pinned(&full, 8);
    assert!(visible.contains(&rating_line), "{visible:?}");
}

#[test]
fn long_statement_does_not_hide_final_summary() {
    let statement = (0..60)
        .map(|i| format!("Statement line {i} with enough detail to fill rows"))
        .collect::<Vec<_>>()
        .join("\n");
    let view = CandidateView {
        phase: "Completed".to_string(),
        sync_phase: String::new(),
        question_title: Some("Big problem".to_string()),
        question_index: Some(0),
        statement: Some(statement),
        options: Vec::new(),
        selected: None,
        latest_test: None,
        chat_replies: Vec::new(),
        final_breakdown: Some(SnapshotFinal {
            rating: 8.25,
            per_question: vec![SnapshotFinalQuestion {
                slot_index: 0,
                grade: Some(8.0),
                rationale: Some("Solid".to_string()),
            }],
        }),
        recovery: None,
    };
    let lines = view.lines();
    let visible = visible_with_pinned(&lines, 12);
    assert!(visible.len() <= 12, "{visible:?}");
    assert!(
        visible.iter().any(|l| l.contains("Final rating: 8.25")),
        "{visible:?}"
    );
    assert!(
        visible.iter().any(|l| l.contains("more lines")),
        "{visible:?}"
    );
    let content: Vec<String> = (0..60).map(|i| format!("line {i}")).collect();
    let pinned = vec!["Final rating: 8.25".to_string()];
    let visible = visible_lines(&content, 6, &pinned);
    assert_eq!(visible.len(), 6);
    assert_eq!(
        visible.last().map(String::as_str),
        Some("Final rating: 8.25")
    );
}

#[test]
fn terminal_resize_preserves_usable_layout() {
    let session = completed();
    let result =
        candidate_final(&session, &BTreeMap::new(), &[graded(0, 9.0)]).expect("candidate final");
    let full = candidate_final_lines(&result);
    let rating_line = full
        .iter()
        .find(|l| l.contains("Final rating:"))
        .expect("rating line")
        .clone();
    for height in [50_usize, 24, 12, 6, 3, 2, 1, 0] {
        let visible = visible_with_pinned(&full, height);
        if height == 0 {
            assert!(visible.is_empty());
            continue;
        }
        assert!(visible.len() <= height, "h={height}: {visible:?}");
        assert!(visible.contains(&rating_line), "h={height}: {visible:?}");
    }
    let tall = visible_with_pinned(&full, 50);
    assert_eq!(tall, full);
    assert!(!tall.iter().any(|l| l.contains("more lines")), "{tall:?}");
    let shrunk = visible_with_pinned(&full, 6);
    assert!(
        shrunk.iter().any(|l| l.contains("more lines")),
        "{shrunk:?}"
    );
    let regrown = visible_with_pinned(&full, 50);
    assert_eq!(regrown, full);
    assert!(visible_lines(&[], 0, &[]).is_empty());
    assert_eq!(visible_lines(&[String::new()], 1, &[]), vec![String::new()]);
    let edge = visible_lines(&["x".repeat(500)], 2, &["Final rating: 1.00".to_string()]);
    assert_eq!(edge.len(), 2);
    assert_eq!(edge[1], "Final rating: 1.00");
}

#[test]
fn viewport_markers_and_pinned_split_are_exact() {
    assert_eq!(
        truncation_marker(3),
        "... 3 more lines \u{2014} scroll/resize for full text"
    );
    assert!(is_pinned_summary("  Final rating: 8.25"));
    assert!(is_pinned_summary(" Live rating: 8.25"));
    assert!(is_pinned_summary("   Solved: 3"));
    assert!(!is_pinned_summary(
        "Given an array of integers, return the indices."
    ));
    assert!(!is_pinned_summary("  The final rating is pending."));
    let lines = vec![
        "Statement A".to_string(),
        "Final rating: 8.25".to_string(),
        "Statement B".to_string(),
        "  Solved: 3".to_string(),
        "Live rating: 8.00".to_string(),
    ];
    let (content, pinned) = split_pinned(&lines);
    assert_eq!(
        content,
        vec!["Statement A".to_string(), "Statement B".to_string()]
    );
    assert_eq!(
        pinned,
        vec![
            "Final rating: 8.25".to_string(),
            "  Solved: 3".to_string(),
            "Live rating: 8.00".to_string(),
        ]
    );
}

#[test]
fn candidate_final_pinned_lines_are_the_completion_summary() {
    let view = FinalView {
        rating: 8.25,
        lines: vec!["detail".to_string()],
        per_question: vec!["Q1".to_string()],
        solved_count: 3,
        finished_count: 7,
        skipped_count: 2,
        paste_flag_count: 1,
        total_time_ms: 0,
        not_scored: Vec::new(),
        artefact_note: "note".to_string(),
    };
    assert_eq!(
        candidate_final_pinned_lines(&CandidateFinal::Breakdown(view)),
        vec![
            "Final rating: 8.25".to_string(),
            "Solved: 3".to_string(),
            "Finished: 7".to_string(),
            "Skipped: 2".to_string(),
            "Paste flags: 1".to_string(),
        ]
    );
    assert_eq!(
        candidate_final_pinned_lines(&CandidateFinal::TechnicalFailure),
        Vec::<String>::new()
    );
}
