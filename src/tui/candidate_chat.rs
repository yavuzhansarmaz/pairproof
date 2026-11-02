//! Candidate chat controller with paste guard; no terminal I/O.

use crate::core::{CoreError, Session};
use crate::llm::chat::{assemble_chat_messages, log_boundary_violation_if_present, ChatState};
use crate::llm::common::LlmError;
use crate::llm::Message;
use crate::tui::candidate_work::CandidateWork;
use crate::tui::shared::sanitize_display;

/// Payload-free chat service failure shown generically to candidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatServiceError {
    Unavailable,
    Failed,
}

/// Synchronous chat completion port; Phase 10 owns real transport.
pub trait ChatService {
    /// Returns the coach reply for the assembled messages.
    fn complete(&mut self, messages: &[Message]) -> Result<String, ChatServiceError>;
}

/// Legacy Phase-9 seam: always `ChatServiceError::Unavailable`, never
/// fakes a reply. Live Phase-10 path bypasses it via chat intents in
/// `candidate_runtime`; retained for deterministic tests and offline use.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableChatService;

impl ChatService for UnavailableChatService {
    fn complete(&mut self, _messages: &[Message]) -> Result<String, ChatServiceError> {
        Err(ChatServiceError::Unavailable)
    }
}

impl ChatService for Box<dyn ChatService> {
    fn complete(&mut self, messages: &[Message]) -> Result<String, ChatServiceError> {
        (**self).complete(messages)
    }
}

/// Blocked paste; never recorded as a score or paste flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PasteBlocked {
    /// Paste was rejected and the buffer was left unchanged.
    #[error("pasting is blocked here; type instead.")]
    Blocked,
}

/// Typed chat text buffer; pasting is always blocked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatInput {
    text: String,
}

impl ChatInput {
    /// Creates an empty chat input.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends one typed character, skipping control characters.
    pub fn push_char(&mut self, c: char) {
        if !c.is_control() {
            self.text.push(c);
        }
    }

    /// Appends typed text character by character.
    pub fn push_typed(&mut self, text: &str) {
        for c in text.chars() {
            self.push_char(c);
        }
    }

    /// Always blocks pasting and leaves the buffer unchanged.
    pub fn insert_paste(&mut self, _text: &str) -> Result<(), PasteBlocked> {
        Err(PasteBlocked::Blocked)
    }

    /// Removes the last character, if any.
    pub fn backspace(&mut self) {
        self.text.pop();
    }

    /// Clears the whole buffer.
    pub fn clear(&mut self) {
        self.text.clear();
    }

    /// Returns the current buffer text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns true when the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

/// Inserts into Other text; pasted inserts are always blocked.
pub fn other_insert(
    work: &mut CandidateWork,
    text: &str,
    pasted: bool,
) -> Result<(), PasteBlocked> {
    if pasted {
        return Err(PasteBlocked::Blocked);
    }
    for c in text.chars() {
        work.push_other_char(c);
    }
    Ok(())
}

/// Typed candidate chat failure; service detail is never carried.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChatError {
    /// A chat request is already in flight.
    #[error("chat is busy; wait for the current reply.")]
    Busy,
    /// The message is empty after trimming.
    #[error("message must not be empty.")]
    EmptyMessage,
    /// Chat transport is not wired yet.
    #[error("chat is not available yet.")]
    Unavailable,
    /// Chat transport failed generically.
    #[error("chat failed; try again.")]
    Failed,
    /// Core session failure.
    #[error(transparent)]
    Core(#[from] CoreError),
}

/// Pure candidate chat state; the caller owns the session.
pub struct ChatController {
    service: Box<dyn ChatService>,
    history: Vec<Message>,
    input: ChatInput,
    busy: bool,
    last_error: Option<ChatServiceError>,
    paste_notice: bool,
}

impl ChatController {
    /// Creates a controller wrapping the given chat service.
    pub fn new(service: impl ChatService + 'static) -> Self {
        Self {
            service: Box::new(service),
            history: Vec::new(),
            input: ChatInput::new(),
            busy: false,
            last_error: None,
            paste_notice: false,
        }
    }

    /// Types one character and clears any paste notice.
    pub fn type_char(&mut self, c: char) {
        self.input.push_char(c);
        self.paste_notice = false;
    }

    /// Types text character by character and clears the paste notice.
    pub fn type_text(&mut self, text: &str) {
        self.input.push_typed(text);
        self.paste_notice = false;
    }

    /// Alias for typed text entry.
    pub fn push_typed(&mut self, text: &str) {
        self.type_text(text);
    }

    /// Alias for single-character typed entry.
    pub fn push_char(&mut self, c: char) {
        self.type_char(c);
    }

    /// Blocks a paste attempt and records the paste notice.
    pub fn paste_input(&mut self, text: &str) -> Result<(), PasteBlocked> {
        match self.input.insert_paste(text) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.paste_notice = true;
                Err(error)
            }
        }
    }

    /// Alias for a blocked paste attempt.
    pub fn insert_paste(&mut self, text: &str) -> Result<(), PasteBlocked> {
        self.paste_input(text)
    }

    /// Removes the last input character, if any.
    pub fn backspace_input(&mut self) {
        self.input.backspace();
    }

    /// Clears the whole input buffer.
    pub fn clear_input(&mut self) {
        self.input.clear();
    }

    /// Returns the current input buffer.
    pub fn input(&self) -> &ChatInput {
        &self.input
    }

    /// Returns the current input text.
    pub fn input_text(&self) -> &str {
        self.input.text()
    }

    /// Sets whether a chat request is in flight.
    pub fn set_busy(&mut self, busy: bool) {
        self.busy = busy;
    }

    /// Returns true while a chat request is in flight.
    pub fn is_busy(&self) -> bool {
        self.busy
    }

    /// Returns the completed chat history.
    pub fn history(&self) -> &[Message] {
        &self.history
    }

    /// Returns the last generic service error, if any.
    pub fn last_error(&self) -> Option<ChatServiceError> {
        self.last_error
    }

    /// Returns true while the paste-block notice should show.
    pub fn paste_notice(&self) -> bool {
        self.paste_notice
    }

    /// Submits the verbatim input, logs boundary leaks, keeps input on error.
    pub fn submit(
        &mut self,
        session: &mut Session,
        state: &ChatState,
        at_ms: u64,
    ) -> Result<String, ChatError> {
        if self.busy {
            return Err(ChatError::Busy);
        }
        let text = self.input.text().to_string();
        if text.trim().is_empty() {
            return Err(ChatError::EmptyMessage);
        }
        let messages = assemble_chat_messages(state, &self.history, &text);
        self.busy = true;
        let result = self.service.complete(&messages);
        self.busy = false;
        let reply = match result {
            Ok(reply) => reply,
            Err(ChatServiceError::Unavailable) => {
                self.last_error = Some(ChatServiceError::Unavailable);
                return Err(ChatError::Unavailable);
            }
            Err(ChatServiceError::Failed) => {
                self.last_error = Some(ChatServiceError::Failed);
                return Err(ChatError::Failed);
            }
        };
        match log_boundary_violation_if_present(session, &reply, at_ms) {
            Ok(_) => {}
            Err(LlmError::Core(error)) => return Err(ChatError::Core(error)),
            Err(_) => return Err(ChatError::Failed),
        }
        self.history.push(Message::new("user", &text));
        self.history.push(Message::new("assistant", &reply));
        self.input.clear();
        self.last_error = None;
        Ok(reply)
    }

    /// Builds sanitized candidate chat lines with no scoring data.
    pub fn chat_lines(&self) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        for message in &self.history {
            let label = if message.role == "assistant" {
                "coach"
            } else {
                "you"
            };
            lines.push(format!("{label}: {}", message.content));
        }
        lines.push(format!("message: {}", self.input.text()));
        if self.busy {
            lines.push("sending...".to_string());
        }
        if let Some(error) = self.last_error {
            lines.push(match error {
                ChatServiceError::Unavailable => "chat is not available yet.".to_string(),
                ChatServiceError::Failed => "chat failed; try again.".to_string(),
            });
        }
        if self.paste_notice {
            lines.push(PasteBlocked::Blocked.to_string());
        }
        lines.iter().map(|line| sanitize_display(line)).collect()
    }
}

impl Default for ChatController {
    /// Creates a controller with the legacy Phase-9 unavailable service;
    /// the live Phase-10 path sends chat via intents, not this default.
    fn default() -> Self {
        Self::new(UnavailableChatService)
    }
}
