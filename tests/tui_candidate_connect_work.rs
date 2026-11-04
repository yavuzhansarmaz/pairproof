use std::sync::{Arc, Barrier};

use pairproof::orchestration::control::{CandidateSnapshot, SnapshotOption, SnapshotQuestion};
use pairproof::tui::candidate_runtime::{
    live_waiting_for_question, spawn_connect_work, CandidateRuntimeError, CandidateView,
    ConnectOutcome,
};

#[test]
fn connect_work_runs_off_the_calling_thread() {
    let barrier = Arc::new(Barrier::new(2));
    let worker_barrier = Arc::clone(&barrier);
    let rx = spawn_connect_work(move || {
        worker_barrier.wait();
        ConnectOutcome::Failed(CandidateRuntimeError::Transport)
    });
    // Starting the work must return while it is still blocked, so
    // the TTY loop can keep drawing the animated loading screen.
    assert!(matches!(
        rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    barrier.wait();
    assert!(matches!(
        rx.recv(),
        Ok(ConnectOutcome::Failed(CandidateRuntimeError::Transport))
    ));
}

#[test]
fn rules_accept_shows_loading_until_statement_and_approaches_are_ready() {
    // Immediately after pressing Enter on Rules (`live == true`), before the
    // first background snapshot poll lands (`view == None`), show loading.
    assert!(live_waiting_for_question(true, false, false, None));
    assert!(!live_waiting_for_question(false, false, false, None));

    // Snapshot with no statement yet stays on the loading screen.
    let no_statement = CandidateView::from_snapshot(None);
    assert!(live_waiting_for_question(
        true,
        false,
        false,
        Some(&no_statement)
    ));

    // Statement without generated approaches yet stays on the loading screen.
    let snap_no_options = CandidateSnapshot {
        v: 1,
        session_id: "s1".to_string(),
        phase: "InProgress".to_string(),
        sync_phase: "Sync: active".to_string(),
        current: Some(SnapshotQuestion {
            index: 0,
            title: "Two Sum".to_string(),
            slug: "two-sum".to_string(),
            statement: Some("Find two numbers.".to_string()),
        }),
        options: Vec::new(),
        nav: Vec::new(),
        selected: None,
        latest_test: None,
        chat_replies: Vec::new(),
        final_breakdown: None,
        chat_partial: None,
        recovery: None,
    };
    let view_no_options = CandidateView::from_snapshot(Some(&snap_no_options));
    assert!(live_waiting_for_question(
        true,
        false,
        false,
        Some(&view_no_options)
    ));

    // Once statement and approaches arrive, loading ends and solving panes open.
    let mut snap_ready = snap_no_options;
    snap_ready.options.push(SnapshotOption {
        title: "Hash map".to_string(),
        summary: "One pass".to_string(),
    });
    let view_ready = CandidateView::from_snapshot(Some(&snap_ready));
    assert!(!live_waiting_for_question(
        true,
        false,
        false,
        Some(&view_ready)
    ));
}
