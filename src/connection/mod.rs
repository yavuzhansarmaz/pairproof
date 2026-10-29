//! Connection string, rotating secret, and accept and lock handling.

pub mod error;
pub mod rotator;
pub mod secret;
pub mod string;

pub use error::ConnectionError;
pub use rotator::{ConnectionState, SecretRotator, ROTATION_WINDOW_MS};
pub use secret::Secret;
pub use string::{ConnectionString, Endpoint, EphemeralUsername, Fingerprint};
