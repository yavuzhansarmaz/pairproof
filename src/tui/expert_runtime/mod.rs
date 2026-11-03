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
use crate::leetcode::selection::{pool_with_fallback, FallbackReport, PoolClient};
use crate::leetcode::LeetCodeClient;
use crate::llm::extraction::ExtractionResult;
use crate::orchestration::control::{
    check_app_versions, parse_final_ack, parse_heartbeat, parse_hello, FinalAckOutcome,
};
use crate::orchestration::{
    build_snapshot_with_recovery, compute_final, generate_report, grade_session, publish_snapshot,
    run_cancellable, ApplyOutcome, ApplyReport, ApproachGenerator, ChatTransport,
    CleanupCoordinator, CleanupReport, CleanupSequence, Clock, CodingTransport, ConnectionGate,
    ConnectionGateError, ControlError, ControlExecutor, ControlIo, ControlTransports,
    FetchedStatement, GatePhase, HealthGate, KillReport, LeetCodeProbe, LifecycleError, MetaProbe,
    Orchestrator, OrchestratorError, PauseClock, PipelineError, QuestionGrader, QuestionPipeline,
    RecoveryController, RecoveryOutcome, RequestSupervisor, SandboxIo, SessionSyncPhase,
    StatementFetcher,
};
use crate::sandbox::SandboxHandle;
use crate::scoring::{ApproachSet, GradeOutcome};
use crate::tui::candidate_final::{candidate_final_lines, CandidateFinal};
use crate::tui::monitor::{final_view, monitor_view, MonitorView};

mod build;
mod connection;
mod hello;
mod recovery;
mod selection;
mod session;
pub use self::build::expert_context_from_extraction;
pub(crate) use self::build::{build_expert_runtime, meta_client};
pub use self::hello::{
    ConsumeFuture, DockerHelloSource, HelloFuture, HelloSource, LocalHelloSource,
    SessionPublishFuture,
};
pub use self::recovery::RecoverySnapshot;

/// Errors from the expert runtime; payloads never carry secrets.
#[derive(Debug, thiserror::Error)]
pub enum ExpertRuntimeError {
    #[error(transparent)]
    Orchestrator(#[from] OrchestratorError),
    #[error(transparent)]
    Gate(#[from] ConnectionGateError),
    #[error(transparent)]
    Control(#[from] ControlError),
    #[error(transparent)]
    Pipeline(#[from] PipelineError),
    #[error(transparent)]
    Finalize(#[from] crate::orchestration::finalize::FinalizeError),
    #[error("{0}")]
    Message(String),
}

/// Outcome of one candidate poll on the connection screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollOutcome {
    Waiting,
    Connected,
    WrongSession,
    AlreadyConnected,
    VersionMismatch,
}

pub use crate::sandbox::preflight::first_free_port;
pub(crate) use crate::tui::expert_session::select_sandbox_port;
pub use crate::tui::expert_session::{format_session_id, SANDBOX_PORT_RANGE};

/// Missed-liveness window before the expert marks the candidate disconnected.
pub const HEARTBEAT_TIMEOUT_MS: u64 = 30_000;
/// Selection retry floor: one attempt per 30s per screen while failing.
pub const SELECTION_RETRY_MS: u64 = 30_000;
/// Selection screens with independent failure backoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionScreen {
    Medium,
    Hard,
}
/// Identity inputs for one expert session, from sandbox provisioning.
pub struct ExpertSessionSpec {
    pub session_id: String,
    pub language: Language,
    pub easy: Vec<QuestionRef>,
    pub endpoint: Endpoint,
    pub username: EphemeralUsername,
    pub fingerprint: Fingerprint,
}

/// Injected transports and storage for one expert runtime.
pub struct ExpertRuntimeDeps {
    pub control_io: Box<dyn ControlIo>,
    pub hello: Box<dyn HelloSource>,
    pub fetcher: Arc<dyn StatementFetcher>,
    pub generator: Arc<dyn ApproachGenerator>,
    pub coding: Arc<dyn CodingTransport>,
    pub sandbox_io: Arc<dyn SandboxIo>,
    pub chat: Arc<dyn ChatTransport>,
    pub grader: Arc<dyn QuestionGrader>,
    pub leetcode: Option<LeetCodeClient>,
    pub sandbox_handle: Option<SandboxHandle>,
    pub artifact_root: PathBuf,
    pub topics: Vec<String>,
    pub level: String,
    pub sync_phase: SessionSyncPhase,
}

struct LoadOutcome {
    session_id: String,
    index: usize,
    result: Result<Result<(FetchedStatement, ApproachSet), PipelineError>, LifecycleError>,
}

struct CannedFetcher(FetchedStatement);
impl StatementFetcher for CannedFetcher {
    fn fetch_statement<'a>(
        &'a self,
        _slug: &'a str,
        _l: Language,
    ) -> crate::orchestration::PipelineFuture<'a, FetchedStatement> {
        let fetched = self.0.clone();
        Box::pin(async move { Ok(fetched) })
    }
}
struct CannedGenerator(Mutex<Option<ApproachSet>>);
impl ApproachGenerator for CannedGenerator {
    fn generate<'a>(
        &'a self,
        _s: &'a str,
        _l: Language,
    ) -> crate::orchestration::PipelineFuture<'a, ApproachSet> {
        Box::pin(async move {
            self.0
                .lock()
                .ok()
                .and_then(|mut slot| slot.take())
                .ok_or(PipelineError::NoApproaches)
        })
    }
    fn grade_other<'a>(
        &'a self,
        _s: &'a str,
        _t: &'a str,
    ) -> crate::orchestration::PipelineFuture<'a, crate::scoring::ApproachGrade> {
        Box::pin(async move { Err(PipelineError::Llm("canned generator cannot grade".into())) })
    }
}

/// Live expert session: orchestration state behind the TTY adapter.
pub struct ExpertRuntime {
    orch: Orchestrator,
    gate: ConnectionGate,
    pipeline: QuestionPipeline,
    executor: ControlExecutor,
    control_io: Box<dyn ControlIo>,
    hello: Box<dyn HelloSource>,
    supervisor: RequestSupervisor,
    health: HealthGate,
    fetcher: Arc<dyn StatementFetcher>,
    generator: Arc<dyn ApproachGenerator>,
    coding: Arc<dyn CodingTransport>,
    sandbox_io: Arc<dyn SandboxIo>,
    chat: Arc<dyn ChatTransport>,
    grader: Arc<dyn QuestionGrader>,
    pool: Option<Arc<dyn PoolClient>>,
    sandbox_handle: Option<SandboxHandle>,
    layout: ArtifactLayout,
    topics: Vec<String>,
    level: String,
    sync_phase: SessionSyncPhase,
    started: bool,
    ended: bool,
    selection: selection::SelectionState,
    pending_load: Option<(usize, tokio::sync::oneshot::Receiver<LoadOutcome>)>,
    loaded_index: Option<usize>,
    cleanup_report: Option<CleanupReport>,
    last_kill: Option<KillReport>,
    final_ack_outcome: Option<FinalAckOutcome>,
    ack_waited: bool,
    hb_timeout_ms: u64,
    last_seen_ms: u64,
    clock: Option<Arc<dyn Clock>>,
    final_grades: Vec<GradeOutcome>,
    last_hb_at: Option<u64>,
    reissued: Option<ConnectionString>,
    username: EphemeralUsername,
    pushed_window: Option<u64>,
    secret_pusher: Option<Arc<dyn crate::orchestration::secret_push::SecretPush>>,
    hello_pending_consume: bool,
    version_mismatch: Option<(String, String)>,
    recovery: RecoveryController,
    pause_clock: PauseClock,
    recovery_rx: Option<(u64, tokio::sync::oneshot::Receiver<RecoveryOutcome>)>,
    recovery_cancel: Option<crate::orchestration::CancelToken>,
    leetcode_probe: Option<Arc<dyn LeetCodeProbe>>,
    meta_probe: Option<Arc<dyn MetaProbe>>,
    recovery_timed_out: bool,
    port_mapping: Option<crate::sandbox::nat::PortMapping>,
}

impl ExpertRuntime {
    /// Builds the runtime and moves the core to waiting-for-candidate.
    /// The caller sets up control dirs through the injected I/O first.
    pub async fn new(
        spec: ExpertSessionSpec,
        deps: ExpertRuntimeDeps,
        now_ms: u64,
    ) -> Result<Self, ExpertRuntimeError> {
        let mut orch =
            Orchestrator::create(spec.session_id.clone(), spec.language, spec.easy, now_ms)?;
        orch.mark_waiting_for_candidate(now_ms)?;
        let gate = ConnectionGate::new(
            spec.endpoint,
            spec.username.clone(),
            spec.fingerprint,
            now_ms,
        );
        let executor = ControlExecutor::restore(&spec.session_id, &*deps.control_io).await?;
        deps.hello.publish_session_binding(&spec.session_id).await?;
        if let Some(handle) = &deps.sandbox_handle {
            hello::write_session_docker(handle, &spec.session_id).await?;
        }
        let secret_pusher: Option<Arc<dyn crate::orchestration::secret_push::SecretPush>> =
            deps.sandbox_handle.clone().map(|handle| {
                Arc::new(crate::orchestration::secret_push::DockerSecretPush::new(
                    handle,
                    spec.username.clone(),
                )) as Arc<dyn crate::orchestration::secret_push::SecretPush>
            });
        let layout = ArtifactLayout::create(&deps.artifact_root, &spec.session_id, spec.language)
            .map_err(|e| ExpertRuntimeError::Message(e.to_string()))?;
        Ok(Self {
            orch,
            gate,
            pipeline: QuestionPipeline::new(),
            executor,
            control_io: deps.control_io,
            hello: deps.hello,
            supervisor: RequestSupervisor::new(),
            health: HealthGate::default(),
            fetcher: deps.fetcher,
            generator: deps.generator,
            coding: deps.coding,
            sandbox_io: deps.sandbox_io,
            chat: deps.chat,
            grader: deps.grader,
            pool: deps.leetcode.map(|c| Arc::new(c) as Arc<dyn PoolClient>),
            sandbox_handle: deps.sandbox_handle,
            layout,
            topics: deps.topics,
            level: deps.level,
            sync_phase: deps.sync_phase,
            started: false,
            ended: false,
            selection: selection::SelectionState::default(),
            pending_load: None,
            loaded_index: None,
            cleanup_report: None,
            last_kill: None,
            final_ack_outcome: None,
            ack_waited: false,
            hb_timeout_ms: HEARTBEAT_TIMEOUT_MS,
            last_seen_ms: now_ms,
            last_hb_at: None,
            reissued: None,
            username: spec.username,
            pushed_window: None,
            secret_pusher,
            hello_pending_consume: false,
            version_mismatch: None,
            clock: None,
            recovery: RecoveryController::new(60_000),
            pause_clock: PauseClock::new(),
            recovery_rx: None,
            recovery_cancel: None,
            leetcode_probe: None,
            meta_probe: None,
            recovery_timed_out: false,
            final_grades: Vec::new(),
            port_mapping: None,
        })
    }

    pub fn orchestrator(&self) -> &Orchestrator {
        &self.orch
    }
    pub fn gate_phase(&self) -> GatePhase {
        self.gate.phase()
    }
    /// Gate driving recovery probes: `begin_recovery` spawns its
    /// `probe_both` loop off the TTY tick and `poll_recovery` consumes it.
    pub fn health_gate(&self) -> HealthGate {
        self.health
    }
    pub fn is_killed(&self) -> bool {
        self.supervisor.is_killed()
    }
    pub fn pending_count(&self) -> usize {
        self.supervisor.pending_count()
    }
    pub fn cleanup_report(&self) -> Option<&CleanupReport> {
        self.cleanup_report.as_ref()
    }
    pub fn final_ack_outcome(&self) -> Option<FinalAckOutcome> {
        self.final_ack_outcome
    }
    pub fn sandbox_handle(&self) -> Option<&SandboxHandle> {
        self.sandbox_handle.as_ref()
    }
    /// Returns the session artifact layout for the raw-data lifecycle.
    pub fn layout(&self) -> &ArtifactLayout {
        &self.layout
    }
    /// Expert-owned sync phase published in snapshots. The Mutagen
    /// session is candidate-started (`CandidateRuntime` transport), so
    /// this runtime has no `SyncSessionInfo` to derive it from and never
    /// queries a sync daemon itself; it starts at `NotStarted` (no sync
    /// started by this runtime) until a real sync owner reports otherwise.
    pub fn sync_phase(&self) -> SessionSyncPhase {
        self.sync_phase
    }
    /// Records a sync phase reported by the real sync owner; the label
    /// is published at the next snapshot build.
    pub fn set_sync_phase(&mut self, phase: SessionSyncPhase) {
        self.sync_phase = phase;
    }
    /// Returns the gate's current string; displaying it never locks.
    pub fn current_string(&mut self, now_ms: u64) -> Result<ConnectionString, ConnectionGateError> {
        self.gate.current_string(now_ms)
    }
    /// Seconds left in the current rotation window.
    pub fn countdown_secs(&self, now_ms: u64) -> u64 {
        self.gate.countdown_secs(now_ms)
    }
    pub fn set_heartbeat_timeout(&mut self, timeout_ms: u64) {
        self.hb_timeout_ms = timeout_ms;
    }
    fn fresh_now(&self, fallback: u64) -> u64 {
        self.clock.as_ref().map_or(fallback, |clock| clock.now_ms())
    }
    pub fn is_disconnected(&self) -> bool {
        self.started && !self.ended && !self.orch.is_connected()
    }
    pub fn reissue_string(&self) -> Option<&ConnectionString> {
        self.reissued.as_ref()
    }
    pub fn username(&self) -> &EphemeralUsername {
        &self.username
    }
    /// Attaches the session's router port mapping, removed at session end.
    pub(crate) fn set_port_mapping(&mut self, mapping: Option<crate::sandbox::nat::PortMapping>) {
        self.port_mapping = mapping;
    }
    /// Returns true when a router port mapping is held for this session.
    pub fn has_port_mapping(&self) -> bool {
        self.port_mapping.is_some()
    }
    pub fn version_mismatch(&self) -> Option<(String, String)> {
        self.version_mismatch.clone()
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
