//! Candidate live TTY loop and live key handling.

use super::*;

use super::panes::{
    connect_submit_shows_loading, divider_column, divider_hit, live_lines, live_panes,
    screen_centered, split_pct_for_column, DEFAULT_QUESTION_PCT,
};

/// Local workspace synced to the container `/workspace` root.
pub fn default_workspace() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("pairproof-workspace")
}
pub(crate) fn fresh_verifier(
    state: &mut CandidateRunState,
) -> Arc<Mutex<Option<(ConnectionString, String)>>> {
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
        crossterm::event::EnableBracketedPaste,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = ratatui::backend::CrosstermBackend::new(std::io::stdout());
    let mut terminal = ratatui::Terminal::new(backend)?;
    let mut stash = fresh_verifier(state);
    let mut runtime: Option<CandidateRuntime<SshCandidateTransport>> = None;
    let mut live = false;
    let mut status: Option<String> = None;
    let mut loading_tick = 0usize;
    let mut connect_pending = false;
    let mut question_pct = DEFAULT_QUESTION_PCT;
    let mut dragging_split = false;
    let mut question_scroll = 0usize;
    let mut scrolled_question: Option<usize> = None;
    while state.app.screen() != CandidateScreen::Exited {
        if live {
            if let Some(active) = runtime.as_mut() {
                if let Err(err) = active.poll_snapshot() {
                    status = Some(crate::tui::help::candidate_error_message(&err).to_string());
                }
                // A new question starts at the top of the pane.
                let current = active.view().question_index;
                if current != scrolled_question {
                    scrolled_question = current;
                    question_scroll = 0;
                }
            }
        }
        let height = terminal.size().map(|s| s.height as usize).unwrap_or(24);
        // While the statement is still arriving, the shared loading
        // screen replaces the placeholder solving screen entirely.
        // A deferred connection submit shows the same loading screen
        // before its blocking verification runs.
        let waiting = connect_pending
            || (live
                && !state.show_help
                && runtime.as_ref().is_some_and(|active| {
                    active.view().waiting_for_statement() && !active.connection_lost()
                }));
        let panes = if live && !state.show_help && !waiting {
            runtime.as_ref().and_then(|active| {
                if active.view().is_terminal() {
                    None
                } else {
                    let (left, right) = live_panes(state, active, &status);
                    let pane_height = height.saturating_sub(2);
                    Some((
                        left,
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
                crate::tui::render::draw_candidate_screen(
                    frame,
                    left,
                    right,
                    *chat_focused,
                    question_pct,
                    question_scroll,
                )
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
        if connect_pending {
            // The loading frame is on screen; now run the blocking
            // connection submit and runtime build behind it.
            connect_pending = false;
            handle_candidate_key(state, KeyEvent::new(K::Enter, KeyModifiers::NONE));
            if state.app.screen() == CandidateScreen::Rules {
                establish_runtime_after_rules(state, &mut runtime, &mut stash, &mut status);
            }
            if runtime.is_some() && state.app.rules_accepted() {
                live = true;
            }
        }
        if crossterm::event::poll(Duration::from_millis(250)).unwrap_or(false) {
            // Drain every queued event before the next snapshot/draw:
            // mouse drags emit many events, and processing only one
            // per snapshot makes pane resizing trail far behind.
            while let Ok(event) = crossterm::event::read() {
                match event {
                    Event::Key(key) if key.kind != KeyEventKind::Release => {
                        // Arrow/page keys scroll the question pane
                        // while the two panes are up; they were
                        // no-ops in the live key handler, so no
                        // candidate action changes.
                        let scrolls_pane = live
                            && panes.is_some()
                            && matches!(key.code, K::Up | K::Down | K::PageUp | K::PageDown);
                        if scrolls_pane {
                            match key.code {
                                K::Up => question_scroll = question_scroll.saturating_sub(1),
                                K::Down => question_scroll = question_scroll.saturating_add(1),
                                K::PageUp => question_scroll = question_scroll.saturating_sub(10),
                                K::PageDown => question_scroll = question_scroll.saturating_add(10),
                                _ => {}
                            }
                        } else if live {
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
                                    "paste the fresh connection string, then press enter."
                                        .to_string(),
                                );
                            } else if let Some(active) = runtime.as_mut() {
                                handle_live_key(state, active, key, &mut status);
                            }
                        } else {
                            let connect_enter = state.app.screen() == CandidateScreen::Connect
                                && key.code == K::Enter;
                            if connect_submit_shows_loading(
                                state.app.screen(),
                                state.app.input_len(),
                                state.show_help,
                            ) && connect_enter
                            {
                                // Show the loading frame first; the
                                // submit runs right after that draw.
                                connect_pending = true;
                                break;
                            }
                            handle_candidate_key(state, key);
                            if connect_enter && state.app.screen() == CandidateScreen::Rules {
                                establish_runtime_after_rules(
                                    state,
                                    &mut runtime,
                                    &mut stash,
                                    &mut status,
                                );
                            }
                            if runtime.is_some() && state.app.rules_accepted() {
                                live = true;
                            }
                        }
                    }
                    Event::Paste(text) => handle_candidate_paste(state, &text),
                    Event::Mouse(mouse) => {
                        // Dragging the pane divider resizes the solving
                        // panes; both keep at least 30% of the width.
                        use crossterm::event::{MouseButton, MouseEventKind};
                        let width = terminal.size().map(|s| s.width).unwrap_or(0);
                        match mouse.kind {
                            MouseEventKind::ScrollDown
                                if panes.is_some()
                                    && mouse.column < divider_column(width, question_pct) =>
                            {
                                question_scroll = question_scroll.saturating_add(3);
                            }
                            MouseEventKind::ScrollUp
                                if panes.is_some()
                                    && mouse.column < divider_column(width, question_pct) =>
                            {
                                question_scroll = question_scroll.saturating_sub(3);
                            }
                            MouseEventKind::Down(MouseButton::Left)
                                if panes.is_some()
                                    && divider_hit(mouse.column, width, question_pct) =>
                            {
                                dragging_split = true;
                            }
                            MouseEventKind::Drag(MouseButton::Left) if dragging_split => {
                                question_pct = split_pct_for_column(mouse.column, width);
                            }
                            MouseEventKind::Up(MouseButton::Left) => dragging_split = false,
                            _ => {}
                        }
                    }
                    _ => {}
                }
                if state.app.screen() == CandidateScreen::Exited
                    || !crossterm::event::poll(Duration::ZERO).unwrap_or(false)
                {
                    break;
                }
            }
        }
    }
    if let Some(active) = runtime.as_mut() {
        let _ = active.end();
    }
    Ok(())
}
