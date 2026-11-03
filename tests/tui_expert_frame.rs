//! Expert frame behaviour reachable through the public screen
//! view-model: screen lines, footers, and the runtime-aware
//! selection footer. (The TTY loop state itself is crate-internal;
//! its frame decisions are covered only where a public surface
//! exposes the same lines.)

use pairproof::tui::app::{screen_lines, selection_footer, AppEvent, AppScreen, ExpertApp};

fn app_on(screen_after_setup: &[AppEvent]) -> ExpertApp {
    let mut app = ExpertApp::new();
    app.set_preflight_passed(true);
    app.set_disclosure_acknowledged(true);
    app.set_api_key_ready(true);
    let _ = app.handle(AppEvent::SelectExpert);
    let _ = app.handle(AppEvent::PreflightCompleted);
    assert_eq!(app.screen(), AppScreen::Setup);
    for event in screen_after_setup {
        let _ = app.handle(*event);
    }
    app
}

fn app_on_select_medium() -> ExpertApp {
    let mut app = ExpertApp::new();
    app.set_has_proposal(true);
    app.set_preflight_passed(true);
    app.set_disclosure_acknowledged(true);
    app.set_api_key_ready(true);
    let _ = app.handle(AppEvent::SelectExpert);
    let _ = app.handle(AppEvent::PreflightCompleted);
    let _ = app.handle(AppEvent::ProposalReady);
    let _ = app.handle(AppEvent::ProposalConfirmed);
    app.set_connected(true);
    let _ = app.handle(AppEvent::ConnectionLocked);
    assert_eq!(app.screen(), AppScreen::SelectMedium);
    app
}

#[test]
fn role_select_lines_start_with_main_menu_and_end_with_footer() {
    let app = ExpertApp::new();
    let lines = screen_lines(&app);
    assert_eq!(lines.first().map(String::as_str), Some("Main Menu"));
    assert_eq!(
        lines.last().map(String::as_str),
        Some("e expert, c candidate, esc quit, ? help")
    );
}

#[test]
fn selection_footer_hides_random_until_questions_load() {
    let app = app_on_select_medium();
    // Before the pool loads there is nothing to draw from, so the
    // footer must not offer `r random` yet; once loaded, it does.
    let before = selection_footer(&app, false).expect("footer before load");
    assert!(!before.contains("r random"), "{before}");
    let after = selection_footer(&app, true).expect("footer after load");
    assert!(after.contains("r random"), "{after}");
    // The plain screen lines carry the same heading and no random
    // offer in the static footer.
    let lines = screen_lines(&app);
    assert_eq!(
        lines.first().map(String::as_str),
        Some("Select medium questions")
    );
    assert!(
        !lines.last().is_some_and(|line| line.contains("r random")),
        "{lines:?}"
    );
}

#[test]
fn setup_screen_lines_keep_heading_and_footer() {
    let app = app_on(&[]);
    let lines = screen_lines(&app);
    assert_eq!(lines.first().map(String::as_str), Some("Session setup"));
    assert_eq!(
        lines.last().map(String::as_str),
        Some("enter continue, b back, esc quit, ? help")
    );
}
