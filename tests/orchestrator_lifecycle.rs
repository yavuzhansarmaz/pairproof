use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use pairproof::core::{Language, QuestionRef};
use pairproof::orchestration::lifecycle::{
    run_cancellable, CancelToken, CleanupSequence, CleanupStep, LifecycleError, RequestSupervisor,
};
use pairproof::orchestration::Orchestrator;

use crate::common_mod;
use common_mod::q_easy as q;

struct DropFlag(Arc<AtomicBool>);
impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn pending(dropped: Arc<AtomicBool>, delivered: Arc<AtomicBool>) -> impl Future<Output = ()> {
    let guard = DropFlag(dropped);
    async move {
        let _guard = guard;
        std::future::pending::<()>().await;
        delivered.store(true, Ordering::SeqCst);
    }
}

fn started() -> Orchestrator {
    let easy: Vec<QuestionRef> = (0..5).map(|i| q(&format!("e{i}"))).collect();
    let mut orch = Orchestrator::create("s1".into(), Language::Python, easy, 0).expect("create");
    orch.mark_waiting_for_candidate(1).expect("waiting");
    orch.attach_candidate(2).expect("attach");
    orch.start(3).expect("start");
    orch
}

async fn assert_kill_aborts(name: &str) {
    let dropped = Arc::new(AtomicBool::new(false));
    let delivered = Arc::new(AtomicBool::new(false));
    let mut supervisor = RequestSupervisor::new();
    supervisor.spawn(name, pending(dropped.clone(), delivered.clone()));
    assert_eq!(supervisor.pending_count(), 1);
    assert!(!supervisor.token().is_cancelled());
    let report = supervisor.kill_switch().await;
    assert_eq!(report.cancelled, vec![name.to_string()]);
    assert!(!report.already_done);
    assert!(supervisor.token().is_cancelled());
    assert!(dropped.load(Ordering::SeqCst), "aborted future must drop");
    assert!(!delivered.load(Ordering::SeqCst), "late value delivered");
    assert_eq!(supervisor.pending_count(), 0);
    let second = supervisor.kill_switch().await;
    assert!(second.already_done && second.cancelled.is_empty());
}

#[tokio::test]
async fn kill_switch_aborts_inflight_coding_request() {
    assert_kill_aborts("coding").await;
}

#[tokio::test]
async fn kill_switch_aborts_inflight_chat_request() {
    assert_kill_aborts("chat").await;
}

#[tokio::test]
async fn kill_switch_aborts_inflight_grading_request() {
    assert_kill_aborts("grading").await;
}

#[tokio::test]
async fn cancelled_request_cannot_write_solution_after_end() {
    let mut orch = started();
    orch.end_by_expert(9).expect("end by expert");
    assert!(orch.is_terminal());
    let writes: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let dropped = Arc::new(AtomicBool::new(false));
    let token = CancelToken::new();
    token.cancel();
    let sink = writes.clone();
    let guard = DropFlag(dropped.clone());
    let result: Result<(), LifecycleError> = run_cancellable(&token, async move {
        let _guard = guard;
        std::future::pending::<()>().await;
        sink.lock().expect("lock").push("solution".to_string());
    })
    .await;
    assert_eq!(result, Err(LifecycleError::Cancelled));
    assert!(dropped.load(Ordering::SeqCst), "cancelled future must drop");
    assert!(writes.lock().expect("lock").is_empty());
}

#[tokio::test]
async fn cancelled_request_cannot_record_late_tokens_or_events() {
    let mut orch = started();
    let tokens_before = orch.session().current_slot().expect("slot").actual_tokens;
    let events_before = orch.session().events().len();
    let token = CancelToken::new();
    token.cancel();
    let result: Result<(), LifecycleError> = run_cancellable(&token, async {
        orch.add_tokens(999).expect("tokens would record");
        orch.continue_work(10).expect("event would record");
    })
    .await;
    assert_eq!(result, Err(LifecycleError::Cancelled));
    assert_eq!(
        orch.session().current_slot().expect("slot").actual_tokens,
        tokens_before
    );
    assert_eq!(orch.session().events().len(), events_before);
}

type Log = Arc<Mutex<Vec<String>>>;

fn step(name: &'static str, log: Log, fail: bool) -> CleanupStep {
    Box::new(move || {
        let log = log.clone();
        let fut: Pin<Box<dyn Future<Output = Result<(), String>> + Send>> = Box::pin(async move {
            log.lock().expect("lock").push(name.to_string());
            if fail {
                Err("boom".to_string())
            } else {
                Ok(())
            }
        });
        fut
    })
}

#[tokio::test]
async fn kill_switch_cleanup_runs_once_and_in_order() {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let names = [
        "cancel_inflight",
        "terminate_connection",
        "stop_sync",
        "destroy_sandbox",
        "delete_raw",
    ];
    let mut sequence = CleanupSequence::new(
        step(names[0], log.clone(), false),
        step(names[1], log.clone(), false),
        step(names[2], log.clone(), true),
        step(names[3], log.clone(), false),
        step(names[4], log.clone(), false),
    );
    let report = sequence.run().await;
    let expected: Vec<String> = names.iter().map(ToString::to_string).collect();
    assert_eq!(report.attempted, expected);
    assert_eq!(*log.lock().expect("lock"), expected);
    assert_eq!(report.succeeded.len(), 4);
    assert_eq!(
        report.failed,
        vec![("stop_sync".to_string(), "boom".to_string())]
    );
    assert!(!report.already_done && !report.all_succeeded());
    let second = sequence.run().await;
    assert!(second.already_done);
    assert!(second.attempted.is_empty() && second.succeeded.is_empty());
    assert_eq!(log.lock().expect("lock").len(), 5, "second run attempts");
}
