use pairproof::core::{
    ApproachChoice, CoreError, EventKind, Language, QuestionRef, Session, SessionEvent, TestSummary,
};
use pairproof::leetcode::Difficulty;

fn q(slug: &str, difficulty: Difficulty) -> QuestionRef {
    QuestionRef {
        title_slug: slug.to_string(),
        title: slug.to_string(),
        difficulty,
        acceptance_pct: None,
    }
}

fn qs(prefix: &str, n: usize, d: Difficulty) -> Vec<QuestionRef> {
    (0..n).map(|i| q(&format!("{prefix}{i}"), d)).collect()
}

fn new_session() -> Session {
    Session::new(
        "s1".into(),
        Language::Python,
        qs("e", 5, Difficulty::Easy),
        0,
    )
    .expect("session")
}

fn started() -> Session {
    let mut s = new_session();
    s.mark_waiting_for_candidate(1).expect("waiting");
    s.candidate_connected(2).expect("connected");
    s.start(3).expect("start");
    s
}

fn with_expert() -> Session {
    let mut s = new_session();
    s.set_expert_questions(
        qs("m", 3, Difficulty::Medium),
        qs("h", 2, Difficulty::Hard),
        1,
    )
    .expect("expert set");
    s.mark_waiting_for_candidate(2).expect("waiting");
    s.candidate_connected(3).expect("connected");
    s.start(4).expect("start");
    s
}

fn at_expert() -> Session {
    let mut s = with_expert();
    for i in 0..5u64 {
        s.finish_question(10 + i).expect("finish easy");
    }
    s
}

fn kind_count(s: &Session, pred: impl Fn(&EventKind) -> bool) -> usize {
    s.events().iter().filter(|e| pred(&e.kind)).count()
}

#[test]
fn disconnect_blocks_candidate_actions_but_not_expert_resolution() {
    let mut s = started();
    s.select_approach(ApproachChoice::Preset(0), 100, 5)
        .expect("select");
    s.candidate_disconnected(10).expect("disconnect");
    assert!(!s.is_connected());
    let err = CoreError::CandidateDisconnected;
    assert_eq!(
        s.select_approach(ApproachChoice::Preset(1), 50, 11),
        Err(err.clone())
    );
    assert_eq!(s.continue_work(11), Err(err.clone()));
    assert_eq!(s.redirect("use a heap", 11), Err(err.clone()));
    assert_eq!(s.request_tests(11), Err(err.clone()));
    assert_eq!(s.change_approach(11), Err(err.clone()));
    assert_eq!(s.add_tokens(1), Err(err.clone()));
    assert_eq!(s.request_requestion(11), Err(err.clone()));
    assert_eq!(s.finish_question(11), Err(err.clone()));
    assert_eq!(s.skip_question(11), Err(err));
    s.record_test_result(1, 0, 12)
        .expect("system recording allowed");
    s.end_by_expert(13)
        .expect("expert end allowed while disconnected");

    let mut s = at_expert();
    s.request_requestion(20).expect("request");
    s.candidate_disconnected(21).expect("disconnect");
    assert_eq!(s.finish_question(22), Err(CoreError::ReQuestionPending));
    assert_eq!(
        s.request_requestion(22),
        Err(CoreError::ReQuestionAlreadyUsed)
    );
    s.resolve_requestion(true, Some(q("r0", Difficulty::Medium)), 23)
        .expect("expert resolution allowed while disconnected");
    assert_eq!(s.current_slot().expect("slot").question.title_slug, "r0");
    s.set_expert_questions(
        qs("n", 3, Difficulty::Medium),
        qs("k", 2, Difficulty::Hard),
        24,
    )
    .expect_err("already set");
}

#[test]
fn reconnect_resumes_same_question_and_reports_downtime() {
    let mut s = started();
    s.finish_question(10).expect("finish");
    assert_eq!(s.current_index(), Some(1));
    s.candidate_disconnected(100).expect("disconnect");
    assert_eq!(s.current_index(), Some(1));
    s.candidate_connected(450).expect("reconnect");
    assert!(s.is_connected());
    assert_eq!(s.current_index(), Some(1));
    assert!(s
        .events()
        .iter()
        .any(|e| e.kind == EventKind::CandidateDisconnected));
    assert!(s
        .events()
        .iter()
        .any(|e| e.kind == EventKind::CandidateReconnected { downtime_ms: 350 }));
    assert_eq!(
        kind_count(&s, |k| matches!(k, EventKind::CandidateConnected)),
        1
    );
    s.continue_work(451).expect("actions work again");
    assert_eq!(
        s.candidate_connected(452),
        Err(CoreError::CandidateAlreadyConnected)
    );
}

#[test]
fn disconnect_before_start_then_reconnect_allows_start() {
    let mut s = new_session();
    s.mark_waiting_for_candidate(1).expect("waiting");
    s.candidate_connected(2).expect("connected");
    s.candidate_disconnected(10).expect("disconnect");
    assert_eq!(s.start(11), Err(CoreError::CandidateNotConnected));
    s.candidate_connected(60).expect("reconnect");
    assert!(s
        .events()
        .iter()
        .any(|e| e.kind == EventKind::CandidateReconnected { downtime_ms: 50 }));
    s.start(61).expect("start after reconnect");
    assert_eq!(s.current_index(), Some(0));
}

#[test]
fn disconnect_when_not_connected_and_after_terminal_is_rejected() {
    let mut s = new_session();
    assert_eq!(
        s.candidate_disconnected(1),
        Err(CoreError::CandidateNotConnected)
    );
    let mut s = started();
    s.candidate_disconnected(5).expect("disconnect");
    assert_eq!(
        s.candidate_disconnected(6),
        Err(CoreError::CandidateNotConnected)
    );
    let mut s = started();
    s.end_by_expert(5).expect("end");
    assert_eq!(s.candidate_disconnected(6), Err(CoreError::Terminal));
    assert_eq!(s.candidate_connected(6), Err(CoreError::Terminal));
    let mut s = started();
    s.candidate_disconnected(5).expect("disconnect");
    s.mark_technical_failure(6)
        .expect("failure while disconnected");
    assert_eq!(s.candidate_connected(7), Err(CoreError::Terminal));
}

#[test]
fn test_result_recorded_as_latest_and_event() {
    let mut s = started();
    s.record_test_result(3, 1, 10).expect("first run");
    assert_eq!(
        s.current_slot().expect("slot").latest_test,
        Some(TestSummary {
            passed: 3,
            failed: 1,
            at_ms: 10
        })
    );
    s.record_test_result(4, 0, 20).expect("second run");
    assert_eq!(
        s.current_slot().expect("slot").latest_test,
        Some(TestSummary {
            passed: 4,
            failed: 0,
            at_ms: 20
        })
    );
    assert_eq!(
        kind_count(&s, |k| matches!(k, EventKind::TestRunCompleted { .. })),
        2
    );
    assert!(s.events().iter().any(|e| e.kind
        == EventKind::TestRunCompleted {
            index: 0,
            passed: 3,
            failed: 1
        }));
    s.candidate_disconnected(30).expect("disconnect");
    s.record_test_result(5, 0, 31)
        .expect("recording allowed while disconnected");
    assert_eq!(
        s.current_slot().expect("slot").latest_test,
        Some(TestSummary {
            passed: 5,
            failed: 0,
            at_ms: 31
        })
    );
    let mut s = new_session();
    assert_eq!(
        s.record_test_result(1, 0, 1),
        Err(CoreError::NoActiveQuestion)
    );
}

#[test]
fn paste_flag_counts_and_zero_rejected() {
    let mut s = started();
    assert_eq!(s.record_paste_flag(0, 5), Err(CoreError::InvalidPasteFlag));
    assert_eq!(s.paste_flag_count(), 0);
    s.record_paste_flag(100, 6).expect("flag");
    s.record_paste_flag(200, 7).expect("flag");
    assert_eq!(s.slot(0).expect("slot").paste_flags, 2);
    assert!(s.events().iter().any(|e| e.kind
        == EventKind::PasteFlagged {
            index: 0,
            inserted_bytes: 100
        }));
    s.finish_question(8).expect("finish");
    s.record_paste_flag(50, 9).expect("flag on next");
    assert_eq!(s.slot(1).expect("slot").paste_flags, 1);
    assert_eq!(s.paste_flag_count(), 3);
    let mut s = new_session();
    assert!(s.record_paste_flag(10, 1).is_err());
}

#[test]
fn chat_boundary_violation_logged_with_and_without_active_question() {
    let mut s = new_session();
    s.record_chat_boundary_violation(1).expect("before start");
    assert!(s
        .events()
        .iter()
        .any(|e| e.kind == EventKind::ChatBoundaryViolated { index: None }));
    s.mark_waiting_for_candidate(2).expect("waiting");
    s.candidate_connected(3).expect("connected");
    s.start(4).expect("start");
    let before = s.current_slot().expect("slot").clone();
    s.record_chat_boundary_violation(5).expect("after start");
    assert!(s
        .events()
        .iter()
        .any(|e| e.kind == EventKind::ChatBoundaryViolated { index: Some(0) }));
    assert_eq!(s.chat_boundary_violations(), 2);
    assert_eq!(s.current_slot().expect("slot"), &before);
    s.end_by_expert(6).expect("end");
    assert_eq!(
        s.record_chat_boundary_violation(7),
        Err(CoreError::Terminal)
    );
    assert_eq!(s.chat_boundary_violations(), 2);
}

#[test]
fn token_locks_preserve_history_across_approach_change() {
    let mut s = started();
    s.select_approach(ApproachChoice::Preset(0), 100, 5)
        .expect("lock 100");
    s.change_approach(6).expect("change");
    s.select_approach(ApproachChoice::Preset(1), 200, 7)
        .expect("lock 200");
    assert_eq!(s.token_locks(0), vec![100, 200]);
    assert_eq!(s.token_locks(1), Vec::<u64>::new());
    assert_eq!(
        s.current_slot().expect("slot").locked_expected_tokens,
        Some(200)
    );
}

#[test]
fn new_event_kinds_round_trip_through_json() {
    let kinds = [
        EventKind::TestRunCompleted {
            index: 2,
            passed: 7,
            failed: 1,
        },
        EventKind::PasteFlagged {
            index: 2,
            inserted_bytes: 4096,
        },
        EventKind::ChatBoundaryViolated { index: Some(2) },
        EventKind::ChatBoundaryViolated { index: None },
        EventKind::CandidateDisconnected,
        EventKind::CandidateReconnected { downtime_ms: 350 },
    ];
    for kind in kinds {
        let event = SessionEvent {
            seq: 3,
            at_ms: 99,
            kind,
        };
        let json = serde_json::to_string(&event).expect("serializes");
        let back: SessionEvent = serde_json::from_str(&json).expect("parses");
        assert_eq!(back, event);
    }
}
