//! Candidate live TTY loop and live key handling.

use super::*;

use super::panes::{live_lines, live_panes, screen_centered};
use crate::tui::shared::pop_last;

/// Local workspace synced to the container `/workspace` root.
pub fn default_workspace() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("pairproof-workspace")
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
fn fresh_verifier(state: &mut CandidateRunState) -> Arc<Mutex<Option<(ConnectionString, String)>>> {
    let verifier = SshAuthVerifier::new();
    let stash = verifier.stash();
    state.app.set_verifier(verifier);
    stash
}
/// Production TTY loop; each tick polls at most one bounded op.
pub fn candidate_loop(state: &mut CandidateRunState) -> Result<(), Box<dyn Error>> {
    let _guard = CandidateTtyGuard;
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableBracketedPaste
    )?;
    let backend = ratatui::backend::CrosstermBackend::new(std::io::stdout());
    let mut terminal = ratatui::Terminal::new(backend)?;
    let mut stash = fresh_verifier(state);
    let mut runtime: Option<CandidateRuntime<SshCandidateTransport>> = None;
    let mut live = false;
    let mut status: Option<String> = None;
    let mut loading_tick = 0usize;
    while state.app.screen() != CandidateScreen::Exited {
        if live {
            if let Some(active) = runtime.as_mut() {
                if let Err(err) = active.poll_snapshot() {
                    status = Some(crate::tui::help::candidate_error_message(&err).to_string());
                }
            }
        }
        let height = terminal.size().map(|s| s.height as usize).unwrap_or(24);
        // While the statement is still arriving, the shared loading
        // screen replaces the placeholder solving screen entirely.
        let waiting = live
            && !state.show_help
            && runtime.as_ref().is_some_and(|active| {
                active.view().waiting_for_statement() && !active.connection_lost()
            });
        let panes = if live && !state.show_help && !waiting {
            runtime.as_ref().and_then(|active| {
                if active.view().is_terminal() {
                    None
                } else {
                    let (left, right) = live_panes(state, active, &status);
                    let pane_height = height.saturating_sub(2);
                    Some((
                        crate::tui::viewport::visible_with_pinned(&left, pane_height),
                        crate::tui::viewport::visible_with_pinned(&right, pane_height),
                        state.focus == CandidateFocus::Chat,
                    ))
                }
            })
        } else {
            None
        };
        if waiting {
            loading_tick += 1;
            let visible = crate::tui::render::loading_lines(loading_tick);
            let _ = terminal.draw(|frame| crate::tui::render::draw_loading_screen(frame, &visible));
        } else if let Some((left, right, chat_focused)) = &panes {
            let _ = terminal.draw(|frame| {
                crate::tui::render::draw_candidate_screen(frame, left, right, *chat_focused)
            });
        } else {
            let lines = if live {
                match runtime.as_ref() {
                    Some(active) => {
                        if state.show_help {
                            crate::tui::help::candidate_help_lines_for(
                                CandidateScreen::Solving,
                                state.focus,
                            )
                        } else {
                            live_lines(state, active, &status)
                        }
                    }
                    None => candidate_screen_lines(state),
                }
            } else {
                let mut lines = candidate_screen_lines(state);
                if let Some(text) = &status {
                    lines.push(text.clone());
                }
                lines
            };
            let visible =
                crate::tui::viewport::visible_with_pinned(&lines, height.saturating_sub(2));
            let centered = screen_centered(state.show_help, live, state.app.screen());
            let _ =
                terminal.draw(|frame| crate::tui::render::draw_screen(frame, &visible, centered));
        }
        if crossterm::event::poll(Duration::from_millis(250)).unwrap_or(false) {
            match crossterm::event::read() {
                Ok(Event::Key(key)) if key.kind != KeyEventKind::Release => {
                    if live {
                        let lost = runtime
                            .as_ref()
                            .map(|active| active.connection_lost())
                            .unwrap_or(false);
                        if lost
                            && key.code == K::Char('R')
                            && !key.modifiers.contains(KeyModifiers::CONTROL)
                        {
                            live = false;
                            *state = CandidateRunState::new();
                            stash = fresh_verifier(state);
                            status = Some(
                                "paste the fresh connection string, then press enter.".to_string(),
                            );
                        } else if let Some(active) = runtime.as_mut() {
                            handle_live_key(state, active, key, &mut status);
                        }
                    } else {
                        let connect_enter =
                            state.app.screen() == CandidateScreen::Connect && key.code == K::Enter;
                        handle_candidate_key(state, key);
                        if connect_enter && state.app.screen() == CandidateScreen::Rules {
                            let material = stash.lock().ok().and_then(|slot| slot.clone());
                            match material {
                                Some((conn, key_line)) => {
                                    if let Some(existing) = runtime.as_mut() {
                                        match existing.reconnect(&conn) {
                                            Ok(()) => {
                                                status = None;
                                            }
                                            Err(err) => {
                                                status = Some(
                                                    crate::tui::help::candidate_error_message(&err)
                                                        .to_string(),
                                                );
                                                *state = CandidateRunState::new();
                                                stash = fresh_verifier(state);
                                            }
                                        }
                                    } else {
                                        let mut built = CandidateRuntime::new_discovering(
                                            default_workspace(),
                                            SshCandidateTransport::from_verified(&conn, key_line),
                                        );
                                        match built.connect(&conn) {
                                            Ok(()) => {
                                                runtime = Some(built);
                                                status = None;
                                            }
                                            Err(err) => {
                                                status = Some(
                                                    crate::tui::help::candidate_error_message(&err)
                                                        .to_string(),
                                                );
                                                *state = CandidateRunState::new();
                                                stash = fresh_verifier(state);
                                            }
                                        }
                                    }
                                }
                                None => {
                                    status = Some(
                                        crate::tui::help::candidate_error_message(
                                            &CandidateRuntimeError::Transport,
                                        )
                                        .to_string(),
                                    );
                                    *state = CandidateRunState::new();
                                    stash = fresh_verifier(state);
                                }
                            }
                        }
                        if runtime.is_some() && state.app.rules_accepted() {
                            live = true;
                        }
                    }
                }
                Ok(Event::Paste(text)) => handle_candidate_paste(state, &text),
                _ => {}
            }
        }
    }
    if let Some(active) = runtime.as_mut() {
        let _ = active.end();
    }
    Ok(())
}
