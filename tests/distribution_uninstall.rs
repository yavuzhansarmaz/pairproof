use std::path::{Path, PathBuf};

use clap::Parser;
use pairproof::distribution::uninstall::{
    build_plan, execute_plan, run_uninstall_at, uninstall_with_io, PlanEntry, PlanKind, RemovalPlan,
};
use pairproof::distribution::DistributionError;
use pairproof::tui::args::{Args, Command};

struct Fixture {
    root: PathBuf,
    home: PathBuf,
    exe: PathBuf,
    config_dir: PathBuf,
    api_key: PathBuf,
}

fn unique_dir(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "pairproof-uninstall-{name}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn fixture(name: &str) -> Fixture {
    let root = unique_dir(name);
    let home = root.join("home");
    let exe = root.join("bin").join("pairproof");
    let config_dir = home.join(".config").join("pairproof");
    let api_key = config_dir.join("api_key");
    std::fs::create_dir_all(exe.parent().expect("exe parent")).expect("exe dir");
    std::fs::create_dir_all(&config_dir).expect("config dir");
    std::fs::write(&exe, b"fake-binary").expect("write exe");
    std::fs::write(&api_key, b"secret-key").expect("write api key");
    Fixture {
        root,
        home,
        exe,
        config_dir,
        api_key,
    }
}

fn cleanup(fx: &Fixture) {
    let _ = std::fs::remove_dir_all(&fx.root);
}

#[test]
fn uninstall_lists_all_owned_paths() {
    let fx = fixture("lists");
    let plan = build_plan(&fx.exe, &fx.home, false);
    let paths: Vec<&Path> = plan.entries.iter().map(|e| e.path.as_path()).collect();
    assert_eq!(
        paths,
        vec![
            fx.exe.as_path(),
            fx.api_key.as_path(),
            fx.config_dir.as_path()
        ]
    );
    assert_eq!(plan.entries[0].kind, PlanKind::Binary);
    assert_eq!(plan.entries[1].kind, PlanKind::ApiKey);
    assert_eq!(plan.entries[2].kind, PlanKind::ConfigDir);
    assert!(plan.entries.iter().all(|e| !e.description.is_empty()));
    cleanup(&fx);
}

#[test]
fn uninstall_excludes_user_project_files() {
    let fx = fixture("excludes");
    let sessions = fx.home.join("sessions");
    let workspace = fx.home.join("pairproof-workspace");
    let artifacts = fx.home.join("artifacts");
    let dot = fx.home.join(".pairproof");
    for dir in [&sessions, &workspace, &artifacts, &dot] {
        std::fs::create_dir_all(dir).expect("user dir");
    }
    let conn = fx.home.join("connection-string.txt");
    let conn2 = fx.home.join("connection-string-2.txt");
    std::fs::write(&conn, b"conn").expect("conn");
    std::fs::write(&conn2, b"conn2").expect("conn2");
    std::fs::write(sessions.join("report.txt"), b"r").expect("report");
    let plan = build_plan(&fx.exe, &fx.home, false);
    for user in [&sessions, &workspace, &artifacts, &dot, &conn, &conn2] {
        assert!(
            plan.entries.iter().all(|e| e.path != **user),
            "user file in plan: {user:?}"
        );
    }
    execute_plan(&plan, true).expect("execute");
    for user in [&sessions, &workspace, &artifacts, &dot, &conn, &conn2] {
        assert!(user.exists(), "user file deleted: {user:?}");
    }
    cleanup(&fx);
}

#[test]
fn uninstall_requires_confirmation() {
    let fx = fixture("confirm");
    let plan = build_plan(&fx.exe, &fx.home, false);
    let result = execute_plan(&plan, false);
    assert!(result.is_err(), "expected error, got {result:?}");
    assert!(fx.exe.exists());
    assert!(fx.api_key.exists());
    assert!(fx.config_dir.exists());
    // Interactive path with a refusing confirmation also deletes nothing.
    let mut out: Vec<u8> = Vec::new();
    uninstall_with_io(&plan, false, false, true, &|| false, &mut out)
        .expect("abort is not an error");
    assert!(String::from_utf8_lossy(&out).contains("Aborted."));
    assert!(fx.exe.exists());
    assert!(fx.api_key.exists());
    cleanup(&fx);
}

#[test]
fn uninstall_yes_skips_confirmation() {
    let fx = fixture("yes");
    let plan = build_plan(&fx.exe, &fx.home, false);
    let mut out: Vec<u8> = Vec::new();
    let confirm = || -> bool { panic!("confirm must not be called with --yes") };
    uninstall_with_io(&plan, true, false, false, &confirm, &mut out).expect("yes flow");
    let text = String::from_utf8_lossy(&out);
    assert!(text.contains("Will remove:"), "plan printed: {text}");
    assert!(text.contains("PairProof uninstalled."), "final: {text}");
    assert!(!fx.exe.exists());
    assert!(!fx.config_dir.exists());
    cleanup(&fx);
}

#[test]
fn uninstall_keep_data_removes_only_binary() {
    let fx = fixture("keep-data");
    let plan = build_plan(&fx.exe, &fx.home, true);
    assert_eq!(plan.entries.len(), 1);
    assert_eq!(plan.entries[0].kind, PlanKind::Binary);
    execute_plan(&plan, true).expect("execute");
    assert!(!fx.exe.exists());
    assert!(fx.api_key.exists());
    assert!(fx.config_dir.exists());
    // Full CLI core also notes data is kept.
    let fx2 = fixture("keep-data-cli");
    let mut out: Vec<u8> = Vec::new();
    run_uninstall_at(
        &fx2.exe,
        &fx2.home,
        true,
        true,
        false,
        &|| panic!("no confirm"),
        &mut out,
    )
    .expect("keep-data flow");
    assert!(String::from_utf8_lossy(&out).contains("Keeping saved data"));
    assert!(!fx2.exe.exists());
    assert!(fx2.api_key.exists());
    cleanup(&fx);
    cleanup(&fx2);
}

#[test]
fn uninstall_removes_saved_api_key() {
    let fx = fixture("api-key");
    assert_eq!(std::fs::read(&fx.api_key).expect("read key"), b"secret-key");
    let plan = build_plan(&fx.exe, &fx.home, false);
    let report = execute_plan(&plan, true).expect("execute");
    assert!(!fx.api_key.exists());
    assert!(!fx.config_dir.exists());
    assert!(report.iter().any(|l| l.contains("api_key")));
    cleanup(&fx);
}

#[test]
fn uninstall_handles_missing_paths() {
    let root = unique_dir("missing");
    let home = root.join("home");
    let exe = root.join("bin").join("pairproof");
    let plan = build_plan(&exe, &home, false);
    let report = execute_plan(&plan, true).expect("missing is ok");
    assert_eq!(report.len(), 3);
    assert!(report.iter().all(|l| l.contains("Not present")));
    // Idempotent: a second run also succeeds.
    execute_plan(&plan, true).expect("second run ok");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn uninstall_rejects_unsafe_paths() {
    let fx = fixture("unsafe");
    let bad_exe = RemovalPlan {
        entries: vec![
            PlanEntry {
                path: PathBuf::from("/"),
                kind: PlanKind::Binary,
                description: "bad".to_string(),
            },
            PlanEntry {
                path: fx.api_key.clone(),
                kind: PlanKind::ApiKey,
                description: "key".to_string(),
            },
        ],
    };
    let result = execute_plan(&bad_exe, true);
    assert!(
        matches!(result, Err(DistributionError::UnsafePath { .. })),
        "expected UnsafePath, got {result:?}"
    );
    assert!(fx.api_key.exists(), "nothing may be deleted");
    let bad_dir = RemovalPlan {
        entries: vec![PlanEntry {
            path: fx.home.clone(),
            kind: PlanKind::ConfigDir,
            description: "bad".to_string(),
        }],
    };
    assert!(matches!(
        execute_plan(&bad_dir, true),
        Err(DistributionError::UnsafePath { .. })
    ));
    assert!(fx.home.exists());
    assert!(fx.api_key.exists());
    cleanup(&fx);
}

#[test]
#[cfg(unix)]
fn uninstall_rejects_symlink_escape() {
    let fx = fixture("symlink");
    // Replace the real config dir with a symlink to an external dir.
    std::fs::remove_dir_all(&fx.config_dir).expect("remove real config");
    let external = fx.root.join("external");
    std::fs::create_dir_all(&external).expect("external");
    let sentinel = external.join("sentinel.txt");
    std::fs::write(&sentinel, b"keep me").expect("sentinel");
    std::fs::write(external.join("api_key"), b"external-key").expect("ext key");
    std::os::unix::fs::symlink(&external, &fx.config_dir).expect("symlink config");
    let plan = build_plan(&fx.exe, &fx.home, false);
    let report = execute_plan(&plan, true).expect("symlink plan executes");
    assert!(sentinel.exists(), "external sentinel deleted");
    assert_eq!(std::fs::read(&sentinel).expect("read sentinel"), b"keep me");
    assert!(external.join("api_key").exists(), "external key deleted");
    assert!(
        fx.config_dir
            .symlink_metadata()
            .expect("link metadata")
            .file_type()
            .is_symlink(),
        "symlink itself must be left in place"
    );
    assert!(
        report.iter().any(|l| l.to_lowercase().contains("symlink")),
        "symlink handling reported: {report:?}"
    );
    cleanup(&fx);
}

#[test]
fn uninstall_has_no_delete_alias() {
    assert!(Args::try_parse_from(["pairproof", "delete"]).is_err());
    let parsed = Args::try_parse_from(["pairproof", "uninstall"]).expect("uninstall parses");
    match parsed.command {
        Some(Command::Uninstall(args)) => {
            assert!(!args.yes);
            assert!(!args.keep_data);
        }
        other => panic!("expected Uninstall, got {other:?}"),
    }
}

#[test]
fn uninstall_non_tty_requires_yes() {
    let fx = fixture("non-tty");
    let plan = build_plan(&fx.exe, &fx.home, false);
    let mut out: Vec<u8> = Vec::new();
    let result = uninstall_with_io(&plan, false, false, false, &|| true, &mut out);
    assert!(
        matches!(result, Err(DistributionError::NonInteractiveConfirmation)),
        "expected NonInteractiveConfirmation, got {result:?}"
    );
    assert!(result
        .expect_err("checked")
        .to_string()
        .contains("refusing to uninstall without confirmation"));
    assert!(fx.exe.exists());
    assert!(fx.api_key.exists());
    assert!(fx.config_dir.exists());
    cleanup(&fx);
}

#[test]
fn uninstall_args_parse_yes_and_keep_data() {
    let parsed = Args::try_parse_from(["pairproof", "uninstall", "--yes", "--keep-data"])
        .expect("flags parse");
    match parsed.command {
        Some(Command::Uninstall(args)) => {
            assert!(args.yes);
            assert!(args.keep_data);
        }
        other => panic!("expected Uninstall, got {other:?}"),
    }
    let plain = Args::try_parse_from(["pairproof", "update"]).expect("update still parses");
    assert!(matches!(plain.command, Some(Command::Update(_))));
}
