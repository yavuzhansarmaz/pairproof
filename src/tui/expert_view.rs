//! Expert screen text: titles, per-screen key hints, and the
//! deterministic sanitized lines the TTY loop renders.

use crate::tui::app::{AppScreen, ExpertApp};
use crate::tui::candidate::placeholder_message;
use crate::tui::shared::sanitize_display;

/// Returns the static title for a screen.
pub fn screen_title(screen: AppScreen) -> &'static str {
    match screen {
        AppScreen::RoleSelect => "PairProof",
        AppScreen::Preflight => "Expert Setup",
        AppScreen::Setup => "Session setup",
        AppScreen::Proposal => "Proposed context",
        AppScreen::ConnectionWait => "Waiting for candidate",
        AppScreen::SelectMedium => "Select medium questions",
        AppScreen::SelectHard => "Select hard questions",
        AppScreen::Monitor => "Live monitor",
        AppScreen::Final => "Final breakdown",
        AppScreen::Exited => "Exited",
    }
}

fn footer(app: &ExpertApp) -> String {
    match app.screen() {
        AppScreen::Exited => "esc quit".to_string(),
        AppScreen::RoleSelect => "e expert, c candidate, esc quit, ? help".to_string(),
        AppScreen::Preflight => {
            if app.api_key_entry {
                "Backspace delete, esc quit".to_string()
            } else {
                "b back, r re-run, esc quit, ? help".to_string()
            }
        }
        AppScreen::Setup => "Enter continue, b back, esc quit, ? help".to_string(),
        AppScreen::Proposal => "Enter confirm, b back, esc quit, ? help".to_string(),
        AppScreen::ConnectionWait => "esc quit, ? help".to_string(),
        AppScreen::SelectMedium => "Enter load medium questions, esc quit, ? help".to_string(),
        AppScreen::SelectHard => "Enter load hard questions, esc quit, ? help".to_string(),
        AppScreen::Monitor => {
            "e end the session, R reissue after disconnect, esc quit, ? help".to_string()
        }
        AppScreen::Final => "Enter close, esc quit".to_string(),
    }
}

/// Builds deterministic sanitized lines for rendering one screen.
pub fn screen_lines(app: &ExpertApp) -> Vec<String> {
    use AppScreen as S;
    let screen = app.screen();
    let heading = if screen == S::RoleSelect {
        "Main Menu"
    } else {
        screen_title(screen)
    };
    let mut lines = vec![heading.to_string()];
    if app.candidate_placeholder && screen == S::Exited {
        lines.push(placeholder_message().to_string());
    } else {
        match screen {
            S::RoleSelect => lines.extend([
                "AI-native technical interviews.".into(),
                "The AI writes the code; candidates are scored".into(),
                "on how well they direct, check, and correct it.".into(),
                "".into(),
                "  e  Expert     Set up and run a live interview".into(),
                "  c  Candidate  Join with a connection string".into(),
                "".into(),
                "Choose a role to continue.".into(),
            ]),
            S::Preflight => {
                lines.extend([
                    "Everything PairProof needs on this machine,".into(),
                    "checked before you invite a candidate.".into(),
                    "".into(),
                ]);
                if app.preflight_lines.is_empty() {
                    lines.push("  Running environment checks...".into());
                } else {
                    for check in &app.preflight_lines {
                        lines.push(format!("  {check}"));
                    }
                }
                if app.api_key_entry {
                    lines.push("  Meta API key: entry open (hidden).".into());
                } else {
                    lines.push(format!(
                        "  {}",
                        app.api_key_status
                            .as_deref()
                            .unwrap_or("Meta API key: not checked yet")
                    ));
                }
                if app.disclosure_acknowledged {
                    lines.push("  Disclosure: acknowledged.".into());
                } else {
                    lines.extend([
                        "Meta disclosure".into(),
                        "  Your job description and additional notes".into(),
                        "  (each up to 50,000 characters) will be sent".into(),
                        "  to the Meta API to propose interview topics,".into(),
                        "  level, and language.".into(),
                    ]);
                }
                lines.push("".into());
                lines.push(if app.api_key_entry {
                    "Press Enter to save the key.".into()
                } else if !app.disclosure_acknowledged {
                    "Press Enter to acknowledge the disclosure.".into()
                } else if !app.api_key_ready {
                    "Press Enter to add your Meta API key.".into()
                } else if !app.preflight_passed {
                    "Fix failing checks, then press r to re-run them.".into()
                } else {
                    "Ready. Press Enter to continue.".into()
                });
            }
            S::Setup => {
                lines.extend([
                    "Paste the job description first.".into(),
                    "Additional notes are optional.".into(),
                    "".into(),
                ]);
                lines.push(if app.has_proposal {
                    "Proposal ready. Press Enter to review it.".into()
                } else {
                    "Waiting for the job description paste.".into()
                });
            }
            S::Proposal => {
                lines.push("Review the proposal. Press Enter to wait for the candidate.".into())
            }
            S::ConnectionWait => lines.push(if app.connected {
                "Candidate connected.".into()
            } else {
                "Waiting for candidate to connect.".into()
            }),
            S::SelectMedium => lines.push("Press Enter to load 3 medium questions.".into()),
            S::SelectHard => {
                lines.push("Press Enter to load 2 hard questions and open the monitor.".into())
            }
            S::Monitor => {
                lines.push(if app.disconnected {
                    "Candidate disconnected. Session is still live.".into()
                } else {
                    "Session live.".into()
                });
                lines.push("Press e to end the session.".into());
            }
            S::Final => lines.push("Session ended.".into()),
            S::Exited => lines.push("Goodbye.".into()),
        }
    }
    if let Some(message) = app.status_message() {
        lines.push(format!("Status: {}", sanitize_display(message)));
    }
    lines.push(footer(app));
    lines.iter().map(|line| sanitize_display(line)).collect()
}
