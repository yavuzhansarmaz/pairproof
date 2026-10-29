use pairproof::core::Language;
use pairproof::sandbox::validate::{
    container_name, network_name, question_layout, validate_container_name, validate_session_id,
    WorkspacePath,
};
use pairproof::sandbox::SandboxError;
use pairproof::sync::command::sync_session_name;

#[test]
fn session_id_accepts_and_rejects() {
    assert!(validate_session_id("a").is_ok());
    assert!(validate_session_id("Sess_01-x").is_ok());
    assert!(validate_session_id(&"a".repeat(40)).is_ok());
    for bad in ["", "a/b", "x;rm", "a b", "a.b", "a:b", "--privileged", "-x"] {
        assert_eq!(
            validate_session_id(bad),
            Err(SandboxError::InvalidSessionId(bad.to_string())),
            "should reject {bad:?}"
        );
    }
    assert!(validate_session_id(&"a".repeat(41)).is_err());
}

#[test]
fn container_and_network_names_derived() {
    assert_eq!(container_name("s1").expect("container"), "pairproof-s1");
    assert_eq!(network_name("s1").expect("network"), "pairproof-net-s1");
    assert!(container_name("--privileged").is_err());
    assert!(network_name("bad/id").is_err());
    assert!(validate_container_name("pairproof-s1").is_ok());
    assert!(validate_container_name("pairproof-net-s1").is_err());
    assert!(validate_container_name("pairproof-").is_err());
    assert!(validate_container_name("other-s1").is_err());
    assert!(validate_container_name("pairproof-bad/id").is_err());
}

#[test]
fn workspace_path_rejects_traversal_absolute_and_dot() {
    for bad in [
        "../secret",
        "questions/../secret",
        "..",
        "/etc/passwd",
        ".",
        "./solution.py",
        "questions/./solution.py",
    ] {
        assert!(WorkspacePath::new(bad).is_err(), "should reject {bad:?}");
    }
}

#[test]
fn workspace_path_rejects_backslash_control_and_empty_components() {
    for bad in [
        "questions\\01\\solution.py",
        "\\windows\\path",
        "questions//solution.py",
        "questions/01/",
        "bad\u{7}name",
        "bad\nname",
        "bad\u{0}name",
        "",
    ] {
        assert!(WorkspacePath::new(bad).is_err(), "should reject {bad:?}");
    }
    assert!(WorkspacePath::new(&"a".repeat(256)).is_err());
    let over_total = format!("{}/{}/c", "a".repeat(255), "b".repeat(255));
    assert_eq!(over_total.len(), 513);
    assert!(WorkspacePath::new(&over_total).is_err());
}

#[test]
fn workspace_path_container_path_and_parent() {
    let path = WorkspacePath::new("questions/01/solution.py").expect("valid path");
    assert_eq!(path.as_str(), "questions/01/solution.py");
    assert_eq!(path.container_path(), "/workspace/questions/01/solution.py");
    assert_eq!(
        path.parent_container_dir(),
        Some("/workspace/questions/01".to_string())
    );
    let root = WorkspacePath::new("solution.py").expect("valid root file");
    assert_eq!(root.container_path(), "/workspace/solution.py");
    assert_eq!(root.parent_container_dir(), None);
}

#[test]
fn question_layout_per_language() {
    let cases = [
        (Language::Python, "solution.py"),
        (Language::JavaScript, "solution.js"),
        (Language::Sql, "solution.sql"),
    ];
    for (language, file) in cases {
        let (question, solution) = question_layout(0, language).expect("layout");
        assert_eq!(question.as_str(), "questions/01/question.md");
        assert_eq!(solution.as_str(), format!("questions/01/{file}"));
    }
    let (_, solution) = question_layout(9, Language::Python).expect("layout");
    assert_eq!(solution.as_str(), "questions/10/solution.py");
    let (_, solution) = question_layout(98, Language::Python).expect("layout");
    assert_eq!(solution.as_str(), "questions/99/solution.py");
}

#[test]
fn question_layout_rejects_large_index() {
    assert!(question_layout(99, Language::Python).is_err());
    assert!(question_layout(1000, Language::JavaScript).is_err());
    assert!(question_layout(usize::MAX, Language::Sql).is_err());
}

#[test]
fn session_id_rejects_net_prefix_to_avoid_network_collision() {
    for bad in ["net-x", "net-"] {
        assert!(validate_session_id(bad).is_err(), "validate {bad:?}");
        assert!(container_name(bad).is_err(), "container {bad:?}");
        assert!(network_name(bad).is_err(), "network {bad:?}");
        assert!(sync_session_name(bad).is_err(), "sync {bad:?}");
    }
    let valid = ["network1", "net", "a-net-x", "s1", "x"];
    for id in valid {
        let container = container_name(id).expect("valid container name");
        assert!(
            validate_container_name(&container).is_ok(),
            "container for {id:?}"
        );
        for other in valid {
            assert_ne!(container, network_name(other).expect("network name"));
        }
    }
}
