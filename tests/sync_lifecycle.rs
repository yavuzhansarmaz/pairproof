use std::path::Path;
use std::time::{Duration, Instant};

use pairproof::connection::{Endpoint, EphemeralUsername};
use pairproof::sandbox::process::{run_bounded, CommandOutcome, CommandSpec, DEFAULT_MAX_OUTPUT};
use pairproof::sync::{
    check_mutagen_available, create_sync, flush_sync, sync_status, terminate_sync, SyncError,
};

fn creds() -> (Endpoint, EphemeralUsername) {
    (
        Endpoint::new("203.0.113.10", 2222).expect("endpoint"),
        EphemeralUsername::parse("pp-abcd1234").expect("user"),
    )
}

#[tokio::test]
async fn runners_reject_invalid_session_id_before_spawn() {
    let (endpoint, user) = creds();
    for bad in ["--privileged", "a/b", "x;rm", "", "a b", "net-abc"] {
        assert!(
            matches!(
                create_sync(bad, "/tmp/alpha", &endpoint, &user).await,
                Err(SyncError::InvalidSessionId(_))
            ),
            "create {bad:?}"
        );
        assert!(
            matches!(flush_sync(bad).await, Err(SyncError::InvalidSessionId(_))),
            "flush {bad:?}"
        );
        assert!(
            matches!(
                terminate_sync(bad).await,
                Err(SyncError::InvalidSessionId(_))
            ),
            "terminate {bad:?}"
        );
        assert!(
            matches!(sync_status(bad).await, Err(SyncError::InvalidSessionId(_))),
            "status {bad:?}"
        );
    }
}

#[tokio::test]
async fn create_rejects_bad_alpha_before_spawn() {
    let (endpoint, user) = creds();
    for bad in ["relative/dir", "/", "/tmp/../etc", "/tmp/a:b", ""] {
        assert!(
            matches!(
                create_sync("s1", bad, &endpoint, &user).await,
                Err(SyncError::InvalidEndpoint(_))
            ),
            "alpha {bad:?}"
        );
    }
}

#[test]
fn mutagen_not_found_display_is_distinct() {
    let missing = SyncError::MutagenNotFound;
    assert_eq!(missing.to_string(), "mutagen executable not found");
    let others = [
        SyncError::CommandFailed {
            operation: "op",
            exit_code: Some(1),
            stderr: "x".into(),
        },
        SyncError::Timeout {
            program: "mutagen".into(),
            timeout_ms: 1,
        },
        SyncError::InvalidSessionId("s".into()),
        SyncError::InvalidEndpoint("e".into()),
        SyncError::ParseError {
            operation: "op",
            detail: "d".into(),
        },
    ];
    for other in &others {
        assert_ne!(&missing, other);
        assert_ne!(missing.to_string(), other.to_string());
    }
}

// Live tests below need mutagen on PATH; they touch only pid-derived names
// created here (plus guaranteed-absent probe ids) and clean up afterwards.
async fn run_async(args: Vec<String>) -> Result<CommandOutcome, String> {
    let spec = CommandSpec::new("mutagen", args);
    run_bounded(&spec, None, Duration::from_secs(30), DEFAULT_MAX_OUTPUT)
        .await
        .map_err(|e| e.to_string())
}

async fn run_ok(args: Vec<String>, op: &str) -> Result<String, String> {
    let outcome = run_async(args).await?;
    if outcome.success() {
        Ok(outcome.stdout_text())
    } else {
        Err(format!(
            "{op} exit {:?}: {}",
            outcome.exit_code,
            outcome.stderr_text()
        ))
    }
}

fn absent_id(tag: &str) -> String {
    format!("liveabsent{}{}", std::process::id(), tag)
}

#[tokio::test]
#[ignore]
async fn live_mutagen_available_reports_version() {
    let version = check_mutagen_available().await.expect("mutagen present");
    assert!(!version.trim().is_empty(), "version text must not be empty");
}

#[tokio::test]
#[ignore]
async fn live_terminate_missing_session_is_ok() {
    terminate_sync(&absent_id("t"))
        .await
        .expect("absent session terminates as Ok");
}

#[tokio::test]
#[ignore]
async fn live_status_missing_session_is_none() {
    assert_eq!(
        sync_status(&absent_id("s")).await.expect("status runs"),
        None
    );
}

async fn roundtrip(session_id: &str, alpha: &Path, beta: &Path) -> Result<(), String> {
    std::fs::create_dir_all(alpha).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(beta).map_err(|e| e.to_string())?;
    let name = format!("pairproof-sync-{session_id}");
    let alpha_s = alpha.display().to_string();
    let beta_s = beta.display().to_string();
    // Drop a stale session under our own probe name from an earlier run.
    let _ = terminate_sync(session_id).await;
    // Raw CLI only for create: the SSH beta needs interactive password
    // auth, so creation cannot go live non-interactively via the runner.
    run_ok(
        vec![
            "sync".into(),
            "create".into(),
            "--name".into(),
            name.clone(),
            "--mode".into(),
            "two-way-resolved".into(),
            "--symlink-mode".into(),
            "ignore".into(),
            alpha_s.clone(),
            beta_s.clone(),
        ],
        "create",
    )
    .await?;
    let start = Instant::now();
    loop {
        let present = sync_status(session_id)
            .await
            .map_err(|e| e.to_string())?
            .is_some();
        if present {
            break;
        }
        if start.elapsed() >= Duration::from_secs(10) {
            return Err(format!("session {name} never appeared in list"));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    std::fs::write(alpha.join("hello.txt"), "synced-content").map_err(|e| e.to_string())?;
    flush_sync(session_id).await.map_err(|e| e.to_string())?;
    let start = Instant::now();
    loop {
        let got = std::fs::read_to_string(beta.join("hello.txt")).unwrap_or_default();
        if got == "synced-content" {
            break;
        }
        if start.elapsed() >= Duration::from_secs(30) {
            return Err("file never appeared in beta".to_string());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    terminate_sync(session_id)
        .await
        .map_err(|e| e.to_string())?;
    let alpha_content =
        std::fs::read_to_string(alpha.join("hello.txt")).map_err(|e| e.to_string())?;
    let beta_content =
        std::fs::read_to_string(beta.join("hello.txt")).map_err(|e| e.to_string())?;
    if alpha_content != "synced-content" || beta_content != "synced-content" {
        return Err("terminate removed or altered local files".to_string());
    }
    let gone = sync_status(session_id).await.map_err(|e| e.to_string())?;
    if gone.is_some() {
        return Err(format!("session {name} still listed after terminate"));
    }
    Ok(())
}

#[tokio::test]
#[ignore]
async fn live_two_directory_sync_roundtrip() {
    let pid = std::process::id();
    let session_id = format!("rt{pid}");
    let base = std::env::temp_dir().join(format!("pairproof-sync-rt{pid}"));
    let result = roundtrip(&session_id, &base.join("alpha"), &base.join("beta")).await;
    // Terminate our own probe session even on failure; then drop temp dirs.
    let _ = terminate_sync(&session_id).await;
    std::fs::remove_dir_all(&base).ok();
    result.expect("roundtrip");
}

#[tokio::test]
#[ignore]
async fn live_create_sync_bogus_endpoint_fails_typed() {
    let pid = std::process::id();
    let session_id = format!("bogus{pid}");
    let alpha = std::env::temp_dir().join(format!("pairproof-alpha-{pid}"));
    std::fs::create_dir_all(&alpha).expect("alpha dir");
    let endpoint = Endpoint::new("pairproof-nonexistent.invalid", 2222).expect("endpoint");
    let user = EphemeralUsername::generate();
    let start = Instant::now();
    let result = create_sync(&session_id, &alpha.display().to_string(), &endpoint, &user).await;
    let elapsed = start.elapsed();
    // Cleanup even on failure: idempotent terminate plus temp dir removal.
    let terminated = terminate_sync(&session_id).await;
    std::fs::remove_dir_all(&alpha).ok();
    assert!(
        matches!(result, Err(SyncError::CommandFailed { .. })),
        "bogus endpoint must fail typed, got {result:?}"
    );
    assert!(
        terminated.is_ok(),
        "cleanup terminate stays idempotent Ok, got {terminated:?}"
    );
    assert!(
        elapsed < Duration::from_secs(60),
        "must fail well within timeout, took {elapsed:?}"
    );
}
