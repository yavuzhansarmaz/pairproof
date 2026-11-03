//! UI-independent orchestration that owns a core session.

pub mod chat;
pub mod cleanup;
pub mod coding;
pub mod connection;
pub mod consistency;
pub mod control;
pub mod control_client;
pub mod control_codec;
pub mod control_server;
pub mod error;
pub mod evidence;
pub mod finalize;
pub mod health;
pub mod lifecycle;
pub mod mutagen_auth;
pub mod pipeline;
pub(crate) mod process_util;
pub mod recovery;
pub mod report;
pub mod secret_push;
pub mod session;
pub mod ssh_auth;
pub mod sync_gate;

pub use chat::{ChatTransport, MetaChatTransport};
pub use cleanup::{
    assert_artifacts_clean, collect_solution_codes, write_bundle, ArtifactBundle,
    CleanupCoordinator,
};
pub use coding::{
    CodingStepOutcome, CodingTransport, DockerSandboxIo, MetaCodingTransport, SandboxIo,
};
pub use connection::{
    ConnectionGate, ConnectionGateError, GatePhase, HostKeyVerifier, SshKeyscanVerifier,
};
pub use consistency::{compare_grade_runs, run_consistency, ConsistencyReport, QuestionDelta};
pub use control::{
    build_snapshot, build_snapshot_with_recovery, generate_intent_id, parse_intent_bytes,
    parse_intent_line, parse_snapshot, resolve_control_file, snapshot_service_name,
    CandidateSnapshot, ControlError, Intent, IntentKind, ParsedIntents, ProcessedState,
    SnapshotFinal, SnapshotOption, SnapshotQuestion, SnapshotRecovery, SnapshotSelection,
    SnapshotTest, CONTROL_ROOT, INBOX_DIR, INTENT_FILE, INTENT_REL, INTENT_VERSION,
    MAX_CONTROL_READ_BYTES, MAX_INTENT_BYTES, OUTBOX_DIR, PROCESSED_FILE, SNAPSHOT_FILE,
    SNAPSHOT_REL, SNAPSHOT_VERSION,
};
pub use control_client::{CandidateControlClient, SshControlClient};
pub use control_server::{
    publish_snapshot, ApplyOutcome, ApplyReport, ControlExecutor, ControlFuture, ControlIo,
    ControlTransports, DockerControlIo, LocalControlIo,
};
pub use error::OrchestratorError;
pub use evidence::{
    per_question_evidence, render_evidence, score_line_evidence, summary_evidence, EventRef,
    PerQuestionEvidence, Provenance, ScoreEvidence, SummaryEvidence,
};
pub use finalize::{
    both_roles_final, compute_final, final_visible_to_candidate, grade_session, session_elapsed_ms,
    FinalizeError, GradeFuture, MetaQuestionGrader, QuestionGrader,
};
pub use health::{
    classify_llm_error, classify_meta_error, retry_bounded, HealthError, HealthFailure, HealthGate,
    InterruptedStep, LeetCodeHealthProbe, LeetCodeProbe, MetaHealthProbe, MetaProbe, Probes,
    RecoveryOutcome, RetryPolicy, ServiceHealth, ServiceKind, ServiceProbes, Sleeper, TokioSleeper,
};
pub use lifecycle::{
    run_cancellable, CancelToken, CleanupFuture, CleanupReport, CleanupSequence, CleanupStep,
    KillReport, LifecycleError, RequestSupervisor,
};
pub use mutagen_auth::{
    run_mutagen, scp_wrapper_script, start_password_feeder, stop_isolated_daemon, wrapper_script,
    MutagenAuthEnv, MutagenOutcome, PasswordFeeder,
};
pub use pipeline::{
    fetched_from_question, ApproachGenerator, FetchedStatement, LeetcodeStatementFetcher,
    MetaApproachGenerator, PipelineError, PipelineFuture, QuestionPipeline, StatementFetcher,
    TestRunRecord,
};
pub use recovery::{
    classify_leetcode_recoverability, classify_llm_recoverability, classify_meta_recoverability,
    classify_pipeline_recoverability, classify_status, is_recoverable, PauseClock, Recoverability,
    RecoveryController, RecoveryError, RecoveryPhase, RecoveryState, RecoveryTimer,
};
pub use report::{generate_report, limitations_footer, ChatExcerpt};
pub use session::Orchestrator;
pub use ssh_auth::{
    helper_script, run_ssh_command, run_ssh_command_with_stdin, ssh_invocation,
    verify_and_key_line, AskpassSession, FeederHandle, SshAuthError, SshInvocation,
};
pub use sync_gate::{SessionSyncPhase, SyncGateError};

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Source of monotonic wall-clock time in milliseconds.
pub trait Clock: Send + Sync {
    /// Returns the current time in milliseconds.
    fn now_ms(&self) -> u64;
}

/// Clock backed by the system wall clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    /// Returns milliseconds since the Unix epoch.
    fn now_ms(&self) -> u64 {
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        u64::try_from(ms).unwrap_or(u64::MAX)
    }
}

/// Deterministic clock controlled by tests.
#[derive(Debug)]
pub struct ManualClock {
    now: AtomicU64,
}

impl ManualClock {
    /// Creates a clock fixed at `initial_ms`.
    pub fn new(initial_ms: u64) -> Self {
        Self {
            now: AtomicU64::new(initial_ms),
        }
    }

    /// Sets the current time to `ms`.
    pub fn set(&self, ms: u64) {
        self.now.store(ms, Ordering::SeqCst);
    }

    /// Advances the current time by `delta_ms`.
    pub fn advance(&self, delta_ms: u64) {
        self.now.fetch_add(delta_ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    /// Returns the manually controlled time.
    fn now_ms(&self) -> u64 {
        self.now.load(Ordering::SeqCst)
    }
}

impl Default for ManualClock {
    /// Creates a clock fixed at zero.
    fn default() -> Self {
        Self::new(0)
    }
}
