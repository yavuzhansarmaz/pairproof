//! Candidate run state, key and paste dispatch, and the terminal guard.

use std::collections::BTreeMap;

use crossterm::event::{KeyCode as K, KeyEvent, KeyModifiers};

use crate::core::Session;
use crate::llm::chat::ChatState;
use crate::tui::candidate::{CandidateApp, CandidateScreen};
use crate::tui::shared::pop_last;

use crate::tui::candidate_actions::{
    UnavailableCodingRunner, UnavailableOtherGrader, UnavailableTestRunner,
};
use crate::tui::candidate_chat::{other_insert, ChatController};
use crate::tui::candidate_final::{candidate_final, candidate_final_lines};
use crate::tui::candidate_work::{work_lines, CandidateWork};
use crate::tui::help::{candidate_help_lines_for, candidate_is_typing};
pub use crate::tui::shared::now_ms;

/// Focus pane while solving; Tab cycles Work, Other, Redirect, Chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CandidateFocus {
    #[default]
    Work,
    Other,
    Redirect,
    Chat,
}

/// One work-pane key action, shared by the offline and live handlers
/// so both paths map the same keys to the same actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkKeyAction {
    SelectPreset(u8),
    OpenOther,
    Continue,
    OpenRedirect,
    ChangeApproach,
    RequestTests,
    Finish,
    Skip,
    RequestReQuestion,
}

/// Classifies a typed work-pane character; `None` is not a work key.
pub fn work_key_action(c: char) -> Option<WorkKeyAction> {
    match c {
        '1'..='4' => Some(WorkKeyAction::SelectPreset(c as u8 - b'1')),
        '5' | 'o' => Some(WorkKeyAction::OpenOther),
        'c' => Some(WorkKeyAction::Continue),
        'd' => Some(WorkKeyAction::OpenRedirect),
        'a' => Some(WorkKeyAction::ChangeApproach),
        't' => Some(WorkKeyAction::RequestTests),
        'f' => Some(WorkKeyAction::Finish),
        's' => Some(WorkKeyAction::Skip),
        'r' => Some(WorkKeyAction::RequestReQuestion),
        _ => None,
    }
}

/// Next pane in the Tab cycle: Work, Other, Redirect, Chat, Work.
pub fn next_focus(focus: CandidateFocus) -> CandidateFocus {
    match focus {
        CandidateFocus::Work => CandidateFocus::Other,
        CandidateFocus::Other => CandidateFocus::Redirect,
        CandidateFocus::Redirect => CandidateFocus::Chat,
        CandidateFocus::Chat => CandidateFocus::Work,
    }
}

/// Pure candidate run state; runners are fresh unavailable values.
pub struct CandidateRunState {
    pub app: CandidateApp,
    pub work: CandidateWork,
    pub chat: ChatController,
    pub focus: CandidateFocus,
    pub paste_notice: bool,
    pub show_help: bool,
    /// Set when the candidate backs out of Connect with an empty
    /// Enter; the caller returns to the main menu instead of exiting.
    pub back_to_menu: bool,
}

impl CandidateRunState {
    /// Creates run state with the production unavailable seams.
    pub fn new() -> Self {
        Self::with_parts(
            CandidateApp::new(),
            CandidateWork::new(),
            ChatController::default(),
        )
    }

    /// Creates run state from injected parts, mainly for tests.
    pub fn with_parts(app: CandidateApp, work: CandidateWork, chat: ChatController) -> Self {
        Self {
            app,
            work,
            chat,
            focus: CandidateFocus::Work,
            paste_notice: false,
            show_help: false,
            back_to_menu: false,
        }
    }

    fn with_session(&mut self, action: impl FnOnce(&mut CandidateWork, &mut Session)) {
        if let Some(session) = self.app.session_mut() {
            action(&mut self.work, session);
        }
    }
}

impl Default for CandidateRunState {
    /// Creates run state with the production unavailable seams.
    fn default() -> Self {
        Self::new()
    }
}

fn submit_chat(state: &mut CandidateRunState) {
    let Some(session_ref) = state.app.session() else {
        return;
    };
    let statement = match session_ref.current_slot() {
        Some(slot) => state
            .work
            .statement_for(&slot.question.title_slug)
            .map(str::to_string)
            .unwrap_or_else(|| slot.question.title.clone()),
        None => String::new(),
    };
    let code = state.work.current_code().to_string();
    let chat_state = ChatState::from_session(session_ref, &statement, &code);
    if let Some(session) = state.app.session_mut() {
        let _ = state.chat.submit(session, &chat_state, now_ms());
    }
}

fn handle_work_key(state: &mut CandidateRunState, key: KeyEvent, control: bool) {
    match key.code {
        K::Tab => state.focus = next_focus(state.focus),
        K::Char(c) if !control => match work_key_action(c) {
            Some(WorkKeyAction::SelectPreset(preset)) => {
                state.with_session(|work, session| {
                    let _ =
                        work.select_preset(session, preset, &mut UnavailableCodingRunner, now_ms());
                });
            }
            Some(WorkKeyAction::OpenOther) => state.focus = CandidateFocus::Other,
            Some(WorkKeyAction::Continue) => state.with_session(|work, session| {
                let _ = work.continue_work(session, &mut UnavailableCodingRunner, now_ms());
            }),
            Some(WorkKeyAction::OpenRedirect) => state.focus = CandidateFocus::Redirect,
            Some(WorkKeyAction::ChangeApproach) => state.with_session(|work, session| {
                let _ = work.change_approach(session, now_ms());
            }),
            Some(WorkKeyAction::RequestTests) => state.with_session(|work, session| {
                let _ = work.request_tests(session, &mut UnavailableTestRunner, now_ms());
            }),
            Some(WorkKeyAction::Finish) => state.with_session(|work, session| {
                let _ = work.finish_question(session, now_ms());
            }),
            Some(WorkKeyAction::Skip) => state.with_session(|work, session| {
                let _ = work.skip_question(session, now_ms());
            }),
            Some(WorkKeyAction::RequestReQuestion) => state.with_session(|work, session| {
                let _ = work.request_requestion(session, now_ms());
            }),
            None => {}
        },
        _ => {}
    }
}

fn handle_other_key(state: &mut CandidateRunState, key: KeyEvent, control: bool) {
    match key.code {
        K::Tab => state.focus = next_focus(state.focus),
        K::Enter => {
            state.with_session(|work, session| {
                let _ = work.select_other_graded(
                    session,
                    &mut UnavailableOtherGrader,
                    &mut UnavailableCodingRunner,
                    now_ms(),
                );
            });
            // select_other clears the text only on success; keep the
            // pane focused when the attempt left the text in place.
            if state.work.other_text().is_empty() {
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
    }
}

fn handle_redirect_key(state: &mut CandidateRunState, key: KeyEvent, control: bool) {
    match key.code {
        K::Tab => state.focus = next_focus(state.focus),
        K::Enter => {
            state.with_session(|work, session| {
                let _ = work.redirect(session, &mut UnavailableCodingRunner, now_ms());
            });
            if state.work.redirect_text().is_empty() {
                state.focus = CandidateFocus::Work;
            }
        }
        K::Backspace => {
            let text = pop_last(state.work.redirect_text());
            state.work.set_redirect_text(&text);
        }
        K::Char(c) if !control => state.work.push_redirect_char(c),
        _ => {}
    }
}

fn handle_chat_key(state: &mut CandidateRunState, key: KeyEvent, control: bool) {
    match key.code {
        K::Tab => state.focus = next_focus(state.focus),
        K::Enter => submit_chat(state),
        K::Backspace => state.chat.backspace_input(),
        K::Char(c) if !control => state.chat.type_char(c),
        _ => {}
    }
}

/// Maps one key press to a candidate action for the current screen.
pub fn handle_candidate_key(state: &mut CandidateRunState, key: KeyEvent) {
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
    if key.code == K::Char('?') && !control && !candidate_is_typing(state.app.screen(), state.focus)
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
    match state.app.screen() {
        CandidateScreen::Connect => match key.code {
            K::Enter => {
                // `b` types into the connection string, so an empty
                // Enter is the way back to the main menu.
                if state.app.input_len() == 0 {
                    state.back_to_menu = true;
                    state.app.quit();
                } else {
                    let _ = state.app.submit_connection(now_ms());
                }
            }
            K::Backspace => state.app.backspace(),
            K::Char(c) if !control => state.app.push_char(c),
            _ => {}
        },
        CandidateScreen::Rules => {
            if key.code == K::Enter {
                let _ = state.app.accept_rules();
            }
        }
        CandidateScreen::Waiting => {}
        CandidateScreen::Final => {
            if key.code == K::Enter {
                state.app.quit();
            }
        }
        CandidateScreen::Solving => match state.focus {
            CandidateFocus::Work => handle_work_key(state, key, control),
            CandidateFocus::Other => handle_other_key(state, key, control),
            CandidateFocus::Redirect => handle_redirect_key(state, key, control),
            CandidateFocus::Chat => handle_chat_key(state, key, control),
        },
        CandidateScreen::Exited => {}
    }
}

/// Routes a paste by screen; solving pastes are always blocked.
pub fn handle_candidate_paste(state: &mut CandidateRunState, text: &str) {
    match state.app.screen() {
        CandidateScreen::Connect => state.app.replace_text(text),
        CandidateScreen::Solving => match state.focus {
            CandidateFocus::Chat => {
                let _ = state.chat.paste_input(text);
                state.paste_notice = true;
            }
            _ => state.paste_notice = true,
        },
        _ => {}
    }
}

/// Builds candidate screen lines; every producer below sanitizes
/// its own lines, so this aggregator does not sanitize again.
pub fn candidate_screen_lines(state: &CandidateRunState) -> Vec<String> {
    if state.show_help {
        return candidate_help_lines_for(state.app.screen(), state.focus);
    }
    match state.app.screen() {
        CandidateScreen::Solving => {
            let mut lines = match state.app.session() {
                Some(session) => work_lines(session, &state.work),
                None => vec!["no active question.".to_string()],
            };
            let chat_shows_notice =
                state.focus == CandidateFocus::Chat && state.chat.paste_notice();
            if state.paste_notice && !chat_shows_notice {
                lines.push("pasting is blocked here; type instead.".to_string());
            }
            lines.extend(state.chat.chat_lines());
            lines.push("focus: work/other/redirect/chat (tab switches)".to_string());
            lines
        }
        CandidateScreen::Final => match state.app.session() {
            Some(session) => match candidate_final(session, &BTreeMap::new(), &[]) {
                Ok(result) => candidate_final_lines(&result),
                Err(_) => state.app.screen_lines(),
            },
            None => state.app.screen_lines(),
        },
        _ => state.app.screen_lines(),
    }
}

/// Restores the candidate terminal on drop for every exit path.
pub struct CandidateTtyGuard;

impl Drop for CandidateTtyGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableBracketedPaste
        );
    }
}
