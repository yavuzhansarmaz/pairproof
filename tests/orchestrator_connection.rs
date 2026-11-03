use pairproof::connection::{
    ConnectionError, ConnectionState, ConnectionString, Endpoint, EphemeralUsername, Fingerprint,
    Secret, ROTATION_WINDOW_MS,
};
use pairproof::core::{CoreError, EventKind, Language, QuestionStatus, SessionStatus};
use pairproof::orchestration::connection::{
    ConnectionGate, ConnectionGateError, GatePhase, HostKeyVerifier, SshKeyscanVerifier,
};
use pairproof::orchestration::{Orchestrator, OrchestratorError};
use std::sync::{Arc, Mutex};

mod common_mod;
use common_mod::{easy_five as easy, endpoint, fingerprint, username};
const T0: u64 = 1_730_000_000_000;
#[derive(Debug)]
struct FakeState {
    calls: usize,
    result: Result<(), ConnectionGateError>,
    seen: Vec<Endpoint>,
}
#[derive(Debug, Clone)]
struct FakeVerifier(Arc<Mutex<FakeState>>);
impl FakeVerifier {
    fn new(result: Result<(), ConnectionGateError>) -> (Self, Arc<Mutex<FakeState>>) {
        let state = Arc::new(Mutex::new(FakeState {
            calls: 0,
            result,
            seen: Vec::new(),
        }));
        let v = Self(state.clone());
        (v, state)
    }
}
impl HostKeyVerifier for FakeVerifier {
    fn verify_host_key(&self, e: &Endpoint, _f: &Fingerprint) -> Result<(), ConnectionGateError> {
        let mut s = self.0.lock().expect("fake state");
        s.calls += 1;
        s.seen.push(e.clone());
        s.result.clone()
    }
}

fn waiting() -> Orchestrator {
    let mut o = Orchestrator::create("s1".into(), Language::Python, easy(), 0).expect("create");
    o.mark_waiting_for_candidate(1).expect("waiting");
    o
}
fn gate_with(v: FakeVerifier) -> ConnectionGate {
    ConnectionGate::with_verifier(endpoint(), username(), fingerprint(), v, T0)
}
fn connected(o: &Orchestrator) -> usize {
    o.session()
        .events()
        .iter()
        .filter(|e| e.kind == EventKind::CandidateConnected)
        .count()
}
fn aerr(
    g: &mut ConnectionGate,
    o: &mut Orchestrator,
    s: &ConnectionString,
    now: u64,
) -> ConnectionGateError {
    g.accept(o, s, now).expect_err("accept must fail")
}
fn rebuild(base: &ConnectionString, secret: Secret, exp: u64, fp: Fingerprint) -> ConnectionString {
    let (e, u) = (base.endpoint().clone(), base.username().clone());
    cs(e, u, secret, exp, fp)
}
fn cs(e: Endpoint, u: EphemeralUsername, s: Secret, x: u64, f: Fingerprint) -> ConnectionString {
    ConnectionString::new(e, u, s, x, f)
}
fn locked() -> ConnectionGateError {
    ConnectionError::AlreadyLocked.into()
}
fn ended() -> ConnectionGateError {
    ConnectionError::SessionEnded.into()
}
fn started() -> (ConnectionGate, Orchestrator, ConnectionString) {
    let (v, _) = FakeVerifier::new(Ok(()));
    let mut gate = gate_with(v);
    let mut o = waiting();
    let s = gate.current_string(T0).expect("string");
    gate.accept(&mut o, &s, T0 + 1_000).expect("accept");
    o.start(T0 + 2_000).expect("start");
    (gate, o, s)
}
#[test]
fn connection_string_is_not_accepted_before_host_key_verification() {
    let (v, state) = FakeVerifier::new(Err(ConnectionGateError::HostKeyMismatch));
    let mut gate = gate_with(v);
    let mut o = waiting();
    let cur = gate.current_string(T0).expect("string");
    let (x, f) = (cur.expires_at_ms(), cur.fingerprint().clone());
    let mut wrong = rebuild(&cur, Secret::from_bytes([7u8; 16]), x, f.clone());
    if wrong.secret() == cur.secret() {
        wrong = rebuild(&cur, Secret::from_bytes([6u8; 16]), x, f);
    }
    let err = aerr(&mut gate, &mut o, &wrong, T0 + 1_000);
    assert_eq!(err, ConnectionGateError::HostKeyMismatch);
    assert_eq!(state.lock().expect("fake state").calls, 1);
    assert!(!o.is_connected() && connected(&o) == 0);
    assert!(gate.state() == ConnectionState::Rotating && gate.phase() == GatePhase::Awaiting);
    state.lock().expect("fake state").result = Ok(());
    let genuine = gate.current_string(T0 + 1_500).expect("string");
    gate.accept(&mut o, &genuine, T0 + 2_000)
        .expect("accept after pass");
    assert!(o.is_connected() && connected(&o) == 1);
}
#[test]
fn connection_accept_requires_real_ssh_authentication() {
    let dead = Endpoint::new("127.0.0.1", 1).expect("endpoint");
    assert!(matches!(
        SshKeyscanVerifier.verify_host_key(&dead, &fingerprint()),
        Err(ConnectionGateError::HostKeyUnavailable(_))
    ));
    let mut gate = ConnectionGate::new(dead, username(), fingerprint(), T0);
    let mut o = waiting();
    let s = gate.current_string(T0).expect("string");
    assert!(matches!(
        aerr(&mut gate, &mut o, &s, T0 + 1_000),
        ConnectionGateError::HostKeyUnavailable(_)
    ));
    assert!(!o.is_connected() && gate.state() == ConnectionState::Rotating);
    let (v, _) = FakeVerifier::new(Ok(()));
    let mut gate = gate_with(v);
    let mut o = waiting();
    let s = gate.current_string(T0).expect("string");
    gate.accept(&mut o, &s, T0 + 1_000)
        .expect("fake-verified accept");
    assert!(o.is_connected());
}
#[test]
fn connection_accept_verifies_endpoint_username_secret_expiry_and_fingerprint() {
    use ConnectionError::*;
    let (v, _) = FakeVerifier::new(Ok(()));
    let mut gate = gate_with(v);
    let mut o = waiting();
    let base = gate.current_string(T0).expect("string");
    let mut other_secret = Secret::generate();
    while other_secret == *base.secret() {
        other_secret = Secret::generate();
    }
    let e2 = Endpoint::new("198.51.100.20", 22022).expect("ep");
    let u2 = EphemeralUsername::parse("pp-zzzz9999").expect("user");
    let f2 = Fingerprint::parse(&"b".repeat(64)).expect("fp");
    let e = base.endpoint().clone();
    let u = base.username().clone();
    let s = base.secret().clone();
    let (x, f) = (base.expires_at_ms(), base.fingerprint().clone());
    let p = cs(e2, u.clone(), s.clone(), x, f.clone());
    let err = aerr(&mut gate, &mut o, &p, T0 + 1_000);
    assert_eq!(err, EndpointMismatch.into());
    let p = cs(e.clone(), u2, s.clone(), x, f.clone());
    let err = aerr(&mut gate, &mut o, &p, T0 + 1_000);
    assert_eq!(err, UsernameMismatch.into());
    let p = cs(e.clone(), u.clone(), other_secret, x, f.clone());
    assert_eq!(aerr(&mut gate, &mut o, &p, T0 + 1_000), WrongSecret.into());
    let p = cs(e.clone(), u.clone(), s.clone(), x + 1_000, f.clone());
    assert_eq!(aerr(&mut gate, &mut o, &p, T0 + 1_000), Expired.into());
    let p = cs(e, u, s, x, f2);
    let err = aerr(&mut gate, &mut o, &p, T0 + 1_000);
    assert_eq!(err, FingerprintMismatch.into());
    assert!(!o.is_connected() && connected(&o) == 0 && gate.state() == ConnectionState::Rotating);
    gate.accept(&mut o, &base, T0 + 2_000)
        .expect("genuine still accepted");
    assert!(o.is_connected() && connected(&o) == 1);
}
#[test]
fn connection_accept_transitions_core_exactly_once() {
    let (v, _) = FakeVerifier::new(Ok(()));
    let mut gate = gate_with(v);
    let mut o = waiting();
    let s = gate.current_string(T0).expect("string");
    gate.accept(&mut o, &s, T0 + 1_000).expect("accept");
    assert!(o.is_connected() && o.status() == SessionStatus::WaitingForCandidate);
    assert!(gate.phase() == GatePhase::Locked && gate.state() == ConnectionState::Locked);
    assert_eq!(connected(&o), 1);
    let n = o.session().events().len();
    assert_eq!(aerr(&mut gate, &mut o, &s, T0 + 2_000), locked());
    assert_eq!(o.session().events().len(), n);
    assert_eq!(connected(&o), 1);
}
#[test]
fn wrong_or_expired_secret_never_connects() {
    let (v, _) = FakeVerifier::new(Ok(()));
    let mut gate = gate_with(v);
    let mut o = waiting();
    let base = gate.current_string(T0).expect("string");
    let (x, f) = (base.expires_at_ms(), base.fingerprint().clone());
    let mut wrong = rebuild(&base, Secret::from_bytes([9u8; 16]), x, f.clone());
    if wrong.secret() == base.secret() {
        wrong = rebuild(&base, Secret::from_bytes([8u8; 16]), x, f);
    }
    let err = aerr(&mut gate, &mut o, &wrong, T0 + 1_000);
    assert_eq!(err, ConnectionError::WrongSecret.into());
    assert!(!o.is_connected());
    let (v, _) = FakeVerifier::new(Ok(()));
    let mut gate = gate_with(v);
    let mut o = waiting();
    let s = gate.current_string(T0).expect("string");
    assert_eq!(gate.countdown_secs(T0), 60);
    let err = aerr(&mut gate, &mut o, &s, T0 + ROTATION_WINDOW_MS);
    assert_eq!(err, ConnectionError::Expired.into());
    assert!(!o.is_connected() && connected(&o) == 0 && gate.phase() == GatePhase::ReissueNeeded);
    let fresh = gate
        .current_string(T0 + ROTATION_WINDOW_MS)
        .expect("rotated");
    gate.accept(&mut o, &fresh, T0 + ROTATION_WINDOW_MS + 1_000)
        .expect("fresh accepted");
    assert!(o.is_connected());
}
#[test]
fn stale_string_cannot_reconnect() {
    let (mut gate, mut o, s) = started();
    gate.handle_disconnect(&mut o, T0 + 3_000)
        .expect("disconnect");
    assert!(!o.is_connected());
    let n = o.session().events().len();
    assert_eq!(aerr(&mut gate, &mut o, &s, T0 + 4_000), locked());
    assert!(!o.is_connected() && o.session().events().len() == n && connected(&o) == 1);
}
#[test]
fn accidental_disconnect_preserves_container_and_session() {
    let (mut gate, mut o, _) = started();
    let slots_before = o.session().slots().to_vec();
    gate.handle_disconnect(&mut o, T0 + 5_000)
        .expect("disconnect");
    assert_eq!(o.status(), SessionStatus::InProgress);
    assert_eq!(o.current_index(), Some(0));
    assert_eq!(o.session().slots(), slots_before.as_slice());
    assert!(!o.is_connected() && !o.is_terminal());
    assert!(o.session().events().iter().all(|e| !matches!(
        e.kind,
        EventKind::ExpertEnded | EventKind::TechnicalFailure | EventKind::Completed
    )));
    assert!(gate.phase() == GatePhase::ReissueNeeded && gate.state() == ConnectionState::Locked);
    let err = gate.current_string(T0 + 6_000).expect_err("still locked");
    assert_eq!(err, locked());
}
#[test]
fn reissue_resumes_same_question_and_stage() {
    let (mut gate, mut o, old) = started();
    let started_at = o.session().slots()[0].started_at_ms;
    gate.handle_disconnect(&mut o, T0 + 3_000)
        .expect("disconnect");
    let fresh = gate.reissue_after_disconnect(T0 + 4_000).expect("reissue");
    assert_ne!(fresh.secret(), old.secret());
    assert_eq!(gate.phase(), GatePhase::Awaiting);
    assert!(gate.accept(&mut o, &old, T0 + 5_000).is_err() && !o.is_connected());
    gate.accept(&mut o, &fresh, T0 + 6_000).expect("reconnect");
    assert!(
        o.is_connected() && o.status() == SessionStatus::InProgress && o.current_index() == Some(0)
    );
    assert_eq!(o.session().slots()[0].status, QuestionStatus::Active);
    assert_eq!(o.session().slots()[0].started_at_ms, started_at);
    assert_eq!(connected(&o), 1);
    assert!(o.session().events().iter().any(|e| matches!(
        e.kind,
        EventKind::CandidateReconnected { downtime_ms: 3_000 }
    )));
}
#[test]
fn deliberate_expert_end_cannot_reissue() {
    let (mut gate, mut o, s) = started();
    gate.terminate(&mut o, T0 + 3_000).expect("terminate");
    assert_eq!(o.status(), SessionStatus::EndedByExpert);
    assert!(o.is_terminal() && gate.phase() == GatePhase::Ended);
    assert_eq!(gate.state(), ConnectionState::Ended);
    let err = gate
        .reissue_after_disconnect(T0 + 4_000)
        .expect_err("reissue");
    assert_eq!(err, ended());
    assert_eq!(aerr(&mut gate, &mut o, &s, T0 + 4_000), ended());
    let err = gate.current_string(T0 + 4_000).expect_err("string");
    assert_eq!(err, ended());
    assert_eq!(connected(&o), 1);
}
#[test]
fn accept_verifies_gate_endpoint_not_presented_endpoint() {
    let (v, state) = FakeVerifier::new(Ok(()));
    let mut gate = gate_with(v);
    let mut o = waiting();
    let base = gate.current_string(T0).expect("string");
    let other = Endpoint::new("198.51.100.20", 22022).expect("ep");
    let (u, s, x, f) = (
        base.username().clone(),
        base.secret().clone(),
        base.expires_at_ms(),
        base.fingerprint().clone(),
    );
    let presented = cs(other, u, s, x, f);
    let err = aerr(&mut gate, &mut o, &presented, T0 + 1_000);
    assert_eq!(err, ConnectionError::EndpointMismatch.into());
    assert_eq!(state.lock().expect("fake state").seen, vec![endpoint()]);
    assert_eq!(state.lock().expect("fake state").calls, 1);
    assert!(!o.is_connected() && connected(&o) == 0);
    gate.accept(&mut o, &base, T0 + 2_000).expect("genuine");
    assert!(o.is_connected());
}
#[test]
fn accept_failure_before_attach_keeps_secret_alive() {
    let (v, state) = FakeVerifier::new(Ok(()));
    let mut gate = gate_with(v);
    let mut o = Orchestrator::create("s1".into(), Language::Python, easy(), 0).expect("create");
    assert_eq!(o.status(), SessionStatus::Setup);
    let s = gate.current_string(T0).expect("string");
    let err = aerr(&mut gate, &mut o, &s, T0 + 1_000);
    let want = OrchestratorError::Core(CoreError::InvalidState(
        "candidate_connected in Setup".into(),
    ));
    assert_eq!(err, ConnectionGateError::Orchestrator(want));
    assert_eq!(state.lock().expect("fake state").calls, 0);
    assert!(state.lock().expect("fake state").seen.is_empty());
    assert!(gate.state() == ConnectionState::Rotating && gate.phase() == GatePhase::Awaiting);
    assert!(!o.is_connected());
    o.mark_waiting_for_candidate(1).expect("waiting");
    gate.accept(&mut o, &s, T0 + 2_000).expect("same genuine");
    assert!(o.is_connected() && gate.phase() == GatePhase::Locked);
}
#[test]
fn accept_sets_locked_only_after_successful_attach() {
    let (v, _) = FakeVerifier::new(Ok(()));
    let mut gate = gate_with(v);
    let mut o = Orchestrator::create("s1".into(), Language::Python, easy(), 0).expect("create");
    let s = gate.current_string(T0).expect("string");
    assert!(gate.accept(&mut o, &s, T0 + 1_000).is_err());
    assert_ne!(gate.phase(), GatePhase::Locked);
    assert_eq!(gate.state(), ConnectionState::Rotating);
    o.mark_waiting_for_candidate(1).expect("waiting");
    gate.accept(&mut o, &s, T0 + 2_000).expect("accept");
    assert!(gate.phase() == GatePhase::Locked && gate.state() == ConnectionState::Locked);
    assert!(o.is_connected());
}
#[test]
fn password_authentication_is_explicitly_unsupported() {
    let (v, _) = FakeVerifier::new(Ok(()));
    let gate = gate_with(v);
    let result = gate.authenticate_password();
    assert_eq!(result, Err(ConnectionGateError::PasswordAuthUnsupported));
    let mut gate = ConnectionGate::new(endpoint(), username(), fingerprint(), T0);
    let _ = gate.current_string(T0).expect("string");
    let result = gate.authenticate_password();
    assert_eq!(result, Err(ConnectionGateError::PasswordAuthUnsupported));
}
#[test]
fn connection_gate_error_redacts_secret_canary() {
    let canary = "0123456789abcdeffedcba9876543210";
    let (v, _) = FakeVerifier::new(Ok(()));
    let mut gate = gate_with(v);
    let mut o = waiting();
    let base = gate.current_string(T0).expect("string");
    let (x, f) = (base.expires_at_ms(), base.fingerprint().clone());
    let presented = rebuild(&base, Secret::from_hex(canary).expect("canary"), x, f);
    if presented.secret() != base.secret() {
        let err = gate
            .accept(&mut o, &presented, T0 + 1_000)
            .expect_err("canary rejected");
        assert!(!format!("{err:?}").contains(canary) && !format!("{err}").contains(canary));
    }
    let payload = ConnectionGateError::HostKeyUnavailable(canary.to_string());
    assert!(!format!("{payload:?}").contains(canary) && !format!("{payload}").contains(canary));
    assert!(!format!("{gate:?}").contains(&base.secret().expose_hex()));
}
#[test]
#[ignore = "live ssh-keyscan: requires ssh-keyscan on PATH (dead-endpoint probe must fail closed)"]
fn live_ssh_password_auth_is_blocked_and_keyscan_fails_closed() {
    let gate = ConnectionGate::new(endpoint(), username(), fingerprint(), T0);
    let result = gate.authenticate_password();
    assert_eq!(result, Err(ConnectionGateError::PasswordAuthUnsupported));
    let dead = Endpoint::new("127.0.0.1", 1).expect("endpoint");
    assert!(matches!(
        SshKeyscanVerifier.verify_host_key(&dead, &fingerprint()),
        Err(ConnectionGateError::HostKeyUnavailable(_))
    ));
}
