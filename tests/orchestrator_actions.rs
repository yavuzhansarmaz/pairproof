use pairproof::core::{ApproachChoice, CoreError, EventKind, Language, QuestionRef, TestSummary};
use pairproof::leetcode::Difficulty;
use pairproof::orchestration::{Orchestrator, OrchestratorError};

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

fn easy() -> Vec<QuestionRef> {
    qs("e", 5, Difficulty::Easy)
}

fn medium() -> Vec<QuestionRef> {
    qs("m", 3, Difficulty::Medium)
}

fn hard() -> Vec<QuestionRef> {
    qs("h", 2, Difficulty::Hard)
}

fn created() -> Orchestrator {
    Orchestrator::create("s1".into(), Language::Python, easy(), 0).expect("create")
}

fn started() -> Orchestrator {
    let mut o = created();
    o.mark_waiting_for_candidate(1).expect("waiting");
    o.attach_candidate(2).expect("attach");
    o.start(3).expect("start");
    o
}

fn with_expert_started() -> Orchestrator {
    let mut o = created();
    o.set_expert_questions(medium(), hard(), 1).expect("expert");
    o.mark_waiting_for_candidate(2).expect("waiting");
    o.attach_candidate(3).expect("attach");
    o.start(4).expect("start");
    o
}

fn at_expert() -> Orchestrator {
    let mut o = with_expert_started();
    for i in 0..5u64 {
        o.finish_question(10 + i).expect("finish easy");
    }
    o
}

fn count(o: &Orchestrator, pred: impl Fn(&EventKind) -> bool) -> usize {
    o.session()
        .events()
        .iter()
        .filter(|e| pred(&e.kind))
        .count()
}

#[test]
fn orchestrator_maps_candidate_actions_to_core_once() {
    let mut o = started();
    let base = o.session().events().len();
    o.select_approach(ApproachChoice::Preset(1), 100, 5)
        .expect("select");
    // Exactly two events: ApproachSelected + TokensLocked.
    assert_eq!(o.session().events().len(), base + 2);
    assert_eq!(
        count(&o, |k| matches!(k, EventKind::ApproachSelected { .. })),
        1
    );
    assert_eq!(
        count(&o, |k| matches!(k, EventKind::TokensLocked { .. })),
        1
    );
    assert_eq!(o.session().token_locks(0), vec![100]);
    o.continue_work(6).expect("continue");
    assert_eq!(count(&o, |k| matches!(k, EventKind::Continued { .. })), 1);
    o.redirect("  use a heap  ", 7).expect("redirect");
    assert!(o.session().events().iter().any(|e| e.kind
        == EventKind::Redirected {
            index: 0,
            instruction: "use a heap".into()
        }));
    o.request_tests(8).expect("tests");
    assert_eq!(
        count(&o, |k| matches!(k, EventKind::TestsRequested { .. })),
        1
    );
    o.record_test_result(3, 1, 9).expect("record");
    assert_eq!(
        o.session().current_slot().expect("slot").latest_test,
        Some(TestSummary {
            passed: 3,
            failed: 1,
            at_ms: 9
        })
    );
    assert_eq!(
        count(&o, |k| matches!(k, EventKind::TestRunCompleted { .. })),
        1
    );
    o.change_approach(10).expect("change");
    assert_eq!(
        count(&o, |k| matches!(k, EventKind::ApproachChanged { .. })),
        1
    );
    assert_eq!(o.session().current_slot().expect("slot").approach, None);
    o.select_approach(ApproachChoice::Preset(2), 200, 11)
        .expect("reselect");
    assert_eq!(o.session().token_locks(0), vec![100, 200]);
    // Finish and skip each advance exactly one question.
    o.finish_question(12).expect("finish");
    assert_eq!(o.current_index(), Some(1));
    assert_eq!(
        count(&o, |k| matches!(k, EventKind::QuestionFinished { .. })),
        1
    );
    o.skip_question(13).expect("skip");
    assert_eq!(o.current_index(), Some(2));
    assert_eq!(
        count(&o, |k| matches!(k, EventKind::QuestionSkipped { .. })),
        1
    );
    // Re-question flow on an expert question delegates exactly once.
    let mut expert = at_expert();
    assert_eq!(expert.current_index(), Some(5));
    expert.request_requestion(20).expect("request");
    assert_eq!(
        count(&expert, |k| matches!(
            k,
            EventKind::ReQuestionRequested { .. }
        )),
        1
    );
    assert!(
        expert
            .session()
            .current_slot()
            .expect("slot")
            .re_question_pending
    );
    expert
        .resolve_requestion(true, Some(q("r0", Difficulty::Medium)), 21)
        .expect("resolve");
    assert_eq!(
        expert
            .session()
            .current_slot()
            .expect("slot")
            .question
            .title_slug,
        "r0"
    );
    assert_eq!(
        count(&expert, |k| matches!(
            k,
            EventKind::ReQuestionResolved { .. }
        )),
        1
    );
    expert.finish_question(22).expect("finish after resolve");
    assert_eq!(expert.current_index(), Some(6));
}

#[test]
fn orchestrator_rejects_illegal_action_with_typed_error() {
    // Actions before start have no active question.
    let mut o = created();
    assert_eq!(
        o.continue_work(1).expect_err("no active"),
        OrchestratorError::Core(CoreError::NoActiveQuestion)
    );
    let mut s = started();
    // Empty redirect and invalid approach surface core errors unchanged.
    assert_eq!(
        s.redirect("   ", 5).expect_err("empty redirect"),
        OrchestratorError::Core(CoreError::EmptyInstruction)
    );
    assert!(matches!(
        s.select_approach(ApproachChoice::Preset(4), 100, 5),
        Err(OrchestratorError::Core(CoreError::InvalidApproach(_)))
    ));
    assert_eq!(
        s.select_approach(ApproachChoice::Preset(0), 0, 5)
            .expect_err("zero tokens"),
        OrchestratorError::Core(CoreError::InvalidExpectedTokens)
    );
    s.select_approach(ApproachChoice::Preset(0), 100, 5)
        .expect("select");
    assert_eq!(
        s.select_approach(ApproachChoice::Preset(1), 50, 6)
            .expect_err("dup approach"),
        OrchestratorError::Core(CoreError::ApproachAlreadySelected)
    );
    // Change without an approach and re-question on an easy question fail.
    let mut fresh = started();
    assert_eq!(
        fresh.change_approach(5).expect_err("no approach"),
        OrchestratorError::Core(CoreError::NoApproachSelected)
    );
    assert_eq!(
        fresh.request_requestion(5).expect_err("easy requestion"),
        OrchestratorError::Core(CoreError::ReQuestionNotAllowed)
    );
    assert_eq!(
        fresh
            .resolve_requestion(false, None, 5)
            .expect_err("not pending"),
        OrchestratorError::Core(CoreError::ReQuestionNotPending)
    );
    // Disconnected candidate actions surface CandidateDisconnected.
    fresh.disconnect_candidate(10).expect("disconnect");
    assert_eq!(
        fresh.continue_work(11).expect_err("disconnected"),
        OrchestratorError::Core(CoreError::CandidateDisconnected)
    );
    // Terminal actions surface Terminal.
    let mut ended = started();
    ended.end_by_expert(9).expect("end");
    assert_eq!(
        ended
            .select_approach(ApproachChoice::Preset(0), 10, 10)
            .expect_err("terminal select"),
        OrchestratorError::Core(CoreError::Terminal)
    );
    assert_eq!(
        ended.add_tokens(1).expect_err("terminal tokens"),
        OrchestratorError::Core(CoreError::Terminal)
    );
}

#[test]
fn orchestrator_does_not_duplicate_token_usage_or_events() {
    let mut o = started();
    let base = o.session().events().len();
    o.select_approach(ApproachChoice::Preset(0), 100, 5)
        .expect("select");
    assert_eq!(o.session().events().len(), base + 2);
    assert_eq!(o.session().current_slot().expect("slot").actual_tokens, 0);
    // Each add_tokens call changes actual_tokens exactly once, no event.
    o.add_tokens(30).expect("tokens 30");
    assert_eq!(o.session().current_slot().expect("slot").actual_tokens, 30);
    assert_eq!(o.session().events().len(), base + 2);
    o.add_tokens(12).expect("tokens 12");
    assert_eq!(o.session().current_slot().expect("slot").actual_tokens, 42);
    assert_eq!(o.session().events().len(), base + 2);
    // A rejected duplicate selection adds neither events nor token locks.
    let before_events = o.session().events().len();
    let before_actual = o.session().current_slot().expect("slot").actual_tokens;
    assert!(o
        .select_approach(ApproachChoice::Preset(1), 999, 6)
        .is_err());
    assert_eq!(o.session().events().len(), before_events);
    assert_eq!(
        o.session().current_slot().expect("slot").actual_tokens,
        before_actual
    );
    assert_eq!(o.session().token_locks(0), vec![100]);
    // Change + reselect appends exactly one more lock, preserving history.
    o.change_approach(7).expect("change");
    assert_eq!(o.session().events().len(), before_events + 1);
    assert_eq!(o.session().current_slot().expect("slot").actual_tokens, 42);
    o.select_approach(ApproachChoice::Preset(3), 250, 8)
        .expect("reselect");
    assert_eq!(o.session().events().len(), before_events + 3);
    assert_eq!(o.session().token_locks(0), vec![100, 250]);
    assert_eq!(
        o.session()
            .current_slot()
            .expect("slot")
            .locked_expected_tokens,
        Some(250)
    );
    // Event sequence numbers stay strictly increasing from zero.
    let seqs: Vec<u64> = o.session().events().iter().map(|e| e.seq).collect();
    let expected: Vec<u64> = (0..seqs.len() as u64).collect();
    assert_eq!(seqs, expected);
    // record_test_result emits exactly one event per call.
    let n = o.session().events().len();
    o.record_test_result(2, 0, 20).expect("record");
    assert_eq!(o.session().events().len(), n + 1);
}
