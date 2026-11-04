//! In-container execution through one bounded user-level primitive.
//! All commands run argv-only as the ephemeral user; code is file/stdin data.

use std::time::{Duration, Instant};

use crate::connection::EphemeralUsername;
use crate::core::Language;

use super::command::{exec_as, exec_root};
use super::error::{command_failed, map_docker_not_found, SandboxError};
use super::lifecycle::SandboxHandle;
use super::process::{run_bounded, CommandOutcome, CommandSpec, EXEC_MAX_OUTPUT};
use super::validate::WorkspacePath;
use super::workspace::read_file;

/// Programs the sandbox may execute; argv[0] must match exactly.
const ALLOWLIST: [&str; 3] = ["python3", "node", "mariadb"];
const TIMEOUT_GRACE: Duration = Duration::from_secs(5);
/// How long a candidate test-run may block the expert drain.
pub const RUN_TIMEOUT: Duration = Duration::from_secs(8);
const PPCHECK_USER: &str = "ppcheck";

fn strs(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| (*p).to_string()).collect()
}

/// Rejects argv outside the allowlist (plus empty and NUL) before docker.
fn validate_run_argv(argv: &[String]) -> Result<(), SandboxError> {
    let first = argv.first().map(String::as_str).unwrap_or("");
    if !ALLOWLIST.contains(&first) {
        return Err(SandboxError::InvalidCommand(
            argv.first().cloned().unwrap_or_default(),
        ));
    }
    if argv.iter().any(|arg| arg.contains('\0')) {
        return Err(SandboxError::InvalidCommand(first.to_string()));
    }
    Ok(())
}

fn timeout_ms(timeout: Duration) -> u64 {
    u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX)
}

/// Formats a duration as a GNU timeout seconds argument.
fn timeout_duration_arg(timeout: Duration) -> String {
    let ms = timeout.as_millis();
    if ms.is_multiple_of(1000) {
        return format!("{}", ms / 1000);
    }
    format!(
        "{}.{}",
        ms / 1000,
        format!("{:03}", ms % 1000).trim_end_matches('0')
    )
}

/// Wraps argv with in-container GNU timeout using SIGKILL.
pub fn timeout_argv(argv: &[String], timeout: Duration) -> Vec<String> {
    let mut wrapped = strs(&["timeout", "-s", "KILL"]);
    wrapped.push(timeout_duration_arg(timeout));
    wrapped.extend(argv.iter().cloned());
    wrapped
}

/// Returns true when an outcome is the inner timeout firing.
fn outcome_is_timeout(outcome: &CommandOutcome, elapsed: Duration, timeout: Duration) -> bool {
    matches!(outcome.exit_code, Some(124) | Some(137))
        && elapsed.saturating_add(Duration::from_millis(200)) >= timeout
}

/// Runs a spec with outer grace, mapping inner timeout to Timeout.
async fn run_wrapped(
    spec: &CommandSpec,
    stdin: Option<&[u8]>,
    timeout: Duration,
    program: &str,
) -> Result<CommandOutcome, SandboxError> {
    let outer = timeout.saturating_add(TIMEOUT_GRACE);
    let started = Instant::now();
    let outcome = match run_bounded(spec, stdin, outer, EXEC_MAX_OUTPUT).await {
        Ok(outcome) => outcome,
        Err(SandboxError::Timeout { .. }) => {
            return Err(SandboxError::Timeout {
                program: program.to_string(),
                timeout_ms: timeout_ms(timeout),
            })
        }
        Err(err) => return Err(map_docker_not_found(err)),
    };
    if outcome_is_timeout(&outcome, started.elapsed(), timeout) {
        return Err(SandboxError::Timeout {
            program: program.to_string(),
            timeout_ms: timeout_ms(timeout),
        });
    }
    Ok(outcome)
}

/// Builds root mariadb argv for a SQL string.
fn root_mariadb_argv(sql: &str) -> Vec<String> {
    let mut argv = strs(&["mariadb", "-u", "root", "-e"]);
    argv.push(sql.to_string());
    argv
}

/// Builds ppcheck mariadb argv via runuser with inner timeout.
fn ppcheck_mariadb_argv(timeout: Duration) -> Vec<String> {
    let mut argv = strs(&["runuser", "-u", PPCHECK_USER, "--", "timeout", "-s", "KILL"]);
    argv.push(timeout_duration_arg(timeout));
    argv.extend(strs(&["mariadb", "pp_runnable"]));
    argv
}

/// Runs argv as root, mapping the inner timeout to Timeout.
async fn run_root(
    handle: &SandboxHandle,
    argv: &[String],
    stdin: Option<&[u8]>,
    timeout: Duration,
) -> Result<CommandOutcome, SandboxError> {
    let spec = exec_root(handle.container_name(), argv, stdin.is_some())?;
    run_wrapped(&spec, stdin, timeout, "mariadb").await
}

/// Runs argv as the user; inner timeout kills the workload.
pub async fn run_in_container(
    handle: &SandboxHandle,
    user: &EphemeralUsername,
    argv: &[String],
    stdin: Option<&[u8]>,
    timeout: Duration,
) -> Result<CommandOutcome, SandboxError> {
    validate_run_argv(argv)?;
    let wrapped = timeout_argv(argv, timeout);
    let spec = exec_as(handle.container_name(), user, &wrapped, stdin.is_some())?;
    run_wrapped(&spec, stdin, timeout, &argv[0]).await
}

/// Compile-check argv for a solution file; sql has none (stdin path), empty.
pub fn runnability_argv(language: Language, path: &WorkspacePath) -> Vec<String> {
    let file = path.container_path();
    match language {
        Language::Python => vec![
            "python3".to_string(),
            "-m".to_string(),
            "py_compile".to_string(),
            file,
        ],
        Language::JavaScript => vec!["node".to_string(), "--check".to_string(), file],
        Language::Sql => Vec::new(),
    }
}

/// Test-run argv for a solution file; sql has none (stdin path), empty.
pub fn test_argv(language: Language, path: &WorkspacePath) -> Vec<String> {
    let file = path.container_path();
    match language {
        Language::Python => vec!["python3".to_string(), file],
        Language::JavaScript => vec!["node".to_string(), file],
        Language::Sql => Vec::new(),
    }
}

/// Outcome of a runnability check, derived from exit status only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnabilityReport {
    pub runnable: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}

/// Builds a runnability report from one finished outcome.
fn report_from(outcome: &CommandOutcome) -> RunnabilityReport {
    RunnabilityReport {
        runnable: outcome.success(),
        exit_code: outcome.exit_code,
        stdout: outcome.stdout_text(),
        stderr: outcome.stderr_text(),
        truncated: outcome.stdout_truncated || outcome.stderr_truncated,
    }
}

/// Checks a file runs; sql runs only as ppcheck in pp_runnable.
pub async fn check_runnability(
    handle: &SandboxHandle,
    user: &EphemeralUsername,
    path: &WorkspacePath,
    language: Language,
    timeout: Duration,
) -> Result<RunnabilityReport, SandboxError> {
    if language == Language::Sql {
        return check_sql_runnability(handle, user, path, timeout).await;
    }
    let argv = runnability_argv(language, path);
    let outcome = run_in_container(handle, user, &argv, None, timeout).await?;
    Ok(report_from(&outcome))
}

/// Runs sql as ppcheck in pp_runnable, dropping it again best-effort.
#[rustfmt::skip]
async fn check_sql_runnability(handle: &SandboxHandle, user: &EphemeralUsername, path: &WorkspacePath, timeout: Duration) -> Result<RunnabilityReport, SandboxError> {
    let content = read_file(handle, user, path).await?;
    let setup = timeout_argv(&root_mariadb_argv("DROP DATABASE IF EXISTS pp_runnable; CREATE DATABASE pp_runnable"), timeout);
    let setup_out = run_root(handle, &setup, None, timeout).await?;
    if !setup_out.success() { return Err(command_failed("sql_scratch_setup", &setup_out)); }
    let run = ppcheck_mariadb_argv(timeout);
    let outcome = run_root(handle, &run, Some(&content), timeout).await?;
    let drop = timeout_argv(&root_mariadb_argv("DROP DATABASE IF EXISTS pp_runnable"), timeout);
    let _ = run_root(handle, &drop, None, timeout).await;
    Ok(report_from(&outcome))
}

/// Test-run outcome; passed/failed counts are parsed by [`parse_test_counts`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestOutcome {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

/// Reads passed/failed counts from test-run output.
/// `TESTS: N passed, M failed` wins; otherwise unittest-style `Ran N tests`,
/// then per-case `Test N: PASSED` / `FAILED` lines.
/// An empty successful run is `(0, 0)` — no tests executed.
pub fn parse_test_counts(stdout: &str, stderr: &str, success: bool) -> (u32, u32) {
    let text = format!("{stdout}\n{stderr}");
    if let Some(pair) = parse_tests_prefix(&text) {
        return pair;
    }
    if let Some(pair) = parse_passed_failed_phrase(&text.to_ascii_lowercase()) {
        return pair;
    }
    if let Some(pair) = parse_unittest(&text) {
        return pair;
    }
    if let Some(pair) = parse_per_case_results(&text) {
        return pair;
    }
    let lower = text.to_ascii_lowercase();
    if lower.contains("assertionerror") {
        return (0, 1);
    }
    if success {
        (0, 0)
    } else {
        (0, 1)
    }
}

/// Parsed counts when the output actually executed at least one test.
pub fn counted_test_counts(stdout: &str, stderr: &str, success: bool) -> Option<(u32, u32)> {
    let (passed, failed) = parse_test_counts(stdout, stderr, success);
    (passed + failed > 0).then_some((passed, failed))
}

fn parse_tests_prefix(text: &str) -> Option<(u32, u32)> {
    for line in text.lines() {
        let rest = line.trim().strip_prefix("TESTS:")?.trim();
        if let Some(pair) = parse_passed_failed_phrase(&rest.to_ascii_lowercase()) {
            return Some(pair);
        }
    }
    None
}

fn parse_passed_failed_phrase(s: &str) -> Option<(u32, u32)> {
    let passed = number_before(s, " passed")?;
    let failed = number_before(s, " failed").unwrap_or(0);
    Some((passed, failed))
}

fn number_before(s: &str, marker: &str) -> Option<u32> {
    let idx = s.find(marker)?;
    let prefix = s[..idx].trim();
    let digits: String = prefix
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

fn parse_per_case_results(text: &str) -> Option<(u32, u32)> {
    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut found = false;
    for line in text.lines() {
        let lower = line.trim().to_ascii_lowercase();
        let Some(after_test) = lower.strip_prefix("test ") else {
            continue;
        };
        let after_num = after_test.trim_start();
        let digits = after_num.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 {
            continue;
        }
        let rest = after_num[digits..].trim_start();
        let rest = rest.strip_prefix(':').unwrap_or(rest).trim_start();
        if rest.starts_with("passed") {
            passed = passed.saturating_add(1);
            found = true;
        } else if rest.starts_with("failed") {
            failed = failed.saturating_add(1);
            found = true;
        }
    }
    found.then_some((passed, failed))
}

fn parse_unittest(text: &str) -> Option<(u32, u32)> {
    let lower = text.to_ascii_lowercase();
    let idx = lower.find("ran ")?;
    let rest = &lower[idx + 4..];
    let n: u32 = rest.split_whitespace().next()?.parse().ok()?;
    if !rest.contains("test") {
        return None;
    }
    if let Some(after) = lower.split("failures=").nth(1) {
        let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        let failed: u32 = digits.parse().ok()?;
        return Some((n.saturating_sub(failed), failed));
    }
    if lower.contains("\nok") || lower.trim_end().ends_with("ok") {
        Some((n, 0))
    } else {
        None
    }
}

/// Runs a solution file; sql runs file bytes in the session db pairproof.
pub async fn run_tests(
    handle: &SandboxHandle,
    user: &EphemeralUsername,
    path: &WorkspacePath,
    language: Language,
    timeout: Duration,
) -> Result<TestOutcome, SandboxError> {
    let content;
    let (argv, stdin): (Vec<String>, Option<&[u8]>) = match language {
        Language::Sql => {
            content = read_file(handle, user, path).await?;
            (
                vec!["mariadb".to_string(), "pairproof".to_string()],
                Some(&content),
            )
        }
        _ => (test_argv(language, path), None),
    };
    let outcome = run_in_container(handle, user, &argv, stdin, timeout).await?;
    Ok(TestOutcome {
        success: outcome.success(),
        exit_code: outcome.exit_code,
        stdout: outcome.stdout_text(),
        stderr: outcome.stderr_text(),
        stdout_truncated: outcome.stdout_truncated,
        stderr_truncated: outcome.stderr_truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(rel: &str) -> WorkspacePath {
        WorkspacePath::new(rel).expect("valid workspace path")
    }

    fn outcome(exit_code: Option<i32>) -> CommandOutcome {
        CommandOutcome {
            exit_code,
            stdout: Vec::new(),
            stderr: Vec::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        }
    }

    #[test]
    fn runnability_argv_shapes() {
        let py = workspace("questions/01/solution.py");
        assert_eq!(
            runnability_argv(Language::Python, &py),
            strs(&[
                "python3",
                "-m",
                "py_compile",
                "/workspace/questions/01/solution.py"
            ])
        );
        let js = workspace("questions/02/solution.js");
        assert_eq!(
            runnability_argv(Language::JavaScript, &js),
            strs(&["node", "--check", "/workspace/questions/02/solution.js"])
        );
        let sql = workspace("questions/03/query.sql");
        assert!(runnability_argv(Language::Sql, &sql).is_empty());
    }

    #[test]
    fn test_argv_shapes() {
        let py = workspace("questions/01/solution.py");
        assert_eq!(
            test_argv(Language::Python, &py),
            strs(&["python3", "/workspace/questions/01/solution.py"])
        );
        let js = workspace("questions/02/solution.js");
        assert_eq!(
            test_argv(Language::JavaScript, &js),
            strs(&["node", "/workspace/questions/02/solution.js"])
        );
        let sql = workspace("questions/03/query.sql");
        assert!(test_argv(Language::Sql, &sql).is_empty());
    }

    #[test]
    fn run_argv_allowlist() {
        assert!(validate_run_argv(&["python3".to_string()]).is_ok());
        assert!(validate_run_argv(&["node".to_string(), "--check".to_string()]).is_ok());
        assert!(validate_run_argv(&[]).is_err());
        assert!(validate_run_argv(&["sh".to_string()]).is_err());
        assert!(validate_run_argv(&["python3;rm".to_string()]).is_err());
        assert!(validate_run_argv(&["python3".to_string(), "a\0b".to_string()]).is_err());
    }

    #[test]
    #[rustfmt::skip]
    fn timeout_argv_wraps_with_kill() {
        let argv = strs(&["python3", "-c", "pass"]);
        assert_eq!(timeout_argv(&argv, Duration::from_secs(1)), strs(&["timeout", "-s", "KILL", "1", "python3", "-c", "pass"]));
        assert_eq!(timeout_duration_arg(Duration::from_millis(1500)), "1.5");
        assert_eq!(timeout_duration_arg(Duration::from_millis(1001)), "1.001");
        assert_eq!(timeout_duration_arg(Duration::from_millis(1100)), "1.1");
        assert_eq!(timeout_duration_arg(Duration::from_secs(30)), "30");
    }

    #[test]
    #[rustfmt::skip]
    fn outcome_timeout_requires_elapsed() {
        let req = Duration::from_secs(1);
        for (code, ms, want) in [(137, 1000, true), (124, 900, true), (124, 10, false), (137, 10, false), (0, 2000, false)] {
            assert_eq!(outcome_is_timeout(&outcome(Some(code)), Duration::from_millis(ms), req), want, "code {code} ms {ms}");
        }
        assert!(!outcome_is_timeout(&outcome(None), Duration::from_secs(2), req));
    }

    #[test]
    fn parse_test_counts_reads_expected_vs_actual() {
        assert_eq!(
            parse_test_counts("TESTS: 2 passed, 1 failed\n", "", false),
            (2, 1)
        );
        assert_eq!(
            parse_test_counts("ok\nTESTS: 3 passed, 0 failed", "", true),
            (3, 0)
        );
        assert_eq!(
            parse_test_counts("Ran 4 tests in 0.01s\n\nOK\n", "", true),
            (4, 0)
        );
        assert_eq!(
            parse_test_counts("", "Ran 3 tests in 0.01s\nFAILED (failures=1)\n", false),
            (2, 1)
        );
        assert_eq!(
            parse_test_counts("", "AssertionError: expected 3", false),
            (0, 1)
        );
        assert_eq!(parse_test_counts("", "", true), (0, 0));
        assert_eq!(parse_test_counts("boom", "traceback", false), (0, 1));
        let per_case = "\
Test 1: PASSED - nums=[0, 1, 1, 0] -> [0, 1]
Test 2: PASSED - nums=[0, 3, 2, 1, 3, 2] -> [2, 3]
Test 3: FAILED - nums=[1, 0], expected [0, 1]
Test 4: PASSED - nums=[0, 0, 1, 2, 2] -> [0, 2]
no tests ran. press t to write tests.";
        assert_eq!(parse_test_counts(per_case, "", true), (3, 1));
        assert_eq!(counted_test_counts(per_case, "", true), Some((3, 1)));
        assert_eq!(counted_test_counts("", "", true), None);
        assert_eq!(
            parse_test_counts(
                "Test 1: PASSED\nTest 2: PASSED\nTESTS: 2 passed, 0 failed\n",
                "",
                true
            ),
            (2, 0)
        );
    }

    #[test]
    #[rustfmt::skip]
    fn scratch_and_ppcheck_argv_shapes() {
        assert_eq!(ppcheck_mariadb_argv(Duration::from_secs(30)), strs(&["runuser", "-u", "ppcheck", "--", "timeout", "-s", "KILL", "30", "mariadb", "pp_runnable"]));
        assert_eq!(root_mariadb_argv("SELECT 1"), strs(&["mariadb", "-u", "root", "-e", "SELECT 1"]));
        let setup = timeout_argv(&root_mariadb_argv("DROP DATABASE IF EXISTS pp_runnable; CREATE DATABASE pp_runnable"), Duration::from_secs(30));
        assert_eq!(setup[..4], ["timeout", "-s", "KILL", "30"]);
        assert!(setup[8].contains("CREATE DATABASE pp_runnable"));
    }
}
