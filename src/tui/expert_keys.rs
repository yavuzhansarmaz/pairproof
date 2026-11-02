//! Expert key and paste handling for the interactive loop:
//! API-key entry, disclosure acknowledgement, and Enter actions.

use crate::tui::app;
use crate::tui::expert_runtime::{self, ExpertRuntime, SelectionScreen};
use crate::tui::expert_tty::{self, block_on, now_ms, set_error};
use crate::tui::setup::{ExpertSetupFocus, ExpertSetupPastes};
use crate::tui::tick_runtime;

/// Feeds a bracketed paste on Setup into the collector; ignored elsewhere.
/// The only path that sets `has_proposal`, with progress on the status line.
#[rustfmt::skip]
pub(crate) fn handle_paste(
    app: &mut app::ExpertApp,
    pastes: &mut ExpertSetupPastes,
    api_key_input: &mut String,
    text: &str,
) {
    if app.api_key_entry && app.screen() == app::AppScreen::Preflight {
        *api_key_input = text.lines().next().unwrap_or("").trim().to_string();
        return;
    }
    if app.screen() != app::AppScreen::Setup { return; }
    match pastes.apply_focused_paste(text) {
        Ok(()) => app.set_status_message(match pastes.focus() {
            ExpertSetupFocus::JobDescription => {
                "Job description pasted. Press Enter to continue."
            }
            ExpertSetupFocus::AdditionalNotes => {
                "Additional notes pasted. Press Enter to review the proposal."
            }
        }),
        Err(reason) => app.set_status_message(reason),
    }
}

/// Maps one key press: app events before the runtime exists, real
/// runtime operations afterwards (`e` ends a live session).
pub(crate) fn handle_key(
    app: &mut app::ExpertApp,
    runtime: &mut Option<ExpertRuntime>,
    tokio_rt: &mut Option<tokio::runtime::Runtime>,
    pastes: &mut ExpertSetupPastes,
    preflight_done: &mut bool,
    api_key_input: &mut String,
    code: crossterm::event::KeyCode,
) {
    use app::{AppEvent, AppScreen};
    use crossterm::event::KeyCode as K;
    if app.api_key_entry && app.screen() == AppScreen::Preflight {
        match code {
            K::Esc => _ = app.handle(AppEvent::Quit),
            K::Enter => expert_tty::save_entered_api_key(app, api_key_input),
            K::Backspace => {
                api_key_input.pop();
            }
            K::Char(c) if api_key_input.chars().count() < 512 => {
                api_key_input.push(c);
            }
            _ => {}
        }
        return;
    }
    match code {
        K::Esc => _ = app.handle(AppEvent::Quit),
        K::Char('b')
            if app.screen() == AppScreen::Setup
                && pastes.focus() == ExpertSetupFocus::AdditionalNotes =>
        {
            pastes.back_to_job_description()
        }
        K::Up if app.screen() == AppScreen::Setup => pastes.scroll_focused(-1),
        K::Down if app.screen() == AppScreen::Setup => pastes.scroll_focused(1),
        K::Char('b') => _ = app.handle(AppEvent::Back),
        K::Char('r') => {
            if expert_tty::preflight_retry_requested(app.screen(), code) {
                *preflight_done = false;
            }
            _ = app.handle(AppEvent::Retry)
        }
        K::Char('e') if app.screen() == AppScreen::RoleSelect => {
            _ = app.handle(AppEvent::SelectExpert)
        }
        K::Char('c') if app.screen() == AppScreen::RoleSelect => {
            _ = app.handle(AppEvent::SelectCandidate)
        }
        K::Char('e') if app.screen() == AppScreen::Monitor => {
            if let Some(active) = runtime.as_mut() {
                match block_on(tokio_rt, active.end_session(now_ms())) {
                    Some(Ok(_)) => {
                        app.clear_status_message();
                        let _ = app.handle(AppEvent::EndSession);
                    }
                    Some(Err(err)) => set_error(app, "End failed", &err.to_string()),
                    None => set_error(app, "End failed", "runtime unavailable"),
                }
            }
        }
        K::Char('R') if app.screen() == AppScreen::Monitor => {
            if let Some(active) = runtime.as_mut() {
                match block_on(tokio_rt, active.reissue(now_ms())) {
                    Some(Ok(fresh)) => {
                        app.set_status_message(format!("Fresh string ready: {}", fresh.encode()))
                    }
                    Some(Err(err)) => set_error(app, "Reissue failed", &err.to_string()),
                    None => set_error(app, "Reissue failed", "runtime unavailable"),
                }
            }
        }
        K::Char('a') if app.screen() == AppScreen::Preflight => {
            let _ = app.handle(AppEvent::DisclosureAcknowledged);
            if app.disclosure_acknowledged {
                expert_tty::save_disclosure_ack_default();
            }
        }
        K::Enter => advance(app, runtime, tokio_rt, pastes),
        _ => {}
    }
}

/// Enter is the context action for the current screen. Pre-runtime
/// screens keep their controller-checked transitions; building the
/// runtime is the Proposal action, and afterwards Enter only polls,
/// fetches real questions through the runtime, or does nothing.
fn advance(
    app: &mut app::ExpertApp,
    runtime: &mut Option<ExpertRuntime>,
    tokio_rt: &mut Option<tokio::runtime::Runtime>,
    pastes: &mut ExpertSetupPastes,
) {
    use app::{AppEvent, AppScreen};
    match app.screen() {
        AppScreen::Preflight => {
            if !app.disclosure_acknowledged {
                let _ = app.handle(AppEvent::DisclosureAcknowledged);
                if app.disclosure_acknowledged {
                    expert_tty::save_disclosure_ack_default();
                }
            } else if !app.api_key_ready {
                app.set_api_key_entry(true);
            } else {
                let _ = app.handle(AppEvent::PreflightCompleted);
                if app.screen() == AppScreen::Setup {
                    app.clear_status_message();
                }
            }
        }
        AppScreen::Setup => match pastes.focus() {
            ExpertSetupFocus::JobDescription => {
                if pastes.confirm_job_description() {
                    app.set_has_proposal(true);
                    app.set_status_message(
                        "Additional notes are optional. Paste notes or press Enter to skip.",
                    );
                } else {
                    app.set_status_message("Paste the job description first.");
                }
            }
            ExpertSetupFocus::AdditionalNotes => {
                app.set_has_proposal(true);
                let _ = app.handle(AppEvent::ProposalReady);
            }
        },
        AppScreen::Proposal => {
            if runtime.is_none() {
                let build =
                    expert_runtime::build_expert_runtime(now_ms(), pastes.jd(), pastes.notes());
                match block_on(tokio_rt, build) {
                    Some(Ok(built)) => *runtime = Some(built),
                    Some(Err(err)) => {
                        set_error(app, "Session setup failed", &err.to_string());
                        return;
                    }
                    None => {
                        set_error(app, "Session setup failed", "runtime unavailable");
                        return;
                    }
                }
            }
            let _ = app.handle(AppEvent::ProposalConfirmed);
        }
        AppScreen::ConnectionWait => {
            // Poll only: the lock comes from poll_candidate, never Enter.
            tick_runtime(app, runtime, tokio_rt);
        }
        AppScreen::SelectMedium => {
            if let Some(active) = runtime.as_mut() {
                if !active.selection_allowed(SelectionScreen::Medium, now_ms()) {
                    return;
                }
                match block_on(tokio_rt, active.fetch_medium(now_ms())) {
                    Some(Ok(_)) => {
                        if let Some(note) = active.selection_note() {
                            app.set_status_message(note);
                        }
                        let _ = app.handle(AppEvent::MediumSelected);
                    }
                    Some(Err(err)) => set_error(app, "Selection failed", &err.to_string()),
                    None => set_error(app, "Selection failed", "runtime unavailable"),
                }
            }
        }
        AppScreen::SelectHard => {
            if let Some(active) = runtime.as_mut() {
                if !active.selection_allowed(SelectionScreen::Hard, now_ms()) {
                    return;
                }
                match block_on(tokio_rt, active.complete_selection(now_ms())) {
                    Some(Ok(())) => {
                        if let Some(note) = active.selection_note() {
                            app.set_status_message(note);
                        }
                        app.set_expert_selection_done(true);
                        let _ = app.handle(AppEvent::ExpertSelectionDone);
                    }
                    Some(Err(err)) => set_error(app, "Selection failed", &err.to_string()),
                    None => set_error(app, "Selection failed", "runtime unavailable"),
                }
            }
        }
        AppScreen::Monitor | AppScreen::RoleSelect | AppScreen::Exited => {}
        AppScreen::Final => _ = app.handle(AppEvent::Quit),
    }
}
