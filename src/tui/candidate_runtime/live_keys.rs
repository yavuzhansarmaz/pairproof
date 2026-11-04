//! Candidate live key handling: work/other/redirect/chat pane
//! keys mapped to control-channel intents and question lifecycle states.

use super::*;

use crate::tui::candidate_stage::QuestionFlowState;
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

/// Remembers a submitted approach so the status box can say the code
/// is being written before the expert publishes the selection.
fn remember_submit(
    state: &mut CandidateRunState,
    view: &CandidateView,
    selection: SnapshotSelection,
) {
    let Some(index) = view.question_index else {
        return;
    };
    state.pending_approach = Some(crate::tui::candidate_stage::PendingApproach {
        index,
        selection: selection.clone(),
    });
    state.staging.set_flow(
        index,
        QuestionFlowState::GeneratingInitialSolution {
            selection,
            base_revision: view.code_revision(),
        },
    );
    state.after_code = None;
}

/// Enter on the `other` row. An empty field opens the editor; text
/// already typed, or a row already marked, is submitted.
fn submit_other_row<T: CandidateTransport>(
    state: &mut CandidateRunState,
    runtime: &mut CandidateRuntime<T>,
    status: &mut Option<String>,
    view: &CandidateView,
) {
    let text = state.work.other_text().to_string();
    let marked_other = state
        .staging
        .current_stage()
        .and_then(|stage| stage.choice.clone())
        == Some(crate::tui::candidate_stage::StagedChoice::Other);
    if text.trim().is_empty() && !marked_other {
        state.staging.mark_cursor(view.options.len());
        state.focus = CandidateFocus::Other;
        return;
    }
    state.staging.mark_cursor(view.options.len());
    if apply(
        status,
        runtime.send_intent(IntentKind::SelectOther { text: text.clone() }),
    ) {
        remember_submit(state, view, SnapshotSelection::Other(text));
        state.work.set_other_text("");
        state.staging.save_other_text("");
    }
}

/// Settles pending operations against both the snapshot and real workspace sync.
pub(crate) fn settle_runtime<T: CandidateTransport>(
    state: &mut CandidateRunState,
    runtime: &CandidateRuntime<T>,
    view: &CandidateView,
) {
    let synced = view
        .question_index
        .is_none_or(|idx| runtime.is_solution_synced(idx));
    crate::tui::candidate_stage::settle_with_sync(state, view, synced, runtime.sync_failed());
}

/// Settles pending operations from the view alone (used when no runtime is attached).
pub(crate) fn settle_pending(state: &mut CandidateRunState, view: &CandidateView) {
    crate::tui::candidate_stage::settle_with_sync(state, view, true, false);
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

/// Blocks arrow-key question switching: the candidate stays on the
/// current question until pressing `f`, and submitted questions stay locked.
fn navigate(status: &mut Option<String>, delta: i32) {
    if delta < 0 {
        *status = Some("submitted questions cannot be reopened.".to_string());
    } else if delta > 0 {
        *status = Some("press f to submit this question and advance to the next.".to_string());
    }
}

fn has_solution<T: CandidateTransport>(
    runtime: &CandidateRuntime<T>,
    view: &CandidateView,
) -> bool {
    view.question_index.is_none()
        || view.selected.is_some()
        || view.question_index.is_some_and(|i| {
            crate::tui::candidate_sync::has_synced_solution(runtime.workspace(), i)
        })
}

fn flush_before_action<T: CandidateTransport>(
    runtime: &mut CandidateRuntime<T>,
    status: &mut Option<String>,
) -> bool {
    match runtime.flush() {
        Ok(()) => true,
        Err(err) => {
            *status = Some(crate::tui::help::candidate_error_message(&err).to_string());
            false
        }
    }
}

fn guard_question_action(
    state: &CandidateRunState,
    view: &CandidateView,
    status: &mut Option<String>,
    submitting_attempt: bool,
) -> bool {
    let Some(index) = view.question_index else {
        return true;
    };
    let flow = state.staging.flow_for(index);
    if flow.is_submitted_or_submitting()
        || view
            .nav
            .iter()
            .any(|n| n.index == index && n.state == "Finished")
    {
        *status = Some("this question has already been submitted.".to_string());
        return false;
    }
    if flow.is_busy() {
        *status = Some(if submitting_attempt {
            "wait for the current operation to finish before submitting.".to_string()
        } else {
            "working on the current step; wait a moment.".to_string()
        });
        return false;
    }
    true
}

fn run_solution_action<T: CandidateTransport>(
    state: &mut CandidateRunState,
    runtime: &mut CandidateRuntime<T>,
    status: &mut Option<String>,
    view: &CandidateView,
) {
    if !guard_question_action(state, view, status, false) {
        return;
    }
    if !has_solution(runtime, view) {
        *status = Some("select an approach and generate a solution first.".to_string());
        return;
    }
    if !flush_before_action(runtime, status) {
        return;
    }
    if apply(status, runtime.send_intent(IntentKind::RunSolution)) {
        if let Some(index) = view.question_index {
            state.staging.set_flow(
                index,
                QuestionFlowState::RunningTests {
                    base_seq: view.run_seq(),
                },
            );
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
    if runtime.connection_lost() && !runtime.view().is_terminal() {
        if key.code == K::Enter {
            state.app.quit();
        }
        return;
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
    settle_runtime(state, runtime, &view);
    match state.focus {
        CandidateFocus::Work => match key.code {
            K::Tab => state.focus = next_focus(state.focus),
            K::Up => state.staging.move_cursor(-1, view.options.len()),
            K::Down => state.staging.move_cursor(1, view.options.len()),
            K::Left => navigate(status, -1),
            K::Right => navigate(status, 1),
            K::Char(' ') if !control => {
                if !guard_question_action(state, &view, status, false) {
                    return;
                }
                state.staging.mark_cursor(view.options.len());
                if cursor_on_other(state, view.options.len()) {
                    state.focus = CandidateFocus::Other;
                }
            }
            K::Enter => {
                if !guard_question_action(state, &view, status, false) {
                    return;
                }
                let option_count = view.options.len();
                if option_count == 0 {
                    *status = Some("loading".to_string());
                } else if cursor_on_other(state, option_count) {
                    submit_other_row(state, runtime, status, &view);
                } else if let Some(preset) = state
                    .staging
                    .current_stage()
                    .map(|stage| stage.cursor as u8)
                    .filter(|preset| (*preset as usize) < option_count)
                {
                    state.staging.mark_cursor(option_count);
                    if apply(
                        status,
                        runtime.send_intent(IntentKind::SelectPreset { preset }),
                    ) {
                        remember_submit(state, &view, SnapshotSelection::Preset(preset));
                    }
                }
            }
            K::Char(c) if !control && cursor_on_other(state, view.options.len()) => {
                if !guard_question_action(state, &view, status, false) {
                    return;
                }
                state.staging.mark_cursor(view.options.len());
                state.focus = CandidateFocus::Other;
                let _ = other_insert(&mut state.work, &c.to_string(), false);
                state.staging.save_other_text(state.work.other_text());
            }
            K::Char(c) if !control => match work_key_action(c) {
                Some(WorkKeyAction::SelectPreset(preset)) => {
                    if !guard_question_action(state, &view, status, false) {
                        return;
                    }
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
                    if !guard_question_action(state, &view, status, false) {
                        return;
                    }
                    state.staging.cursor_to_other(view.options.len());
                    state.staging.mark_cursor(view.options.len());
                    state.focus = CandidateFocus::Other;
                }
                Some(WorkKeyAction::Continue) => {
                    if !guard_question_action(state, &view, status, false) {
                        return;
                    }
                    if !has_solution(runtime, &view) && state.pending_approach.is_none() {
                        *status = Some("select an approach and press enter first.".to_string());
                        return;
                    }
                    if let Some(index) = view.question_index {
                        state
                            .staging
                            .set_flow(index, QuestionFlowState::ContinuingInteraction);
                    }
                    state.focus = CandidateFocus::Redirect;
                }
                Some(WorkKeyAction::ChangeApproach) => {
                    if !guard_question_action(state, &view, status, false) {
                        return;
                    }
                    if apply(status, runtime.send_intent(IntentKind::ChangeApproach)) {
                        if let Some(index) = view.question_index {
                            state
                                .staging
                                .set_flow(index, QuestionFlowState::AwaitingApproach);
                        }
                        state.pending_approach = None;
                        state.after_code = None;
                    }
                }
                Some(WorkKeyAction::RunSolution) => {
                    run_solution_action(state, runtime, status, &view);
                }
                Some(WorkKeyAction::RequestTests) => {
                    if !guard_question_action(state, &view, status, false) {
                        return;
                    }
                    if !has_solution(runtime, &view) {
                        *status =
                            Some("select an approach and generate a solution first.".to_string());
                        return;
                    }
                    if !flush_before_action(runtime, status) {
                        return;
                    }
                    if apply(status, runtime.send_intent(IntentKind::RequestTests)) {
                        if let Some(index) = view.question_index {
                            state.staging.set_flow(
                                index,
                                QuestionFlowState::UpdatingTests {
                                    instruction: "generate tests".into(),
                                    base_revision: view.code_revision(),
                                },
                            );
                        }
                    }
                }
                Some(WorkKeyAction::Finish) => {
                    if !guard_question_action(state, &view, status, true) {
                        return;
                    }
                    if !has_solution(runtime, &view) {
                        *status =
                            Some("select an approach and generate a solution first.".to_string());
                        return;
                    }
                    if !flush_before_action(runtime, status) {
                        return;
                    }
                    if apply(status, runtime.send_intent(IntentKind::Finish)) {
                        if let Some(index) = view.question_index {
                            state.staging.set_flow(index, QuestionFlowState::Submitting);
                        }
                        state.pending_approach = None;
                        state.after_code = None;
                    }
                }
                None => {}
            },
            _ => {}
        },
        CandidateFocus::Other => match key.code {
            K::Tab => state.focus = next_focus(state.focus),
            K::Enter => {
                if !guard_question_action(state, &view, status, false) {
                    return;
                }
                let text = state.work.other_text().to_string();
                if apply(
                    status,
                    runtime.send_intent(IntentKind::SelectOther { text: text.clone() }),
                ) {
                    remember_submit(state, &view, SnapshotSelection::Other(text));
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
            K::Tab => {
                if let Some(index) = view.question_index {
                    if state.staging.flow_for(index) == QuestionFlowState::ContinuingInteraction {
                        state
                            .staging
                            .set_flow(index, QuestionFlowState::SolutionReady);
                    }
                }
                state.focus = next_focus(state.focus);
            }
            K::Enter => {
                if !guard_question_action(state, &view, status, false) {
                    return;
                }
                if !flush_before_action(runtime, status) {
                    return;
                }
                let text = state.work.redirect_text().trim().to_string();
                let base_revision = view.code_revision();
                if text.is_empty() {
                    if let Some(index) = view.question_index {
                        state
                            .staging
                            .set_flow(index, QuestionFlowState::SolutionReady);
                    }
                    state.after_code = None;
                    state.focus = CandidateFocus::Work;
                    *status = Some(
                        "continue cancelled. type instructions after c to update the code."
                            .to_string(),
                    );
                } else if apply(
                    status,
                    runtime.send_intent(IntentKind::Redirect { text: text.clone() }),
                ) {
                    if let Some(index) = view.question_index {
                        let next_flow =
                            if crate::orchestration::coding::instruction_wants_tests(&text) {
                                QuestionFlowState::UpdatingTests {
                                    instruction: text,
                                    base_revision,
                                }
                            } else {
                                QuestionFlowState::UpdatingSolution {
                                    instruction: Some(text),
                                    base_revision,
                                }
                            };
                        state.staging.set_flow(index, next_flow);
                    }
                    state.after_code = None;
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
                    state.chat.remember_sent(view.question_index, &text);
                    state.chat.clear_input();
                }
            }
            K::Backspace => state.chat.backspace_input(),
            K::Char(c) if !control => state.chat.type_char(c),
            _ => {}
        },
    }
}
