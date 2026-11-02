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
                "backspace delete, esc quit".to_string()
            } else {
                "b back, esc quit, ? help".to_string()
            }
        }
        AppScreen::Setup => "enter continue, b back, esc quit, ? help".to_string(),
        // Proposal carries its keys on the wait line; no footer.
        AppScreen::Proposal => String::new(),
        AppScreen::ConnectionWait => "esc quit, ? help".to_string(),
        AppScreen::SelectMedium => "enter load medium questions, esc quit, ? help".to_string(),
        AppScreen::SelectHard => "enter load hard questions, esc quit, ? help".to_string(),
        AppScreen::Monitor => {
            "e end the session, R reissue after disconnect, esc quit, ? help".to_string()
        }
        AppScreen::Final => "enter close, esc quit".to_string(),
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
                "interviewing for the AI era.".into(),
                "the AI writes the code. the candidate directs it.".into(),
                "every prompt, check, and correction is scored —".into(),
                "so what shows is judgment, not typing speed.".into(),
                "".into(),
                "  e  Expert     set up and run a live interview".into(),
                "  c  Candidate  join a session with a connection string".into(),
                "".into(),
                "choose your role to begin.".into(),
            ]),
            S::Preflight => {
                lines.extend([
                    "everything PairProof needs on this machine,".into(),
                    "checked before you invite a candidate.".into(),
                    "".into(),
                ]);
                if app.preflight_lines.is_empty() {
                    lines.push("  running environment checks...".into());
                } else {
                    for check in &app.preflight_lines {
                        lines.push(format!("  {check}"));
                    }
                }
                if app.api_key_entry {
                    lines.push("  Meta API key: entry open (hidden).".into());
                } else {
                    let status = app
                        .api_key_status
                        .as_deref()
                        .unwrap_or("Meta API key: not checked yet");
                    lines.push(if app.api_key_ready {
                        format!("  [PASS] {status}")
                    } else {
                        format!("  {status}")
                    });
                }
                if app.disclosure_acknowledged {
                    lines.push("  [PASS] Disclosure: acknowledged".into());
                } else {
                    lines.extend([
                        "Meta disclosure".into(),
                        "  your job description and additional notes".into(),
                        "  (each up to 50,000 characters) will be sent".into(),
                        "  to the Meta API to propose interview topics,".into(),
                        "  level, and language.".into(),
                    ]);
                }
                lines.push("".into());
                lines.push(if app.api_key_entry {
                    "press enter to save the key (empty enter closes without saving).".into()
                } else if !app.disclosure_acknowledged {
                    "press enter to acknowledge the disclosure.".into()
                } else if !app.api_key_ready {
                    "press enter to add your Meta API key.".into()
                } else if !app.preflight_passed {
                    "fix the failing checks, then restart to check again.".into()
                } else {
                    "ready. press enter to continue.".into()
                });
            }
            S::Setup => {
                lines.extend([
                    "paste the job description first.".into(),
                    "additional notes are optional.".into(),
                    "".into(),
                ]);
                lines.push(if app.has_proposal {
                    "proposal ready. press enter to review it.".into()
                } else {
                    "waiting for the job description paste.".into()
                });
            }
            S::Proposal => {
                if app.proposal_lines.is_empty() {
                    lines.push(
                        "review the proposal. press enter to wait for the candidate, b back, esc quit, ? help"
                            .into(),
                    );
                } else {
                    for line in &app.proposal_lines {
                        lines.push(format!("  {line}"));
                    }
                    lines.push(
                        "press enter to wait for the candidate, b back, esc quit, ? help".into(),
                    );
                }
            }
            S::ConnectionWait => lines.push(if app.connected {
                "candidate connected.".into()
            } else {
                "waiting for candidate to connect.".into()
            }),
            S::SelectMedium => lines.push("press enter to load 3 medium questions.".into()),
            S::SelectHard => {
                lines.push("press enter to load 2 hard questions and open the monitor.".into())
            }
            S::Monitor => {
                lines.push(if app.disconnected {
                    "candidate disconnected. session is still live.".into()
                } else {
                    "session live.".into()
                });
                lines.push("press e to end the session.".into());
            }
            S::Final => {
                if app.final_lines.is_empty() {
                    lines.push("session ended.".into());
                } else {
                    lines.extend(app.final_lines.iter().cloned());
                }
            }
            S::Exited => lines.push("goodbye.".into()),
        }
    }
    if let Some(message) = app.status_message() {
        lines.push(format!("status: {message}"));
    }
    let foot = footer(app);
    if !foot.is_empty() {
        lines.push(foot);
    }
    lines.iter().map(|line| sanitize_display(line)).collect()
}
