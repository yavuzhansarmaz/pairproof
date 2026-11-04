use pairproof::connection::{Endpoint, EphemeralUsername};
use pairproof::sandbox::SandboxError;
use pairproof::sync::command::{
    beta_url, find_sync, mutagen_version, parse_mutagen_version, parse_sync_list, parse_sync_names,
    sync_create, sync_flush, sync_list, sync_list_template, sync_resume, sync_session_name,
    sync_terminate, validate_alpha_dir, SyncSessionInfo, STATUS_TEMPLATE,
};
use pairproof::sync::SyncError;

const LIST_FIXTURE: &str = "\
--------------------------------------------------------------------------------
Name: pairproof-sync-s1
Identifier: sync_Rp9exampleOne
Labels:
\tmutagen.io/session: pairproof
Alpha:
\tURL: /home/candidate/session-s1
\tConnection state: Connected
Beta:
\tURL: pp-abcd1234@203.0.113.10:2222:/workspace
\tConnection state: Connected
Status: Watching for changes
--------------------------------------------------------------------------------
Name: other-session
Identifier: sync_Rp9exampleTwo
Alpha:
\tURL: /home/candidate/other
Beta:
\tURL: someone@example-host:22:/workspace
Status: Scanning files
--------------------------------------------------------------------------------
Name: pairproof-sync-broken
Identifier: sync_Rp9exampleThree
--------------------------------------------------------------------------------
";

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

#[test]
fn sync_create_argv_exact() {
    let endpoint = Endpoint::new("203.0.113.10", 2222).expect("endpoint");
    let user = EphemeralUsername::parse("pp-abcd1234").expect("username");
    let spec = sync_create("s1", "/home/candidate/session-s1", &endpoint, &user).expect("create");
    assert_eq!(spec.program, "mutagen");
    let mut expected = vec![
        "sync",
        "create",
        "--name",
        "pairproof-sync-s1",
        "--mode",
        "two-way-safe",
        "--symlink-mode",
        "ignore",
    ];
    expected.extend(pairproof::sync::SYNC_IGNORE_ARGS);
    expected.extend([
        "/home/candidate/session-s1",
        "pp-abcd1234@203.0.113.10:2222:/workspace",
    ]);
    assert_eq!(spec.args, strings(&expected));
    assert_eq!(
        mutagen_version().expect("version").args,
        strings(&["version"])
    );
    assert_eq!(sync_list().expect("list").args, strings(&["sync", "list"]));
    assert_eq!(
        sync_flush("s1").expect("flush").args,
        strings(&["sync", "flush", "pairproof-sync-s1"])
    );
    assert_eq!(
        sync_terminate("s1").expect("terminate").args,
        strings(&["sync", "terminate", "pairproof-sync-s1"])
    );
}

#[test]
fn beta_url_from_phase6_types() {
    let endpoint = Endpoint::new("expert.example-host", 2200).expect("endpoint");
    let user = EphemeralUsername::parse("pp-xyz98765").expect("username");
    assert_eq!(
        beta_url(&endpoint, &user),
        "pp-xyz98765@expert.example-host:2200:/workspace"
    );
    assert_eq!(sync_session_name("s1").expect("name"), "pairproof-sync-s1");
}

#[test]
fn alpha_dir_validation_rejects_relative_and_flaglike() {
    assert!(validate_alpha_dir("/home/candidate/session-s1").is_ok());
    for bad in [
        "",
        "relative/dir",
        "./dir",
        "-flag",
        "--mode=x",
        "/bad\npath",
        "/bad\u{1}path",
    ] {
        assert!(validate_alpha_dir(bad).is_err(), "should reject {bad:?}");
    }
    assert!(validate_alpha_dir(&format!("/{}", "a".repeat(512))).is_err());
}

#[test]
fn parse_sync_list_fixture() {
    let sessions = parse_sync_list(LIST_FIXTURE);
    assert_eq!(
        sessions,
        vec![
            SyncSessionInfo {
                name: "pairproof-sync-s1".to_string(),
                status: "Watching for changes".to_string(),
                paused: false,
                conflicts: 0,
            },
            SyncSessionInfo {
                name: "other-session".to_string(),
                status: "Scanning files".to_string(),
                paused: false,
                conflicts: 0,
            },
            SyncSessionInfo {
                name: "pairproof-sync-broken".to_string(),
                status: "unknown".to_string(),
                paused: false,
                conflicts: 0,
            },
        ]
    );
    assert_eq!(
        parse_sync_names(LIST_FIXTURE),
        strings(&[
            "pairproof-sync-s1",
            "other-session",
            "pairproof-sync-broken"
        ])
    );
    let found = find_sync(LIST_FIXTURE, "pairproof-sync-s1").expect("found");
    assert_eq!(found.status, "Watching for changes");
    assert_eq!(find_sync(LIST_FIXTURE, "missing"), None);
    assert!(parse_sync_list("").is_empty());
    assert!(parse_sync_list("No sessions found\n").is_empty());
}

#[test]
fn parse_mutagen_version_fixture() {
    assert_eq!(
        parse_mutagen_version("0.18.1\n").expect("version"),
        "0.18.1"
    );
    assert!(parse_mutagen_version("").is_err());
    assert!(parse_mutagen_version("   \n").is_err());
}

#[test]
fn sync_builders_reject_invalid_session_ids() {
    let endpoint = Endpoint::new("203.0.113.10", 2222).expect("endpoint");
    let user = EphemeralUsername::parse("pp-abcd1234").expect("username");
    for bad in ["--privileged", "a/b", "x;rm", "", "a b"] {
        assert!(
            matches!(sync_session_name(bad), Err(SyncError::InvalidSessionId(_))),
            "name {bad:?}"
        );
        assert!(
            sync_create(bad, "/tmp/alpha", &endpoint, &user).is_err(),
            "create {bad:?}"
        );
        assert!(sync_flush(bad).is_err(), "flush {bad:?}");
        assert!(sync_terminate(bad).is_err(), "terminate {bad:?}");
    }
    assert!(sync_create("s1", "relative", &endpoint, &user).is_err());
}

#[test]
fn alpha_dir_rejects_root_traversal_dot_and_colon() {
    assert!(validate_alpha_dir("/home/user/proj").is_ok());
    for bad in [
        "/",
        "/tmp/../etc",
        "/tmp/a:b",
        "/tmp/./x",
        "/tmp//x",
        "/tmp/x/",
        "/..",
        "/.",
        "/tmp/..",
        "//tmp/x",
        "/tmp/x:",
    ] {
        assert!(validate_alpha_dir(bad).is_err(), "should reject {bad:?}");
    }
}

#[test]
fn resume_and_template_builder_argv() {
    assert_eq!(
        sync_resume("s1").expect("resume").args,
        strings(&["sync", "resume", "pairproof-sync-s1"])
    );
    assert_eq!(
        sync_list_template().expect("template").args,
        strings(&["sync", "list", "--template", STATUS_TEMPLATE])
    );
    assert!(STATUS_TEMPLATE.contains("{{range .}}"));
    assert!(STATUS_TEMPLATE.contains("{{if .SessionState}}"));
    for bad in ["a/b", "net-x"] {
        assert!(sync_resume(bad).is_err(), "resume {bad:?}");
    }
}

#[test]
fn sync_error_converts_sandbox_error() {
    let err: SyncError = SandboxError::DockerNotFound.into();
    assert_eq!(err, SyncError::Sandbox(SandboxError::DockerNotFound));
}
