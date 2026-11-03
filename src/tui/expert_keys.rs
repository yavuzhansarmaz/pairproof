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

/// Starts a background pool fetch for a selection screen: the
/// runtime moves to a worker thread (the loading screen shows in
/// the meantime) and comes back through `PendingWork::selection_rx`.
fn start_selection_fetch(
    runtime: &mut Option<ExpertRuntime>,
    pending: &mut PendingWork,
    screen: SelectionScreen,
    now: u64,
) {
    let Some(mut active) = runtime.take() else {
        return;
    };
    let (tx, rx) = std::sync::mpsc::channel();
    pending.selection_rx = Some(rx);
    std::thread::spawn(move || {
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| err.to_string())
            .and_then(|rt| {
                rt.block_on(async {
                    match screen {
                        SelectionScreen::Medium => active.fetch_medium(now).await,
                        SelectionScreen::Hard => active.fetch_hard(now).await,
                    }
                    .map_err(|err| err.to_string())
                })
            });
        let _ = tx.send((active, screen, result));
    });
}

/// Selection list screen for the app screen, when one is showing.
pub(crate) fn selection_screen(app: &app::ExpertApp) -> Option<SelectionScreen> {
    match app.screen() {
        app::AppScreen::SelectMedium => Some(SelectionScreen::Medium),
        app::AppScreen::SelectHard => Some(SelectionScreen::Hard),
        _ => None,
    }
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
                    // The health check is a network call of up to 15
                    // seconds; it runs on a worker thread behind the
                    // loading screen instead of freezing this frame.
                    let (tx, rx) = std::sync::mpsc::channel();
                    pending.api_key_rx = Some(rx);
                    let key = api_key_input.clone();
                    std::thread::spawn(move || {
                        let result = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .map_err(|err| err.to_string())
                            .and_then(|rt| rt.block_on(expert_tty::check_api_key(&key)));
                        let _ = tx.send(result);
                    });
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
        K::Up if selection_screen(app).is_some() => {
            if let (Some(active), Some(screen)) = (runtime.as_mut(), selection_screen(app)) {
                active.move_selection_cursor(screen, -1);
            }
        }
        K::Down if selection_screen(app).is_some() => {
            if let (Some(active), Some(screen)) = (runtime.as_mut(), selection_screen(app)) {
                active.move_selection_cursor(screen, 1);
            }
        }
        K::Left if selection_screen(app).is_some() => {
            if let (Some(active), Some(screen)) = (runtime.as_mut(), selection_screen(app)) {
                active.move_selection_page(screen, -1);
            }
        }
        K::Right if selection_screen(app).is_some() => {
            if let (Some(active), Some(screen)) = (runtime.as_mut(), selection_screen(app)) {
                active.move_selection_page(screen, 1);
            }
        }
        K::Char(' ') if selection_screen(app).is_some() => {
            if let (Some(active), Some(screen)) = (runtime.as_mut(), selection_screen(app)) {
                if !active.toggle_selection_pick(screen) && active.has_options(screen) {
                    if active.auto_selected(screen) {
                        app.set_status_message(format!(
                            "only {} questions were found for this candidate; all of them must stay selected.",
                            active.option_count(screen)
                        ));
                    } else if active.pick_count(screen)
                        >= match screen {
                            SelectionScreen::Medium => 3,
                            SelectionScreen::Hard => 2,
                        }
                    {
                        app.set_status_message(match screen {
                            SelectionScreen::Medium => {
                                "3 medium questions are already selected; deselect one first."
                            }
                            SelectionScreen::Hard => {
                                "2 hard questions are already selected; deselect one first."
                            }
                        });
                    }
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
            // First Enter loads the list in the background (loading
            // screen); Enter with a list confirms the picks.
            if let Some(active) = runtime.as_mut() {
                if !active.has_options(SelectionScreen::Medium) {
                    if !active.selection_allowed(SelectionScreen::Medium, now_ms()) {
                        return;
                    }
                    start_selection_fetch(runtime, pending, SelectionScreen::Medium, now_ms());
                } else {
                    match active.confirm_medium() {
                        Ok(()) => {
                            app.clear_status_message();
                            let _ = app.handle(AppEvent::MediumSelected);
                        }
                        Err(err) => app.set_status_message(err.to_string()),
                    }
                }
            }
        }
        AppScreen::SelectHard => {
            // First Enter loads the list in the background (loading
            // screen); Enter with a list commits the picks.
            if let Some(active) = runtime.as_mut() {
                if !active.has_options(SelectionScreen::Hard) {
                    if !active.selection_allowed(SelectionScreen::Hard, now_ms()) {
                        return;
                    }
                    start_selection_fetch(runtime, pending, SelectionScreen::Hard, now_ms());
                } else {
                    match block_on(tokio_rt, active.complete_selection(now_ms())) {
                        Some(Ok(())) => {
                            app.set_expert_selection_done(true);
                            app.clear_status_message();
                            let _ = app.handle(AppEvent::ExpertSelectionDone);
                        }
                        Some(Err(err)) => app.set_status_message(err.to_string()),
                        None => set_error(app, "selection failed", "runtime unavailable"),
                    }
                }
            }
        }
        AppScreen::Monitor | AppScreen::RoleSelect | AppScreen::Exited => {}
        AppScreen::Final => _ = app.handle(AppEvent::Quit),
    }
}
