use pairproof::connection::{Endpoint, EphemeralUsername, Secret};
use pairproof::core::Language;
use pairproof::sandbox::error::SandboxError;
use pairproof::sandbox::lifecycle::{
    create_and_provision, destroy, destroy_by_session_id, SandboxHandle,
};
use pairproof::sandbox::validate::{WorkspacePath, MAX_FILE_BYTES};
use pairproof::sandbox::workspace::{read_file, write_file, write_question};

fn handle() -> SandboxHandle {
    let endpoint = Endpoint::new("127.0.0.1", 2222).expect("endpoint");
    SandboxHandle::ids("ws1", &endpoint).expect("handle")
}

fn user() -> EphemeralUsername {
    EphemeralUsername::parse("pp-abcd1234").expect("user")
}

#[tokio::test]
async fn read_file_missing_container_is_infra_error_not_notfound() {
    let session_id = format!("noexist{:x}", std::process::id());
    let endpoint = Endpoint::new("127.0.0.1", 2222).expect("endpoint");
    let handle = SandboxHandle::ids(&session_id, &endpoint).expect("handle");
    let path = WorkspacePath::new("questions/01/solution.py").expect("path");
    let err = read_file(&handle, &user(), &path)
        .await
        .expect_err("read in a missing container must fail");
    assert!(
        !matches!(err, SandboxError::ContainerFileNotFound(_)),
        "dead container must not look like a missing file: {err:?}"
    );
}

#[tokio::test]
async fn write_file_rejects_oversize_before_docker() {
    let content = vec![b'x'; MAX_FILE_BYTES + 1];
    let path = WorkspacePath::new("questions/01/solution.py").expect("path");
    let err = write_file(&handle(), &user(), &path, &content)
        .await
        .expect_err("oversize write must fail");
    assert_eq!(
        err,
        SandboxError::FileTooLarge {
            len: MAX_FILE_BYTES + 1,
            max: MAX_FILE_BYTES,
        }
    );
}

#[tokio::test]
async fn write_question_rejects_oversize_statement_before_docker() {
    let statement = "x".repeat(MAX_FILE_BYTES + 1);
    let err = write_question(&handle(), &user(), 0, Language::Python, &statement)
        .await
        .expect_err("oversize statement must fail");
    assert_eq!(
        err,
        SandboxError::FileTooLarge {
            len: MAX_FILE_BYTES + 1,
            max: MAX_FILE_BYTES,
        }
    );
}

#[test]
fn invalid_workspace_path_rejected_at_construction() {
    for bad in [
        "",
        "/abs/path",
        "a/../b",
        "..",
        "a/./b",
        "a//b",
        "a\\b",
        "a\x00b",
        "questions/01/",
    ] {
        assert!(
            WorkspacePath::new(bad).is_err(),
            "constructor must reject {bad:?}"
        );
    }
}

fn live_session_id(tag: &str) -> String {
    format!("ws{}w{tag}", std::process::id())
}

fn free_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

async fn live_handle(session_id: &str) -> (SandboxHandle, EphemeralUsername) {
    let endpoint = Endpoint::new("127.0.0.1", free_port()).expect("endpoint");
    let user = EphemeralUsername::generate();
    let secret = Secret::generate();
    match create_and_provision(session_id, &endpoint, &user, &secret).await {
        Ok((handle, _)) => (handle, user),
        Err(err) => {
            let _ = destroy_by_session_id(session_id).await;
            panic!("create_and_provision failed: {err}");
        }
    }
}

#[tokio::test]
#[ignore = "live docker: write/read roundtrip with unicode in own container"]
async fn live_write_read_roundtrip_unicode() {
    let session_id = live_session_id("u");
    let (handle, user) = live_handle(&session_id).await;
    let outcome = async {
        let path = WorkspacePath::new("questions/01/solution.py").expect("path");
        let content = "# héllo 🦀\nprint('ünïcode ✓')\n".as_bytes();
        write_file(&handle, &user, &path, content)
            .await
            .map_err(|e| e.to_string())?;
        let back = read_file(&handle, &user, &path)
            .await
            .map_err(|e| e.to_string())?;
        if back != content {
            return Err("roundtrip bytes differ".to_string());
        }
        let missing = WorkspacePath::new("questions/01/absent.py").expect("path");
        match read_file(&handle, &user, &missing).await {
            Err(SandboxError::ContainerFileNotFound(_)) => Ok(()),
            Err(other) => Err(format!("wrong error for missing file: {other}")),
            Ok(_) => Err("missing file read must fail".to_string()),
        }
    }
    .await;
    let _ = destroy(&handle).await;
    assert!(outcome.is_ok(), "{outcome:?}");
}

#[tokio::test]
#[ignore = "live docker: symlink-to-/etc parent dir must be refused"]
async fn live_symlink_escape_rejected() {
    use pairproof::sandbox::exec::run_in_container;
    use std::time::Duration;

    let session_id = live_session_id("s");
    let (handle, user) = live_handle(&session_id).await;
    let outcome = async {
        let link = vec![
            "python3".to_string(),
            "-c".to_string(),
            "import os; os.symlink('/etc', '/workspace/evil')".to_string(),
        ];
        let linked = run_in_container(&handle, &user, &link, None, Duration::from_secs(30))
            .await
            .map_err(|e| e.to_string())?;
        if !linked.success() {
            return Err(format!("symlink setup failed: {linked:?}"));
        }
        let evil = WorkspacePath::new("evil/passwd").expect("path");
        match write_file(&handle, &user, &evil, b"data").await {
            Err(SandboxError::PathEscape(_)) => Ok(()),
            Err(other) => Err(format!("wrong error for symlink escape: {other}")),
            Ok(()) => Err("symlink escape write must fail".to_string()),
        }
    }
    .await;
    let _ = destroy(&handle).await;
    assert!(outcome.is_ok(), "{outcome:?}");
}

#[tokio::test]
#[ignore = "live docker: existing solution file must survive write_question"]
async fn live_solution_not_overwritten() {
    let session_id = live_session_id("o");
    let (handle, user) = live_handle(&session_id).await;
    let outcome = async {
        let existing = b"print('candidate')\n";
        let statement = "Add one.";
        let (question, solution) = write_question(&handle, &user, 0, Language::Python, "draft")
            .await
            .map_err(|e| e.to_string())?;
        write_file(&handle, &user, &solution, existing)
            .await
            .map_err(|e| e.to_string())?;
        let (question2, solution2) = write_question(&handle, &user, 0, Language::Python, statement)
            .await
            .map_err(|e| e.to_string())?;
        if question != question2 || solution != solution2 {
            return Err("layout paths changed".to_string());
        }
        let kept = read_file(&handle, &user, &solution)
            .await
            .map_err(|e| e.to_string())?;
        if kept != existing {
            return Err("candidate solution was overwritten".to_string());
        }
        let asked = read_file(&handle, &user, &question)
            .await
            .map_err(|e| e.to_string())?;
        if asked != statement.as_bytes() {
            return Err("question text mismatch".to_string());
        }
        Ok(())
    }
    .await;
    let _ = destroy(&handle).await;
    assert!(outcome.is_ok(), "{outcome:?}");
}

#[tokio::test]
#[ignore = "live docker: oversized-but-allowed file must fail explicit, not truncated"]
async fn live_read_large_file_explicit_error() {
    use pairproof::sandbox::process::EXEC_MAX_OUTPUT;

    let session_id = live_session_id("l");
    let (handle, user) = live_handle(&session_id).await;
    let outcome = async {
        let path = WorkspacePath::new("large/big.bin").expect("path");
        let content = vec![b'a'; 300_000];
        write_file(&handle, &user, &path, &content)
            .await
            .map_err(|e| e.to_string())?;
        match read_file(&handle, &user, &path).await {
            Err(SandboxError::FileTooLarge { len, max }) => {
                if len == 300_000 && max == EXEC_MAX_OUTPUT {
                    Ok(())
                } else {
                    Err(format!("wrong FileTooLarge bounds: len={len} max={max}"))
                }
            }
            Err(other) => Err(format!("wrong error for large file: {other}")),
            Ok(_) => Err("large file read must fail explicitly".to_string()),
        }
    }
    .await;
    let _ = destroy(&handle).await;
    assert!(outcome.is_ok(), "{outcome:?}");
}
