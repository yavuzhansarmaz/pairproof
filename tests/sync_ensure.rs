use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pairproof::connection::{Endpoint, EphemeralUsername};
use pairproof::sandbox::process::{run_bounded, CommandSpec, DEFAULT_MAX_OUTPUT};
use pairproof::sync::{
    ensure_sync, resume_sync, sync_status, terminate_sync, EnsureOutcome, SyncError,
};

fn creds() -> (Endpoint, EphemeralUsername) {
    (
        Endpoint::new("203.0.113.10", 2222).expect("endpoint"),
        EphemeralUsername::parse("pp-abcd1234").expect("user"),
    )
}

fn bogus_creds() -> (Endpoint, EphemeralUsername) {
    (
        Endpoint::new("pairproof-nonexistent.invalid", 2222).expect("endpoint"),
        EphemeralUsername::generate(),
    )
}

#[tokio::test]
async fn ensure_resume_reject_invalid_ids_before_spawn() {
    let (endpoint, user) = creds();
    for bad in ["--privileged", "a/b", "x;rm", "", "a b", "net-x"] {
        assert!(
            matches!(resume_sync(bad).await, Err(SyncError::InvalidSessionId(_))),
            "resume {bad:?}"
        );
        assert!(
            matches!(
                ensure_sync(bad, "/tmp/alpha", &endpoint, &user).await,
                Err(SyncError::InvalidSessionId(_))
            ),
            "ensure {bad:?}"
        );
    }
}

#[tokio::test]
async fn ensure_rejects_bad_alpha_before_spawn() {
    let (endpoint, user) = creds();
    for bad in ["relative/dir", "/", "/tmp/../etc", "/tmp/a:b", ""] {
        assert!(
            matches!(
                ensure_sync("s1", bad, &endpoint, &user).await,
                Err(SyncError::InvalidEndpoint(_))
            ),
            "alpha {bad:?}"
        );
    }
}

// Live tests below need mutagen on PATH; they touch only pid-derived names
// created here (plus guaranteed-absent probe ids) and clean up afterwards.
async fn run_raw(args: Vec<String>) -> Result<String, String> {
    let spec = CommandSpec::new("mutagen", args);
    let outcome = run_bounded(&spec, None, Duration::from_secs(30), DEFAULT_MAX_OUTPUT)
        .await
        .map_err(|e| e.to_string())?;
    if outcome.success() {
        Ok(outcome.stdout_text())
    } else {
        Err(format!(
            "exit {:?}: {}",
            outcome.exit_code,
            outcome.stderr_text()
        ))
    }
}

async fn raw_list_has(name: &str) -> Result<bool, String> {
    Ok(run_raw(vec!["sync".into(), "list".into()])
        .await?
        .contains(name))
}

async fn wait_for_some(session_id: &str, timeout: Duration) -> Result<bool, String> {
    let start = Instant::now();
    loop {
        match sync_status(session_id).await.map_err(|e| e.to_string())? {
            Some(info) => return Ok(info.paused),
            None if start.elapsed() >= timeout => {
                return Err(format!("session {session_id} never appeared"));
            }
            None => tokio::time::sleep(Duration::from_millis(200)).await,
        }
    }
}

async fn wait_for_paused(session_id: &str, timeout: Duration) -> Result<(), String> {
    let start = Instant::now();
    loop {
        match sync_status(session_id).await.map_err(|e| e.to_string())? {
            Some(info) if info.paused => return Ok(()),
            _ if start.elapsed() >= timeout => {
                return Err(format!("session {session_id} never paused"));
            }
            _ => tokio::time::sleep(Duration::from_millis(200)).await,
        }
    }
}

struct LocalPair {
    session_id: String,
    name: String,
    alpha: PathBuf,
    beta: PathBuf,
    base: PathBuf,
}

async fn create_local(tag: &str, mode: &str) -> Result<LocalPair, String> {
    let pid = std::process::id();
    let session_id = format!("{tag}{pid}");
    let base = std::env::temp_dir().join(format!("pairproof-sync-{tag}{pid}"));
    let alpha = base.join("alpha");
    let beta = base.join("beta");
    std::fs::create_dir_all(&alpha).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&beta).map_err(|e| e.to_string())?;
    let _ = terminate_sync(&session_id).await;
    let _ = run_raw(vec!["daemon".into(), "start".into()]).await;
    let name = format!("pairproof-sync-{session_id}");
    run_raw(vec![
        "sync".into(),
        "create".into(),
        "--name".into(),
        name.clone(),
        "--mode".into(),
        mode.into(),
        "--symlink-mode".into(),
        "ignore".into(),
        alpha.display().to_string(),
        beta.display().to_string(),
    ])
    .await?;
    wait_for_some(&session_id, Duration::from_secs(10)).await?;
    Ok(LocalPair {
        session_id,
        name,
        alpha,
        beta,
        base,
    })
}

async fn cleanup(pair: &LocalPair) {
    let _ = terminate_sync(&pair.session_id).await;
    std::fs::remove_dir_all(&pair.base).ok();
}

fn write_file(dir: &Path, content: &str) -> Result<(), String> {
    std::fs::write(dir.join("file.txt"), content).map_err(|e| e.to_string())
}

fn read_file(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("file.txt")).unwrap_or_default()
}

#[tokio::test]
#[ignore]
async fn live_ensure_running_is_already_running() {
    let pair = create_local("ens", "two-way-safe").await.expect("create");
    // Bogus endpoint proves AlreadyRunning touches no network.
    let (endpoint, user) = bogus_creds();
    let outcome = ensure_sync(
        &pair.session_id,
        &pair.alpha.display().to_string(),
        &endpoint,
        &user,
    )
    .await;
    cleanup(&pair).await;
    assert!(
        matches!(outcome, Ok(EnsureOutcome::AlreadyRunning)),
        "got {outcome:?}"
    );
    assert_eq!(raw_list_has(&pair.name).await, Ok(false));
}

#[tokio::test]
#[ignore]
async fn live_ensure_resumes_paused() {
    let pair = create_local("enp", "two-way-safe").await.expect("create");
    run_raw(vec!["sync".into(), "pause".into(), pair.name.clone()])
        .await
        .expect("pause");
    wait_for_paused(&pair.session_id, Duration::from_secs(10))
        .await
        .expect("paused visible");
    let (endpoint, user) = bogus_creds();
    let outcome = ensure_sync(
        &pair.session_id,
        &pair.alpha.display().to_string(),
        &endpoint,
        &user,
    )
    .await;
    let after = sync_status(&pair.session_id).await.expect("status");
    cleanup(&pair).await;
    assert!(
        matches!(outcome, Ok(EnsureOutcome::Resumed)),
        "got {outcome:?}"
    );
    assert!(after.is_some_and(|info| !info.paused), "unpaused after");
    assert_eq!(raw_list_has(&pair.name).await, Ok(false));
}

#[tokio::test]
#[ignore]
async fn live_ensure_missing_bogus_endpoint_fails_typed() {
    let pid = std::process::id();
    let session_id = format!("enb{pid}");
    let alpha = std::env::temp_dir().join(format!("pairproof-alpha-enb{pid}"));
    std::fs::create_dir_all(&alpha).expect("alpha dir");
    let (endpoint, user) = bogus_creds();
    let start = Instant::now();
    let result = ensure_sync(&session_id, &alpha.display().to_string(), &endpoint, &user).await;
    let elapsed = start.elapsed();
    let terminated = terminate_sync(&session_id).await;
    std::fs::remove_dir_all(&alpha).ok();
    assert!(
        matches!(result, Err(SyncError::CommandFailed { .. })),
        "bogus endpoint must fail typed, got {result:?}"
    );
    assert!(terminated.is_ok(), "cleanup stays idempotent");
    assert!(
        elapsed < Duration::from_secs(60),
        "must fail fast, took {elapsed:?}"
    );
}

#[tokio::test]
#[ignore]
async fn live_resume_running_is_ok() {
    let pair = create_local("enr", "two-way-safe").await.expect("create");
    let result = resume_sync(&pair.session_id).await;
    cleanup(&pair).await;
    assert!(result.is_ok(), "running resume is Ok, got {result:?}");
    assert_eq!(raw_list_has(&pair.name).await, Ok(false));
}

#[tokio::test]
#[ignore]
async fn live_resume_missing_is_error() {
    let session_id = format!("enm{}", std::process::id());
    let result = resume_sync(&session_id).await;
    assert!(
        matches!(result, Err(SyncError::CommandFailed { .. })),
        "missing session must Err, NOT Ok; got {result:?}"
    );
}

struct ConflictObs {
    alive: bool,
    alpha: String,
    beta: String,
    conflicts: u64,
}

async fn conflict_case(mode: &str, tag: &str) -> Result<ConflictObs, String> {
    let pair = create_local(tag, mode).await?;
    write_file(&pair.alpha, "base")?;
    write_file(&pair.beta, "base")?;
    run_raw(vec!["sync".into(), "flush".into(), pair.name.clone()]).await?;
    run_raw(vec!["sync".into(), "pause".into(), pair.name.clone()]).await?;
    write_file(&pair.alpha, "ALPHA-VERSION")?;
    write_file(&pair.beta, "BETA-VERSION")?;
    run_raw(vec!["sync".into(), "resume".into(), pair.name.clone()]).await?;
    run_raw(vec!["sync".into(), "flush".into(), pair.name.clone()]).await?;
    let start = Instant::now();
    let obs = loop {
        let info = sync_status(&pair.session_id)
            .await
            .map_err(|e| e.to_string())?;
        let (alive, conflicts) = info.map(|i| (true, i.conflicts)).unwrap_or((false, 0));
        let alpha = read_file(&pair.alpha);
        let beta = read_file(&pair.beta);
        if conflicts > 0 || alpha == beta || start.elapsed() >= Duration::from_secs(25) {
            break ConflictObs {
                alive,
                alpha,
                beta,
                conflicts,
            };
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    };
    cleanup(&pair).await;
    if raw_list_has(&pair.name).await.unwrap_or(true) {
        return Err(format!("session {} still listed", pair.name));
    }
    Ok(obs)
}

// RAW OBSERVATIONS (Mutagen 0.18.1, local alpha+beta dirs, pause/diverge/resume):
// two-way-resolved: session Watching (alive), conflicts 0|0, alpha and beta
//   both hold ALPHA-VERSION; BETA-VERSION is unrecoverable (alpha wins).
// two-way-safe: session Watching (alive), conflicts 1|0, alpha holds
//   ALPHA-VERSION and beta holds BETA-VERSION; both versions retrievable.
// MODE DECISION: RESOLVED (two-way-safe shipped, two-way-resolved is lossy).
#[tokio::test]
#[ignore]
async fn live_conflict_modes_compared() {
    let start = Instant::now();
    let resolved = conflict_case("two-way-resolved", "cfr")
        .await
        .expect("case");
    let safe = conflict_case("two-way-safe", "cfs").await.expect("case");
    println!(
        "resolved: alive={} conflicts={} alpha={:?} beta={:?}",
        resolved.alive, resolved.conflicts, resolved.alpha, resolved.beta
    );
    println!(
        "safe: alive={} conflicts={} alpha={:?} beta={:?}",
        safe.alive, safe.conflicts, safe.alpha, safe.beta
    );
    assert!(safe.alive, "two-way-safe: session must stay alive");
    let have = [safe.alpha.trim(), safe.beta.trim()];
    assert!(
        have.contains(&"ALPHA-VERSION") && have.contains(&"BETA-VERSION"),
        "two-way-safe: both versions retrievable, got {have:?}"
    );
    assert!(
        safe.conflicts >= 1,
        "two-way-safe: conflicts must be reported, got {}",
        safe.conflicts
    );
    // Pins the rationale for shipping safe: resolved silently discards one
    // conflicting version with conflicts=0; if Mutagen ever changes resolved
    // semantics, this fails loudly and forces re-examination.
    assert!(resolved.alive, "two-way-resolved: session must stay alive");
    assert_eq!(
        resolved.beta.trim(),
        "ALPHA-VERSION",
        "two-way-resolved: beta must hold ALPHA-VERSION (BETA-VERSION lost)"
    );
    assert_eq!(
        resolved.conflicts, 0,
        "two-way-resolved: loss must stay silent (conflicts==0)"
    );
    assert!(
        start.elapsed() < Duration::from_secs(120),
        "experiment must finish in < 120s"
    );
}
