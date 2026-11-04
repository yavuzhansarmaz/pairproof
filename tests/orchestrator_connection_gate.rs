use pairproof::connection::{
    ConnectionError, ConnectionState, ConnectionString, Endpoint, Fingerprint,
};
use pairproof::core::{EventKind, Language};
use pairproof::orchestration::connection::{
    ConnectionGate, ConnectionGateError, GatePhase, HostKeyVerifier,
};
use pairproof::orchestration::Orchestrator;
use std::sync::{Arc, Mutex};

use crate::common_mod;
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
fn locked() -> ConnectionGateError {
    ConnectionError::AlreadyLocked.into()
}
#[test]
fn connection_gate_accept_after_ssh_auth_locks_once() {
    let (v, state) = FakeVerifier::new(Ok(()));
    let mut gate = gate_with(v);
    let mut o = waiting();
    let presented = gate.current_string(T0).expect("string");
    gate.accept_after_ssh_auth(&mut o, T0 + 1_000)
        .expect("ssh-auth accept");
    assert!(o.is_connected() && connected(&o) == 1);
    assert_eq!(gate.phase(), GatePhase::Locked);
    assert_eq!(gate.state(), ConnectionState::Locked);
    assert_eq!(
        state.lock().expect("fake state").calls,
        0,
        "host-key verifier must not run"
    );
    o.start(T0 + 2_000).expect("start after ssh-auth accept");
    let events = o.session().events().len();
    let err = gate
        .accept_after_ssh_auth(&mut o, T0 + 3_000)
        .expect_err("second ssh-auth accept");
    assert_eq!(err, locked());
    assert_eq!(o.session().events().len(), events);
    assert_eq!(connected(&o), 1);
    assert_eq!(aerr(&mut gate, &mut o, &presented, T0 + 4_000), locked());
}
