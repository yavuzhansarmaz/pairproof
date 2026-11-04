//! Full dual-runtime interview simulation over live Docker, SSH,
//! Mutagen, Meta, and LeetCode. Expert setup pastes and candidate
//! connect/rules/work keys are the same handlers the TTY loops use.
//! Ignored by default so `cargo test` stays hermetic.

mod flow;
mod support;
mod tty;

use pairproof::connection::{Endpoint, EphemeralUsername, Secret};
use pairproof::llm::MetaClient;
use pairproof::sandbox::lifecycle::{create_and_provision, destroy, destroy_by_session_id};
use pairproof::sandbox::preflight::port_is_free;
use pairproof::tui::expert_runtime::format_session_id;

use flow::flow;
use support::now_ms;

#[tokio::test]
#[ignore = "live docker+ssh+mutagen+meta+leetcode: full TTY session sim"]
async fn full_interview_simulation_live_docker_meta_tty() {
    let meta = MetaClient::from_env().expect("MODEL_API_KEY must be set for live full-session sim");
    let port = (22700..22900u16)
        .find(|p| port_is_free(*p))
        .expect("free port");
    let session_id = format_session_id(std::process::id(), now_ms());
    let endpoint = Endpoint::new("127.0.0.1", port).expect("endpoint");
    let username = EphemeralUsername::generate();
    let secret = Secret::generate();
    let (handle, fingerprint) =
        match create_and_provision(&session_id, &endpoint, &username, &secret).await {
            Ok(pair) => pair,
            Err(err) => {
                let _ = destroy_by_session_id(&session_id).await;
                panic!("create_and_provision failed: {err}");
            }
        };
    println!(
        "livesim provisioned container={} port={port}",
        handle.container_name()
    );
    let outcome = flow(
        &session_id,
        &endpoint,
        &username,
        &handle,
        fingerprint,
        meta,
    )
    .await;
    let destroyed = destroy(&handle).await;
    let _ = destroy_by_session_id(&session_id).await;
    assert!(destroyed.is_ok(), "destroy: {destroyed:?}");
    outcome.expect("live full-session sim (Docker + Meta + TTY handlers)");
}
