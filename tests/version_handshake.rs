use pairproof::orchestration::control::{
    check_app_versions, expert_version_from_binding, hello_bytes, parse_hello,
    parse_session_binding, parse_session_binding_full, session_binding_bytes, ControlError,
};

#[test]
fn hello_roundtrip_carries_app_version() {
    let bytes = hello_bytes("s1", "1.2.3");
    let hello = parse_hello(&bytes, "s1").unwrap();
    assert_eq!(hello.v, 1);
    assert_eq!(hello.session_id, "s1");
    assert_eq!(hello.app_version.as_deref(), Some("1.2.3"));
    let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(wire["app_version"], "1.2.3");
    assert!(parse_hello(&bytes, "other").is_err());
}

#[test]
fn session_binding_carries_app_version_and_old_parse_still_works() {
    let bytes = session_binding_bytes("s1");
    assert_eq!(parse_session_binding(&bytes).unwrap(), "s1");
    let full = parse_session_binding_full(&bytes).unwrap();
    assert_eq!(full.session_id, "s1");
    assert_eq!(full.app_version.as_deref(), Some(env!("CARGO_PKG_VERSION")));
    assert_eq!(
        expert_version_from_binding(&bytes).unwrap().as_deref(),
        Some(env!("CARGO_PKG_VERSION"))
    );
    let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(wire["v"], 1);
}

#[test]
fn old_binding_without_version_parses() {
    let data = br#"{"v":1,"session_id":"s1"}"#;
    assert_eq!(parse_session_binding(data).unwrap(), "s1");
    assert_eq!(expert_version_from_binding(data).unwrap(), None);
    assert_eq!(parse_session_binding_full(data).unwrap().app_version, None);
    // Old hello without version also parses (version gate rejects it separately).
    let hello = parse_hello(br#"{"v":1,"session_id":"s1"}"#, "s1").unwrap();
    assert_eq!(hello.app_version, None);
}

#[test]
fn check_app_versions_requires_exact_equality() {
    assert!(check_app_versions(Some("0.1.0"), "0.1.0").is_ok());
    assert!(check_app_versions(Some(" 0.1.0 "), "0.1.0").is_ok());
    let err = check_app_versions(Some("0.1.0"), "0.1.1").unwrap_err();
    assert!(matches!(err, ControlError::VersionMismatch { .. }));
    assert!(err.to_string().contains("0.1.0") && err.to_string().contains("0.1.1"));
    let missing = check_app_versions(None, "0.1.0").unwrap_err();
    assert!(missing.to_string().contains("unknown"));
    assert!(missing.to_string().contains("update/restart"));
    assert!(check_app_versions(Some(""), "0.1.0").is_err());
    assert!(check_app_versions(Some("   "), "0.1.0").is_err());
}

#[test]
fn version_check_handles_release_published_between_participant_launches() {
    // Expert launched at 0.1.0; candidate auto-updated to 0.2.0 at launch.
    // Expert-side gate: candidate hello 0.2.0 vs expert 0.1.0.
    let hello = hello_bytes("s1", "0.2.0");
    let parsed = parse_hello(&hello, "s1").unwrap();
    let expert_err = check_app_versions(parsed.app_version.as_deref(), "0.1.0").unwrap_err();
    assert!(expert_err.to_string().contains("0.2.0"));
    assert!(expert_err.to_string().contains("0.1.0"));
    assert!(expert_err.to_string().contains("update/restart"));
    // Candidate-side check: binding published by expert at 0.1.0 vs candidate 0.2.0.
    let binding = br#"{"v":1,"session_id":"s1","app_version":"0.1.0"}"#;
    let expert_version = expert_version_from_binding(binding).unwrap();
    assert_eq!(expert_version.as_deref(), Some("0.1.0"));
    let candidate_err = check_app_versions(Some("0.1.0"), "0.2.0").unwrap_err();
    assert!(candidate_err.to_string().contains("0.1.0"));
    assert!(candidate_err.to_string().contains("0.2.0"));
    assert!(candidate_err.to_string().contains("update/restart"));
    assert_ne!(expert_version.as_deref(), Some("0.2.0"));
}
