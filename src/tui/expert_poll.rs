//! Expert loop polling: finished background work and the
//! bounded per-tick runtime step.

use crate::tui::app;
use crate::tui::expert_keys::{selection_screen, start_selection_fetch};
use crate::tui::expert_loop::ExpertLoopState;
use crate::tui::expert_runtime::{ExpertRuntime, PollOutcome, SelectionScreen};
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

    /// Applies a finished background session end. Success opens the
    /// final breakdown; failure returns to the live monitor.
    pub(crate) fn poll_end_session(&mut self, app: &mut app::ExpertApp) {
        let Some(rx) = &self.pending.end_rx else {
            return;
        };
        match rx.try_recv() {
            Ok((active, Ok(()))) => {
                self.pending.end_rx = None;
                app.set_final_lines(active.final_lines());
                app.clear_status_message();
                self.runtime = Some(active);
                let _ = app.handle(app::AppEvent::EndSession);
            }
            Ok((active, Err(err))) => {
                self.pending.end_rx = None;
                self.runtime = Some(active);
                set_error(app, "end failed", &err);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.pending.end_rx = None;
                set_error(app, "end failed", "runtime unavailable");
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }
}

/// Whether entering `screen` should auto-start its pool fetch.
pub(crate) fn should_auto_start_selection(
    attempted: Option<SelectionScreen>,
    screen: SelectionScreen,
    has_options: bool,
    fetch_pending: bool,
    allowed: bool,
) -> bool {
    attempted != Some(screen) && !has_options && !fetch_pending && allowed
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
    /// Scrolls the final breakdown by `delta` rows, clamped to
    /// the range the last frame could actually display.
    pub(crate) fn scroll_final_by(&mut self, delta: i32) {
        if delta >= 0 {
            self.final_scroll = self
                .final_scroll
                .saturating_add(delta as usize)
                .min(self.final_max_scroll);
        } else {
            self.final_scroll = self
                .final_scroll
                .saturating_sub(delta.unsigned_abs() as usize);
        }
    }

    /// Scrolls the question preview by `delta` rows, clamped to
    /// the range the last frame could actually display.
    pub(crate) fn scroll_preview_by(&mut self, delta: i32) {
        if delta >= 0 {
            self.question_preview_scroll = self
                .question_preview_scroll
                .saturating_add(delta as usize)
                .min(self.question_preview_max_scroll);
        } else {
            self.question_preview_scroll = self
                .question_preview_scroll
                .saturating_sub(delta.unsigned_abs() as usize);
        }
    }

    /// Steps the preview to a neighbouring question: the scroll
    /// resets to the top and the new statement fetch starts, so
    /// the preview shows the loading line until it lands.
    pub(crate) fn step_preview_question(&mut self, app: &app::ExpertApp, delta: i32) {
        let Some(screen) = selection_screen(app) else {
            return;
        };
        let moved = match self.runtime.as_mut() {
            Some(active) => active.step_preview_question(screen, delta),
            None => false,
        };
        if moved {
            self.question_preview_scroll = 0;
            self.question_preview_max_scroll = usize::MAX;
            self.start_question_preview_fetch(screen);
        }
    }

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
                    match result {
                        Ok(text) => active.store_preview(screen, slug, text),
                        Err(reason) => active.store_preview_error(screen, slug, reason),
                    }
                }
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.pending.preview_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    /// Auto-starts the pool fetch once per selection-screen entry.
    pub(crate) fn maybe_auto_start_selection_fetch(&mut self, app: &mut app::ExpertApp) {
        let Some(screen) = selection_screen(app) else {
            return;
        };
        let (has_options, allowed) = match self.runtime.as_ref() {
            Some(active) => (
                active.has_options(screen),
                active.selection_allowed(screen, now_ms()),
            ),
            None => return,
        };
        if should_auto_start_selection(
            self.auto_load_attempted,
            screen,
            has_options,
            self.pending.selection_rx.is_some(),
            allowed,
        ) {
            self.auto_load_attempted = Some(screen);
            start_selection_fetch(&mut self.runtime, &mut self.pending, screen, now_ms());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_when_fresh() {
        assert!(should_auto_start_selection(
            None,
            SelectionScreen::Medium,
            false,
            false,
            true
        ));
    }

    #[test]
    fn does_not_start_when_already_attempted() {
        assert!(!should_auto_start_selection(
            Some(SelectionScreen::Medium),
            SelectionScreen::Medium,
            false,
            false,
            true
        ));
    }

    #[test]
    fn does_not_start_when_options_pending_or_disallowed() {
        assert!(!should_auto_start_selection(
            None,
            SelectionScreen::Medium,
            true,
            false,
            true
        ));
        assert!(!should_auto_start_selection(
            None,
            SelectionScreen::Medium,
            false,
            true,
            true
        ));
        assert!(!should_auto_start_selection(
            None,
            SelectionScreen::Medium,
            false,
            false,
            false
        ));
    }

    #[test]
    fn different_screen_may_start() {
        assert!(should_auto_start_selection(
            Some(SelectionScreen::Medium),
            SelectionScreen::Hard,
            false,
            false,
            true
        ));
    }
}
