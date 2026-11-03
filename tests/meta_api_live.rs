//! Live Meta Model API spike checks. Requires MODEL_API_KEY; ignored by default.
//!
//! Run with: MODEL_API_KEY=... cargo test meta_api_live -- --ignored

use pairproof::llm::{Message, MetaClient, LIGHT_MODEL, STRONG_MODEL};

fn live_client() -> MetaClient {
    MetaClient::from_env().expect("MODEL_API_KEY must be set for the live spike")
}

#[tokio::test]
#[ignore = "live network spike, requires MODEL_API_KEY"]
async fn live_models_include_configured_tiers() {
    let client = live_client();
    let models = client.list_models().await.expect("models responds");
    assert!(
        models.iter().any(|m| m == STRONG_MODEL),
        "strong model visible"
    );
    assert!(
        models.iter().any(|m| m == LIGHT_MODEL),
        "light model visible"
    );
}

#[tokio::test]
#[ignore = "live network spike, requires MODEL_API_KEY"]
async fn live_strong_chat_reports_usage() {
    let client = live_client();
    let outcome = client
        .chat(STRONG_MODEL, vec![Message::new("user", "Reply with OK.")])
        .await
        .expect("strong chat responds");
    assert!(!outcome.content.is_empty());
    assert!(outcome.usage.total_tokens > 0);
}

#[tokio::test]
#[ignore = "live network spike, requires MODEL_API_KEY"]
async fn live_light_chat_streams_with_usage() {
    let client = live_client();
    let outcome = client
        .chat_stream(LIGHT_MODEL, vec![Message::new("user", "Reply with OK.")])
        .await
        .expect("light stream responds");
    assert!(!outcome.content.is_empty());
    assert!(outcome.usage.total_tokens > 0);
}
