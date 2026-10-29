use std::path::{Path, PathBuf};

use pairproof::connection::{Endpoint, EphemeralUsername};
use pairproof::core::Language;
use pairproof::sync::{SyncError, SyncSessionInfo};
use pairproof::tui::candidate::candidate_sync_message;
use pairproof::tui::candidate_sync::{
    candidate_ensure_sync, candidate_sync_status, candidate_terminate_sync, map_sync_error,
    phase_for, read_local_solution, solution_progress, solution_snapshot, sync_lines, sync_view,
    CandidateSyncError, SolutionProgress, SyncPhase,
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
    assert_eq!(err.to_string(), "Sync problem \u{2014} ask the expert");
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
    assert_eq!(view.lines, vec!["Sync: active".to_string()]);
    let paused = info("pairproof-sync-s1", "Watching", true, 0);
    assert_eq!(phase_for(Some(&paused)), SyncPhase::Paused);
    assert_eq!(SyncPhase::Paused.label(), "Sync: paused");
    assert_eq!(sync_lines(Some(&paused)), vec!["Sync: paused".to_string()]);
    assert_eq!(sync_lines(None), vec!["Sync: not started".to_string()]);
    assert_eq!(SyncPhase::Problem.label(), candidate_sync_message());
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
    assert_eq!(shown, "Sync problem \u{2014} ask the expert");
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
    assert_eq!(lines, vec!["Sync: active".to_string()]);
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
        vec![phase.label().to_string()]
    );
}

#[tokio::test]
async fn candidate_workspace_preserves_local_copy_on_termination() {
    let alpha = fresh_alpha("preserve");
    write_solution(&alpha, "questions/01/solution.py", b"print('mine')\n");
    let path = alpha.join("questions/01/solution.py");
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
    write_solution(&alpha, "questions/01/solution.py", b"a\nb\n");
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
    write_solution(&alpha, "questions/02/solution.py", b"");
    assert_eq!(
        solution_progress(&alpha, 1, Language::Python),
        SolutionProgress::Empty
    );
    write_solution(&alpha, "questions/01/solution.js", b"x\n");
    assert_eq!(
        solution_progress(&alpha, 0, Language::JavaScript),
        SolutionProgress::Present { bytes: 2, lines: 1 }
    );
    write_solution(&alpha, "questions/01/solution.sql", b"select 1;\n");
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
    write_solution(&alpha, "questions/01/solution.py", b"old\n");
    let path = alpha.join("questions/01/solution.py");
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
    assert_eq!(shown, "Sync problem \u{2014} ask the expert");
    assert_eq!(shown, candidate_sync_message());
    assert_eq!(
        CandidateSyncError::Failed.to_string(),
        candidate_sync_message()
    );
}
