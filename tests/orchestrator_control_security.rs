use pairproof::orchestration::*;
use std::path::PathBuf;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pairproof-ctrlsec-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[tokio::test]
async fn candidate_cannot_modify_outbox_snapshot() {
    use std::os::unix::fs::PermissionsExt;
    let root = tmp("outbox");
    let io = LocalControlIo::new(root.clone());
    io.setup_dirs("pp-testuser").await.unwrap();
    let snap = build_snapshot(
        &{
            let mut o = Orchestrator::create(
                "s1".into(),
                pairproof::core::Language::Python,
                (0..5)
                    .map(|i| pairproof::core::QuestionRef {
                        title_slug: format!("e{i}"),
                        title: format!("e{i}"),
                        difficulty: pairproof::leetcode::Difficulty::Easy,
                        acceptance_pct: None,
                    })
                    .collect(),
                0,
            )
            .unwrap();
            o.mark_waiting_for_candidate(1).unwrap();
            o.attach_candidate(2).unwrap();
            o.start(3).unwrap();
            o
        },
        &QuestionPipeline::new(),
        "Sync: active",
        &[],
        None,
    );
    publish_snapshot(&io, &snap).await.unwrap();
    let target = root.join(SNAPSHOT_REL);
    let before = std::fs::read(&target).unwrap();
    // Permission model of the real dirs: root-owned outbox 0755, files 0644,
    // candidate owns only inbox/. Replicated here with read-only modes.
    std::fs::set_permissions(root.join("outbox"), std::fs::Permissions::from_mode(0o555)).unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o444)).unwrap();
    assert!(
        std::fs::write(&target, b"hacked").is_err(),
        "overwrite must fail"
    );
    assert!(std::fs::remove_file(&target).is_err(), "delete must fail");
    let staging = root.join("outbox").join("staging.json");
    assert!(
        std::fs::rename(&staging, &target).is_err() || std::fs::read(&target).unwrap() == before
    );
    std::fs::set_permissions(root.join("outbox"), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), before);
    assert_eq!(parse_snapshot(&before, "s1").unwrap().session_id, "s1");
    // The candidate client's only write target is the inbox intent log.
    assert_eq!(
        pairproof::orchestration::control_client::REMOTE_INTENT_PATH,
        "/pairproof-control/inbox/intents.jsonl"
    );
    assert_eq!(
        pairproof::orchestration::control_client::REMOTE_SNAPSHOT_PATH,
        "/pairproof-control/outbox/snapshot.json"
    );
}

#[tokio::test]
async fn expert_reader_rejects_symlink_escape() {
    let root = tmp("symlink");
    let io = LocalControlIo::new(root.clone());
    io.setup_dirs("pp-testuser").await.unwrap();
    let outside = tmp("outside-target");
    let secret_file = outside.join("secret.txt");
    std::fs::write(&secret_file, "hidden grading internals").unwrap();
    let link = root.join(INTENT_REL);
    std::os::unix::fs::symlink(&secret_file, &link).unwrap();
    assert!(matches!(
        resolve_control_file(&root, INTENT_REL),
        Err(ControlError::SymlinkRejected(_))
    ));
    assert!(matches!(
        io.read_intents().await,
        Err(ControlError::SymlinkRejected(_))
    ));
    let snap_link = root.join(SNAPSHOT_REL);
    std::os::unix::fs::symlink(&secret_file, &snap_link).unwrap();
    assert!(matches!(
        resolve_control_file(&root, SNAPSHOT_REL),
        Err(ControlError::SymlinkRejected(_))
    ));
}

#[test]
fn expert_reader_rejects_path_traversal() {
    let root = tmp("traversal");
    std::fs::create_dir_all(root.join("inbox")).unwrap();
    for rel in [
        "../outside.json",
        "inbox/../../etc/passwd",
        "/etc/passwd",
        "inbox/other.jsonl",
        "outbox/../processed.json",
        "..\\windows",
    ] {
        assert!(
            matches!(
                resolve_control_file(&root, rel),
                Err(ControlError::PathEscape(_))
            ),
            "{rel}"
        );
    }
    assert!(resolve_control_file(&root, INTENT_REL).is_ok());
    assert!(resolve_control_file(&root, SNAPSHOT_REL).is_ok());
    assert!(resolve_control_file(&root, PROCESSED_FILE).is_ok());
}

#[test]
fn snapshot_reader_rejects_wrong_session_and_version() {
    let mut orch = Orchestrator::create(
        "s1".into(),
        pairproof::core::Language::Python,
        (0..5)
            .map(|i| pairproof::core::QuestionRef {
                title_slug: format!("e{i}"),
                title: format!("e{i}"),
                difficulty: pairproof::leetcode::Difficulty::Easy,
                acceptance_pct: None,
            })
            .collect(),
        0,
    )
    .unwrap();
    orch.mark_waiting_for_candidate(1).unwrap();
    orch.attach_candidate(2).unwrap();
    orch.start(3).unwrap();
    let snap = build_snapshot(&orch, &QuestionPipeline::new(), "Sync: active", &[], None);
    let bytes = serde_json::to_vec(&snap).unwrap();
    assert_eq!(
        parse_snapshot(&bytes, "other"),
        Err(ControlError::WrongSession)
    );
    let client = CandidateControlClient::new("other");
    assert_eq!(
        client.read_snapshot(&bytes),
        Err(ControlError::WrongSession)
    );
    let mut tampered = snap.clone();
    tampered.v = 99;
    let tampered_bytes = serde_json::to_vec(&tampered).unwrap();
    assert_eq!(
        parse_snapshot(&tampered_bytes, "s1"),
        Err(ControlError::WrongVersion(99))
    );
}

#[test]
fn intent_text_travels_as_stdin_data_never_argv() {
    use pairproof::connection::{Endpoint, EphemeralUsername, Secret};
    use std::time::Duration;
    let session = AskpassSession::create("ctrlargv").unwrap();
    let endpoint = Endpoint::new("127.0.0.1", 2222).unwrap();
    let user = EphemeralUsername::parse("pp-a1b2c3d4").unwrap();
    let intent = Intent::new(
        "s1",
        1,
        IntentKind::Redirect {
            text: "use a heap; rm -rf / $(reboot)".into(),
        },
    );
    let invocation = ssh_invocation(
        &session,
        &endpoint,
        &user,
        &["tee", "-a", "/pairproof-control/inbox/intents.jsonl"],
        Duration::from_secs(5),
    );
    let argv = invocation.args.join(" ");
    assert!(!argv.contains("rm -rf"), "{argv}");
    assert!(!argv.contains("reboot"), "{argv}");
    assert!(
        argv.contains("tee -a /pairproof-control/inbox/intents.jsonl"),
        "{argv}"
    );
    let line = CandidateControlClient::intent_line(&intent).unwrap();
    assert!(
        line.contains("use a heap"),
        "payload stays in stdin line: {line}"
    );
    let _secret = Secret::generate();
}

#[tokio::test]
async fn processed_ledger_rejects_wrong_session_on_restore() {
    let root = tmp("ledger");
    let io = LocalControlIo::new(root.clone());
    io.setup_dirs("pp-testuser").await.unwrap();
    io.store_processed(&ProcessedState::new("s1"))
        .await
        .unwrap();
    assert!(ControlExecutor::restore("s1", &io).await.is_ok());
    assert_eq!(
        ControlExecutor::restore("other", &io).await.unwrap_err(),
        ControlError::WrongSession
    );
}
