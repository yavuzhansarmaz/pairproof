//! Candidate chat over the pipeline: hidden-free session state only.
//! A second submit while one is in flight is rejected, and history is
//! extended by the caller slice only after a successful reply.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::core::{CoreError, Session};
use crate::llm::chat::{
    log_boundary_violation_if_present, run_chat, run_chat_streaming, ChatState,
};
use crate::llm::{Message, MetaClient};
use crate::orchestration::pipeline::{PipelineError, PipelineFuture, QuestionPipeline};
use crate::orchestration::session::Orchestrator;

/// Narrow boundary for one Phase 4 candidate chat turn.
pub trait ChatTransport: Send + Sync {
    /// Runs one chat turn and returns the assistant reply text.
    fn chat<'a>(
        &'a self,
        session: &'a mut Session,
        state: &'a ChatState,
        history: &'a [Message],
        user_message: &'a str,
        at_ms: u64,
    ) -> PipelineFuture<'a, String>;

    /// Runs one streaming chat turn; the default emits the full reply once.
    fn chat_streaming<'a>(
        &'a self,
        session: &'a mut Session,
        state: &'a ChatState,
        history: &'a [Message],
        user_message: &'a str,
        at_ms: u64,
        on_chunk: &'a mut (dyn FnMut(&str) + Send),
    ) -> PipelineFuture<'a, String> {
        Box::pin(async move {
            let reply = self
                .chat(session, state, history, user_message, at_ms)
                .await?;
            on_chunk(&reply);
            Ok(reply)
        })
    }
}

/// Production chat transport wrapping the real Phase 4 chat turn, which
/// uses the light model with low reasoning inside `run_chat`.
#[derive(Debug)]
pub struct MetaChatTransport {
    client: MetaClient,
}

impl MetaChatTransport {
    /// Creates a transport wrapping `client`.
    pub fn new(client: MetaClient) -> Self {
        Self { client }
    }
}

impl ChatTransport for MetaChatTransport {
    fn chat<'a>(
        &'a self,
        session: &'a mut Session,
        state: &'a ChatState,
        history: &'a [Message],
        user_message: &'a str,
        at_ms: u64,
    ) -> PipelineFuture<'a, String> {
        Box::pin(async move {
            let outcome = run_chat(&self.client, session, state, history, user_message, at_ms)
                .await
                .map_err(|e| PipelineError::Llm(e.to_string()))?;
            Ok(outcome.value)
        })
    }

    fn chat_streaming<'a>(
        &'a self,
        session: &'a mut Session,
        state: &'a ChatState,
        history: &'a [Message],
        user_message: &'a str,
        at_ms: u64,
        on_chunk: &'a mut (dyn FnMut(&str) + Send),
    ) -> PipelineFuture<'a, String> {
        Box::pin(async move {
            let outcome = run_chat_streaming(
                &self.client,
                session,
                state,
                history,
                user_message,
                at_ms,
                on_chunk,
            )
            .await
            .map_err(|e| PipelineError::Llm(e.to_string()))?;
            Ok(outcome.value)
        })
    }
}

struct ChatGuard<'a>(&'a AtomicBool);

impl<'a> ChatGuard<'a> {
    fn acquire(flag: &'a AtomicBool) -> Result<Self, PipelineError> {
        flag.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| Self(flag))
            .map_err(|_| PipelineError::ChatBusy)
    }
}

impl Drop for ChatGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl QuestionPipeline {
    /// Returns whether a chat submit is currently in flight.
    pub fn is_chat_busy(&self) -> bool {
        self.chat_busy.load(Ordering::SeqCst)
    }

    /// Submits one candidate chat message. State is built only through
    /// `ChatState::from_session`, so locked tokens, approach quality,
    /// and the best approach never enter the chat. On success the user
    /// and assistant messages are pushed onto `history`; on failure or
    /// when another submit is in flight, history is left unchanged.
    pub async fn submit_chat(
        &self,
        orch: &mut Orchestrator,
        transport: &dyn ChatTransport,
        history: &mut Vec<Message>,
        user_message: &str,
        now_ms: u64,
    ) -> Result<String, PipelineError> {
        let _guard = ChatGuard::acquire(&self.chat_busy)?;
        let statement = self
            .current_statement(orch)
            .map(|fetched| fetched.statement)
            .unwrap_or_default();
        let code = orch
            .current_index()
            .and_then(|index| self.codes.get(&index))
            .cloned()
            .unwrap_or_default();
        let state = ChatState::from_session(orch.session(), &statement, &code);
        let index = orch.current_index();
        let result = transport
            .chat(orch.session_mut(), &state, history, user_message, now_ms)
            .await;
        if orch.is_terminal() {
            return Err(CoreError::Terminal.into());
        }
        if orch.current_index() != index {
            return Err(CoreError::InvalidState("session changed during chat".to_string()).into());
        }
        let reply = result?;
        history.push(Message::new("user", user_message));
        history.push(Message::new("assistant", &reply));
        Ok(reply)
    }

    /// Streaming submit mirroring `submit_chat`; history changes only on success.
    #[allow(clippy::too_many_arguments)]
    pub async fn submit_chat_streaming(
        &self,
        orch: &mut Orchestrator,
        transport: &dyn ChatTransport,
        history: &mut Vec<Message>,
        user_message: &str,
        now_ms: u64,
        on_chunk: &mut (dyn FnMut(&str) + Send),
    ) -> Result<String, PipelineError> {
        let _guard = ChatGuard::acquire(&self.chat_busy)?;
        let statement = self
            .current_statement(orch)
            .map(|fetched| fetched.statement)
            .unwrap_or_default();
        let code = orch
            .current_index()
            .and_then(|index| self.codes.get(&index))
            .cloned()
            .unwrap_or_default();
        let state = ChatState::from_session(orch.session(), &statement, &code);
        let index = orch.current_index();
        let mut partial = String::new();
        let result = {
            let mut tracking = |delta: &str| {
                partial.push_str(delta);
                on_chunk(delta);
            };
            transport
                .chat_streaming(
                    orch.session_mut(),
                    &state,
                    history,
                    user_message,
                    now_ms,
                    &mut tracking,
                )
                .await
        };
        if orch.is_terminal() {
            return Err(CoreError::Terminal.into());
        }
        if orch.current_index() != index {
            return Err(CoreError::InvalidState("session changed during chat".to_string()).into());
        }
        let reply = match result {
            Ok(reply) => reply,
            Err(err) => {
                if !partial.is_empty() {
                    log_boundary_violation_if_present(orch.session_mut(), &partial, now_ms)
                        .map_err(|e| PipelineError::Llm(e.to_string()))?;
                }
                return Err(err);
            }
        };
        history.push(Message::new("user", user_message));
        history.push(Message::new("assistant", &reply));
        Ok(reply)
    }
}
