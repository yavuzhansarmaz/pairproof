use std::process::Command;

use pairproof::tui::app::{screen_lines, screen_title, AppError, AppEvent, AppScreen, ExpertApp};

fn advance_to_setup(app: &mut ExpertApp) {
    assert_eq!(app.handle(AppEvent::SelectExpert), Ok(AppScreen::Preflight));
    app.set_preflight_passed(true);
    app.set_api_key_ready(true);
    assert_eq!(
        app.handle(AppEvent::DisclosureAcknowledged),
        Ok(AppScreen::Preflight)
    );
    assert_eq!(
        app.handle(AppEvent::PreflightCompleted),
        Ok(AppScreen::Setup)
    );
}

fn advance_to_connection_wait(app: &mut ExpertApp) {
    advance_to_setup(app);
    assert_eq!(
        app.handle(AppEvent::DisclosureAcknowledged),
        Ok(AppScreen::Setup)
    );
    app.set_has_proposal(true);
    assert_eq!(app.handle(AppEvent::ProposalReady), Ok(AppScreen::Proposal));
    assert_eq!(
        app.handle(AppEvent::ProposalConfirmed),
        Ok(AppScreen::ConnectionWait)
    );
}

fn advance_to_monitor(app: &mut ExpertApp) {
    advance_to_connection_wait(app);
    app.set_connected(true);
    assert_eq!(
        app.handle(AppEvent::ConnectionLocked),
        Ok(AppScreen::SelectMedium)
    );
    assert_eq!(
        app.handle(AppEvent::MediumSelected),
        Ok(AppScreen::SelectHard)
    );
    assert_eq!(
        app.handle(AppEvent::ExpertSelectionDone),
        Ok(AppScreen::Monitor)
    );
}

#[test]
fn role_selection_dispatches_expert() {
    let mut app = ExpertApp::new();
    assert_eq!(app.screen(), AppScreen::RoleSelect);
    assert_eq!(app.handle(AppEvent::SelectExpert), Ok(AppScreen::Preflight));
    assert_eq!(app.screen(), AppScreen::Preflight);
}

#[test]
fn expert_flow_advances_through_all_screens() {
    let mut app = ExpertApp::new();
    advance_to_monitor(&mut app);
    assert!(app.expert_selection_done);
    assert_eq!(app.handle(AppEvent::EndSession), Ok(AppScreen::Final));
    assert!(app.ended);
    assert_eq!(app.handle(AppEvent::Quit), Ok(AppScreen::Exited));
    assert_eq!(app.screen(), AppScreen::Exited);
}

#[test]
fn preflight_failure_blocks_advance() {
    let mut app = ExpertApp::new();
    app.handle(AppEvent::SelectExpert).expect("to preflight");
    app.set_preflight_passed(false);
    assert_eq!(
        app.handle(AppEvent::PreflightCompleted),
        Err(AppError::PreflightBlocked)
    );
    assert_eq!(app.screen(), AppScreen::Preflight);
    app.set_preflight_passed(true);
    app.set_api_key_ready(true);
    assert_eq!(
        app.handle(AppEvent::PreflightCompleted),
        Err(AppError::DisclosureRequired)
    );
    app.handle(AppEvent::DisclosureAcknowledged)
        .expect("disclosure");
    assert_eq!(
        app.handle(AppEvent::PreflightCompleted),
        Ok(AppScreen::Setup)
    );
}

#[test]
fn disclosure_required_before_proposal() {
    let mut app = ExpertApp::new();
    app.handle(AppEvent::SelectExpert).expect("to preflight");
    app.set_preflight_passed(true);
    app.set_api_key_ready(true);
    assert_eq!(
        app.handle(AppEvent::PreflightCompleted),
        Err(AppError::DisclosureRequired)
    );
    assert_eq!(app.screen(), AppScreen::Preflight);
    assert_eq!(
        app.handle(AppEvent::DisclosureAcknowledged),
        Ok(AppScreen::Preflight)
    );
    assert!(app.disclosure_acknowledged);
    assert_eq!(
        app.handle(AppEvent::PreflightCompleted),
        Ok(AppScreen::Setup)
    );
    app.set_has_proposal(true);
    assert_eq!(app.handle(AppEvent::ProposalReady), Ok(AppScreen::Proposal));
}

#[test]
fn connection_requires_locked_event_not_timer() {
    let mut app = ExpertApp::new();
    advance_to_connection_wait(&mut app);
    assert_eq!(
        app.handle(AppEvent::ConnectionLocked),
        Err(AppError::NotConnected)
    );
    assert_eq!(app.screen(), AppScreen::ConnectionWait);
    app.set_connected(true);
    assert_eq!(app.screen(), AppScreen::ConnectionWait);
    assert_eq!(
        app.handle(AppEvent::ConnectionLocked),
        Ok(AppScreen::SelectMedium)
    );
}

#[test]
fn accidental_disconnect_stays_on_monitor() {
    let mut app = ExpertApp::new();
    advance_to_monitor(&mut app);
    assert_eq!(
        app.handle(AppEvent::AccidentalDisconnect),
        Ok(AppScreen::Monitor)
    );
    assert_eq!(app.screen(), AppScreen::Monitor);
    assert!(app.is_disconnected());
    assert!(!app.ended);
    assert_eq!(app.handle(AppEvent::EndSession), Ok(AppScreen::Final));
}

#[test]
fn back_navigation_returns_to_previous_screen() {
    let mut app = ExpertApp::new();
    advance_to_setup(&mut app);
    assert!(app.can_go_back());
    assert_eq!(app.back(), Ok(AppScreen::Preflight));
    assert_eq!(app.screen(), AppScreen::Preflight);
    assert_eq!(app.handle(AppEvent::Back), Ok(AppScreen::RoleSelect));
    assert_eq!(app.screen(), AppScreen::RoleSelect);
}

#[test]
fn quit_from_any_screen_exits() {
    let mut at_role = ExpertApp::new();
    assert_eq!(at_role.handle(AppEvent::Quit), Ok(AppScreen::Exited));

    let mut at_preflight = ExpertApp::new();
    at_preflight
        .handle(AppEvent::SelectExpert)
        .expect("to preflight");
    assert_eq!(at_preflight.handle(AppEvent::Quit), Ok(AppScreen::Exited));

    let mut at_monitor = ExpertApp::new();
    advance_to_monitor(&mut at_monitor);
    assert_eq!(at_monitor.handle(AppEvent::Quit), Ok(AppScreen::Exited));

    let mut at_final = ExpertApp::new();
    advance_to_monitor(&mut at_final);
    at_final.handle(AppEvent::EndSession).expect("to final");
    assert_eq!(at_final.handle(AppEvent::Quit), Ok(AppScreen::Exited));
}

#[test]
fn illegal_transition_is_typed_error_not_panic() {
    let mut app = ExpertApp::new();
    let err = app.handle(AppEvent::EndSession).expect_err("illegal");
    assert_eq!(
        err,
        AppError::InvalidTransition {
            from: AppScreen::RoleSelect,
            event: "EndSession".to_string()
        }
    );
    assert_eq!(app.screen(), AppScreen::RoleSelect);
    assert!(app.handle(AppEvent::MediumSelected).is_err());
    assert!(app.handle(AppEvent::ConnectionLocked).is_err());
    assert_eq!(app.screen(), AppScreen::RoleSelect);
}

#[test]
fn candidate_selection_never_enters_expert_flow() {
    let mut app = ExpertApp::new();
    assert_eq!(app.handle(AppEvent::SelectCandidate), Ok(AppScreen::Exited));
    assert!(app.is_candidate_placeholder());
    assert!(app.candidate_placeholder);
    assert_ne!(app.screen(), AppScreen::Preflight);
    assert!(app.handle(AppEvent::SelectExpert).is_err());
    assert_eq!(app.screen(), AppScreen::Exited);
    let lines = screen_lines(&app).join("\n");
    assert!(lines.contains("candidate:"));
    assert!(!lines.contains("Phase 9"));
    assert!(!lines.contains("not implemented"));
}

#[test]
fn preflight_advance_keeps_screen_until_disclosure() {
    let mut app = ExpertApp::new();
    app.handle(AppEvent::SelectExpert).expect("to preflight");
    app.set_preflight_passed(true);
    app.set_api_key_ready(true);
    assert_eq!(app.screen(), AppScreen::Preflight);
    assert!(app.preflight_passed);
    app.handle(AppEvent::DisclosureAcknowledged)
        .expect("disclosure");
    assert_eq!(
        app.handle(AppEvent::PreflightCompleted),
        Ok(AppScreen::Setup)
    );
}

#[test]
fn back_is_rejected_past_the_runtime_commit_point() {
    let mut app = ExpertApp::new();
    advance_to_monitor(&mut app);
    assert!(!app.can_go_back());
    assert!(app.handle(AppEvent::Back).is_err());
    assert_eq!(app.screen(), AppScreen::Monitor);
    let lines = screen_lines(&app).join("\n");
    assert!(!lines.contains("b back"), "{lines}");
    assert!(lines.contains("press e to end the session."), "{lines}");
    assert!(!lines.contains("Press Enter to end"), "{lines}");
    app.handle(AppEvent::EndSession).expect("to final");
    assert!(!app.can_go_back());
    assert!(app.handle(AppEvent::Back).is_err());
    assert_eq!(app.screen(), AppScreen::Final);
}

#[test]
fn proposal_and_final_lines_render_on_their_screens() {
    let mut app = ExpertApp::new();
    advance_to_setup(&mut app);
    app.set_has_proposal(true);
    app.set_proposal_lines(vec![
        "Level: mid".to_string(),
        "Topics: array, hash-table".to_string(),
        "Language: Python".to_string(),
    ]);
    assert_eq!(app.handle(AppEvent::ProposalReady), Ok(AppScreen::Proposal));
    let proposal = screen_lines(&app).join("\n");
    assert!(proposal.contains("Level: mid"), "{proposal}");
    assert!(proposal.contains("Topics: array, hash-table"), "{proposal}");
    assert!(proposal.contains("Language: Python"), "{proposal}");
    assert!(
        proposal.contains("press enter to wait for the candidate, b back, esc quit, ? help"),
        "{proposal}"
    );
    assert!(!proposal.contains("enter confirm"), "{proposal}");

    let mut done = ExpertApp::new();
    advance_to_monitor(&mut done);
    done.set_final_lines(vec![
        "Final breakdown".to_string(),
        "final rating: 7.50".to_string(),
    ]);
    assert_eq!(done.handle(AppEvent::EndSession), Ok(AppScreen::Final));
    let final_lines = screen_lines(&done).join("\n");
    assert!(final_lines.contains("Final breakdown"), "{final_lines}");
    assert!(final_lines.contains("final rating: 7.50"), "{final_lines}");
}

#[test]
fn status_message_can_be_cleared() {
    let mut app = ExpertApp::new();
    app.set_status_message("Fresh string ready: pp1:stale");
    assert!(screen_lines(&app).join("\n").contains("stale"));
    app.clear_status_message();
    assert_eq!(app.status_message(), None);
    assert!(!screen_lines(&app).join("\n").contains("stale"));
}

#[test]
fn back_at_role_select_is_error() {
    let mut app = ExpertApp::new();
    assert!(!app.can_go_back());
    assert!(app.back().is_err());
    assert_eq!(app.screen(), AppScreen::RoleSelect);
    assert!(app.handle(AppEvent::Back).is_err());
    assert_eq!(app.screen(), AppScreen::RoleSelect);
}

#[test]
fn back_forward_loop_keeps_history_bounded() {
    let mut app = ExpertApp::new();
    app.handle(AppEvent::SelectExpert).expect("to preflight");
    for _ in 0..5 {
        assert_eq!(app.back(), Ok(AppScreen::RoleSelect));
        assert_eq!(app.handle(AppEvent::SelectExpert), Ok(AppScreen::Preflight));
    }
    assert_eq!(app.history.len(), 1);
}

#[test]
fn setters_feed_controller_gates() {
    let mut app = ExpertApp::new();
    advance_to_setup(&mut app);
    app.set_disclosure_acknowledged(true);
    app.set_has_proposal(true);
    assert_eq!(app.handle(AppEvent::ProposalReady), Ok(AppScreen::Proposal));
    app.handle(AppEvent::ProposalConfirmed)
        .expect("to connection wait");
    app.set_connected(true);
    app.set_expert_selection_done(false);
    assert!(!app.expert_selection_done);
    assert_eq!(
        app.handle(AppEvent::ConnectionLocked),
        Ok(AppScreen::SelectMedium)
    );
}

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
fn end_session_twice_is_already_ended() {
    let mut app = ExpertApp::new();
    advance_to_monitor(&mut app);
    app.handle(AppEvent::EndSession).expect("to final");
    assert_eq!(
        app.handle(AppEvent::EndSession),
        Err(AppError::AlreadyEnded)
    );
    assert_eq!(app.screen(), AppScreen::Final);
}

#[test]
fn binary_no_tty_expert_still_prints_hint() {
    let output = Command::new(env!("CARGO_BIN_EXE_pairproof"))
        .args(["--role", "expert"])
        .output()
        .expect("binary runs");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
    assert!(stdout.contains("pairproof"));
    assert!(stdout.contains("expert"));
}
