use pairproof::core::{CoreError, Language, QuestionRef, Session};
use pairproof::leetcode::Difficulty;
use pairproof::tui::selection::{difficulty_filter, SelectionController, SelectionError};

fn q(slug: &str, difficulty: Difficulty) -> QuestionRef {
    QuestionRef {
        title_slug: slug.to_string(),
        title: format!("Title {slug}"),
        difficulty,
        acceptance_pct: Some(50),
    }
}

fn qs(prefix: &str, n: usize, difficulty: Difficulty) -> Vec<QuestionRef> {
    (0..n)
        .map(|i| q(&format!("{prefix}{i}"), difficulty))
        .collect()
}

fn easy_five() -> Vec<QuestionRef> {
    qs("e", 5, Difficulty::Easy)
}

fn new_controller() -> SelectionController {
    let session =
        Session::new("s1".to_string(), Language::Python, easy_five(), 0).expect("session");
    SelectionController::new(session, vec!["array".to_string(), "hash-table".to_string()])
}

fn controller_with_expert() -> SelectionController {
    let mut controller = new_controller();
    controller
        .select_medium(qs("m", 3, Difficulty::Medium), 1)
        .expect("medium");
    controller
        .select_hard(qs("h", 2, Difficulty::Hard), 2)
        .expect("hard");
    controller
}

fn at_expert_slot() -> SelectionController {
    let mut controller = controller_with_expert();
    controller
        .session_mut()
        .mark_waiting_for_candidate(10)
        .expect("waiting");
    controller
        .session_mut()
        .candidate_connected(11)
        .expect("connected");
    controller.session_mut().start(12).expect("start");
    for i in 0..5u64 {
        controller
            .session_mut()
            .finish_question(20 + i)
            .expect("finish easy");
    }
    assert_eq!(controller.session().current_index(), Some(5));
    controller
}

#[test]
fn expert_selection_is_three_medium_then_two_hard() {
    let mut controller = new_controller();
    assert_eq!(controller.session().slots().len(), 5);
    controller
        .select_medium(qs("m", 3, Difficulty::Medium), 100)
        .expect("medium");
    assert_eq!(controller.session().slots().len(), 5);
    assert!(controller.has_pending_medium());
    assert_eq!(controller.pending_medium().map(|p| p.len()), Some(3));
    controller
        .select_hard(qs("h", 2, Difficulty::Hard), 101)
        .expect("hard");
    assert_eq!(controller.session().slots().len(), 10);
    assert!(!controller.has_pending_medium());
    let slots = controller.session().slots();
    for (i, slot) in slots.iter().enumerate().take(5) {
        assert_eq!(slot.index, i);
        assert_eq!(slot.question.difficulty, Difficulty::Easy);
        assert_eq!(slot.question.title_slug, format!("e{i}"));
    }
    for (i, slot) in slots.iter().enumerate().skip(5).take(3) {
        assert_eq!(slot.question.difficulty, Difficulty::Medium);
        assert_eq!(slot.question.title_slug, format!("m{}", i - 5));
    }
    for (i, slot) in slots.iter().enumerate().skip(8).take(2) {
        assert_eq!(slot.question.difficulty, Difficulty::Hard);
        assert_eq!(slot.question.title_slug, format!("h{}", i - 8));
    }
    let mut via_submit = new_controller();
    via_submit
        .submit(
            qs("m", 3, Difficulty::Medium),
            qs("h", 2, Difficulty::Hard),
            200,
        )
        .expect("submit");
    assert_eq!(via_submit.session().slots().len(), 10);
    assert_eq!(via_submit.session().slots()[5].question.title_slug, "m0");
    assert_eq!(via_submit.session().slots()[9].question.title_slug, "h1");
}

#[test]
fn medium_before_hard_ordering_is_enforced() {
    let mut controller = new_controller();
    let err = controller
        .select_hard(qs("h", 2, Difficulty::Hard), 100)
        .expect_err("hard before medium must fail");
    assert_eq!(err, SelectionError::MediumRequired);
    assert_eq!(controller.session().slots().len(), 5);
    // After medium, hard succeeds; a second hard attempt hits core ExpertAlreadySet.
    controller
        .select_medium(qs("m", 3, Difficulty::Medium), 101)
        .expect("medium");
    controller
        .select_hard(qs("h", 2, Difficulty::Hard), 102)
        .expect("hard");
    let mut again = controller_with_expert();
    // Re-selecting medium after commit then hard again surfaces the core guard.
    let _ = again.select_medium(qs("m2", 3, Difficulty::Medium), 103);
    let err = again
        .select_hard(qs("h2", 2, Difficulty::Hard), 104)
        .expect_err("second expert set must fail");
    assert_eq!(err, SelectionError::Core(CoreError::ExpertAlreadySet));
}

#[test]
fn selection_rejects_wrong_count_and_wrong_difficulty() {
    let mut controller = new_controller();
    assert_eq!(
        controller.select_medium(qs("m", 2, Difficulty::Medium), 1),
        Err(SelectionError::WrongCount {
            expected: 3,
            got: 2
        })
    );
    assert_eq!(
        controller.select_medium(qs("m", 4, Difficulty::Medium), 1),
        Err(SelectionError::WrongCount {
            expected: 3,
            got: 4
        })
    );
    let mut wrong_medium = qs("m", 3, Difficulty::Medium);
    wrong_medium[0] = q("m0", Difficulty::Easy);
    assert_eq!(
        controller.select_medium(wrong_medium, 1),
        Err(SelectionError::WrongDifficulty {
            slug: "m0".to_string()
        })
    );
    assert!(!controller.has_pending_medium());
    controller
        .select_medium(qs("m", 3, Difficulty::Medium), 1)
        .expect("medium");
    assert_eq!(
        controller.select_hard(qs("h", 1, Difficulty::Hard), 2),
        Err(SelectionError::WrongCount {
            expected: 2,
            got: 1
        })
    );
    let mut wrong_hard = qs("h", 2, Difficulty::Hard);
    wrong_hard[1] = q("h1", Difficulty::Medium);
    assert_eq!(
        controller.select_hard(wrong_hard, 2),
        Err(SelectionError::WrongDifficulty {
            slug: "h1".to_string()
        })
    );
    // Failed hard does not consume the pending medium.
    assert!(controller.has_pending_medium());
    assert_eq!(controller.session().slots().len(), 5);
}

#[test]
fn duplicate_slug_rejection_covers_easy_and_within_selection() {
    let mut controller = new_controller();
    // Duplicate against an existing easy slug.
    let mut clash_easy = qs("m", 3, Difficulty::Medium);
    clash_easy[0] = q("e0", Difficulty::Medium);
    assert_eq!(
        controller.select_medium(clash_easy, 1),
        Err(SelectionError::DuplicateSlug("e0".to_string()))
    );
    // Duplicate within the medium list itself.
    let mut dup_within = qs("m", 3, Difficulty::Medium);
    dup_within[1] = q("m0", Difficulty::Medium);
    assert_eq!(
        controller.select_medium(dup_within, 1),
        Err(SelectionError::DuplicateSlug("m0".to_string()))
    );
    assert!(!controller.has_pending_medium());
    // Duplicate hard against pending medium.
    controller
        .select_medium(qs("m", 3, Difficulty::Medium), 1)
        .expect("medium");
    let mut clash_medium = qs("h", 2, Difficulty::Hard);
    clash_medium[0] = q("m1", Difficulty::Hard);
    assert_eq!(
        controller.select_hard(clash_medium, 2),
        Err(SelectionError::DuplicateSlug("m1".to_string()))
    );
    // Duplicate within hard list.
    let mut dup_hard = qs("h", 2, Difficulty::Hard);
    dup_hard[1] = q("h0", Difficulty::Hard);
    assert_eq!(
        controller.select_hard(dup_hard, 2),
        Err(SelectionError::DuplicateSlug("h0".to_string()))
    );
    // Valid hard still succeeds after rejected attempts.
    controller
        .select_hard(qs("h", 2, Difficulty::Hard), 3)
        .expect("hard");
    assert_eq!(controller.session().slots().len(), 10);
}

#[test]
fn approved_requestion_keeps_difficulty() {
    let mut controller = at_expert_slot();
    assert_eq!(
        controller
            .session()
            .current_slot()
            .expect("slot")
            .question
            .title_slug,
        "m0"
    );
    controller
        .session_mut()
        .request_requestion(30)
        .expect("request");
    assert_eq!(controller.pending_requestion(), Some(5));
    // Wrong-difficulty replacement is rejected by core and pending stays.
    let err = controller
        .approve_requestion(q("r0", Difficulty::Hard), 31)
        .expect_err("hard replacement for medium slot must fail");
    assert_eq!(
        err,
        SelectionError::Core(CoreError::ReplacementDifficultyMismatch)
    );
    assert_eq!(controller.pending_requestion(), Some(5));
    assert_eq!(
        controller
            .session()
            .current_slot()
            .expect("slot")
            .question
            .title_slug,
        "m0"
    );
    // Same-difficulty replacement succeeds and keeps Medium.
    controller
        .approve_requestion(q("r0", Difficulty::Medium), 32)
        .expect("approve");
    assert_eq!(controller.pending_requestion(), None);
    let slot = controller.session().current_slot().expect("slot");
    assert_eq!(slot.question.title_slug, "r0");
    assert_eq!(slot.question.difficulty, Difficulty::Medium);
    assert!(slot.re_question_used);
    assert!(!slot.re_question_pending);
}

#[test]
fn approved_requestion_rejects_duplicate_slug() {
    let mut controller = at_expert_slot();
    controller
        .session_mut()
        .request_requestion(30)
        .expect("request");
    // Replacement slug already used by another slot (h0) with matching difficulty check first:
    // use a medium slug that duplicates an existing medium slot m1.
    let err = controller
        .approve_requestion(q("m1", Difficulty::Medium), 31)
        .expect_err("duplicate replacement must fail");
    assert_eq!(
        err,
        SelectionError::Core(CoreError::DuplicateSlug("m1".to_string()))
    );
    assert_eq!(controller.pending_requestion(), Some(5));
}

#[test]
fn rejected_requestion_preserves_current_question() {
    assert_eq!(at_expert_slot().pending_requestion(), None);
    assert_eq!(new_controller().pending_requestion(), None);
    let mut controller = at_expert_slot();
    let before = controller
        .session()
        .current_slot()
        .expect("slot")
        .question
        .clone();
    controller
        .session_mut()
        .request_requestion(30)
        .expect("request");
    assert_eq!(controller.pending_requestion(), Some(5));
    controller.reject_requestion(31).expect("reject");
    assert_eq!(controller.pending_requestion(), None);
    let slot = controller.session().current_slot().expect("slot");
    assert_eq!(slot.question, before);
    assert_eq!(slot.question.title_slug, "m0");
    assert!(slot.re_question_used);
    assert!(!slot.re_question_pending);
    // Rejection burns the right: a second request fails in core.
    assert!(controller.session_mut().request_requestion(32).is_err());
}

#[test]
fn extra_question_cannot_be_added_twice() {
    let mut controller = controller_with_expert();
    assert_eq!(
        controller.add_extra(q("e0", Difficulty::Easy), 49),
        Err(SelectionError::Core(CoreError::DuplicateSlug(
            "e0".to_string()
        )))
    );
    assert_eq!(controller.session().slots().len(), 10);
    controller
        .add_extra(q("x0", Difficulty::Hard), 50)
        .expect("first extra");
    assert_eq!(controller.session().slots().len(), 11);
    assert_eq!(controller.session().slots()[10].question.title_slug, "x0");
    let err = controller
        .add_extra(q("x1", Difficulty::Easy), 51)
        .expect_err("second extra must fail");
    assert_eq!(err, SelectionError::Core(CoreError::ExtraAlreadyAdded));
    assert_eq!(controller.session().slots().len(), 11);
}

#[test]
fn extra_before_expert_set_is_rejected() {
    let mut controller = new_controller();
    let err = controller
        .add_extra(q("x0", Difficulty::Medium), 10)
        .expect_err("extra before expert set must fail");
    assert_eq!(err, SelectionError::Core(CoreError::ExpertNotSet));
    assert_eq!(controller.session().slots().len(), 5);
    // Also rejected when only medium is pending (core not yet committed).
    controller
        .select_medium(qs("m", 3, Difficulty::Medium), 11)
        .expect("medium");
    let err = controller
        .add_extra(q("x0", Difficulty::Medium), 12)
        .expect_err("extra with only pending medium must fail");
    assert_eq!(err, SelectionError::Core(CoreError::ExpertNotSet));
}

#[test]
fn difficulty_filter_and_pool_helpers_keep_core_authoritative() {
    let pool = vec![
        q("a", Difficulty::Easy),
        q("b", Difficulty::Medium),
        q("c", Difficulty::Medium),
        q("d", Difficulty::Hard),
    ];
    assert_eq!(
        difficulty_filter(&pool, Difficulty::Medium)
            .iter()
            .map(|item| item.title_slug.as_str())
            .collect::<Vec<_>>(),
        vec!["b", "c"]
    );
    assert_eq!(difficulty_filter(&pool, Difficulty::Hard).len(), 1);
    assert!(difficulty_filter(&[], Difficulty::Easy).is_empty());
    // Filtering alone does not bypass session validation: wrong counts still fail.
    let mut controller = new_controller();
    let mediums = difficulty_filter(&pool, Difficulty::Medium);
    assert_eq!(
        controller.select_medium(mediums, 1),
        Err(SelectionError::WrongCount {
            expected: 3,
            got: 2
        })
    );
}
