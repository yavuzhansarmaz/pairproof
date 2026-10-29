use pairproof::connection::{
    ConnectionError, ConnectionState, Endpoint, EphemeralUsername, Fingerprint, SecretRotator,
    ROTATION_WINDOW_MS,
};
use pairproof::core::{EventKind, Language, QuestionRef, Session};
use pairproof::leetcode::Difficulty;

const T0: u64 = 1_730_000_000_000;
const FINGERPRINT_HEX: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn endpoint() -> Endpoint {
    Endpoint::new("203.0.113.10", 22022).expect("endpoint")
}

fn username() -> EphemeralUsername {
    EphemeralUsername::parse("pp-a1b2c3d4").expect("username")
}

fn fingerprint() -> Fingerprint {
    Fingerprint::parse(FINGERPRINT_HEX).expect("fingerprint")
}

fn rotator() -> SecretRotator {
    SecretRotator::new(endpoint(), username(), fingerprint(), T0)
}

fn easy_questions() -> Vec<QuestionRef> {
    (0..5)
        .map(|i| QuestionRef {
            title_slug: format!("e{i}"),
            title: format!("e{i}"),
            difficulty: Difficulty::Easy,
            acceptance_pct: None,
        })
        .collect()
}

#[test]
fn reissue_from_rotating_kills_previous_secret() {
    let mut rotator = rotator();
    let old = rotator.current_string(T0).expect("old string");
    let reissue_now = T0 + 1_000;
    let new = rotator.reissue(reissue_now).expect("reissued string");
    assert_ne!(new.secret(), old.secret());
    assert_eq!(rotator.state(), ConnectionState::Rotating);
    // Old expiry no longer matches the held window, so verify yields Expired.
    assert_eq!(
        rotator.verify(&old, reissue_now + 1_000),
        Err(ConnectionError::Expired)
    );
    assert_eq!(rotator.verify(&new, reissue_now + 2_000), Ok(()));
    assert_eq!(rotator.state(), ConnectionState::Locked);
}

#[test]
fn reissue_restarts_full_window_and_countdown() {
    let mut rotator = rotator();
    let _old = rotator.current_string(T0).expect("old string");
    let reissue_now = T0 + 40_000;
    let new = rotator.reissue(reissue_now).expect("reissued string");
    assert_eq!(new.expires_at_ms(), reissue_now + ROTATION_WINDOW_MS);
    assert_eq!(rotator.countdown_secs(reissue_now), 60);
    assert_eq!(rotator.countdown_secs(reissue_now + 59_000), 1);
    assert_eq!(rotator.countdown_secs(reissue_now + 60_000), 0);
}

#[test]
fn reissue_from_locked_allows_new_connection() {
    let mut rotator = rotator();
    let current = rotator.current_string(T0).expect("current string");
    rotator.verify(&current, T0 + 1_000).expect("first verify");
    assert_eq!(rotator.state(), ConnectionState::Locked);
    let reissue_now = T0 + 2_000;
    let new = rotator.reissue(reissue_now).expect("reissued string");
    assert_eq!(rotator.state(), ConnectionState::Rotating);
    assert_eq!(rotator.verify(&new, reissue_now + 1_000), Ok(()));
    assert_eq!(rotator.state(), ConnectionState::Locked);
}

#[test]
fn reissue_keeps_endpoint_username_fingerprint() {
    let mut rotator = rotator();
    let old = rotator.current_string(T0).expect("old string");
    let new = rotator.reissue(T0 + 1_000).expect("reissued string");
    assert_eq!(new.endpoint(), old.endpoint());
    assert_eq!(new.username(), old.username());
    assert_eq!(new.fingerprint(), old.fingerprint());
    assert_ne!(new.secret(), old.secret());
}

#[test]
fn previously_connected_secret_stays_dead_after_reissue() {
    let mut rotator = rotator();
    let connected = rotator.current_string(T0).expect("current string");
    rotator
        .verify(&connected, T0 + 1_000)
        .expect("first verify");
    let reissue_now = T0 + 2_000;
    let new = rotator.reissue(reissue_now).expect("reissued string");
    // Held window moved on, so the old expiry yields Expired.
    assert_eq!(
        rotator.verify(&connected, reissue_now + 1_000),
        Err(ConnectionError::Expired)
    );
    assert_eq!(rotator.verify(&new, reissue_now + 2_000), Ok(()));
    // Once locked again, state precedence yields AlreadyLocked.
    assert_eq!(
        rotator.verify(&connected, reissue_now + 3_000),
        Err(ConnectionError::AlreadyLocked)
    );
}

#[test]
fn terminate_from_rotating_blocks_everything() {
    let mut rotator = rotator();
    let current = rotator.current_string(T0).expect("current string");
    rotator.terminate();
    assert_eq!(rotator.state(), ConnectionState::Ended);
    assert_eq!(rotator.countdown_secs(T0 + 1_000), 0);
    assert_eq!(
        rotator.verify(&current, T0 + 1_000),
        Err(ConnectionError::SessionEnded)
    );
    assert_eq!(
        rotator.current_string(T0 + 1_000),
        Err(ConnectionError::SessionEnded)
    );
    assert_eq!(
        rotator.reissue(T0 + 1_000),
        Err(ConnectionError::SessionEnded)
    );
}

#[test]
fn terminate_from_locked_blocks_everything() {
    let mut rotator = rotator();
    let current = rotator.current_string(T0).expect("current string");
    rotator.verify(&current, T0 + 1_000).expect("verify");
    assert_eq!(rotator.state(), ConnectionState::Locked);
    rotator.terminate();
    assert_eq!(rotator.state(), ConnectionState::Ended);
    assert_eq!(rotator.countdown_secs(T0 + 1_000), 0);
    assert_eq!(
        rotator.verify(&current, T0 + 2_000),
        Err(ConnectionError::SessionEnded)
    );
    assert_eq!(
        rotator.current_string(T0 + 2_000),
        Err(ConnectionError::SessionEnded)
    );
    assert_eq!(
        rotator.reissue(T0 + 2_000),
        Err(ConnectionError::SessionEnded)
    );
}

#[test]
fn terminate_is_idempotent_and_final() {
    let mut rotator = rotator();
    let current = rotator.current_string(T0).expect("current string");
    rotator.terminate();
    rotator.terminate();
    assert_eq!(rotator.state(), ConnectionState::Ended);
    assert_eq!(
        rotator.reissue(T0 + 1_000),
        Err(ConnectionError::SessionEnded)
    );
    assert_eq!(
        rotator.verify(&current, T0 + 1_000),
        Err(ConnectionError::SessionEnded)
    );
    assert_eq!(
        rotator.current_string(T0 + 1_000),
        Err(ConnectionError::SessionEnded)
    );
    assert_eq!(rotator.countdown_secs(T0 + 1_000), 0);
}

#[test]
fn reissue_after_terminate_is_session_ended() {
    let mut rotator = rotator();
    rotator.terminate();
    assert_eq!(
        rotator.reissue(T0 + 1_000),
        Err(ConnectionError::SessionEnded)
    );
    assert_eq!(rotator.state(), ConnectionState::Ended);
}

#[test]
fn core_seam_reconnect_story() {
    let mut session = Session::new("seam-1".to_string(), Language::Python, easy_questions(), T0)
        .expect("session");
    session.mark_waiting_for_candidate(T0).expect("waiting");

    let mut rotator = rotator();
    let first = rotator.current_string(T0).expect("first string");
    let connect_at = T0 + 1_000;
    assert_eq!(rotator.verify(&first, connect_at), Ok(()));
    session.candidate_connected(connect_at).expect("connected");
    assert!(session.is_connected());
    assert!(session
        .events()
        .iter()
        .any(|e| e.kind == EventKind::CandidateConnected));

    // The rotator lock is the gate relied on for a second connection attempt.
    assert_eq!(
        rotator.verify(&first, T0 + 2_000),
        Err(ConnectionError::AlreadyLocked)
    );

    let t1 = T0 + 3_000;
    session.candidate_disconnected(t1).expect("disconnected");
    assert!(!session.is_connected());
    assert!(session
        .events()
        .iter()
        .any(|e| e.kind == EventKind::CandidateDisconnected));

    let second = rotator.reissue(t1).expect("reissued string");
    assert_eq!(second.expires_at_ms(), t1 + ROTATION_WINDOW_MS);
    // Pre-disconnect string is dead: its expiry no longer matches the held window.
    assert_eq!(
        rotator.verify(&first, t1 + 500),
        Err(ConnectionError::Expired)
    );

    let t2 = T0 + 4_000;
    assert_eq!(rotator.verify(&second, t2), Ok(()));
    session.candidate_connected(t2).expect("reconnected");
    assert!(session.is_connected());
    assert!(session.events().iter().any(|e| {
        e.kind
            == EventKind::CandidateReconnected {
                downtime_ms: t2 - t1,
            }
    }));
    assert_eq!(
        rotator.verify(&first, t2 + 1_000),
        Err(ConnectionError::AlreadyLocked)
    );
}
