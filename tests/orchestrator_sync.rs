use std::path::PathBuf;
use std::time::{Duration, Instant};

use pairproof::connection::{Endpoint, EphemeralUsername, Secret};
use pairproof::orchestration::sync_gate::{
    ensure_session_sync, flush_session_sync, session_sync_phase, sync_phase_for,
    terminate_session_sync, SessionSyncPhase, CANDIDATE_SYNC_MESSAGE,
};
use pairproof::sandbox::process::{run_bounded, CommandSpec, DEFAULT_MAX_OUTPUT};
use pairproof::sync::command::sync_create;
use pairproof::sync::{SyncError, SyncSessionInfo};

fn endpoint() -> Endpoint {
    Endpoint::new("203.0.113.10", 2222).expect("endpoint")
}

fn user() -> EphemeralUsername {
    EphemeralUsername::parse("pp-abcd1234").expect("user")
}

fn absent_id(tag: &str) -> String {
    format!("gateabsent{}{}", std::process::id(), tag)
}

fn temp_base(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("pairproof-gate-{tag}-{}", std::process::id()))
}

#[test]
fn mutagen_uses_secret_without_argv_or_environment_leak() {
    let canary = "deadbeefcafebabe0123456789abcdef";
    let secret = Secret::from_hex(canary).expect("canary secret");
    let spec =
        sync_create("s1", "/home/candidate/session-s1", &endpoint(), &user()).expect("sync create");
    let joined = spec.args.join(" ");
    assert!(!joined.contains(canary));
    assert!(!spec.describe().contains(canary));
    assert!(spec
        .args
        .iter()
        .all(|arg| !arg.contains(&secret.expose_hex())));
    assert_eq!(spec.program, "mutagen");
    // The beta URL carries only the ephemeral user and endpoint, no password.
    assert!(spec
        .args
        .contains(&"pp-abcd1234@203.0.113.10:2222:/workspace".to_string()));
}

#[tokio::test]
async fn candidate_sync_starts_two_way_safe_session() {
    let spec =
        sync_create("s1", "/home/candidate/session-s1", &endpoint(), &user()).expect("sync create");
    assert!(spec.args.contains(&"two-way-safe".to_string()));
    assert!(spec.args.contains(&"sync".to_string()));
    assert!(spec.args.contains(&"create".to_string()));
    // Invalid session ids and alpha dirs fail typed before any process spawn.
    let err = ensure_session_sync("a/b", "/tmp/alpha", &endpoint(), &user())
        .await
        .expect_err("invalid session id");
    assert_eq!(
        err.candidate_message(),
        "Sync problem \u{2014} ask the expert"
    );
    assert_eq!(format!("{err}"), CANDIDATE_SYNC_MESSAGE);
    assert!(matches!(err.expert_error(), SyncError::InvalidSessionId(_)));
    let err = ensure_session_sync("s1", "relative/dir", &endpoint(), &user())
        .await
        .expect_err("invalid alpha");
    assert_eq!(err.candidate_message(), CANDIDATE_SYNC_MESSAGE);
    assert!(matches!(err.expert_error(), SyncError::InvalidEndpoint(_)));
}

#[tokio::test]
async fn candidate_sync_flush_and_status_are_real() {
    // Pure mapping boundary over real lifecycle output shapes.
    let watching = SyncSessionInfo {
        name: "pairproof-sync-s1".to_string(),
        status: "Watching".to_string(),
        paused: false,
        conflicts: 0,
    };
    assert_eq!(sync_phase_for(Some(&watching)), SessionSyncPhase::Active);
    assert_eq!(sync_phase_for(None), SessionSyncPhase::NotStarted);
    // The wrappers really delegate: invalid ids surface the typed lifecycle
    // error at expert level and only the safe message at candidate level.
    let err = flush_session_sync("a/b").await.expect_err("flush invalid");
    assert_eq!(err.candidate_message(), CANDIDATE_SYNC_MESSAGE);
    assert!(matches!(err.expert_error(), SyncError::InvalidSessionId(_)));
    let err = session_sync_phase("a/b").await.expect_err("status invalid");
    assert_eq!(err.candidate_message(), CANDIDATE_SYNC_MESSAGE);
    assert!(matches!(err.expert_error(), SyncError::InvalidSessionId(_)));
    // A valid but absent session reports NotStarted through the real runner.
    assert_eq!(
        session_sync_phase(&absent_id("st"))
            .await
            .expect("absent status"),
        SessionSyncPhase::NotStarted
    );
}

#[test]
fn candidate_sync_conflict_is_not_reported_as_clean() {
    let conflicted = SyncSessionInfo {
        name: "pairproof-sync-s1".to_string(),
        status: "Watching".to_string(),
        paused: false,
        conflicts: 3,
    };
    let phase = sync_phase_for(Some(&conflicted));
    assert_eq!(phase, SessionSyncPhase::Conflict);
    assert_ne!(phase, SessionSyncPhase::Active);
    assert_eq!(
        phase.label(),
        "Sync: conflict needs attention \u{2014} ask the expert"
    );
    // Paused plus conflicted still reports the conflict.
    let paused_conflict = SyncSessionInfo {
        paused: true,
        ..conflicted.clone()
    };
    assert_eq!(
        sync_phase_for(Some(&paused_conflict)),
        SessionSyncPhase::Conflict
    );
    let paused_clean = SyncSessionInfo {
        name: "pairproof-sync-s1".to_string(),
        status: "Watching".to_string(),
        paused: true,
        conflicts: 0,
    };
    assert_eq!(
        sync_phase_for(Some(&paused_clean)),
        SessionSyncPhase::Paused
    );
}

#[tokio::test]
async fn sync_termination_preserves_candidate_local_files() {
    let base = temp_base("term");
    std::fs::create_dir_all(&base).expect("temp dir");
    let file = base.join("candidate.txt");
    std::fs::write(&file, "work in progress").expect("write candidate file");
    terminate_session_sync(&absent_id("tp"))
        .await
        .expect("absent terminate");
    assert_eq!(
        std::fs::read_to_string(&file).expect("candidate file remains"),
        "work in progress"
    );
    // Even a typed failure leaves local files untouched.
    let _ = terminate_session_sync("a/b").await;
    assert_eq!(
        std::fs::read_to_string(&file).expect("candidate file still remains"),
        "work in progress"
    );
    std::fs::remove_dir_all(&base).ok();
}

#[tokio::test]
async fn sync_cleanup_is_idempotent() {
    let id = absent_id("idem");
    terminate_session_sync(&id).await.expect("first terminate");
    terminate_session_sync(&id).await.expect("second terminate");
    assert_eq!(
        session_sync_phase(&id).await.expect("status after cleanup"),
        SessionSyncPhase::NotStarted
    );
}

#[tokio::test]
#[ignore]
async fn live_mutagen_two_way_sync_round_trip() {
    let session_id = format!("rtgate{}", std::process::id());
    let base = temp_base("live");
    let alpha = base.join("alpha");
    let beta = base.join("beta");
    std::fs::create_dir_all(&alpha).expect("alpha dir");
    std::fs::create_dir_all(&beta).expect("beta dir");
    let name = format!("pairproof-sync-{session_id}");
    let _ = terminate_session_sync(&session_id).await;
    // Local-to-local create exercises Mutagen without the blocked SSH login.
    let spec = CommandSpec::new(
        "mutagen",
        vec![
            "sync".to_string(),
            "create".to_string(),
            "--name".to_string(),
            name,
            "--mode".to_string(),
            "two-way-safe".to_string(),
            "--symlink-mode".to_string(),
            "ignore".to_string(),
            alpha.display().to_string(),
            beta.display().to_string(),
        ],
    );
    let outcome = run_bounded(&spec, None, Duration::from_secs(30), DEFAULT_MAX_OUTPUT)
        .await
        .expect("mutagen create runs");
    assert!(
        outcome.success(),
        "create failed: {}",
        outcome.stderr_text()
    );
    let result = async {
        let start = Instant::now();
        loop {
            let phase = session_sync_phase(&session_id)
                .await
                .map_err(|err| err.expert_error().to_string())?;
            if phase != SessionSyncPhase::NotStarted {
                break;
            }
            if start.elapsed() >= Duration::from_secs(10) {
                return Err("session never appeared".to_string());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        std::fs::write(alpha.join("hello.txt"), "synced-content").map_err(|e| e.to_string())?;
        flush_session_sync(&session_id)
            .await
            .map_err(|err| err.expert_error().to_string())?;
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
        terminate_session_sync(&session_id)
            .await
            .map_err(|err| err.expert_error().to_string())?;
        let alpha_content =
            std::fs::read_to_string(alpha.join("hello.txt")).map_err(|e| e.to_string())?;
        let beta_content =
            std::fs::read_to_string(beta.join("hello.txt")).map_err(|e| e.to_string())?;
        if alpha_content != "synced-content" || beta_content != "synced-content" {
            return Err("terminate removed local files".to_string());
        }
        let phase = session_sync_phase(&session_id)
            .await
            .map_err(|err| err.expert_error().to_string())?;
        if phase != SessionSyncPhase::NotStarted {
            return Err("session still listed after terminate".to_string());
        }
        Ok(())
    }
    .await;
    let _ = terminate_session_sync(&session_id).await;
    std::fs::remove_dir_all(&base).ok();
    result.expect("live mutagen round trip");
}
