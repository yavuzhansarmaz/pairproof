//! Lifecycle-owned async cancellation and ordered cleanup.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use tokio::sync::watch;
use tokio::task::JoinHandle;

/// Errors from lifecycle cancellation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LifecycleError {
    /// The request was cancelled before it completed.
    #[error("request cancelled")]
    Cancelled,
}

/// Cloneable cancellation token backed by a `tokio::sync::watch` channel.
#[derive(Debug, Clone)]
pub struct CancelToken {
    tx: watch::Sender<bool>,
    rx: watch::Receiver<bool>,
}

impl CancelToken {
    /// Creates a token that is not cancelled.
    pub fn new() -> Self {
        let (tx, rx) = watch::channel(false);
        Self { tx, rx }
    }

    /// Cancels the token; all clones observe the cancellation.
    pub fn cancel(&self) {
        let _ = self.tx.send(true);
    }

    /// Returns whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        *self.rx.borrow()
    }

    /// Resolves once cancellation has been requested.
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        let mut rx = self.rx.clone();
        while rx.changed().await.is_ok() {
            if *rx.borrow() {
                return;
            }
        }
    }
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Outcome of attempting to cancel in-flight work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    Cancelled,
    Unsupported,
}

/// Report from one kill-switch execution.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KillReport {
    pub cancelled: Vec<String>,
    pub already_done: bool,
}

/// Record of every cleanup step attempted.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CleanupReport {
    pub attempted: Vec<String>,
    pub succeeded: Vec<String>,
    pub failed: Vec<(String, String)>,
    pub already_done: bool,
}

impl CleanupReport {
    /// Returns true when no attempted step failed.
    pub fn all_succeeded(&self) -> bool {
        self.failed.is_empty()
    }

    /// Derives the cancel outcome from the recorded step.
    pub fn cancel_outcome(&self) -> Option<CancelOutcome> {
        let n = "cancel_inflight";
        let bad = self
            .failed
            .iter()
            .any(|(s, m)| s == n && m.contains("unsupported"));
        if self.succeeded.iter().any(|s| s == n) {
            Some(CancelOutcome::Cancelled)
        } else if bad {
            Some(CancelOutcome::Unsupported)
        } else {
            None
        }
    }
}

struct NamedTask {
    name: String,
    handle: JoinHandle<()>,
}

/// Owns the session cancel token and every in-flight request handle.
pub struct RequestSupervisor {
    token: CancelToken,
    tasks: Vec<NamedTask>,
    killed: bool,
}

impl RequestSupervisor {
    /// Creates a supervisor owning a fresh cancel token.
    pub fn new() -> Self {
        Self {
            token: CancelToken::new(),
            tasks: Vec::new(),
            killed: false,
        }
    }

    /// Creates a supervisor owning `token`.
    pub fn with_token(token: CancelToken) -> Self {
        Self {
            token,
            tasks: Vec::new(),
            killed: false,
        }
    }

    /// Returns a clone of the owned cancel token.
    pub fn token(&self) -> CancelToken {
        self.token.clone()
    }

    /// Returns whether the kill switch has fired.
    pub fn is_killed(&self) -> bool {
        self.killed
    }

    /// Spawns `future` and tracks its handle under `name`.
    pub fn spawn<F>(&mut self, name: impl Into<String>, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let handle = tokio::spawn(future);
        self.tasks.push(NamedTask {
            name: name.into(),
            handle,
        });
    }

    /// Returns how many tracked handles have not finished.
    pub fn pending_count(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| !t.handle.is_finished())
            .count()
    }

    /// Cancels the token, aborts all handles and awaits them (bounded).
    /// A second call is a no-op and reports `already_done`.
    pub async fn kill_switch(&mut self) -> KillReport {
        if self.killed {
            return KillReport {
                cancelled: Vec::new(),
                already_done: true,
            };
        }
        self.killed = true;
        self.token.cancel();
        let mut cancelled = Vec::new();
        for task in std::mem::take(&mut self.tasks) {
            let NamedTask { name, handle } = task;
            if handle.is_finished() {
                let _ = handle.await;
            } else {
                cancelled.push(name);
                handle.abort();
                let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
            }
        }
        KillReport {
            cancelled,
            already_done: false,
        }
    }
}

impl Default for RequestSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

/// Runs `future` until it completes or `token` is cancelled. On
/// cancellation the future is dropped and never polled to completion.
pub async fn run_cancellable<T, F>(token: &CancelToken, future: F) -> Result<T, LifecycleError>
where
    F: Future<Output = T>,
{
    tokio::select! {
        biased;
        () = token.cancelled() => Err(LifecycleError::Cancelled),
        out = future => Ok(out),
    }
}

/// Boxed future produced by one cleanup step.
pub type CleanupFuture = Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;

/// One injected async cleanup step.
pub type CleanupStep = Box<dyn FnMut() -> CleanupFuture + Send>;

/// Fixed cleanup step names, in execution order.
pub const CLEANUP_STEP_NAMES: [&str; 5] = [
    "cancel_inflight",
    "terminate_connection",
    "stop_sync",
    "destroy_sandbox",
    "delete_raw",
];

/// Ordered cleanup: every step is attempted even after a failure,
/// failures are collected, and a second run attempts nothing.
pub struct CleanupSequence {
    steps: Vec<(String, CleanupStep)>,
    done: bool,
}

impl CleanupSequence {
    /// Creates a sequence from the five injected steps, in fixed order.
    pub fn new(
        cancel_inflight: CleanupStep,
        terminate_connection: CleanupStep,
        stop_sync: CleanupStep,
        destroy_sandbox: CleanupStep,
        delete_raw: CleanupStep,
    ) -> Self {
        let steps = vec![
            (CLEANUP_STEP_NAMES[0].to_string(), cancel_inflight),
            (CLEANUP_STEP_NAMES[1].to_string(), terminate_connection),
            (CLEANUP_STEP_NAMES[2].to_string(), stop_sync),
            (CLEANUP_STEP_NAMES[3].to_string(), destroy_sandbox),
            (CLEANUP_STEP_NAMES[4].to_string(), delete_raw),
        ];
        Self { steps, done: false }
    }

    /// Creates a sequence from named steps, preserving the given order.
    pub fn with_steps(steps: Vec<(String, CleanupStep)>) -> Self {
        Self { steps, done: false }
    }

    /// Returns true after the sequence has run.
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Runs every step once, in order, collecting failures by name.
    pub async fn run(&mut self) -> CleanupReport {
        if self.done {
            return CleanupReport {
                attempted: Vec::new(),
                succeeded: Vec::new(),
                failed: Vec::new(),
                already_done: true,
            };
        }
        self.done = true;
        let mut report = CleanupReport::default();
        for (name, step) in &mut self.steps {
            report.attempted.push(name.clone());
            match step().await {
                Ok(()) => report.succeeded.push(name.clone()),
                Err(message) => report.failed.push((name.clone(), message)),
            }
        }
        report
    }
}
