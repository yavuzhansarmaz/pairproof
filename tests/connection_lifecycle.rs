use pairproof::connection::{
    ConnectionError, ConnectionState, ConnectionString, Endpoint, EphemeralUsername, Fingerprint,
    Secret, SecretRotator, ROTATION_WINDOW_MS,
};

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

fn rotator() -> SecretRotator {
    SecretRotator::new(endpoint(), username(), fingerprint(), T0)
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
fn two_generated_secrets_differ() {
    assert_ne!(Secret::generate(), Secret::generate());
}

#[test]
fn new_rotator_is_rotating_with_full_countdown() {
    let rotator = rotator();
    assert_eq!(rotator.state(), ConnectionState::Rotating);
    assert_eq!(ROTATION_WINDOW_MS, 60_000);
    assert_eq!(rotator.countdown_secs(T0), 60);
    assert_eq!(rotator.countdown_secs(T0 + 59_000), 1);
    assert_eq!(rotator.countdown_secs(T0 + 59_999), 1);
    assert_eq!(rotator.countdown_secs(T0 + 60_000), 0);
    assert_eq!(rotator.countdown_secs(T0 - 10_000), 60);
}

#[test]
fn current_string_stable_within_window() {
    let mut rotator = rotator();
    let first = rotator.current_string(T0).expect("first string");
    let second = rotator.current_string(T0 + 1_000).expect("second string");
    assert_eq!(first, second);
}

#[test]
fn rotation_on_expiry_issues_new_secret() {
    let mut rotator = rotator();
    let first = rotator.current_string(T0).expect("first string");
    let second = rotator
        .current_string(T0 + ROTATION_WINDOW_MS)
        .expect("rotated string");
    assert_ne!(second.secret(), first.secret());
    assert_eq!(second.endpoint(), first.endpoint());
    assert_eq!(second.username(), first.username());
    assert_eq!(second.fingerprint(), first.fingerprint());
    assert_eq!(first.expires_at_ms(), T0 + ROTATION_WINDOW_MS);
    assert_eq!(second.expires_at_ms(), T0 + 2 * ROTATION_WINDOW_MS);
}

#[test]
fn verify_succeeds_with_current_string_and_locks() {
    let mut rotator = rotator();
    let current = rotator.current_string(T0).expect("current string");
    assert_eq!(rotator.verify(&current, T0 + 1_000), Ok(()));
    assert_eq!(rotator.state(), ConnectionState::Locked);
}

#[test]
fn second_verify_after_lock_is_already_locked() {
    let mut rotator = rotator();
    let current = rotator.current_string(T0).expect("current string");
    rotator.verify(&current, T0 + 1_000).expect("first verify");
    assert_eq!(
        rotator.verify(&current, T0 + 2_000),
        Err(ConnectionError::AlreadyLocked)
    );
}

#[test]
fn verify_wrong_secret_rejected_and_still_rotating() {
    let mut rotator = rotator();
    let current = rotator.current_string(T0).expect("current string");
    let wrong = rebuild(
        &current,
        different_secret(current.secret()),
        current.expires_at_ms(),
    );
    assert_eq!(
        rotator.verify(&wrong, T0 + 1_000),
        Err(ConnectionError::WrongSecret)
    );
    assert_eq!(rotator.state(), ConnectionState::Rotating);
    assert_eq!(rotator.verify(&current, T0 + 2_000), Ok(()));
}

#[test]
fn verify_old_secret_after_rotation_is_expired() {
    let mut rotator = rotator();
    let old = rotator.current_string(T0).expect("old string");
    let _new = rotator
        .current_string(T0 + ROTATION_WINDOW_MS)
        .expect("rotated string");
    assert_eq!(
        rotator.verify(&old, T0 + ROTATION_WINDOW_MS),
        Err(ConnectionError::Expired)
    );
}

#[test]
fn verify_at_exact_expiry_is_expired_and_one_ms_before_succeeds() {
    let mut at_expiry = rotator();
    let current = at_expiry.current_string(T0).expect("current string");
    assert_eq!(
        at_expiry.verify(&current, T0 + ROTATION_WINDOW_MS),
        Err(ConnectionError::Expired)
    );

    let mut before_expiry = rotator();
    let current = before_expiry.current_string(T0).expect("current string");
    assert_eq!(
        before_expiry.verify(&current, T0 + ROTATION_WINDOW_MS - 1),
        Ok(())
    );
}

#[test]
fn verify_tampered_later_expiry_rejected() {
    let mut rotator = rotator();
    let current = rotator.current_string(T0).expect("current string");
    let tampered = rebuild(
        &current,
        current.secret().clone(),
        current.expires_at_ms() + 10_000_000,
    );
    assert_eq!(
        rotator.verify(&tampered, T0 + 1_000),
        Err(ConnectionError::Expired)
    );
}

#[test]
fn verify_tampered_fingerprint_rejected() {
    let mut rotator = rotator();
    let current = rotator.current_string(T0).expect("current string");
    let tampered = ConnectionString::new(
        current.endpoint().clone(),
        current.username().clone(),
        current.secret().clone(),
        current.expires_at_ms(),
        Fingerprint::parse(OTHER_FINGERPRINT_HEX).expect("other fingerprint"),
    );
    assert_eq!(
        rotator.verify(&tampered, T0 + 1_000),
        Err(ConnectionError::FingerprintMismatch)
    );
}

#[test]
fn verify_tampered_endpoint_and_username_rejected() {
    let mut rotator = rotator();
    let current = rotator.current_string(T0).expect("current string");
    let wrong_endpoint = ConnectionString::new(
        Endpoint::new("203.0.113.11", 22022).expect("other endpoint"),
        current.username().clone(),
        current.secret().clone(),
        current.expires_at_ms(),
        current.fingerprint().clone(),
    );
    assert_eq!(
        rotator.verify(&wrong_endpoint, T0 + 1_000),
        Err(ConnectionError::EndpointMismatch)
    );
    let wrong_username = ConnectionString::new(
        current.endpoint().clone(),
        EphemeralUsername::parse("pp-zzzz9999").expect("other username"),
        current.secret().clone(),
        current.expires_at_ms(),
        current.fingerprint().clone(),
    );
    assert_eq!(
        rotator.verify(&wrong_username, T0 + 1_000),
        Err(ConnectionError::UsernameMismatch)
    );
}

#[test]
fn verify_after_window_without_rotation_is_expired() {
    let mut rotator = rotator();
    let current = rotator.current_string(T0).expect("current string");
    assert_eq!(
        rotator.verify(&current, T0 + ROTATION_WINDOW_MS),
        Err(ConnectionError::Expired)
    );
}

#[test]
fn countdown_zero_after_lock() {
    let mut rotator = rotator();
    let current = rotator.current_string(T0).expect("current string");
    rotator.verify(&current, T0 + 1_000).expect("verify");
    assert_eq!(rotator.countdown_secs(T0), 0);
    assert_eq!(rotator.countdown_secs(T0 + 1_000), 0);
}

#[test]
fn current_string_after_lock_is_already_locked() {
    let mut rotator = rotator();
    let current = rotator.current_string(T0).expect("current string");
    rotator.verify(&current, T0 + 1_000).expect("verify");
    assert_eq!(
        rotator.current_string(T0 + 2_000),
        Err(ConnectionError::AlreadyLocked)
    );
}

#[test]
fn rotator_debug_does_not_leak_secret() {
    let mut rotator = rotator();
    let current = rotator.current_string(T0).expect("current string");
    let secret_hex = current.secret().expose_hex();
    let debug = format!("{rotator:?}");
    assert!(!debug.contains(&secret_hex));
}
