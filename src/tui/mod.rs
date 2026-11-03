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
pub mod expert_frame;
pub mod expert_keys;
pub mod expert_loop;
pub mod expert_mouse;
pub mod expert_poll;
pub mod expert_runtime;
pub mod expert_session;
pub mod expert_tty;
pub mod expert_view;
pub mod help;
pub mod monitor;
pub mod render;
pub mod render_preview;
pub mod setup;
pub mod setup_panel;
pub mod shared;
pub mod viewport;

use std::io::IsTerminal;

use clap::Parser;

use expert_frame::FrameKind;
use expert_loop::ExpertLoopState;
use expert_tty::{run_preflight_check, TtyGuard};

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

pub(crate) use expert_poll::tick_runtime;

/// TTY shell around [`ExpertLoopState`]: owns the terminal and the
/// event poll only. Tick, frame decisions, event routing, and exit
/// cleanup live on the state, where they can be tested without a TTY.
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
        state.tick(app, preflight_done);
        let size = terminal.size().unwrap_or_default();
        let plan = state.frame_plan(app, size.width as usize, size.height as usize);
        let _ = terminal.draw(|frame| match plan.kind {
            FrameKind::Loading => render::draw_loading_screen(frame, &plan.visible),
            FrameKind::Setup => {
                render::draw_setup_screen(frame, &state.setup_panel());
            }
            FrameKind::Preview => {
                if let Some(preview) = &plan.preview {
                    render::draw_question_preview_screen(frame, preview, plan.preview_scroll);
                }
            }
            FrameKind::Screen => render::draw_screen(frame, &plan.visible, plan.centered),
        });
        if crossterm::event::poll(std::time::Duration::from_millis(250)).unwrap_or(false) {
            // Drain every queued event before the next tick/draw: a
            // tick does blocking runtime work, so one event per tick
            // makes fast key repeats (space, arrows) feel laggy.
            while let Ok(event) = crossterm::event::read() {
                let size = terminal.size().unwrap_or_default();
                let area = ratatui::layout::Rect::new(0, 0, size.width, size.height);
                if let Some(token) = state.handle_event(app, event, area, &plan.visible) {
                    expert_tty::copy_to_clipboard(&token);
                }
                if app.screen() == app::AppScreen::Exited
                    || !crossterm::event::poll(std::time::Duration::ZERO).unwrap_or(false)
                {
                    break;
                }
            }
        }
    }
    state.finish();
    Ok(())
}
