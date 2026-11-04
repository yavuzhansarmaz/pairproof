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

/// Result of one background snapshot poll off the TTY thread.
pub(crate) struct BackgroundPollOutcome {
    pub snapshot: Result<Option<CandidateSnapshot>, CandidateRuntimeError>,
    pub ack_written: bool,
    pub synced_revision: Option<(usize, u64)>,
    pub sync_failed: Option<bool>,
}

impl CandidateRuntime<SshCandidateTransport> {
    /// Spawns a non-blocking heartbeat write on a worker thread (e.g. while on Rules).
    pub(crate) fn spawn_background_heartbeat(&self, now_ms: u64) {
        if self.phase != CandidateRuntimePhase::Connected {
            return;
        }
        let Some((conn, key_line, _, _)) = self.transport.poll_parts() else {
            return;
        };
        let session_id = self.session_id.clone();
        std::thread::spawn(move || {
            let body = heartbeat_bytes(&session_id, now_ms);
            let _ = run_ssh_command_with_stdin(
                conn.endpoint(),
                conn.username(),
                conn.secret(),
                &key_line,
                &["tee", HEARTBEAT_PATH],
                Some(body.as_slice()),
                SSH_TIMEOUT,
            );
        });
    }

    /// Spawns one snapshot + heartbeat + workspace-sync poll on a worker thread.
    pub(crate) fn spawn_background_poll(
        &mut self,
        now_ms: u64,
    ) -> Option<std::sync::mpsc::Receiver<BackgroundPollOutcome>> {
        if self.phase != CandidateRuntimePhase::Connected {
            return None;
        }
        let (conn, key_line, client, sync_env) = self.transport.poll_parts()?;
        let send_hb = now_ms.saturating_sub(self.last_hb_ms) > HEARTBEAT_THROTTLE_MS;
        if send_hb {
            self.last_hb_ms = now_ms;
        }
        let session_id = self.session_id.clone();
        let workspace = self.workspace.clone();
        let ack_sent = self.ack_sent;
        let synced_revisions = self.synced_revisions.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let snapshot = match client.fetch_snapshot() {
                Ok(snap) => Ok(Some(snap)),
                Err(crate::orchestration::control::ControlError::WrongSession) => {
                    Err(CandidateRuntimeError::WrongSession)
                }
                Err(_) => Ok(None),
            };
            if send_hb {
                let body = heartbeat_bytes(&session_id, now_ms);
                let _ = run_ssh_command_with_stdin(
                    conn.endpoint(),
                    conn.username(),
                    conn.secret(),
                    &key_line,
                    &["tee", HEARTBEAT_PATH],
                    Some(body.as_slice()),
                    SSH_TIMEOUT,
                );
            }
            let mut ack_written = false;
            let mut synced_revision = None;
            let mut sync_failed = None;
            if let Ok(Some(ref snap)) = snapshot {
                if snap.session_id == session_id {
                    if matches!(snap.phase.as_str(), "WaitingForCandidate" | "Setup") {
                        let body = hello_bytes(&session_id, env!("CARGO_PKG_VERSION"));
                        let _ = run_ssh_command_with_stdin(
                            conn.endpoint(),
                            conn.username(),
                            conn.secret(),
                            &key_line,
                            &["tee", HELLO_PATH],
                            Some(body.as_slice()),
                            SSH_TIMEOUT,
                        );
                    }
                    if snap.final_breakdown.is_some() && !ack_sent {
                        let body = final_ack_bytes(&session_id);
                        let _ = run_ssh_command_with_stdin(
                            conn.endpoint(),
                            conn.username(),
                            conn.secret(),
                            &key_line,
                            &["tee", FINAL_ACK_PATH],
                            Some(body.as_slice()),
                            SSH_TIMEOUT,
                        );
                        ack_written = true;
                    }
                    if let Some(current) = &snap.current {
                        let index = current.index;
                        let rev = snap.code_revision();
                        let target_rev = if rev > 0 {
                            rev
                        } else {
                            u64::from(snap.selected.is_some())
                        };
                        if target_rev > 0 {
                            let local_ok =
                                crate::tui::candidate_sync::has_synced_solution(&workspace, index);
                            if synced_revisions.get(&index).copied().unwrap_or(0) < target_rev
                                || !local_ok
                            {
                                let flush_res = match sync_env.as_ref() {
                                    Some(env) => {
                                        super::transport::flush_sync_with_env(env, &session_id)
                                    }
                                    None => Ok(()),
                                };
                                match flush_res {
                                    Ok(()) => {
                                        if crate::tui::candidate_sync::has_synced_solution(
                                            &workspace, index,
                                        ) {
                                            synced_revision = Some((index, target_rev));
                                            sync_failed = Some(false);
                                        }
                                    }
                                    Err(_) => {
                                        sync_failed = Some(true);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            let _ = tx.send(BackgroundPollOutcome {
                snapshot,
                ack_written,
                synced_revision,
                sync_failed,
            });
        });
        Some(rx)
    }

    /// Applies a completed background poll outcome on the TTY thread without blocking.
    pub(crate) fn apply_background_poll(
        &mut self,
        outcome: BackgroundPollOutcome,
    ) -> Result<Option<CandidateSnapshot>, CandidateRuntimeError> {
        if outcome.ack_written {
            self.ack_sent = true;
        }
        if let Some((index, rev)) = outcome.synced_revision {
            self.synced_revisions.insert(index, rev);
        }
        if let Some(failed) = outcome.sync_failed {
            self.sync_failed = failed;
        }
        match outcome.snapshot {
            Err(CandidateRuntimeError::Transport) => {
                self.lost = true;
                Err(CandidateRuntimeError::Transport)
            }
            Err(err) => Err(err),
            Ok(None) => Ok(None),
            Ok(Some(snapshot)) => {
                if snapshot.session_id != self.session_id {
                    return Err(CandidateRuntimeError::WrongSession);
                }
                self.lost = false;
                match &snapshot.recovery {
                    None => self.recovery_view = None,
                    Some(r) if r.generation >= self.last_recovery_generation => {
                        self.last_recovery_generation = r.generation;
                        self.recovery_view = Some(r.clone());
                    }
                    Some(_) => {}
                }
                self.last_snapshot = Some(snapshot.clone());
                Ok(Some(snapshot))
            }
        }
    }
}
