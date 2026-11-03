use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use pairproof::core::{read_events, ArtifactLayout, ContextSummary, Language};
use pairproof::leetcode::Difficulty;
use pairproof::orchestration::cleanup::{assert_artifacts_clean, CleanupCoordinator};
use pairproof::orchestration::finalize::FinalizeError;
use pairproof::orchestration::lifecycle::{CleanupReport, CleanupSequence, CleanupStep};
use pairproof::orchestration::{Orchestrator, PipelineError, PipelineFuture, SandboxIo};
use pairproof::sandbox::{RunnabilityReport, TestOutcome};

use crate::common_mod;
use common_mod::qs;

type Log = Arc<Mutex<Vec<String>>>;
type Codes = HashMap<usize, String>;
type PF<'a, T> = PipelineFuture<'a, T>;

struct TempDir(PathBuf);
impl TempDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("pairproof-cleanup-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        Self(dir)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn started() -> Orchestrator {
    let mut o = Orchestrator::create(
        "s1".into(),
        Language::Python,
        qs("e", 5, Difficulty::Easy),
        0,
    )
    .expect("create");
    o.mark_waiting_for_candidate(1).expect("waiting");
    o.attach_candidate(2).expect("attach");
    o.start(3).expect("start");
    o.finish_question(10).expect("finish 0");
    o.skip_question(11).expect("skip 1");
    o
}
fn completed() -> Orchestrator {
    let mut o = started();
    o.set_expert_questions(
        qs("m", 3, Difficulty::Medium),
        qs("h", 2, Difficulty::Hard),
        20,
    )
    .expect("expert");
    for i in 0..8u64 {
        o.finish_question(30 + i).expect("finish");
    }
    o.mark_completed(100).expect("completed");
    o
}
fn layout(tmp: &TempDir) -> ArtifactLayout {
    ArtifactLayout::create(tmp.0.join("root"), "s1", Language::Python).expect("layout")
}
fn context(o: &Orchestrator) -> ContextSummary {
    ContextSummary::from_session(o.session(), vec!["arrays".into()], "mid".into())
}
fn read(p: impl AsRef<Path>) -> String {
    std::fs::read_to_string(p).expect("read")
}
async fn fin(
    c: &mut CleanupCoordinator,
    l: &ArtifactLayout,
    o: &Orchestrator,
    codes: &Codes,
) -> Result<CleanupReport, FinalizeError> {
    c.finalize_and_cleanup(l, o, "# r\n", &context(o), codes)
        .await
}
fn step(name: &'static str, log: Log, fail: bool) -> CleanupStep {
    Box::new(move || {
        let log = log.clone();
        Box::pin(async move {
            log.lock().expect("lock").push(name.to_string());
            if fail {
                Err("boom".into())
            } else {
                Ok(())
            }
        })
    })
}
fn seq_with(log: Log, fourth: CleanupStep, fifth: CleanupStep) -> CleanupSequence {
    CleanupSequence::new(
        step("cancel_inflight", log.clone(), false),
        step("terminate_connection", log.clone(), false),
        step("stop_sync", log.clone(), false),
        fourth,
        fifth,
    )
}
fn coordinator(log: Log, fail: Option<&'static str>) -> CleanupCoordinator {
    let mk = |n: &'static str| (n.to_string(), step(n, log.clone(), fail == Some(n)));
    let steps = vec![
        mk("cancel_inflight"),
        mk("terminate_connection"),
        mk("stop_sync"),
        mk("destroy_sandbox"),
        mk("delete_raw"),
    ];
    CleanupCoordinator::new(CleanupSequence::with_steps(steps))
}
struct FakeSandbox {
    log: Log,
    codes: Codes,
    fail_read: bool,
}
impl SandboxIo for FakeSandbox {
    fn write_solution<'a>(&'a self, _i: usize, _l: Language, _c: &'a str) -> PF<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn read_solution<'a>(&'a self, i: usize, _l: Language) -> PF<'a, String> {
        let log = self.log.clone();
        let code = self.codes.get(&i).cloned().unwrap_or_default();
        let fail = self.fail_read;
        Box::pin(async move {
            log.lock().expect("lock").push(format!("read:{i}"));
            if fail {
                Err(PipelineError::Sandbox("copy failed".into()))
            } else {
                Ok(code)
            }
        })
    }
    // Only `read_solution` is exercised by finalize; the rest is unused.
    fn check_runnable<'a>(&'a self, _i: usize, _l: Language) -> PF<'a, RunnabilityReport> {
        Box::pin(async { unimplemented!("read-only fake") })
    }
    fn run_requested_tests<'a>(&'a self, _i: usize, _l: Language) -> PF<'a, TestOutcome> {
        Box::pin(async { unimplemented!("read-only fake") })
    }
}

#[tokio::test]
async fn artifact_bundle_contains_all_section15_files() {
    let (tmp, o) = (TempDir::new("bundle"), started());
    let layout = layout(&tmp);
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let mut coord = coordinator(log, None);
    let codes = HashMap::from([(0usize, "print(1)\n".to_string())]);
    let report = coord
        .finalize_and_cleanup(&layout, &o, "# report\n", &context(&o), &codes)
        .await
        .expect("finalize");
    assert!(report.all_succeeded());
    assert!(layout.events_path().exists() && layout.report_path().exists());
    assert!(layout.tokens_path().exists() && layout.context_path().exists());
    assert_eq!(read(layout.solution_path(0)), "print(1)\n");
    assert_eq!(read(layout.report_path()), "# report\n");
    let events = read_events(layout.events_path()).expect("events");
    assert_eq!(events.as_slice(), o.session().events());
    assert!(!layout.raw_dir().exists());
}

#[tokio::test]
async fn solution_files_are_copied_before_container_destroy() {
    let (tmp, o) = (TempDir::new("copy-order"), started());
    let layout = layout(&tmp);
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let solutions = layout.solutions_dir().to_path_buf();
    let destroy_log = log.clone();
    let destroy: CleanupStep = Box::new(move || {
        let (log, solutions) = (destroy_log.clone(), solutions.clone());
        Box::pin(async move {
            let present = solutions.join("01.py").exists();
            log.lock()
                .expect("lock")
                .push(format!("destroy_sandbox:solutions_present={present}"));
            Ok(())
        })
    });
    let seq = seq_with(log.clone(), destroy, step("delete_raw", log.clone(), false));
    let mut coord = CleanupCoordinator::new(seq);
    fin(
        &mut coord,
        &layout,
        &o,
        &HashMap::from([(0usize, "print(9)\n".to_string())]),
    )
    .await
    .expect("finalize");
    assert!(log
        .lock()
        .expect("lock")
        .iter()
        .any(|e| e == "destroy_sandbox:solutions_present=true"));
}

#[tokio::test]
async fn raw_pastes_are_not_in_final_artifacts() {
    let (tmp, o) = (TempDir::new("raw"), started());
    let layout = layout(&tmp);
    std::fs::write(layout.raw_dir().join("jd.txt"), "RAW-JD-CANARY-123").expect("raw jd");
    std::fs::write(layout.raw_dir().join("additional.txt"), "RAW-NOTES-CANARY").expect("raw");
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let mut coord = coordinator(log, None);
    fin(
        &mut coord,
        &layout,
        &o,
        &HashMap::from([(0usize, "x=1\n".to_string())]),
    )
    .await
    .expect("finalize");
    assert!(!layout.raw_dir().exists());
    assert_artifacts_clean(
        layout.session_dir(),
        &["RAW-JD-CANARY-123", "RAW-NOTES-CANARY"],
    )
    .expect("clean");
}

#[tokio::test]
async fn api_key_and_connection_secret_never_appear_in_artifacts() {
    let (tmp, o) = (TempDir::new("secrets"), started());
    let layout = layout(&tmp);
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let mut coord = coordinator(log, None);
    fin(
        &mut coord,
        &layout,
        &o,
        &HashMap::from([(0usize, "x=1\n".to_string())]),
    )
    .await
    .expect("finalize");
    let canaries = ["[REDACTED]", "conn-secret-canary-111"];
    assert_artifacts_clean(layout.session_dir(), &canaries).expect("clean");
    let texts = [
        read(layout.events_path()),
        read(layout.report_path()),
        read(layout.tokens_path()),
        read(layout.context_path()),
        read(layout.solution_path(0)),
    ];
    for text in &texts {
        for c in canaries {
            assert!(!text.contains(c), "canary leaked");
        }
    }
    std::fs::write(layout.report_path(), "leak [REDACTED]").expect("plant");
    let err = assert_artifacts_clean(layout.session_dir(), &canaries).expect_err("detect");
    assert!(!format!("{err}").contains("[REDACTED]") && !format!("{err:?}").contains("[REDACTED]"));
}

#[tokio::test]
async fn cleanup_is_idempotent_for_every_terminal_path() {
    for (name, kind) in [("completed", 0u8), ("ended", 1u8), ("technical", 2u8)] {
        let tmp = TempDir::new(name);
        let layout = layout(&tmp);
        let o = match kind {
            0 => completed(),
            1 => {
                let mut o = started();
                o.end_by_expert(50).expect("ended");
                o
            }
            _ => {
                let mut o = started();
                o.mark_technical_failure(50).expect("technical");
                o
            }
        };
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let mut coord = coordinator(log.clone(), None);
        fin(&mut coord, &layout, &o, &HashMap::new())
            .await
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let before = read(layout.report_path());
        assert_eq!(
            fin(&mut coord, &layout, &o, &HashMap::new()).await,
            Err(FinalizeError::AlreadyFinalized),
            "{name}"
        );
        assert_eq!(log.lock().expect("lock").len(), 5, "{name} steps once");
        assert_eq!(read(layout.report_path()), before, "{name} not rewritten");
    }
}

#[tokio::test]
async fn cleanup_failure_does_not_mask_primary_session_outcome() {
    let (tmp, o) = (TempDir::new("mask"), started());
    let layout = layout(&tmp);
    std::fs::write(layout.events_path(), "{\"seq\":0}\n").expect("plant events");
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let mut coord = coordinator(log.clone(), Some("destroy_sandbox"));
    let err = fin(&mut coord, &layout, &o, &HashMap::new())
        .await
        .expect_err("primary kept");
    assert!(matches!(err, FinalizeError::Artifact(_)), "{err:?}");
    assert_eq!(log.lock().expect("lock").len(), 5);
    let failed = &coord.last_report().expect("report").failed;
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].0, "destroy_sandbox");
    let tmp2 = TempDir::new("mask-ok");
    let layout2 =
        ArtifactLayout::create(tmp2.0.join("root"), "s1", Language::Python).expect("layout2");
    let log2: Log = Arc::new(Mutex::new(Vec::new()));
    let mut coord2 = coordinator(log2, Some("stop_sync"));
    let report = fin(&mut coord2, &layout2, &o, &HashMap::new())
        .await
        .expect("bundle outcome wins");
    assert_eq!(report.failed.len(), 1);
}

#[tokio::test]
async fn mutagen_termination_does_not_delete_local_workspace() {
    let (tmp, o) = (TempDir::new("mutagen"), started());
    let local = tmp.0.join("candidate-local");
    std::fs::create_dir_all(local.join("questions/01")).expect("local dir");
    std::fs::write(local.join("questions/01/solution.py"), "keep me\n").expect("local file");
    let layout = layout(&tmp);
    std::fs::write(layout.raw_dir().join("jd.txt"), "raw").expect("raw");
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let layout2 = layout.clone();
    let delete_raw: CleanupStep = Box::new(move || {
        let layout = layout2.clone();
        Box::pin(async move { layout.delete_raw_dir().map_err(|e| e.to_string()) })
    });
    let seq = seq_with(
        log.clone(),
        step("destroy_sandbox", log.clone(), false),
        delete_raw,
    );
    let mut coord = CleanupCoordinator::new(seq);
    fin(&mut coord, &layout, &o, &HashMap::new())
        .await
        .expect("finalize");
    assert_eq!(read(local.join("questions/01/solution.py")), "keep me\n");
    assert!(!layout.raw_dir().exists());
}

#[tokio::test]
async fn container_destroy_occurs_after_artifact_collection() {
    let (tmp, o) = (TempDir::new("destroy-order"), started());
    let layout = layout(&tmp);
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let sandbox = FakeSandbox {
        log: log.clone(),
        codes: HashMap::from([(0usize, "print(3)\n".to_string())]),
        fail_read: false,
    };
    let mut coord = coordinator(log.clone(), None);
    coord
        .finalize_and_cleanup_with_sandbox(&layout, &o, "# r\n", &context(&o), &sandbox, &[0])
        .await
        .expect("finalize");
    let entries = log.lock().expect("lock").clone();
    assert!(
        entries.iter().position(|e| e == "read:0")
            < entries.iter().position(|e| e == "destroy_sandbox"),
        "{entries:?}"
    );
    assert_eq!(read(layout.solution_path(0)), "print(3)\n");
    let tmp2 = TempDir::new("destroy-order-fail");
    let layout2 =
        ArtifactLayout::create(tmp2.0.join("root"), "s1", Language::Python).expect("layout2");
    let log2: Log = Arc::new(Mutex::new(Vec::new()));
    let failing = FakeSandbox {
        log: log2.clone(),
        codes: HashMap::new(),
        fail_read: true,
    };
    let mut coord2 = coordinator(log2.clone(), None);
    let err = coord2
        .finalize_and_cleanup_with_sandbox(&layout2, &o, "# r\n", &context(&o), &failing, &[0])
        .await
        .expect_err("copy primary");
    assert!(matches!(err, FinalizeError::Io(_)), "{err:?}");
    assert!(log2
        .lock()
        .expect("lock")
        .iter()
        .any(|e| e == "destroy_sandbox"));
}
