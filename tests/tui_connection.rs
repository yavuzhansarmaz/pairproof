use pairproof::connection::{
    ConnectionError, ConnectionState, ConnectionString, Endpoint, EphemeralUsername, Fingerprint,
    Secret, ROTATION_WINDOW_MS,
};
use pairproof::tui::connection::{error_is_expired, ConnectionController, ConnectionPhase};

const T0: u64 = 1_730_000_000_000;
const FINGERPRINT_HEX: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_FINGERPRINT_HEX: &str =
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn endpoint() -> Endpoint {
    Endpoint::new("203.0.113.10", 22022).expect("endpoint")
}

fn username() -> EphemeralUsername {
    EphemeralUsername::parse("pp-a1b2c3d4").expect("username")
}

fn fingerprint() -> Fingerprint {
    Fingerprint::parse(FINGERPRINT_HEX).expect("fingerprint")
}

fn controller() -> ConnectionController {
    ConnectionController::new(endpoint(), username(), fingerprint(), T0)
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

#[test]
fn connection_countdown_uses_rotator_window() {
    let controller = controller();
    assert_eq!(controller.state(), ConnectionState::Rotating);
    assert_eq!(controller.phase(), ConnectionPhase::AwaitingCandidate);
    assert_eq!(controller.countdown_secs(T0), 60);
    assert_eq!(controller.countdown_secs(T0 + 1_000), 59);
    assert_eq!(controller.countdown_secs(T0 + 59_000), 1);
    assert_eq!(controller.countdown_secs(T0 + 59_999), 1);
    assert_eq!(controller.countdown_secs(T0 + 60_000), 0);
    assert_eq!(controller.countdown_secs(T0 + 61_000), 0);
    // Countdown delegates to rotator: capped at 60 even before window start.
    assert_eq!(controller.countdown_secs(T0 - 10_000), 60);
}

#[test]
fn countdown_boundary_at_exact_expiry_is_half_open() {
    let mut ctrl = controller();
    let current = ctrl.current_string(T0).expect("current");
    // One ms before expiry still valid and locks.
    let mut before = controller();
    let before_current = before.current_string(T0).expect("before current");
    assert!(!before.is_expired(T0 + ROTATION_WINDOW_MS - 1));
    assert_eq!(before.countdown_secs(T0 + ROTATION_WINDOW_MS - 1), 1);
    assert_eq!(
        before.on_candidate_connected(&before_current, T0 + ROTATION_WINDOW_MS - 1),
        Ok(())
    );
    assert_eq!(before.state(), ConnectionState::Locked);
    assert_eq!(before.phase(), ConnectionPhase::Locked);

    // At exact expiry the presented string is expired (half-open [issue, expiry)).
    assert!(ctrl.is_expired(T0 + ROTATION_WINDOW_MS));
    assert_eq!(ctrl.countdown_secs(T0 + ROTATION_WINDOW_MS), 0);
    let err = ctrl
        .on_candidate_connected(&current, T0 + ROTATION_WINDOW_MS)
        .expect_err("exact expiry must fail");
    assert_eq!(err, ConnectionError::Expired);
    assert!(error_is_expired(&err));
    assert_eq!(ctrl.state(), ConnectionState::Rotating);
    assert_eq!(ctrl.phase(), ConnectionPhase::ReissueNeeded);
}

#[test]
fn expired_connection_requires_reissue() {
    let mut controller = controller();
    let old = controller.current_string(T0).expect("old");
    assert!(controller.is_expired(T0 + ROTATION_WINDOW_MS));
    let err = controller
        .on_candidate_connected(&old, T0 + ROTATION_WINDOW_MS)
        .expect_err("expired must not lock");
    assert_eq!(err, ConnectionError::Expired);
    assert!(error_is_expired(&err));
    assert_ne!(controller.state(), ConnectionState::Locked);
    // Reissue gives a fresh window and the new string can lock.
    let fresh = controller
        .reissue(T0 + ROTATION_WINDOW_MS)
        .expect("reissue");
    assert_ne!(fresh.secret(), old.secret());
    assert_eq!(fresh.expires_at_ms(), T0 + 2 * ROTATION_WINDOW_MS);
    assert!(!controller.is_expired(T0 + ROTATION_WINDOW_MS));
    assert_eq!(controller.countdown_secs(T0 + ROTATION_WINDOW_MS), 60);
    assert_eq!(controller.phase(), ConnectionPhase::AwaitingCandidate);
    assert_eq!(
        controller.on_candidate_connected(&fresh, T0 + ROTATION_WINDOW_MS + 1_000),
        Ok(())
    );
    assert_eq!(controller.state(), ConnectionState::Locked);
}

#[test]
fn reissue_invalidates_previous_secret() {
    let mut controller = controller();
    let old = controller.current_string(T0).expect("old");
    let reissue_at = T0 + 1_000;
    let new = controller.reissue(reissue_at).expect("new");
    assert_ne!(new.secret(), old.secret());
    assert_eq!(new.endpoint(), old.endpoint());
    assert_eq!(new.username(), old.username());
    assert_eq!(new.fingerprint(), old.fingerprint());
    // Old string now fails: its expiry no longer matches the held window.
    let err = controller
        .on_candidate_connected(&old, reissue_at + 1_000)
        .expect_err("old secret must be dead");
    assert_eq!(err, ConnectionError::Expired);
    assert_eq!(controller.state(), ConnectionState::Rotating);
    // New string succeeds.
    assert_eq!(
        controller.on_candidate_connected(&new, reissue_at + 2_000),
        Ok(())
    );
    assert_eq!(controller.state(), ConnectionState::Locked);
    assert_eq!(controller.phase(), ConnectionPhase::Locked);
}

#[test]
fn genuine_connection_event_locks_and_wrong_secret_does_not() {
    let mut controller = controller();
    let current = controller.current_string(T0).expect("current");
    let wrong = rebuild(
        &current,
        different_secret(current.secret()),
        current.expires_at_ms(),
    );
    assert_eq!(
        controller.on_candidate_connected(&wrong, T0 + 1_000),
        Err(ConnectionError::WrongSecret)
    );
    assert_eq!(controller.state(), ConnectionState::Rotating);
    assert_eq!(controller.phase(), ConnectionPhase::AwaitingCandidate);
    // Genuine presented string locks.
    assert_eq!(
        controller.on_candidate_connected(&current, T0 + 2_000),
        Ok(())
    );
    assert_eq!(controller.state(), ConnectionState::Locked);
    assert_eq!(controller.phase(), ConnectionPhase::Locked);
    assert_eq!(controller.countdown_secs(T0 + 2_000), 0);
    // Second attempt is already locked, not a timer inference.
    assert_eq!(
        controller.on_candidate_connected(&current, T0 + 3_000),
        Err(ConnectionError::AlreadyLocked)
    );
}

#[test]
fn fingerprint_mismatch_does_not_lock() {
    let mut controller = controller();
    let current = controller.current_string(T0).expect("current");
    let tampered = ConnectionString::new(
        current.endpoint().clone(),
        current.username().clone(),
        current.secret().clone(),
        current.expires_at_ms(),
        Fingerprint::parse(OTHER_FINGERPRINT_HEX).expect("other fingerprint"),
    );
    assert_eq!(
        controller.on_candidate_connected(&tampered, T0 + 1_000),
        Err(ConnectionError::FingerprintMismatch)
    );
    assert_eq!(controller.state(), ConnectionState::Rotating);
    assert_ne!(controller.phase(), ConnectionPhase::Locked);
    // Correct string still works after a failed attempt.
    assert_eq!(
        controller.on_candidate_connected(&current, T0 + 2_000),
        Ok(())
    );
}

#[test]
fn expired_presented_string_does_not_lock() {
    let mut controller = controller();
    let current = controller.current_string(T0).expect("current");
    // Tamper expiry to a later value: rotator treats mismatched expiry as expired.
    let tampered = rebuild(
        &current,
        current.secret().clone(),
        current.expires_at_ms() + 10_000,
    );
    assert_eq!(
        controller.on_candidate_connected(&tampered, T0 + 1_000),
        Err(ConnectionError::Expired)
    );
    assert_eq!(controller.state(), ConnectionState::Rotating);
}

#[test]
fn display_string_is_canonical_and_countdown_matches() {
    let mut controller = controller();
    let displayed = controller.display_string(T0).expect("display");
    assert!(displayed.starts_with("pairproof:v1:203.0.113.10:22022:pp-a1b2c3d4:"));
    assert!(displayed.ends_with(FINGERPRINT_HEX));
    let parsed = ConnectionString::parse(&displayed).expect("display parses");
    let current = controller.current_string(T0).expect("current");
    assert_eq!(parsed, current);
    assert_eq!(parsed.expires_at_ms(), T0 + ROTATION_WINDOW_MS);
    assert!(!displayed.contains('\u{1b}'));
    // Stable within the window.
    let again = controller.display_string(T0 + 1_000).expect("again");
    assert_eq!(displayed, again);
}

#[test]
fn current_string_rotates_on_expiry_and_old_is_dead() {
    let mut controller = controller();
    let first = controller.current_string(T0).expect("first");
    let second = controller
        .current_string(T0 + ROTATION_WINDOW_MS)
        .expect("rotated");
    assert_ne!(second.secret(), first.secret());
    assert_eq!(second.expires_at_ms(), T0 + 2 * ROTATION_WINDOW_MS);
    assert_eq!(
        controller.on_candidate_connected(&first, T0 + ROTATION_WINDOW_MS + 1_000),
        Err(ConnectionError::Expired)
    );
    assert_eq!(
        controller.on_candidate_connected(&second, T0 + ROTATION_WINDOW_MS + 1_000),
        Ok(())
    );
}

#[test]
fn disconnect_reissue_path_issues_fresh_string() {
    let mut controller = controller();
    let first = controller.current_string(T0).expect("first");
    assert_eq!(
        controller.on_candidate_connected(&first, T0 + 1_000),
        Ok(())
    );
    assert_eq!(controller.state(), ConnectionState::Locked);
    let disconnect_at = T0 + 5_000;
    let fresh = controller
        .on_accidental_disconnect(disconnect_at)
        .expect("reissue on disconnect");
    assert_ne!(fresh.secret(), first.secret());
    assert_eq!(fresh.expires_at_ms(), disconnect_at + ROTATION_WINDOW_MS);
    assert_eq!(controller.state(), ConnectionState::Rotating);
    assert_eq!(controller.phase(), ConnectionPhase::AwaitingCandidate);
    assert_eq!(controller.countdown_secs(disconnect_at), 60);
    // Pre-disconnect string is dead.
    assert_eq!(
        controller.on_candidate_connected(&first, disconnect_at + 500),
        Err(ConnectionError::Expired)
    );
    // Fresh string reconnects.
    assert_eq!(
        controller.on_candidate_connected(&fresh, disconnect_at + 1_000),
        Ok(())
    );
    assert_eq!(controller.state(), ConnectionState::Locked);
}

#[test]
fn terminate_is_idempotent_and_blocks_all() {
    let mut controller = controller();
    let current = controller.current_string(T0).expect("current");
    controller.terminate();
    assert_eq!(controller.state(), ConnectionState::Ended);
    assert_eq!(controller.phase(), ConnectionPhase::Ended);
    assert_eq!(controller.countdown_secs(T0 + 1_000), 0);
    assert!(!controller.is_expired(T0 + 1_000));
    assert_eq!(
        controller.current_string(T0 + 1_000),
        Err(ConnectionError::SessionEnded)
    );
    assert_eq!(
        controller.display_string(T0 + 1_000),
        Err(ConnectionError::SessionEnded)
    );
    assert_eq!(
        controller.reissue(T0 + 1_000),
        Err(ConnectionError::SessionEnded)
    );
    assert_eq!(
        controller.on_accidental_disconnect(T0 + 1_000),
        Err(ConnectionError::SessionEnded)
    );
    assert_eq!(
        controller.on_candidate_connected(&current, T0 + 1_000),
        Err(ConnectionError::SessionEnded)
    );
    // Second terminate stays ended.
    controller.terminate();
    assert_eq!(controller.state(), ConnectionState::Ended);
    assert_eq!(controller.phase(), ConnectionPhase::Ended);
    assert_eq!(
        controller.reissue(T0 + 2_000),
        Err(ConnectionError::SessionEnded)
    );
}

#[test]
fn terminate_from_locked_blocks_reissue() {
    let mut controller = controller();
    let current = controller.current_string(T0).expect("current");
    assert_eq!(
        controller.on_candidate_connected(&current, T0 + 1_000),
        Ok(())
    );
    controller.terminate();
    controller.terminate();
    assert_eq!(controller.state(), ConnectionState::Ended);
    assert_eq!(
        controller.current_string(T0 + 2_000),
        Err(ConnectionError::SessionEnded)
    );
    assert_eq!(
        controller.reissue(T0 + 2_000),
        Err(ConnectionError::SessionEnded)
    );
}

#[test]
fn reissue_from_locked_allows_new_connection() {
    let mut controller = controller();
    let current = controller.current_string(T0).expect("current");
    assert_eq!(
        controller.on_candidate_connected(&current, T0 + 1_000),
        Ok(())
    );
    let fresh = controller.reissue(T0 + 2_000).expect("reissue");
    assert_eq!(controller.state(), ConnectionState::Rotating);
    assert_eq!(
        controller.on_candidate_connected(&fresh, T0 + 3_000),
        Ok(())
    );
    assert_eq!(controller.state(), ConnectionState::Locked);
}

#[test]
fn error_is_expired_helper_distinguishes_variants() {
    assert!(error_is_expired(&ConnectionError::Expired));
    assert!(!error_is_expired(&ConnectionError::WrongSecret));
    assert!(!error_is_expired(&ConnectionError::SessionEnded));
    assert!(!error_is_expired(&ConnectionError::AlreadyLocked));
    assert!(!error_is_expired(&ConnectionError::FingerprintMismatch));
}

#[test]
fn controller_debug_does_not_leak_secret() {
    let mut controller = controller();
    let current = controller.current_string(T0).expect("current");
    let secret_hex = current.secret().expose_hex();
    let debug = format!("{controller:?}");
    assert!(!debug.contains(&secret_hex));
}
