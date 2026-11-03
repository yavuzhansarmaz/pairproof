//! Candidate live key handling: work/other/redirect/chat pane
//! keys mapped to control-channel intents.

use super::*;

use crate::tui::shared::pop_last;

fn apply(status: &mut Option<String>, result: Result<Intent, CandidateRuntimeError>) -> bool {
    match result {
        Ok(_) => {
            *status = None;
            true
        }
        Err(err) => {
            *status = Some(crate::tui::help::candidate_error_message(&err).to_string());
            false
        }
    }
}
pub fn handle_live_key<T: CandidateTransport>(
    state: &mut CandidateRunState,
    runtime: &mut CandidateRuntime<T>,
    key: KeyEvent,
    status: &mut Option<String>,
) {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        K::Esc => {
            state.app.quit();
            return;
        }
        K::Char('c') if control => {
            state.app.quit();
            return;
        }
        _ => {}
    }
    if key.code == K::Char('?')
        && !control
        && !crate::tui::help::candidate_is_typing(CandidateScreen::Solving, state.focus)
    {
        if !state.show_help {
            state.show_help = true;
        }
        return;
    }
    if state.show_help {
        if key.code == K::Char('b') && !control {
            state.show_help = false;
        }
        return;
    }
    if runtime.view().is_terminal() {
        if key.code == K::Enter {
            state.app.quit();
        }
        return;
    }
    match state.focus {
        CandidateFocus::Work => match key.code {
            K::Tab => state.focus = next_focus(state.focus),
            K::Char(c) if !control => match work_key_action(c) {
                Some(WorkKeyAction::SelectPreset(preset)) => {
                    apply(
                        status,
                        runtime.send_intent(IntentKind::SelectPreset { preset }),
                    );
                }
                Some(WorkKeyAction::OpenOther) => state.focus = CandidateFocus::Other,
                Some(WorkKeyAction::Continue) => {
                    apply(status, runtime.send_intent(IntentKind::Continue));
                }
                Some(WorkKeyAction::ChangeApproach) => {
                    apply(status, runtime.send_intent(IntentKind::ChangeApproach));
                }
                Some(WorkKeyAction::OpenRedirect) => state.focus = CandidateFocus::Redirect,
                Some(WorkKeyAction::RequestTests) => {
                    apply(status, runtime.send_intent(IntentKind::RequestTests));
                }
                Some(WorkKeyAction::Skip) => {
                    apply(status, runtime.send_intent(IntentKind::Skip));
                }
                Some(WorkKeyAction::Finish) => {
                    if apply(status, runtime.send_intent(IntentKind::Finish)) {
                        if let Err(err) = runtime.flush() {
                            *status =
                                Some(crate::tui::help::candidate_error_message(&err).to_string());
                        }
                    }
                }
                Some(WorkKeyAction::RequestReQuestion) => {
                    apply(status, runtime.send_intent(IntentKind::RequestReQuestion));
                }
                None => {}
            },
            _ => {}
        },
        CandidateFocus::Other => match key.code {
            K::Tab => state.focus = next_focus(state.focus),
            K::Enter => {
                let text = state.work.other_text().to_string();
                if text.trim().is_empty() {
                    *status = Some("type your own approach first, then press enter.".to_string());
                } else if apply(
                    status,
                    runtime.send_intent(IntentKind::SelectOther { text }),
                ) {
                    state.work.set_other_text("");
                    state.focus = CandidateFocus::Work;
                }
            }
            K::Backspace => {
                let text = pop_last(state.work.other_text());
                state.work.set_other_text(&text);
            }
            K::Char(c) if !control => {
                let _ = other_insert(&mut state.work, &c.to_string(), false);
            }
            _ => {}
        },
        CandidateFocus::Redirect => match key.code {
            K::Tab => state.focus = next_focus(state.focus),
            K::Enter => {
                let text = state.work.redirect_text().to_string();
                if text.trim().is_empty() {
                    *status = Some("type a redirect first, then press enter.".to_string());
                } else if apply(status, runtime.send_intent(IntentKind::Redirect { text })) {
                    state.work.set_redirect_text("");
                    state.focus = CandidateFocus::Work;
                }
            }
            K::Backspace => {
                let text = pop_last(state.work.redirect_text());
                state.work.set_redirect_text(&text);
            }
            K::Char(c) if !control => state.work.push_redirect_char(c),
            _ => {}
        },
        CandidateFocus::Chat => match key.code {
            K::Tab => state.focus = next_focus(state.focus),
            K::Enter => {
                let text = state.chat.input_text().to_string();
                if text.trim().is_empty() {
                    *status = Some("type a message first, then press enter.".to_string());
                } else if apply(status, runtime.send_chat(&text)) {
                    state.chat.clear_input();
                }
            }
            K::Backspace => state.chat.backspace_input(),
            K::Char(c) if !control => state.chat.type_char(c),
            _ => {}
        },
    }
}
