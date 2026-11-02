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
pub mod expert_keys;
pub mod expert_runtime;
pub mod expert_session;
pub mod expert_tty;
pub mod expert_view;
pub mod help;
pub mod monitor;
pub mod render;
pub mod setup;
pub mod shared;
pub mod viewport;

use std::io::IsTerminal;

use clap::Parser;

use expert_keys::{handle_key, handle_paste};
use expert_runtime::{ExpertRuntime, PollOutcome};
use expert_tty::{block_on, now_ms, run_preflight_check, TtyGuard};

pub use setup::{apply_expert_setup_paste, ExpertSetupPastes};

pub use shared::{destination_for, Role, RoleDestination};

/// Dispatches a role to its destination.
pub fn dispatch_role(role: Role) -> RoleDestination {
    destination_for(role)
}

/// Parses arguments and hands control to role dispatch.
#[rustfmt::skip]
pub fn run() -> anyhow::Result<()> {
    let original: Vec<String> = std::env::args().skip(1).collect();
    let args = args::Args::parse();
    if let Some(command) = &args.command { return args::run_command(command); }
    // Startup auto-update only on interactive (TTY) launches; non-TTY keeps placeholder behaviour. --help/--version exit in parse above.
    if std::io::stdout().is_terminal() && crate::distribution::startup::startup_gate(&original)? == crate::distribution::StartupGate::Reexec { return Ok(()); }
    match args.role {
        Some(Role::Candidate) => if std::io::stdout().is_terminal() { run_candidate_interactive() } else { println!("pairproof: {}", candidate::placeholder_message()); Ok(()) },
        Some(Role::Expert) => if std::io::stdout().is_terminal() { run_interactive(Some(Role::Expert)) } else { println!("pairproof: expert flow \u{2014} run preflight before starting a session"); Ok(()) },
        None => if std::io::stdout().is_terminal() { run_interactive(None) } else { println!("pairproof: select a role with --role expert or --role candidate"); Ok(()) },
    }
}

/// Starts the interactive loop, falling back to a hint on init failure.
fn run_interactive(initial: Option<Role>) -> anyhow::Result<()> {
    let mut app = app::ExpertApp::new();
    if expert_tty::load_disclosure_ack_default() {
        app.set_disclosure_acknowledged(true);
    }
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

/// TTY adapter: draws app lines plus live runtime data, polls the
/// runtime in bounded steps, and maps keys to real operations.
fn interactive_loop(
    app: &mut app::ExpertApp,
    preflight_done: &mut bool,
) -> Result<(), Box<dyn std::error::Error>> {
    crossterm::terminal::enable_raw_mode()?;
    #[rustfmt::skip]
    crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen, crossterm::event::EnableBracketedPaste, crossterm::event::EnableMouseCapture)?;
    let _guard = TtyGuard;
    let backend = ratatui::backend::CrosstermBackend::new(std::io::stdout());
    let mut terminal = ratatui::Terminal::new(backend)?;
    let mut runtime: Option<ExpertRuntime> = None;
    let mut tokio_rt: Option<tokio::runtime::Runtime> = None;
    let mut pastes = ExpertSetupPastes::new();
    let mut api_key_input = String::new();
    let mut setup_jd_height: u16 = 5;
    let mut setup_notes_height: u16 = 2;
    let mut setup_drag: Option<setup::ExpertSetupFocus> = None;
    let mut show_help = false;
    let mut show_disclosure = false;
    while app.screen() != app::AppScreen::Exited {
        if app.screen() == app::AppScreen::Preflight && !*preflight_done {
            *preflight_done = true;
            run_preflight_check(app);
        }
        tick_runtime(app, &mut runtime, &mut tokio_rt);
        let mut lines = app::screen_lines(app);
        if let Some(active) = runtime.as_mut() {
            let now = now_ms();
            match app.screen() {
                app::AppScreen::ConnectionWait => {
                    if let Ok(current) = active.current_string(now) {
                        lines.push(format!("Connection: {}", current.encode()));
                        lines.push(help::countdown_line(active.countdown_secs(now)));
                    }
                    lines.push("Waiting for candidate SSH login (no manual lock).".to_string());
                }
                #[rustfmt::skip]
                app::AppScreen::Monitor => {
                    lines.extend(monitor::render_monitor_lines(&active.monitor_view(now)));
                    lines.extend(active.recovery_lines());
                    if let Some(fresh) = active.reissue_string() { lines.push(format!("Reissue: {}", fresh.encode())); lines.push(help::countdown_line(active.countdown_secs(now))); }
                    if active.is_disconnected() { lines.push("Press R to reissue a fresh connection string.".to_string()); }
                    lines.push("Press e to end the session.".to_string());
                }
                _ => {}
            }
        }
        if app.api_key_entry && app.screen() == app::AppScreen::Preflight {
            let masked = format!(
                "  Key: {} ({} characters)",
                "\u{2022}".repeat(api_key_input.chars().count()),
                api_key_input.chars().count()
            );
            let at = lines.len().saturating_sub(1);
            lines.insert(at, masked);
        }
        if show_disclosure {
            lines = help::disclosure_lines();
        } else if show_help {
            lines = help::expert_help_lines(app.screen());
        }
        let height = terminal.size().map(|s| s.height as usize).unwrap_or(24);
        let visible = viewport::visible_with_pinned(&lines, height.saturating_sub(2));
        let centered = matches!(
            app.screen(),
            app::AppScreen::RoleSelect | app::AppScreen::Preflight
        ) || show_help
            || show_disclosure;
        let _ = terminal.draw(|frame| {
            if app.screen() == app::AppScreen::Setup && !show_help && !show_disclosure {
                render::draw_setup_screen(
                    frame,
                    &render::SetupPanel {
                        job_description: pastes.jd(),
                        additional_notes: pastes.notes(),
                        job_focused: pastes.focus() == setup::ExpertSetupFocus::JobDescription,
                        job_scroll: pastes.jd_scroll(),
                        notes_scroll: pastes.notes_scroll(),
                        status: app.status_message(),
                        job_height: setup_jd_height,
                        notes_height: setup_notes_height,
                    },
                );
            } else {
                render::draw_screen(frame, &visible, centered);
            }
        });
        if crossterm::event::poll(std::time::Duration::from_millis(250)).unwrap_or(false) {
            match crossterm::event::read() {
                Ok(crossterm::event::Event::Key(key)) => {
                    if key.kind != crossterm::event::KeyEventKind::Release {
                        if app.api_key_entry {
                            handle_key(
                                app,
                                &mut runtime,
                                &mut tokio_rt,
                                &mut pastes,
                                preflight_done,
                                &mut api_key_input,
                                key.code,
                            );
                            continue;
                        }
                        if show_help
                            && !show_disclosure
                            && help::help_overlay_dispatches(key.code, app.screen())
                        {
                            show_help = false;
                            handle_key(
                                app,
                                &mut runtime,
                                &mut tokio_rt,
                                &mut pastes,
                                preflight_done,
                                &mut api_key_input,
                                key.code,
                            );
                            continue;
                        }
                        match if show_disclosure {
                            help::expert_disclosure_action(key)
                        } else {
                            help::expert_event_action(key, show_help)
                        } {
                            help::ExpertKeyAction::ToggleHelp => {
                                if show_disclosure {
                                    show_disclosure = false;
                                    show_help = false;
                                } else {
                                    show_help = !show_help;
                                }
                            }
                            help::ExpertKeyAction::ShowDisclosure => {
                                show_disclosure = !show_disclosure;
                            }
                            help::ExpertKeyAction::ReturnToHelp => {
                                show_disclosure = false;
                            }
                            help::ExpertKeyAction::Ignore => {}
                            help::ExpertKeyAction::Quit => {
                                let _ = app.handle(app::AppEvent::Quit);
                            }
                            help::ExpertKeyAction::Dispatch => handle_key(
                                app,
                                &mut runtime,
                                &mut tokio_rt,
                                &mut pastes,
                                preflight_done,
                                &mut api_key_input,
                                key.code,
                            ),
                        }
                    }
                }
                Ok(crossterm::event::Event::Paste(text)) => {
                    handle_paste(app, &mut pastes, &mut api_key_input, &text)
                }
                Ok(crossterm::event::Event::Mouse(mouse))
                    if app.screen() == app::AppScreen::Setup && !show_help =>
                {
                    use crossterm::event::{MouseButton, MouseEventKind};
                    let size = terminal.size().unwrap_or_default();
                    let area = ratatui::layout::Rect::new(0, 0, size.width, size.height);
                    let (jd_box, notes_box) =
                        render::setup_box_areas(area, setup_jd_height, setup_notes_height);
                    let in_box = |rect: ratatui::layout::Rect| {
                        mouse.column >= rect.x
                            && mouse.column < rect.x + rect.width
                            && mouse.row >= rect.y
                            && mouse.row < rect.y + rect.height
                    };
                    match mouse.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
                            if in_box(jd_box) {
                                pastes.set_focus(setup::ExpertSetupFocus::JobDescription);
                                if mouse.row == jd_box.y + jd_box.height - 1 {
                                    setup_drag = Some(setup::ExpertSetupFocus::JobDescription);
                                }
                            } else if in_box(notes_box) {
                                pastes.set_focus(setup::ExpertSetupFocus::AdditionalNotes);
                                if mouse.row == notes_box.y + notes_box.height - 1 {
                                    setup_drag = Some(setup::ExpertSetupFocus::AdditionalNotes);
                                }
                            }
                        }
                        MouseEventKind::Drag(MouseButton::Left) => match setup_drag {
                            Some(setup::ExpertSetupFocus::JobDescription) => {
                                setup_jd_height =
                                    mouse.row.saturating_sub(jd_box.y + 1).clamp(5, 20);
                            }
                            Some(setup::ExpertSetupFocus::AdditionalNotes) => {
                                setup_notes_height =
                                    mouse.row.saturating_sub(notes_box.y + 1).clamp(2, 12);
                            }
                            None => {}
                        },
                        MouseEventKind::Up(MouseButton::Left) => setup_drag = None,
                        MouseEventKind::ScrollDown => {
                            if in_box(jd_box) {
                                pastes.set_focus(setup::ExpertSetupFocus::JobDescription);
                            } else if in_box(notes_box) {
                                pastes.set_focus(setup::ExpertSetupFocus::AdditionalNotes);
                            }
                            if in_box(jd_box) || in_box(notes_box) {
                                pastes.scroll_focused(1);
                            }
                        }
                        MouseEventKind::ScrollUp => {
                            if in_box(jd_box) {
                                pastes.set_focus(setup::ExpertSetupFocus::JobDescription);
                            } else if in_box(notes_box) {
                                pastes.set_focus(setup::ExpertSetupFocus::AdditionalNotes);
                            }
                            if in_box(jd_box) || in_box(notes_box) {
                                pastes.scroll_focused(-1);
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }
    // Quitting with a built runtime still ends the session (without
    // waiting for the final ack) so sandbox cleanup always runs.
    if let Some(active) = runtime.as_mut() {
        let _ = block_on(
            &mut tokio_rt,
            active.end_session_with_ack_deadline(now_ms(), 0),
        );
    }
    Ok(())
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
            let _ = block_on(tokio_rt, active.drain_intents(now_ms()));
            let _ = block_on(tokio_rt, active.check_heartbeat(now_ms()));
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
