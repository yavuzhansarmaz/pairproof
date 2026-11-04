use pairproof::core::{EventKind, Language, Session};
use pairproof::leetcode::Difficulty;
use pairproof::llm::coding::{CodingStepInput, CodingStepOutput};
use pairproof::llm::Message;
use pairproof::orchestration::*;
use pairproof::sandbox::{RunnabilityReport, TestOutcome};
use pairproof::scoring::{ApproachGrade, ApproachQuality, ApproachSet};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use crate::common_mod;
use common_mod::qs;

fn tmp(name: &str) -> PathBuf {
    common_mod::tmp("ctrl", name)
}
type PF<'a, T> = PipelineFuture<'a, T>;

#[rustfmt::skip]
fn started() -> Orchestrator {
    let mut o = Orchestrator::create("s1".into(), Language::Python, qs("e", 5, Difficulty::Easy), 0).unwrap();
    o.mark_waiting_for_candidate(1).unwrap(); o.attach_candidate(2).unwrap(); o.start(3).unwrap(); o
}

#[rustfmt::skip]
fn count(orch: &Orchestrator, pred: impl Fn(&EventKind) -> bool) -> usize { orch.session().events().iter().filter(|e| pred(&e.kind)).count() }
#[rustfmt::skip]
fn append(root: &Path, text: &str) {
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(root.join(INTENT_REL)).unwrap();
    f.write_all(text.as_bytes()).unwrap();
}
#[rustfmt::skip]
fn line(intent: &Intent) -> String { CandidateControlClient::intent_line(intent).unwrap() }
struct FakeFetcher;
#[rustfmt::skip]
impl StatementFetcher for FakeFetcher {
    fn fetch_statement<'a>(&'a self, slug: &'a str, _l: Language) -> PF<'a, FetchedStatement> {
        let s = slug.to_string();
        Box::pin(async move { Ok(FetchedStatement { statement: format!("Statement for {s}"), starter_code: "# starter".into() }) })
    }
}
#[derive(Default)]
struct FakeGenerator {
    grade_calls: AtomicU32,
}
#[rustfmt::skip]
impl ApproachGenerator for FakeGenerator {
    fn generate<'a>(&'a self, _s: &'a str, _l: Language) -> PF<'a, ApproachSet> {
        Box::pin(async move {
            let opt = |t: &str, quality: ApproachQuality, tok: u64| pairproof::scoring::ApproachOption { title: t.into(), summary: format!("{t} summary"), quality, expected_tokens: tok };
            ApproachSet::new(vec![opt("Brute force", ApproachQuality::Weak, 111), opt("Hash map", ApproachQuality::Best, 424242), opt("Sort first", ApproachQuality::Workable, 333), opt("Two pointers", ApproachQuality::Workable, 444)]).map_err(|e| PipelineError::Llm(e.to_string()))
        })
    }
    fn grade_other<'a>(&'a self, _s: &'a str, _t: &'a str) -> PF<'a, ApproachGrade> {
        Box::pin(async move { self.grade_calls.fetch_add(1, Ordering::SeqCst); ApproachGrade::new(ApproachQuality::Workable, 250, "sound".into()).map_err(|e| PipelineError::Llm(e.to_string())) })
    }
}
#[derive(Default)]
struct FakeCoding {
    calls: AtomicU32,
}
#[rustfmt::skip]
impl CodingTransport for FakeCoding {
    fn coding_step<'a>(&'a self, s: &'a mut Session, _i: &'a CodingStepInput) -> PF<'a, CodingStepOutput> {
        Box::pin(async move { self.calls.fetch_add(1, Ordering::SeqCst); s.add_tokens(10).map_err(|e| PipelineError::Llm(e.to_string()))?; Ok(CodingStepOutput { code: "def solve():\n    return 42".into(), ready: false, summary: "draft".into() }) })
    }
}
#[derive(Default)]
struct FakeSandbox {
    test_calls: AtomicU32,
    fail_tests: bool,
}
#[rustfmt::skip]
impl SandboxIo for FakeSandbox {
    fn write_solution<'a>(&'a self, _i: usize, _l: Language, _c: &'a str) -> PF<'a, ()> { Box::pin(async move { Ok(()) }) }
    fn read_solution<'a>(&'a self, _i: usize, _l: Language) -> PF<'a, String> { Box::pin(async move { Ok(String::new()) }) }
    fn check_runnable<'a>(&'a self, _i: usize, _l: Language) -> PF<'a, RunnabilityReport> {
        Box::pin(async move { Ok(RunnabilityReport { runnable: true, exit_code: Some(0), stdout: String::new(), stderr: String::new(), truncated: false }) })
    }
    fn run_requested_tests<'a>(&'a self, _i: usize, _l: Language) -> PF<'a, TestOutcome> {
        let failed = self.fail_tests;
        Box::pin(async move {
            self.test_calls.fetch_add(1, Ordering::SeqCst);
            Ok(TestOutcome {
                success: !failed,
                exit_code: Some(if failed { 1 } else { 0 }),
                stdout: "ok".into(),
                stderr: String::new(),
                stdout_truncated: false,
                stderr_truncated: false,
            })
        })
    }
}
struct ReadyCoding;
#[rustfmt::skip]
impl CodingTransport for ReadyCoding {
    fn coding_step<'a>(&'a self, s: &'a mut Session, _i: &'a CodingStepInput) -> PF<'a, CodingStepOutput> {
        Box::pin(async move { s.add_tokens(10).map_err(|e| PipelineError::Llm(e.to_string()))?; Ok(CodingStepOutput { code: "def solve():\n    return 42".into(), ready: true, summary: "done".into() }) })
    }
}
#[derive(Default)]
struct FakeChat {
    calls: AtomicU32,
    seen: Mutex<Vec<String>>,
}
#[rustfmt::skip]
impl ChatTransport for FakeChat {
    fn chat<'a>(&'a self, _s: &'a mut Session, _st: &'a pairproof::llm::chat::ChatState, _h: &'a [Message], msg: &'a str, _at: u64) -> PF<'a, String> {
        Box::pin(async move { self.calls.fetch_add(1, Ordering::SeqCst); self.seen.lock().unwrap().push(msg.into()); Ok("Think about a map.".into()) })
    }
}
struct Fakes {
    coding: FakeCoding,
    sandbox: FakeSandbox,
    generator: FakeGenerator,
    chat: FakeChat,
}
#[rustfmt::skip]
impl Fakes {
    fn new() -> Self { Self { coding: FakeCoding::default(), sandbox: FakeSandbox::default(), generator: FakeGenerator::default(), chat: FakeChat::default() } }
    fn transports(&self) -> ControlTransports<'_> { ControlTransports { coding: &self.coding, sandbox: &self.sandbox, generator: &self.generator, chat: &self.chat } }
}
#[rustfmt::skip]
async fn setup(name: &str) -> (PathBuf, LocalControlIo, ControlExecutor) {
    let root = tmp(name); let io = LocalControlIo::new(root.clone());
    io.setup_dirs("pp-testuser").await.unwrap();
    let executor = ControlExecutor::restore("s1", &io).await.unwrap();
    (root, io, executor)
}
#[rustfmt::skip]
async fn loaded_pipe(orch: &Orchestrator) -> QuestionPipeline {
    let mut pipe = QuestionPipeline::new();
    pipe.load_current_statement(orch, &FakeFetcher).await.unwrap();
    pipe.generate_current_approaches(orch, &FakeGenerator::default()).await.unwrap();
    pipe
}

#[tokio::test]
#[rustfmt::skip]
async fn control_dirs_separate_candidate_and_expert_writes() {
    assert_eq!(CONTROL_ROOT, "/pairproof-control");
    assert!(!CONTROL_ROOT.starts_with("/workspace"));
    let (root, io, _ex) = setup("dirs").await;
    assert!(root.join("inbox").is_dir() && root.join("outbox").is_dir());
    assert_ne!(root.join("inbox"), root.join("outbox"));
    let snap = build_snapshot(&started(), &QuestionPipeline::new(), SessionSyncPhase::Active, &[], None);
    publish_snapshot(&io, &snap).await.unwrap();
    assert!(root.join(SNAPSHOT_REL).is_file());
    assert!(!root.join("inbox").join(SNAPSHOT_FILE).exists());
}

#[test]
#[rustfmt::skip]
fn intent_schema_requires_id_session_seq_version() {
    let intent = Intent::new("s1", 1, IntentKind::Continue);
    assert_eq!(parse_intent_line(&intent.to_line().unwrap(), "s1"), Ok(intent.clone()));
    let no_id = r#"{"v":1,"session_id":"s1","seq":1,"kind":"Continue","payload":{}}"#;
    assert!(matches!(parse_intent_line(no_id, "s1"), Err(ControlError::Malformed(_))));
    let bad_v = intent.to_line().unwrap().replace("\"v\":1", "\"v\":2");
    assert_eq!(parse_intent_line(&bad_v, "s1"), Err(ControlError::WrongVersion(2)));
    let no_seq = r#"{"v":1,"id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","session_id":"s1","kind":"Continue","payload":{}}"#;
    assert!(matches!(parse_intent_line(no_seq, "s1"), Err(ControlError::Malformed(_))));
    let mut bad_id = intent.clone(); bad_id.id = "not-hex!".into();
    assert_eq!(bad_id.validate("s1"), Err(ControlError::InvalidId));
    let mut zero_seq = intent.clone(); zero_seq.seq = 0;
    assert!(matches!(zero_seq.validate("s1"), Err(ControlError::InvalidPayload(_))));
}

#[test]
#[rustfmt::skip]
fn request_requestion_serializes_with_legacy_wire_name() {
    assert_eq!(INTENT_VERSION, 1);
    let intent = Intent::new("s1", 1, IntentKind::RequestReQuestion);
    assert_eq!(intent.v, 1);
    assert_eq!(intent.kind.name(), "RequestRequestion");
    let line = intent.to_line().unwrap();
    assert!(line.contains(r#""kind":"RequestRequestion""#), "{line}");
    assert!(!line.contains("RequestReQuestion"), "{line}");
    assert_eq!(parse_intent_line(&line, "s1"), Ok(intent));
}

#[test]
#[rustfmt::skip]
fn change_approach_intent_roundtrips() {
    let intent = Intent::new("s1", 1, IntentKind::ChangeApproach);
    assert_eq!(intent.kind.name(), "ChangeApproach");
    assert!(intent.validate("s1").is_ok());
    let line = intent.to_line().unwrap();
    assert!(line.contains(r#""kind":"ChangeApproach""#), "{line}");
    assert_eq!(parse_intent_line(&line, "s1"), Ok(intent));
}

#[tokio::test]
#[rustfmt::skip]
async fn change_approach_intent_clears_the_selection() {
    let (root, io, mut ex) = setup("change-approach").await;
    let mut orch = started(); let mut pipe = loaded_pipe(&orch).await; let mut fakes = Fakes::new();
    // A clean run would close the question. Fail the run so the
    // selection stays and ChangeApproach still has something to clear.
    fakes.sandbox.fail_tests = true;
    append(&root, &line(&Intent::new("s1", 1, IntentKind::SelectPreset { preset: 0 })));
    append(&root, &line(&Intent::new("s1", 2, IntentKind::ChangeApproach)));
    let reports = ex.drain(&io, &mut orch, &mut pipe, &fakes.transports(), 10).await.unwrap();
    assert_eq!(reports.iter().map(|r| r.outcome).collect::<Vec<_>>(), vec![ApplyOutcome::Applied, ApplyOutcome::Applied], "{reports:?}");
    assert_eq!(count(&orch, |k| matches!(k, EventKind::ApproachChanged { .. })), 1);
    assert!(orch.session().current_slot().is_some_and(|s| s.approach.is_none()));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
#[rustfmt::skip]
fn parse_accepts_both_request_requestion_spellings() {
    for kind in ["RequestRequestion", "RequestReQuestion"] {
        let raw = format!(r#"{{"v":1,"id":"{}","session_id":"s1","seq":1,"kind":"{kind}","payload":{{}}}}"#, "a".repeat(32));
        let parsed = parse_intent_line(&raw, "s1").unwrap();
        assert_eq!(parsed.kind, IntentKind::RequestReQuestion, "{kind}");
        assert_eq!(parsed.v, INTENT_VERSION);
        // Re-serializing either accepted spelling emits only the legacy wire name.
        let line = parsed.to_line().unwrap();
        assert!(line.contains(r#""kind":"RequestRequestion""#), "{line}");
        assert_eq!(parse_intent_line(&line, "s1"), Ok(parsed));
    }
}

#[test]
#[rustfmt::skip]
fn generate_intent_id_is_32_lowercase_hex() {
    let a = generate_intent_id(); let b = generate_intent_id();
    for id in [&a, &b] { assert_eq!(id.len(), 32); assert!(id.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')), "{id}"); }
    assert_ne!(a, b);
}

#[test]
#[rustfmt::skip]
fn intent_allowlist_rejects_expert_only_actions() {
    for kind in ["ResolveRequestion", "ResolveReQuestion", "Finalize", "Grade", "GradeSession", "EndByExpert", "SetExpertQuestions", "RecordTestResult"] {
        let raw = format!(r#"{{"v":1,"id":"{}","session_id":"s1","seq":1,"kind":"{kind}","payload":{{}}}}"#, "a".repeat(32));
        assert_eq!(parse_intent_line(&raw, "s1"), Err(ControlError::ForbiddenKind(kind.into())), "{kind}");
    }
    for kind in [IntentKind::SelectPreset { preset: 0 }, IntentKind::Continue, IntentKind::Finish] {
        assert!(Intent::new("s1", 1, kind).validate("s1").is_ok());
    }
}

#[test]
#[rustfmt::skip]
fn intent_payload_size_is_bounded() {
    let big = Intent::new("s1", 1, IntentKind::SelectOther { text: "x".repeat(MAX_INTENT_BYTES) });
    assert_eq!(big.to_line(), Err(ControlError::Oversized));
    let line = format!("{}\n", "x".repeat(MAX_INTENT_BYTES + 1));
    assert_eq!(parse_intent_line(line.trim_end(), "s1"), Err(ControlError::Oversized));
    let ok = Intent::new("s1", 1, IntentKind::Redirect { text: "use a heap".into() });
    assert!(ok.to_line().unwrap().len() <= MAX_INTENT_BYTES);
}

#[tokio::test]
#[rustfmt::skip]
async fn duplicate_intent_is_applied_once() {
    let (root, io, mut ex) = setup("dup").await;
    let mut orch = started(); let mut pipe = QuestionPipeline::new(); let fakes = Fakes::new();
    let intent = Intent::new("s1", 1, IntentKind::Finish);
    append(&root, &line(&intent));
    let reports = ex.drain(&io, &mut orch, &mut pipe, &fakes.transports(), 10).await.unwrap();
    assert_eq!(reports[0].outcome, ApplyOutcome::Applied);
    assert_eq!(count(&orch, |k| matches!(k, EventKind::QuestionFinished { .. })), 1);
    let events = orch.session().events().len();
    let again = ex.drain(&io, &mut orch, &mut pipe, &fakes.transports(), 11).await.unwrap();
    assert_eq!(again[0].outcome, ApplyOutcome::Duplicate);
    assert_eq!(orch.session().events().len(), events);
}

#[tokio::test]
#[rustfmt::skip]
async fn sequence_gap_is_not_applied_out_of_order() {
    let (root, io, mut ex) = setup("gap").await;
    let mut orch = started(); let mut pipe = QuestionPipeline::new(); let fakes = Fakes::new();
    append(&root, &line(&Intent::new("s1", 2, IntentKind::Finish)));
    let reports = ex.drain(&io, &mut orch, &mut pipe, &fakes.transports(), 10).await.unwrap();
    assert_eq!(reports[0].outcome, ApplyOutcome::Rejected);
    assert!(reports[0].error.as_deref().unwrap().contains("sequence gap"));
    assert_eq!(orch.current_index(), Some(0));
    assert_eq!(count(&orch, |k| matches!(k, EventKind::QuestionFinished { .. })), 0);
    append(&root, &line(&Intent::new("s1", 1, IntentKind::Finish)));
    ex.drain(&io, &mut orch, &mut pipe, &fakes.transports(), 11).await.unwrap();
    assert_eq!(count(&orch, |k| matches!(k, EventKind::QuestionFinished { .. })), 2);
    assert_eq!(orch.current_index(), Some(2));
}

#[tokio::test]
#[rustfmt::skip]
async fn malformed_and_partial_lines_are_tolerated() {
    let first = Intent::new("s1", 1, IntentKind::Finish);
    let second = Intent::new("s1", 2, IntentKind::Finish);
    let bytes = format!("{}not json\n{}", line(&first), second.to_line().unwrap());
    let parsed = parse_intent_bytes(bytes.as_bytes(), "s1");
    assert_eq!(parsed.intents.len(), 1); assert_eq!(parsed.rejected.len(), 1);
    let (root, io, mut ex) = setup("partial").await;
    let mut orch = started(); let mut pipe = QuestionPipeline::new(); let fakes = Fakes::new();
    append(&root, &bytes);
    ex.drain(&io, &mut orch, &mut pipe, &fakes.transports(), 10).await.unwrap();
    assert_eq!(count(&orch, |k| matches!(k, EventKind::QuestionFinished { .. })), 1);
    append(&root, "\n");
    ex.drain(&io, &mut orch, &mut pipe, &fakes.transports(), 11).await.unwrap();
    assert_eq!(count(&orch, |k| matches!(k, EventKind::QuestionFinished { .. })), 2);
}

#[tokio::test]
#[rustfmt::skip]
async fn crash_restart_does_not_reapply_intent() {
    let (root, io, mut ex) = setup("crash").await;
    let mut orch = started(); let mut pipe = QuestionPipeline::new(); let fakes = Fakes::new();
    let intent = Intent::new("s1", 1, IntentKind::Finish);
    append(&root, &line(&intent));
    ex.drain(&io, &mut orch, &mut pipe, &fakes.transports(), 10).await.unwrap();
    let events = orch.session().events().len();
    drop(ex);
    let mut restarted = ControlExecutor::restore("s1", &io).await.unwrap();
    assert!(restarted.processed().ids.contains(&intent.id));
    assert_eq!(restarted.processed().last_seq, 1);
    let reports = restarted.drain(&io, &mut orch, &mut pipe, &fakes.transports(), 11).await.unwrap();
    assert_eq!(reports[0].outcome, ApplyOutcome::Duplicate);
    assert_eq!(orch.session().events().len(), events);
}

#[tokio::test]
#[rustfmt::skip]
async fn snapshot_contains_only_candidate_safe_fields() {
    let orch = started(); let pipe = loaded_pipe(&orch).await;
    let snap = build_snapshot(&orch, &pipe, SessionSyncPhase::Active, &["hint reply".into()], None);
    let json = serde_json::to_string(&snap).unwrap();
    assert!(json.contains("Hash map") && json.contains("Statement for e0") && json.contains("hint reply"));
    for hidden in ["424242", "111", "333", "444", "Best", "Workable", "Weak", "best_index", "expected_tokens", "quality", "locked"] {
        assert!(!json.contains(hidden), "leaked {hidden}: {json}");
    }
    assert_eq!(snap.options.len(), 4); assert_eq!(snap.current.as_ref().unwrap().slug, "e0");
}

#[test]
#[rustfmt::skip]
fn snapshot_sync_phase_labels_preserve_wire_format() {
    let cases = [
        (SessionSyncPhase::NotStarted, "Sync: not started"),
        (SessionSyncPhase::Active, "Sync: active"),
        (SessionSyncPhase::Paused, "Sync: paused"),
        (SessionSyncPhase::Conflict, "Sync: conflict needs attention \u{2014} ask the expert"),
    ];
    for (phase, label) in cases {
        let snap = build_snapshot(&started(), &QuestionPipeline::new(), phase, &[], None);
        assert_eq!(snap.v, SNAPSHOT_VERSION);
        assert_eq!(snap.sync_phase, label, "phase {phase:?}");
        let wire: serde_json::Value = serde_json::to_value(&snap).unwrap();
        assert_eq!(wire["sync_phase"], label);
        assert_eq!(wire["v"], 1);
    }
}

#[tokio::test]
#[rustfmt::skip]
async fn snapshot_published_atomically() {
    let (_root, io, _ex) = setup("atomic").await;
    let root = io.root().to_path_buf();
    let snap = build_snapshot(&started(), &QuestionPipeline::new(), SessionSyncPhase::Active, &[], None);
    publish_snapshot(&io, &snap).await.unwrap();
    let bytes = std::fs::read(root.join(SNAPSHOT_REL)).unwrap();
    assert_eq!(parse_snapshot(&bytes, "s1").unwrap(), snap);
    let names: Vec<String> = std::fs::read_dir(root.join("outbox")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, [SNAPSHOT_FILE.to_string()]);
    publish_snapshot(&io, &snap).await.unwrap();
    let names: Vec<String> = std::fs::read_dir(root.join("outbox")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, [SNAPSHOT_FILE.to_string()]);
}

#[tokio::test]
#[rustfmt::skip]
async fn snapshot_hidden_until_final_state() {
    let mut orch = started(); let pipe = QuestionPipeline::new();
    let breakdown = compute_final(&orch, &BTreeMap::new(), &[]).unwrap();
    let before = build_snapshot(&orch, &pipe, SessionSyncPhase::Active, &[], Some(&breakdown));
    assert_eq!(before.final_breakdown, None);
    orch.end_by_expert(50).unwrap();
    let after = build_snapshot(&orch, &pipe, SessionSyncPhase::Active, &[], Some(&breakdown));
    let final_snap = after.final_breakdown.expect("final visible after expert end");
    assert_eq!(final_snap.per_question.len(), 5);
    let mut failed = started(); failed.mark_technical_failure(60).unwrap();
    let hidden = build_snapshot(&failed, &pipe, SessionSyncPhase::Active, &[], Some(&breakdown));
    assert_eq!(hidden.final_breakdown, None);
}

#[tokio::test]
#[rustfmt::skip]
async fn chat_request_deduped_by_chat_id() {
    let (root, io, mut ex) = setup("chat").await;
    let mut orch = started(); let mut pipe = QuestionPipeline::new(); let fakes = Fakes::new();
    append(&root, &line(&Intent::new("s1", 1, IntentKind::ChatRequest { chat_id: "c1".into(), text: "hint?".into() })));
    let reports = ex.drain(&io, &mut orch, &mut pipe, &fakes.transports(), 10).await.unwrap();
    assert_eq!(reports[0].outcome, ApplyOutcome::Applied);
    assert_eq!(fakes.chat.calls.load(Ordering::SeqCst), 1);
    assert_eq!(ex.chat_replies(), ["Think about a map.".to_string()]);
    append(&root, &line(&Intent::new("s1", 2, IntentKind::ChatRequest { chat_id: "c1".into(), text: "hint again?".into() })));
    let retry = ex.drain(&io, &mut orch, &mut pipe, &fakes.transports(), 11).await.unwrap();
    assert!(retry.iter().any(|r| r.outcome == ApplyOutcome::Duplicate));
    assert_eq!(fakes.chat.calls.load(Ordering::SeqCst), 1);
    assert_eq!(ex.chat_replies().len(), 1);
}

#[tokio::test]
#[rustfmt::skip]
async fn executor_applies_through_core_validation() {
    let (root, io, mut ex) = setup("coreval").await;
    let mut orch = started(); let mut pipe = QuestionPipeline::new(); let fakes = Fakes::new();
    append(&root, &line(&Intent::new("s1", 1, IntentKind::SelectPreset { preset: 0 })));
    let reports = ex.drain(&io, &mut orch, &mut pipe, &fakes.transports(), 10).await.unwrap();
    assert_eq!(reports[0].outcome, ApplyOutcome::Rejected);
    assert_eq!(count(&orch, |k| matches!(k, EventKind::ApproachSelected { .. })), 0);
    let mut pipe = loaded_pipe(&orch).await;
    append(&root, &line(&Intent::new("s1", 2, IntentKind::SelectPreset { preset: 0 })));
    let applied = ex.drain(&io, &mut orch, &mut pipe, &fakes.transports(), 11).await.unwrap();
    assert!(applied.iter().any(|r| r.outcome == ApplyOutcome::Applied));
    assert_eq!(count(&orch, |k| matches!(k, EventKind::ApproachSelected { .. })), 1);
}

#[tokio::test]
#[rustfmt::skip]
async fn a_clean_run_keeps_the_question_open_until_submit() {
    let (root, io, mut ex) = setup("advance-clean").await;
    let mut orch = started();
    let mut pipe = loaded_pipe(&orch).await;
    let fakes = Fakes::new();
    let coding = ReadyCoding;
    let transports = ControlTransports { coding: &coding, sandbox: &fakes.sandbox, generator: &fakes.generator, chat: &fakes.chat };
    append(&root, &line(&Intent::new("s1", 1, IntentKind::SelectPreset { preset: 0 })));
    let reports = ex.drain(&io, &mut orch, &mut pipe, &transports, 10).await.unwrap();
    assert_eq!(reports[0].outcome, ApplyOutcome::Applied, "{reports:?}");
    assert_eq!(fakes.sandbox.test_calls.load(Ordering::SeqCst), 0, "selection must not auto-run tests");
    assert_eq!(count(&orch, |k| matches!(k, EventKind::TestsRequested { .. })), 0);
    assert_eq!(count(&orch, |k| matches!(k, EventKind::QuestionFinished { .. })), 0);
    assert_eq!(orch.current_index(), Some(0));
    append(&root, &line(&Intent::new("s1", 2, IntentKind::RequestTests)));
    let test_reports = ex.drain(&io, &mut orch, &mut pipe, &transports, 11).await.unwrap();
    assert_eq!(test_reports[1].outcome, ApplyOutcome::Applied, "{test_reports:?}");
    assert_eq!(fakes.sandbox.test_calls.load(Ordering::SeqCst), 1);
    assert_eq!(count(&orch, |k| matches!(k, EventKind::TestsRequested { .. })), 1);
    assert_eq!(count(&orch, |k| matches!(k, EventKind::QuestionFinished { .. })), 0);
    assert_eq!(orch.current_index(), Some(0));
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[rustfmt::skip]
async fn a_failed_run_leaves_the_solved_question_open() {
    let (root, io, mut ex) = setup("advance-fail").await;
    let mut orch = started();
    let mut pipe = loaded_pipe(&orch).await;
    let mut fakes = Fakes::new();
    fakes.sandbox.fail_tests = true;
    let coding = ReadyCoding;
    let transports = ControlTransports { coding: &coding, sandbox: &fakes.sandbox, generator: &fakes.generator, chat: &fakes.chat };
    append(&root, &line(&Intent::new("s1", 1, IntentKind::SelectPreset { preset: 0 })));
    append(&root, &line(&Intent::new("s1", 2, IntentKind::RequestTests)));
    let reports = ex.drain(&io, &mut orch, &mut pipe, &transports, 10).await.unwrap();
    assert_eq!(reports.iter().map(|r| r.outcome).collect::<Vec<_>>(), vec![ApplyOutcome::Applied, ApplyOutcome::Applied], "{reports:?}");
    assert_eq!(fakes.sandbox.test_calls.load(Ordering::SeqCst), 1);
    assert_eq!(count(&orch, |k| matches!(k, EventKind::QuestionFinished { .. })), 0);
    assert_eq!(orch.current_index(), Some(0));
    assert!(orch.session().current_slot().is_some_and(|s| s.approach.is_some()));
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[rustfmt::skip]
async fn a_clean_run_stays_on_the_question_even_without_ready() {
    let (root, io, mut ex) = setup("advance-draft").await;
    let mut orch = started();
    let mut pipe = loaded_pipe(&orch).await;
    let fakes = Fakes::new();
    append(&root, &line(&Intent::new("s1", 1, IntentKind::SelectPreset { preset: 0 })));
    append(&root, &line(&Intent::new("s1", 2, IntentKind::RunSolution)));
    let reports = ex.drain(&io, &mut orch, &mut pipe, &fakes.transports(), 10).await.unwrap();
    assert_eq!(reports.iter().map(|r| r.outcome).collect::<Vec<_>>(), vec![ApplyOutcome::Applied, ApplyOutcome::Applied], "{reports:?}");
    assert_eq!(fakes.sandbox.test_calls.load(Ordering::SeqCst), 1);
    assert_eq!(count(&orch, |k| matches!(k, EventKind::TestsRequested { .. })), 0);
    assert_eq!(count(&orch, |k| matches!(k, EventKind::QuestionFinished { .. })), 0);
    assert_eq!(orch.current_index(), Some(0));
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[rustfmt::skip]
async fn wrong_session_intent_is_rejected() {
    let intent = Intent::new("other-session", 1, IntentKind::Finish);
    assert_eq!(parse_intent_line(&intent.to_line().unwrap(), "s1"), Err(ControlError::WrongSession));
    let (root, io, mut ex) = setup("wrongsession").await;
    let mut orch = started(); let mut pipe = QuestionPipeline::new(); let fakes = Fakes::new();
    append(&root, &line(&intent));
    let reports = ex.drain(&io, &mut orch, &mut pipe, &fakes.transports(), 10).await.unwrap();
    assert!(reports.iter().all(|r| r.outcome == ApplyOutcome::Rejected));
    assert_eq!(count(&orch, |k| matches!(k, EventKind::QuestionFinished { .. })), 0);
    assert_eq!(orch.current_index(), Some(0));
}

#[tokio::test]
#[ignore = "live docker+ssh: provisions a real sandbox, requires ssh-keyscan on PATH"]
#[rustfmt::skip]
async fn live_control_channel_round_trip() {
    use pairproof::connection::{Endpoint, EphemeralUsername, Secret};
    use pairproof::sandbox::lifecycle::{create_and_provision, destroy, destroy_by_session_id};
    use pairproof::sandbox::preflight::port_is_free;
    let session_id = format!("ctrllive{}x{}", std::process::id(), 1);
    let port = (22500..22700u16).find(|p| port_is_free(*p)).expect("free port");
    let endpoint = Endpoint::new("127.0.0.1", port).expect("endpoint");
    let username = EphemeralUsername::generate(); let secret = Secret::generate();
    let (handle, fingerprint) = match create_and_provision(&session_id, &endpoint, &username, &secret).await {
        Ok(pair) => pair,
        Err(err) => { let _ = destroy_by_session_id(&session_id).await; panic!("create_and_provision failed: {err}"); }
    };
    let docker_io = DockerControlIo::new(handle.clone());
    let outcome: Result<(Vec<ApplyReport>, CandidateSnapshot), ControlError> = async {
        docker_io.setup_dirs(username.as_str()).await?;
        let key_line = verify_and_key_line(&endpoint, &fingerprint).map_err(|e| ControlError::Io(format!("{e:?}")))?;
        let mut client = SshControlClient::new(&session_id, endpoint.clone(), username.clone(), secret.clone(), key_line.clone());
        let mut orch = Orchestrator::create(session_id.clone(), Language::Python, qs("e", 5, Difficulty::Easy), 0).map_err(|e| ControlError::Io(e.to_string()))?;
        orch.mark_waiting_for_candidate(1).map_err(|e| ControlError::Io(e.to_string()))?;
        orch.attach_candidate(2).map_err(|e| ControlError::Io(e.to_string()))?;
        orch.start(3).map_err(|e| ControlError::Io(e.to_string()))?;
        let mut executor = ControlExecutor::restore(&session_id, &docker_io).await?;
        let mut pipe = QuestionPipeline::new(); let fakes = Fakes::new();
        client.send_next(IntentKind::Finish)?;
        let reports = executor.drain(&docker_io, &mut orch, &mut pipe, &fakes.transports(), 10).await?;
        let snap = build_snapshot(&orch, &pipe, SessionSyncPhase::Active, executor.chat_replies(), None);
        publish_snapshot(&docker_io, &snap).await?;
        let read_back = client.fetch_snapshot()?;
        Ok((reports, read_back))
    }.await;
    let key_line2 = verify_and_key_line(&endpoint, &fingerprint);
    let outbox_probe = key_line2.as_ref().map_err(|e| format!("{e:?}")).and_then(|line| {
        run_ssh_command_with_stdin(&endpoint, &username, &secret, line, &["tee", "/pairproof-control/outbox/snapshot.json"], Some(b"hacked"), std::time::Duration::from_secs(20)).map_err(|e| format!("{e:?}"))
    });
    let snapshot_after = key_line2.as_ref().map_err(|e| format!("{e:?}")).and_then(|line| {
        run_ssh_command(&endpoint, &username, &secret, line, &["cat", "/pairproof-control/outbox/snapshot.json"], std::time::Duration::from_secs(20)).map_err(|e| format!("{e:?}"))
    });
    let destroyed = destroy(&handle).await;
    assert!(destroyed.is_ok(), "destroy: {destroyed:?}");
    let snapshot_after = snapshot_after.expect("snapshot still readable after probe");
    assert!(snapshot_after.contains(&session_id) && !snapshot_after.contains("hacked"), "snapshot intact: {snapshot_after}");
    let (reports, read_back) = outcome.expect("live round trip");
    println!("live_control_channel_round_trip reports={reports:?} phase={} current={:?} outbox_probe={outbox_probe:?}", read_back.phase, read_back.current.as_ref().map(|c| c.index));
    assert!(reports.iter().any(|r| r.outcome == ApplyOutcome::Applied), "reports: {reports:?}");
    assert_eq!(read_back.session_id, session_id);
    assert_eq!(read_back.current.as_ref().map(|c| c.index), Some(1));
    assert!(outbox_probe.is_err(), "candidate outbox write must fail: {outbox_probe:?}");
}

#[tokio::test]
#[rustfmt::skip]
async fn disconnect_intent_disconnects_the_candidate_at_once() {
    let (root, io, mut ex) = setup("disconnect").await;
    let mut orch = started(); let mut pipe = loaded_pipe(&orch).await; let fakes = Fakes::new();
    assert!(orch.is_connected());
    append(&root, &line(&Intent::new("s1", 1, IntentKind::Disconnect)));
    let reports = ex.drain(&io, &mut orch, &mut pipe, &fakes.transports(), 10).await.unwrap();
    assert_eq!(reports.iter().map(|r| r.outcome).collect::<Vec<_>>(), vec![ApplyOutcome::Applied], "{reports:?}");
    assert!(!orch.is_connected(), "monitor must see the disconnect immediately");
    assert_eq!(count(&orch, |k| matches!(k, EventKind::CandidateDisconnected)), 1);
    // A second Disconnect (e.g. teardown racing the signal) is a no-op.
    append(&root, &line(&Intent::new("s1", 2, IntentKind::Disconnect)));
    let again = ex.drain(&io, &mut orch, &mut pipe, &fakes.transports(), 11).await.unwrap();
    assert_eq!(again.iter().map(|r| r.outcome).collect::<Vec<_>>(), vec![ApplyOutcome::Duplicate, ApplyOutcome::Applied], "{again:?}");
    assert_eq!(count(&orch, |k| matches!(k, EventKind::CandidateDisconnected)), 1);
    let _ = std::fs::remove_dir_all(root);
}
