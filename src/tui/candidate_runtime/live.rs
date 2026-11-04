//! Candidate live TTY loop and live key handling.

use super::*;

use super::panes::{
    clamp_scroll, coach_pane_width, coach_scroll_rows, connect_submit_shows_loading, divider_hit,
    focus_for_pane, live_lines, live_panes, max_scroll, pane_click_target, pane_content_width,
    pane_side_for_column, question_pane_width, question_pinned_count, question_scroll_rows,
    screen_centered, scroll_after_growth, split_pct_for_column, PaneSide, DEFAULT_QUESTION_PCT,
};
use crate::tui::viewport::wrapped_row_count;

/// Cycles `loading.` and `writing …. this takes a moment.` the same
/// way as the full-screen loading panel: one, two, then three dots.
fn animate_loading(lines: &mut [String], tick: usize) {
    let dots = ".".repeat(tick % 3 + 1);
    for line in lines {
        if line == "loading" {
            *line = format!("loading{dots}");
        } else if let Some(rest) = line.strip_prefix("working:") {
            *line = format!("working: {}{dots}", rest.trim());
        }
    }
}

/// The folder the candidate launched from. Mutagen syncs it with the
/// container workspace, so `q1/`, `q2/`, and so on appear here.
pub fn default_workspace() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir())
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
    let mut connect_rx: Option<std::sync::mpsc::Receiver<ConnectOutcome>> = None;
    let mut poll_rx: Option<std::sync::mpsc::Receiver<super::connect_work::BackgroundPollOutcome>> =
        None;
    let mut question_pct = DEFAULT_QUESTION_PCT;
    let mut dragging_split = false;
    let mut question_scroll = 0usize;
    let mut coach_scroll = 0usize;
    let mut final_scroll = 0usize;
    let mut final_max_scroll = 0usize;
    let mut final_page = 1usize;
    let mut coach_following = true;
    let mut scrolled_question: Option<usize> = None;
    let mut last_snapshot_poll = std::time::Instant::now() - Duration::from_secs(2);
    let mut last_live_hb = std::time::Instant::now();
    let mut breakdown_open = false;
    let mut final_scrolling = false;
    let mut held_view = None;
    let mut final_doc: Option<Vec<String>> = None;
    while state.app.screen() != CandidateScreen::Exited {
        // Apply a finished background connection, if one landed; while
        // it runs the loop keeps drawing the animated loading screen.
        match connect_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(ConnectOutcome::SubmitFailed(err))) => {
                connect_rx = None;
                state.app.record_connect_error(err);
            }
            Some(Ok(ConnectOutcome::Failed(err))) => {
                connect_rx = None;
                runtime = None;
                super::reset_after_connect_failure(state, &mut stash, &mut status, &err);
            }
            Some(Ok(ConnectOutcome::Connected(built))) => {
                connect_rx = None;
                state.app.mark_connection_verified();
                runtime = Some(*built);
                status = None;
                last_live_hb = std::time::Instant::now();
            }
            Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                connect_rx = None;
                runtime = None;
                super::reset_after_connect_failure(
                    state,
                    &mut stash,
                    &mut status,
                    &CandidateRuntimeError::Transport,
                );
            }
            Some(Err(std::sync::mpsc::TryRecvError::Empty)) | None => {}
        }
        // Snapshot polling runs on a background worker thread so the
        // TTY loop never freezes on SSH or Mutagen during loading or solving.
        match poll_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(outcome)) => {
                poll_rx = None;
                last_snapshot_poll = std::time::Instant::now();
                if let Some(active) = runtime.as_mut() {
                    if let Err(err) = active.apply_background_poll(outcome) {
                        status = Some(crate::tui::help::candidate_error_message(&err).to_string());
                    }
                }
            }
            Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                poll_rx = None;
                last_snapshot_poll = std::time::Instant::now();
            }
            Some(Err(std::sync::mpsc::TryRecvError::Empty)) | None => {}
        }
        let poll_interval = if breakdown_open {
            Duration::from_secs(1)
        } else {
            Duration::from_millis(350)
        };
        // Heartbeat is independent of snapshot polling so a slow
        // Mutagen flush or SSH read cannot look like a disconnect.
        if let Some(active) = runtime.as_ref() {
            if last_live_hb.elapsed() >= Duration::from_secs(5) {
                last_live_hb = std::time::Instant::now();
                active.spawn_background_heartbeat(now_ms());
            }
        }
        if !live {
            held_view = None;
            poll_rx = None;
        } else if !final_scrolling {
            final_doc = None;
            if !dragging_split && poll_rx.is_none() && last_snapshot_poll.elapsed() >= poll_interval
            {
                if let Some(active) = runtime.as_mut() {
                    poll_rx = active.spawn_background_poll(now_ms());
                }
            }
            held_view = runtime.as_ref().map(|active| active.view());
        }
        let tick_view = held_view.as_ref();
        let connection_lost = runtime
            .as_ref()
            .map(|active| active.connection_lost())
            .unwrap_or(false);
        if live {
            if let Some(view) = &tick_view {
                super::live_keys::sync_staging(state, view);
                if let Some(active) = runtime.as_ref() {
                    super::live_keys::settle_runtime(state, active, view);
                } else {
                    super::live_keys::settle_pending(state, view);
                }
                // A new question starts at the top of the pane.
                let current = view.question_index;
                if current != scrolled_question {
                    scrolled_question = current;
                    question_scroll = 0;
                    coach_scroll = 0;
                    coach_following = true;
                }
            }
        }
        let (term_width, height) = terminal
            .size()
            .map(|s| (s.width, s.height as usize))
            .unwrap_or((0, 24));
        // While the statement is still arriving, the shared loading
        // screen replaces the placeholder solving screen entirely.
        // A background connection submit shows the same loading
        // screen, animated, until its result lands.
        loading_tick = loading_tick.wrapping_add(1);
        let waiting = connect_rx.is_some()
            || super::panes::live_waiting_for_question(
                live,
                state.show_help,
                connection_lost,
                tick_view,
            );
        let panes = if live && !state.show_help && !waiting && !connection_lost {
            tick_view.as_ref().and_then(|view| {
                if view.is_terminal() {
                    None
                } else {
                    let workspace = runtime
                        .as_ref()
                        .map(|active| active.workspace().to_path_buf());
                    let (mut left, right) =
                        live_panes(state, view, connection_lost, &status, workspace.as_deref());
                    animate_loading(&mut left, loading_tick);
                    Some((left, right, state.focus == CandidateFocus::Chat))
                }
            })
        } else {
            None
        };
        let question_width = pane_content_width(question_pane_width(term_width, question_pct));
        let coach_width = pane_content_width(coach_pane_width(term_width, question_pct));
        let mut coach_visible = coach_scroll_rows(height, 0);
        let mut coach_total_wrapped = 0usize;
        if let Some((left, right, _)) = &panes {
            let pinned = question_pinned_count(left);
            let question_visible = question_scroll_rows(height)
                .saturating_sub(pinned.saturating_sub(1))
                .max(1);
            let left_content = &left[..left.len().saturating_sub(pinned)];
            let right_content: &[String] = match right.last() {
                Some(last) if last.starts_with("message:") => {
                    &right[..right.len().saturating_sub(1)]
                }
                _ => right,
            };
            let current_flow = state.staging.current_flow();
            let status_rows = tick_view
                .as_ref()
                .map(|view| {
                    view.status_lines_with_flow(
                        state.pending_approach.as_ref(),
                        Some(&current_flow),
                    )
                    .len()
                })
                .unwrap_or(0);
            coach_visible = coach_scroll_rows(height, status_rows);
            let question_total_wrapped = wrapped_row_count(left_content, question_width);
            coach_total_wrapped = wrapped_row_count(right_content, coach_width);
            coach_scroll = scroll_after_growth(
                coach_scroll,
                coach_following,
                coach_total_wrapped,
                coach_visible,
            );
            question_scroll =
                clamp_scroll(question_scroll, question_total_wrapped, question_visible);
        }
        if waiting {
            breakdown_open = false;
            final_scrolling = false;
            final_doc = None;
            let visible = crate::tui::render::loading_lines(loading_tick);
            let _ = terminal.draw(|frame| crate::tui::render::draw_loading_screen(frame, &visible));
        } else if let Some((left, right, chat_focused)) = &panes {
            breakdown_open = false;
            final_scrolling = false;
            final_doc = None;
            let current_flow = state.staging.current_flow();
            let status_lines = tick_view
                .as_ref()
                .map(|view| {
                    view.status_lines_with_flow(
                        state.pending_approach.as_ref(),
                        Some(&current_flow),
                    )
                })
                .unwrap_or_default();
            let _ = terminal.draw(|frame| {
                crate::tui::render::draw_candidate_screen_with_status(
                    frame,
                    left,
                    &status_lines,
                    right,
                    *chat_focused,
                    question_pct,
                    question_scroll,
                    coach_scroll,
                )
            });
        } else {
            let lines = if live {
                match tick_view.as_ref() {
                    Some(view) => {
                        if state.show_help {
                            crate::tui::help::candidate_help_lines_for(
                                CandidateScreen::Solving,
                                state.focus,
                            )
                        } else if view.is_terminal() {
                            if let Some(doc) = final_doc.clone() {
                                doc
                            } else {
                                let built = view.final_screen_lines();
                                final_doc = Some(built.clone());
                                built
                            }
                        } else if connection_lost {
                            CandidateView::expert_closed_lines()
                        } else {
                            live_lines(state, view, connection_lost, &status)
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
            let inner_height = height.saturating_sub(2);
            let breakdown = lines.first().is_some_and(|line| line == "Final breakdown");
            breakdown_open = breakdown;
            if !breakdown {
                final_scrolling = false;
                final_doc = None;
            }
            let visible = if breakdown {
                let frame = crate::tui::viewport::final_frame(
                    &lines,
                    inner_height,
                    final_scroll,
                    term_width,
                );
                final_scroll = frame.scroll;
                final_max_scroll = frame.max_scroll;
                final_page = frame.page;
                frame.lines
            } else {
                crate::tui::viewport::visible_with_pinned(&lines, inner_height)
            };
            let centered = screen_centered(state.show_help, live, state.app.screen())
                || connection_lost
                || tick_view.as_ref().is_some_and(|view| view.is_terminal());
            let _ =
                terminal.draw(|frame| crate::tui::render::draw_screen(frame, &visible, centered));
        }
        if crossterm::event::poll(Duration::from_millis(250)).unwrap_or(false) {
            // Drag events are drained together so the split follows
            // the cursor. A final-breakdown scroll paints after one
            // step, so a wheel burst moves row by row instead of
            // jumping.
            while let Ok(event) = crossterm::event::read() {
                let mut step_final = false;
                match event {
                    Event::Key(key) if key.kind != KeyEventKind::Release => {
                        if connect_rx.is_some() {
                            // Connecting in the background: only quit
                            // keys are live; the rest is ignored so
                            // the attempt cannot be re-entered.
                            if key.code == K::Esc
                                || (key.code == K::Char('c')
                                    && key.modifiers.contains(KeyModifiers::CONTROL))
                            {
                                state.app.quit();
                            }
                        }
                        // The final breakdown is one scrolling page.
                        // Up/down move it; Enter still closes.
                        let scrolls_final = connect_rx.is_none()
                            && live
                            && !state.show_help
                            && panes.is_none()
                            && tick_view.as_ref().is_some_and(|view| {
                                view.is_terminal()
                                    && (view.final_breakdown.is_some()
                                        || view.phase == "TechnicalFailure")
                            })
                            && matches!(
                                key.code,
                                K::Up | K::Down | K::PageUp | K::PageDown | K::Home | K::End
                            );
                        if scrolls_final {
                            match key.code {
                                K::Up => {
                                    final_scroll = final_scroll.saturating_sub(1);
                                }
                                K::Down => {
                                    final_scroll =
                                        final_scroll.saturating_add(1).min(final_max_scroll);
                                }
                                K::PageUp => {
                                    final_scroll = final_scroll.saturating_sub(final_page);
                                }
                                K::PageDown => {
                                    final_scroll = final_scroll
                                        .saturating_add(final_page)
                                        .min(final_max_scroll);
                                }
                                K::Home => final_scroll = 0,
                                K::End => final_scroll = final_max_scroll,
                                _ => {}
                            }
                            final_scrolling = true;
                            break;
                        }
                        // Arrow/page keys scroll the question pane
                        // while the two panes are up; they were
                        // no-ops in the live key handler, so no
                        // candidate action changes.
                        let scrolls_pane = connect_rx.is_none()
                            && live
                            && panes.is_some()
                            && match key.code {
                                K::Up | K::Down => state.focus == CandidateFocus::Chat,
                                K::PageUp | K::PageDown => true,
                                _ => false,
                            };
                        if connect_rx.is_some() {
                            // Already handled above; swallow the key.
                        } else if scrolls_pane {
                            if state.focus == CandidateFocus::Chat {
                                match key.code {
                                    K::Up => coach_scroll = coach_scroll.saturating_sub(1),
                                    K::Down => coach_scroll = coach_scroll.saturating_add(1),
                                    K::PageUp => coach_scroll = coach_scroll.saturating_sub(10),
                                    K::PageDown => coach_scroll = coach_scroll.saturating_add(10),
                                    _ => {}
                                }
                                coach_following =
                                    coach_scroll >= max_scroll(coach_total_wrapped, coach_visible);
                            } else {
                                match key.code {
                                    K::Up => question_scroll = question_scroll.saturating_sub(1),
                                    K::Down => question_scroll = question_scroll.saturating_add(1),
                                    K::PageUp => {
                                        question_scroll = question_scroll.saturating_sub(10)
                                    }
                                    K::PageDown => {
                                        question_scroll = question_scroll.saturating_add(10)
                                    }
                                    _ => {}
                                }
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
                                break;
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
                                // Start the blocking connection on a
                                // worker thread; the loop keeps drawing
                                // the animated loading screen and polls
                                // the result at the top of each tick.
                                let input = state.app.connect_input();
                                let worker_stash = Arc::clone(&stash);
                                let existing = runtime.take();
                                connect_rx = Some(spawn_connect_work(move || {
                                    run_connect_work(input, now_ms(), worker_stash, existing)
                                }));
                                break;
                            }
                            handle_candidate_key(state, key);
                            if connect_enter && state.app.screen() == CandidateScreen::Rules {
                                // The submit already verified on this
                                // thread (the path above was not taken);
                                // still build the runtime in the
                                // background behind the loading screen.
                                let worker_stash = Arc::clone(&stash);
                                let existing = runtime.take();
                                connect_rx = Some(spawn_connect_work(move || {
                                    run_runtime_build(existing, worker_stash)
                                }));
                            }
                            if runtime.is_some() && state.app.rules_accepted() {
                                live = true;
                                break;
                            }
                        }
                    }
                    Event::Paste(text) => {
                        if connect_rx.is_none() {
                            handle_candidate_paste(state, &text)
                        }
                    }
                    Event::Mouse(mouse) if connect_rx.is_none() => {
                        // Dragging the pane divider resizes the solving
                        // panes; both keep at least 30% of the width.
                        use crossterm::event::{MouseButton, MouseEventKind};
                        let width = term_width;
                        match mouse.kind {
                            MouseEventKind::ScrollDown
                                if panes.is_none() && final_max_scroll > 0 =>
                            {
                                final_scroll = final_scroll.saturating_add(1).min(final_max_scroll);
                                step_final = true;
                            }
                            MouseEventKind::ScrollUp if panes.is_none() && final_max_scroll > 0 => {
                                final_scroll = final_scroll.saturating_sub(1);
                                step_final = true;
                            }
                            MouseEventKind::ScrollDown if panes.is_some() => {
                                match pane_side_for_column(mouse.column, width, question_pct) {
                                    PaneSide::Question => {
                                        question_scroll = question_scroll.saturating_add(3)
                                    }
                                    PaneSide::Coach => {
                                        coach_scroll = coach_scroll.saturating_add(3);
                                        coach_following = coach_scroll
                                            >= max_scroll(coach_total_wrapped, coach_visible);
                                    }
                                }
                            }
                            MouseEventKind::ScrollUp if panes.is_some() => {
                                match pane_side_for_column(mouse.column, width, question_pct) {
                                    PaneSide::Question => {
                                        question_scroll = question_scroll.saturating_sub(3)
                                    }
                                    PaneSide::Coach => {
                                        coach_scroll = coach_scroll.saturating_sub(3);
                                        coach_following = coach_scroll
                                            >= max_scroll(coach_total_wrapped, coach_visible);
                                    }
                                }
                            }
                            MouseEventKind::Down(MouseButton::Left)
                                if panes.is_some()
                                    && divider_hit(mouse.column, width, question_pct) =>
                            {
                                dragging_split = true;
                            }
                            MouseEventKind::Down(MouseButton::Left) if panes.is_some() => {
                                state.focus = focus_for_pane(
                                    state.focus,
                                    pane_click_target(mouse.column, width, question_pct),
                                );
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
                if step_final {
                    final_scrolling = true;
                }
                if step_final
                    || state.app.screen() == CandidateScreen::Exited
                    || !crossterm::event::poll(Duration::ZERO).unwrap_or(false)
                {
                    break;
                }
            }
        } else {
            final_scrolling = false;
        }
    }
    if let Some(active) = runtime.as_mut() {
        // Leaving a live session (Esc/Ctrl-C): signal the expert
        // before tearing sync down, so its monitor reflects the
        // disconnect immediately rather than at heartbeat timeout.
        if live && !active.view().is_terminal() {
            let _ = active.disconnect();
        }
        let _ = active.end();
    }
    Ok(())
}
