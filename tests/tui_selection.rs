//! Migrated from `SelectionController` (deleted): it delegated every
//! lifecycle/re-question/extra call to core `Session` and validated the
//! expert set with the same rules core `set_expert_questions` enforces
//! atomically (count/difficulty/duplicate). The two-screen pending-medium
//! staging now lives in production `ExpertRuntime` (`fetch_medium` caches
//! pending medium; `complete_selection` requires it — covered by
//! `tui_selection_pool`); here a failed atomic commit leaving slots
//! unchanged and retryable is the equivalent observable.

use pairproof::core::{CoreError, Language, QuestionRef, Session};
use pairproof::leetcode::selection::{difficulty_filter, pool_to_refs};
use pairproof::leetcode::{CatalogEntry, Difficulty};

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
fn new_session() -> Session {
    Session::new("s1".to_string(), Language::Python, easy_five(), 0).expect("session")
}
fn session_with_expert() -> Session {
    let mut s = new_session();
    s.set_expert_questions(
        qs("m", 3, Difficulty::Medium),
        qs("h", 2, Difficulty::Hard),
        2,
    )
    .expect("expert");
    s
}
fn at_expert_slot() -> Session {
    let mut s = session_with_expert();
    s.mark_waiting_for_candidate(10).expect("waiting");
    s.candidate_connected(11).expect("connected");
    s.start(12).expect("start");
    for i in 0..5u64 {
        s.finish_question(20 + i).expect("finish easy");
    }
    assert_eq!(s.current_index(), Some(5));
    s
}
fn pending_requestion(s: &Session) -> Option<usize> {
    s.current_slot()
        .filter(|slot| slot.re_question_pending)
        .map(|slot| slot.index)
}

#[test]
fn expert_selection_is_three_medium_then_two_hard() {
    let mut s = new_session();
    assert_eq!(s.slots().len(), 5);
    // No partial commit before the atomic expert set (was pending medium).
    s.set_expert_questions(
        qs("m", 3, Difficulty::Medium),
        qs("h", 2, Difficulty::Hard),
        101,
    )
    .expect("expert set");
    assert_eq!(s.slots().len(), 10);
    let slots = s.slots();
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
}

#[test]
fn medium_before_hard_ordering_is_enforced() {
    // Core API is atomic: hard cannot commit without 3 medium in same call.
    let mut s = new_session();
    let err = s
        .set_expert_questions(Vec::new(), qs("h", 2, Difficulty::Hard), 100)
        .expect_err("hard without medium must fail");
    assert_eq!(
        err,
        CoreError::InvalidQuestionCount {
            expected: 3,
            got: 0
        }
    );
    assert_eq!(s.slots().len(), 5);
    s.set_expert_questions(
        qs("m", 3, Difficulty::Medium),
        qs("h", 2, Difficulty::Hard),
        102,
    )
    .expect("expert");
    let err = s
        .set_expert_questions(
            qs("m2", 3, Difficulty::Medium),
            qs("h2", 2, Difficulty::Hard),
            104,
        )
        .expect_err("second expert set must fail");
    assert_eq!(err, CoreError::ExpertAlreadySet);
}

#[test]
fn selection_rejects_wrong_count_and_wrong_difficulty() {
    let mut s = new_session();
    assert_eq!(
        s.set_expert_questions(
            qs("m", 2, Difficulty::Medium),
            qs("h", 2, Difficulty::Hard),
            1
        ),
        Err(CoreError::InvalidQuestionCount {
            expected: 3,
            got: 2
        })
    );
    assert_eq!(
        s.set_expert_questions(
            qs("m", 4, Difficulty::Medium),
            qs("h", 2, Difficulty::Hard),
            1
        ),
        Err(CoreError::InvalidQuestionCount {
            expected: 3,
            got: 4
        })
    );
    let mut wrong_medium = qs("m", 3, Difficulty::Medium);
    wrong_medium[0] = q("m0", Difficulty::Easy);
    assert_eq!(
        s.set_expert_questions(wrong_medium, qs("h", 2, Difficulty::Hard), 1),
        Err(CoreError::WrongDifficulty {
            slug: "m0".to_string()
        })
    );
    assert_eq!(s.slots().len(), 5);
    assert_eq!(
        s.set_expert_questions(
            qs("m", 3, Difficulty::Medium),
            qs("h", 1, Difficulty::Hard),
            2
        ),
        Err(CoreError::InvalidQuestionCount {
            expected: 2,
            got: 1
        })
    );
    let mut wrong_hard = qs("h", 2, Difficulty::Hard);
    wrong_hard[1] = q("h1", Difficulty::Medium);
    assert_eq!(
        s.set_expert_questions(qs("m", 3, Difficulty::Medium), wrong_hard, 2),
        Err(CoreError::WrongDifficulty {
            slug: "h1".to_string()
        })
    );
    // Failed commit does not consume anything: valid retry succeeds.
    assert_eq!(s.slots().len(), 5);
    s.set_expert_questions(
        qs("m", 3, Difficulty::Medium),
        qs("h", 2, Difficulty::Hard),
        3,
    )
    .expect("valid after failures");
    assert_eq!(s.slots().len(), 10);
}

#[test]
fn duplicate_slug_rejection_covers_easy_and_within_selection() {
    let mut s = new_session();
    let mut clash_easy = qs("m", 3, Difficulty::Medium);
    clash_easy[0] = q("e0", Difficulty::Medium);
    assert_eq!(
        s.set_expert_questions(clash_easy, qs("h", 2, Difficulty::Hard), 1),
        Err(CoreError::DuplicateSlug("e0".to_string()))
    );
    let mut dup_within = qs("m", 3, Difficulty::Medium);
    dup_within[1] = q("m0", Difficulty::Medium);
    assert_eq!(
        s.set_expert_questions(dup_within, qs("h", 2, Difficulty::Hard), 1),
        Err(CoreError::DuplicateSlug("m0".to_string()))
    );
    let mut clash_medium = qs("h", 2, Difficulty::Hard);
    clash_medium[0] = q("m1", Difficulty::Hard);
    assert_eq!(
        s.set_expert_questions(qs("m", 3, Difficulty::Medium), clash_medium, 2),
        Err(CoreError::DuplicateSlug("m1".to_string()))
    );
    let mut dup_hard = qs("h", 2, Difficulty::Hard);
    dup_hard[1] = q("h0", Difficulty::Hard);
    assert_eq!(
        s.set_expert_questions(qs("m", 3, Difficulty::Medium), dup_hard, 2),
        Err(CoreError::DuplicateSlug("h0".to_string()))
    );
    s.set_expert_questions(
        qs("m", 3, Difficulty::Medium),
        qs("h", 2, Difficulty::Hard),
        3,
    )
    .expect("hard");
    assert_eq!(s.slots().len(), 10);
}

#[test]
fn approved_requestion_keeps_difficulty() {
    let mut s = at_expert_slot();
    assert_eq!(s.current_slot().expect("slot").question.title_slug, "m0");
    s.request_requestion(30).expect("request");
    assert_eq!(pending_requestion(&s), Some(5));
    let err = s
        .resolve_requestion(true, Some(q("r0", Difficulty::Hard)), 31)
        .expect_err("hard replacement for medium slot must fail");
    assert_eq!(err, CoreError::ReplacementDifficultyMismatch);
    assert_eq!(pending_requestion(&s), Some(5));
    assert_eq!(s.current_slot().expect("slot").question.title_slug, "m0");
    s.resolve_requestion(true, Some(q("r0", Difficulty::Medium)), 32)
        .expect("approve");
    assert_eq!(pending_requestion(&s), None);
    let slot = s.current_slot().expect("slot");
    assert_eq!(slot.question.title_slug, "r0");
    assert_eq!(slot.question.difficulty, Difficulty::Medium);
    assert!(slot.re_question_used);
    assert!(!slot.re_question_pending);
}

#[test]
fn approved_requestion_rejects_duplicate_slug() {
    let mut s = at_expert_slot();
    s.request_requestion(30).expect("request");
    let err = s
        .resolve_requestion(true, Some(q("m1", Difficulty::Medium)), 31)
        .expect_err("duplicate replacement must fail");
    assert_eq!(err, CoreError::DuplicateSlug("m1".to_string()));
    assert_eq!(pending_requestion(&s), Some(5));
}

#[test]
fn rejected_requestion_preserves_current_question() {
    assert_eq!(pending_requestion(&at_expert_slot()), None);
    assert_eq!(pending_requestion(&new_session()), None);
    let mut s = at_expert_slot();
    let before = s.current_slot().expect("slot").question.clone();
    s.request_requestion(30).expect("request");
    assert_eq!(pending_requestion(&s), Some(5));
    s.resolve_requestion(false, None, 31).expect("reject");
    assert_eq!(pending_requestion(&s), None);
    let slot = s.current_slot().expect("slot");
    assert_eq!(slot.question, before);
    assert_eq!(slot.question.title_slug, "m0");
    assert!(slot.re_question_used);
    assert!(!slot.re_question_pending);
    assert!(s.request_requestion(32).is_err());
}

#[test]
fn extra_question_cannot_be_added_twice() {
    let mut s = session_with_expert();
    assert_eq!(
        s.add_extra_question(q("e0", Difficulty::Easy), 49),
        Err(CoreError::DuplicateSlug("e0".to_string()))
    );
    assert_eq!(s.slots().len(), 10);
    s.add_extra_question(q("x0", Difficulty::Hard), 50)
        .expect("first extra");
    assert_eq!(s.slots().len(), 11);
    assert_eq!(s.slots()[10].question.title_slug, "x0");
    let err = s
        .add_extra_question(q("x1", Difficulty::Easy), 51)
        .expect_err("second extra must fail");
    assert_eq!(err, CoreError::ExtraAlreadyAdded);
    assert_eq!(s.slots().len(), 11);
}

#[test]
fn extra_before_expert_set_is_rejected() {
    let mut s = new_session();
    let err = s
        .add_extra_question(q("x0", Difficulty::Medium), 10)
        .expect_err("extra before expert set must fail");
    assert_eq!(err, CoreError::ExpertNotSet);
    assert_eq!(s.slots().len(), 5);
    // Still rejected after an uncommitted (failed) medium attempt: in core
    // terms the expert set is not committed until the atomic call succeeds
    // (was: only pending medium in the deleted controller).
    let _ = s.set_expert_questions(
        qs("m", 2, Difficulty::Medium),
        qs("h", 2, Difficulty::Hard),
        11,
    );
    let err = s
        .add_extra_question(q("x0", Difficulty::Medium), 12)
        .expect_err("extra with uncommitted medium must fail");
    assert_eq!(err, CoreError::ExpertNotSet);
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
    // pool_to_refs (relocated to leetcode::selection) drops paid/unknown.
    let entry = |slug: &str, diff: &str, paid: bool| CatalogEntry {
        title: slug.to_string(),
        title_slug: slug.to_string(),
        difficulty: diff.to_string(),
        ac_rate: 50.0,
        is_paid_only: paid,
        topic_tags: Vec::new(),
    };
    let refs = pool_to_refs(&[
        entry("free-med", "Medium", false),
        entry("paid-med", "Medium", true),
        entry("odd", "Extreme", false),
    ]);
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].title_slug, "free-med");
    assert_eq!(refs[0].difficulty, Difficulty::Medium);
    // Filtering alone does not bypass session validation: wrong counts fail.
    let mut s = new_session();
    let mediums = difficulty_filter(&pool, Difficulty::Medium);
    assert_eq!(
        s.set_expert_questions(mediums, qs("h", 2, Difficulty::Hard), 1),
        Err(CoreError::InvalidQuestionCount {
            expected: 3,
            got: 2
        })
    );
}
