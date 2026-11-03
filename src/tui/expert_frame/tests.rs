use super::*;

fn app_on_setup() -> app::ExpertApp {
    let mut app = app::ExpertApp::new();
    app.set_preflight_passed(true);
    app.set_disclosure_acknowledged(true);
    app.set_api_key_ready(true);
    let _ = app.handle(app::AppEvent::SelectExpert);
    let _ = app.handle(app::AppEvent::PreflightCompleted);
    assert_eq!(app.screen(), app::AppScreen::Setup);
    app
}

#[test]
fn frame_plan_centres_role_select_as_screen() {
    let app = app::ExpertApp::new();
    let mut state = ExpertLoopState::default();
    let plan = state.frame_plan(&app, 24);
    assert_eq!(plan.kind, FrameKind::Screen);
    assert!(plan.centered);
    assert!(plan.visible.iter().any(|line| line == "Main Menu"));
}

#[test]
fn frame_plan_uses_setup_kind_only_on_setup_without_overlays() {
    let app = app_on_setup();
    let mut state = ExpertLoopState::default();
    assert_eq!(state.frame_plan(&app, 24).kind, FrameKind::Setup);
    state.show_help = true;
    let plan = state.frame_plan(&app, 24);
    assert_eq!(plan.kind, FrameKind::Screen);
    assert!(plan.centered);
}

#[test]
fn frame_plan_centres_select_medium_as_screen() {
    let mut app = app_on_setup();
    app.set_has_proposal(true);
    let _ = app.handle(app::AppEvent::ProposalReady);
    let _ = app.handle(app::AppEvent::ProposalConfirmed);
    app.set_connected(true);
    let _ = app.handle(app::AppEvent::ConnectionLocked);
    assert_eq!(app.screen(), app::AppScreen::SelectMedium);
    let mut state = ExpertLoopState::default();
    let plan = state.frame_plan(&app, 24);
    assert_eq!(plan.kind, FrameKind::Screen);
    assert!(plan.centered);
}

#[test]
fn frame_plan_question_preview_uses_full_screen() {
    let mut app = app_on_setup();
    app.set_has_proposal(true);
    let _ = app.handle(app::AppEvent::ProposalReady);
    let _ = app.handle(app::AppEvent::ProposalConfirmed);
    app.set_connected(true);
    let _ = app.handle(app::AppEvent::ConnectionLocked);
    assert_eq!(app.screen(), app::AppScreen::SelectMedium);
    let mut state = ExpertLoopState {
        show_question_preview: true,
        ..ExpertLoopState::default()
    };
    let plan = state.frame_plan(&app, 24);
    assert_eq!(plan.kind, FrameKind::Screen);
    assert!(!plan.centered);
}

#[test]
fn frame_plan_centres_select_hard_before_list_opens() {
    let mut app = app_on_setup();
    app.set_has_proposal(true);
    let _ = app.handle(app::AppEvent::ProposalReady);
    let _ = app.handle(app::AppEvent::ProposalConfirmed);
    app.set_connected(true);
    let _ = app.handle(app::AppEvent::ConnectionLocked);
    let _ = app.handle(app::AppEvent::MediumSelected);
    assert_eq!(app.screen(), app::AppScreen::SelectHard);
    let mut state = ExpertLoopState::default();
    let plan = state.frame_plan(&app, 24);
    assert_eq!(plan.kind, FrameKind::Screen);
    assert!(plan.centered);
}

#[test]
fn frame_centered_expands_when_selection_list_open() {
    use app::AppScreen as S;
    // Intro selection screens are centred until the list opens,
    // then the frame goes full-area for picking.
    assert!(frame_centered(S::SelectMedium, false, false, false, false));
    assert!(frame_centered(S::SelectHard, false, false, false, false));
    assert!(!frame_centered(S::SelectMedium, false, false, false, true));
    assert!(!frame_centered(S::SelectHard, false, false, false, true));
    // Help and loading stay centred even over an open list, and
    // non-intro screens are never centred by this rule.
    assert!(frame_centered(S::SelectHard, true, false, false, true));
    assert!(frame_centered(S::Monitor, false, false, true, false));
    assert!(!frame_centered(S::Monitor, false, false, false, false));
}

#[test]
fn frame_plan_loading_wins_over_setup() {
    let app = app_on_setup();
    let mut state = ExpertLoopState::default();
    let (_tx, rx) = std::sync::mpsc::channel();
    state.pending.proposal_rx = Some(rx);
    let plan = state.frame_plan(&app, 24);
    assert_eq!(plan.kind, FrameKind::Loading);
    assert!(plan.centered);
}

#[test]
fn paste_event_is_ignored_while_loading() {
    let mut app = app_on_setup();
    let mut state = ExpertLoopState::default();
    let (_tx, rx) = std::sync::mpsc::channel();
    state.pending.proposal_rx = Some(rx);
    let area = ratatui::layout::Rect::new(0, 0, 80, 24);
    let copied = state.handle_event(
        &mut app,
        crossterm::event::Event::Paste("job text".to_string()),
        area,
        &[],
    );
    assert_eq!(copied, None);
    assert!(state.pastes.jd().is_empty());
}

#[test]
fn build_lines_hides_random_until_selection_questions_load() {
    let mut app = app_on_setup();
    app.set_has_proposal(true);
    let _ = app.handle(app::AppEvent::ProposalReady);
    let _ = app.handle(app::AppEvent::ProposalConfirmed);
    app.set_connected(true);
    let _ = app.handle(app::AppEvent::ConnectionLocked);
    assert_eq!(app.screen(), app::AppScreen::SelectMedium);
    let mut state = ExpertLoopState::default();
    let lines = state.build_lines(&app);
    assert!(
        !lines.last().is_some_and(|line| line.contains("r random")),
        "{lines:?}"
    );
    state.show_help = true;
    let lines = state.build_lines(&app);
    assert!(
        !lines.iter().any(|line| line.contains("r pick at random")),
        "{lines:?}"
    );
}

#[test]
fn build_lines_keeps_footer_last_behind_masked_key() {
    let mut app = app::ExpertApp::new();
    let _ = app.handle(app::AppEvent::SelectExpert);
    app.set_api_key_entry(true);
    let mut state = ExpertLoopState {
        api_key_input: "secret123".to_string(),
        ..ExpertLoopState::default()
    };
    let lines = state.build_lines(&app);
    assert_eq!(
        lines.last().map(String::as_str),
        app::footer(&app).as_deref()
    );
    let masked = lines
        .iter()
        .position(|line| line.contains("Key:"))
        .expect("masked line");
    assert_eq!(masked, lines.len() - 2, "{lines:?}");
    assert!(!lines.join("\n").contains("secret123"));
}
