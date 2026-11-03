use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pairproof::tui::app::{AppEvent, AppScreen, ExpertApp};
use pairproof::tui::help::{
    disclosure_lines, expert_disclosure_action, expert_event_action, expert_help_lines,
    expert_key_action, help_overlay_dispatches, ExpertKeyAction,
};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn expert_event_action_never_dispatches_modified_letter_keys() {
    let ctrl = |code: KeyCode| KeyEvent::new(code, KeyModifiers::CONTROL);
    assert_eq!(
        expert_event_action(ctrl(KeyCode::Char('c')), false),
        ExpertKeyAction::Quit
    );
    // Ctrl-E on Monitor must not end the session, Ctrl-C on
    // RoleSelect must not select the candidate, Ctrl-B must not go back.
    for code in [
        KeyCode::Char('e'),
        KeyCode::Char('b'),
        KeyCode::Char('r'),
        KeyCode::Char('q'),
    ] {
        assert_eq!(
            expert_event_action(ctrl(code), false),
            ExpertKeyAction::Ignore
        );
    }
    assert_eq!(
        expert_event_action(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::ALT), false),
        ExpertKeyAction::Ignore
    );
    assert_eq!(
        expert_event_action(key(KeyCode::Char('e')), false),
        ExpertKeyAction::Dispatch
    );
    // Shift-modified keys (R reissue, ?) keep working.
    assert_eq!(
        expert_event_action(
            KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT),
            false
        ),
        ExpertKeyAction::Dispatch
    );
    assert_eq!(
        expert_event_action(
            KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT),
            false
        ),
        ExpertKeyAction::ToggleHelp
    );
}

#[test]
fn preflight_help_offers_api_key_change_without_rerun() {
    let lines = expert_help_lines(AppScreen::Preflight).join("\n");
    assert!(lines.contains("m change the Meta API key"), "{lines}");
    assert!(!lines.contains("enter follows"), "{lines}");
    assert!(!lines.contains("b back to the main menu"), "{lines}");
    assert!(lines.contains("b close help"), "{lines}");
    let d_at = lines.find("d read the disclosure").expect("d line");
    let m_at = lines.find("m change the Meta API key").expect("m line");
    let everywhere_at = lines.find("Everywhere:").expect("everywhere");
    assert!(m_at < d_at && d_at < everywhere_at, "{lines}");
    assert!(!lines.contains("b close help, d"), "{lines}");
    assert!(!lines.contains("re-run"), "{lines}");
    assert!(help_overlay_dispatches(
        KeyCode::Char('m'),
        AppScreen::Preflight
    ));
    assert!(help_overlay_dispatches(
        KeyCode::Char('m'),
        AppScreen::Setup
    ));
    assert!(!help_overlay_dispatches(
        KeyCode::Char('m'),
        AppScreen::Monitor
    ));
    assert!(!help_overlay_dispatches(
        KeyCode::Char('r'),
        AppScreen::Preflight
    ));
}

#[test]
fn failed_api_key_check_keeps_entry_and_saves_nothing() {
    use pairproof::tui::expert_tty::save_checked_api_key;
    let mut app = ExpertApp::new();
    app.handle(AppEvent::SelectExpert).expect("preflight");
    app.set_api_key_entry(true);
    let mut input = "bad-key".to_string();
    save_checked_api_key(&mut app, &mut input, Err("status 401".to_string()));
    assert!(app.api_key_entry, "entry stays open to fix the key");
    assert!(!app.api_key_ready);
    assert_eq!(input, "bad-key", "typed key is kept for editing");
    assert!(
        app.status_message()
            .is_some_and(|m| m.contains("key not saved")),
        "{:?}",
        app.status_message()
    );
}

#[test]
fn empty_api_key_enter_closes_without_saving() {
    use pairproof::tui::expert_tty::save_entered_api_key;
    let mut app = ExpertApp::new();
    app.handle(AppEvent::SelectExpert).expect("preflight");
    app.set_api_key_ready(true);
    app.set_api_key_entry(true);
    let mut input = String::new();
    save_entered_api_key(&mut app, &mut input);
    assert!(!app.api_key_entry);
    assert!(app.api_key_ready, "existing key must stay ready");
    assert!(input.is_empty());
    assert_eq!(app.status_message(), Some("Meta API key unchanged."));
    // Whitespace-only input is also "0 characters" and must not save.
    app.set_api_key_entry(true);
    let mut blank = "   ".to_string();
    save_entered_api_key(&mut app, &mut blank);
    assert!(!app.api_key_entry);
    assert!(blank.is_empty());
}

#[test]
fn disclosure_ack_persists_to_disk() {
    use pairproof::tui::expert_tty::{load_disclosure_ack, save_disclosure_ack};
    let dir = std::env::temp_dir().join(format!("pairproof-ack-{}", std::process::id()));
    let path = dir.join("disclosure_acknowledged");
    assert!(!load_disclosure_ack(&path));
    save_disclosure_ack(&path).expect("save ack");
    assert!(load_disclosure_ack(&path));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn d_opens_disclosure_only_from_help() {
    assert_eq!(
        expert_key_action(KeyCode::Char('d'), true),
        ExpertKeyAction::ShowDisclosure
    );
    assert_eq!(
        expert_key_action(KeyCode::Char('d'), false),
        ExpertKeyAction::Dispatch
    );
}

#[test]
fn help_overlay_still_dispatches_role_keys_on_main_menu() {
    assert!(help_overlay_dispatches(
        KeyCode::Char('e'),
        AppScreen::RoleSelect
    ));
    assert!(help_overlay_dispatches(
        KeyCode::Char('c'),
        AppScreen::RoleSelect
    ));
    assert!(!help_overlay_dispatches(
        KeyCode::Char('e'),
        AppScreen::Monitor
    ));
    assert!(!help_overlay_dispatches(
        KeyCode::Enter,
        AppScreen::RoleSelect
    ));
    assert!(!help_overlay_dispatches(
        KeyCode::Char('x'),
        AppScreen::RoleSelect
    ));
}

#[test]
fn disclosure_reader_esc_quits_and_b_returns_to_help() {
    assert_eq!(
        expert_disclosure_action(key(KeyCode::Esc)),
        ExpertKeyAction::Quit
    );
    assert_eq!(
        expert_disclosure_action(key(KeyCode::Char('b'))),
        ExpertKeyAction::ReturnToHelp
    );
    assert_eq!(
        expert_disclosure_action(key(KeyCode::Char('d'))),
        ExpertKeyAction::Ignore
    );
    assert_eq!(
        expert_disclosure_action(key(KeyCode::Char('?'))),
        ExpertKeyAction::Ignore
    );
    let disclosure = disclosure_lines().join("\n");
    assert!(
        disclosure.contains("b back to help, esc quit"),
        "{disclosure}"
    );
    assert!(!disclosure.contains("Press d"), "{disclosure}");
}

#[test]
fn expert_key_action_toggles_and_swallows_while_help_open() {
    assert_eq!(
        expert_key_action(KeyCode::Char('?'), false),
        ExpertKeyAction::ToggleHelp
    );
    assert_eq!(
        expert_key_action(KeyCode::Char('?'), true),
        ExpertKeyAction::Ignore
    );
    assert_eq!(
        expert_key_action(KeyCode::Char('b'), true),
        ExpertKeyAction::ToggleHelp
    );
    assert_eq!(
        expert_key_action(KeyCode::Char('q'), true),
        ExpertKeyAction::Ignore
    );
    assert_eq!(
        expert_key_action(KeyCode::Esc, true),
        ExpertKeyAction::Dispatch
    );
    assert_eq!(
        expert_key_action(KeyCode::Char('b'), false),
        ExpertKeyAction::Dispatch
    );
    assert_eq!(
        expert_key_action(KeyCode::Char('q'), false),
        ExpertKeyAction::Dispatch
    );
    assert_eq!(
        expert_key_action(KeyCode::Esc, false),
        ExpertKeyAction::Dispatch
    );
}
