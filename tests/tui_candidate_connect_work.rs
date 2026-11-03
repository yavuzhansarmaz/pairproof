use std::sync::{Arc, Barrier};

use pairproof::tui::candidate_runtime::{
    spawn_connect_work, CandidateRuntimeError, ConnectOutcome,
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
