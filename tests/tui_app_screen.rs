use pairproof::tui::app::{
    screen_lines, screen_parts, screen_title, selection_footer, AppError, AppEvent, AppScreen,
    ExpertApp,
};

#[test]
fn screen_lines_are_sanitized_with_footers() {
    for screen in [
        AppScreen::RoleSelect,
        AppScreen::Preflight,
        AppScreen::Setup,
        AppScreen::Proposal,
        AppScreen::ConnectionWait,
        AppScreen::SelectMedium,
        AppScreen::SelectHard,
        AppScreen::Monitor,
        AppScreen::Final,
        AppScreen::Exited,
    ] {
        assert!(!screen_title(screen).is_empty());
    }
    let mut app = ExpertApp::new();
    assert_eq!(screen_title(AppScreen::RoleSelect), "PairProof");
    let role_lines = screen_lines(&app).join("\n");
    assert!(role_lines.contains("Main Menu"), "{role_lines}");
    assert!(!role_lines.contains("Select role"), "{role_lines}");
    assert!(role_lines.contains("e  Expert"), "{role_lines}");
    assert!(role_lines.contains("c  Candidate"), "{role_lines}");
    assert!(
        role_lines.contains("judgment, not typing speed"),
        "{role_lines}"
    );
    assert!(
        role_lines.contains("choose your role to begin."),
        "{role_lines}"
    );
    assert!(role_lines.contains("esc quit"));
    assert!(!role_lines.contains("q quit"));
    assert!(!role_lines.contains("Enter advance"), "{role_lines}");
    assert!(!role_lines.contains("b back"));

    app.handle(AppEvent::SelectExpert).expect("to preflight");
    app.set_status_message("\u{1b}[31mhello\u{7} world");
    let lines = screen_lines(&app);
    let joined = lines.join("\n");
    assert!(joined.contains("Expert Setup"), "{joined}");
    assert!(joined.contains("Meta disclosure"), "{joined}");
    assert!(joined.contains("Meta API key:"), "{joined}");
    assert!(
        joined.contains("press enter to acknowledge the disclosure."),
        "{joined}"
    );
    assert_eq!(
        lines.last().map(String::as_str),
        Some("b back, esc quit, ? help")
    );
    assert!(!joined.contains("Esc qui\n"), "{joined}");
    assert!(!joined.contains("API key is never shown"), "{joined}");
    assert!(joined.contains("hello world"));
    assert!(!joined.contains('\u{1b}'));
    assert!(!joined.contains('\u{7}'));
}

#[test]
fn screen_parts_separates_footer_without_guessing_last_line() {
    // RoleSelect: footer is reported separately, never inside content,
    // and re-appending it reproduces screen_lines exactly.
    let app = ExpertApp::new();
    let (content, foot) = screen_parts(&app);
    let foot = foot.expect("role select has a footer");
    assert!(!content.contains(&foot), "{content:?}");
    let mut rejoined = content.clone();
    rejoined.push(foot.clone());
    assert_eq!(rejoined, screen_lines(&app));

    // Proposal carries its keys on its own wait line and has no footer,
    // so the wait line stays in content rather than being mistaken for one.
    let mut app = ExpertApp::new();
    app.set_disclosure_acknowledged(true);
    app.set_has_proposal(true);
    app.handle(AppEvent::SelectExpert).expect("to preflight");
    app.set_preflight_passed(true);
    app.set_api_key_ready(true);
    app.handle(AppEvent::PreflightCompleted).expect("to setup");
    app.handle(AppEvent::ProposalReady).expect("to proposal");
    let (content, foot) = screen_parts(&app);
    assert_eq!(foot, None);
    assert!(
        content
            .last()
            .is_some_and(|last| last.contains("press enter to wait")),
        "{content:?}"
    );
    assert_eq!(content, screen_lines(&app));
}

#[test]
fn expert_setup_steps_ack_then_api_key_then_continue() {
    let mut app = ExpertApp::new();
    app.handle(AppEvent::SelectExpert).expect("to preflight");
    app.set_preflight_passed(true);
    let before = screen_lines(&app).join("\n");
    assert!(before.contains("will be sent"), "{before}");
    assert!(before.contains("press enter to acknowledge"), "{before}");
    assert_eq!(
        app.handle(AppEvent::PreflightCompleted),
        Err(AppError::DisclosureRequired)
    );
    app.handle(AppEvent::DisclosureAcknowledged)
        .expect("acknowledge");
    let after = screen_lines(&app).join("\n");
    assert!(!after.contains("will be sent"), "{after}");
    assert!(after.contains("[PASS] Disclosure: acknowledged"), "{after}");
    assert!(
        after.contains("press enter to add your Meta API key."),
        "{after}"
    );
    assert_eq!(
        app.handle(AppEvent::PreflightCompleted),
        Err(AppError::ApiKeyMissing)
    );
    assert_eq!(app.screen(), AppScreen::Preflight);
    app.set_api_key_ready(true);
    app.set_api_key_status("Meta API key: configured (••••1234)");
    let ready_lines = screen_lines(&app).join("\n");
    assert!(
        ready_lines.contains("[PASS] Meta API key: configured"),
        "{ready_lines}"
    );
    assert_eq!(
        app.handle(AppEvent::PreflightCompleted),
        Ok(AppScreen::Setup)
    );
}

#[test]
fn select_medium_explains_loading_and_mentions_enter_once() {
    let mut app = ExpertApp::new();
    app.set_disclosure_acknowledged(true);
    app.set_has_proposal(true);
    app.handle(AppEvent::SelectExpert).expect("to preflight");
    app.set_preflight_passed(true);
    app.set_api_key_ready(true);
    app.handle(AppEvent::PreflightCompleted).expect("to setup");
    app.handle(AppEvent::ProposalReady).expect("to proposal");
    app.handle(AppEvent::ProposalConfirmed)
        .expect("to connection wait");
    app.set_connected(true);
    app.handle(AppEvent::ConnectionLocked)
        .expect("to select medium");
    assert_eq!(app.screen(), AppScreen::SelectMedium);
    let joined = screen_lines(&app).join("\n");
    assert!(
        joined.contains("3 medium questions matched to the proposed topics will be loaded."),
        "{joined}"
    );
    assert_eq!(joined.matches("enter").count(), 1, "{joined}");
}

#[test]
fn selection_footer_offers_random_only_after_questions_load() {
    let mut app = ExpertApp::new();
    app.set_disclosure_acknowledged(true);
    app.set_has_proposal(true);
    app.handle(AppEvent::SelectExpert).expect("to preflight");
    app.set_preflight_passed(true);
    app.set_api_key_ready(true);
    app.handle(AppEvent::PreflightCompleted).expect("to setup");
    app.handle(AppEvent::ProposalReady).expect("to proposal");
    app.handle(AppEvent::ProposalConfirmed)
        .expect("to connection wait");
    app.set_connected(true);
    app.handle(AppEvent::ConnectionLocked)
        .expect("to select medium");
    let before = selection_footer(&app, false).expect("footer");
    assert!(!before.contains("r random"), "{before}");
    let after = selection_footer(&app, true).expect("footer");
    assert!(after.contains("r random"), "{after}");
}

#[test]
fn select_hard_explains_choice_and_mentions_enter_once() {
    let mut app = ExpertApp::new();
    app.set_disclosure_acknowledged(true);
    app.set_has_proposal(true);
    app.handle(AppEvent::SelectExpert).expect("to preflight");
    app.set_preflight_passed(true);
    app.set_api_key_ready(true);
    app.handle(AppEvent::PreflightCompleted).expect("to setup");
    app.handle(AppEvent::ProposalReady).expect("to proposal");
    app.handle(AppEvent::ProposalConfirmed)
        .expect("to connection wait");
    app.set_connected(true);
    app.handle(AppEvent::ConnectionLocked)
        .expect("to select medium");
    app.handle(AppEvent::MediumSelected)
        .expect("to select hard");
    assert_eq!(app.screen(), AppScreen::SelectHard);
    let joined = screen_lines(&app).join("\n");
    assert!(
        joined.contains("choose 2 hard questions from the topic-matched list"),
        "{joined}"
    );
    assert!(joined.contains("the monitor opens next."), "{joined}");
    assert_eq!(joined.matches("enter").count(), 1, "{joined}");
}
