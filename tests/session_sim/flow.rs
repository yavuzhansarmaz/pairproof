use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::KeyCode;
use pairproof::connection::{Endpoint, EphemeralUsername, Fingerprint};
use pairproof::core::{Language, SessionStatus};
use pairproof::leetcode::LeetCodeClient;
use pairproof::llm::MetaClient;
use pairproof::orchestration::{
    ControlIo, DockerControlIo, DockerSandboxIo, IntentKind, LeetcodeStatementFetcher,
    MetaApproachGenerator, MetaChatTransport, MetaCodingTransport, MetaQuestionGrader,
    SessionSyncPhase, SystemClock,
};
use pairproof::sandbox::lifecycle::rotate_secret;
use pairproof::sandbox::SandboxHandle;
use pairproof::scoring::live_rating;
use pairproof::tui::candidate::{CandidateApp, CandidateScreen};
use pairproof::tui::candidate_chat::ChatController;
use pairproof::tui::candidate_run::{
    handle_candidate_key, handle_candidate_paste, CandidateRunState,
};
use pairproof::tui::candidate_runtime::{
    CandidateRuntime, CandidateRuntimePhase, SshAuthVerifier, SshCandidateTransport,
};
use pairproof::tui::candidate_work::CandidateWork;
use pairproof::tui::expert_runtime::{
    DockerHelloSource, ExpertRuntime, ExpertRuntimeDeps, ExpertSessionSpec, PollOutcome,
    SelectionScreen,
};
use pairproof::tui::setup::SetupController;
use pairproof::tui::ExpertSetupPastes;
use rand::rngs::StdRng;
use rand::SeedableRng;

use super::support::{
    chk, event_kinds, has_kind, now_ms, other_texts, selected_presets, solution_synced,
    unused_medium, JD, META_WAIT_SECS, SYNC_WAIT_SECS,
};
use super::tty::{
    apply_kind, enter, key_chat, key_other, key_preset, key_redirect, key_work, leave_other_row,
    press, pump_until, wait_idle, wait_options,
};

pub(super) async fn flow(
    session_id: &str,
    endpoint: &Endpoint,
    username: &EphemeralUsername,
    handle: &SandboxHandle,
    fingerprint: Fingerprint,
    meta: MetaClient,
) -> Result<(), String> {
    let mut pastes = ExpertSetupPastes::new();
    pastes
        .apply_focused_paste(JD)
        .map_err(|e| format!("paste JD: {e}"))?;
    chk(pastes.confirm_job_description(), "expert JD confirm failed")?;
    pastes.acknowledge_disclosure();
    println!("livesim setup JD pasted");
    let proposed = pastes
        .request_proposal(&meta)
        .await
        .map_err(|e| format!("live Meta proposal: {e}"))?;
    chk(
        !proposed.topics.is_empty(),
        "live extraction returned no topics",
    )?;
    let language = pastes.selected_language().unwrap_or(Language::Python);
    println!(
        "livesim proposal level={} language={language:?} topics={:?}",
        proposed.level, proposed.topics
    );

    let mut setup = SetupController::new();
    setup.apply_proposal(proposed.clone());
    let leetcode = LeetCodeClient::default();
    let mut rng = StdRng::seed_from_u64(u64::from(std::process::id()));
    setup
        .auto_select_easy(&leetcode, &mut rng)
        .await
        .map_err(|e| format!("auto_select_easy: {e}"))?;
    let easy = setup.easy_questions().to_vec();
    chk(easy.len() == 5, "auto easy count")?;
    println!(
        "livesim auto easy={:?}",
        easy.iter()
            .map(|q| q.title_slug.clone())
            .collect::<Vec<_>>()
    );

    let base = std::env::temp_dir().join(format!(
        "pairproof-livesim-{}-{}",
        std::process::id(),
        endpoint.port()
    ));
    let _ = std::fs::remove_dir_all(&base);
    let workspace = base.join("alpha");
    let artifacts = base.join("artifacts");
    std::fs::create_dir_all(&workspace).map_err(|e| e.to_string())?;

    let control_io = DockerControlIo::new(handle.clone());
    control_io
        .setup_dirs(username.as_str())
        .await
        .map_err(|e| e.to_string())?;
    let spec = ExpertSessionSpec {
        session_id: session_id.into(),
        language,
        easy,
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
        topics: proposed.topics.clone(),
        level: proposed.level.clone(),
        sync_phase: SessionSyncPhase::NotStarted,
        shadow_evaluator: None,
    };
    let mut expert = ExpertRuntime::new(spec, deps, now_ms())
        .await
        .map_err(|e| e.to_string())?;
    expert.set_clock(Arc::new(SystemClock));
    println!("livesim expert built session={session_id}");

    let conn = expert
        .current_string(now_ms())
        .map_err(|e| format!("connection string: {e}"))?;
    rotate_secret(handle, username, conn.secret())
        .await
        .map_err(|e| format!("rotate_secret: {e}"))?;
    let encoded = conn.encode();
    chk(
        encoded.starts_with("pairproof:v1:"),
        "connection string prefix",
    )?;
    println!("livesim rotated live connection secret; connecting candidate");

    let app = CandidateApp::with_verifier(SshAuthVerifier::new());
    let mut state =
        CandidateRunState::with_parts(app, CandidateWork::new(), ChatController::default());
    handle_candidate_paste(&mut state, &encoded);
    state
        .app
        .submit_connection(now_ms())
        .map_err(|e| format!("candidate TTY verify: {e}"))?;
    chk(
        state.app.screen() == CandidateScreen::Rules,
        "candidate TTY did not reach Rules after live SSH verify",
    )?;
    handle_candidate_key(&mut state, enter());
    chk(state.app.rules_accepted(), "rules not accepted")?;
    chk(
        state.app.screen() == CandidateScreen::Waiting,
        "candidate TTY did not reach Waiting",
    )?;

    let mut candidate =
        CandidateRuntime::new_discovering(workspace.clone(), SshCandidateTransport::new());
    candidate
        .connect(&conn)
        .map_err(|e| format!("candidate SSH/Mutagen connect: {e}"))?;
    chk(
        candidate.phase() == CandidateRuntimePhase::Connected,
        "candidate not Connected",
    )?;
    chk(
        candidate.session_id() == session_id,
        "candidate did not discover expert session id",
    )?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if expert.poll_candidate(now_ms()).await == PollOutcome::Connected {
            break;
        }
        chk(Instant::now() < deadline, "expert never accepted hello")?;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    println!("livesim expert poll_candidate=Connected");

    expert
        .fetch_medium(now_ms())
        .await
        .map_err(|e| format!("fetch_medium: {e}"))?;
    chk(
        expert.randomize_picks(SelectionScreen::Medium) >= 1,
        "medium randomize picked nothing",
    )?;
    expert
        .confirm_medium()
        .map_err(|e| format!("confirm_medium: {e}"))?;
    expert
        .fetch_hard(now_ms())
        .await
        .map_err(|e| format!("fetch_hard: {e}"))?;
    chk(
        expert.randomize_picks(SelectionScreen::Hard) >= 1,
        "hard randomize picked nothing",
    )?;
    expert
        .complete_selection(now_ms())
        .await
        .map_err(|e| format!("complete_selection: {e}"))?;
    println!("livesim expert medium/hard committed");

    wait_options(&mut expert, &mut candidate, &mut state, 0).await?;
    println!(
        "livesim Q0 ready title={:?} options={:?}",
        candidate.view().question_title,
        candidate
            .view()
            .options
            .iter()
            .map(|o| o.title.clone())
            .collect::<Vec<_>>()
    );

    key_preset(&mut expert, &mut candidate, &mut state, 0).await?;
    chk(
        solution_synced(&workspace, 0),
        "Q0 solution never synced after preset 0",
    )?;
    key_redirect(&mut expert, &mut candidate, &mut state, "use a heap").await?;
    key_work(
        &mut expert,
        &mut candidate,
        &mut state,
        't',
        "request tests",
        META_WAIT_SECS,
    )
    .await?;
    key_work(
        &mut expert,
        &mut candidate,
        &mut state,
        'r',
        "run solution",
        SYNC_WAIT_SECS,
    )
    .await?;
    pump_until(
        &mut expert,
        &mut candidate,
        &mut state,
        SYNC_WAIT_SECS,
        |_, candidate, _| candidate.view().latest_test.is_some(),
        "t then r never recorded latest_test counts",
    )
    .await?;
    assert_counted_tests(&expert, &candidate, now_ms())?;
    println!(
        "livesim Q0 run latest_test={:?} latest_run={:?}",
        candidate.view().latest_test,
        candidate
            .view()
            .latest_run()
            .map(|r| (r.kind.clone(), r.success))
    );
    key_chat(&mut expert, &mut candidate, &mut state, "hint please").await?;
    let q0_replies = candidate.view().chat_replies.clone();
    chk(
        !q0_replies.is_empty(),
        "Q0 coach reply missing before finish",
    )?;
    chk(
        state
            .chat
            .sent_messages(Some(0))
            .iter()
            .any(|m| m.contains("hint please")),
        &format!(
            "Q0 sent chat missing: {:?}",
            state.chat.sent_messages(Some(0))
        ),
    )?;
    key_work(
        &mut expert,
        &mut candidate,
        &mut state,
        'f',
        "finish Q0",
        META_WAIT_SECS,
    )
    .await?;
    wait_options(&mut expert, &mut candidate, &mut state, 1).await?;
    println!("livesim Q0 finished");
    assert_coach_is_question_local(&candidate, &state, 1, &q0_replies)?;
    key_chat(
        &mut expert,
        &mut candidate,
        &mut state,
        "what is the invariant?",
    )
    .await?;
    chk(
        candidate
            .view()
            .chat_replies
            .iter()
            .all(|reply| !q0_replies.contains(reply)),
        &format!(
            "Q1 coach still showing Q0 replies: q1={:?} q0={q0_replies:?}",
            candidate.view().chat_replies
        ),
    )?;
    chk(
        state
            .chat
            .sent_messages(Some(1))
            .iter()
            .any(|m| m.contains("invariant")),
        &format!(
            "Q1 sent chat missing: {:?}",
            state.chat.sent_messages(Some(1))
        ),
    )?;
    chk(
        state
            .chat
            .sent_messages(Some(0))
            .iter()
            .any(|m| m.contains("hint please")),
        "Q0 coach messages were dropped after switching questions",
    )?;
    println!(
        "livesim Q1 coach replies={:?} q0_kept={}",
        candidate.view().chat_replies,
        state.chat.sent_messages(Some(0)).len()
    );

    key_preset(&mut expert, &mut candidate, &mut state, 1).await?;
    apply_kind(
        &mut expert,
        &mut candidate,
        IntentKind::Continue,
        "Continue",
    )
    .await?;
    pump_until(
        &mut expert,
        &mut candidate,
        &mut state,
        SYNC_WAIT_SECS,
        |_, candidate, _| {
            candidate
                .view()
                .question_index
                .is_some_and(|index| solution_synced(candidate.workspace(), index))
        },
        "continue never synced solution",
    )
    .await?;
    key_work(
        &mut expert,
        &mut candidate,
        &mut state,
        'f',
        "finish Q1",
        META_WAIT_SECS,
    )
    .await?;
    wait_options(&mut expert, &mut candidate, &mut state, 2).await?;
    println!("livesim Q1 finished");

    key_other(
        &mut expert,
        &mut candidate,
        &mut state,
        "two pointers from both ends",
    )
    .await?;
    wait_idle(
        &mut expert,
        &mut candidate,
        &mut state,
        META_WAIT_SECS,
        "change approach: not idle",
    )
    .await?;
    leave_other_row(&mut state, &mut candidate)?;
    press(
        &mut state,
        &mut candidate,
        KeyCode::Char('a'),
        "change approach",
    )?;
    pump_until(
        &mut expert,
        &mut candidate,
        &mut state,
        META_WAIT_SECS,
        |_, candidate, _| {
            candidate.view().question_index == Some(2) && candidate.view().selected.is_none()
        },
        "change approach never cleared the selection",
    )
    .await?;
    wait_options(&mut expert, &mut candidate, &mut state, 2).await?;
    key_preset(&mut expert, &mut candidate, &mut state, 2).await?;
    key_work(
        &mut expert,
        &mut candidate,
        &mut state,
        'f',
        "finish Q2",
        META_WAIT_SECS,
    )
    .await?;
    wait_options(&mut expert, &mut candidate, &mut state, 3).await?;
    println!("livesim Q2 finished");

    key_preset(&mut expert, &mut candidate, &mut state, 3).await?;
    apply_kind(&mut expert, &mut candidate, IntentKind::Skip, "Skip").await?;
    wait_options(&mut expert, &mut candidate, &mut state, 4).await?;
    chk(
        candidate
            .view()
            .nav
            .iter()
            .any(|n| n.index == 3 && n.state == "Skipped"),
        &format!("Q3 should stay skipped: {:?}", candidate.view().nav),
    )?;
    println!("livesim Q3 skipped");

    key_other(&mut expert, &mut candidate, &mut state, "").await?;
    key_work(
        &mut expert,
        &mut candidate,
        &mut state,
        't',
        "blank other tests",
        META_WAIT_SECS,
    )
    .await?;
    key_work(
        &mut expert,
        &mut candidate,
        &mut state,
        'f',
        "finish Q4",
        META_WAIT_SECS,
    )
    .await?;
    wait_options(&mut expert, &mut candidate, &mut state, 5).await?;
    println!("livesim Q4 finished");

    apply_kind(
        &mut expert,
        &mut candidate,
        IntentKind::Navigate { index: 8 },
        "Navigate 8",
    )
    .await?;
    wait_options(&mut expert, &mut candidate, &mut state, 8).await?;
    apply_kind(
        &mut expert,
        &mut candidate,
        IntentKind::Navigate { index: 5 },
        "Navigate 5",
    )
    .await?;
    wait_options(&mut expert, &mut candidate, &mut state, 5).await?;
    chk(
        candidate
            .view()
            .nav
            .iter()
            .any(|n| n.index == 3 && n.state == "Skipped"),
        &format!("navigate must not un-skip Q3: {:?}", candidate.view().nav),
    )?;
    println!("livesim navigate 8 then 5");

    apply_kind(
        &mut expert,
        &mut candidate,
        IntentKind::RequestReQuestion,
        "re-question reject",
    )
    .await?;
    expert
        .resolve_requestion(false, None, now_ms())
        .map_err(|e| format!("reject re-question: {e}"))?;
    wait_options(&mut expert, &mut candidate, &mut state, 5).await?;
    key_preset(&mut expert, &mut candidate, &mut state, 0).await?;
    key_work(
        &mut expert,
        &mut candidate,
        &mut state,
        'f',
        "finish Q5",
        META_WAIT_SECS,
    )
    .await?;
    wait_options(&mut expert, &mut candidate, &mut state, 6).await?;
    println!("livesim Q5 re-question rejected then finished");

    let replacement = unused_medium(expert.orchestrator().session())?;
    let old_options: Vec<String> = candidate
        .view()
        .options
        .iter()
        .map(|option| option.title.clone())
        .collect();
    apply_kind(
        &mut expert,
        &mut candidate,
        IntentKind::RequestReQuestion,
        "re-question approve",
    )
    .await?;
    expert
        .resolve_requestion(true, Some(replacement.clone()), now_ms())
        .map_err(|e| format!("approve re-question: {e}"))?;
    pump_until(
        &mut expert,
        &mut candidate,
        &mut state,
        META_WAIT_SECS,
        |expert, candidate, _| {
            expert
                .orchestrator()
                .session()
                .slots()
                .get(6)
                .is_some_and(|slot| slot.question.title_slug == replacement.title_slug)
                && candidate.view().question_index == Some(6)
                && candidate.view().selected.is_none()
                && candidate.view().options.len() == 4
                && candidate
                    .view()
                    .options
                    .iter()
                    .map(|option| option.title.clone())
                    .collect::<Vec<_>>()
                    != old_options
        },
        &format!(
            "approved replacement {} never loaded fresh approaches (old={old_options:?})",
            replacement.title_slug
        ),
    )
    .await?;
    key_preset(&mut expert, &mut candidate, &mut state, 1).await?;
    key_work(
        &mut expert,
        &mut candidate,
        &mut state,
        'f',
        "finish Q6",
        META_WAIT_SECS,
    )
    .await?;
    println!("livesim Q6 re-question approved then finished");

    apply_kind(
        &mut expert,
        &mut candidate,
        IntentKind::Disconnect,
        "Disconnect",
    )
    .await?;
    chk(
        !expert.orchestrator().is_connected(),
        "Disconnect must drop the live candidate",
    )?;

    let session = expert.orchestrator().session();
    chk(
        selected_presets(session) == BTreeSet::from([0, 1, 2, 3]),
        &format!(
            "candidate must try every approach option, got {:?}",
            selected_presets(session)
        ),
    )?;
    let others = other_texts(session);
    chk(
        others.iter().any(|t| t == "two pointers from both ends"),
        &format!("custom Other missing: {others:?}"),
    )?;
    chk(
        others.iter().any(|t| t.is_empty()),
        &format!("blank Other missing: {others:?}"),
    )?;

    let live = live_rating(session, &std::collections::BTreeMap::new());
    let monitor = expert.monitor_view(now_ms());
    chk(
        (3.0..=10.0).contains(&monitor.rating),
        &format!("live rating {}", monitor.rating),
    )?;
    chk(
        (3.0..=10.0).contains(&live.rating),
        &format!("live_rating {}", live.rating),
    )?;

    let poller = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            let _ = candidate.poll_snapshot();
            if candidate.view().final_breakdown.is_some() {
                break;
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        let rating = candidate.view().final_breakdown.as_ref().map(|f| f.rating);
        let lines = candidate.view().lines();
        let acked = candidate.final_ack_sent();
        (candidate, rating, lines, acked)
    });
    let report = expert
        .end_session(now_ms())
        .await
        .map_err(|e| format!("end_session: {e}"))?;
    println!(
        "livesim end_session attempted={:?} failed={:?}",
        report.attempted, report.failed
    );
    let (mut candidate, rating, lines, acked) = poller
        .join()
        .map_err(|_| "final poller panicked".to_string())?;
    if acked {
        let rating = rating.ok_or("candidate acked without a final rating")?;
        chk(rating.is_finite(), "final rating not finite")?;
        chk(
            lines.iter().any(|l| l.contains("final rating")),
            "candidate view lacks final rating line",
        )?;
        println!("livesim candidate final rating={rating:.2}");
    } else {
        println!(
            "livesim final ack timed out after Disconnect (expert waits 2s when the candidate is already gone)"
        );
    }
    let _ = candidate.end();

    let session = expert.orchestrator().session();
    chk(
        matches!(
            session.status(),
            SessionStatus::Completed | SessionStatus::EndedByExpert
        ),
        &format!("status {:?}", session.status()),
    )?;
    for name in [
        "SessionCreated",
        "CandidateConnected",
        "ApproachSelected",
        "TestsRequested",
        "QuestionFinished",
        "QuestionSkipped",
        "ReQuestionRequested",
        "ReQuestionResolved",
        "Continued",
        "Redirected",
        "ApproachChanged",
        "QuestionReopened",
        "CandidateDisconnected",
    ] {
        chk(
            has_kind(session, name),
            &format!("missing event {name}: {:?}", event_kinds(session)),
        )?;
    }
    chk(
        has_kind(session, "TestRunCompleted"),
        &format!(
            "missing TestRunCompleted after t then r: {:?}",
            event_kinds(session)
        ),
    )?;

    let report_md =
        std::fs::read_to_string(expert.layout().report_path()).map_err(|e| e.to_string())?;
    for heading in [
        "# PairProof Session Report",
        "## Final breakdown",
        "## Per-question grades",
    ] {
        chk(
            report_md.contains(heading),
            &format!("missing {heading} in {report_md}"),
        )?;
    }
    chk(
        !report_md.contains("pairproof:v1:"),
        "connection string leaked into report",
    )?;
    chk(
        report_md.contains("Final rating:"),
        "final rating missing from report",
    )?;
    let _ = std::fs::remove_dir_all(&base);
    Ok(())
}

fn assert_counted_tests(
    expert: &ExpertRuntime,
    candidate: &CandidateRuntime<SshCandidateTransport>,
    now: u64,
) -> Result<(), String> {
    let view = candidate.view();
    let latest = view.latest_test;
    let run = view.latest_run();
    let parsed = run.as_ref().and_then(|r| r.counted_tests());
    chk(
        latest.is_some(),
        &format!(
            "t then r left latest_test empty; parsed={parsed:?} run={:?} stdout={:?} stderr={:?}",
            run.as_ref()
                .map(|r| (r.kind.clone(), r.success, r.exit_code)),
            run.as_ref().map(|r| r.stdout.clone()),
            run.as_ref().map(|r| r.stderr.clone()),
        ),
    )?;
    if let (Some(test), Some((passed, failed))) = (latest, parsed) {
        chk(
            test.passed == passed && test.failed == failed,
            &format!(
                "snapshot latest_test {} passed {} failed != parsed ({passed}, {failed})",
                test.passed, test.failed
            ),
        )?;
    }
    let status = view.status_lines();
    chk(
        status
            .iter()
            .any(|line| line.starts_with("latest test:") && !line.contains("no test run yet")),
        &format!("candidate status still says no test run yet: {status:?}"),
    )?;
    let monitor = expert.monitor_view(now);
    chk(
        monitor.latest_test.is_some(),
        "expert monitor still says no test run yet after t then r",
    )?;
    Ok(())
}

fn assert_coach_is_question_local(
    candidate: &CandidateRuntime<SshCandidateTransport>,
    state: &CandidateRunState,
    index: usize,
    prior_replies: &[String],
) -> Result<(), String> {
    chk(
        candidate.view().chat_replies.is_empty(),
        &format!(
            "Q{} coach must start empty, got {:?}",
            index + 1,
            candidate.view().chat_replies
        ),
    )?;
    chk(
        candidate
            .view()
            .chat_replies
            .iter()
            .all(|reply| !prior_replies.contains(reply)),
        &format!(
            "Q{} coach leaked prior replies: current={:?} prior={prior_replies:?}",
            index + 1,
            candidate.view().chat_replies
        ),
    )?;
    chk(
        state.chat.sent_messages(Some(index)).is_empty(),
        &format!(
            "Q{} sent chat leaked {:?}",
            index + 1,
            state.chat.sent_messages(Some(index))
        ),
    )?;
    Ok(())
}
