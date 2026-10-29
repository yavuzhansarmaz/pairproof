use crate::connection::{Endpoint, EphemeralUsername};

use super::error::SandboxError;
use super::process::CommandSpec;
use super::validate::{container_name, network_name, validate_container_name};

/// Returns the versioned sandbox image tag for this crate.
pub fn image_tag() -> String {
    format!("pairproof-sandbox:{}", env!("CARGO_PKG_VERSION"))
}

/// Rejects values that could be parsed as a docker flag or split as words.
fn validate_plain_value(value: &str) -> Result<(), SandboxError> {
    let bad = value.is_empty()
        || value.starts_with('-')
        || value.chars().any(|c| c.is_whitespace() || c.is_control());
    if bad {
        Err(SandboxError::InvalidCommand(value.to_string()))
    } else {
        Ok(())
    }
}

/// Builds `docker --version`.
pub fn docker_version() -> CommandSpec {
    CommandSpec::new("docker", ["--version"])
}

/// Builds the daemon probe printing only the server version.
pub fn docker_info() -> CommandSpec {
    CommandSpec::new("docker", ["info", "--format", "{{.ServerVersion}}"])
}

/// Builds an image presence check, validating `tag` as a plain argv value.
pub fn image_inspect(tag: &str) -> Result<CommandSpec, SandboxError> {
    validate_plain_value(tag)?;
    Ok(CommandSpec::new("docker", ["image", "inspect", tag]))
}

/// Builds an image build, validating `tag` and `context_dir` as plain argv values.
pub fn image_build(tag: &str, context_dir: &str) -> Result<CommandSpec, SandboxError> {
    validate_plain_value(tag)?;
    validate_plain_value(context_dir)?;
    Ok(CommandSpec::new(
        "docker",
        ["build", "-t", tag, context_dir],
    ))
}

/// Builds creation of the per-session plain bridge network.
pub fn network_create(session_id: &str) -> Result<CommandSpec, SandboxError> {
    let net = network_name(session_id)?;
    Ok(CommandSpec::new("docker", ["network", "create", &net]))
}

/// Builds removal of the per-session network.
pub fn network_remove(session_id: &str) -> Result<CommandSpec, SandboxError> {
    let net = network_name(session_id)?;
    Ok(CommandSpec::new("docker", ["network", "rm", &net]))
}

/// Builds the hardened container start with exactly one published port.
pub fn container_run(
    session_id: &str,
    endpoint: &Endpoint,
    tag: &str,
) -> Result<CommandSpec, SandboxError> {
    validate_plain_value(tag)?;
    let name = container_name(session_id)?;
    let net = network_name(session_id)?;
    let publish = format!("0.0.0.0:{}:22", endpoint.port());
    Ok(CommandSpec::new(
        "docker",
        [
            "run",
            "-d",
            "--name",
            &name,
            "--network",
            &net,
            "--memory",
            "512m",
            "--cpus",
            "1",
            "--pids-limit",
            "256",
            "--cap-drop",
            "NET_RAW",
            "--cap-add",
            "NET_ADMIN",
            "-p",
            &publish,
            tag,
        ],
    ))
}

/// Builds forced removal of the session container.
pub fn container_remove(session_id: &str) -> Result<CommandSpec, SandboxError> {
    let name = container_name(session_id)?;
    Ok(CommandSpec::new("docker", ["rm", "-f", &name]))
}

/// Validates an in-container argv: non-empty, no NUL, program not flag-like.
fn validate_exec_argv(argv: &[String]) -> Result<(), SandboxError> {
    let bad = argv.is_empty()
        || argv[0].is_empty()
        || argv[0].starts_with('-')
        || argv.iter().any(|a| a.contains('\0'));
    if bad {
        Err(SandboxError::InvalidCommand(
            argv.first().cloned().unwrap_or_default(),
        ))
    } else {
        Ok(())
    }
}

/// Builds `docker exec` as the ephemeral user, carrying `argv` verbatim.
pub fn exec_as(
    container: &str,
    user: &EphemeralUsername,
    argv: &[String],
    interactive: bool,
) -> Result<CommandSpec, SandboxError> {
    validate_container_name(container)?;
    validate_exec_argv(argv)?;
    let mut args: Vec<String> = vec!["exec".to_string()];
    if interactive {
        args.push("-i".to_string());
    }
    args.push("-u".to_string());
    args.push(user.as_str().to_string());
    args.push(container.to_string());
    args.extend(argv.iter().cloned());
    Ok(CommandSpec::new("docker", args))
}

/// Builds `docker exec` as root for provisioning steps only.
pub fn exec_root(
    container: &str,
    argv: &[String],
    interactive: bool,
) -> Result<CommandSpec, SandboxError> {
    validate_container_name(container)?;
    validate_exec_argv(argv)?;
    let mut args: Vec<String> = vec!["exec".to_string()];
    if interactive {
        args.push("-i".to_string());
    }
    args.push(container.to_string());
    args.extend(argv.iter().cloned());
    Ok(CommandSpec::new("docker", args))
}
