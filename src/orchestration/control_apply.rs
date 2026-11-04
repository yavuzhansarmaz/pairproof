//! One-intent application for the control executor: every
//! candidate intent kind dispatched to the pipeline, the session,
//! or the chat transport, exactly once and in sequence.

use std::time::Duration;

use crate::core::CoreError;
use crate::orchestration::control::{Intent, IntentKind};
use crate::orchestration::control_server::{
    publish_snapshot, ControlExecutor, ControlIo, ControlTransports,
};
use crate::orchestration::pipeline::{PipelineError, QuestionPipeline};
use crate::orchestration::session::Orchestrator;

impl ControlExecutor {
    #[rustfmt::skip] #[allow(unused_assignments)]
    pub(crate) async fn apply_one(&mut self, io: &dyn ControlIo, intent: &Intent, orch: &mut Orchestrator,
        pipe: &mut QuestionPipeline, t: &ControlTransports<'_>, now_ms: u64) -> Result<(), PipelineError> {
        match &intent.kind {
            IntentKind::SelectPreset { preset } => {
                pipe.select_preset(orch, *preset, t.coding, t.sandbox, now_ms).await?;
                Ok(())
            }
            IntentKind::SelectOther { text } => {
                pipe.select_other(orch, text, t.generator, t.coding, t.sandbox, now_ms).await?;
                Ok(())
            }
            IntentKind::Continue => {
                pipe.continue_coding(orch, t.coding, t.sandbox, now_ms).await?;
                Ok(())
            }
            IntentKind::Redirect { text } => {
                pipe.redirect_coding(orch, text, t.coding, t.sandbox, now_ms).await?;
                Ok(())
            }
            IntentKind::RunSolution => {
                pipe.run_solution(orch, t.sandbox, now_ms).await?;
                Ok(())
            }
            IntentKind::RequestTests => {
                pipe.request_tests(orch, t.coding, t.sandbox, now_ms).await?;
                Ok(())
            }
            IntentKind::Finish => pipe.finish_question_with_sandbox(orch, t.sandbox, now_ms).await,
            IntentKind::Skip => pipe.skip_question(orch, now_ms),
            IntentKind::RequestReQuestion => orch.request_requestion(now_ms).map_err(PipelineError::from),
            IntentKind::ChangeApproach => pipe.change_approach(orch, now_ms),
            IntentKind::Navigate { index } => pipe.navigate_question(orch, *index, now_ms),
            IntentKind::Disconnect => {
                if orch.is_connected() {
                    orch.disconnect_candidate(now_ms).map_err(PipelineError::from)
                } else {
                    Ok(())
                }
            }
            IntentKind::ChatRequest { text, .. } => {
                let index = orch.current_index().ok_or(CoreError::NoActiveQuestion)?;
                self.bind_chat(Some(index));
                // Bounded publication: ~10/s ticker + first-chunk wakeup + completion flush;
                // memory is bounded by the partial reply text itself — no delta queue.
                let shared: std::sync::Arc<std::sync::Mutex<String>> = std::sync::Arc::new(std::sync::Mutex::new(String::new())); let notify = std::sync::Arc::new(tokio::sync::Notify::new());
                let (buf, wake) = (shared.clone(), notify.clone());
                let mut on_chunk = move |delta: &str| {
                    let was_empty = { let mut guard = buf.lock().unwrap_or_else(|e| e.into_inner()); let empty = guard.is_empty(); guard.push_str(delta); empty };
                    if was_empty { wake.notify_one(); }
                };
                self.chat_partial.clear(); let mut last_published = String::new();
                macro_rules! publish_if_changed { () => {{ let current = shared.lock().unwrap_or_else(|e| e.into_inner()).clone();
                    if !current.is_empty() && current != last_published { if let Some(mut snap) = self.snapshot_base.clone() { snap.chat_replies = self.replies_by_question.get(&index).cloned().unwrap_or_default(); snap.chat_partial = Some(current.clone()); let _ = publish_snapshot(io, &snap).await; } self.chat_partial.clone_from(&current); last_published = current; } }}; }
                let history = self.histories_by_question.entry(index).or_default();
                let submit = pipe.submit_chat_streaming(orch, t.chat, history, text, now_ms, &mut on_chunk);
                tokio::pin!(submit);
                let mut ticker = tokio::time::interval(Duration::from_millis(100)); ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                // Biased polling checks wakeup/ticker before submit, so a ready stream cannot starve first-chunk publication.
                let result = loop { tokio::select! { biased;
                    _ = notify.notified() => publish_if_changed!(),
                    _ = ticker.tick() => publish_if_changed!(),
                    res = &mut submit => { publish_if_changed!(); break res; }
                } };
                let full = shared.lock().unwrap_or_else(|e| e.into_inner()).clone();
                self.chat_partial.clear();
                match result { Ok(reply) => { self.replies_by_question.entry(index).or_default().push(reply); Ok(()) } Err(err) => {
                    if !full.is_empty() { self.replies_by_question.entry(index).or_default().push(format!("{full} [response interrupted]")); } Err(err) } }
            }
        }
    }
}
