//! Terminal startup, role dispatch, and expert and candidate screens.
pub mod app;
pub mod args;
pub mod candidate;
pub mod candidate_actions;
pub mod candidate_chat;
pub mod candidate_final;
pub mod candidate_run;
pub mod candidate_runtime;
pub mod candidate_sync;
pub mod candidate_work;
pub mod config;
pub mod expert;
pub mod expert_runtime;
pub mod expert_session;
pub mod monitor;
pub mod setup;
pub mod shared;

use std::future::Future;
use std::io::IsTerminal;

use clap::Parser;

use crate::orchestration::{Clock, SystemClock};
use expert_runtime::{ExpertRuntime, PollOutcome, SelectionScreen};

pub use setup::{apply_expert_setup_paste, ExpertSetupPastes};

pub use shared::{destination_for, Role, RoleDestination};

/// Dispatches a role to its destination.
pub fn dispatch_role(role: Role) -> RoleDestination {
    destination_for(role)
}

/// Parses arguments and hands control to role dispatch.
pub fn run() -> anyhow::Result<()> {
    let args = args::Args::parse();
    match args.role {
        Some(Role::Candidate) => {
            if std::io::stdout().is_terminal() {
                run_candidate_interactive()
            } else {
                println!("pairproof: {}", candidate::placeholder_message());
                Ok(())
            }
        }
        Some(Role::Expert) => {
            if std::io::stdout().is_terminal() {
                run_interactive(Some(Role::Expert))
            } else {
                println!("pairproof: expert flow \u{2014} run preflight before starting a session");
                Ok(())
            }
        }
        None => {
            if std::io::stdout().is_terminal() {
                run_interactive(None)
            } else {
                println!("pairproof: select a role with --role expert or --role candidate");
                Ok(())
            }
        }
    }
}

/// Restores the terminal on drop for every exit or panic path.
struct TtyGuard;

impl Drop for TtyGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
        #[rustfmt::skip]
        let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen, crossterm::event::DisableBracketedPaste);
    }
}

/// Runs preflight once and feeds the gated outcome into the app.
/// Checks the port `build_expert_runtime` will select (first free in
/// `SANDBOX_PORT_RANGE`); when none is free, the range start is reported
/// busy so preflight still fails on the port check.
fn run_preflight_check(app: &mut app::ExpertApp) {
    let port =
        expert_runtime::select_sandbox_port().unwrap_or(expert_runtime::SANDBOX_PORT_RANGE.start);
    let passed = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map(|runtime| runtime.block_on(crate::sandbox::preflight::gather_preflight(port)))
        .map(|report| expert::can_proceed(&report))
        .unwrap_or(false);
    app.set_preflight_passed(passed);
}

/// Starts the interactive loop, falling back to a hint on init failure.
fn run_interactive(initial: Option<Role>) -> anyhow::Result<()> {
    let mut app = app::ExpertApp::new();
    let mut preflight_done = false;
    if initial == Some(Role::Expert) {
        let _ = app.handle(app::AppEvent::SelectExpert);
        run_preflight_check(&mut app);
        preflight_done = true;
    }
    if interactive_loop(&mut app, &mut preflight_done).is_err() {
        println!("pairproof: expert flow \u{2014} run preflight before starting a session");
        return Ok(());
    }
    if app.is_candidate_placeholder() {
        return run_candidate_interactive();
    }
    Ok(())
}

/// Starts the candidate loop, falling back to a hint on init failure.
fn run_candidate_interactive() -> anyhow::Result<()> {
    let mut state = candidate_run::CandidateRunState::new();
    if candidate_runtime::candidate_loop(&mut state).is_err() {
        println!("pairproof: {}", candidate::placeholder_message());
    }
    Ok(())
}

fn now_ms() -> u64 {
    SystemClock.now_ms()
}

/// Runs one bounded async step on a lazily built current-thread runtime.
#[rustfmt::skip]
fn block_on<T>(slot: &mut Option<tokio::runtime::Runtime>, future: impl Future<Output = T>) -> Option<T> { if slot.is_none() { *slot = tokio::runtime::Builder::new_current_thread().enable_all().build().ok(); } slot.as_ref().map(|runtime| runtime.block_on(future)) }

/// TTY adapter: draws app lines plus live runtime data, polls the
/// runtime in bounded steps, and maps keys to real operations.
fn interactive_loop(
    app: &mut app::ExpertApp,
    preflight_done: &mut bool,
) -> Result<(), Box<dyn std::error::Error>> {
    crossterm::terminal::enable_raw_mode()?;
    #[rustfmt::skip]
    crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen, crossterm::event::EnableBracketedPaste)?;
    let _guard = TtyGuard;
    let backend = ratatui::backend::CrosstermBackend::new(std::io::stdout());
    let mut terminal = ratatui::Terminal::new(backend)?;
    let mut runtime: Option<ExpertRuntime> = None;
    let mut tokio_rt: Option<tokio::runtime::Runtime> = None;
    let mut pastes = ExpertSetupPastes::new();
    while app.screen() != app::AppScreen::Exited {
        if app.screen() == app::AppScreen::Preflight && !*preflight_done {
            *preflight_done = true;
            run_preflight_check(app);
        }
        tick_runtime(app, &mut runtime, &mut tokio_rt);
        let title = app::screen_title(app.screen());
        let mut lines = app::screen_lines(app);
        if let Some(active) = runtime.as_mut() {
            let now = now_ms();
            match app.screen() {
                app::AppScreen::ConnectionWait => {
                    if let Ok(current) = active.current_string(now) {
                        lines.push(format!("Connection: {}", current.encode()));
                        lines.push(format!("Expires in {}s.", active.countdown_secs(now)));
                    }
                    lines.push("Waiting for candidate SSH login (no manual lock).".to_string());
                }
                #[rustfmt::skip]
                app::AppScreen::Monitor => {
                    lines.extend(monitor::render_monitor_lines(&active.monitor_view(now)));
                    if let Some(fresh) = active.reissue_string() { lines.push(format!("Reissue: {}", fresh.encode())); lines.push(format!("Expires in {}s.", active.countdown_secs(now))); }
                    if active.is_disconnected() { lines.push("Press R to reissue a fresh connection string.".to_string()); }
                    lines.push("Press e to end the session.".to_string());
                }
                _ => {}
            }
        }
        let _ = terminal.draw(|frame| {
            let widget = ratatui::widgets::Paragraph::new(lines.join("\n")).block(
                ratatui::widgets::Block::default()
                    .title(title)
                    .borders(ratatui::widgets::Borders::ALL),
            );
            frame.render_widget(widget, frame.area());
        });
        if crossterm::event::poll(std::time::Duration::from_millis(250)).unwrap_or(false) {
            match crossterm::event::read() {
                Ok(crossterm::event::Event::Key(key)) => {
                    if key.kind != crossterm::event::KeyEventKind::Release {
                        handle_key(app, &mut runtime, &mut tokio_rt, &mut pastes, key.code);
                    }
                }
                Ok(crossterm::event::Event::Paste(text)) => handle_paste(app, &mut pastes, &text),
                _ => {}
            }
        }
    }
    Ok(())
}

/// Bounded per-tick runtime step: poll for the candidate hello on
/// the connection screen, drain intents once the session is live.
fn tick_runtime(
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
            if let Some(PollOutcome::Connected) =
                block_on(tokio_rt, active.poll_candidate(now_ms()))
            {
                app.set_connected(true);
                let _ = app.handle(AppEvent::ConnectionLocked);
            }
        }
        AppScreen::SelectMedium | AppScreen::SelectHard | AppScreen::Monitor => {
            let _ = block_on(tokio_rt, active.refresh_pre_lock_secret(now_ms()));
            let outcome = block_on(tokio_rt, active.poll_candidate(now_ms()));
            let _ = block_on(tokio_rt, active.drain_intents(now_ms()));
            let _ = block_on(tokio_rt, active.check_heartbeat(now_ms()));
            if active.is_disconnected() {
                let _ = app.handle(AppEvent::AccidentalDisconnect);
            }
            if outcome == Some(PollOutcome::Connected) && app.is_disconnected() {
                let _ = app.handle(AppEvent::CandidateReconnected);
            }
        }
        _ => {}
    }
}

/// Feeds a bracketed paste on Setup into the collector; ignored elsewhere.
/// The only path that sets `has_proposal`, with progress on the status line.
#[rustfmt::skip]
fn handle_paste(app: &mut app::ExpertApp, pastes: &mut ExpertSetupPastes, text: &str) {
    if app.screen() != app::AppScreen::Setup { return; }
    match apply_expert_setup_paste(pastes, text) {
        Ok(false) => app.set_status_message("JD pasted. Paste additional notes."),
        Ok(true) => { app.set_has_proposal(true); app.set_status_message("Proposal ready. Press Enter."); }
        Err(reason) => app.set_status_message(reason),
    }
}

/// Maps one key press: app events before the runtime exists, real
/// runtime operations afterwards (`e` ends a live session).
fn handle_key(
    app: &mut app::ExpertApp,
    runtime: &mut Option<ExpertRuntime>,
    tokio_rt: &mut Option<tokio::runtime::Runtime>,
    pastes: &mut ExpertSetupPastes,
    code: crossterm::event::KeyCode,
) {
    use app::{AppEvent, AppScreen};
    use crossterm::event::KeyCode as K;
    match code {
        K::Char('q') | K::Esc => _ = app.handle(AppEvent::Quit),
        K::Char('b') => _ = app.handle(AppEvent::Back),
        K::Char('r') => _ = app.handle(AppEvent::Retry),
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
                        let _ = app.handle(AppEvent::EndSession);
                    }
                    Some(Err(err)) => app.set_status_message(format!("End failed: {err}")),
                    None => app.set_status_message("End failed: runtime unavailable"),
                }
            }
        }
        K::Char('R') if app.screen() == AppScreen::Monitor => {
            if let Some(active) = runtime.as_mut() {
                match block_on(tokio_rt, active.reissue(now_ms())) {
                    Some(Ok(fresh)) => {
                        app.set_status_message(format!("Fresh string ready: {}", fresh.encode()))
                    }
                    Some(Err(err)) => app.set_status_message(err.to_string()),
                    None => app.set_status_message("Reissue failed: runtime unavailable"),
                }
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
        AppScreen::Preflight => _ = app.handle(AppEvent::PreflightCompleted),
        AppScreen::ApiKey => {
            let _ = app.handle(AppEvent::ApiKeySaved);
        }
        AppScreen::Setup => {
            if app.disclosure_acknowledged {
                let _ = app.handle(AppEvent::ProposalReady);
            } else {
                let _ = app.handle(AppEvent::DisclosureAcknowledged);
            }
        }
        AppScreen::Proposal => {
            if runtime.is_none() {
                let build =
                    expert_runtime::build_expert_runtime(now_ms(), pastes.jd(), pastes.notes());
                match block_on(tokio_rt, build) {
                    Some(Ok(built)) => *runtime = Some(built),
                    Some(Err(err)) => {
                        app.set_status_message(format!("Session setup failed: {err}"));
                        return;
                    }
                    None => {
                        app.set_status_message("Session setup failed: runtime unavailable");
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
                    Some(Err(err)) => app.set_status_message(format!("Selection failed: {err}")),
                    None => app.set_status_message("Selection failed: runtime unavailable"),
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
                    Some(Err(err)) => app.set_status_message(format!("Selection failed: {err}")),
                    None => app.set_status_message("Selection failed: runtime unavailable"),
                }
            }
        }
        AppScreen::Monitor | AppScreen::RoleSelect | AppScreen::Exited => {}
        AppScreen::Final => _ = app.handle(AppEvent::Quit),
    }
}
