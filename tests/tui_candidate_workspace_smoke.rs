//! End-to-end Candidate Question/Coach workspace smoke tests.
//! Public APIs only, with controlled fakes: no network, Docker,
//! SSH, or API key. Each test observes rendered text, recorded
//! intents, or captured snapshots.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use crossterm::event::{KeyCode as K, KeyEvent, KeyModifiers};
use pairproof::connection::{ConnectionString, Endpoint, EphemeralUsername, Fingerprint, Secret};
use pairproof::core::{Language, Session};
use pairproof::leetcode::Difficulty;
use pairproof::llm::chat::ChatState;
use pairproof::llm::coding::{CodingStepInput, CodingStepOutput};
use pairproof::llm::Message;
use pairproof::orchestration::control::{
    CandidateSnapshot, Intent, IntentKind, SnapshotOption, SnapshotQuestion,
};
use pairproof::orchestration::*;
use pairproof::sandbox::{RunnabilityReport, TestOutcome};
use pairproof::scoring::{ApproachGrade, ApproachQuality, ApproachSet};
use pairproof::tui::candidate_run::{CandidateFocus, CandidateRunState};
use pairproof::tui::candidate_runtime::{
    focus_for_pane, handle_live_key, pane_click_target, CandidateRuntime, CandidateRuntimeError,
    CandidateTransport, CandidateView, PaneSide,
};
use pairproof::tui::render::{draw_candidate_screen, FRAME_COLOR, PROOF_COLOR};

mod common_mod;
use common_mod::qs;

type PF<'a, T> = PipelineFuture<'a, T>;
const TITLES: [&str; 4] = ["Hash map", "Two pointers", "Sorting", "Binary search"];

// --- Candidate-side fake transport and fixtures ---

#[derive(Default)]
struct FakeTransport {
    sent: Vec<Intent>,
    snapshots: VecDeque<Option<CandidateSnapshot>>,
}

impl CandidateTransport for FakeTransport {
    fn authenticate(&mut self, _c: &ConnectionString) -> Result<(), CandidateRuntimeError> {
        Ok(())
    }
    fn discover_session_id(&mut self) -> Result<String, CandidateRuntimeError> {
        Ok("s1".to_string())
    }
    fn write_hello(&mut self, _s: &str) -> Result<(), CandidateRuntimeError> {
        Ok(())
    }
    fn start_sync(&mut self, _s: &str, _a: &Path) -> Result<(), CandidateRuntimeError> {
        Ok(())
    }
    fn send_intent(&mut self, intent: &Intent) -> Result<(), CandidateRuntimeError> {
        self.sent.push(intent.clone());
        Ok(())
    }
    fn read_snapshot(&mut self) -> Result<Option<CandidateSnapshot>, CandidateRuntimeError> {
        Ok(self.snapshots.pop_front().unwrap_or(None))
    }
    fn flush_sync(&mut self, _s: &str) -> Result<(), CandidateRuntimeError> {
        Ok(())
    }
    fn terminate_sync(&mut self, _s: &str) -> Result<(), CandidateRuntimeError> {
        Ok(())
    }
}

fn connection() -> ConnectionString {
    ConnectionString::new(
        Endpoint::new("secret-host.example.com", 2222).expect("endpoint"),
        EphemeralUsername::parse("pp-abcd1234").expect("username"),
        Secret::from_bytes([0xab; 16]),
        4_102_444_800_000,
        Fingerprint::parse("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
            .expect("fingerprint"),
    )
}

#[rustfmt::skip]
fn snapshot4() -> CandidateSnapshot {
    let summaries = ["one pass", "two ends", "sort first", "half split"];
    CandidateSnapshot { v: 1, session_id: "s1".to_string(), phase: "InProgress".to_string(), sync_phase: "Sync: active".to_string(),
        current: Some(SnapshotQuestion { index: 0, title: "Two Sum".to_string(), slug: "two-sum".to_string(), statement: Some("Find two numbers that sum to target.".to_string()) }),
        options: TITLES.iter().zip(summaries).map(|(t, s)| SnapshotOption { title: t.to_string(), summary: s.to_string() }).collect(),
        nav: Vec::new(),
        selected: None, latest_test: None, chat_replies: vec![], final_breakdown: None, chat_partial: None, recovery: None }
}

fn connected(name: &str, fake: FakeTransport) -> (PathBuf, CandidateRuntime<FakeTransport>) {
    let dir = std::env::temp_dir().join(format!("pairproof-smoke-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("workspace");
    let mut runtime = CandidateRuntime::new("s1", dir.clone(), fake);
    runtime.connect(&connection()).expect("connect");
    (dir, runtime)
}

fn key(code: K) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn press(
    state: &mut CandidateRunState,
    runtime: &mut CandidateRuntime<FakeTransport>,
    code: K,
    status: &mut Option<String>,
) {
    handle_live_key(state, runtime, key(code), status);
}

fn pane_lines(view: &CandidateView) -> (Vec<String>, Vec<String>) {
    let mut left = view.question_lines();
    left.extend([
        "other: ".to_string(),
        "redirect: ".to_string(),
        "focus: tab or click a panel to switch question/coach".to_string(),
    ]);
    let mut right = view.chat_lines();
    right.push("message: ".to_string());
    (left, right)
}

#[rustfmt::skip]
fn drawn(left: &[String], right: &[String], chat_focused: bool, w: u16, h: u16, qs: usize, cs: usize) -> ratatui::buffer::Buffer {
    let backend = ratatui::backend::TestBackend::new(w, h);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    terminal.draw(|f| draw_candidate_screen(f, left, right, chat_focused, 60, qs, cs)).expect("draw");
    terminal.backend().buffer().clone()
}

fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
    let area = *buffer.area();
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// --- Expert-side streaming fakes (as in orchestrator_chat_streaming) ---

#[rustfmt::skip]
fn started() -> Orchestrator {
    let mut o = Orchestrator::create("s1".into(), Language::Python, qs("e", 5, Difficulty::Easy), 0).expect("orch");
    o.mark_waiting_for_candidate(1).expect("wait"); o.attach_candidate(2).expect("attach"); o.start(3).expect("start"); o
}

struct StreamingChat {
    chunks: Vec<String>,
    calls: AtomicU32,
}
#[rustfmt::skip]
impl ChatTransport for StreamingChat {
    fn chat<'a>(&'a self, _s: &'a mut Session, _st: &'a ChatState, _h: &'a [Message], _m: &'a str, _at: u64) -> PF<'a, String> { Box::pin(async move { Ok("fallback".to_string()) }) }
    fn chat_streaming<'a>(&'a self, _s: &'a mut Session, _st: &'a ChatState, _h: &'a [Message], _m: &'a str, _at: u64, on_chunk: &'a mut (dyn FnMut(&str) + Send)) -> PF<'a, String> {
        Box::pin(async move { self.calls.fetch_add(1, Ordering::SeqCst); for chunk in &self.chunks { on_chunk(chunk); tokio::task::yield_now().await; } Ok(self.chunks.concat()) })
    }
}
struct FakeCoding;
#[rustfmt::skip]
impl CodingTransport for FakeCoding {
    fn coding_step<'a>(&'a self, _s: &'a mut Session, _i: &'a CodingStepInput) -> PF<'a, CodingStepOutput> { Box::pin(async move { Err(PipelineError::ChatUnavailable) }) }
}
struct FakeSandbox;
#[rustfmt::skip]
impl SandboxIo for FakeSandbox {
    fn write_solution<'a>(&'a self, _i: usize, _l: Language, _c: &'a str) -> PF<'a, ()> { Box::pin(async move { Ok(()) }) }
    fn read_solution<'a>(&'a self, _i: usize, _l: Language) -> PF<'a, String> { Box::pin(async move { Ok(String::new()) }) }
    fn check_runnable<'a>(&'a self, _i: usize, _l: Language) -> PF<'a, RunnabilityReport> { Box::pin(async move { Ok(RunnabilityReport { runnable: true, exit_code: Some(0), stdout: String::new(), stderr: String::new(), truncated: false }) }) }
    fn run_requested_tests<'a>(&'a self, _i: usize, _l: Language) -> PF<'a, TestOutcome> { Box::pin(async move { Ok(TestOutcome { success: true, exit_code: Some(0), stdout: "ok".into(), stderr: String::new(), stdout_truncated: false, stderr_truncated: false }) }) }
}
struct FakeGenerator;
#[rustfmt::skip]
impl ApproachGenerator for FakeGenerator {
    fn generate<'a>(&'a self, _s: &'a str, _l: Language) -> PF<'a, ApproachSet> { Box::pin(async move { Err(PipelineError::ChatUnavailable) }) }
    fn grade_other<'a>(&'a self, _s: &'a str, _t: &'a str) -> PF<'a, ApproachGrade> { Box::pin(async move { ApproachGrade::new(ApproachQuality::Workable, 424242, "hidden".into()).map_err(|e| PipelineError::Llm(e.to_string())) }) }
}

#[derive(Default)]
struct CapturingIo {
    intents: Mutex<Vec<u8>>,
    snapshots: Mutex<Vec<Vec<u8>>>,
    processed: Mutex<Option<ProcessedState>>,
}
#[rustfmt::skip]
impl CapturingIo {
    fn append(&self, intent: &Intent) { let mut line = CandidateControlClient::intent_line(intent).expect("line"); line.push('\n'); self.intents.lock().expect("intents").extend_from_slice(line.as_bytes()); }
    fn partials(&self) -> Vec<Option<String>> { self.snapshots().iter().map(|s| s.chat_partial.clone()).collect() }
    fn snapshots(&self) -> Vec<CandidateSnapshot> { self.snapshots.lock().expect("snaps").iter().map(|b| serde_json::from_slice(b).expect("snapshot")).collect() }
}
#[rustfmt::skip]
impl ControlIo for CapturingIo {
    fn setup_dirs<'a>(&'a self, _u: &'a str) -> ControlFuture<'a, ()> { Box::pin(async move { Ok(()) }) }
    fn read_intents<'a>(&'a self) -> ControlFuture<'a, Vec<u8>> { Box::pin(async move { Ok(self.intents.lock().expect("intents").clone()) }) }
    fn write_snapshot_atomic<'a>(&'a self, bytes: &'a [u8]) -> ControlFuture<'a, ()> { Box::pin(async move { self.snapshots.lock().expect("snaps").push(bytes.to_vec()); Ok(()) }) }
    fn load_processed<'a>(&'a self) -> ControlFuture<'a, Option<ProcessedState>> { Box::pin(async move { Ok(self.processed.lock().expect("processed").clone()) }) }
    fn store_processed<'a>(&'a self, state: &'a ProcessedState) -> ControlFuture<'a, ()> { Box::pin(async move { *self.processed.lock().expect("processed") = Some(state.clone()); Ok(()) }) }
}

#[test]
fn smoke_candidate_sees_approaches_and_selects_preset() {
    let mut fake = FakeTransport::default();
    fake.snapshots.push_back(Some(snapshot4()));
    let (dir, mut runtime) = connected("preset", fake);
    runtime.poll_snapshot().expect("poll").expect("snapshot");
    let view = runtime.view();
    let lines = view.question_lines().join("\n");
    for title in TITLES {
        assert!(lines.contains(title), "{lines}");
    }
    assert!(
        lines.contains("5: other - describe your own approach"),
        "{lines}"
    );
    let (left, right) = pane_lines(&view);
    let text = buffer_text(&drawn(&left, &right, false, 80, 24, 0, 0));
    for title in TITLES {
        assert!(text.contains(title), "{text}");
    }
    assert!(text.contains("other - describe your own"), "{text}");
    let mut state = CandidateRunState::new();
    let mut status = None;
    press(&mut state, &mut runtime, K::Char('1'), &mut status);
    assert!(runtime.transport().sent.is_empty(), "number key only marks");
    press(&mut state, &mut runtime, K::Enter, &mut status);
    let sent = &runtime.transport().sent;
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].kind, IntentKind::SelectPreset { preset: 0 });
    assert_eq!(status, None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn smoke_candidate_other_input_submits_custom_approach() {
    let (dir, mut runtime) = connected("other", FakeTransport::default());
    let mut state = CandidateRunState::new();
    state.focus = CandidateFocus::Work;
    let mut status = None;
    press(&mut state, &mut runtime, K::Char('a'), &mut status);
    press(&mut state, &mut runtime, K::Char('5'), &mut status);
    assert_eq!(state.focus, CandidateFocus::Other);
    for c in "preset 2 but with a twist".chars() {
        press(&mut state, &mut runtime, K::Char(c), &mut status);
    }
    assert_eq!(state.work.other_text(), "preset 2 but with a twist");
    press(&mut state, &mut runtime, K::Enter, &mut status);
    let sent = &runtime.transport().sent;
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert_eq!(sent[0].kind, IntentKind::ChangeApproach);
    assert_eq!(
        sent[1].kind,
        IntentKind::SelectOther {
            text: "preset 2 but with a twist".to_string()
        }
    );
    assert_eq!(state.work.other_text(), "", "other text cleared after send");
    assert_eq!(state.focus, CandidateFocus::Work);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn smoke_candidate_streams_coach_reply_and_keeps_scrolling() {
    let io = CapturingIo::default();
    let mut ex = ControlExecutor::restore("s1", &io).await.expect("executor");
    let mut orch = started();
    let mut pipe = QuestionPipeline::new();
    let (coding, sandbox, generator) = (FakeCoding, FakeSandbox, FakeGenerator);
    let chat = StreamingChat {
        chunks: vec!["Think step ".into(), "by step.".into()],
        calls: AtomicU32::new(0),
    };
    let transports = ControlTransports {
        coding: &coding,
        sandbox: &sandbox,
        generator: &generator,
        chat: &chat,
    };
    io.append(&Intent::new(
        "s1",
        1,
        IntentKind::ChatRequest {
            chat_id: "c1".into(),
            text: "hint?".into(),
        },
    ));
    ex.set_snapshot_base(build_snapshot(
        &orch,
        &pipe,
        SessionSyncPhase::Active,
        ex.chat_replies(),
        None,
    ));
    let reports = ex
        .drain(&io, &mut orch, &mut pipe, &transports, 10)
        .await
        .expect("drain");
    assert_eq!(reports[0].outcome, ApplyOutcome::Applied);
    assert!(
        io.partials().contains(&Some("Think step ".to_string())),
        "partials: {:?}",
        io.partials()
    );
    assert_eq!(ex.chat_replies(), ["Think step by step.".to_string()]);
    assert_eq!(ex.chat_partial(), None);
    let final_snap = build_snapshot(
        &orch,
        &pipe,
        SessionSyncPhase::Active,
        ex.chat_replies(),
        None,
    );
    let view = CandidateView::from_snapshot(Some(&final_snap));
    assert!(view.chat_lines().join("\n").contains("Think step by step."));
    // Coach pane scrolled to bottom shows the reply end, not the welcome.
    let mut right = vec![view.chat_lines()[0].clone()];
    right.extend((0..30).map(|i| format!("coach filler {i:02}")));
    right.push(view.chat_lines()[1].clone());
    right.push("message: ".to_string());
    let mut left = view.question_lines();
    left.extend((0..30).map(|i| format!("question filler {i:02}")));
    left.push("focus: tab or click a panel to switch question/coach".to_string());
    for question_scroll in [0usize, 5] {
        let text = buffer_text(&drawn(&left, &right, true, 80, 24, question_scroll, 999));
        assert!(
            text.contains("by step."),
            "qscroll={question_scroll}: {text}"
        );
        assert!(!text.contains("Talking with me"), "{text}");
    }
}

#[test]
fn smoke_candidate_navigates_by_keyboard_and_mouse_and_resizes() {
    let mut fake = FakeTransport::default();
    fake.snapshots.push_back(Some(snapshot4()));
    let (dir, mut runtime) = connected("nav", fake);
    runtime.poll_snapshot().expect("poll").expect("snapshot");
    let mut state = CandidateRunState::new();
    let mut status = None;
    for _ in 0..3 {
        press(&mut state, &mut runtime, K::Tab, &mut status);
    }
    assert_eq!(state.focus, CandidateFocus::Chat);
    press(&mut state, &mut runtime, K::Tab, &mut status);
    assert_eq!(state.focus, CandidateFocus::Work);
    assert_eq!(pane_click_target(10, 80, 60), PaneSide::Question);
    assert_eq!(pane_click_target(70, 80, 60), PaneSide::Coach);
    assert_eq!(pane_click_target(10, 80, 60), PaneSide::Question);
    assert_eq!(
        focus_for_pane(CandidateFocus::Chat, PaneSide::Question),
        CandidateFocus::Work
    );
    assert_eq!(
        focus_for_pane(CandidateFocus::Work, PaneSide::Coach),
        CandidateFocus::Chat
    );
    let view = runtime.view();
    let (left, right) = pane_lines(&view);
    // The question itself is what a short terminal shows first.
    let small = buffer_text(&drawn(&left, &right, false, 60, 12, 0, 0));
    assert!(small.contains("Two Sum"), "{small}");
    let roomy = buffer_text(&drawn(&left, &right, false, 80, 24, 0, 0));
    for title in TITLES {
        assert!(roomy.contains(title), "{roomy}");
    }
    assert!(roomy.contains("other"), "{roomy}");
    // Focus border follows chat_focused at the large size, without panic.
    let divider = 72u16; // 60% of 120
    let unfocused = drawn(&left, &right, false, 120, 40, 0, 0);
    assert_eq!(unfocused[(0, 0)].fg, PROOF_COLOR);
    assert_eq!(unfocused[(divider, 0)].fg, FRAME_COLOR);
    let focused = drawn(&left, &right, true, 120, 40, 0, 0);
    assert_eq!(focused[(0, 0)].fg, FRAME_COLOR);
    assert_eq!(focused[(divider, 0)].fg, PROOF_COLOR);
    let _ = std::fs::remove_dir_all(dir);
}
