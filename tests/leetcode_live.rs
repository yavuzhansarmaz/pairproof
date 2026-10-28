//! Live LeetCode spike checks. Networked and ignored by default.
//!
//! Run with: cargo test --test leetcode_live -- --ignored

use pairproof::leetcode::{Difficulty, LeetCodeClient};

#[tokio::test]
#[ignore = "live network spike"]
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
#[ignore = "live network spike"]
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
#[ignore = "live network spike"]
async fn live_premium_question_reports_premium_only() {
    let client = LeetCodeClient::default();
    let result = client.fetch_question("meeting-rooms").await;
    assert!(result.is_err());
}
