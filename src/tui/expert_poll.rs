//! Expert loop polling: finished background work and the
//! bounded per-tick runtime step.

use crate::tui::app;
use crate::tui::expert_loop::ExpertLoopState;
use crate::tui::expert_runtime::{ExpertRuntime, PollOutcome};
use crate::tui::expert_tty::{block_on, now_ms, set_error};

impl ExpertLoopState {
    /// Applies a finished background proposal request, if one landed.
    pub(crate) fn poll_proposal(&mut self, app: &mut app::ExpertApp) {
        let Some(rx) = &self.pending.proposal_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(proposed)) => {
                self.pending.proposal_rx = None;
                self.pastes.apply_proposal(proposed);
                app.set_proposal_lines(self.pastes.proposal_lines());
                app.set_has_proposal(true);
                app.clear_status_message();
                let _ = app.handle(app::AppEvent::ProposalReady);
            }
            Ok(Err(err)) => {
                self.pending.proposal_rx = None;
                set_error(app, "proposal failed", &err);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.pending.proposal_rx = None;
                set_error(app, "proposal failed", "request stopped unexpectedly");
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    /// Applies a finished background runtime build, if one landed:
    /// storing it confirms the proposal and waits for the candidate.
    pub(crate) fn poll_runtime_build(&mut self, app: &mut app::ExpertApp) {
        let Some(rx) = &self.pending.runtime_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(built)) => {
                self.pending.runtime_rx = None;
                self.runtime = Some(built);
                let _ = app.handle(app::AppEvent::ProposalConfirmed);
            }
            Ok(Err(err)) => {
                self.pending.runtime_rx = None;
                set_error(app, "session setup failed", &err);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.pending.runtime_rx = None;
                set_error(app, "session setup failed", "runtime unavailable");
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }
}

/// Bounded per-tick runtime step: poll for the candidate hello on
/// the connection screen, drain intents once the session is live.
pub(crate) fn tick_runtime(
    app: &mut app::ExpertApp,
    runtime: &mut Option<ExpertRuntime>,
    tokio_rt: &mut Option<tokio::runtime::Runtime>,
) {
    use app::{AppEvent, AppScreen};
    let Some(active) = runtime.as_mut() else {
        return;
    };
    match app.screen() {
        AppScreen::ConnectionWait => {
            let _ = block_on(tokio_rt, active.refresh_pre_lock_secret(now_ms()));
            let _ = block_on(tokio_rt, active.poll_recovery(now_ms()));
            match block_on(tokio_rt, active.poll_candidate(now_ms())) {
                Some(PollOutcome::Connected) => {
                    app.set_connected(true);
                    let _ = app.handle(AppEvent::ConnectionLocked);
                }
                Some(PollOutcome::VersionMismatch) => {
                    if let Some((c, e)) = active.version_mismatch() {
                        app.set_status_message(format!("Version mismatch: candidate {c}, you {e}. Candidate must update/restart PairProof; session not started."));
                    }
                }
                _ => {}
            }
        }
        AppScreen::SelectMedium | AppScreen::SelectHard | AppScreen::Monitor => {
            let _ = block_on(tokio_rt, active.refresh_pre_lock_secret(now_ms()));
            let outcome = block_on(tokio_rt, active.poll_candidate(now_ms()));
            let _ = block_on(tokio_rt, active.poll_recovery(now_ms()));
            // drain_intents already checks the heartbeat before and
            // after its drain; a third read per tick only adds lag.
            let _ = block_on(tokio_rt, active.drain_intents(now_ms()));
            if active.is_disconnected() {
                let _ = app.handle(AppEvent::AccidentalDisconnect);
            }
            if outcome == Some(PollOutcome::Connected) && app.is_disconnected() {
                let _ = app.handle(AppEvent::CandidateReconnected);
                app.clear_status_message();
            }
        }
        _ => {}
    }
}

impl ExpertLoopState {
    /// Starts the statement fetch for a freshly opened question
    /// preview, unless that statement is already cached.
    pub(crate) fn start_question_preview_fetch(
        &mut self,
        screen: crate::tui::expert_runtime::SelectionScreen,
    ) {
        if self.pending.preview_rx.is_some() {
            return;
        }
        let Some(active) = self.runtime.as_ref() else {
            return;
        };
        let Some((fetcher, slug, language)) = active.preview_request(screen) else {
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        self.pending.preview_rx = Some(rx);
        std::thread::spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|err| err.to_string())
                .and_then(|rt| {
                    rt.block_on(async {
                        fetcher
                            .fetch_statement(&slug, language)
                            .await
                            .map(|fetched| crate::tui::shared::html_to_text(&fetched.statement))
                            .map_err(|err| err.to_string())
                    })
                });
            let _ = tx.send((screen, slug, result));
        });
    }

    /// Caches a finished preview statement on the runtime.
    pub(crate) fn poll_question_preview(&mut self) {
        let Some(rx) = &self.pending.preview_rx else {
            return;
        };
        match rx.try_recv() {
            Ok((screen, slug, result)) => {
                self.pending.preview_rx = None;
                if let Some(active) = self.runtime.as_mut() {
                    let text =
                        result.unwrap_or_else(|_| "the question could not be loaded.".to_string());
                    active.store_preview(screen, slug, text);
                }
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.pending.preview_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    /// Applies a finished background pool fetch; a fully auto-picked
    /// small pool is announced so it is not mistaken for breakage.
    pub(crate) fn poll_selection_fetch(&mut self, app: &mut app::ExpertApp) {
        let Some(rx) = &self.pending.selection_rx else {
            return;
        };
        match rx.try_recv() {
            Ok((active, screen, result)) => {
                self.pending.selection_rx = None;
                match result {
                    Ok(_) => {
                        if active.auto_selected(screen) {
                            let label = match screen {
                                crate::tui::expert_runtime::SelectionScreen::Medium => "medium",
                                crate::tui::expert_runtime::SelectionScreen::Hard => "hard",
                            };
                            app.set_status_message(format!(
                                "only {} {label} questions were found for this candidate, so all of them are selected automatically.",
                                active.option_count(screen)
                            ));
                        } else if let Some(note) = active.selection_note() {
                            app.set_status_message(note);
                        } else {
                            app.clear_status_message();
                        }
                    }
                    Err(err) => set_error(app, "selection failed", &err),
                }
                self.runtime = Some(active);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.pending.selection_rx = None;
                set_error(app, "selection failed", "selection worker stopped");
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }
}
