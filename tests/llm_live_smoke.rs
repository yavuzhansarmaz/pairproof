//! Live Phase 4 LLM wrapper smoke checks. Ignored by default.
//!
//! Run with: MODEL_API_KEY=... cargo test --test llm_live_smoke -- --ignored

use std::collections::BTreeSet;

use pairproof::core::{Language, QuestionRef, Session};
use pairproof::leetcode::selection::select_questions;
use pairproof::leetcode::{Difficulty, LeetCodeClient};
use pairproof::llm::approaches::{generate_approaches, grade_other_approach};
use pairproof::llm::chat::{run_chat, ChatState};
use pairproof::llm::coding::{run_coding_step, CodingStepInput};
use pairproof::llm::extraction::extract_context;
use pairproof::llm::grading::{grade_question, QuestionScoringInput};
use pairproof::llm::MetaClient;
use pairproof::scoring::ApproachQuality;
use rand::rngs::StdRng;
use rand::SeedableRng;

const QUESTION: &str = "Given nums and target, return indices of two numbers summing to target.";

fn live_client() -> MetaClient {
    MetaClient::from_env().expect("MODEL_API_KEY must be set for live smoke tests")
}

fn q(slug: &str) -> QuestionRef {
    QuestionRef {
        title_slug: slug.to_string(),
        title: slug.to_string(),
        difficulty: Difficulty::Easy,
        acceptance_pct: None,
    }
}

fn started() -> Session {
    let easy = (0..5).map(|i| q(&format!("e{i}"))).collect();
    let mut session = Session::new("s1".into(), Language::Python, easy, 0).expect("session");
    session.mark_waiting_for_candidate(1).expect("waiting");
    session.candidate_connected(2).expect("connected");
    session.start(3).expect("start");
    session
}

#[tokio::test]
#[ignore = "live network smoke, requires MODEL_API_KEY"]
async fn live_generate_approaches_returns_four_with_one_best() {
    let client = live_client();
    let mut rng = StdRng::seed_from_u64(42);
    let outcome = generate_approaches(&client, QUESTION, Language::Python, &mut rng)
        .await
        .expect("approaches generated");
    let set = outcome.value;
    assert_eq!(set.options.len(), 4);
    assert_eq!(
        set.quality_of(set.best_index as u8),
        Some(ApproachQuality::Best)
    );
}

#[tokio::test]
#[ignore = "live network smoke, requires MODEL_API_KEY"]
async fn live_grade_other_approach_returns_grade() {
    let client = live_client();
    let outcome = grade_other_approach(&client, QUESTION, "Use a hash map of seen values.")
        .await
        .expect("other approach graded");
    assert!(outcome.value.expected_tokens > 0);
}

#[tokio::test]
#[ignore = "live network smoke, requires MODEL_API_KEY"]
async fn live_run_coding_step_books_tokens() {
    let client = live_client();
    let mut session = started();
    let input = CodingStepInput {
        question_statement: QUESTION.to_string(),
        approach_text: "Use a hash map of seen values.".to_string(),
        current_code: String::new(),
        instruction: None,
        tests_requested: false,
        language: Language::Python,
    };
    let outcome = run_coding_step(&client, &mut session, &input)
        .await
        .expect("coding step runs");
    assert!(!outcome.value.code.trim().is_empty());
    let slot = session.current_slot().expect("current slot");
    assert!(slot.actual_tokens > 0, "tokens booked on current slot");
}

#[tokio::test]
#[ignore = "live network smoke, requires MODEL_API_KEY"]
async fn live_run_chat_replies_without_booking_tokens() {
    let client = live_client();
    let mut session = started();
    let state = ChatState::from_session(&session, QUESTION, "");
    let outcome = run_chat(
        &client,
        &mut session,
        &state,
        &[],
        "What should I think about first?",
        100,
    )
    .await
    .expect("chat replies");
    assert!(!outcome.value.trim().is_empty());
    let slot = session.current_slot().expect("current slot");
    assert_eq!(slot.actual_tokens, 0, "chat books no tokens");
}

#[tokio::test]
#[ignore = "live network smoke, requires MODEL_API_KEY"]
async fn live_extract_context_proposes_language() {
    let client = live_client();
    let jd = "Backend engineer role. Strong Python required; work with arrays and graphs.";
    let additional = "Expert notes: focus on Python and data processing experience.";
    let outcome = extract_context(&client, jd, additional)
        .await
        .expect("context extracted");
    assert_eq!(outcome.value.proposed_language, Some(Language::Python));
    assert!(!outcome.value.level.trim().is_empty());
}

#[tokio::test]
#[ignore = "live network smoke, requires MODEL_API_KEY"]
async fn live_extract_sql_role_proposes_sql_and_database() {
    let client = live_client();
    let jd = "Data analyst role writing MySQL queries against production databases. \
        Strong SQL required; build reports by querying relational tables.";
    let additional = "Expert notes: candidate must write MySQL queries.";
    let outcome = extract_context(&client, jd, additional)
        .await
        .expect("context extracted");
    assert_eq!(outcome.value.proposed_language, Some(Language::Sql));
    assert_eq!(outcome.value.topics, ["database"]);
}

#[tokio::test]
#[ignore = "live network smoke, requires MODEL_API_KEY"]
async fn live_grade_question_returns_valid_grade() {
    let client = live_client();
    let input = QuestionScoringInput {
        slot_index: 2,
        question_statement: QUESTION.to_string(),
        approach_text: "Use a hash map of seen values.".to_string(),
        solution_code: "def two_sum(nums, target):\n    seen = {}\n    for i, n in enumerate(nums):\n        if target - n in seen:\n            return [seen[target - n], i]\n        seen[n] = i".to_string(),
        test_summary: None,
        tests_requested: false,
        expected_tokens: Some(200),
        actual_tokens: 150,
        actions: vec!["question finished".to_string()],
    };
    let outcome = grade_question(&client, &input)
        .await
        .expect("question graded");
    assert!((0.0..=10.0).contains(&outcome.value.grade));
    assert_eq!(outcome.value.slot_index, input.slot_index);
}

#[tokio::test]
#[ignore = "live LeetCode network smoke"]
async fn live_select_questions_picks_easy_five() {
    let client = LeetCodeClient::default();
    let mut rng = StdRng::seed_from_u64(42);
    let picked = select_questions(
        &client,
        Difficulty::Easy,
        &["array"],
        5,
        &BTreeSet::new(),
        &mut rng,
    )
    .await
    .expect("questions selected");
    assert_eq!(picked.len(), 5);
    assert!(picked.iter().all(|r| r.difficulty == Difficulty::Easy));
}
