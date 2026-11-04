//! Live two-runtime session with REAL Meta + REAL LeetCode at the
//! expert boundary: real Docker sandbox, FIFO SSH auth, control
//! channel and isolated Mutagen sync (mirrors tui_live_session.rs,
//! but statement, approaches, coding, chat and grading hit the real
//! services). Ignored by default; run explicitly with .env sourced.
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pairproof::connection::{ConnectionString, Endpoint, EphemeralUsername, Secret};
use pairproof::core::Language;
use pairproof::leetcode::LeetCodeClient;
use pairproof::llm::MetaClient;
use pairproof::orchestration::*;
use pairproof::sandbox::lifecycle::{create_and_provision, destroy, destroy_by_session_id};
use pairproof::sandbox::preflight::port_is_free;
use pairproof::tui::candidate_runtime::{
    CandidateRuntime, CandidateRuntimePhase, SshCandidateTransport,
};
use pairproof::tui::expert_runtime::{
    DockerHelloSource, ExpertRuntime, ExpertRuntimeDeps, ExpertSessionSpec, PollOutcome,
};

use crate::common_mod;
use common_mod::q_easy as q;
#[rustfmt::skip]
fn now_ms() -> u64 { u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)).unwrap_or(u64::MAX) }

#[rustfmt::skip]
fn chk(cond: bool, msg: &str) -> Result<(), String> { if cond { Ok(()) } else { Err(msg.to_string()) } }

async fn flow(
    session_id: &str,
    endpoint: &Endpoint,
    username: &EphemeralUsername,
    secret: &Secret,
    handle: &pairproof::sandbox::SandboxHandle,
    fingerprint: pairproof::connection::Fingerprint,
    meta: MetaClient,
) -> Result<(), String> {
    let base = std::env::temp_dir().join(format!(
        "pairproof-livemeta-{}-{}",
        std::process::id(),
        endpoint.port()
    ));
    let _ = std::fs::remove_dir_all(&base);
    let alpha = base.join("alpha");
    let artifacts = base.join("artifacts");
    std::fs::create_dir_all(&alpha).map_err(|e| e.to_string())?;
    let control_io = DockerControlIo::new(handle.clone());
    control_io
        .setup_dirs(username.as_str())
        .await
        .map_err(|e| e.to_string())?;
    let spec = ExpertSessionSpec {
        session_id: session_id.into(),
        language: Language::Python,
        easy: [
            "two-sum",
            "palindrome-number",
            "roman-to-integer",
            "longest-common-prefix",
            "valid-parentheses",
        ]
        .iter()
        .map(|s| q(s))
        .collect(),
        endpoint: endpoint.clone(),
        username: username.clone(),
        fingerprint: fingerprint.clone(),
    };
    let deps = ExpertRuntimeDeps {
        control_io: Box::new(control_io),
        hello: Box::new(DockerHelloSource::new(handle.clone())),
        fetcher: Arc::new(LeetcodeStatementFetcher::new(LeetCodeClient::default())),
        generator: Arc::new(MetaApproachGenerator::new(meta.clone())),
        coding: Arc::new(MetaCodingTransport::new(meta.clone())),
        sandbox_io: Arc::new(DockerSandboxIo::new(handle.clone(), username.clone())),
        chat: Arc::new(MetaChatTransport::new(meta.clone())),
        grader: Arc::new(MetaQuestionGrader::new(meta)),
        leetcode: Some(LeetCodeClient::default()),
        sandbox_handle: Some(handle.clone()),
        artifact_root: artifacts.clone(),
        topics: Vec::new(),
        level: "live".into(),
        sync_phase: SessionSyncPhase::NotStarted,
        shadow_evaluator: None,
    };
    let mut expert = ExpertRuntime::new(spec, deps, now_ms())
        .await
        .map_err(|e| e.to_string())?;
    println!("livemeta expert built session={session_id} session_json=published");
    let connection = ConnectionString::new(
        endpoint.clone(),
        username.clone(),
        secret.clone(),
        4_102_444_800_000,
        fingerprint,
    );
    let mut candidate =
        CandidateRuntime::new_discovering(alpha.clone(), SshCandidateTransport::new());
    candidate.connect(&connection).map_err(|e| e.to_string())?;
    chk(
        candidate.phase() == CandidateRuntimePhase::Connected,
        "candidate not Connected",
    )?;
    chk(
        candidate.session_id() == session_id,
        "candidate did not discover expert session id",
    )?;
    println!(
        "livemeta candidate connected discovered={} phase=Connected",
        candidate.session_id()
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if expert.poll_candidate(now_ms()).await == PollOutcome::Connected {
            break;
        }
        chk(Instant::now() < deadline, "expert never accepted hello")?;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    println!("livemeta expert poll_candidate=Connected");
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        expert
            .drain_intents(now_ms())
            .await
            .map_err(|e| e.to_string())?;
        let _ = candidate.poll_snapshot();
        if candidate.view().options.len() == 4 {
            break;
        }
        chk(
            Instant::now() < deadline,
            "real approaches never reached candidate",
        )?;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    // Real set: 4 generated options plus the candidate "Other" choice.
    chk(
        candidate
            .view()
            .statement
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty()),
        "real LeetCode statement missing from snapshot",
    )?;
    println!(
        "livemeta real approaches options={} (plus Other) titles={:?}",
        candidate.view().options.len(),
        candidate
            .view()
            .options
            .iter()
            .map(|o| o.title.clone())
            .collect::<Vec<_>>()
    );
    candidate
        .send_intent(IntentKind::SelectPreset { preset: 1 })
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        expert
            .drain_intents(now_ms())
            .await
            .map_err(|e| e.to_string())?;
        let _ = candidate.poll_snapshot();
        if candidate.view().question_index == Some(0) && candidate.view().selected.is_some() {
            break;
        }
        chk(Instant::now() < deadline, "SelectPreset never applied")?;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    println!(
        "livemeta SelectPreset applied selected={:?}",
        candidate.view().selected
    );
    let tokens = expert
        .orchestrator()
        .session()
        .slot(0)
        .map(|s| s.actual_tokens)
        .unwrap_or(0);
    chk(tokens > 0, "real Meta coding booked no tokens on slot 0")?;
    println!("livemeta tokens slot0 actual_tokens={tokens}");
    let _ = candidate.flush();
    let solution = alpha.join("q1").join("solution.py");
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if std::fs::read_to_string(&solution)
            .map(|c| !c.trim().is_empty())
            .unwrap_or(false)
        {
            break;
        }
        chk(
            Instant::now() < deadline,
            "real solution never synced to candidate alpha",
        )?;
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    println!(
        "livemeta real solution synced to alpha: {}",
        solution.display()
    );
    if expert.orchestrator().current_index() == Some(0) {
        candidate
            .send_intent(IntentKind::RequestTests)
            .map_err(|e| e.to_string())?;
    }
    candidate
        .send_chat("What should I think about first?")
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        expert
            .drain_intents(now_ms())
            .await
            .map_err(|e| e.to_string())?;
        let _ = candidate.poll_snapshot();
        if !candidate.view().chat_replies.is_empty() {
            break;
        }
        chk(Instant::now() < deadline, "real chat reply never arrived")?;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let reply = candidate
        .view()
        .chat_replies
        .last()
        .cloned()
        .unwrap_or_default();
    chk(!reply.trim().is_empty(), "real chat reply is empty")?;
    let q0_reply = reply.clone();
    println!("livemeta real chat reply in snapshot: {reply:?}");
    if expert.orchestrator().current_index() == Some(0) {
        candidate
            .send_intent(IntentKind::Finish)
            .map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            expert
                .drain_intents(now_ms())
                .await
                .map_err(|e| e.to_string())?;
            if expert.orchestrator().current_index() == Some(1) {
                break;
            }
            chk(Instant::now() < deadline, "Finish never applied")?;
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
    println!(
        "livemeta question advanced current_index={}",
        expert.orchestrator().current_index().unwrap_or(0)
    );
    let _ = candidate.poll_snapshot();
    chk(
        candidate.view().question_index == Some(1),
        "candidate snapshot did not follow Finish onto Q1",
    )?;
    chk(
        candidate.view().chat_replies.is_empty()
            && !candidate.view().chat_replies.contains(&q0_reply),
        &format!(
            "Q1 coach leaked Q0 replies: {:?}",
            candidate.view().chat_replies
        ),
    )?;
    let poller = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(600);
        let mut polls = 0u32;
        let mut ok = 0u32;
        let mut err = 0u32;
        loop {
            polls += 1;
            match candidate.poll_snapshot() {
                Ok(_) => ok += 1,
                Err(_) => err += 1,
            }
            if candidate.view().final_breakdown.is_some() {
                break;
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let rating = candidate.view().final_breakdown.as_ref().map(|f| f.rating);
        let lines = candidate.view().lines();
        let chats = candidate.view().chat_replies.len();
        let phase = candidate.view().phase.clone();
        let acked = candidate.final_ack_sent();
        (
            candidate, rating, lines, chats, polls, ok, err, phase, acked,
        )
    });
    let end_started = Instant::now();
    let report = expert
        .end_session(now_ms())
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "livemeta end_session elapsed={:?} attempted={:?} failed={:?}",
        end_started.elapsed(),
        report.attempted,
        report.failed
    );
    chk(
        expert.final_ack_outcome()
            == Some(pairproof::orchestration::control::FinalAckOutcome::Received),
        "expert never received candidate final ack before destroy",
    )?;
    println!("livemeta expert final_snapshot_ack=received before destroy");
    let (mut candidate, rating, lines, chats, polls, ok, err, phase, acked) = poller
        .join()
        .map_err(|_| "final poller panicked".to_string())?;
    chk(acked, "candidate never acked final snapshot")?;
    println!(
        "livemeta final poll polls={polls} ok={ok} err={err} last_phase={phase:?} chats={chats}"
    );
    let rating = rating.ok_or("candidate never polled final snapshot with rating")?;
    chk(rating.is_finite(), "final rating not finite")?;
    chk(
        chats == 0,
        &format!("final snapshot carried Q0 coach onto the next question, chats={chats}"),
    )?;
    chk(
        lines.iter().any(|l| l.contains("final rating")),
        "candidate view lacks final rating line",
    )?;
    println!("livemeta candidate final rating={rating:.2} lines={lines:?}");
    candidate.end().map_err(|e| e.to_string())?;
    chk(
        candidate.phase() == CandidateRuntimePhase::Ended,
        "candidate not Ended",
    )?;
    let kept = std::fs::read_to_string(&solution).map_err(|e| e.to_string())?;
    chk(
        !kept.trim().is_empty(),
        "terminate_sync lost synced real solution",
    )?;
    println!("livemeta terminate_sync preserved alpha solution");
    let bundle = artifacts.join(session_id);
    chk(
        bundle.join("report.md").is_file(),
        "section-15 report.md missing",
    )?;
    chk(
        bundle.join("events.jsonl").is_file(),
        "section-15 events.jsonl missing",
    )?;
    chk(!bundle.join("raw").exists(), "raw artifacts remain")?;
    println!("livemeta bundle ok: {}", bundle.display());
    let _ = std::fs::remove_dir_all(&base);
    Ok(())
}

#[tokio::test]
#[ignore = "live docker+ssh+mutagen+meta+leetcode: real sandbox/SSH/Mutagen/control and real Meta/LeetCode"]
async fn live_two_runtime_session_with_real_meta_and_leetcode() {
    let meta =
        MetaClient::from_env().expect("MODEL_API_KEY must be set for live Meta session test");
    let port = (22500..22700u16)
        .find(|p| port_is_free(*p))
        .expect("free port");
    let session_id = format!("livemeta{}x{}", std::process::id(), port);
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
        "livemeta provisioned container={} port={port}",
        handle.container_name()
    );
    let outcome = flow(
        &session_id,
        &endpoint,
        &username,
        &secret,
        &handle,
        fingerprint,
        meta,
    )
    .await;
    let destroyed = destroy(&handle).await;
    let _ = destroy_by_session_id(&session_id).await;
    assert!(destroyed.is_ok(), "destroy: {destroyed:?}");
    outcome.expect("live two-runtime session (real Meta + real LeetCode)");
}
