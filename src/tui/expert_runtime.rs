//! Expert session runtime: real orchestration behind the TTY loop.
//! Candidate hello evidence arrives as `inbox/hello.json` (written
//! candidate-side in 10I-2 after SSH auth); only a valid hello locks.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::connection::{ConnectionString, Endpoint, EphemeralUsername, Fingerprint};
use crate::core::types::Difficulty;
use crate::core::{ArtifactLayout, ContextSummary, Language, QuestionRef};
use crate::ingest::{prepare_documents, store_raw_documents, ExpertDocuments};
use crate::leetcode::selection::{select_questions_with_fallback, FallbackReport, PoolClient};
use crate::leetcode::LeetCodeClient;
use crate::llm::extraction::{extract_context, ExtractionResult};
#[rustfmt::skip]
use crate::orchestration::control::{parse_final_ack, parse_heartbeat, FinalAckOutcome};
#[rustfmt::skip]
use crate::orchestration::{build_snapshot_with_recovery, compute_final, generate_report, grade_session, publish_snapshot, run_cancellable, ApplyOutcome, ApplyReport, ApproachGenerator, ChatTransport, CleanupCoordinator, CleanupReport, CleanupSequence, Clock, CodingTransport, ConnectionGate, ConnectionGateError, ControlError, ControlExecutor, ControlIo, ControlTransports, FetchedStatement, GatePhase, HealthGate, KillReport, LeetCodeProbe, LifecycleError, MetaProbe, Orchestrator, OrchestratorError, PauseClock, PipelineError, QuestionGrader, QuestionPipeline, RecoveryController, RecoveryOutcome, RequestSupervisor, SandboxIo, SessionSyncPhase, StatementFetcher};
use crate::sandbox::SandboxHandle;
use crate::scoring::ApproachSet;
use crate::tui::monitor::{monitor_view, MonitorView};

#[path = "expert_recovery.rs"]
mod expert_recovery;
#[path = "hello.rs"]
mod hello;
pub use self::expert_recovery::RecoverySnapshot;
pub use self::hello::{
    ConsumeFuture, DockerHelloSource, HelloFuture, HelloSource, LocalHelloSource,
    SessionPublishFuture,
};

/// Errors from the expert runtime; payloads never carry secrets.
#[derive(Debug, thiserror::Error)]
#[rustfmt::skip]
pub enum ExpertRuntimeError {
    #[error(transparent)] Orchestrator(#[from] OrchestratorError),
    #[error(transparent)] Gate(#[from] ConnectionGateError),
    #[error(transparent)] Control(#[from] ControlError),
    #[error(transparent)] Pipeline(#[from] PipelineError),
    #[error(transparent)] Finalize(#[from] crate::orchestration::finalize::FinalizeError),
    #[error("{0}")] Message(String),
}

/// Outcome of one candidate poll on the connection screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[rustfmt::skip]
pub enum PollOutcome { Waiting, Connected, WrongSession, AlreadyConnected }

#[derive(serde::Deserialize)]
#[rustfmt::skip]
struct HelloFile { v: u32, session_id: String }

pub use crate::sandbox::preflight::first_free_port;
pub(crate) use crate::tui::expert_session::select_sandbox_port;
pub use crate::tui::expert_session::{format_session_id, SANDBOX_PORT_RANGE};

/// Missed-liveness window before the expert marks the candidate disconnected.
pub const HEARTBEAT_TIMEOUT_MS: u64 = 30_000;
/// Selection retry floor: one attempt per 30s per screen while failing.
pub const SELECTION_RETRY_MS: u64 = 30_000;
/// Selection screens with independent failure backoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[rustfmt::skip]
pub enum SelectionScreen { Medium, Hard }
/// Builds a Meta client from the environment or the stored API key.
#[rustfmt::skip]
fn meta_client() -> Result<crate::llm::MetaClient, String> { if let Ok(client) = crate::llm::MetaClient::from_env() { return Ok(client); } let path = crate::tui::config::default_api_key_path().ok_or_else(|| "no API key path".to_string())?; let key = crate::tui::config::resolve_api_key(&path).map_err(|e| e.to_string())?; Ok(crate::llm::MetaClient::new(crate::llm::DEFAULT_BASE_URL, &key)) }
/// Maps an extraction result to runtime context (test seam for the build path).
#[rustfmt::skip]
pub fn expert_context_from_extraction(result: &ExtractionResult) -> (Vec<String>, String, Language) { (result.topics.clone(), result.level.clone(), result.proposed_language.unwrap_or(Language::Python)) }
/// Builds the expert-visible note for a widened selection report.
#[rustfmt::skip]
fn widened_note(report: &FallbackReport) -> Option<String> { if !report.widened { return None; } let tags = if report.tags_used.is_empty() { "unfiltered".to_string() } else { report.tags_used.join(", ") }; Some(format!("Selection widened beyond topics ({tags}).")) }
/// Resolves Setup pastes to stored documents plus runtime context.
#[rustfmt::skip]
async fn resolve_expert_context(jd: &str, notes: &str) -> Result<(Option<ExpertDocuments>, Vec<String>, String, Language), String> {
    if jd.trim().is_empty() || notes.trim().is_empty() { return Ok((None, Vec::new(), String::new(), Language::Python)); }
    let docs = prepare_documents(jd, notes).map_err(|e| e.to_string())?; let outcome = extract_context(&meta_client()?, &docs.jd.text, docs.additional_text().unwrap_or("")).await.map_err(|e| format!("extraction failed: {e}"))?;
    let (topics, level, language) = expert_context_from_extraction(&outcome.value); Ok((Some(docs), topics, level, language))
}

/// Builds the real expert runtime from the Setup pastes.
#[rustfmt::skip]
pub(crate) async fn build_expert_runtime(now: u64, jd: &str, notes: &str) -> Result<ExpertRuntime, String> {
    use crate::connection::{Endpoint, EphemeralUsername, Secret}; use crate::leetcode::selection::auto_pick_easy; use crate::leetcode::LeetCodeClient;
    use crate::orchestration::{ControlIo, DockerControlIo, LeetcodeStatementFetcher, MetaApproachGenerator, MetaChatTransport, MetaCodingTransport, MetaQuestionGrader};
    use crate::sandbox::lifecycle::{create_and_provision, destroy, rotate_secret}; use crate::orchestration::Clock;
    let (docs, topics, level, language) = resolve_expert_context(jd, notes).await?;
    let leetcode = LeetCodeClient::default(); let mut rng = rand::thread_rng();
    let easy = auto_pick_easy(&leetcode, &[], &mut rng).await.map_err(|e| e.to_string())?;
    let session_id = format_session_id(std::process::id(), now);
    let port = select_sandbox_port().ok_or_else(|| "no free sandbox port".to_string())?;
    let endpoint = Endpoint::new("127.0.0.1", port).map_err(|e| e.to_string())?; let username = EphemeralUsername::generate(); let secret0 = Secret::generate();
    let (handle, fingerprint) = create_and_provision(&session_id, &endpoint, &username, &secret0).await.map_err(|e| e.to_string())?;
    let built: Result<ExpertRuntime, String> = async {
        let control_io = DockerControlIo::new(handle.clone());
        control_io.setup_dirs(username.as_str()).await.map_err(|e| e.to_string())?;
        let deps = ExpertRuntimeDeps {
            control_io: Box::new(control_io), hello: Box::new(DockerHelloSource::new(handle.clone())),
            fetcher: Arc::new(LeetcodeStatementFetcher::new(LeetCodeClient::default())), generator: Arc::new(MetaApproachGenerator::new(meta_client()?)),
            coding: Arc::new(MetaCodingTransport::new(meta_client()?)), sandbox_io: Arc::new(crate::orchestration::DockerSandboxIo::new(handle.clone(), username.clone())),
            chat: Arc::new(MetaChatTransport::new(meta_client()?)), grader: Arc::new(MetaQuestionGrader::new(meta_client()?)),
            leetcode: Some(leetcode), sandbox_handle: Some(handle.clone()), artifact_root: std::path::PathBuf::from("sessions"),
            topics, level, sync_phase: SessionSyncPhase::NotStarted,
        };
        let spec = ExpertSessionSpec { session_id: session_id.clone(), language, easy, endpoint: endpoint.clone(), username: username.clone(), fingerprint };
        let fresh_now = crate::orchestration::SystemClock.now_ms(); let mut runtime = ExpertRuntime::new(spec, deps, fresh_now).await.map_err(|e| e.to_string())?; runtime.set_clock(Arc::new(crate::orchestration::SystemClock)); if let Ok(meta) = meta_client() { runtime.set_health_probes(Arc::new(crate::orchestration::LeetCodeHealthProbe::new(LeetCodeClient::default())), Arc::new(crate::orchestration::MetaHealthProbe::new(meta))); }
        if let Some(docs) = &docs { store_raw_documents(runtime.layout(), docs).map_err(|e| e.to_string())?; }
        let current = runtime.current_string(fresh_now).map_err(|e| e.to_string())?;
        rotate_secret(&handle, &username, current.secret()).await.map_err(|e| e.to_string())?; runtime.pushed_window = Some(current.expires_at_ms());
        Ok(runtime)
    }.await;
    match built { Ok(runtime) => Ok(runtime), Err(err) => { let _ = destroy(&handle).await; Err(err) } }
}

/// Identity inputs for one expert session, from sandbox provisioning.
#[rustfmt::skip]
pub struct ExpertSessionSpec { pub session_id: String, pub language: Language, pub easy: Vec<QuestionRef>, pub endpoint: Endpoint, pub username: EphemeralUsername, pub fingerprint: Fingerprint }

/// Injected transports and storage for one expert runtime.
#[rustfmt::skip]
pub struct ExpertRuntimeDeps {
    pub control_io: Box<dyn ControlIo>, pub hello: Box<dyn HelloSource>, pub fetcher: Arc<dyn StatementFetcher>, pub generator: Arc<dyn ApproachGenerator>,
    pub coding: Arc<dyn CodingTransport>, pub sandbox_io: Arc<dyn SandboxIo>, pub chat: Arc<dyn ChatTransport>, pub grader: Arc<dyn QuestionGrader>,
    pub leetcode: Option<LeetCodeClient>, pub sandbox_handle: Option<SandboxHandle>, pub artifact_root: PathBuf,
    pub topics: Vec<String>, pub level: String, pub sync_phase: SessionSyncPhase,
}

#[rustfmt::skip]
struct LoadOutcome { session_id: String, index: usize, result: Result<Result<(FetchedStatement, ApproachSet), PipelineError>, LifecycleError> }

struct CannedFetcher(FetchedStatement);
#[rustfmt::skip]
impl StatementFetcher for CannedFetcher {
    fn fetch_statement<'a>(&'a self, _slug: &'a str, _l: Language) -> crate::orchestration::PipelineFuture<'a, FetchedStatement> { let fetched = self.0.clone(); Box::pin(async move { Ok(fetched) }) }
}
struct CannedGenerator(Mutex<Option<ApproachSet>>);
#[rustfmt::skip]
impl ApproachGenerator for CannedGenerator {
    fn generate<'a>(&'a self, _s: &'a str, _l: Language) -> crate::orchestration::PipelineFuture<'a, ApproachSet> { Box::pin(async move { self.0.lock().ok().and_then(|mut slot| slot.take()).ok_or(PipelineError::NoApproaches) }) }
    fn grade_other<'a>(&'a self, _s: &'a str, _t: &'a str) -> crate::orchestration::PipelineFuture<'a, crate::scoring::ApproachGrade> { Box::pin(async move { Err(PipelineError::Llm("canned generator cannot grade".into())) }) }
}

/// Live expert session: orchestration state behind the TTY adapter.
#[rustfmt::skip]
pub struct ExpertRuntime {
    orch: Orchestrator, gate: ConnectionGate, pipeline: QuestionPipeline, executor: ControlExecutor,
    control_io: Box<dyn ControlIo>, hello: Box<dyn HelloSource>, supervisor: RequestSupervisor, health: HealthGate, // NOT wired: see `health_gate`.
    fetcher: Arc<dyn StatementFetcher>, generator: Arc<dyn ApproachGenerator>, coding: Arc<dyn CodingTransport>,
    sandbox_io: Arc<dyn SandboxIo>, chat: Arc<dyn ChatTransport>, grader: Arc<dyn QuestionGrader>,
    pool: Option<Arc<dyn PoolClient>>, sandbox_handle: Option<SandboxHandle>, layout: ArtifactLayout,
    topics: Vec<String>, level: String, sync_phase: SessionSyncPhase, started: bool, ended: bool,
    pending_medium: Option<Vec<QuestionRef>>, pending_load: Option<(usize, tokio::sync::oneshot::Receiver<LoadOutcome>)>,
    loaded_index: Option<usize>, cleanup_report: Option<CleanupReport>, last_kill: Option<KillReport>,
    final_ack_outcome: Option<FinalAckOutcome>, ack_waited: bool, hb_timeout_ms: u64, last_seen_ms: u64, clock: Option<Arc<dyn Clock>>,
    last_hb_at: Option<u64>, reissued: Option<ConnectionString>, username: EphemeralUsername,
    selection_note: Option<String>, medium_failed_at: Option<u64>, hard_failed_at: Option<u64>, pushed_window: Option<u64>, secret_pusher: Option<Arc<dyn crate::orchestration::secret_push::SecretPush>>, hello_pending_consume: bool, recovery: RecoveryController, pause_clock: PauseClock, recovery_rx: Option<(u64, tokio::sync::oneshot::Receiver<RecoveryOutcome>)>, recovery_cancel: Option<crate::orchestration::CancelToken>, leetcode_probe: Option<Arc<dyn LeetCodeProbe>>, meta_probe: Option<Arc<dyn MetaProbe>>, recovery_timed_out: bool,
}

#[rustfmt::skip]
impl ExpertRuntime {
    /// Builds the runtime and moves the core to waiting-for-candidate.
    /// The caller sets up control dirs through the injected I/O first.
    pub async fn new(spec: ExpertSessionSpec, deps: ExpertRuntimeDeps, now_ms: u64) -> Result<Self, ExpertRuntimeError> {
        let mut orch = Orchestrator::create(spec.session_id.clone(), spec.language, spec.easy, now_ms)?;
        orch.mark_waiting_for_candidate(now_ms)?;
        let gate = ConnectionGate::new(spec.endpoint, spec.username.clone(), spec.fingerprint, now_ms);
        let executor = ControlExecutor::restore(&spec.session_id, &*deps.control_io).await?;
        deps.hello.publish_session_binding(&spec.session_id).await?; if let Some(handle) = &deps.sandbox_handle { hello::write_session_docker(handle, &spec.session_id).await?; } let secret_pusher: Option<Arc<dyn crate::orchestration::secret_push::SecretPush>> = deps.sandbox_handle.clone().map(|handle| Arc::new(crate::orchestration::secret_push::DockerSecretPush::new(handle, spec.username.clone())) as Arc<dyn crate::orchestration::secret_push::SecretPush>);
        let layout = ArtifactLayout::create(&deps.artifact_root, &spec.session_id, spec.language).map_err(|e| ExpertRuntimeError::Message(e.to_string()))?;
        Ok(Self { orch, gate, pipeline: QuestionPipeline::new(), executor, control_io: deps.control_io, hello: deps.hello,
            supervisor: RequestSupervisor::new(), health: HealthGate::default(), fetcher: deps.fetcher, generator: deps.generator,
            coding: deps.coding, sandbox_io: deps.sandbox_io, chat: deps.chat, grader: deps.grader, pool: deps.leetcode.map(|c| Arc::new(c) as Arc<dyn PoolClient>),
            sandbox_handle: deps.sandbox_handle, layout, topics: deps.topics, level: deps.level, sync_phase: deps.sync_phase,
            started: false, ended: false, pending_medium: None, pending_load: None, loaded_index: None, cleanup_report: None, last_kill: None, final_ack_outcome: None, ack_waited: false,
            hb_timeout_ms: HEARTBEAT_TIMEOUT_MS, last_seen_ms: now_ms, last_hb_at: None, reissued: None, username: spec.username, selection_note: None, medium_failed_at: None, hard_failed_at: None, pushed_window: None, secret_pusher, hello_pending_consume: false, clock: None, recovery: RecoveryController::new(60_000), pause_clock: PauseClock::new(), recovery_rx: None, recovery_cancel: None, leetcode_probe: None, meta_probe: None, recovery_timed_out: false })
    }

    pub fn orchestrator(&self) -> &Orchestrator { &self.orch } pub fn gate_phase(&self) -> GatePhase { self.gate.phase() }
    /// Implemented gate logic, NOT wired into TTY loop; wiring pending product/design decision on interception point + countdown propagation + non-blocking tick.
    pub fn health_gate(&self) -> HealthGate { self.health } pub fn is_killed(&self) -> bool { self.supervisor.is_killed() }
    pub fn pending_count(&self) -> usize { self.supervisor.pending_count() } pub fn cleanup_report(&self) -> Option<&CleanupReport> { self.cleanup_report.as_ref() }
    pub fn final_ack_outcome(&self) -> Option<FinalAckOutcome> { self.final_ack_outcome } pub fn sandbox_handle(&self) -> Option<&SandboxHandle> { self.sandbox_handle.as_ref() }
    /// Returns the session artifact layout for the raw-data lifecycle.
    pub fn layout(&self) -> &ArtifactLayout { &self.layout }
    /// Expert-owned sync phase published in snapshots. The Mutagen
    /// session is candidate-started (`CandidateRuntime` transport), so
    /// this runtime has no `SyncSessionInfo` to derive it from and never
    /// queries a sync daemon itself; it starts at `NotStarted` (no sync
    /// started by this runtime) until a real sync owner reports otherwise.
    pub fn sync_phase(&self) -> SessionSyncPhase { self.sync_phase }
    /// Records a sync phase reported by the real sync owner; the label
    /// is published at the next snapshot build.
    pub fn set_sync_phase(&mut self, phase: SessionSyncPhase) { self.sync_phase = phase; }
    /// Returns the gate's current string; displaying it never locks.
    pub fn current_string(&mut self, now_ms: u64) -> Result<ConnectionString, ConnectionGateError> { self.gate.current_string(now_ms) }
    /// Seconds left in the current rotation window.
    pub fn countdown_secs(&self, now_ms: u64) -> u64 { self.gate.countdown_secs(now_ms) }
    pub fn set_heartbeat_timeout(&mut self, timeout_ms: u64) { self.hb_timeout_ms = timeout_ms; } fn fresh_now(&self, fallback: u64) -> u64 { self.clock.as_ref().map_or(fallback, |clock| clock.now_ms()) } pub fn is_disconnected(&self) -> bool { self.started && !self.ended && !self.orch.is_connected() }
    pub fn reissue_string(&self) -> Option<&ConnectionString> { self.reissued.as_ref() } pub fn username(&self) -> &EphemeralUsername { &self.username }

    /// Polls for candidate hello evidence. Only a first valid
    /// session-bound hello locks the gate and starts the interview;
    /// polling without one never changes connection state.
    pub async fn poll_candidate(&mut self, now_ms: u64) -> PollOutcome {
        self.apply_pending_load(now_ms).await;
        if self.started && self.orch.is_connected() { return PollOutcome::AlreadyConnected; }
        // Persistent rm failure implies broken container exec, so snapshot
        // publish is broken too; expert End/cleanup paths never need hello.
        if self.hello_pending_consume { if self.hello.consume_hello().await.is_err() { return PollOutcome::Waiting; } self.hello_pending_consume = false; }
        let Some(bytes) = self.hello.read_hello().await else { return PollOutcome::Waiting; }; let Ok(hello) = serde_json::from_slice::<HelloFile>(&bytes) else { return PollOutcome::Waiting; };
        if hello.v != 1 { return PollOutcome::Waiting; }
        if hello.session_id != self.orch.session_id() { return PollOutcome::WrongSession; }
        if self.gate.accept_after_ssh_auth(&mut self.orch, now_ms).is_err() { return PollOutcome::Waiting; }
        self.hello_pending_consume = self.hello.consume_hello().await.is_err();
        if !self.started { if self.orch.start(now_ms).is_err() { return PollOutcome::Waiting; } self.started = true; } else { self.reissued = None; }
        self.last_seen_ms = now_ms;
        self.maybe_trigger_load();
        PollOutcome::Connected
    }

    /// Refreshes liveness from a fresh heartbeat; disconnects past the timeout.
    /// Only an advancing candidate `at_ms` counts, so a stale file never masks death.
    pub async fn check_heartbeat(&mut self, now_ms: u64) -> bool {
        if let Some(bytes) = self.hello.read_heartbeat().await { if let Ok(hb) = parse_heartbeat(&bytes, self.orch.session_id()) { if self.last_hb_at.is_none_or(|prev| hb.at_ms > prev) { self.last_hb_at = Some(hb.at_ms); self.last_seen_ms = now_ms; } } }
        if self.started && !self.ended && self.orch.is_connected() && now_ms.saturating_sub(self.last_seen_ms) >= self.hb_timeout_ms { let _ = self.gate.handle_disconnect(&mut self.orch, now_ms); return true; }
        false
    }

    /// Expert 'R': fresh string after disconnect, rotated into the sandbox.
    pub async fn reissue(&mut self, now_ms: u64) -> Result<ConnectionString, ExpertRuntimeError> {
        if !self.is_disconnected() { return Err(ExpertRuntimeError::Message("No disconnect to recover".into())); } let fresh = self.gate.reissue_after_disconnect(now_ms)?;
        if let Some(handle) = self.sandbox_handle.clone() { let user = self.username.clone(); crate::sandbox::lifecycle::rotate_secret(&handle, &user, fresh.secret()).await.map_err(|e| ExpertRuntimeError::Message(e.to_string()))?; }
        self.reissued = Some(fresh.clone()); self.pushed_window = Some(fresh.expires_at_ms()); Ok(fresh)
    }
    pub async fn refresh_pre_lock_secret(&mut self, now_ms: u64) -> Result<bool, ExpertRuntimeError> { if self.ended || self.orch.is_connected() { return Ok(false); } let Some(pusher) = self.secret_pusher.clone() else { return Ok(false); }; let current = self.gate.current_string(now_ms)?; if Some(current.expires_at_ms()) == self.pushed_window { return Ok(false); } pusher.push(current.secret()).await.map_err(|_| ExpertRuntimeError::Message("secret refresh push failed".into()))?; self.pushed_window = Some(current.expires_at_ms()); Ok(true) }

    /// Rejects a stale presented string against the live reissue window.
    pub fn verify_reconnect(&mut self, presented: &ConnectionString, now_ms: u64) -> Result<(), ExpertRuntimeError> {
        if !self.is_disconnected() { return Err(ExpertRuntimeError::Message("No disconnect to recover".into())); }
        let live = self.gate.current_string(now_ms)?; if presented != &live { return Err(ExpertRuntimeError::Message("stale connection string".into())); }
        Ok(())
    }

    /// Drains candidate intents once through the control executor, then publishes the snapshot;
    /// heartbeat is checked on a fresh clock before and after the drain, never the stale start time.
    pub async fn drain_intents(&mut self, now_ms: u64) -> Result<Vec<ApplyReport>, ExpertRuntimeError> {
        if self.ended { return Ok(Vec::new()); }
        self.check_heartbeat(self.fresh_now(now_ms)).await;
        self.apply_pending_load(now_ms).await;
        let transports = ControlTransports { coding: &*self.coding, sandbox: &*self.sandbox_io, generator: &*self.generator, chat: &*self.chat };
        let reports = self.executor.drain(&*self.control_io, &mut self.orch, &mut self.pipeline, &transports, now_ms).await?;
        if reports.iter().any(|r| r.outcome == ApplyOutcome::Applied) { self.last_seen_ms = self.fresh_now(now_ms); }
        let snapshot = build_snapshot_with_recovery(&self.orch, &self.pipeline, self.sync_phase, self.executor.chat_replies(), None, self.snapshot_recovery()); publish_snapshot(&*self.control_io, &snapshot).await?;
        self.maybe_trigger_load();
        self.check_heartbeat(self.fresh_now(now_ms)).await;
        Ok(reports)
    }

    /// Commits expert medium and hard picks through the core session.
    pub fn expert_select(&mut self, medium: Vec<QuestionRef>, hard: Vec<QuestionRef>, now_ms: u64) -> Result<(), ExpertRuntimeError> {
        self.orch.set_expert_questions(medium, hard, now_ms)?; self.pending_medium = None; self.maybe_trigger_load();
        Ok(())
    }

    /// Fetches three real medium questions for the medium screen.
    /// A cached pending set returns without touching the pool again.
    pub async fn fetch_medium(&mut self, now_ms: u64) -> Result<usize, ExpertRuntimeError> {
        if let Some(pending) = &self.pending_medium { return Ok(pending.len()); }
        let client = self.pool.as_ref().ok_or_else(|| ExpertRuntimeError::Message("question pool unavailable".into()))?;
        let topics: Vec<&str> = self.topics.iter().map(String::as_str).collect(); let mut rng = rand::thread_rng();
        let exclude: BTreeSet<String> = self.orch.session().slots().iter().map(|s| s.question.title_slug.clone()).collect();
        match select_questions_with_fallback(client.as_ref(), Difficulty::Medium, &topics, 3, &exclude, &mut rng).await {
            Ok((picked, report)) => { let count = picked.len(); self.pending_medium = Some(picked); self.medium_failed_at = None; self.selection_note = widened_note(&report); Ok(count) }
            Err(err) => { self.medium_failed_at = Some(now_ms); Err(ExpertRuntimeError::Message(err.to_string())) }
        }
    }

    /// Fetches two real hard questions and commits the expert set.
    pub async fn complete_selection(&mut self, now_ms: u64) -> Result<(), ExpertRuntimeError> {
        let medium = self.pending_medium.clone().ok_or_else(|| ExpertRuntimeError::Message("medium questions are not selected yet".into()))?;
        let client = self.pool.as_ref().ok_or_else(|| ExpertRuntimeError::Message("question pool unavailable".into()))?;
        let topics: Vec<&str> = self.topics.iter().map(String::as_str).collect();
        let mut exclude: BTreeSet<String> = self.orch.session().slots().iter().map(|s| s.question.title_slug.clone()).collect();
        for question in &medium { exclude.insert(question.title_slug.clone()); }
        let mut rng = rand::thread_rng();
        match select_questions_with_fallback(client.as_ref(), Difficulty::Hard, &topics, 2, &exclude, &mut rng).await {
            Ok((hard, report)) => { self.hard_failed_at = None; self.selection_note = widened_note(&report); self.expert_select(medium, hard, now_ms) }
            Err(err) => { self.hard_failed_at = Some(now_ms); Err(ExpertRuntimeError::Message(err.to_string())) }
        }
    }

    /// Returns false while the screen backs off after a failed attempt.
    /// Pure in `now_ms`: no wall-clock reads, first attempt is immediate.
    pub fn selection_allowed(&self, screen: SelectionScreen, now_ms: u64) -> bool { let failed = match screen { SelectionScreen::Medium => self.medium_failed_at, SelectionScreen::Hard => self.hard_failed_at }; failed.is_none_or(|at| now_ms.saturating_sub(at) >= SELECTION_RETRY_MS) }
    /// Expert-visible note from the last widened selection, if any.
    pub fn selection_note(&self) -> Option<&str> { self.selection_note.as_deref() }
    /// Overrides the question pool source (test seam for selection).
    pub fn set_pool(&mut self, pool: Arc<dyn PoolClient>) { self.pool = Some(pool); } pub fn set_secret_pusher(&mut self, pusher: Arc<dyn crate::orchestration::secret_push::SecretPush>) { self.secret_pusher = Some(pusher); } pub fn set_clock(&mut self, clock: Arc<dyn Clock>) { self.clock = Some(clock); }

    /// Live monitor data from permitted expert-side state only.
    /// Uses the pause-adjusted clock so elapsed freezes during recovery
    /// and excludes recovery duration after resume.
    pub fn monitor_view(&self, now_ms: u64) -> MonitorView { monitor_view(self.orch.session(), &self.pipeline.approach_qualities(&self.orch), self.adjusted_now_ms(now_ms)) }
    /// Ends the session with the production 60s final-ack wait.
    pub async fn end_session(&mut self, now_ms: u64) -> Result<CleanupReport, ExpertRuntimeError> { self.end_session_with_ack_deadline(now_ms, 60_000).await }
    /// Ends the session: publish the final snapshot, wait bounded
    /// for the candidate final ack, then run cleanup. A timeout
    /// proceeds anyway; a second call re-attempts nothing.
    pub async fn end_session_with_ack_deadline(&mut self, now_ms: u64, ack_deadline_ms: u64) -> Result<CleanupReport, ExpertRuntimeError> {
        if self.ended { return Ok(self.cleanup_report.clone().unwrap_or_default()); }
        self.ended = true; if self.is_recovery_active() { self.cancel_recovery(); } let kill = self.supervisor.kill_switch().await; self.last_kill = Some(kill);
        if self.gate.phase() != GatePhase::Ended { let _ = self.gate.terminate(&mut self.orch, now_ms); }
        let mut statements: HashMap<String, String> = HashMap::new();
        if let (Some(question), Some(fetched)) = (self.orch.current_question().cloned(), self.pipeline.current_statement(&self.orch)) { statements.insert(question.title_slug.clone(), fetched.statement); }
        let codes: HashMap<usize, String> = (0..self.orch.session().slots().len()).filter_map(|index| self.pipeline.current_code(index).map(|code| (index, code.to_string()))).collect();
        let (_grades, final_snapshot, report_md) = if self.is_technical_failure() { let snap = build_snapshot_with_recovery(&self.orch, &self.pipeline, self.sync_phase, self.executor.chat_replies(), None, self.snapshot_recovery()); (Vec::new(), snap, format!("# PairProof Session Report\n\nSession: {}\n\nStatus: TechnicalFailure\n\nSession ended due to technical failure.\n", self.orch.session_id())) } else { let grades = grade_session(&self.orch, &*self.grader, &statements, &codes).await?; let breakdown = compute_final(&self.orch, &self.pipeline.approach_qualities(&self.orch), &grades)?; let snap = build_snapshot_with_recovery(&self.orch, &self.pipeline, self.sync_phase, self.executor.chat_replies(), Some(&breakdown), self.snapshot_recovery()); let md = generate_report(self.orch.session(), &breakdown, &grades, &[]); (grades, snap, md) };
        let published = publish_snapshot(&*self.control_io, &final_snapshot).await;
        let ack_outcome = if published.is_ok() { Some(self.wait_for_final_ack(ack_deadline_ms).await) } else { None };
        let context = ContextSummary::from_session(self.orch.session(), self.topics.clone(), self.level.clone());
        let token = self.supervisor.token();
        let gate_ended = self.gate.phase() == GatePhase::Ended;
        let session_id = self.orch.session_id().to_string();
        let handle = self.sandbox_handle.clone();
        let layout = self.layout.clone();
        let sequence = CleanupSequence::new(
            Box::new(move || { let token = token.clone(); Box::pin(async move { token.cancel(); Ok(()) }) }),
            Box::new(move || Box::pin(async move { if gate_ended { Ok(()) } else { Err("connection not terminated".to_string()) } })),
            Box::new(move || { let session_id = session_id.clone(); Box::pin(async move { crate::sync::terminate_sync(&session_id).await.map_err(|e| e.to_string()) }) }),
            Box::new(move || { let handle = handle.clone(); Box::pin(async move { match &handle { Some(handle) => crate::sandbox::destroy(handle).await.map_err(|e| e.to_string()), None => Ok(()) } }) }),
            Box::new(move || { let layout = layout.clone(); Box::pin(async move { layout.delete_raw_dir().map_err(|e| e.to_string()) }) }),
        );
        let mut coordinator = CleanupCoordinator::new(sequence);
        let result = coordinator.finalize_and_cleanup(&self.layout, &self.orch, &report_md, &context, &codes).await;
        if let Some(report) = coordinator.last_report() { let mut stored = report.clone(); if ack_outcome == Some(FinalAckOutcome::Timeout) { stored.failed.push(("final_snapshot_ack".into(), "timeout".into())); } self.cleanup_report = Some(stored); }
        let mut report = result?;
        if ack_outcome == Some(FinalAckOutcome::Timeout) { report.failed.push(("final_snapshot_ack".into(), "timeout".into())); }
        published?;
        self.cleanup_report = Some(report.clone());
        Ok(report)
    }

    /// Polls the ack file every ~250ms until `deadline_ms`; a wrong
    /// session/version ack is ignored. Re-entry never waits twice.
    async fn wait_for_final_ack(&mut self, deadline_ms: u64) -> FinalAckOutcome {
        if self.ack_waited { return self.final_ack_outcome.unwrap_or(FinalAckOutcome::Timeout); } self.ack_waited = true;
        let deadline = tokio::time::Instant::now() + Duration::from_millis(deadline_ms);
        let outcome = loop {
            if let Some(bytes) = self.hello.read_final_ack().await { if parse_final_ack(&bytes, self.orch.session_id()).is_ok() { break FinalAckOutcome::Received; } }
            let now = tokio::time::Instant::now(); if now >= deadline { break FinalAckOutcome::Timeout; }
            tokio::time::sleep(Duration::from_millis(250).min(deadline.saturating_duration_since(now))).await;
        };
        self.final_ack_outcome = Some(outcome); outcome
    }

    fn maybe_trigger_load(&mut self) {
        if self.supervisor.is_killed() || self.supervisor.token().is_cancelled() { return; }
        let Some(index) = self.orch.current_index() else { return; }; if self.loaded_index == Some(index) { return; }
        if let Some((pending, _)) = &self.pending_load { if *pending == index { return; } }
        let Some(question) = self.orch.current_question() else { return; }; let slug = question.title_slug.clone();
        let session_id = self.orch.session_id().to_string(); let language = self.orch.session().language();
        let (tx, rx) = tokio::sync::oneshot::channel(); let fetcher = self.fetcher.clone(); let generator = self.generator.clone(); let token = self.supervisor.token();
        self.supervisor.spawn(format!("load-q{index}"), async move { let work = async move { let fetched = fetcher.fetch_statement(&slug, language).await?; let statement = fetched.statement.clone(); let set = generator.generate(&statement, language).await?; Ok((fetched, set)) }; let result = run_cancellable(&token, work).await; let _ = tx.send(LoadOutcome { session_id, index, result }); }); self.pending_load = Some((index, rx));
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_session_id_uses_e_pid_x_now() {
        assert_eq!(format_session_id(123, 456), "e123x456");
        assert_eq!(format_session_id(1, 1_730_000_000_000), "e1x1730000000000");
    }

    #[test]
    fn sandbox_port_range_preserves_historical_bounds() {
        assert_eq!(SANDBOX_PORT_RANGE, 2222..2322);
        assert_eq!(first_free_port(SANDBOX_PORT_RANGE.clone(), |_| false), None);
        assert_eq!(
            first_free_port(SANDBOX_PORT_RANGE, |p| p != 2222),
            Some(2223)
        );
    }
}
