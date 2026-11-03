//! Expert-side control executor: the single sequential applier.
//! Processed ids are persisted after each successful core apply, so a
//! restarted executor reloads them and never applies an intent twice.
//! Residual window: a crash between core apply and ledger persist could
//! redeliver once; the window is closed before the next intent is read.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use crate::connection::EphemeralUsername;
use crate::core::EventKind;
use crate::llm::Message;
use crate::orchestration::chat::ChatTransport;
use crate::orchestration::coding::{CodingTransport, SandboxIo};
use crate::orchestration::control::{
    parse_intent_bytes, resolve_control_file, CandidateSnapshot, ControlError, Intent, IntentKind,
    ProcessedState, CONTROL_ROOT, INBOX_DIR, INTENT_REL, MAX_CONTROL_READ_BYTES, OUTBOX_DIR,
    PROCESSED_FILE, SNAPSHOT_REL,
};
use crate::orchestration::pipeline::{ApproachGenerator, PipelineError, QuestionPipeline};
use crate::orchestration::session::Orchestrator;
use crate::sandbox::command::{exec_as, exec_root};
use crate::sandbox::lifecycle::{SandboxHandle, OP_TIMEOUT};
use crate::sandbox::process::{run_bounded, CommandSpec};

/// Boxed control-I/O future.
pub type ControlFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ControlError>> + Send + 'a>>;

/// Expert control storage boundary. Implementations treat every inbox
/// byte as untrusted: fixed filenames, bounded reads, no shell.
pub trait ControlIo: Send + Sync {
    /// Creates control dirs with candidate owning only the inbox.
    fn setup_dirs<'a>(&'a self, candidate_user: &'a str) -> ControlFuture<'a, ()>;
    /// Bounded-reads the intent log; a missing log reads as empty.
    fn read_intents<'a>(&'a self) -> ControlFuture<'a, Vec<u8>>;
    /// Atomically publishes snapshot bytes (temp file, then rename).
    fn write_snapshot_atomic<'a>(&'a self, bytes: &'a [u8]) -> ControlFuture<'a, ()>;
    /// Loads the durable processed ledger, if one was persisted.
    fn load_processed<'a>(&'a self) -> ControlFuture<'a, Option<ProcessedState>>;
    /// Atomically persists the processed ledger.
    fn store_processed<'a>(&'a self, state: &'a ProcessedState) -> ControlFuture<'a, ()>;
}

/// Transports injected into the executor; all Meta calls stay expert-side.
#[rustfmt::skip]
pub struct ControlTransports<'a> {
    pub coding: &'a dyn CodingTransport, pub sandbox: &'a dyn SandboxIo,
    pub generator: &'a dyn ApproachGenerator, pub chat: &'a dyn ChatTransport,
}

/// Expert-visible outcome of one intent in a drain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyOutcome {
    Applied,
    Duplicate,
    Rejected,
}
/// Expert-visible per-intent apply report. Candidates never see this.
#[rustfmt::skip]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyReport { pub intent_id: Option<String>, pub seq: Option<u64>, pub outcome: ApplyOutcome, pub error: Option<String> }
#[rustfmt::skip]
impl ApplyReport {
    fn applied(intent: &Intent) -> Self { Self { intent_id: Some(intent.id.clone()), seq: Some(intent.seq), outcome: ApplyOutcome::Applied, error: None } }
    fn duplicate(intent: &Intent) -> Self { Self { intent_id: Some(intent.id.clone()), seq: Some(intent.seq), outcome: ApplyOutcome::Duplicate, error: None } }
    fn rejected(intent: &Intent, error: String) -> Self { Self { intent_id: Some(intent.id.clone()), seq: Some(intent.seq), outcome: ApplyOutcome::Rejected, error: Some(error) } }
}

/// Sequential exactly-once executor over one control directory set.
#[derive(Debug)]
pub struct ControlExecutor {
    session_id: String,
    processed: ProcessedState,
    chat_history: Vec<Message>,
    chat_replies: Vec<String>,
    reconnects_seen: Option<usize>,
    snapshot_base: Option<CandidateSnapshot>,
    chat_partial: String,
}
impl ControlExecutor {
    /// Creates an executor, reloading any persisted processed ledger.
    #[rustfmt::skip]
    pub async fn restore(session_id: &str, io: &dyn ControlIo) -> Result<Self, ControlError> {
        let processed = match io.load_processed().await? {
            Some(state) if state.session_id == session_id => state,
            Some(_) => return Err(ControlError::WrongSession),
            None => ProcessedState::new(session_id),
        };
        Ok(Self { session_id: session_id.into(), processed, chat_history: Vec::new(), chat_replies: Vec::new(), reconnects_seen: None, snapshot_base: None, chat_partial: String::new() })
    }
    /// Returns the durable processed ledger.
    pub fn processed(&self) -> &ProcessedState {
        &self.processed
    }
    /// Returns candidate chat replies in arrival order.
    pub fn chat_replies(&self) -> &[String] {
        &self.chat_replies
    }
    /// Sets the base snapshot used for in-progress partial publications.
    #[rustfmt::skip]
    pub fn set_snapshot_base(&mut self, snapshot: CandidateSnapshot) { self.snapshot_base = Some(snapshot); }
    /// Returns the in-progress partial reply, if any.
    #[rustfmt::skip]
    pub fn chat_partial(&self) -> Option<&str> { if self.chat_partial.is_empty() { None } else { Some(&self.chat_partial) } }
    /// Returns the session this executor is bound to.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Reads the intent log once and applies every new intent in seq
    /// order through real pipeline/core validation.
    #[rustfmt::skip]
    pub async fn drain(&mut self, io: &dyn ControlIo, orch: &mut Orchestrator,
        pipe: &mut QuestionPipeline, transports: &ControlTransports<'_>, now_ms: u64)
        -> Result<Vec<ApplyReport>, ControlError> {
        self.reset_sequence_on_reconnect(io, orch).await?;
        let raw = io.read_intents().await?;
        let parsed = parse_intent_bytes(&raw, &self.session_id);
        let mut reports: Vec<ApplyReport> = parsed.rejected.iter()
            .map(|err| ApplyReport { intent_id: None, seq: None, outcome: ApplyOutcome::Rejected, error: Some(err.to_string()) }).collect();
        let mut intents = parsed.intents;
        intents.sort_by_key(|intent| intent.seq);
        for intent in &intents {
            if self.processed.ids.contains(&intent.id) { reports.push(ApplyReport::duplicate(intent)); continue; }
            if let IntentKind::ChatRequest { chat_id, .. } = &intent.kind {
                if self.processed.chat_ids.contains(chat_id) {
                    self.consume(io, intent).await?;
                    reports.push(ApplyReport::duplicate(intent));
                    continue;
                }
            }
            let expected = self.processed.last_seq + 1;
            if intent.seq != expected {
                reports.push(ApplyReport { intent_id: Some(intent.id.clone()), seq: Some(intent.seq),
                    outcome: ApplyOutcome::Rejected, error: Some(ControlError::SequenceGap { expected, got: intent.seq }.to_string()) });
                continue;
            }
            match self.apply_one(io, intent, orch, pipe, transports, now_ms).await {
                Ok(()) => { self.consume(io, intent).await?; reports.push(ApplyReport::applied(intent)); }
                Err(err) => {
                    // Illegal transitions are consumed: redelivery is a duplicate, never a retry loop.
                    self.consume(io, intent).await?;
                    reports.push(ApplyReport::rejected(intent, err.to_string()));
                }
            }
        }
        Ok(reports)
    }

    /// Resets `last_seq` once per verified reconnect so a restarted
    /// candidate sequencer (fresh `seq` from 1) is accepted instead of
    /// rejected with `SequenceGap`. Intent-ID and chat-ID dedup sets are
    /// kept intact: redelivered IDs still report `Duplicate` and are
    /// never re-applied. The baseline is learned on the first drain so
    /// a restored executor never resets spuriously.
    async fn reset_sequence_on_reconnect(
        &mut self,
        io: &dyn ControlIo,
        orch: &Orchestrator,
    ) -> Result<(), ControlError> {
        let reconnects = orch
            .session()
            .events()
            .iter()
            .filter(|e| matches!(e.kind, EventKind::CandidateReconnected { .. }))
            .count();
        match self.reconnects_seen {
            None => self.reconnects_seen = Some(reconnects),
            Some(seen) if reconnects > seen => {
                self.reconnects_seen = Some(reconnects);
                if self.processed.last_seq != 0 {
                    self.processed.last_seq = 0;
                    io.store_processed(&self.processed).await?;
                }
            }
            Some(_) => {}
        }
        Ok(())
    }

    #[rustfmt::skip]
    async fn consume(&mut self, io: &dyn ControlIo, intent: &Intent) -> Result<(), ControlError> {
        self.processed.mark(intent);
        io.store_processed(&self.processed).await
    }

    #[rustfmt::skip] #[allow(unused_assignments)]
    async fn apply_one(&mut self, io: &dyn ControlIo, intent: &Intent, orch: &mut Orchestrator,
        pipe: &mut QuestionPipeline, t: &ControlTransports<'_>, now_ms: u64) -> Result<(), PipelineError> {
        match &intent.kind {
            IntentKind::SelectPreset { preset } => pipe.select_preset(orch, *preset, t.coding, t.sandbox, now_ms).await.map(|_| ()),
            IntentKind::SelectOther { text } => pipe.select_other(orch, text, t.generator, t.coding, t.sandbox, now_ms).await.map(|_| ()),
            IntentKind::Continue => pipe.continue_coding(orch, t.coding, t.sandbox, now_ms).await.map(|_| ()),
            IntentKind::Redirect { text } => pipe.redirect_coding(orch, text, t.coding, t.sandbox, now_ms).await.map(|_| ()),
            IntentKind::RequestTests => pipe.request_tests(orch, t.sandbox, now_ms).await.map(|_| ()),
            IntentKind::Finish => pipe.finish_question(orch, now_ms),
            IntentKind::Skip => pipe.skip_question(orch, now_ms),
            IntentKind::RequestReQuestion => orch.request_requestion(now_ms).map_err(PipelineError::from),
            IntentKind::ChangeApproach => pipe.change_approach(orch, now_ms),
            IntentKind::ChatRequest { text, .. } => {
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
                    if !current.is_empty() && current != last_published { if let Some(mut snap) = self.snapshot_base.clone() { snap.chat_replies.clone_from(&self.chat_replies); snap.chat_partial = Some(current.clone()); let _ = publish_snapshot(io, &snap).await; } self.chat_partial.clone_from(&current); last_published = current; } }}; }
                let submit = pipe.submit_chat_streaming(orch, t.chat, &mut self.chat_history, text, now_ms, &mut on_chunk);
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
                match result { Ok(reply) => { self.chat_replies.push(reply); Ok(()) } Err(err) => {
                    if !full.is_empty() { self.chat_replies.push(format!("{full} [response interrupted]")); } Err(err) } }
            }
        }
    }
}

/// Serializes and atomically publishes one snapshot through `io`.
pub async fn publish_snapshot(
    io: &dyn ControlIo,
    snapshot: &CandidateSnapshot,
) -> Result<(), ControlError> {
    let bytes = serde_json::to_vec(snapshot)?;
    io.write_snapshot_atomic(&bytes).await
}

#[rustfmt::skip]
fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<(), ControlError> {
    let tmp = path.with_extension(format!("tmp-{}", crate::orchestration::control::generate_intent_id()));
    std::fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    { use std::os::unix::fs::PermissionsExt; let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644)); }
    std::fs::rename(&tmp, path)?;
    Ok(())
}
#[rustfmt::skip]
fn bounded_read(path: &std::path::Path) -> Result<Vec<u8>, ControlError> {
    use std::io::Read;
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    let mut buf = Vec::new();
    file.by_ref().take(MAX_CONTROL_READ_BYTES as u64).read_to_end(&mut buf)?;
    Ok(buf)
}

/// Filesystem control I/O rooted at a local directory (tests, tooling).
#[derive(Debug, Clone)]
pub struct LocalControlIo {
    root: PathBuf,
}
#[rustfmt::skip]
impl LocalControlIo {
    /// Creates local control I/O rooted at `root`.
    pub fn new(root: PathBuf) -> Self { Self { root } }
    /// Returns the local control root.
    pub fn root(&self) -> &std::path::Path { &self.root }
}
#[rustfmt::skip]
impl ControlIo for LocalControlIo {
    fn setup_dirs<'a>(&'a self, _candidate_user: &'a str) -> ControlFuture<'a, ()> {
        Box::pin(async move {
            std::fs::create_dir_all(self.root.join("inbox"))?;
            std::fs::create_dir_all(self.root.join("outbox"))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                for dir in [self.root.clone(), self.root.join("inbox"), self.root.join("outbox")] {
                    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755))?;
                }
            }
            Ok(())
        })
    }
    fn read_intents<'a>(&'a self) -> ControlFuture<'a, Vec<u8>> {
        Box::pin(async move { bounded_read(&resolve_control_file(&self.root, INTENT_REL)?) })
    }
    fn write_snapshot_atomic<'a>(&'a self, bytes: &'a [u8]) -> ControlFuture<'a, ()> {
        Box::pin(async move { atomic_write(&resolve_control_file(&self.root, SNAPSHOT_REL)?, bytes) })
    }
    fn load_processed<'a>(&'a self) -> ControlFuture<'a, Option<ProcessedState>> {
        Box::pin(async move {
            let bytes = bounded_read(&resolve_control_file(&self.root, PROCESSED_FILE)?)?;
            if bytes.is_empty() { Ok(None) } else { Ok(Some(serde_json::from_slice(&bytes)?)) }
        })
    }
    fn store_processed<'a>(&'a self, state: &'a ProcessedState) -> ControlFuture<'a, ()> {
        Box::pin(async move {
            let bytes = serde_json::to_vec(state)?;
            atomic_write(&resolve_control_file(&self.root, PROCESSED_FILE)?, &bytes)
        })
    }
}

/// Docker control I/O: expert root exec reads/writes the control dirs.
#[derive(Debug, Clone)]
pub struct DockerControlIo {
    handle: SandboxHandle,
}
impl DockerControlIo {
    /// Creates Docker control I/O for one session container.
    pub fn new(handle: SandboxHandle) -> Self {
        Self { handle }
    }
}
#[rustfmt::skip]
async fn run_root_ok(handle: &SandboxHandle, argv: &[String], stdin: Option<&[u8]>) -> Result<Vec<u8>, ControlError> {
    let spec: CommandSpec = exec_root(handle.container_name(), argv, stdin.is_some()).map_err(|e| ControlError::Io(e.to_string()))?;
    let outcome = run_bounded(&spec, stdin, OP_TIMEOUT, MAX_CONTROL_READ_BYTES).await.map_err(|e| ControlError::Io(e.to_string()))?;
    if outcome.success() { Ok(outcome.stdout) } else { Err(ControlError::Io(format!("docker exec failed: {}", outcome.stderr_text()))) }
}
fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| (*p).to_string()).collect()
}
#[rustfmt::skip]
impl ControlIo for DockerControlIo {
    fn setup_dirs<'a>(&'a self, candidate_user: &'a str) -> ControlFuture<'a, ()> {
        let user = candidate_user.to_string();
        Box::pin(async move {
            run_root_ok(&self.handle, &argv(&["mkdir", "-p", INBOX_DIR, OUTBOX_DIR]), None).await?;
            run_root_ok(&self.handle, &argv(&["chown", &format!("{user}:{user}"), INBOX_DIR]), None).await?;
            run_root_ok(&self.handle, &argv(&["chmod", "0755", CONTROL_ROOT, INBOX_DIR, OUTBOX_DIR]), None).await?;
            Ok(())
        })
    }
    fn read_intents<'a>(&'a self) -> ControlFuture<'a, Vec<u8>> {
        Box::pin(async move {
            let path = format!("{CONTROL_ROOT}/{INTENT_REL}");
            let link_check = argv(&["test", "!", "-L", &path]);
            if run_root_ok(&self.handle, &link_check, None).await.is_err() {
                let exists = argv(&["test", "-e", &path]);
                if run_root_ok(&self.handle, &exists, None).await.is_ok() {
                    return Err(ControlError::SymlinkRejected(path));
                }
                return Ok(Vec::new());
            }
            match run_root_ok(&self.handle, &argv(&["cat", &path]), None).await {
                Ok(bytes) => Ok(bytes.into_iter().take(MAX_CONTROL_READ_BYTES).collect()),
                Err(_) => Ok(Vec::new()),
            }
        })
    }
    fn write_snapshot_atomic<'a>(&'a self, bytes: &'a [u8]) -> ControlFuture<'a, ()> {
        Box::pin(async move {
            let tmp = format!("{OUTBOX_DIR}/.snapshot-{}.tmp", crate::orchestration::control::generate_intent_id());
            let target = format!("{CONTROL_ROOT}/{SNAPSHOT_REL}");
            run_root_ok(&self.handle, &argv(&["tee", &tmp]), Some(bytes)).await?;
            run_root_ok(&self.handle, &argv(&["chmod", "0644", &tmp]), None).await?;
            run_root_ok(&self.handle, &argv(&["mv", &tmp, &target]), None).await?;
            Ok(())
        })
    }
    fn load_processed<'a>(&'a self) -> ControlFuture<'a, Option<ProcessedState>> {
        Box::pin(async move {
            let path = format!("{CONTROL_ROOT}/{PROCESSED_FILE}");
            match run_root_ok(&self.handle, &argv(&["cat", &path]), None).await {
                Ok(bytes) if !bytes.is_empty() => Ok(Some(serde_json::from_slice(&bytes)?)),
                _ => Ok(None),
            }
        })
    }
    fn store_processed<'a>(&'a self, state: &'a ProcessedState) -> ControlFuture<'a, ()> {
        Box::pin(async move {
            let bytes = serde_json::to_vec(state)?;
            let tmp = format!("{CONTROL_ROOT}/.processed-{}.tmp", crate::orchestration::control::generate_intent_id());
            let target = format!("{CONTROL_ROOT}/{PROCESSED_FILE}");
            run_root_ok(&self.handle, &argv(&["tee", &tmp]), Some(&bytes)).await?;
            run_root_ok(&self.handle, &argv(&["chmod", "0644", &tmp]), None).await?;
            run_root_ok(&self.handle, &argv(&["mv", &tmp, &target]), None).await?;
            Ok(())
        })
    }
}

/// Builds candidate-user docker exec argv (live-test outbox probe).
#[rustfmt::skip]
pub fn candidate_exec_argv(handle: &SandboxHandle, user: &EphemeralUsername, argv_in: &[String]) -> Result<CommandSpec, ControlError> {
    exec_as(handle.container_name(), user, argv_in, false).map_err(|e| ControlError::Io(e.to_string()))
}
/// Timeout for short docker control operations.
pub fn control_timeout() -> Duration {
    OP_TIMEOUT
}
