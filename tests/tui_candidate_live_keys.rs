use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode as K, KeyEvent, KeyModifiers};
use pairproof::connection::{ConnectionString, Endpoint, EphemeralUsername, Fingerprint, Secret};
use pairproof::orchestration::control::{CandidateSnapshot, Intent, IntentKind};
use pairproof::tui::candidate::CandidateScreen;
use pairproof::tui::candidate_run::{CandidateFocus, CandidateRunState};
use pairproof::tui::candidate_runtime::{
    handle_live_key, CandidateRuntime, CandidateRuntimeError, CandidateTransport,
};

#[derive(Default)]
struct FakeTransport {
    sent: Vec<Intent>,
    fail_read: bool,
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
        if self.fail_read {
            Err(CandidateRuntimeError::Transport)
        } else {
            Ok(None)
        }
    }
    fn flush_sync(&mut self, _s: &str) -> Result<(), CandidateRuntimeError> {
        Ok(())
    }
    fn terminate_sync(&mut self, _s: &str) -> Result<(), CandidateRuntimeError> {
        Ok(())
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
    let dir: PathBuf =
        std::env::temp_dir().join(format!("pairproof-live-key-{}", std::process::id()));
    let mut runtime = CandidateRuntime::new("s1", dir, FakeTransport::default());
    runtime.connect(&conn).expect("connect");
    runtime
}

fn key(code: K) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn question_mark_opens_help_in_empty_chat_focus() {
    let mut state = CandidateRunState::new();
    state.focus = CandidateFocus::Chat;
    let mut runtime = connected_runtime();
    let mut status = None;
    handle_live_key(&mut state, &mut runtime, key(K::Char('?')), &mut status);
    assert!(state.show_help);
    assert_eq!(state.chat.input_text(), "");
}

#[test]
fn question_mark_types_in_nonempty_chat_focus() {
    let mut state = CandidateRunState::new();
    state.focus = CandidateFocus::Chat;
    state.chat.type_char('h');
    let mut runtime = connected_runtime();
    let mut status = None;
    handle_live_key(&mut state, &mut runtime, key(K::Char('?')), &mut status);
    assert_eq!(state.chat.input_text(), "h?");
    assert!(!state.show_help);
}

#[test]
fn question_mark_toggles_help_in_work_focus() {
    let mut state = CandidateRunState::new();
    state.focus = CandidateFocus::Work;
    let mut runtime = connected_runtime();
    let mut status = None;
    handle_live_key(&mut state, &mut runtime, key(K::Char('?')), &mut status);
    assert!(state.show_help);
}

#[test]
fn a_sends_change_approach_when_connected() {
    let mut state = CandidateRunState::new();
    state.focus = CandidateFocus::Work;
    let mut runtime = connected_runtime();
    let mut status = None;
    handle_live_key(&mut state, &mut runtime, key(K::Char('a')), &mut status);
    assert_eq!(runtime.transport().sent.len(), 1);
    assert_eq!(runtime.transport().sent[0].kind, IntentKind::ChangeApproach);
    assert_eq!(status, None);
}

#[test]
fn chat_enter_keeps_the_sent_message_and_clears_the_box() {
    let mut state = CandidateRunState::new();
    state.focus = CandidateFocus::Chat;
    state.chat.type_text("where do I start?");
    let mut runtime = connected_runtime();
    let mut status = None;
    handle_live_key(&mut state, &mut runtime, key(K::Enter), &mut status);
    assert!(state.chat.input_text().is_empty());
    assert_eq!(
        state.chat.sent_messages(None),
        vec!["where do I start?".to_string()]
    );
    assert_eq!(runtime.transport().sent.len(), 1);
}

#[test]
fn empty_continue_enter_goes_back_without_coding() {
    let mut state = CandidateRunState::new();
    let mut runtime = connected_runtime();
    let mut status = None;
    handle_live_key(&mut state, &mut runtime, key(K::Char('c')), &mut status);
    assert_eq!(state.focus, CandidateFocus::Redirect);
    handle_live_key(&mut state, &mut runtime, key(K::Enter), &mut status);
    assert_eq!(state.focus, CandidateFocus::Work);
    assert!(
        runtime.transport().sent.is_empty(),
        "empty enter must not start a coding step: {:?}",
        runtime.transport().sent
    );
    assert!(
        status
            .as_deref()
            .unwrap_or("")
            .contains("continue cancelled"),
        "{status:?}"
    );
}

#[test]
fn empty_chat_enter_sends_nothing() {
    let mut state = CandidateRunState::new();
    state.focus = CandidateFocus::Chat;
    let mut runtime = connected_runtime();
    let mut status = None;
    handle_live_key(&mut state, &mut runtime, key(K::Enter), &mut status);
    assert!(runtime.transport().sent.is_empty());
    assert!(
        status
            .as_ref()
            .is_some_and(|s| s.contains("type a message")),
        "{status:?}"
    );
}

#[test]
fn q_does_not_quit_live_or_offline() {
    let mut state = CandidateRunState::new();
    state.focus = CandidateFocus::Work;
    let mut runtime = connected_runtime();
    let mut status = None;
    handle_live_key(&mut state, &mut runtime, key(K::Char('q')), &mut status);
    assert_ne!(state.app.screen(), CandidateScreen::Exited);
    assert!(runtime.transport().sent.is_empty());
}

#[test]
fn shared_work_key_classifier_maps_both_paths() {
    use pairproof::tui::candidate_run::{next_focus, work_key_action, WorkKeyAction};
    assert_eq!(work_key_action('1'), Some(WorkKeyAction::SelectPreset(0)));
    assert_eq!(work_key_action('4'), Some(WorkKeyAction::SelectPreset(3)));
    assert_eq!(work_key_action('5'), Some(WorkKeyAction::OpenOther));
    assert_eq!(work_key_action('o'), Some(WorkKeyAction::OpenOther));
    assert_eq!(work_key_action('c'), Some(WorkKeyAction::Continue));
    assert_eq!(work_key_action('a'), Some(WorkKeyAction::ChangeApproach));
    assert_eq!(work_key_action('r'), Some(WorkKeyAction::RunSolution));
    assert_eq!(work_key_action('t'), Some(WorkKeyAction::RequestTests));
    assert_eq!(work_key_action('f'), Some(WorkKeyAction::Finish));
    for removed in ['d', 'u', 'e', 's', 'x'] {
        assert_eq!(work_key_action(removed), None, "alias {removed:?} removed");
    }
    assert_eq!(next_focus(CandidateFocus::Work), CandidateFocus::Chat);
    assert_eq!(next_focus(CandidateFocus::Other), CandidateFocus::Chat);
    assert_eq!(next_focus(CandidateFocus::Redirect), CandidateFocus::Chat);
    assert_eq!(next_focus(CandidateFocus::Chat), CandidateFocus::Work);
}

#[test]
fn enter_closes_when_the_expert_client_is_gone() {
    let mut state = CandidateRunState::new();
    state.focus = CandidateFocus::Work;
    let mut runtime = connected_runtime();
    runtime.transport_mut().fail_read = true;
    let _ = runtime.poll_snapshot();
    assert!(runtime.connection_lost());
    let mut status = None;
    handle_live_key(&mut state, &mut runtime, key(K::Enter), &mut status);
    assert_eq!(state.app.screen(), CandidateScreen::Exited);
    assert!(runtime.transport().sent.is_empty());
}

#[test]
fn iterative_question_workflow_and_submission_guards() {
    use pairproof::orchestration::control::{
        SnapshotNav, SnapshotOption, SnapshotQuestion, SnapshotRun, SnapshotSelection, SnapshotTest,
    };
    use pairproof::tui::candidate_stage::QuestionFlowState;

    #[derive(Default)]
    struct SyncAwareTransport {
        sent: Vec<Intent>,
        snapshot: Option<CandidateSnapshot>,
        tracks_workspace: bool,
        fail_flush: bool,
        flushes: usize,
    }
    impl CandidateTransport for SyncAwareTransport {
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
        fn sync_tracks_workspace(&self) -> bool {
            self.tracks_workspace
        }
        fn send_intent(&mut self, intent: &Intent) -> Result<(), CandidateRuntimeError> {
            self.sent.push(intent.clone());
            Ok(())
        }
        fn read_snapshot(&mut self) -> Result<Option<CandidateSnapshot>, CandidateRuntimeError> {
            Ok(self.snapshot.clone())
        }
        fn flush_sync(&mut self, _s: &str) -> Result<(), CandidateRuntimeError> {
            self.flushes += 1;
            if self.fail_flush {
                Err(CandidateRuntimeError::SyncFailed)
            } else {
                Ok(())
            }
        }
        fn terminate_sync(&mut self, _s: &str) -> Result<(), CandidateRuntimeError> {
            Ok(())
        }
    }

    let make_snap = |idx: usize,
                     selected: Option<SnapshotSelection>,
                     latest_test: Option<SnapshotTest>,
                     q0_state: &str| {
        CandidateSnapshot {
            v: 1,
            session_id: "s1".to_string(),
            phase: "InProgress".to_string(),
            sync_phase: "Sync: active".to_string(),
            current: Some(SnapshotQuestion {
                index: idx,
                title: format!("Question {idx}"),
                slug: format!("q{idx}"),
                statement: Some("Solve the problem.".to_string()),
            }),
            options: vec![
                SnapshotOption {
                    title: "Hash map".to_string(),
                    summary: "O(n)".to_string(),
                },
                SnapshotOption {
                    title: "Two pointers".to_string(),
                    summary: "O(n log n)".to_string(),
                },
            ],
            nav: vec![
                SnapshotNav {
                    index: 0,
                    title: "Question 0".to_string(),
                    state: q0_state.to_string(),
                },
                SnapshotNav {
                    index: 1,
                    title: "Question 1".to_string(),
                    state: if idx == 1 { "Active" } else { "Pending" }.to_string(),
                },
            ],
            selected,
            latest_test,
            chat_replies: Vec::new(),
            final_breakdown: None,
            chat_partial: None,
            recovery: None,
        }
    };

    let conn = ConnectionString::new(
        Endpoint::new("secret-host.example.com", 2222).expect("endpoint"),
        EphemeralUsername::parse("pp-abcd1234").expect("username"),
        Secret::from_bytes([0xab; 16]),
        4_102_444_800_000,
        Fingerprint::parse("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
            .expect("fingerprint"),
    );
    let dir = std::env::temp_dir().join(format!("pairproof-iter-workflow-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("q1")).expect("q1 dir");
    let mut runtime = CandidateRuntime::new(
        "s1",
        dir.clone(),
        SyncAwareTransport {
            snapshot: Some(make_snap(0, None, None, "Active")),
            tracks_workspace: true,
            ..Default::default()
        },
    );
    runtime.connect(&conn).expect("connect");
    runtime.poll_snapshot().expect("initial poll");

    let mut state = CandidateRunState::new();
    let mut status = None;

    // Before selecting an approach, `c`, `r`, `t`, and `f` explain that an approach is needed first.
    handle_live_key(&mut state, &mut runtime, key(K::Char('c')), &mut status);
    assert!(runtime.transport().sent.is_empty());
    assert!(status
        .as_deref()
        .unwrap_or("")
        .contains("select an approach"));
    handle_live_key(&mut state, &mut runtime, key(K::Char('r')), &mut status);
    assert!(runtime.transport().sent.is_empty());
    assert!(status
        .as_deref()
        .unwrap_or("")
        .contains("select an approach"));
    handle_live_key(&mut state, &mut runtime, key(K::Char('t')), &mut status);
    assert!(runtime.transport().sent.is_empty());
    assert!(status
        .as_deref()
        .unwrap_or("")
        .contains("select an approach"));
    handle_live_key(&mut state, &mut runtime, key(K::Char('f')), &mut status);
    assert!(runtime.transport().sent.is_empty());
    assert!(status
        .as_deref()
        .unwrap_or("")
        .contains("select an approach"));

    // 1-2. Select an approach with Space and confirm with Enter.
    handle_live_key(&mut state, &mut runtime, key(K::Char(' ')), &mut status);
    assert!(runtime.transport().sent.is_empty());
    handle_live_key(&mut state, &mut runtime, key(K::Enter), &mut status);
    assert_eq!(runtime.transport().sent.len(), 1);
    assert_eq!(
        runtime.transport().sent[0].kind,
        IntentKind::SelectPreset { preset: 0 }
    );
    assert!(matches!(
        state.staging.flow_for(0),
        QuestionFlowState::GeneratingInitialSolution { .. }
    ));

    // Pressing `f` or `c` while initial generation is in flight is blocked.
    handle_live_key(&mut state, &mut runtime, key(K::Char('f')), &mut status);
    assert_eq!(runtime.transport().sent.len(), 1);
    assert!(status
        .as_deref()
        .unwrap_or("")
        .contains("wait for the current operation"));
    handle_live_key(&mut state, &mut runtime, key(K::Char('c')), &mut status);
    assert_eq!(runtime.transport().sent.len(), 1);

    // Delayed Mutagen sync: snapshot reports selection, but file is not on disk yet.
    runtime.transport_mut().snapshot = Some(make_snap(
        0,
        Some(SnapshotSelection::Preset(0)),
        None,
        "Active",
    ));
    runtime.poll_snapshot().expect("poll before file arrives");
    handle_live_key(&mut state, &mut runtime, key(K::Down), &mut status);
    assert!(
        matches!(
            state.staging.flow_for(0),
            QuestionFlowState::SyncingWorkspace { .. }
        ),
        "must wait for real workspace file before marking SolutionReady: {:?}",
        state.staging.flow_for(0)
    );
    assert!(state.after_code.is_none());

    // File arrives in candidate workspace; next poll transitions to SolutionReady.
    std::fs::write(dir.join("q1/solution.py"), "def solve():\n    return 1\n").expect("write q1");
    runtime.poll_snapshot().expect("poll after file arrives");
    handle_live_key(&mut state, &mut runtime, key(K::Up), &mut status);
    assert_eq!(state.staging.flow_for(0), QuestionFlowState::SolutionReady);
    let ready = state.after_code.as_ref().expect("ready banner");
    assert!(ready.1.contains("code is in q1/"), "{}", ready.1);
    assert!(!ready.1.contains("c continue"), "{}", ready.1);
    assert!(!ready.1.contains("t write tests"), "{}", ready.1);

    // 5. Press `c` to continue with additional instructions ("handle empty input").
    handle_live_key(&mut state, &mut runtime, key(K::Char('c')), &mut status);
    assert_eq!(state.focus, CandidateFocus::Redirect);
    for ch in "handle empty input".chars() {
        handle_live_key(&mut state, &mut runtime, key(K::Char(ch)), &mut status);
    }
    handle_live_key(&mut state, &mut runtime, key(K::Enter), &mut status);
    assert_eq!(state.focus, CandidateFocus::Work);
    assert_eq!(
        runtime.transport().sent.last().map(|i| &i.kind),
        Some(&IntentKind::Redirect {
            text: "handle empty input".to_string()
        })
    );
    assert!(matches!(
        state.staging.flow_for(0),
        QuestionFlowState::UpdatingSolution { .. }
    ));

    // Simulate agent update completing with code_revision = 2 and updated file on disk.
    std::fs::write(
        dir.join("q1/solution.py"),
        "def solve(xs):\n    return 0 if not xs else 1\n",
    )
    .expect("update q1");
    let snap = make_snap(0, Some(SnapshotSelection::Preset(0)), None, "Active");
    let mut wire = serde_json::to_value(&snap).unwrap();
    wire["latest_run"] = serde_json::to_value(SnapshotRun {
        kind: "code".to_string(),
        seq: 2,
        code_revision: 2,
        success: true,
        exit_code: Some(0),
        stdout: String::new(),
        stderr: String::new(),
        at_ms: 20,
    })
    .unwrap();
    let parsed =
        pairproof::orchestration::parse_snapshot(&serde_json::to_vec(&wire).unwrap(), "s1")
            .unwrap();
    runtime.transport_mut().snapshot = Some(parsed);
    runtime.poll_snapshot().expect("poll rev 2");
    handle_live_key(&mut state, &mut runtime, key(K::Up), &mut status);
    assert_eq!(state.staging.flow_for(0), QuestionFlowState::SolutionReady);

    // 7. Request test generation through `c` ("add unit tests").
    handle_live_key(&mut state, &mut runtime, key(K::Char('c')), &mut status);
    for ch in "add unit tests".chars() {
        handle_live_key(&mut state, &mut runtime, key(K::Char(ch)), &mut status);
    }
    handle_live_key(&mut state, &mut runtime, key(K::Enter), &mut status);
    assert!(matches!(
        state.staging.flow_for(0),
        QuestionFlowState::UpdatingTests { .. }
    ));
    wire["latest_run"] = serde_json::to_value(SnapshotRun {
        kind: "code".to_string(),
        seq: 3,
        code_revision: 3,
        success: true,
        exit_code: Some(0),
        stdout: String::new(),
        stderr: String::new(),
        at_ms: 30,
    })
    .unwrap();
    let parsed =
        pairproof::orchestration::parse_snapshot(&serde_json::to_vec(&wire).unwrap(), "s1")
            .unwrap();
    runtime.transport_mut().snapshot = Some(parsed);
    runtime.poll_snapshot().expect("poll rev 3");
    handle_live_key(&mut state, &mut runtime, key(K::Up), &mut status);
    assert_eq!(state.staging.flow_for(0), QuestionFlowState::SolutionReady);

    // 8. Write tests (`t`); extra score, does not execute yet.
    handle_live_key(&mut state, &mut runtime, key(K::Char('t')), &mut status);
    assert_eq!(
        runtime.transport().sent.last().map(|i| &i.kind),
        Some(&IntentKind::RequestTests)
    );
    assert!(matches!(
        state.staging.flow_for(0),
        QuestionFlowState::UpdatingTests { .. }
    ));
    wire["latest_run"] = serde_json::to_value(SnapshotRun {
        kind: "code".to_string(),
        seq: 4,
        code_revision: 4,
        success: true,
        exit_code: Some(0),
        stdout: String::new(),
        stderr: String::new(),
        at_ms: 40,
    })
    .unwrap();
    let parsed =
        pairproof::orchestration::parse_snapshot(&serde_json::to_vec(&wire).unwrap(), "s1")
            .unwrap();
    runtime.transport_mut().snapshot = Some(parsed);
    runtime.poll_snapshot().expect("poll tests written");
    handle_live_key(&mut state, &mut runtime, key(K::Up), &mut status);
    assert_eq!(state.staging.flow_for(0), QuestionFlowState::SolutionReady);

    // 9. Run those tests against the solution (`r`).
    handle_live_key(&mut state, &mut runtime, key(K::Char('r')), &mut status);
    assert_eq!(
        runtime.transport().sent.last().map(|i| &i.kind),
        Some(&IntentKind::RunSolution)
    );
    let snap_tested = make_snap(
        0,
        Some(SnapshotSelection::Preset(0)),
        Some(SnapshotTest {
            passed: 1,
            failed: 0,
            at_ms: 50,
        }),
        "Active",
    );
    let mut wire = serde_json::to_value(&snap_tested).unwrap();
    wire["latest_run"] = serde_json::to_value(SnapshotRun {
        kind: "test".to_string(),
        seq: 5,
        code_revision: 4,
        success: true,
        exit_code: Some(0),
        stdout: "TESTS: 1 passed, 0 failed".to_string(),
        stderr: String::new(),
        at_ms: 50,
    })
    .unwrap();
    let parsed =
        pairproof::orchestration::parse_snapshot(&serde_json::to_vec(&wire).unwrap(), "s1")
            .unwrap();
    runtime.transport_mut().snapshot = Some(parsed);
    runtime.poll_snapshot().expect("poll test");
    handle_live_key(&mut state, &mut runtime, key(K::Up), &mut status);
    assert_eq!(state.staging.flow_for(0), QuestionFlowState::SolutionReady);

    // Failed sync before `f` prevents marking the question as submitted.
    runtime.transport_mut().fail_flush = true;
    let sent_before = runtime.transport().sent.len();
    handle_live_key(&mut state, &mut runtime, key(K::Char('f')), &mut status);
    assert_eq!(runtime.transport().sent.len(), sent_before);
    assert_eq!(state.staging.flow_for(0), QuestionFlowState::SolutionReady);
    assert!(status.as_deref().unwrap_or("").contains("sync problem"));

    // 14-17. Successful `f` submits once, blocks duplicate `f`, advances to Q2, and locks Q1.
    runtime.transport_mut().fail_flush = false;
    handle_live_key(&mut state, &mut runtime, key(K::Char('f')), &mut status);
    assert_eq!(
        runtime.transport().sent.last().map(|i| &i.kind),
        Some(&IntentKind::Finish)
    );
    assert_eq!(state.staging.flow_for(0), QuestionFlowState::Submitting);
    let finish_count = runtime
        .transport()
        .sent
        .iter()
        .filter(|i| i.kind == IntentKind::Finish)
        .count();
    assert_eq!(finish_count, 1);

    // Rapid second `f` press before snapshot transition is blocked!
    handle_live_key(&mut state, &mut runtime, key(K::Char('f')), &mut status);
    let finish_count_after = runtime
        .transport()
        .sent
        .iter()
        .filter(|i| i.kind == IntentKind::Finish)
        .count();
    assert_eq!(finish_count_after, 1, "must not submit twice on rapid f");

    // Expert transitions to Q2 (index 1), with Q1 marked Finished.
    runtime.transport_mut().snapshot = Some(make_snap(1, None, None, "Finished"));
    runtime.poll_snapshot().expect("poll q2");
    handle_live_key(&mut state, &mut runtime, key(K::Left), &mut status);
    assert_eq!(state.staging.flow_for(0), QuestionFlowState::Submitted);
    assert!(
        status
            .as_deref()
            .unwrap_or("")
            .contains("submitted questions cannot be reopened"),
        "status: {status:?}"
    );
    assert!(
        !runtime
            .transport()
            .sent
            .iter()
            .any(|i| i.kind == IntentKind::Navigate { index: 0 }),
        "must not send Navigate to a finished question"
    );

    let _ = std::fs::remove_dir_all(dir);
}
