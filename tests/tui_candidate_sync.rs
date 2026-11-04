use std::path::{Path, PathBuf};

use pairproof::connection::{Endpoint, EphemeralUsername};
use pairproof::core::Language;
use pairproof::orchestration::sync_gate::{sync_phase_for, SessionSyncPhase};
use pairproof::sync::{SyncError, SyncSessionInfo};
use pairproof::tui::candidate::candidate_sync_message;
use pairproof::tui::candidate_sync::{
    candidate_ensure_sync, candidate_sync_status, candidate_terminate_sync, map_sync_error,
    phase_for, read_local_solution, solution_progress, solution_snapshot, sync_lines, sync_view,
    workspace_preview, CandidateSyncError, SolutionProgress, SyncPhase,
};

/// Returns a unique temp alpha dir for one test.
fn temp_alpha(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "pairproof-candidate-sync-{}-{name}",
        std::process::id()
    ))
}

/// Creates a fresh alpha dir and returns its path.
fn fresh_alpha(name: &str) -> PathBuf {
    let alpha = temp_alpha(name);
    let _ = std::fs::remove_dir_all(&alpha);
    std::fs::create_dir_all(&alpha).expect("alpha");
    alpha
}

/// Writes a solution file under the alpha workspace.
fn write_solution(alpha: &Path, rel: &str, content: &[u8]) {
    let path = alpha.join(rel);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
    std::fs::write(&path, content).expect("write");
}

/// Builds a hand-made session info value.
fn info(name: &str, status: &str, paused: bool, conflicts: u64) -> SyncSessionInfo {
    SyncSessionInfo {
        name: name.to_string(),
        status: status.to_string(),
        paused,
        conflicts,
    }
}

#[tokio::test]
async fn candidate_sync_uses_existing_lifecycle_api() {
    let endpoint = Endpoint::new("203.0.113.10", 2222).expect("endpoint");
    let user = EphemeralUsername::parse("pp-abcd1234").expect("user");
    let ensured = candidate_ensure_sync("bad id!;rm", "relative/path", &endpoint, &user).await;
    let err = ensured.expect_err("invalid ensure must fail");
    assert_eq!(err.to_string(), candidate_sync_message());
    assert_eq!(err.to_string(), "sync problem \u{2014} ask the expert");
    let status = candidate_sync_status("bad/id").await;
    let err = status.expect_err("invalid status id must fail");
    assert_eq!(err.to_string(), candidate_sync_message());
    assert!(!err.to_string().contains("bad"));
}

#[test]
fn candidate_sync_status_reflects_actual_result() {
    assert_eq!(phase_for(None), SyncPhase::NotStarted);
    assert_eq!(SyncPhase::NotStarted.label(), "Sync: not started");
    let active = info("pairproof-sync-s1", "Watching", false, 0);
    assert_eq!(phase_for(Some(&active)), SyncPhase::Active);
    assert_eq!(SyncPhase::Active.label(), "Sync: active");
    let view = sync_view(Some(&active));
    assert_eq!(view.phase, SyncPhase::Active);
    assert_eq!(view.lines, vec!["sync: active".to_string()]);
    let paused = info("pairproof-sync-s1", "Watching", true, 0);
    assert_eq!(phase_for(Some(&paused)), SyncPhase::Paused);
    assert_eq!(SyncPhase::Paused.label(), "Sync: paused");
    assert_eq!(sync_lines(Some(&paused)), vec!["sync: paused".to_string()]);
    assert_eq!(sync_lines(None), vec!["sync: not started".to_string()]);
    // The sync-gate label stays canonical; the candidate screen
    // message is its lowercase rendering of the same text.
    assert_eq!(
        SyncPhase::Problem.label(),
        "Sync problem \u{2014} ask the expert"
    );
}

#[test]
fn candidate_sync_failure_is_generic() {
    let raw = SyncError::CommandFailed {
        operation: "create_sync",
        exit_code: Some(1),
        stderr: "ssh root@10.0.0.1 /home/expert pairproof-sync-x".to_string(),
    };
    let mapped = map_sync_error(&raw);
    assert_eq!(mapped, CandidateSyncError::Failed);
    let shown = mapped.to_string();
    assert_eq!(shown, "sync problem \u{2014} ask the expert");
    for secret in [
        "ssh",
        "root@",
        "10.0.0.1",
        "/home/expert",
        "pairproof-sync-x",
    ] {
        assert!(!shown.contains(secret), "leaked {secret}");
        assert!(!format!("{mapped:?}").contains(secret), "debug leaked");
    }
}

#[test]
fn candidate_sync_never_renders_paths_or_ssh_target() {
    let secret = info("pairproof-sync-secret", "Watching", false, 0);
    let lines = sync_lines(Some(&secret));
    assert_eq!(lines, vec!["sync: active".to_string()]);
    let joined = lines.join("\n");
    for needle in ["pairproof-sync-secret", "Watching", "ssh", "/", "secret"] {
        assert!(!joined.contains(needle), "rendered {needle}");
    }
}

#[test]
fn candidate_sync_conflict_is_not_reported_as_clean() {
    let conflicted = info("pairproof-sync-s1", "Watching", false, 2);
    let phase = phase_for(Some(&conflicted));
    assert_eq!(phase, SyncPhase::Conflict);
    assert_ne!(phase, SyncPhase::Active);
    assert!(phase.label().contains("ask the expert"));
    assert_eq!(
        phase.label(),
        "Sync: conflict needs attention \u{2014} ask the expert"
    );
    assert_eq!(
        sync_lines(Some(&conflicted)),
        vec!["sync: conflict needs attention \u{2014} ask the expert".to_string()]
    );
}

#[test]
fn candidate_sync_paused_conflict_prefers_conflict_and_agrees_with_gate() {
    let paused_conflict = info("pairproof-sync-s1", "Watching", true, 1);
    assert_eq!(phase_for(Some(&paused_conflict)), SyncPhase::Conflict);
    assert_eq!(
        sync_phase_for(Some(&paused_conflict)),
        SessionSyncPhase::Conflict
    );
    assert_eq!(
        phase_for(Some(&paused_conflict)),
        sync_phase_for(Some(&paused_conflict))
    );
}

#[tokio::test]
async fn candidate_workspace_preserves_local_copy_on_termination() {
    let alpha = fresh_alpha("preserve");
    write_solution(&alpha, "q1/solution.py", b"print('mine')\n");
    let path = alpha.join("q1/solution.py");
    let before = std::fs::read(&path).expect("before");
    assert_eq!(
        solution_progress(&alpha, 0, Language::Python),
        SolutionProgress::Present {
            bytes: before.len() as u64,
            lines: 1
        }
    );
    let result = candidate_terminate_sync("bad id!").await;
    let err = result.expect_err("illegal id must fail generically");
    assert_eq!(err.to_string(), candidate_sync_message());
    assert_eq!(
        read_local_solution(&alpha, 0, Language::Python).as_deref(),
        Some("print('mine')\n")
    );
    assert_eq!(std::fs::read(&path).expect("after"), before);
    let _ = std::fs::remove_dir_all(&alpha);
}

#[test]
fn candidate_solution_progress_reads_workspace_state() {
    let alpha = fresh_alpha("progress");
    assert_eq!(
        solution_progress(&alpha, 0, Language::Python),
        SolutionProgress::Missing
    );
    assert_eq!(read_local_solution(&alpha, 0, Language::Python), None);
    assert_eq!(solution_snapshot(&alpha, 0, Language::Python), None);
    write_solution(&alpha, "q1/solution.py", b"a\nb\n");
    assert_eq!(
        solution_progress(&alpha, 0, Language::Python),
        SolutionProgress::Present { bytes: 4, lines: 2 }
    );
    assert_eq!(
        read_local_solution(&alpha, 0, Language::Python).as_deref(),
        Some("a\nb\n")
    );
    let snap = solution_snapshot(&alpha, 0, Language::Python).expect("snapshot");
    assert_eq!((snap.len(), snap.newline_count()), (4, 2));
    write_solution(&alpha, "q2/solution.py", b"");
    assert_eq!(
        solution_progress(&alpha, 1, Language::Python),
        SolutionProgress::Empty
    );
    write_solution(&alpha, "q1/solution.js", b"x\n");
    assert_eq!(
        solution_progress(&alpha, 0, Language::JavaScript),
        SolutionProgress::Present { bytes: 2, lines: 1 }
    );
    write_solution(&alpha, "q1/solution.sql", b"select 1;\n");
    assert_eq!(
        solution_progress(&alpha, 0, Language::Sql),
        SolutionProgress::Present {
            bytes: 10,
            lines: 1
        }
    );
    let _ = std::fs::remove_dir_all(&alpha);
}

#[test]
fn candidate_stale_solution_view_is_not_written_back() {
    let alpha = fresh_alpha("stale");
    write_solution(&alpha, "q1/solution.py", b"old\n");
    let path = alpha.join("q1/solution.py");
    assert_eq!(
        read_local_solution(&alpha, 0, Language::Python).as_deref(),
        Some("old\n")
    );
    assert_eq!(std::fs::read(&path).expect("unchanged"), b"old\n");
    std::fs::write(&path, b"new\n").expect("external rewrite");
    assert_eq!(
        read_local_solution(&alpha, 0, Language::Python).as_deref(),
        Some("new\n")
    );
    assert_eq!(std::fs::read(&path).expect("still new"), b"new\n");
    let _ = std::fs::remove_dir_all(&alpha);
}

#[test]
fn candidate_sync_missing_backend_is_generic() {
    assert_eq!(
        map_sync_error(&SyncError::MutagenNotFound),
        CandidateSyncError::Unavailable
    );
    let shown = CandidateSyncError::Unavailable.to_string();
    assert_eq!(shown, "sync problem \u{2014} ask the expert");
    assert_eq!(shown, candidate_sync_message());
    assert_eq!(
        CandidateSyncError::Failed.to_string(),
        candidate_sync_message()
    );
}

#[test]
fn workspace_preview_shows_the_synced_solution_and_how_to_run_it() {
    let alpha = fresh_alpha("preview");
    write_solution(&alpha, "q1/question.md", b"# Two Sum\n");
    write_solution(&alpha, "q1/solution.py", b"def solve():\n    return 1\n");
    let lines = workspace_preview(&alpha, 0);
    let text = lines.join("\n");
    assert!(text.contains("q1/solution.py"), "{text}");
    assert!(text.contains("q1/question.md"), "{text}");
    assert!(text.contains("run: python3 q1/solution.py"), "{text}");
    assert!(text.contains("def solve():"), "{text}");
    assert!(workspace_preview(&alpha, 1).is_empty());
    let _ = std::fs::remove_dir_all(&alpha);
}

#[test]
fn workspace_preview_handles_languages_and_refreshes_on_modification() {
    use pairproof::orchestration::SnapshotRun;
    use pairproof::tui::candidate_sync::{has_synced_solution, workspace_preview_with_run};

    let alpha = fresh_alpha("langs");
    // Empty stub and hidden temp/cache entries must not appear as a ready solution.
    write_solution(&alpha, "q1/question.md", b"# Q1\n");
    write_solution(&alpha, "q1/solution.py", b"");
    write_solution(&alpha, "q1/.solution.py.pairproof-tmp", b"draft");
    std::fs::create_dir_all(alpha.join("q1/__pycache__")).expect("pycache");
    assert!(!has_synced_solution(&alpha, 0));
    let before = workspace_preview(&alpha, 0).join("\n");
    assert!(before.contains("q1/question.md"), "{before}");
    assert!(!before.contains("run: python3 q1/solution.py"), "{before}");
    assert!(!before.contains("__pycache__"), "{before}");

    // Writing real content makes the solution visible immediately.
    write_solution(&alpha, "q1/solution.py", b"def solve():\n    return 1\n");
    assert!(has_synced_solution(&alpha, 0));
    let py = workspace_preview(&alpha, 0).join("\n");
    assert!(py.contains("run: python3 q1/solution.py"), "{py}");
    assert!(py.contains("return 1"), "{py}");

    // Candidate edit on disk refreshes on the next workspace read without stale cache.
    write_solution(&alpha, "q1/solution.py", b"def solve():\n    return 42\n");
    let run = SnapshotRun {
        kind: "run".to_string(),
        seq: 1,
        code_revision: 1,
        success: true,
        exit_code: Some(0),
        stdout: "42".to_string(),
        stderr: String::new(),
        at_ms: 100,
    };
    let updated = workspace_preview_with_run(&alpha, 0, Some(&run)).join("\n");
    assert!(updated.contains("return 42"), "{updated}");
    assert!(!updated.contains("return 1"), "{updated}");
    assert!(updated.contains("last run: passed (exit 0)"), "{updated}");
    assert!(updated.contains("stdout: 42"), "{updated}");

    let no_tests = SnapshotRun {
        kind: "test".to_string(),
        seq: 2,
        code_revision: 1,
        success: true,
        exit_code: Some(0),
        stdout: "no tests ran. press t to write tests.".to_string(),
        stderr: String::new(),
        at_ms: 110,
    };
    let empty = workspace_preview_with_run(&alpha, 0, Some(&no_tests)).join("\n");
    assert!(
        empty.contains("last run: no tests (press t to write tests)"),
        "{empty}"
    );
    assert!(!empty.contains("last test: passed"), "{empty}");

    // JavaScript and SQL paths and run hints.
    write_solution(
        &alpha,
        "q2/solution.js",
        b"function solve() { return 2; }\n",
    );
    assert!(has_synced_solution(&alpha, 1));
    let js = workspace_preview(&alpha, 1).join("\n");
    assert!(js.contains("q2/solution.js"), "{js}");
    assert!(js.contains("run: node q2/solution.js"), "{js}");
    assert!(js.contains("function solve()"), "{js}");

    write_solution(&alpha, "q3/solution.sql", b"SELECT 1;\n");
    assert!(has_synced_solution(&alpha, 2));
    let sql = workspace_preview(&alpha, 2).join("\n");
    assert!(sql.contains("q3/solution.sql"), "{sql}");
    assert!(sql.contains("solution file: q3/solution.sql"), "{sql}");
    assert!(sql.contains("SELECT 1;"), "{sql}");

    let _ = std::fs::remove_dir_all(&alpha);
}

#[tokio::test]
async fn real_mutagen_syncs_generated_and_edited_solution_files() {
    use pairproof::orchestration::sync_gate::{flush_session_sync, terminate_session_sync};
    use pairproof::sandbox::process::{run_bounded, CommandSpec, DEFAULT_MAX_OUTPUT};
    use pairproof::tui::candidate_sync::has_synced_solution;
    use std::time::Duration;

    if pairproof::sync::check_mutagen_available().await.is_err() {
        return;
    }
    let session_id = format!(" candws{}", std::process::id()).trim().to_string();
    let base = temp_alpha("real-mutagen");
    let _ = std::fs::remove_dir_all(&base);
    let alpha = base.join("alpha");
    let beta = base.join("beta");
    std::fs::create_dir_all(&alpha).expect("alpha");
    std::fs::create_dir_all(&beta).expect("beta");
    let name = format!("pairproof-sync-{session_id}");
    let _ = terminate_session_sync(&session_id).await;

    let create = CommandSpec::new(
        "mutagen",
        vec![
            "sync".into(),
            "create".into(),
            "--name".into(),
            name,
            "--mode".into(),
            "two-way-safe".into(),
            "--symlink-mode".into(),
            "ignore".into(),
            alpha.display().to_string(),
            beta.display().to_string(),
        ],
    );
    let created = run_bounded(&create, None, Duration::from_secs(30), DEFAULT_MAX_OUTPUT)
        .await
        .expect("mutagen create");
    assert!(created.success(), "create: {}", created.stderr_text());

    // 1. Expert generates initial solution on beta; flush brings it to candidate alpha.
    write_solution(&beta, "q1/question.md", b"# Two Sum\n");
    write_solution(
        &beta,
        "q1/solution.py",
        b"def two_sum(nums, target):\n    return [0, 1]\n",
    );
    flush_session_sync(&session_id).await.expect("flush 1");
    assert!(has_synced_solution(&alpha, 0));
    let preview1 = workspace_preview(&alpha, 0).join("\n");
    assert!(preview1.contains("q1/solution.py"), "{preview1}");
    assert!(
        preview1.contains("run: python3 q1/solution.py"),
        "{preview1}"
    );
    assert!(
        preview1.contains("def two_sum(nums, target):"),
        "{preview1}"
    );

    // 2. Candidate edits q1/solution.py in alpha; flush pushes edit to beta before agent continuation.
    write_solution(
        &alpha,
        "q1/solution.py",
        b"def two_sum(nums, target):\n    seen = {}\n    return []\n",
    );
    flush_session_sync(&session_id).await.expect("flush 2");
    let on_beta = std::fs::read_to_string(beta.join("q1/solution.py")).expect("beta read");
    assert!(on_beta.contains("seen = {}"), "{on_beta}");

    // 3. Agent updates q1/solution.py on beta after reading candidate's edit; flush updates alpha cleanly.
    write_solution(
        &beta,
        "q1/solution.py",
        b"def two_sum(nums, target):\n    seen = {}\n    for i, x in enumerate(nums):\n        if target - x in seen:\n            return [seen[target - x], i]\n        seen[x] = i\n    return []\n",
    );
    flush_session_sync(&session_id).await.expect("flush 3");
    let preview2 = workspace_preview(&alpha, 0).join("\n");
    assert!(preview2.contains("enumerate(nums)"), "{preview2}");

    let _ = terminate_session_sync(&session_id).await;
    let _ = std::fs::remove_dir_all(&base);
}
