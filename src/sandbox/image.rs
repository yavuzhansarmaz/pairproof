use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::command::{docker_info, docker_version, image_build, image_inspect};
use super::error::{bounded_stderr, command_failed, map_docker_not_found, SandboxError};
use super::parse::{parse_docker_server_version, parse_docker_version};
use super::process::{run_bounded, CommandSpec, DEFAULT_MAX_OUTPUT, EXEC_MAX_OUTPUT};

/// Timeout in seconds for building the sandbox image.
pub const BUILD_TIMEOUT_SECS: u64 = 600;

const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

static BUILD_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Returns the embedded build context files in build order.
pub fn embedded_build_context() -> [(&'static str, &'static str); 3] {
    [
        ("Dockerfile", include_str!("../../sandbox/Dockerfile")),
        ("entrypoint.sh", include_str!("../../sandbox/entrypoint.sh")),
        ("sshd_config", include_str!("../../sandbox/sshd_config")),
    ]
}

/// Computes FNV-1a 64-bit over `parts`, feeding a 0xff byte between parts.
fn fnv1a64(parts: &[&[u8]]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            hash ^= 0xff;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        for &byte in *part {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

/// Returns the base image tag extended with a content hash of the build context.
pub fn effective_image_tag() -> String {
    let context = embedded_build_context();
    let parts: Vec<&[u8]> = context.iter().map(|(_, text)| text.as_bytes()).collect();
    format!(
        "{}-{:016x}",
        crate::sandbox::command::image_tag(),
        fnv1a64(&parts)
    )
}

/// Runs a docker probe, mapping a missing binary to `DockerNotFound`.
async fn run_docker(
    spec: &CommandSpec,
    timeout: Duration,
    max_output: usize,
) -> Result<super::process::CommandOutcome, SandboxError> {
    run_bounded(spec, None, timeout, max_output)
        .await
        .map_err(map_docker_not_found)
}

/// Verifies the docker CLI and daemon, returning the client version.
pub async fn check_docker_available() -> Result<String, SandboxError> {
    let version_out = run_docker(&docker_version(), PROBE_TIMEOUT, DEFAULT_MAX_OUTPUT).await?;
    if !version_out.success() {
        return Err(command_failed("docker_version", &version_out));
    }
    let client_version = parse_docker_version(&version_out.stdout_text())?;
    let info_out = run_docker(&docker_info(), PROBE_TIMEOUT, DEFAULT_MAX_OUTPUT).await?;
    if !info_out.success() {
        return Err(SandboxError::DaemonUnavailable {
            stderr: bounded_stderr(&info_out.stderr),
        });
    }
    if parse_docker_server_version(&info_out.stdout_text()).is_err() {
        let stderr = bounded_stderr(&info_out.stderr);
        return Err(SandboxError::DaemonUnavailable {
            stderr: if stderr.is_empty() {
                bounded_stderr(&info_out.stdout)
            } else {
                stderr
            },
        });
    }
    Ok(client_version)
}

/// Returns true when an image with `tag` exists locally.
pub async fn image_present(tag: &str) -> Result<bool, SandboxError> {
    let spec = image_inspect(tag)?;
    let outcome = run_docker(&spec, PROBE_TIMEOUT, DEFAULT_MAX_OUTPUT).await?;
    Ok(outcome.success())
}

/// Returns a fresh, not-yet-created build context path.
fn fresh_build_dir() -> PathBuf {
    let counter = BUILD_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "pairproof-build-{}-{counter}-{nanos}",
        std::process::id()
    ))
}

/// Writes the embedded context into `dir`, which must already exist.
fn write_build_context(dir: &Path) -> Result<(), SandboxError> {
    for (name, content) in embedded_build_context() {
        let path = dir.join(name);
        std::fs::write(&path, content).map_err(|e| SandboxError::Io(e.to_string()))?;
        #[cfg(unix)]
        if name == "entrypoint.sh" {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| SandboxError::Io(e.to_string()))?;
        }
    }
    Ok(())
}

/// Ensures the image exists, building it when absent. Ok(true) means it was cached.
pub async fn ensure_image(tag: &str) -> Result<bool, SandboxError> {
    if image_present(tag).await? {
        return Ok(true);
    }
    let dir = fresh_build_dir();
    std::fs::create_dir(&dir).map_err(|e| SandboxError::Io(e.to_string()))?;
    let result = build_from_dir(tag, &dir).await;
    let _ = std::fs::remove_dir_all(&dir);
    result?;
    Ok(false)
}

/// Builds the image from a materialised context directory.
async fn build_from_dir(tag: &str, dir: &Path) -> Result<(), SandboxError> {
    write_build_context(dir)?;
    let dir_str = dir
        .to_str()
        .ok_or_else(|| SandboxError::Io("build context path is not valid UTF-8".to_string()))?;
    let spec = image_build(tag, dir_str)?;
    let outcome = run_docker(
        &spec,
        Duration::from_secs(BUILD_TIMEOUT_SECS),
        EXEC_MAX_OUTPUT,
    )
    .await?;
    if !outcome.success() {
        return Err(command_failed("image_build", &outcome));
    }
    Ok(())
}
