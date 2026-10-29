use std::fmt;

use rand::rngs::OsRng;
use rand::RngCore;

use super::error::ConnectionError;

/// A 128-bit one-time secret, never shown by `Debug` and with no `Display` impl.
#[derive(Clone)]
pub struct Secret {
    bytes: [u8; 16],
}

impl Secret {
    /// Generates a fresh secret from the operating system RNG.
    pub fn generate() -> Self {
        let mut bytes = [0u8; 16];
        OsRng.fill_bytes(&mut bytes);
        Self { bytes }
    }

    /// Parses exactly 32 lowercase hex characters.
    pub fn from_hex(raw: &str) -> Result<Self, ConnectionError> {
        if raw.len() != 32 || !raw.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(ConnectionError::InvalidSecret);
        }
        let raw = raw.as_bytes();
        let mut bytes = [0u8; 16];
        for i in 0..16 {
            bytes[i] = (hex_value(raw[2 * i]) << 4) | hex_value(raw[2 * i + 1]);
        }
        Ok(Self { bytes })
    }

    /// Builds a secret from fixed bytes, mainly for tests.
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self { bytes }
    }

    /// Returns the secret as 32 lowercase hex characters for encoding.
    pub fn expose_hex(&self) -> String {
        let mut out = String::with_capacity(32);
        for byte in self.bytes {
            out.push(hex_char(byte >> 4));
            out.push(hex_char(byte & 0x0f));
        }
        out
    }
}

fn hex_value(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        _ => b - b'a' + 10,
    }
}

fn hex_char(nibble: u8) -> char {
    match nibble {
        0..=9 => char::from(b'0' + nibble),
        _ => char::from(b'a' + nibble - 10),
    }
}

impl PartialEq for Secret {
    fn eq(&self, other: &Self) -> bool {
        let mut diff = 0u8;
        for i in 0..16 {
            diff |= self.bytes[i] ^ other.bytes[i];
        }
        diff == 0
    }
}

impl Eq for Secret {}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Secret([REDACTED])")
    }
}
