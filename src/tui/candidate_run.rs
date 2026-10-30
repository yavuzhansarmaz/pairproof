//! Candidate run state, key and paste dispatch, and the TTY loop.

use std::collections::BTreeMap;
use std::error::Error;
use std::time::Duration;

use crossterm::event::{Event, KeyCode as K, KeyEvent, KeyEventKind, KeyModifiers};

use crate::core::Session;
use crate::llm::chat::ChatState;
use crate::tui::candidate::{screen_title, CandidateApp, CandidateScreen};
use crate::tui::candidate_actions::{
    UnavailableCodingRunner, UnavailableOtherGrader, UnavailableTestRunner,
};
use crate::tui::candidate_chat::{other_insert, ChatController};
use crate::tui::candidate_final::{candidate_final, candidate_final_lines};
use crate::tui::candidate_work::{work_lines, CandidateWork};
use crate::tui::shared::sanitize_display;

/// Focus pane while solving; Tab cycles Work, Other, Redirect, Chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CandidateFocus {
    #[default]
    Work,
    Other,
    Redirect,
    Chat,
}

/// Pure candidate run state; runners are fresh unavailable values.
pub struct CandidateRunState {
    pub app: CandidateApp,
    pub work: CandidateWork,
    pub chat: ChatController,
    pub focus: CandidateFocus,
    pub paste_notice: bool,
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

/// Returns the current time in milliseconds for connection expiry.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_millis()).min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn pop_last(text: &str) -> String {
    let mut owned = text.to_string();
    owned.pop();
    owned
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
        K::Tab => state.focus = CandidateFocus::Other,
        K::Char(c) if !control => match c {
            '1'..='4' => {
                let preset = c as u8 - b'1';
                state.with_session(|work, session| {
                    let _ =
                        work.select_preset(session, preset, &mut UnavailableCodingRunner, now_ms());
                });
            }
            '5' | 'o' => state.focus = CandidateFocus::Other,
            'c' => state.with_session(|work, session| {
                let _ = work.continue_work(session, &mut UnavailableCodingRunner, now_ms());
            }),
            'd' => state.focus = CandidateFocus::Redirect,
            'a' => state.with_session(|work, session| {
                let _ = work.change_approach(session, now_ms());
            }),
            't' => state.with_session(|work, session| {
                let _ = work.request_tests(session, &mut UnavailableTestRunner, now_ms());
            }),
            'f' => state.with_session(|work, session| {
                let _ = work.finish_question(session, now_ms());
            }),
            's' => state.with_session(|work, session| {
                let _ = work.skip_question(session, now_ms());
            }),
            'r' => state.with_session(|work, session| {
                let _ = work.request_requestion(session, now_ms());
            }),
            'q' => {
                state.app.quit();
            }
            _ => {}
        },
        _ => {}
    }
}

fn handle_other_key(state: &mut CandidateRunState, key: KeyEvent, control: bool) {
    match key.code {
        K::Tab => state.focus = CandidateFocus::Redirect,
        K::Enter => {
            state.with_session(|work, session| {
                let _ = work.select_other_graded(
                    session,
                    &mut UnavailableOtherGrader,
                    &mut UnavailableCodingRunner,
                    now_ms(),
                );
            });
            state.focus = CandidateFocus::Work;
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
        K::Tab => state.focus = CandidateFocus::Chat,
        K::Enter => {
            state.with_session(|work, session| {
                let _ = work.redirect(session, &mut UnavailableCodingRunner, now_ms());
            });
            state.focus = CandidateFocus::Work;
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
        K::Tab => state.focus = CandidateFocus::Work,
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
    match state.app.screen() {
        CandidateScreen::Connect => match key.code {
            K::Enter => {
                let _ = state.app.submit_connection(now_ms());
            }
            K::Backspace => state.app.backspace(),
            K::Char(c) if !control => state.app.push_char(c),
            _ => {}
        },
        CandidateScreen::Rules => match key.code {
            K::Enter => {
                let _ = state.app.accept_rules();
            }
            K::Char('q') if !control => {
                state.app.quit();
            }
            _ => {}
        },
        CandidateScreen::Waiting => match key.code {
            K::Char('q') if !control => {
                state.app.quit();
            }
            _ => {}
        },
        CandidateScreen::Final => match key.code {
            K::Enter => {
                state.app.quit();
            }
            K::Char('q') if !control => {
                state.app.quit();
            }
            _ => {}
        },
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
        CandidateScreen::Connect => state.app.push_text(text),
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

/// Builds sanitized candidate screen lines for the run state.
pub fn candidate_screen_lines(state: &CandidateRunState) -> Vec<String> {
    let lines = match state.app.screen() {
        CandidateScreen::Solving => {
            let mut lines = match state.app.session() {
                Some(session) => work_lines(session, &state.work),
                None => vec!["No active question.".to_string()],
            };
            let chat_shows_notice =
                state.focus == CandidateFocus::Chat && state.chat.paste_notice();
            if state.paste_notice && !chat_shows_notice {
                lines.push("Pasting is blocked here; type instead.".to_string());
            }
            lines.extend(state.chat.chat_lines());
            lines.push("Focus: work/other/redirect/chat (Tab switches)".to_string());
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
    };
    lines.iter().map(|line| sanitize_display(line)).collect()
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

/// Runs the candidate TTY loop until the app reaches Exited.
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
    while state.app.screen() != CandidateScreen::Exited {
        let title = screen_title(state.app.screen());
        let lines = candidate_screen_lines(state);
        let _ = terminal.draw(|frame| {
            let widget = ratatui::widgets::Paragraph::new(lines.join("\n")).block(
                ratatui::widgets::Block::default()
                    .title(title)
                    .borders(ratatui::widgets::Borders::ALL),
            );
            frame.render_widget(widget, frame.area());
        });
        if crossterm::event::poll(Duration::from_millis(250)).unwrap_or(false) {
            match crossterm::event::read() {
                Ok(Event::Key(key)) => {
                    if key.kind != KeyEventKind::Release {
                        handle_candidate_key(state, key);
                    }
                }
                Ok(Event::Paste(text)) => handle_candidate_paste(state, &text),
                _ => {}
            }
        }
    }
    Ok(())
}
