//! Randomized LeetCode selection checks on in-memory catalog entries.

use pairproof::core::{Language, Session};
use pairproof::leetcode::selection::{
    auto_pick_easy, difficulty_of, expert_pool, pick_random, select_questions, to_question_ref,
    SelectionError,
};
use pairproof::leetcode::{CatalogEntry, Difficulty, LeetCodeClient};
use rand::rngs::StdRng;
use rand::SeedableRng;
use std::collections::BTreeSet;

/// Builds a free catalog entry with the given slug and difficulty string.
fn entry(slug: &str, difficulty: &str) -> CatalogEntry {
    CatalogEntry {
        title: slug.to_string(),
        title_slug: slug.to_string(),
        difficulty: difficulty.to_string(),
        ac_rate: 50.0,
        is_paid_only: false,
        topic_tags: Vec::new(),
    }
}

/// Builds `n` entries named `{prefix}0..{prefix}{n-1}` at one difficulty.
fn entries(prefix: &str, n: usize, difficulty: &str) -> Vec<CatalogEntry> {
    (0..n)
        .map(|i| entry(&format!("{prefix}{i}"), difficulty))
        .collect()
}

#[test]
fn pick_random_deterministic_with_seed_and_respects_count() {
    let pool = entries("q", 8, "Easy");
    let mut rng_a = StdRng::seed_from_u64(42);
    let mut rng_b = StdRng::seed_from_u64(42);
    let first = pick_random(&pool, 3, &BTreeSet::new(), &mut rng_a);
    let second = pick_random(&pool, 3, &BTreeSet::new(), &mut rng_b);
    assert_eq!(first, second);
    assert_eq!(first.len(), 3);
}

#[test]
fn pick_random_excludes_slugs_paid_and_unknown_difficulty() {
    let mut pool = entries("q", 5, "Easy");
    let mut paid = entry("paid-q", "Easy");
    paid.is_paid_only = true;
    pool.push(paid);
    pool.push(entry("extreme-q", "Extreme"));
    pool.push(entry("lower-q", "easy"));
    let exclude = BTreeSet::from(["q0".to_string()]);
    let mut rng = StdRng::seed_from_u64(7);
    let picked = pick_random(&pool, 20, &exclude, &mut rng);
    let slugs: Vec<&str> = picked.iter().map(|q| q.title_slug.as_str()).collect();
    assert!(!slugs.contains(&"q0"));
    assert!(!slugs.contains(&"paid-q"));
    assert!(!slugs.contains(&"extreme-q"));
    assert!(slugs.contains(&"lower-q"));
    assert_eq!(picked.len(), 5);
}

#[test]
fn pick_random_dedupes_duplicate_slugs() {
    let mut pool = entries("q", 3, "Easy");
    let mut dupe = entry("q0", "Easy");
    dupe.title = "Duplicate Title".to_string();
    pool.push(dupe);
    pool.push(entry("q1", "Easy"));
    let mut rng = StdRng::seed_from_u64(11);
    let picked = pick_random(&pool, 20, &BTreeSet::new(), &mut rng);
    assert_eq!(picked.len(), 3);
    let q0 = picked
        .iter()
        .find(|q| q.title_slug == "q0")
        .expect("q0 present");
    assert_eq!(q0.title, "q0");
}

#[test]
fn pick_random_zero_count_and_short_pool() {
    let pool = entries("q", 2, "Easy");
    let mut rng = StdRng::seed_from_u64(3);
    assert!(pick_random(&pool, 0, &BTreeSet::new(), &mut rng).is_empty());
    let mut rng = StdRng::seed_from_u64(3);
    let picked = pick_random(&pool, 5, &BTreeSet::new(), &mut rng);
    assert_eq!(picked.len(), 2);
}

#[test]
fn pick_random_maps_difficulties() {
    let pool = vec![
        entry("easy-q", "Easy"),
        entry("medium-q", " Medium "),
        entry("hard-q", "HARD"),
    ];
    let mut rng = StdRng::seed_from_u64(5);
    let picked = pick_random(&pool, 3, &BTreeSet::new(), &mut rng);
    let by_slug = |slug: &str| {
        picked
            .iter()
            .find(|q| q.title_slug == slug)
            .expect("entry present")
            .difficulty
    };
    assert_eq!(by_slug("easy-q"), Difficulty::Easy);
    assert_eq!(by_slug("medium-q"), Difficulty::Medium);
    assert_eq!(by_slug("hard-q"), Difficulty::Hard);
}

#[test]
fn picked_easy_five_feed_session_new() {
    let pool = entries("easy", 7, "Easy");
    let mut rng = StdRng::seed_from_u64(99);
    let picked = pick_random(&pool, 5, &BTreeSet::new(), &mut rng);
    assert_eq!(picked.len(), 5);
    let session = Session::new("s".to_string(), Language::Python, picked, 0);
    assert!(session.is_ok());
}

#[tokio::test]
async fn select_questions_unreachable_api_is_typed_error() {
    let client = LeetCodeClient::new("http://127.0.0.1:9/graphql");
    let mut rng = StdRng::seed_from_u64(1);
    let result = select_questions(
        &client,
        Difficulty::Easy,
        &["array"],
        5,
        &BTreeSet::new(),
        &mut rng,
    )
    .await;
    assert!(matches!(result, Err(SelectionError::Api(_))));
}

#[tokio::test]
async fn expert_pool_unreachable_api_is_typed_error() {
    let client = LeetCodeClient::new("http://127.0.0.1:9/graphql");
    let result = expert_pool(&client, Difficulty::Medium, &["array"], &BTreeSet::new()).await;
    assert!(matches!(result, Err(SelectionError::Api(_))));
}

#[tokio::test]
async fn auto_pick_easy_unreachable_api_is_typed_error() {
    let client = LeetCodeClient::new("http://127.0.0.1:9/graphql");
    let mut rng = StdRng::seed_from_u64(1);
    let result = auto_pick_easy(&client, &["array"], &mut rng).await;
    assert!(matches!(result, Err(SelectionError::Api(_))));
}

#[test]
fn difficulty_of_maps_case_and_whitespace() {
    assert_eq!(difficulty_of(&entry("q", "Easy")), Some(Difficulty::Easy));
    assert_eq!(
        difficulty_of(&entry("q", "  HARD ")),
        Some(Difficulty::Hard)
    );
    assert_eq!(
        difficulty_of(&entry("q", "medium")),
        Some(Difficulty::Medium)
    );
    assert_eq!(difficulty_of(&entry("q", "")), None);
    assert_eq!(difficulty_of(&entry("q", "Extreme")), None);
    assert_eq!(difficulty_of(&entry("q", "Medium-Hard")), None);
}

#[test]
fn to_question_ref_rejects_paid_and_unknown_and_maps_fields() {
    let mut paid = entry("paid-q", "Easy");
    paid.is_paid_only = true;
    assert_eq!(to_question_ref(&paid), None);
    assert_eq!(to_question_ref(&entry("extreme-q", "Extreme")), None);
    let mut free = entry("two-sum", "Easy");
    free.title = "Two Sum".to_string();
    let question = to_question_ref(&free).expect("free easy entry maps");
    assert_eq!(question.title_slug, "two-sum");
    assert_eq!(question.title, "Two Sum");
    assert_eq!(question.difficulty, Difficulty::Easy);
}
