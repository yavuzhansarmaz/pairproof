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
pub mod expert_loop;
pub mod expert_runtime;
pub mod expert_session;
pub mod expert_tty;
pub mod expert_view;
pub mod help;
pub mod monitor;
pub mod render;
pub mod setup;
pub mod setup_panel;
pub mod shared;
pub mod viewport;

use std::io::IsTerminal;

use clap::Parser;

use expert_keys::handle_paste;
use expert_loop::ExpertLoopState;
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
/// Backing out of candidate Connect with an empty Enter returns here.
fn run_interactive(initial: Option<Role>) -> anyhow::Result<()> {
    let mut initial = initial;
    loop {
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
        if !app.is_candidate_placeholder() {
            return Ok(());
        }
        let mut state = candidate_run::CandidateRunState::new();
        if candidate_runtime::candidate_loop(&mut state).is_err() {
            println!("pairproof: {}", candidate::placeholder_message());
            return Ok(());
        }
        if !state.back_to_menu {
            return Ok(());
        }
        initial = None;
    }
}

/// Starts the candidate loop, falling back to a hint on init failure.
/// An empty Enter on Connect returns to the main menu.
fn run_candidate_interactive() -> anyhow::Result<()> {
    let mut state = candidate_run::CandidateRunState::new();
    if candidate_runtime::candidate_loop(&mut state).is_err() {
        println!("pairproof: {}", candidate::placeholder_message());
        return Ok(());
    }
    if state.back_to_menu {
        return run_interactive(None);
    }
    Ok(())
}

pub(crate) use expert_loop::tick_runtime;

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
    let mut state = ExpertLoopState::default();
    while app.screen() != app::AppScreen::Exited {
        if app.screen() == app::AppScreen::Preflight && !*preflight_done {
            *preflight_done = true;
            run_preflight_check(app);
        }
        state.poll_proposal(app);
        state.poll_runtime_build(app);
        tick_runtime(app, &mut state.runtime, &mut state.tokio_rt);
        let lines = if state.is_loading() {
            state.loading_lines()
        } else {
            state.build_lines(app)
        };
        let height = terminal.size().map(|s| s.height as usize).unwrap_or(24);
        let visible = viewport::visible_with_pinned(&lines, height.saturating_sub(2));
        let centered = matches!(
            app.screen(),
            app::AppScreen::RoleSelect
                | app::AppScreen::Preflight
                | app::AppScreen::Proposal
                | app::AppScreen::ConnectionWait
        ) || state.show_help
            || state.show_disclosure
            || state.is_loading();
        let show_setup = app.screen() == app::AppScreen::Setup
            && !state.show_help
            && !state.show_disclosure
            && !state.is_loading();
        let _ = terminal.draw(|frame| {
            if state.is_loading() {
                render::draw_loading_screen(frame, &visible);
            } else if show_setup {
                render::draw_setup_screen(
                    frame,
                    &render::SetupPanel {
                        job_description: state.pastes.jd(),
                        additional_notes: state.pastes.notes(),
                        job_focused: state.pastes.focus()
                            == setup::ExpertSetupFocus::JobDescription,
                        job_scroll: state.pastes.jd_scroll(),
                        notes_scroll: state.pastes.notes_scroll(),
                        job_height: state.setup_jd_height,
                        notes_height: state.setup_notes_height,
                    },
                );
            } else {
                render::draw_screen(frame, &visible, centered);
            }
        });
        if crossterm::event::poll(std::time::Duration::from_millis(250)).unwrap_or(false) {
            match crossterm::event::read() {
                Ok(crossterm::event::Event::Key(key)) => state.handle_key_event(app, key),
                Ok(crossterm::event::Event::Paste(text)) => {
                    if !state.is_loading() {
                        handle_paste(app, &mut state.pastes, &mut state.api_key_input, &text)
                    }
                }
                Ok(crossterm::event::Event::Mouse(mouse))
                    if app.screen() == app::AppScreen::Setup
                        && !state.show_help
                        && !state.is_loading() =>
                {
                    let size = terminal.size().unwrap_or_default();
                    state.handle_setup_mouse(
                        ratatui::layout::Rect::new(0, 0, size.width, size.height),
                        mouse,
                    );
                }
                Ok(crossterm::event::Event::Mouse(mouse))
                    if matches!(
                        app.screen(),
                        app::AppScreen::ConnectionWait | app::AppScreen::Monitor
                    ) && !state.show_help
                        && matches!(
                            mouse.kind,
                            crossterm::event::MouseEventKind::Down(
                                crossterm::event::MouseButton::Left
                            )
                        ) =>
                {
                    let size = terminal.size().unwrap_or_default();
                    if render::is_copy_button_at(
                        &visible,
                        ratatui::layout::Rect::new(0, 0, size.width, size.height),
                        mouse.column,
                        mouse.row,
                    ) {
                        if let Some(token) = state.copy_token(app.screen()) {
                            expert_tty::copy_to_clipboard(&token);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    // Quitting with a built runtime still ends the session (without
    // waiting for the final ack) so sandbox cleanup always runs.
    if let Some(active) = state.runtime.as_mut() {
        let _ = block_on(
            &mut state.tokio_rt,
            active.end_session_with_ack_deadline(now_ms(), 0),
        );
    }
    Ok(())
}
