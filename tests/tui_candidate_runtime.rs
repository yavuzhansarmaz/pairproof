use std::collections::VecDeque;
use std::path::PathBuf;

use pairproof::connection::{ConnectionString, Endpoint, EphemeralUsername, Fingerprint, Secret};
use pairproof::orchestration::control::{
    CandidateSnapshot, Intent, IntentKind, SnapshotFinal, SnapshotFinalQuestion, SnapshotOption,
    SnapshotQuestion, SnapshotTest,
};
use pairproof::orchestration::ssh_auth::SshAuthError;
use pairproof::tui::candidate::{candidate_sync_message, CandidateConnectError};
use pairproof::tui::candidate_runtime::{
    candidate_connect_error_for_ssh, CandidateRuntime, CandidateRuntimeError,
    CandidateRuntimePhase, CandidateTransport,
};

const HOST: &str = "secret-host.example.com";
const USERNAME: &str = "pp-abcd1234";
const FINGERPRINT: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[derive(Default)]
#[rustfmt::skip]
struct FakeTransport {
    ops: Vec<String>,
    auth_err: Option<CandidateRuntimeError>,
    discover: Option<Result<String, CandidateRuntimeError>>,
    sync_fails: bool,
    snapshots: VecDeque<Option<CandidateSnapshot>>,
    sent: Vec<Intent>,
    terminated: bool, acks: Vec<String>,
    read_err: Option<CandidateRuntimeError>,
    expert_version: Option<String>,
}

impl CandidateTransport for FakeTransport {
    fn authenticate(
        &mut self,
        _connection: &ConnectionString,
    ) -> Result<(), CandidateRuntimeError> {
        self.ops.push("authenticate".to_string());
        match &self.auth_err {
            Some(err) => Err(err.clone()),
            None => Ok(()),
        }
    }
    fn discover_session_id(&mut self) -> Result<String, CandidateRuntimeError> {
        self.ops.push("discover_session".to_string());
        self.discover
            .clone()
            .unwrap_or_else(|| Ok("s1".to_string()))
    }
    fn expert_version(&self) -> Option<String> {
        self.expert_version.clone()
    }
    fn write_hello(&mut self, _session_id: &str) -> Result<(), CandidateRuntimeError> {
        self.ops.push("write_hello".to_string());
        Ok(())
    }
    fn start_sync(
        &mut self,
        _session_id: &str,
        _alpha: &std::path::Path,
    ) -> Result<(), CandidateRuntimeError> {
        self.ops.push("start_sync".to_string());
        if self.sync_fails {
            Err(CandidateRuntimeError::SyncFailed)
        } else {
            Ok(())
        }
    }
    fn send_intent(&mut self, intent: &Intent) -> Result<(), CandidateRuntimeError> {
        self.ops.push("send_intent".to_string());
        self.sent.push(intent.clone());
        Ok(())
    }
    fn read_snapshot(&mut self) -> Result<Option<CandidateSnapshot>, CandidateRuntimeError> {
        self.ops.push("read_snapshot".to_string());
        if let Some(err) = &self.read_err {
            return Err(err.clone());
        }
        Ok(self.snapshots.pop_front().unwrap_or(None))
    }
    #[rustfmt::skip]
    fn write_heartbeat(&mut self, _session_id: &str, _at_ms: u64) -> Result<(), CandidateRuntimeError> { self.ops.push("write_heartbeat".to_string()); Ok(()) }
    #[rustfmt::skip]
    fn write_final_ack(&mut self, session_id: &str) -> Result<(), CandidateRuntimeError> { self.ops.push("write_final_ack".to_string()); self.acks.push(session_id.to_string()); Ok(()) }
    fn flush_sync(&mut self, _session_id: &str) -> Result<(), CandidateRuntimeError> {
        self.ops.push("flush_sync".to_string());
        Ok(())
    }
    fn terminate_sync(&mut self, _session_id: &str) -> Result<(), CandidateRuntimeError> {
        self.ops.push("terminate_sync".to_string());
        self.terminated = true;
        Ok(())
    }
}

fn connection() -> (ConnectionString, String) {
    let secret = Secret::from_bytes([0xab; 16]);
    let secret_hex = secret.expose_hex();
    let value = ConnectionString::new(
        Endpoint::new(HOST, 2222).expect("endpoint"),
        EphemeralUsername::parse(USERNAME).expect("username"),
        secret,
        4_102_444_800_000,
        Fingerprint::parse(FINGERPRINT).expect("fingerprint"),
    );
    (value, secret_hex)
}

#[rustfmt::skip]
fn snapshot(session_id: &str, phase: &str, final_breakdown: Option<SnapshotFinal>) -> CandidateSnapshot {
    CandidateSnapshot { v: 1, session_id: session_id.to_string(), phase: phase.to_string(), sync_phase: "Sync: active".to_string(),
        current: Some(SnapshotQuestion { index: 0, title: "Two Sum".to_string(), slug: "two-sum".to_string(), statement: Some("Find two numbers that sum to target.".to_string()) }),
        options: vec![SnapshotOption { title: "Hash map".to_string(), summary: "One pass lookup".to_string() }], selected: Some("preset 0".to_string()),
        latest_test: Some(SnapshotTest { passed: 3, failed: 1, at_ms: 10 }), chat_replies: vec!["Try a hash map.".to_string()], final_breakdown, recovery: None }
}

fn workspace(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pairproof-candidate-runtime-{}-{}",
        name,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("workspace");
    dir
}

fn runtime(name: &str, fake: FakeTransport) -> (PathBuf, CandidateRuntime<FakeTransport>) {
    let dir = workspace(name);
    let value = CandidateRuntime::new("s1", dir.clone(), fake);
    (dir, value)
}

fn connected(name: &str, fake: FakeTransport) -> (PathBuf, CandidateRuntime<FakeTransport>) {
    let (dir, mut value) = runtime(name, fake);
    let (conn, _secret) = connection();
    value.connect(&conn).expect("connect");
    (dir, value)
}

fn assert_hidden(text: &str, secret_hex: &str) {
    assert!(!text.contains(secret_hex), "secret leaked: {text}");
    assert!(!text.contains(HOST), "host leaked: {text}");
    assert!(!text.contains(USERNAME), "username leaked: {text}");
    assert!(!text.contains(FINGERPRINT), "fingerprint leaked: {text}");
}

#[test]
#[rustfmt::skip]
fn candidate_runtime_authenticates_before_hello() {
    let (dir, mut value) = runtime("order", FakeTransport::default()); let (conn, _secret) = connection();
    value.connect(&conn).expect("connect"); assert_eq!(value.phase(), CandidateRuntimePhase::Connected);
    let ops = &value.transport().ops; let pos = |n: &str| ops.iter().position(|o| o == n).expect(n);
    assert!(pos("authenticate") < pos("write_hello") && pos("write_hello") < pos("start_sync"), "{ops:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn candidate_runtime_rejects_wrong_host_key() {
    let canary = "0123456789abcdeffedcba9876543210";
    assert_eq!(
        candidate_connect_error_for_ssh(&SshAuthError::HostKeyMismatch),
        CandidateConnectError::Rejected
    );
    assert_eq!(
        candidate_connect_error_for_ssh(&SshAuthError::AuthFailed),
        CandidateConnectError::Rejected
    );
    let unavailable =
        candidate_connect_error_for_ssh(&SshAuthError::HostKeyUnavailable(canary.to_string()));
    assert_eq!(unavailable, CandidateConnectError::Rejected);
    assert!(!unavailable.to_string().contains(canary));
    assert!(!format!("{unavailable:?}").contains(canary));

    let fake = FakeTransport {
        auth_err: Some(CandidateRuntimeError::Auth(CandidateConnectError::Rejected)),
        ..FakeTransport::default()
    };
    let (dir, mut value) = runtime("hostkey", fake);
    let (conn, secret_hex) = connection();
    let err = value.connect(&conn).expect_err("host key must reject");
    assert_eq!(
        err,
        CandidateRuntimeError::Auth(CandidateConnectError::Rejected)
    );
    assert_eq!(value.phase(), CandidateRuntimePhase::Disconnected);
    assert_eq!(value.transport().ops, vec!["authenticate".to_string()]);
    assert_hidden(&err.to_string(), &secret_hex);
    assert_hidden(&format!("{err:?}"), &secret_hex);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn candidate_runtime_sends_intents_with_monotonic_seq() {
    let (dir, mut value) = connected("seq", FakeTransport::default());
    value
        .send_intent(IntentKind::SelectPreset { preset: 1 })
        .expect("preset");
    value.send_intent(IntentKind::Continue).expect("continue");
    value.send_chat("How do I start?").expect("chat");
    value.send_intent(IntentKind::Skip).expect("skip");
    let sent = &value.transport().sent;
    assert_eq!(sent.len(), 4);
    let seqs: Vec<u64> = sent.iter().map(|intent| intent.seq).collect();
    assert_eq!(seqs, vec![1, 2, 3, 4]);
    let mut ids: Vec<&str> = sent.iter().map(|intent| intent.id.as_str()).collect();
    assert!(ids.iter().all(|id| id.len() == 32));
    assert!(ids
        .iter()
        .all(|id| id.bytes().all(|b| b.is_ascii_hexdigit())));
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 4, "intent ids must be unique");
    assert!(sent.iter().all(|intent| intent.session_id == "s1"));
    assert_eq!(sent[0].kind, IntentKind::SelectPreset { preset: 1 });
    assert_eq!(sent[1].kind, IntentKind::Continue);
    match &sent[2].kind {
        IntentKind::ChatRequest { chat_id, text } => {
            assert!(!chat_id.is_empty());
            assert_eq!(text, "How do I start?");
        }
        other => panic!("expected chat intent, got {other:?}"),
    }
    assert_eq!(sent[3].kind, IntentKind::Skip);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn candidate_runtime_renders_only_snapshot_data() {
    let mut fake = FakeTransport::default();
    fake.snapshots
        .push_back(Some(snapshot("s1", "InProgress", None)));
    fake.snapshots
        .push_back(Some(snapshot("other-session", "InProgress", None)));
    fake.snapshots.push_back(None);
    let (dir, mut value) = connected("render", fake);
    let (conn, secret_hex) = connection();
    let _ = conn;
    let polled = value.poll_snapshot().expect("poll").expect("snapshot");
    assert_eq!(polled.session_id, "s1");
    let view = value.view();
    assert_eq!(view.question_title.as_deref(), Some("Two Sum"));
    assert_eq!(
        view.statement.as_deref(),
        Some("Find two numbers that sum to target.")
    );
    assert_eq!(view.options.len(), 1);
    assert_eq!(view.options[0].title, "Hash map");
    assert_eq!(view.selected.as_deref(), Some("preset 0"));
    assert_eq!(view.latest_test.map(|t| (t.passed, t.failed)), Some((3, 1)));
    assert_eq!(view.chat_replies, vec!["Try a hash map.".to_string()]);
    let lines = view.lines().join("\n");
    assert!(lines.contains("Two Sum"), "{lines}");
    assert!(lines.contains("Find two numbers"), "{lines}");
    assert!(lines.contains("Hash map"), "{lines}");
    assert!(lines.contains("3 passed, 1 failed"), "{lines}");
    assert!(lines.contains("Try a hash map."), "{lines}");
    for banned in ["expected_tokens", "quality", "Final rating", "grade"] {
        assert!(!lines.contains(banned), "{banned} leaked: {lines}");
    }
    assert_hidden(&lines, &secret_hex);
    let err = value.poll_snapshot().expect_err("wrong session");
    assert_eq!(err, CandidateRuntimeError::WrongSession);
    assert_eq!(value.view().question_title.as_deref(), Some("Two Sum"));
    assert_eq!(value.poll_snapshot().expect("poll none"), None);
    assert_eq!(value.view().question_title.as_deref(), Some("Two Sum"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn candidate_runtime_hides_final_until_snapshot_final() {
    let mut fake = FakeTransport::default();
    fake.snapshots
        .push_back(Some(snapshot("s1", "InProgress", None)));
    fake.snapshots
        .push_back(Some(snapshot("s1", "Completed", None)));
    let final_view = SnapshotFinal {
        rating: 7.5,
        per_question: vec![SnapshotFinalQuestion {
            slot_index: 0,
            grade: Some(8.0),
            rationale: Some("Solid approach".to_string()),
        }],
    };
    fake.snapshots
        .push_back(Some(snapshot("s1", "Completed", Some(final_view))));
    let (dir, mut value) = connected("final", fake);
    value.poll_snapshot().expect("poll in progress");
    assert!(value.view().final_breakdown.is_none());
    assert!(!value.view().lines().join("\n").contains("Final rating"));
    value.poll_snapshot().expect("poll completed bare");
    assert!(value.view().final_breakdown.is_none());
    assert!(!value.view().lines().join("\n").contains("Final rating"));
    value.poll_snapshot().expect("poll final");
    let view = value.view();
    assert!(view.final_breakdown.is_some());
    assert!(view.is_terminal());
    let lines = view.lines().join("\n");
    assert!(lines.contains("Final rating: 7.50"), "{lines}");
    assert!(lines.contains("grade 8.0"), "{lines}");
    assert!(lines.contains("Solid approach"), "{lines}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn candidate_runtime_sync_failure_is_generic() {
    let fake = FakeTransport {
        sync_fails: true,
        ..FakeTransport::default()
    };
    let (dir, mut value) = runtime("syncfail", fake);
    let (conn, secret_hex) = connection();
    let err = value.connect(&conn).expect_err("sync must fail");
    assert_eq!(err, CandidateRuntimeError::SyncFailed);
    assert_eq!(err.to_string(), candidate_sync_message());
    assert_eq!(err.to_string(), "Sync problem \u{2014} ask the expert");
    assert_eq!(value.phase(), CandidateRuntimePhase::Syncing);
    assert_ne!(value.phase(), CandidateRuntimePhase::Connected);
    assert_eq!(
        value.transport().ops,
        vec![
            "authenticate".to_string(),
            "discover_session".to_string(),
            "write_hello".to_string(),
            "start_sync".to_string()
        ]
    );
    assert_hidden(&err.to_string(), &secret_hex);
    assert_hidden(&format!("{err:?}"), &secret_hex);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn candidate_runtime_terminate_preserves_local_files() {
    let (dir, mut value) = connected("preserve", FakeTransport::default());
    let solution_dir = dir.join("questions").join("01");
    std::fs::create_dir_all(&solution_dir).expect("solution dir");
    let solution = solution_dir.join("solution.py");
    std::fs::write(&solution, "print('keep me')\n").expect("solution");
    value.end().expect("end");
    assert_eq!(value.phase(), CandidateRuntimePhase::Ended);
    assert!(value.transport().terminated);
    assert!(value
        .transport()
        .ops
        .iter()
        .any(|op| op == "terminate_sync"));
    assert_eq!(
        std::fs::read_to_string(&solution).expect("preserved"),
        "print('keep me')\n"
    );
    value.end().expect("end is idempotent");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
#[rustfmt::skip]
fn candidate_runtime_discovers_session_id_before_hello() {
    let dir = workspace("discover"); let fake = FakeTransport { discover: Some(Ok("disc-42".to_string())), ..FakeTransport::default() };
    let mut value = CandidateRuntime::new_discovering(dir.clone(), fake); let (conn, _secret) = connection(); value.connect(&conn).expect("connect discovers");
    assert_eq!(value.phase(), CandidateRuntimePhase::Connected); assert_eq!(value.session_id(), "disc-42");
    let ops = &value.transport().ops; let pos = |n: &str| ops.iter().position(|o| o == n).expect(n);
    assert!(pos("authenticate") < pos("discover_session") && pos("discover_session") < pos("write_hello") && pos("write_hello") < pos("start_sync"), "{ops:?}");
    value.send_intent(IntentKind::Continue).expect("intent"); assert_eq!(value.transport().sent[0].session_id, "disc-42");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
#[rustfmt::skip]
fn candidate_runtime_rejects_missing_session_binding() {
    let (conn, _secret) = connection(); let dir = workspace("missing"); let fake = FakeTransport { discover: Some(Err(CandidateRuntimeError::Transport)), ..FakeTransport::default() };
    let mut value = CandidateRuntime::new_discovering(dir.clone(), fake); let err = value.connect(&conn).expect_err("missing binding must fail");
    assert_eq!(err, CandidateRuntimeError::Transport); assert_eq!(value.phase(), CandidateRuntimePhase::Disconnected);
    assert_eq!(value.transport().ops, vec!["authenticate".to_string(), "discover_session".to_string()]);
    let _ = std::fs::remove_dir_all(dir);
    let dir2 = workspace("mismatch"); let fake2 = FakeTransport { discover: Some(Ok("other-9".to_string())), ..FakeTransport::default() };
    let mut bound = CandidateRuntime::new("s1", dir2.clone(), fake2);
    assert!(bound.connect(&conn).is_err()); assert_eq!(bound.phase(), CandidateRuntimePhase::Disconnected);
    let _ = std::fs::remove_dir_all(dir2);
}

#[test]
#[rustfmt::skip]
fn candidate_acks_final_snapshot_once() {
    let fin = SnapshotFinal { rating: 7.5, per_question: vec![SnapshotFinalQuestion { slot_index: 0, grade: Some(8.0), rationale: Some("Solid".into()) }] };
    let mut fake = FakeTransport::default(); fake.snapshots.push_back(Some(snapshot("s1", "InProgress", None)));
    for _ in 0..3 { fake.snapshots.push_back(Some(snapshot("s1", "Completed", Some(fin.clone())))); } let (dir, mut value) = connected("ack-once", fake);
    value.poll_snapshot().expect("poll non-final"); assert!(value.transport().acks.is_empty()); assert!(!value.final_ack_sent());
    value.poll_snapshot().expect("poll final"); assert_eq!(value.transport().acks, vec!["s1".to_string()]); assert!(value.final_ack_sent());
    value.poll_snapshot().expect("poll final again"); value.poll_snapshot().expect("poll final third");
    assert_eq!(value.transport().acks.len(), 1, "ack must be written exactly once: {:?}", value.transport().ops);
    assert!(value.view().lines().join("\n").contains("Final rating: 7.50"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
#[rustfmt::skip]
fn version_mismatch_prevents_session_start() {
    let fake = FakeTransport { expert_version: Some("9.9.9".to_string()), ..FakeTransport::default() };
    let (dir, mut value) = runtime("ver-mismatch", fake); let (conn, _s) = connection();
    let err = value.connect(&conn).expect_err("version mismatch must fail");
    match &err { CandidateRuntimeError::VersionMismatch { candidate, expert } => { assert_eq!(candidate, env!("CARGO_PKG_VERSION")); assert_eq!(expert, "9.9.9"); } other => panic!("expected VersionMismatch, got {other:?}") }
    assert!(err.to_string().contains("9.9.9") && err.to_string().contains("update/restart"), "{err}");
    assert_eq!(value.phase(), CandidateRuntimePhase::Disconnected);
    assert_eq!(value.transport().ops, vec!["authenticate".to_string(), "discover_session".to_string()], "write_hello/start_sync must never run: {:?}", value.transport().ops);
    let _ = std::fs::remove_dir_all(dir);
}
