//! Candidate live key handling: work/other/redirect/chat pane
//! keys mapped to control-channel intents.

use super::*;

use crate::tui::shared::pop_last;

/// True when the approach cursor sits on the `other` row.
fn cursor_on_other(state: &CandidateRunState, option_count: usize) -> bool {
    option_count > 0
        && state
            .staging
            .current_stage()
            .is_some_and(|stage| stage.cursor == option_count)
}

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
/// Syncs the local staging to the snapshot's current question and
/// returns the live view the key handler acts on.
pub(crate) fn sync_staging(state: &mut CandidateRunState, view: &CandidateView) {
    if let Some(index) = view.question_index {
        let stage = state.staging.switch_to(
            index,
            &view.selected,
            view.options.len(),
            state.work.other_text(),
        );
        if stage.other_text != state.work.other_text() {
            state.work.set_other_text(&stage.other_text);
        }
    }
}

/// Sends a question switch for `delta` in the navigation list; the
/// staged choices stay saved per question and are restored when the
/// new snapshot lands.
fn navigate<T: CandidateTransport>(
    state: &mut CandidateRunState,
    runtime: &mut CandidateRuntime<T>,
    status: &mut Option<String>,
    view: &CandidateView,
    delta: i32,
) {
    let (Some(current), false) = (view.question_index, view.nav.is_empty()) else {
        return;
    };
    let Some(pos) = view.nav.iter().position(|entry| entry.index == current) else {
        return;
    };
    let target_pos = pos as i32 + delta;
    if target_pos < 0 || target_pos >= view.nav.len() as i32 {
        return;
    }
    let target = view.nav[target_pos as usize].index;
    let outgoing = state.work.other_text().to_string();
    state.staging.save_other_text(&outgoing);
    apply(
        status,
        runtime.send_intent(IntentKind::Navigate { index: target }),
    );
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
    let view = runtime.view();
    sync_staging(state, &view);
    match state.focus {
        CandidateFocus::Work => match key.code {
            K::Tab => {
                // Tab on the `Other` row (or from anywhere in Work)
                // opens the own-approach editor.
                state.staging.cursor_to_other(view.options.len());
                state.focus = CandidateFocus::Other;
            }
            K::Up => state.staging.move_cursor(-1, view.options.len()),
            K::Down => state.staging.move_cursor(1, view.options.len()),
            K::Left => navigate(state, runtime, status, &view, -1),
            K::Right => navigate(state, runtime, status, &view, 1),
            K::Char(' ') if !control => {
                state.staging.mark_cursor(view.options.len());
                // Selecting the other row opens its text field. The
                // letters that follow belong to the approach, not to
                // the redirect command.
                if cursor_on_other(state, view.options.len()) {
                    state.focus = CandidateFocus::Other;
                }
            }
            K::Enter => {
                let on_other = cursor_on_other(state, view.options.len());
                let choice = state
                    .staging
                    .current_stage()
                    .and_then(|stage| stage.choice.clone());
                match choice {
                    Some(crate::tui::candidate_stage::StagedChoice::Preset(preset)) => {
                        apply(
                            status,
                            runtime.send_intent(IntentKind::SelectPreset { preset }),
                        );
                    }
                    Some(crate::tui::candidate_stage::StagedChoice::Other) => {
                        let text = state.work.other_text().to_string();
                        if apply(
                            status,
                            runtime.send_intent(IntentKind::SelectOther { text }),
                        ) {
                            state.work.set_other_text("");
                            state.staging.save_other_text("");
                        }
                    }
                    None if on_other => {
                        state.staging.mark_cursor(view.options.len());
                        state.focus = CandidateFocus::Other;
                    }
                    None => {
                        *status = Some(
                            "mark an approach with space, then press enter to submit.".to_string(),
                        );
                    }
                }
            }
            // The other row is a text field: `d` and the other work
            // keys are letters of the approach, not commands.
            K::Char(c) if !control && cursor_on_other(state, view.options.len()) => {
                state.staging.mark_cursor(view.options.len());
                state.focus = CandidateFocus::Other;
                let _ = other_insert(&mut state.work, &c.to_string(), false);
                state.staging.save_other_text(state.work.other_text());
            }
            K::Char(c) if !control => match work_key_action(c) {
                Some(WorkKeyAction::SelectPreset(preset)) => {
                    // Number keys only move the mark; Enter submits.
                    if (preset as usize) < view.options.len() {
                        state.staging.move_cursor(
                            preset as i32
                                - state
                                    .staging
                                    .current_stage()
                                    .map_or(0, |stage| stage.cursor)
                                    as i32,
                            view.options.len(),
                        );
                        state.staging.mark_cursor(view.options.len());
                    }
                }
                Some(WorkKeyAction::OpenOther) => {
                    state.staging.cursor_to_other(view.options.len());
                    state.staging.mark_cursor(view.options.len());
                    state.focus = CandidateFocus::Other;
                }
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
                // A blank `Other` is a valid submission.
                let text = state.work.other_text().to_string();
                if apply(
                    status,
                    runtime.send_intent(IntentKind::SelectOther { text }),
                ) {
                    state.work.set_other_text("");
                    state.staging.save_other_text("");
                    state.staging.mark_cursor(view.options.len());
                    state.focus = CandidateFocus::Work;
                }
            }
            K::Backspace => {
                let text = pop_last(state.work.other_text());
                state.work.set_other_text(&text);
                state.staging.save_other_text(state.work.other_text());
            }
            K::Char(c) if !control => {
                let _ = other_insert(&mut state.work, &c.to_string(), false);
                state.staging.save_other_text(state.work.other_text());
            }
            _ => {}
        },
        CandidateFocus::Redirect => match key.code {
            K::Tab => state.focus = next_focus(state.focus),
            K::Enter => {
                let text = state.work.redirect_text().to_string();
                if text.trim().is_empty() {
                    *status = Some("type additional notes first, then press enter.".to_string());
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
                    state.chat.remember_sent(&text);
                    state.chat.clear_input();
                }
            }
            K::Backspace => state.chat.backspace_input(),
            K::Char(c) if !control => state.chat.type_char(c),
            _ => {}
        },
    }
}
