//! Terminal startup, role dispatch, and expert and candidate screens.
pub mod app;
pub mod args;
pub mod candidate;
pub mod candidate_actions;
pub mod candidate_chat;
pub mod candidate_final;
pub mod candidate_run;
pub mod candidate_sync;
pub mod candidate_work;
pub mod config;
pub mod connection;
pub mod expert;
pub mod monitor;
pub mod selection;
pub mod setup;
pub mod shared;

use std::io::IsTerminal;

use clap::Parser;

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
        let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen);
    }
}

/// Runs preflight once and feeds the gated outcome into the app.
fn run_preflight_check(app: &mut app::ExpertApp) {
    let passed = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map(|runtime| runtime.block_on(crate::sandbox::preflight::gather_preflight(2222)))
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
    if candidate_run::candidate_loop(&mut state).is_err() {
        println!("pairproof: {}", candidate::placeholder_message());
    }
    Ok(())
}

/// Thin TTY adapter: draws app lines and maps keys to app events.
fn interactive_loop(
    app: &mut app::ExpertApp,
    preflight_done: &mut bool,
) -> Result<(), Box<dyn std::error::Error>> {
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen)?;
    let _guard = TtyGuard;
    let backend = ratatui::backend::CrosstermBackend::new(std::io::stdout());
    let mut terminal = ratatui::Terminal::new(backend)?;
    while app.screen() != app::AppScreen::Exited {
        if app.screen() == app::AppScreen::Preflight && !*preflight_done {
            *preflight_done = true;
            run_preflight_check(app);
        }
        let title = app::screen_title(app.screen());
        let lines = app::screen_lines(app);
        let _ = terminal.draw(|frame| {
            let widget = ratatui::widgets::Paragraph::new(lines.join("\n")).block(
                ratatui::widgets::Block::default()
                    .title(title)
                    .borders(ratatui::widgets::Borders::ALL),
            );
            frame.render_widget(widget, frame.area());
        });
        if crossterm::event::poll(std::time::Duration::from_millis(250)).unwrap_or(false) {
            if let Ok(crossterm::event::Event::Key(key)) = crossterm::event::read() {
                if key.kind != crossterm::event::KeyEventKind::Release {
                    handle_key(app, key.code);
                }
            }
        }
    }
    Ok(())
}

/// Maps one key press to the app event for the current screen.
fn handle_key(app: &mut app::ExpertApp, code: crossterm::event::KeyCode) {
    use app::{AppEvent, AppScreen};
    use crossterm::event::KeyCode as K;
    match code {
        K::Char('q') | K::Esc => {
            let _ = app.handle(AppEvent::Quit);
        }
        K::Char('b') => {
            let _ = app.handle(AppEvent::Back);
        }
        K::Char('r') => {
            let _ = app.handle(AppEvent::Retry);
        }
        K::Char('e') if app.screen() == AppScreen::RoleSelect => {
            let _ = app.handle(AppEvent::SelectExpert);
        }
        K::Char('c') if app.screen() == AppScreen::RoleSelect => {
            let _ = app.handle(AppEvent::SelectCandidate);
        }
        K::Enter => advance(app),
        _ => {}
    }
}

/// Advances one screen on Enter through controller-checked transitions.
fn advance(app: &mut app::ExpertApp) {
    use app::{AppEvent, AppScreen};
    let event = match app.screen() {
        AppScreen::Preflight => Some(AppEvent::PreflightCompleted),
        AppScreen::ApiKey => Some(AppEvent::ApiKeySaved),
        AppScreen::Setup => {
            if app.disclosure_acknowledged {
                Some(AppEvent::ProposalReady)
            } else {
                Some(AppEvent::DisclosureAcknowledged)
            }
        }
        AppScreen::Proposal => Some(AppEvent::ProposalConfirmed),
        AppScreen::ConnectionWait => Some(AppEvent::ConnectionLocked),
        AppScreen::SelectMedium => Some(AppEvent::MediumSelected),
        AppScreen::SelectHard => Some(AppEvent::ExpertSelectionDone),
        AppScreen::Monitor => Some(AppEvent::EndSession),
        AppScreen::Final => Some(AppEvent::Quit),
        AppScreen::RoleSelect | AppScreen::Exited => None,
    };
    if let Some(event) = event {
        let _ = app.handle(event);
    }
}
