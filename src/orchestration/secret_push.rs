//! Pre-lock secret push: keeps the container password aligned with display.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use crate::connection::{EphemeralUsername, Secret};
use crate::sandbox::SandboxHandle;

/// Pushes one secret into the sandbox; errors stay generic, never secret-derived.
pub trait SecretPush: Send + Sync {
    /// Pushes `secret` as the container password for the ephemeral user.
    fn push<'a>(
        &'a self,
        secret: &'a Secret,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;
}

/// Pushes secrets via `chpasswd` in the session container.
pub struct DockerSecretPush {
    handle: SandboxHandle,
    username: EphemeralUsername,
}

impl DockerSecretPush {
    /// Binds a container handle and ephemeral user for pushes.
    pub fn new(handle: SandboxHandle, username: EphemeralUsername) -> Self {
        Self { handle, username }
    }
}

impl fmt::Debug for DockerSecretPush {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DockerSecretPush {{ .. }}")
    }
}

impl SecretPush for DockerSecretPush {
    fn push<'a>(
        &'a self,
        secret: &'a Secret,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            crate::sandbox::lifecycle::rotate_secret(&self.handle, &self.username, secret)
                .await
                .map_err(|_| "secret push failed".to_string())
        })
    }
}
