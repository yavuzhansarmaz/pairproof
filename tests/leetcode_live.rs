//! Live LeetCode spike checks. Networked and ignored by default.
//!
//! Run with: cargo test --test leetcode_live -- --ignored

use std::collections::BTreeSet;

use pairproof::core::Language;
use pairproof::leetcode::selection::{auto_pick_easy, select_questions};
use pairproof::leetcode::{Difficulty, LeetCodeClient};

#[tokio::test]
#[ignore = "live LeetCode network spike"]
async fn live_catalog_lists_free_easy_array_questions() {
    let client = LeetCodeClient::default();
    let page = client
        .list_questions(Difficulty::Easy, &["array"], 5, 0)
        .await
        .expect("catalog responds");
    assert!(page.total_reported > 0);
    assert!(!page.questions.is_empty());
    assert!(page.questions.iter().all(|q| !q.is_paid_only));
}

#[tokio::test]
#[ignore = "live LeetCode network spike"]
async fn live_question_fetches_statement_and_snippets() {
    let client = LeetCodeClient::default();
    let question = client
        .fetch_question("two-sum")
        .await
        .expect("question responds");
    assert_eq!(question.title_slug, "two-sum");
    assert!(!question.content.is_empty());
    assert!(question.snippet_for("python3").is_some());
    assert!(question.snippet_for("javascript").is_some());
}

#[tokio::test]
#[ignore = "live LeetCode network spike"]
async fn live_premium_question_reports_premium_only() {
    let client = LeetCodeClient::default();
    let result = client.fetch_question("meeting-rooms").await;
    assert!(result.is_err());
}

#[tokio::test]
#[ignore = "live LeetCode network spike"]
async fn live_database_question_has_mysql_snippet() {
    let client = LeetCodeClient::default();
    let page = client
        .list_questions(Difficulty::Easy, &["database"], 5, 0)
        .await
        .expect("catalog responds");
    let first = page.questions.first().expect("free database question");
    let question = client
        .fetch_question(&first.title_slug)
        .await
        .expect("question responds");
    assert!(question
        .snippet_for(Language::Sql.leetcode_slug())
        .is_some());
    assert!(question.snippet_for("python3").is_none());
}

#[tokio::test]
#[ignore = "live LeetCode network spike"]
async fn live_database_full_composition_selectable() {
    let client = LeetCodeClient::default();
    let mut rng = rand::thread_rng();
    let topics = ["database"];
    let easy = auto_pick_easy(&client, &topics, &mut rng)
        .await
        .expect("easy database pick");
    let easy_slugs: BTreeSet<String> = easy.iter().map(|q| q.title_slug.clone()).collect();
    assert_eq!(easy_slugs.len(), 5, "easy slugs distinct: {easy:?}");
    let medium = select_questions(
        &client,
        Difficulty::Medium,
        &topics,
        3,
        &easy_slugs,
        &mut rng,
    )
    .await
    .expect("medium database pick");
    let hard = select_questions(&client, Difficulty::Hard, &topics, 2, &easy_slugs, &mut rng)
        .await
        .expect("hard database pick");
    for (picked, difficulty, count) in [
        (&easy, Difficulty::Easy, 5),
        (&medium, Difficulty::Medium, 3),
        (&hard, Difficulty::Hard, 2),
    ] {
        assert_eq!(picked.len(), count);
        assert!(
            picked
                .iter()
                .all(|q| q.difficulty == difficulty && q.acceptance_pct.is_some()),
            "{picked:?}"
        );
    }
    let all_slugs: BTreeSet<&str> = easy
        .iter()
        .chain(&medium)
        .chain(&hard)
        .map(|q| q.title_slug.as_str())
        .collect();
    assert_eq!(all_slugs.len(), 10, "all selected slugs distinct");
}
