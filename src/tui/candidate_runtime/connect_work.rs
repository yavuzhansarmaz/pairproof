//! Background candidate connection work: the blocking SSH
//! verification and runtime build run here, off the TTY thread, so
//! the loading screen keeps animating instead of freezing on its
//! first frame (SSH probes up to 20s each, Mutagen sync up to 60s).

use super::*;

/// Outcome of the background connection work.
pub enum ConnectOutcome {
    /// Parsing, expiry, or SSH verification failed.
    SubmitFailed(CandidateConnectError),
    /// Verification passed but the runtime build/reconnect failed.
    Failed(CandidateRuntimeError),
    /// Verified and connected; ready to show the Rules screen.
    Connected(Box<CandidateRuntime<SshCandidateTransport>>),
}

/// Runs `work` on a worker thread and returns its result channel;
/// the caller keeps drawing (and animating) until it polls a result.
pub fn spawn_connect_work<F>(work: F) -> std::sync::mpsc::Receiver<ConnectOutcome>
where
    F: FnOnce() -> ConnectOutcome + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx
}

/// The blocking connection sequence, run on the worker thread:
/// parse and expiry-check, SSH-verify into the stash, then build or
/// reconnect the runtime from the verified material.
pub(crate) fn run_connect_work(
    input: String,
    now_ms: u64,
    stash: Arc<Mutex<Option<(ConnectionString, String)>>>,
    existing: Option<CandidateRuntime<SshCandidateTransport>>,
) -> ConnectOutcome {
    let parsed = match ConnectionString::parse(input.trim()) {
        Ok(value) => value,
        Err(_) => return ConnectOutcome::SubmitFailed(CandidateConnectError::Malformed),
    };
    if now_ms >= parsed.expires_at_ms() {
        return ConnectOutcome::SubmitFailed(CandidateConnectError::Expired);
    }
    if let Err(err) = super::transport::verify_and_stash(&parsed, &stash) {
        return ConnectOutcome::SubmitFailed(match err {
            CandidateConnectError::Unavailable => CandidateConnectError::Unavailable,
            _ => CandidateConnectError::Rejected,
        });
    }
    run_runtime_build(existing, stash)
}

/// Builds or reconnects the runtime from already-verified material,
/// run on the worker thread.
pub(crate) fn run_runtime_build(
    existing: Option<CandidateRuntime<SshCandidateTransport>>,
    stash: Arc<Mutex<Option<(ConnectionString, String)>>>,
) -> ConnectOutcome {
    let material = stash.lock().ok().and_then(|slot| slot.clone());
    match super::build_or_reconnect_runtime(existing, material) {
        Ok(runtime) => ConnectOutcome::Connected(Box::new(runtime)),
        Err(err) => ConnectOutcome::Failed(err),
    }
}
