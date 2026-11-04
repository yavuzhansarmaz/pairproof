//! Workspace file access inside the session container.
//! File content travels only as process stdin bytes, never as shell input.

use crate::connection::EphemeralUsername;
use crate::core::Language;

use super::command::exec_as;
use super::error::{command_failed, map_docker_not_found, SandboxError};
use super::lifecycle::{SandboxHandle, OP_TIMEOUT};
use super::parse::{is_path_within_workspace, parse_realpath};
use super::process::{run_bounded, CommandOutcome, DEFAULT_MAX_OUTPUT, EXEC_MAX_OUTPUT};
use super::validate::{question_layout, WorkspacePath, MAX_FILE_BYTES, WORKSPACE_ROOT};

/// Runs one argv as the ephemeral user without a shell.
async fn run_user(
    handle: &SandboxHandle,
    user: &EphemeralUsername,
    argv: &[String],
    stdin: Option<&[u8]>,
    max_output: usize,
) -> Result<CommandOutcome, SandboxError> {
    let spec = exec_as(handle.container_name(), user, argv, stdin.is_some())?;
    run_bounded(&spec, stdin, OP_TIMEOUT, max_output)
        .await
        .map_err(map_docker_not_found)
}

/// Runs one argv as the user and requires a zero exit code.
async fn run_user_ok(
    handle: &SandboxHandle,
    user: &EphemeralUsername,
    argv: &[String],
    stdin: Option<&[u8]>,
    max_output: usize,
    operation: &'static str,
) -> Result<CommandOutcome, SandboxError> {
    let outcome = run_user(handle, user, argv, stdin, max_output).await?;
    if outcome.success() {
        Ok(outcome)
    } else {
        Err(command_failed(operation, &outcome))
    }
}

/// Parses `wc -c` output as a byte count.
fn parse_byte_count(text: &str) -> Result<usize, SandboxError> {
    text.split_whitespace()
        .next()
        .and_then(|token| token.parse::<usize>().ok())
        .ok_or_else(|| SandboxError::ParseError {
            operation: "wc_size",
            detail: text.trim().to_string(),
        })
}

/// Resolves `dir` in-container and rejects escapes from the workspace.
async fn checked_dir(
    handle: &SandboxHandle,
    user: &EphemeralUsername,
    dir: &str,
    operation: &'static str,
) -> Result<(), SandboxError> {
    let argv = vec!["realpath".to_string(), dir.to_string()];
    let outcome = run_user_ok(handle, user, &argv, None, DEFAULT_MAX_OUTPUT, operation).await?;
    let resolved = parse_realpath(&outcome.stdout_text())?;
    if is_path_within_workspace(&resolved) {
        Ok(())
    } else {
        Err(SandboxError::PathEscape(resolved))
    }
}

/// Writes bytes to a workspace path as the user; content is stdin data.
// Residual TOCTOU: a symlink swap between the realpath check and tee stays inside the disposable container.
pub async fn write_file(
    handle: &SandboxHandle,
    user: &EphemeralUsername,
    path: &WorkspacePath,
    content: &[u8],
) -> Result<(), SandboxError> {
    if content.len() > MAX_FILE_BYTES {
        return Err(SandboxError::FileTooLarge {
            len: content.len(),
            max: MAX_FILE_BYTES,
        });
    }
    let target = path.container_path();
    let parent = path
        .parent_container_dir()
        .unwrap_or_else(|| WORKSPACE_ROOT.to_string());
    let mkdir = vec!["mkdir".to_string(), "-p".to_string(), parent.clone()];
    run_user_ok(
        handle,
        user,
        &mkdir,
        None,
        DEFAULT_MAX_OUTPUT,
        "mkdir_parent",
    )
    .await?;
    checked_dir(handle, user, &parent, "realpath_parent").await?;
    let leaf = path.as_str().rsplit('/').next().unwrap_or("file");
    let tmp = format!("{parent}/.{leaf}.pairproof-tmp");
    let tee = vec!["tee".to_string(), tmp.clone()];
    run_user_ok(
        handle,
        user,
        &tee,
        Some(content),
        DEFAULT_MAX_OUTPUT,
        "write_tee",
    )
    .await?;
    let rename = vec!["mv".to_string(), "-f".to_string(), tmp, target.clone()];
    run_user_ok(
        handle,
        user,
        &rename,
        None,
        DEFAULT_MAX_OUTPUT,
        "rename_target",
    )
    .await?;
    let count = vec!["wc".to_string(), "-c".to_string(), target];
    let outcome = run_user_ok(
        handle,
        user,
        &count,
        None,
        DEFAULT_MAX_OUTPUT,
        "write_verify",
    )
    .await?;
    let size = parse_byte_count(&outcome.stdout_text())?;
    if size == content.len() {
        Ok(())
    } else {
        Err(SandboxError::CommandFailed {
            operation: "write_file_verify",
            exit_code: None,
            stderr: format!(
                "size mismatch: expected {} bytes, found {size}",
                content.len()
            ),
        })
    }
}

/// Reads a workspace file as the user, capped at the execution output limit.
pub async fn read_file(
    handle: &SandboxHandle,
    user: &EphemeralUsername,
    path: &WorkspacePath,
) -> Result<Vec<u8>, SandboxError> {
    let target = path.container_path();
    let probe = vec!["test".to_string(), "-f".to_string(), target.clone()];
    let probe_out = run_user(handle, user, &probe, None, DEFAULT_MAX_OUTPUT).await?;
    if !probe_out.success() {
        // `test` prints nothing on plain-false; docker-level failures print to stderr.
        if !probe_out.stderr.is_empty() || probe_out.exit_code != Some(1) {
            return Err(command_failed("read_probe", &probe_out));
        }
        return Err(SandboxError::ContainerFileNotFound(target));
    }
    checked_dir(handle, user, &target, "realpath_file").await?;
    let count = vec!["wc".to_string(), "-c".to_string(), target.clone()];
    let size_out = run_user_ok(handle, user, &count, None, DEFAULT_MAX_OUTPUT, "read_size").await?;
    let size = parse_byte_count(&size_out.stdout_text())?;
    if size > MAX_FILE_BYTES {
        return Err(SandboxError::FileTooLarge {
            len: size,
            max: MAX_FILE_BYTES,
        });
    }
    if size > EXEC_MAX_OUTPUT {
        return Err(SandboxError::FileTooLarge {
            len: size,
            max: EXEC_MAX_OUTPUT,
        });
    }
    let cat = vec!["cat".to_string(), target];
    let outcome = run_user_ok(handle, user, &cat, None, EXEC_MAX_OUTPUT, "read_cat").await?;
    Ok(outcome.stdout)
}

/// Writes the statement without touching or pre-creating an empty solution file.
pub async fn write_question(
    handle: &SandboxHandle,
    user: &EphemeralUsername,
    index: usize,
    language: Language,
    statement: &str,
) -> Result<(WorkspacePath, WorkspacePath), SandboxError> {
    if statement.len() > MAX_FILE_BYTES {
        return Err(SandboxError::FileTooLarge {
            len: statement.len(),
            max: MAX_FILE_BYTES,
        });
    }
    let (question, solution) = question_layout(index, language)?;
    write_file(handle, user, &question, statement.as_bytes()).await?;
    Ok((question, solution))
}
