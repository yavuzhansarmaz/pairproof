//! Candidate hello evidence sources plus consume-on-accept removal.
//! Reads never trust bytes; consume errors stay generic, never path-derived.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use crate::orchestration::control::{
    session_binding_bytes, FINAL_ACK_FILE, FINAL_ACK_PATH, HEARTBEAT_FILE, HEARTBEAT_PATH,
    OUTBOX_DIR, SESSION_FILE,
};
use crate::orchestration::{ControlError, MAX_CONTROL_READ_BYTES};
use crate::sandbox::SandboxHandle;

/// Boxed hello-evidence read; a missing file reads as `None`.
pub type HelloFuture<'a> = Pin<Box<dyn Future<Output = Option<Vec<u8>>> + Send + 'a>>;
/// Boxed hello consume; `NotFound` counts as success (idempotent).
pub type ConsumeFuture<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;
/// Boxed session-binding publish; failures stay expert-side.
#[rustfmt::skip]
pub type SessionPublishFuture<'a> = Pin<Box<dyn Future<Output = Result<(), ControlError>> + Send + 'a>>;

/// Reads the candidate hello evidence file without trusting its bytes.
#[rustfmt::skip]
pub trait HelloSource: Send + Sync {
    fn read_hello<'a>(&'a self) -> HelloFuture<'a>;
    fn consume_hello<'a>(&'a self) -> ConsumeFuture<'a>;
    fn read_heartbeat<'a>(&'a self) -> HelloFuture<'a> { Box::pin(async move { None }) }
    fn read_final_ack<'a>(&'a self) -> HelloFuture<'a> { Box::pin(async move { None }) }
    fn publish_session_binding<'a>(&'a self, _session_id: &'a str) -> SessionPublishFuture<'a> { Box::pin(async move { Ok(()) }) }
}

/// Hello source over a local control root (tests and tooling).
#[derive(Debug, Clone)]
#[rustfmt::skip]
pub struct LocalHelloSource { root: PathBuf }
#[rustfmt::skip]
impl LocalHelloSource { pub fn new(root: PathBuf) -> Self { Self { root } } }
#[rustfmt::skip]
impl HelloSource for LocalHelloSource {
    fn read_hello<'a>(&'a self) -> HelloFuture<'a> { Box::pin(async move { std::fs::read(self.root.join("inbox").join("hello.json")).ok() }) }
    fn consume_hello<'a>(&'a self) -> ConsumeFuture<'a> { Box::pin(async move { match std::fs::remove_file(self.root.join("inbox").join("hello.json")) { Ok(()) => Ok(()), Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()), Err(_) => Err("hello consume failed".to_string()) } }) }
    fn read_heartbeat<'a>(&'a self) -> HelloFuture<'a> { Box::pin(async move { std::fs::read(self.root.join("inbox").join(HEARTBEAT_FILE)).ok() }) }
    fn read_final_ack<'a>(&'a self) -> HelloFuture<'a> { Box::pin(async move { std::fs::read(self.root.join("inbox").join(FINAL_ACK_FILE)).ok() }) }
    fn publish_session_binding<'a>(&'a self, session_id: &'a str) -> SessionPublishFuture<'a> { let root = self.root.clone(); let id = session_id.to_string(); Box::pin(async move { let dir = root.join("outbox"); std::fs::create_dir_all(&dir)?; std::fs::write(dir.join(SESSION_FILE), session_binding_bytes(&id))?; Ok(()) }) }
}

/// Hello source reading the container control inbox via root exec.
#[derive(Debug, Clone)]
#[rustfmt::skip]
pub struct DockerHelloSource { handle: SandboxHandle }
#[rustfmt::skip]
impl DockerHelloSource { pub fn new(handle: SandboxHandle) -> Self { Self { handle } } }
#[rustfmt::skip]
impl HelloSource for DockerHelloSource {
    fn read_hello<'a>(&'a self) -> HelloFuture<'a> {
        Box::pin(async move { let argv = vec!["cat".to_string(), "/pairproof-control/inbox/hello.json".to_string()]; let spec = crate::sandbox::command::exec_root(self.handle.container_name(), &argv, false).ok()?; let outcome = crate::sandbox::process::run_bounded(&spec, None, Duration::from_secs(5), MAX_CONTROL_READ_BYTES).await.ok()?; if outcome.success() { Some(outcome.stdout) } else { None } })
    }
    fn consume_hello<'a>(&'a self) -> ConsumeFuture<'a> {
        Box::pin(async move { let argv = vec!["rm".to_string(), "-f".to_string(), "/pairproof-control/inbox/hello.json".to_string()]; let spec = crate::sandbox::command::exec_root(self.handle.container_name(), &argv, false).map_err(|_| "hello consume failed".to_string())?; let outcome = crate::sandbox::process::run_bounded(&spec, None, Duration::from_secs(5), MAX_CONTROL_READ_BYTES).await.map_err(|_| "hello consume failed".to_string())?; if outcome.success() { Ok(()) } else { Err("hello consume failed".to_string()) } })
    }
    fn read_final_ack<'a>(&'a self) -> HelloFuture<'a> { Box::pin(async move { let argv = vec!["cat".to_string(), FINAL_ACK_PATH.to_string()]; let spec = crate::sandbox::command::exec_root(self.handle.container_name(), &argv, false).ok()?; let outcome = crate::sandbox::process::run_bounded(&spec, None, Duration::from_secs(5), MAX_CONTROL_READ_BYTES).await.ok()?; if outcome.success() { Some(outcome.stdout) } else { None } }) }
    fn read_heartbeat<'a>(&'a self) -> HelloFuture<'a> { Box::pin(async move { let argv = vec!["cat".to_string(), HEARTBEAT_PATH.to_string()]; let spec = crate::sandbox::command::exec_root(self.handle.container_name(), &argv, false).ok()?; let outcome = crate::sandbox::process::run_bounded(&spec, None, Duration::from_secs(5), MAX_CONTROL_READ_BYTES).await.ok()?; if outcome.success() { Some(outcome.stdout) } else { None } }) }
    fn publish_session_binding<'a>(&'a self, session_id: &'a str) -> SessionPublishFuture<'a> { let handle = self.handle.clone(); let id = session_id.to_string(); Box::pin(async move { write_session_docker(&handle, &id).await }) }
}

#[rustfmt::skip]
pub(crate) async fn write_session_docker(handle: &SandboxHandle, session_id: &str) -> Result<(), ControlError> { let bytes = session_binding_bytes(session_id); let target = format!("{OUTBOX_DIR}/{SESSION_FILE}"); let spec = crate::sandbox::command::exec_root(handle.container_name(), &["tee".to_string(), target], true).map_err(|e| ControlError::Io(e.to_string()))?; let outcome = crate::sandbox::process::run_bounded(&spec, Some(&bytes), Duration::from_secs(5), MAX_CONTROL_READ_BYTES).await.map_err(|e| ControlError::Io(e.to_string()))?; if outcome.success() { Ok(()) } else { Err(ControlError::Io("session publish failed".into())) } }
