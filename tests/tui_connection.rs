//! Migrated from `ConnectionController` (deleted): every controller method
//! delegated 1:1 to `SecretRotator`; production acceptance is the stronger
//! `ConnectionGate::accept` (keyscan + attach_precheck + rotator verify).
//! Tests below exercise the gate where production does, and the rotator
//! for lifecycle semantics the gate intentionally requires an orchestrator
//! for (idempotent terminate, reissue-from-locked).

use pairproof::connection::{
    ConnectionError, ConnectionState, ConnectionString, Endpoint, EphemeralUsername, Fingerprint,
    Secret, SecretRotator, ROTATION_WINDOW_MS,
};
use pairproof::core::{Language, QuestionRef};
use pairproof::orchestration::connection::{
    ConnectionGate, ConnectionGateError, GatePhase, HostKeyVerifier,
};
use pairproof::orchestration::Orchestrator;
use pairproof::tui::shared::sanitize_display;

mod common_mod;
use common_mod::q_easy as q;

const T0: u64 = 1_730_000_000_000;
const FINGERPRINT_HEX: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_FINGERPRINT_HEX: &str =
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[derive(Debug)]
struct OkVerifier;
impl HostKeyVerifier for OkVerifier {
    fn verify_host_key(&self, _e: &Endpoint, _f: &Fingerprint) -> Result<(), ConnectionGateError> {
        Ok(())
    }
}
fn endpoint() -> Endpoint {
    Endpoint::new("203.0.113.10", 22022).expect("endpoint")
}
fn username() -> EphemeralUsername {
    EphemeralUsername::parse("pp-a1b2c3d4").expect("username")
}
fn fingerprint() -> Fingerprint {
    Fingerprint::parse(FINGERPRINT_HEX).expect("fingerprint")
}
fn gate() -> ConnectionGate {
    ConnectionGate::with_verifier(endpoint(), username(), fingerprint(), OkVerifier, T0)
}
fn rotator() -> SecretRotator {
    SecretRotator::new(endpoint(), username(), fingerprint(), T0)
}

fn waiting() -> Orchestrator {
    let easy: Vec<QuestionRef> = (0..5).map(|i| q(&format!("e{i}"))).collect();
    let mut o = Orchestrator::create("s1".into(), Language::Python, easy, 0).expect("create");
    o.mark_waiting_for_candidate(1).expect("waiting");
    o
}
fn rebuild(base: &ConnectionString, secret: Secret, expires_at_ms: u64) -> ConnectionString {
    ConnectionString::new(
        base.endpoint().clone(),
        base.username().clone(),
        secret,
        expires_at_ms,
        base.fingerprint().clone(),
    )
}
fn different_secret(current: &Secret) -> Secret {
    let mut candidate = Secret::generate();
    while candidate == *current {
        candidate = Secret::generate();
    }
    candidate
}
fn is_expired(state: ConnectionState, countdown: u64) -> bool {
    state == ConnectionState::Rotating && countdown == 0
}
fn conn_err(err: ConnectionGateError) -> ConnectionError {
    match err {
        ConnectionGateError::Connection(e) => e,
        other => panic!("expected connection error, got {other:?}"),
    }
}

#[test]
fn connection_countdown_uses_rotator_window() {
    let gate = gate();
    assert_eq!(gate.state(), ConnectionState::Rotating);
    assert_eq!(gate.phase(), GatePhase::Awaiting);
    assert_eq!(gate.countdown_secs(T0), 60);
    assert_eq!(gate.countdown_secs(T0 + 1_000), 59);
    assert_eq!(gate.countdown_secs(T0 + 59_000), 1);
    assert_eq!(gate.countdown_secs(T0 + 59_999), 1);
    assert_eq!(gate.countdown_secs(T0 + 60_000), 0);
    assert_eq!(gate.countdown_secs(T0 + 61_000), 0);
    assert_eq!(gate.countdown_secs(T0 - 10_000), 60);
}

#[test]
fn countdown_boundary_at_exact_expiry_is_half_open() {
    let mut g = gate();
    let mut o = waiting();
    let current = g.current_string(T0).expect("current");
    let mut before = gate();
    let mut o2 = waiting();
    let before_current = before.current_string(T0).expect("before current");
    assert!(!is_expired(
        before.state(),
        before.countdown_secs(T0 + ROTATION_WINDOW_MS - 1)
    ));
    assert_eq!(before.countdown_secs(T0 + ROTATION_WINDOW_MS - 1), 1);
    assert_eq!(
        before.accept(&mut o2, &before_current, T0 + ROTATION_WINDOW_MS - 1),
        Ok(())
    );
    assert_eq!(before.state(), ConnectionState::Locked);
    assert_eq!(before.phase(), GatePhase::Locked);
    assert!(is_expired(
        g.state(),
        g.countdown_secs(T0 + ROTATION_WINDOW_MS)
    ));
    assert_eq!(g.countdown_secs(T0 + ROTATION_WINDOW_MS), 0);
    let err = g
        .accept(&mut o, &current, T0 + ROTATION_WINDOW_MS)
        .expect_err("exact expiry must fail");
    assert_eq!(conn_err(err), ConnectionError::Expired);
    assert_eq!(g.state(), ConnectionState::Rotating);
    assert_eq!(g.phase(), GatePhase::ReissueNeeded);
}

#[test]
fn expired_connection_requires_reissue() {
    let mut g = gate();
    let mut o = waiting();
    let old = g.current_string(T0).expect("old");
    assert!(is_expired(
        g.state(),
        g.countdown_secs(T0 + ROTATION_WINDOW_MS)
    ));
    let err = g
        .accept(&mut o, &old, T0 + ROTATION_WINDOW_MS)
        .expect_err("expired must not lock");
    assert_eq!(conn_err(err), ConnectionError::Expired);
    assert_ne!(g.state(), ConnectionState::Locked);
    let fresh = g
        .reissue_after_disconnect(T0 + ROTATION_WINDOW_MS)
        .expect("reissue");
    assert_ne!(fresh.secret(), old.secret());
    assert_eq!(fresh.expires_at_ms(), T0 + 2 * ROTATION_WINDOW_MS);
    assert!(!is_expired(
        g.state(),
        g.countdown_secs(T0 + ROTATION_WINDOW_MS)
    ));
    assert_eq!(g.countdown_secs(T0 + ROTATION_WINDOW_MS), 60);
    assert_eq!(g.phase(), GatePhase::Awaiting);
    assert_eq!(
        g.accept(&mut o, &fresh, T0 + ROTATION_WINDOW_MS + 1_000),
        Ok(())
    );
    assert_eq!(g.state(), ConnectionState::Locked);
}

#[test]
fn reissue_invalidates_previous_secret() {
    let mut g = gate();
    let mut o = waiting();
    let old = g.current_string(T0).expect("old");
    let reissue_at = T0 + 1_000;
    let new = g.reissue_after_disconnect(reissue_at).expect("new");
    assert_ne!(new.secret(), old.secret());
    assert_eq!(new.endpoint(), old.endpoint());
    assert_eq!(new.username(), old.username());
    assert_eq!(new.fingerprint(), old.fingerprint());
    let err = g
        .accept(&mut o, &old, reissue_at + 1_000)
        .expect_err("old secret must be dead");
    assert_eq!(conn_err(err), ConnectionError::Expired);
    assert_eq!(g.state(), ConnectionState::Rotating);
    assert_eq!(g.accept(&mut o, &new, reissue_at + 2_000), Ok(()));
    assert_eq!(g.state(), ConnectionState::Locked);
    assert_eq!(g.phase(), GatePhase::Locked);
}

#[test]
fn genuine_connection_event_locks_and_wrong_secret_does_not() {
    let mut g = gate();
    let mut o = waiting();
    let current = g.current_string(T0).expect("current");
    let wrong = rebuild(
        &current,
        different_secret(current.secret()),
        current.expires_at_ms(),
    );
    assert_eq!(
        conn_err(
            g.accept(&mut o, &wrong, T0 + 1_000)
                .expect_err("wrong secret")
        ),
        ConnectionError::WrongSecret
    );
    assert_eq!(g.state(), ConnectionState::Rotating);
    assert_eq!(g.phase(), GatePhase::Awaiting);
    assert_eq!(g.accept(&mut o, &current, T0 + 2_000), Ok(()));
    assert_eq!(g.state(), ConnectionState::Locked);
    assert_eq!(g.phase(), GatePhase::Locked);
    assert_eq!(g.countdown_secs(T0 + 2_000), 0);
    assert_eq!(
        conn_err(
            g.accept(&mut o, &current, T0 + 3_000)
                .expect_err("already locked")
        ),
        ConnectionError::AlreadyLocked
    );
}

#[test]
fn fingerprint_mismatch_does_not_lock() {
    let mut g = gate();
    let mut o = waiting();
    let current = g.current_string(T0).expect("current");
    let tampered = ConnectionString::new(
        current.endpoint().clone(),
        current.username().clone(),
        current.secret().clone(),
        current.expires_at_ms(),
        Fingerprint::parse(OTHER_FINGERPRINT_HEX).expect("other fingerprint"),
    );
    assert_eq!(
        conn_err(g.accept(&mut o, &tampered, T0 + 1_000).expect_err("fp")),
        ConnectionError::FingerprintMismatch
    );
    assert_eq!(g.state(), ConnectionState::Rotating);
    assert_ne!(g.phase(), GatePhase::Locked);
    assert_eq!(g.accept(&mut o, &current, T0 + 2_000), Ok(()));
}

#[test]
fn expired_presented_string_does_not_lock() {
    let mut g = gate();
    let mut o = waiting();
    let current = g.current_string(T0).expect("current");
    let tampered = rebuild(
        &current,
        current.secret().clone(),
        current.expires_at_ms() + 10_000,
    );
    assert_eq!(
        conn_err(
            g.accept(&mut o, &tampered, T0 + 1_000)
                .expect_err("expired")
        ),
        ConnectionError::Expired
    );
    assert_eq!(g.state(), ConnectionState::Rotating);
}

#[test]
fn display_string_is_canonical_and_countdown_matches() {
    let mut g = gate();
    let current = g.current_string(T0).expect("current");
    let displayed = sanitize_display(&current.encode());
    assert!(displayed.starts_with("pairproof:v1:203.0.113.10:22022:pp-a1b2c3d4:"));
    assert!(displayed.ends_with(FINGERPRINT_HEX));
    let parsed = ConnectionString::parse(&displayed).expect("display parses");
    assert_eq!(parsed, current);
    assert_eq!(parsed.expires_at_ms(), T0 + ROTATION_WINDOW_MS);
    assert!(!displayed.contains('\u{1b}'));
    let again = g.current_string(T0 + 1_000).expect("again");
    assert_eq!(displayed, sanitize_display(&again.encode()));
}

#[test]
fn current_string_rotates_on_expiry_and_old_is_dead() {
    let mut g = gate();
    let mut o = waiting();
    let first = g.current_string(T0).expect("first");
    let second = g.current_string(T0 + ROTATION_WINDOW_MS).expect("rotated");
    assert_ne!(second.secret(), first.secret());
    assert_eq!(second.expires_at_ms(), T0 + 2 * ROTATION_WINDOW_MS);
    assert_eq!(
        conn_err(
            g.accept(&mut o, &first, T0 + ROTATION_WINDOW_MS + 1_000)
                .expect_err("old dead")
        ),
        ConnectionError::Expired
    );
    assert_eq!(
        g.accept(&mut o, &second, T0 + ROTATION_WINDOW_MS + 1_000),
        Ok(())
    );
}

#[test]
fn disconnect_reissue_path_issues_fresh_string() {
    let mut g = gate();
    let mut o = waiting();
    let first = g.current_string(T0).expect("first");
    assert_eq!(g.accept(&mut o, &first, T0 + 1_000), Ok(()));
    assert_eq!(g.state(), ConnectionState::Locked);
    let disconnect_at = T0 + 5_000;
    g.handle_disconnect(&mut o, disconnect_at)
        .expect("disconnect");
    assert_eq!(g.phase(), GatePhase::ReissueNeeded);
    let fresh = g
        .reissue_after_disconnect(disconnect_at)
        .expect("reissue on disconnect");
    assert_ne!(fresh.secret(), first.secret());
    assert_eq!(fresh.expires_at_ms(), disconnect_at + ROTATION_WINDOW_MS);
    assert_eq!(g.state(), ConnectionState::Rotating);
    assert_eq!(g.phase(), GatePhase::Awaiting);
    assert_eq!(g.countdown_secs(disconnect_at), 60);
    assert_eq!(
        conn_err(
            g.accept(&mut o, &first, disconnect_at + 500)
                .expect_err("pre-disconnect dead")
        ),
        ConnectionError::Expired
    );
    assert_eq!(g.accept(&mut o, &fresh, disconnect_at + 1_000), Ok(()));
    assert_eq!(g.state(), ConnectionState::Locked);
}

#[test]
fn terminate_is_idempotent_and_blocks_all() {
    // Gate::terminate also ends the orchestrator (not idempotent there);
    // idempotent termination is the rotator semantics the controller used.
    let mut r = rotator();
    let current = r.current_string(T0).expect("current");
    r.terminate();
    assert_eq!(r.state(), ConnectionState::Ended);
    assert_eq!(r.countdown_secs(T0 + 1_000), 0);
    assert!(!is_expired(r.state(), r.countdown_secs(T0 + 1_000)));
    assert_eq!(
        r.current_string(T0 + 1_000),
        Err(ConnectionError::SessionEnded)
    );
    assert_eq!(r.reissue(T0 + 1_000), Err(ConnectionError::SessionEnded));
    assert_eq!(
        r.verify(&current, T0 + 1_000),
        Err(ConnectionError::SessionEnded)
    );
    r.terminate();
    assert_eq!(r.state(), ConnectionState::Ended);
    assert_eq!(r.reissue(T0 + 2_000), Err(ConnectionError::SessionEnded));
}

#[test]
fn terminate_from_locked_blocks_reissue() {
    let mut r = rotator();
    let current = r.current_string(T0).expect("current");
    assert_eq!(r.verify(&current, T0 + 1_000), Ok(()));
    r.terminate();
    r.terminate();
    assert_eq!(r.state(), ConnectionState::Ended);
    assert_eq!(
        r.current_string(T0 + 2_000),
        Err(ConnectionError::SessionEnded)
    );
    assert_eq!(r.reissue(T0 + 2_000), Err(ConnectionError::SessionEnded));
}

#[test]
fn reissue_from_locked_allows_new_connection() {
    let mut r = rotator();
    let current = r.current_string(T0).expect("current");
    assert_eq!(r.verify(&current, T0 + 1_000), Ok(()));
    let fresh = r.reissue(T0 + 2_000).expect("reissue");
    assert_eq!(r.state(), ConnectionState::Rotating);
    assert_eq!(r.verify(&fresh, T0 + 3_000), Ok(()));
    assert_eq!(r.state(), ConnectionState::Locked);
}

#[test]
fn error_is_expired_helper_distinguishes_variants() {
    // Deleted free helper was `matches!(err, Expired)`; assert that exact
    // predicate (now inline) plus gate wrapping preserves it.
    let is_exp = |e: &ConnectionError| matches!(e, ConnectionError::Expired);
    assert!(is_exp(&ConnectionError::Expired));
    assert!(!is_exp(&ConnectionError::WrongSecret));
    assert!(!is_exp(&ConnectionError::SessionEnded));
    assert!(!is_exp(&ConnectionError::AlreadyLocked));
    assert!(!is_exp(&ConnectionError::FingerprintMismatch));
    assert_eq!(
        ConnectionGateError::from(ConnectionError::Expired),
        ConnectionGateError::Connection(ConnectionError::Expired)
    );
}

#[test]
fn controller_debug_does_not_leak_secret() {
    let mut g = gate();
    let current = g.current_string(T0).expect("current");
    let secret_hex = current.secret().expose_hex();
    assert!(!format!("{g:?}").contains(&secret_hex));
    let mut r = rotator();
    let _ = r.current_string(T0).expect("rotator current");
    assert!(!format!("{r:?}").contains(&secret_hex));
}
