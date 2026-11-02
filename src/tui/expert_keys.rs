//! Expert key and paste handling for the interactive loop:
//! API-key entry, disclosure acknowledgement, and Enter actions.

use crate::tui::app;
use crate::tui::expert_loop::PendingWork;
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
                "job description pasted. press enter to continue."
            }
            ExpertSetupFocus::AdditionalNotes => {
                "additional notes pasted. press enter to review the proposal."
            }
        }),
        Err(reason) => app.set_status_message(reason),
    }
}

/// Opens masked API-key entry on Expert Setup so a mistyped key can
/// be replaced; an env-provided key is changed in the environment.
fn open_api_key_change(app: &mut app::ExpertApp) {
    if crate::tui::config::env_override_active() {
        app.set_status_message(
            "Meta API key comes from the MODEL_API_KEY environment variable; change it there.",
        );
        return;
    }
    app.set_api_key_entry(true);
    app.set_status_message("type the new Meta API key, then press enter to save it.");
}

/// Maps one key press: app events before the runtime exists, real
/// runtime operations afterwards (`e` ends a live session).
pub(crate) fn handle_key(
    app: &mut app::ExpertApp,
    runtime: &mut Option<ExpertRuntime>,
    tokio_rt: &mut Option<tokio::runtime::Runtime>,
    pastes: &mut ExpertSetupPastes,
    pending: &mut PendingWork,
    api_key_input: &mut String,
    code: crossterm::event::KeyCode,
) {
    use app::{AppEvent, AppScreen};
    use crossterm::event::KeyCode as K;
    if app.api_key_entry && app.screen() == AppScreen::Preflight {
        match code {
            K::Esc => _ = app.handle(AppEvent::Quit),
            K::Enter => {
                if api_key_input.trim().is_empty() {
                    expert_tty::save_entered_api_key(app, api_key_input);
                } else {
                    let checked = block_on(tokio_rt, expert_tty::check_api_key(api_key_input));
                    match checked {
                        Some(result) => {
                            expert_tty::save_checked_api_key(app, api_key_input, result)
                        }
                        None => set_error(
                            app,
                            "API key check failed; key not saved",
                            "runtime unavailable",
                        ),
                    }
                }
            }
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
        K::Backspace if app.screen() == AppScreen::Setup => pastes.backspace_focused(),
        K::Up if app.screen() == AppScreen::Setup => pastes.scroll_focused(-1),
        K::Down if app.screen() == AppScreen::Setup => pastes.scroll_focused(1),
        K::Char('b') => {
            // Leaving Session Setup backwards abandons the draft:
            // re-entering starts with both boxes empty.
            let was_setup = app.screen() == AppScreen::Setup;
            let _ = app.handle(AppEvent::Back);
            if was_setup && app.screen() != AppScreen::Setup {
                *pastes = ExpertSetupPastes::new();
                app.set_has_proposal(false);
            }
        }
        K::Char('m') if app.screen() == AppScreen::Preflight => open_api_key_change(app),
        K::Char('m') if app.screen() == AppScreen::Setup => {
            if app.handle(AppEvent::Back).is_ok() && app.screen() == AppScreen::Preflight {
                open_api_key_change(app);
            }
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
                        app.set_final_lines(active.final_lines());
                        app.clear_status_message();
                        let _ = app.handle(AppEvent::EndSession);
                    }
                    Some(Err(err)) => set_error(app, "end failed", &err.to_string()),
                    None => set_error(app, "end failed", "runtime unavailable"),
                }
            }
        }
        K::Char('R') if app.screen() == AppScreen::Monitor => {
            if let Some(active) = runtime.as_mut() {
                match block_on(tokio_rt, active.reissue(now_ms())) {
                    Some(Ok(_fresh)) => app.set_status_message("fresh string ready."),
                    Some(Err(err)) => set_error(app, "reissue failed", &err.to_string()),
                    None => set_error(app, "reissue failed", "runtime unavailable"),
                }
            }
        }
        K::Enter => advance(app, runtime, tokio_rt, pastes, pending),
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
    pending: &mut PendingWork,
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
                        "additional notes are optional. paste notes or press enter to skip.",
                    );
                } else {
                    app.set_status_message("paste the job description first.");
                }
            }
            ExpertSetupFocus::AdditionalNotes => {
                pastes.acknowledge_disclosure();
                if pending.proposal_rx.is_some() {
                    return;
                }
                // Run the Meta request on its own thread so the TTY
                // loop keeps drawing the animated loading screen;
                // `poll_proposal` applies the result when it lands.
                let (tx, rx) = std::sync::mpsc::channel();
                pending.proposal_rx = Some(rx);
                let jd = pastes.jd().to_string();
                let notes = pastes.notes().to_string();
                std::thread::spawn(move || {
                    let result = (|| {
                        let runtime = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .map_err(|e| e.to_string())?;
                        runtime.block_on(async {
                            let client = expert_runtime::meta_client()?;
                            let mut detached = ExpertSetupPastes::from_texts(&jd, &notes);
                            detached.acknowledge_disclosure();
                            detached
                                .request_proposal(&client)
                                .await
                                .map_err(|e| e.to_string())
                        })
                    })();
                    let _ = tx.send(result);
                });
            }
        },
        AppScreen::Proposal => {
            if runtime.is_some() {
                let _ = app.handle(AppEvent::ProposalConfirmed);
                return;
            }
            if pending.runtime_rx.is_some() {
                return;
            }
            // Build the runtime on its own thread so the TTY loop
            // keeps drawing the animated loading screen;
            // `poll_runtime_build` stores it and confirms.
            let (tx, rx) = std::sync::mpsc::channel();
            pending.runtime_rx = Some(rx);
            let jd = pastes.jd().to_string();
            let notes = pastes.notes().to_string();
            let proposal = pastes.proposal().cloned();
            let now = now_ms();
            std::thread::spawn(move || {
                let result = (|| {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| e.to_string())?;
                    runtime.block_on(expert_runtime::build_expert_runtime(
                        now,
                        &jd,
                        &notes,
                        proposal.as_ref(),
                    ))
                })();
                let _ = tx.send(result);
            });
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
                    Some(Err(err)) => set_error(app, "selection failed", &err.to_string()),
                    None => set_error(app, "selection failed", "runtime unavailable"),
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
                    Some(Err(err)) => set_error(app, "selection failed", &err.to_string()),
                    None => set_error(app, "selection failed", "runtime unavailable"),
                }
            }
        }
        AppScreen::Monitor | AppScreen::RoleSelect | AppScreen::Exited => {}
        AppScreen::Final => _ = app.handle(AppEvent::Quit),
    }
}
