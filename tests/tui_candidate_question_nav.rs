use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode as K, KeyEvent, KeyModifiers};
use pairproof::connection::{ConnectionString, Endpoint, EphemeralUsername, Fingerprint, Secret};
use pairproof::core::{ApproachChoice, Language, QuestionRef, QuestionStatus, Session};
use pairproof::leetcode::Difficulty;
use pairproof::orchestration::control::{
    parse_intent_line, CandidateSnapshot, IntentKind, SnapshotNav, SnapshotOption,
    SnapshotQuestion, SnapshotSelection,
};
use pairproof::tui::candidate_run::{CandidateFocus, CandidateRunState};
use pairproof::tui::candidate_runtime::{
    handle_live_key, CandidateRuntime, CandidateRuntimeError, CandidateTransport,
};
use pairproof::tui::render::draw_candidate_screen_with_status;

fn q(slug: &str) -> QuestionRef {
    QuestionRef {
        title_slug: slug.into(),
        title: format!("Title {slug}"),
        difficulty: Difficulty::Easy,
        acceptance_pct: None,
    }
}

fn started() -> Session {
    let easy: Vec<QuestionRef> = (0..5).map(|i| q(&format!("e{i}"))).collect();
    let mut session = Session::new("s1".into(), Language::Python, easy, 0).expect("session");
    session.mark_waiting_for_candidate(1).expect("waiting");
    session.candidate_connected(2).expect("connected");
    session.start(3).expect("start");
    session
}

#[test]
fn navigation_reopens_finished_question_and_keeps_its_approach() {
    let mut session = started();
    session
        .select_approach(ApproachChoice::Preset(1), 100, 5)
        .expect("select");
    session.finish_question(6).expect("finish");
    assert_eq!(session.current_index(), Some(1));
    session.navigate_to(0, 7).expect("reopen finished");
    assert_eq!(session.current_index(), Some(0));
    let slot = session.slot(0).expect("slot 0");
    assert_eq!(slot.status, QuestionStatus::Active);
    assert_eq!(slot.approach, Some(ApproachChoice::Preset(1)));
    assert_eq!(slot.finished_at_ms, None);
    // The question left behind keeps its place and can be resumed.
    session.navigate_to(1, 8).expect("back to second");
    assert_eq!(session.current_index(), Some(1));
    assert_eq!(
        session.slot(0).expect("slot 0").status,
        QuestionStatus::Pending
    );
    // A never-started question can be opened directly too.
    session.navigate_to(3, 9).expect("open pending");
    assert_eq!(session.current_index(), Some(3));
}

#[test]
fn navigation_out_of_completed_easy_block_reopens_a_question() {
    let mut session = started();
    for i in 0..5u64 {
        session.finish_question(10 + i).expect("finish");
    }
    assert_eq!(session.current_index(), None);
    session.navigate_to(2, 30).expect("reopen");
    assert_eq!(session.current_index(), Some(2));
    assert_eq!(
        session.slot(2).expect("slot").status,
        QuestionStatus::Active
    );
}

#[test]
fn blank_other_is_a_valid_submission() {
    let mut session = started();
    session
        .select_approach(ApproachChoice::Other(String::new()), 100, 5)
        .expect("blank other selects");
    assert_eq!(
        session.current_slot().expect("slot").approach,
        Some(ApproachChoice::Other(String::new()))
    );
}

#[test]
fn navigate_intent_round_trips_the_wire_codec() {
    let intent =
        pairproof::orchestration::control::Intent::new("s1", 3, IntentKind::Navigate { index: 4 });
    let line = intent.to_line().expect("serialize");
    let back = parse_intent_line(&line, "s1").expect("parse");
    assert_eq!(back.kind, IntentKind::Navigate { index: 4 });
    // A blank Other intent also validates now.
    let blank = pairproof::orchestration::control::Intent::new(
        "s1",
        4,
        IntentKind::SelectOther {
            text: String::new(),
        },
    );
    assert!(blank.validate("s1").is_ok());
}

#[derive(Default)]
struct FakeTransport {
    sent: Vec<pairproof::orchestration::control::Intent>,
    snapshot: Option<CandidateSnapshot>,
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
    fn send_intent(
        &mut self,
        intent: &pairproof::orchestration::control::Intent,
    ) -> Result<(), CandidateRuntimeError> {
        self.sent.push(intent.clone());
        Ok(())
    }
    fn read_snapshot(&mut self) -> Result<Option<CandidateSnapshot>, CandidateRuntimeError> {
        Ok(self.snapshot.clone())
    }
    fn flush_sync(&mut self, _s: &str) -> Result<(), CandidateRuntimeError> {
        Ok(())
    }
    fn terminate_sync(&mut self, _s: &str) -> Result<(), CandidateRuntimeError> {
        Ok(())
    }
}

fn snapshot(index: usize) -> CandidateSnapshot {
    let statuses = ["Active", "Pending", "Pending"];
    CandidateSnapshot {
        v: 1,
        session_id: "s1".to_string(),
        phase: "InProgress".to_string(),
        sync_phase: "Sync: active".to_string(),
        current: Some(SnapshotQuestion {
            index,
            title: format!("Question {index}"),
            slug: format!("q{index}"),
            statement: Some("Statement body.".to_string()),
        }),
        options: (1..=4)
            .map(|n| SnapshotOption {
                title: format!("Plan {n}"),
                summary: format!("summary {n}"),
            })
            .collect(),
        nav: (0..3)
            .map(|i| SnapshotNav {
                index: i,
                title: format!("Question {i}"),
                state: if i == index { "Active" } else { statuses[i] }.to_string(),
            })
            .collect(),
        selected: None,
        latest_test: None,
        chat_replies: Vec::new(),
        final_breakdown: None,
        chat_partial: None,
        recovery: None,
    }
}

fn connected_runtime() -> CandidateRuntime<FakeTransport> {
    let conn = ConnectionString::new(
        Endpoint::new("secret-host.example.com", 2222).expect("endpoint"),
        EphemeralUsername::parse("pp-abcd1234").expect("username"),
        Secret::from_bytes([0xab; 16]),
        4_102_444_800_000,
        Fingerprint::parse("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
            .expect("fingerprint"),
    );
    let dir: PathBuf = std::env::temp_dir().join(format!("pairproof-qnav-{}", std::process::id()));
    let mut runtime = CandidateRuntime::new(
        "s1",
        dir,
        FakeTransport {
            snapshot: Some(snapshot(0)),
            ..FakeTransport::default()
        },
    );
    runtime.connect(&conn).expect("connect");
    runtime.poll_snapshot().expect("poll").expect("snapshot");
    runtime
}

fn key(code: K) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn press(state: &mut CandidateRunState, runtime: &mut CandidateRuntime<FakeTransport>, code: K) {
    let mut status = None;
    handle_live_key(state, runtime, key(code), &mut status);
}

#[test]
fn enter_submits_the_highlighted_preset_without_space() {
    let mut runtime = connected_runtime();
    let mut state = CandidateRunState::new();
    state.focus = CandidateFocus::Work;
    press(&mut state, &mut runtime, K::Down);
    assert!(
        runtime.transport().sent.is_empty(),
        "moving the highlight sends nothing"
    );
    press(&mut state, &mut runtime, K::Enter);
    assert_eq!(
        runtime.transport().sent.last().map(|i| i.kind.clone()),
        Some(IntentKind::SelectPreset { preset: 1 })
    );
    let pending = state.pending_approach.expect("status shows the submit");
    assert_eq!(pending.index, 0);
    assert_eq!(pending.selection, SnapshotSelection::Preset(1));
}

#[test]
fn space_marks_and_enter_submits_the_preset() {
    let mut runtime = connected_runtime();
    let mut state = CandidateRunState::new();
    state.focus = CandidateFocus::Work;
    press(&mut state, &mut runtime, K::Down);
    press(&mut state, &mut runtime, K::Char(' '));
    assert!(
        runtime.transport().sent.is_empty(),
        "space only marks, nothing is sent"
    );
    press(&mut state, &mut runtime, K::Enter);
    let sent = &runtime.transport().sent;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].kind, IntentKind::SelectPreset { preset: 1 });
}

#[test]
fn arrow_navigation_keeps_the_marked_choice_per_question() {
    let mut runtime = connected_runtime();
    let mut state = CandidateRunState::new();
    state.focus = CandidateFocus::Work;
    press(&mut state, &mut runtime, K::Down);
    press(&mut state, &mut runtime, K::Char(' '));
    press(&mut state, &mut runtime, K::Right);
    assert_eq!(
        runtime.transport().sent.last().map(|i| i.kind.clone()),
        Some(IntentKind::Navigate { index: 1 })
    );
    // The expert moves us to Q2; the candidate stages Q2 fresh...
    runtime.transport_mut().snapshot = Some(snapshot(1));
    runtime.poll_snapshot().expect("poll");
    press(&mut state, &mut runtime, K::Left);
    assert_eq!(
        runtime.transport().sent.last().map(|i| i.kind.clone()),
        Some(IntentKind::Navigate { index: 0 })
    );
    // ...and coming back, the mark on Q1 is still there: Enter
    // submits the same preset without re-marking.
    runtime.transport_mut().snapshot = Some(snapshot(0));
    runtime.poll_snapshot().expect("poll");
    press(&mut state, &mut runtime, K::Enter);
    assert_eq!(
        runtime.transport().sent.last().map(|i| i.kind.clone()),
        Some(IntentKind::SelectPreset { preset: 1 })
    );
    let _ = SnapshotSelection::Preset(0);
}

#[test]
fn typing_on_the_other_row_fills_other_and_enter_submits() {
    let mut runtime = connected_runtime();
    let mut state = CandidateRunState::new();
    state.focus = CandidateFocus::Work;
    for _ in 0..4 {
        press(&mut state, &mut runtime, K::Down);
    }
    for c in ['d', 'o'] {
        press(&mut state, &mut runtime, K::Char(c));
    }
    assert_eq!(state.focus, CandidateFocus::Other);
    assert_eq!(state.work.other_text(), "do");
    assert_eq!(state.work.redirect_text(), "");
    press(&mut state, &mut runtime, K::Enter);
    assert_eq!(
        runtime.transport().sent.last().map(|i| i.kind.clone()),
        Some(IntentKind::SelectOther {
            text: "do".to_string()
        })
    );
}

#[test]
fn blank_other_submits_from_the_other_row() {
    let mut runtime = connected_runtime();
    let mut state = CandidateRunState::new();
    state.focus = CandidateFocus::Work;
    for _ in 0..4 {
        press(&mut state, &mut runtime, K::Down);
    }
    press(&mut state, &mut runtime, K::Char(' '));
    press(&mut state, &mut runtime, K::Enter);
    assert_eq!(
        runtime.transport().sent.last().map(|i| i.kind.clone()),
        Some(IntentKind::SelectOther {
            text: String::new()
        })
    );
}

#[test]
fn status_box_is_fixed_size_above_the_coach() {
    let left = vec!["Q1 of 3: Question 0".to_string(), "focus: keys".to_string()];
    let status = vec![
        "selected approach: none".to_string(),
        "latest test: no test run yet".to_string(),
        "phase: InProgress".to_string(),
        "Sync: active".to_string(),
    ];
    let right = vec!["> coach hello".to_string(), "message: ".to_string()];
    let coach_top = |height: u16| {
        let backend = ratatui::backend::TestBackend::new(80, height);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| {
                draw_candidate_screen_with_status(frame, &left, &status, &right, false, 60, 0, 0)
            })
            .expect("draw");
        let buffer = terminal.backend().buffer().clone();
        let area = *buffer.area();
        let text: Vec<String> = (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect();
        assert!(text.iter().any(|row| row.contains("Status")), "{text:?}");
        assert!(
            text.iter().any(|row| row.contains("selected approach:")),
            "{text:?}"
        );
        text.iter()
            .position(|row| row.contains("Coach"))
            .expect("coach title row")
    };
    // The status box never grows or shrinks with the terminal: the
    // Coach pane starts on the same row at both heights.
    assert_eq!(coach_top(20), 6);
    assert_eq!(coach_top(34), 6);
}

#[test]
fn runtime_disconnect_sends_the_disconnect_intent() {
    let mut runtime = connected_runtime();
    runtime.disconnect().expect("disconnect");
    assert_eq!(
        runtime.transport().sent.last().map(|i| i.kind.clone()),
        Some(IntentKind::Disconnect)
    );
    let intent = pairproof::orchestration::control::Intent::new("s1", 9, IntentKind::Disconnect);
    let line = intent.to_line().expect("serialize");
    assert_eq!(
        parse_intent_line(&line, "s1").expect("parse").kind,
        IntentKind::Disconnect
    );
}

#[test]
fn final_screen_shows_only_candidate_safe_results() {
    use pairproof::orchestration::control::{SnapshotFinal, SnapshotFinalQuestion};
    use pairproof::tui::candidate_runtime::CandidateView;
    let view = CandidateView {
        phase: "EndedByExpert".to_string(),
        sync_phase: "Sync: active".to_string(),
        question_title: Some("Two Sum".to_string()),
        question_index: Some(0),
        statement: Some("Find two numbers.".to_string()),
        options: Vec::new(),
        nav: vec![
            SnapshotNav {
                index: 0,
                title: "Two Sum".to_string(),
                state: "Finished".to_string(),
            },
            SnapshotNav {
                index: 1,
                title: "Add Two".to_string(),
                state: "Skipped".to_string(),
            },
        ],
        selected: None,
        latest_test: None,
        chat_replies: Vec::new(),
        chat_partial: None,
        final_breakdown: Some(SnapshotFinal {
            rating: 7.5,
            per_question: vec![
                SnapshotFinalQuestion {
                    slot_index: 0,
                    title: "Two Sum".to_string(),
                    state: "Finished".to_string(),
                    grade: Some(8.0),
                    rationale: Some("Solid.".to_string()),
                },
                SnapshotFinalQuestion {
                    slot_index: 1,
                    title: "Add Two".to_string(),
                    state: "Skipped".to_string(),
                    grade: None,
                    rationale: None,
                },
            ],
        }),
        recovery: None,
    };
    let lines = view.final_screen_lines();
    let joined = lines.join("\n");
    assert!(joined.contains("final rating: 7.50 / 10"), "{joined}");
    assert!(joined.contains("solved: 1"), "{joined}");
    assert!(joined.contains("finished: 1"), "{joined}");
    assert!(joined.contains("skipped: 1"), "{joined}");
    assert!(joined.contains("Q1  Two Sum"), "{joined}");
    assert!(joined.contains("8.0 - solved"), "{joined}");
    assert!(joined.contains("Q2  Add Two"), "{joined}");
    assert!(joined.contains("skipped"), "{joined}");
    assert_eq!(lines.last().map(String::as_str), Some("enter to exit"));
    assert!(
        !joined.contains("Find two numbers."),
        "no question text: {joined}"
    );
}
