use crate::connection::{Endpoint, EphemeralUsername};
use crate::sandbox::process::CommandSpec;
use crate::sandbox::validate::validate_session_id;

use super::error::SyncError;

/// Derives the Mutagen session name for a validated session id.
pub fn sync_session_name(session_id: &str) -> Result<String, SyncError> {
    validate_session_id(session_id)
        .map_err(|_| SyncError::InvalidSessionId(session_id.to_string()))?;
    Ok(format!("pairproof-sync-{session_id}"))
}

/// Builds the beta endpoint URL from pre-validated Phase 6 types.
pub fn beta_url(endpoint: &Endpoint, user: &EphemeralUsername) -> String {
    format!(
        "{}@{}:{}:/workspace",
        user.as_str(),
        endpoint.host(),
        endpoint.port()
    )
}

/// Validates the candidate-side alpha directory as one safe absolute argv.
/// Rejects root, `.`/`..`/empty components and `:` (Mutagen alpha URL syntax).
pub fn validate_alpha_dir(alpha: &str) -> Result<(), SyncError> {
    let bad = alpha.is_empty()
        || alpha.len() > 512
        || !alpha.starts_with('/')
        || alpha.starts_with('-')
        || alpha == "/"
        || alpha.contains(':')
        || alpha
            .split('/')
            .skip(1)
            .any(|c| c.is_empty() || c == "." || c == "..")
        || alpha.chars().any(|c| c.is_control() || c == '\0');
    if bad {
        Err(SyncError::InvalidEndpoint(alpha.to_string()))
    } else {
        Ok(())
    }
}

/// Builds `mutagen version`.
pub fn mutagen_version() -> Result<CommandSpec, SyncError> {
    Ok(CommandSpec::new("mutagen", ["version"]))
}

/// Ignore flags so a candidate launching from a git/build tree does
/// not two-way-sync `target/`, VCS, or other project files into the
/// interview workspace. Only `qN/` question folders are synchronized.
pub const SYNC_IGNORE_ARGS: &[&str] = &[
    "--ignore-vcs",
    "--ignore",
    "/*",
    "--ignore",
    "!/q*/",
    "--ignore",
    "!/q*/**",
    "--ignore",
    "target/",
    "--ignore",
    "node_modules/",
];

/// Argv for `mutagen sync create` (without the program name).
pub fn sync_create_args(name: &str, alpha: &str, beta: &str) -> Vec<String> {
    let mut args = vec![
        "sync".to_string(),
        "create".to_string(),
        "--name".to_string(),
        name.to_string(),
        "--mode".to_string(),
        // Resolved discards one conflicting version with conflicts=0; safe keeps both and reports.
        "two-way-safe".to_string(),
        "--symlink-mode".to_string(),
        "ignore".to_string(),
    ];
    args.extend(SYNC_IGNORE_ARGS.iter().map(|s| (*s).to_string()));
    args.push(alpha.to_string());
    args.push(beta.to_string());
    args
}

/// Builds the sync creation command with endpoints as single argv items.
pub fn sync_create(
    session_id: &str,
    alpha: &str,
    endpoint: &Endpoint,
    user: &EphemeralUsername,
) -> Result<CommandSpec, SyncError> {
    let name = sync_session_name(session_id)?;
    validate_alpha_dir(alpha)?;
    let beta = beta_url(endpoint, user);
    Ok(CommandSpec::new(
        "mutagen",
        sync_create_args(&name, alpha, &beta),
    ))
}

/// Builds `mutagen sync list`.
pub fn sync_list() -> Result<CommandSpec, SyncError> {
    Ok(CommandSpec::new("mutagen", ["sync", "list"]))
}

// The guard renders 0|0 for paused sessions whose .SessionState is nil.
pub const STATUS_TEMPLATE: &str = "{{range .}}{{.Name}}|{{.Status}}|{{.Paused}}|{{if .SessionState}}{{len .SessionState.Conflicts}}|{{.SessionState.ExcludedConflicts}}{{else}}0|0{{end}}\n{{end}}";

/// Builds `mutagen sync list --template <STATUS_TEMPLATE>`.
pub fn sync_list_template() -> Result<CommandSpec, SyncError> {
    Ok(CommandSpec::new(
        "mutagen",
        ["sync", "list", "--template", STATUS_TEMPLATE],
    ))
}

/// Builds `mutagen sync resume` for the session.
pub fn sync_resume(session_id: &str) -> Result<CommandSpec, SyncError> {
    let name = sync_session_name(session_id)?;
    Ok(CommandSpec::new("mutagen", ["sync", "resume", &name]))
}

/// Builds a sync flush for the session.
pub fn sync_flush(session_id: &str) -> Result<CommandSpec, SyncError> {
    let name = sync_session_name(session_id)?;
    Ok(CommandSpec::new("mutagen", ["sync", "flush", &name]))
}

/// Builds a sync terminate for the session. Local files are never removed.
pub fn sync_terminate(session_id: &str) -> Result<CommandSpec, SyncError> {
    let name = sync_session_name(session_id)?;
    Ok(CommandSpec::new("mutagen", ["sync", "terminate", &name]))
}

/// Parses the first non-empty line of `mutagen version` output.
pub fn parse_mutagen_version(text: &str) -> Result<String, SyncError> {
    let first = text.lines().next().unwrap_or("").trim();
    if first.is_empty() {
        return Err(SyncError::ParseError {
            operation: "parse_mutagen_version",
            detail: "empty version output".to_string(),
        });
    }
    Ok(first.to_string())
}

/// Extracts every `Name:` value from `mutagen sync list` output.
pub fn parse_sync_names(text: &str) -> Vec<String> {
    parse_sync_list(text).into_iter().map(|s| s.name).collect()
}

/// One session parsed from sync output.
/// Human-list parsing yields paused=false/conflicts=0 (no such detail).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncSessionInfo {
    pub name: String,
    pub status: String,
    pub paused: bool,
    pub conflicts: u64,
}

/// Parses sessions, pairing each `Name:` with the next `Status:` in its block.
pub fn parse_sync_list(text: &str) -> Vec<SyncSessionInfo> {
    let mut sessions = Vec::new();
    let mut current: Option<(String, Option<String>)> = None;
    let mut finish = |current: &mut Option<(String, Option<String>)>| {
        if let Some((name, status)) = current.take() {
            sessions.push(SyncSessionInfo {
                name,
                status: status.unwrap_or_else(|| "unknown".to_string()),
                paused: false,
                conflicts: 0,
            });
        }
    };
    for line in text.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix("Name:") {
            finish(&mut current);
            current = Some((name.trim().to_string(), None));
        } else if let Some(status) = line.strip_prefix("Status:") {
            if let Some((_, slot)) = &mut current {
                if slot.is_none() {
                    *slot = Some(status.trim().to_string());
                }
            }
        }
    }
    finish(&mut current);
    sessions
}

/// Finds one parsed session by exact name.
/// Parses human-readable list output only; template callers use parse_sync_template.
pub fn find_sync(text: &str, name: &str) -> Option<SyncSessionInfo> {
    parse_sync_list(text).into_iter().find(|s| s.name == name)
}

/// Parses template rows `name|status|paused|conflicts|excluded`.
/// Skips malformed lines; conflicts adds reported and excluded counts.
pub fn parse_sync_template(text: &str) -> Vec<SyncSessionInfo> {
    let mut out = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('|').collect();
        if f.len() != 5 || f[0].is_empty() || f[1].is_empty() {
            continue;
        }
        if f[2] != "true" && f[2] != "false" {
            continue;
        }
        let (Ok(reported), Ok(excluded)) = (f[3].parse::<u64>(), f[4].trim().parse::<u64>()) else {
            continue;
        };
        out.push(SyncSessionInfo {
            name: f[0].to_string(),
            status: f[1].to_string(),
            paused: f[2] == "true",
            conflicts: reported.saturating_add(excluded),
        });
    }
    out
}

/// Maps a raw template status to display text; paused renders as [Paused].
/// Unknown values pass through verbatim for forward compatibility.
pub fn describe_status(raw: &str, paused: bool) -> String {
    if paused {
        return "[Paused]".to_string();
    }
    match raw {
        "Disconnected" => "Disconnected",
        "HaltedOnRootEmptied" => "Halted due to one-sided root emptying",
        "HaltedOnRootDeletion" => "Halted due to root deletion",
        "HaltedOnRootTypeChange" => "Halted due to root type change",
        "ConnectingAlpha" => "Connecting to alpha",
        "ConnectingBeta" => "Connecting to beta",
        "Watching" => "Watching for changes",
        "Scanning" => "Scanning files",
        "WaitingForRescan" => "Waiting 5 seconds for rescan",
        "Reconciling" => "Reconciling changes",
        "StagingAlpha" => "Staging files on alpha",
        "StagingBeta" => "Staging files on beta",
        "Transitioning" => "Applying changes",
        "Saving" => "Saving archive",
        other => other,
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_parses_rows_paused_and_conflicts() {
        let text = "s1|Watching|false|0|0\ns2|Disconnected|true|0|0\ns3|Watching|false|2|3\n";
        let rows = parse_sync_template(text);
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[0],
            SyncSessionInfo {
                name: "s1".into(),
                status: "Watching".into(),
                paused: false,
                conflicts: 0,
            }
        );
        assert!(rows[1].paused);
        assert_eq!(rows[2].conflicts, 5);
    }

    #[test]
    fn template_skips_malformed_lines() {
        let text = "garbage\ns1|Watching|false|0\ns2|Watching|maybe|0|0\n\
            s3|Watching|false|x|0\ns4|Watching|false|1|2\n\n|Watching|false|0|0\n";
        let rows = parse_sync_template(text);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "s4");
        assert_eq!(rows[0].conflicts, 3);
        assert!(parse_sync_template("").is_empty());
    }

    #[test]
    fn template_exact_name_match_rejects_prefix() {
        let text = "pairproof-sync-s1|Watching|false|0|0\n\
            pairproof-sync-s10|Watching|false|1|0\n";
        let rows = parse_sync_template(text);
        // Same exact-match semantics as find_sync: no prefix matching.
        let hit: Vec<_> = rows
            .iter()
            .filter(|s| s.name == "pairproof-sync-s1")
            .collect();
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].conflicts, 0);
    }

    #[test]
    fn describe_status_table_and_passthrough() {
        let table = [
            ("Disconnected", "Disconnected"),
            (
                "HaltedOnRootEmptied",
                "Halted due to one-sided root emptying",
            ),
            ("HaltedOnRootDeletion", "Halted due to root deletion"),
            ("HaltedOnRootTypeChange", "Halted due to root type change"),
            ("ConnectingAlpha", "Connecting to alpha"),
            ("ConnectingBeta", "Connecting to beta"),
            ("Watching", "Watching for changes"),
            ("Scanning", "Scanning files"),
            ("WaitingForRescan", "Waiting 5 seconds for rescan"),
            ("Reconciling", "Reconciling changes"),
            ("StagingAlpha", "Staging files on alpha"),
            ("StagingBeta", "Staging files on beta"),
            ("Transitioning", "Applying changes"),
            ("Saving", "Saving archive"),
        ];
        for (raw, display) in table {
            assert_eq!(describe_status(raw, false), display, "raw {raw}");
            assert_eq!(describe_status(raw, true), "[Paused]", "paused {raw}");
        }
        assert_eq!(describe_status("SomeFutureState", false), "SomeFutureState");
    }

    #[test]
    fn create_args_ignore_project_tree_and_keep_question_folders() {
        let args = sync_create_args(
            "pairproof-sync-s1",
            "/home/candidate/session-s1",
            "pp-abcd1234@203.0.113.10:2222:/workspace",
        );
        assert!(args.contains(&"--ignore-vcs".to_string()));
        assert!(args.windows(2).any(|w| w == ["--ignore", "/*"]));
        assert!(args.windows(2).any(|w| w == ["--ignore", "!/q*/"]));
        assert!(args.windows(2).any(|w| w == ["--ignore", "!/q*/**"]));
        assert!(args.windows(2).any(|w| w == ["--ignore", "target/"]));
        assert_eq!(
            args.last().map(String::as_str),
            Some("pp-abcd1234@203.0.113.10:2222:/workspace")
        );
        assert!(args.contains(&"/home/candidate/session-s1".to_string()));
    }
}
