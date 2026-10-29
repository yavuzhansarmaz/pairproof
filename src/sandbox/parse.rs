use crate::connection::Fingerprint;

use super::error::SandboxError;
use super::validate::WORKSPACE_ROOT;

fn parse_error(operation: &'static str, detail: impl Into<String>) -> SandboxError {
    SandboxError::ParseError {
        operation,
        detail: detail.into(),
    }
}

/// Extracts the version token from `docker --version` output.
pub fn parse_docker_version(text: &str) -> Result<String, SandboxError> {
    let first = text.lines().next().unwrap_or("").trim();
    let rest = first
        .strip_prefix("Docker version ")
        .ok_or_else(|| parse_error("parse_docker_version", first))?;
    let token = rest
        .split(|c: char| c == ',' || c.is_whitespace())
        .next()
        .unwrap_or("");
    if token.is_empty() {
        return Err(parse_error("parse_docker_version", first));
    }
    Ok(token.to_string())
}

/// Parses the single token printed by `docker info --format`.
pub fn parse_docker_server_version(text: &str) -> Result<String, SandboxError> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_whitespace) {
        return Err(parse_error("parse_docker_server_version", text.trim()));
    }
    Ok(trimmed.to_string())
}

fn b64_value(byte: u8) -> Option<u32> {
    match byte {
        b'A'..=b'Z' => Some(u32::from(byte - b'A')),
        b'a'..=b'z' => Some(u32::from(byte - b'a') + 26),
        b'0'..=b'9' => Some(u32::from(byte - b'0') + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Decodes standard base64, rejecting bad padding and characters.
fn base64_decode(input: &str) -> Result<Vec<u8>, SandboxError> {
    let err = || parse_error("parse_host_key_fingerprint", "invalid base64 key blob");
    let bytes = input.as_bytes();
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return Err(err());
    }
    let pad = bytes.iter().rev().take_while(|&&b| b == b'=').count();
    if pad > 2 {
        return Err(err());
    }
    if bytes[..bytes.len() - pad]
        .iter()
        .any(|&b| b64_value(b).is_none())
    {
        return Err(err());
    }
    if pad > 0 && bytes[..bytes.len() - pad].contains(&b'=') {
        return Err(err());
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let mut vals = [0u32; 4];
        for (i, &b) in chunk.iter().enumerate() {
            vals[i] = if b == b'=' {
                0
            } else {
                b64_value(b).ok_or_else(err)?
            };
        }
        let n = (vals[0] << 18) | (vals[1] << 12) | (vals[2] << 6) | vals[3];
        out.push((n >> 16) as u8);
        if chunk[2] != b'=' {
            out.push((n >> 8) as u8);
        }
        if chunk[3] != b'=' {
            out.push(n as u8);
        }
    }
    if out.is_empty() {
        return Err(err());
    }
    Ok(out)
}

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Computes lowercase hex SHA-256 of `data` without external crates.
fn sha256_hex(data: &[u8]) -> String {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = String::with_capacity(64);
    for word in h {
        out.push_str(&format!("{word:08x}"));
    }
    out
}

/// Derives the host-key fingerprint as hex SHA-256 of the key blob.
pub fn parse_host_key_fingerprint(text: &str) -> Result<Fingerprint, SandboxError> {
    let line = text.lines().next().unwrap_or("").trim();
    let fields: Vec<&str> = line.split_whitespace().collect();
    let is_key_type =
        |s: &str| s.starts_with("ssh-") || s.starts_with("ecdsa-") || s.starts_with("sk-");
    let blob_b64 = if fields.len() >= 2 && is_key_type(fields[0]) {
        fields[1]
    } else if fields.len() >= 3 && is_key_type(fields[1]) {
        fields[2]
    } else {
        return Err(parse_error(
            "parse_host_key_fingerprint",
            "missing key blob field",
        ));
    };
    let blob = base64_decode(blob_b64)?;
    let hex = sha256_hex(&blob);
    Fingerprint::parse(&hex).map_err(|e| parse_error("parse_host_key_fingerprint", e.to_string()))
}

/// Parses `realpath` output as a single absolute path line.
pub fn parse_realpath(text: &str) -> Result<String, SandboxError> {
    let trimmed = text.trim();
    if trimmed.is_empty() || !trimmed.starts_with('/') || trimmed.lines().count() != 1 {
        return Err(parse_error("parse_realpath", trimmed));
    }
    Ok(trimmed.to_string())
}

/// Returns true when a resolved path is the workspace root or below it.
pub fn is_path_within_workspace(resolved: &str) -> bool {
    resolved == WORKSPACE_ROOT || resolved.starts_with(&format!("{WORKSPACE_ROOT}/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn base64_decode_valid_and_rejects_bad() {
        assert_eq!(base64_decode("TWFu").expect("decode"), b"Man");
        assert_eq!(base64_decode("TWE=").expect("decode"), b"Ma");
        assert_eq!(base64_decode("TQ==").expect("decode"), b"M");
        assert!(base64_decode("").is_err());
        assert!(base64_decode("TWF").is_err());
        assert!(base64_decode("TW F").is_err());
        assert!(base64_decode("T===").is_err());
        assert!(base64_decode("####").is_err());
    }

    #[test]
    fn host_key_fingerprint_matches_ssh_keygen() {
        let line = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOr8qXRxd07w1+T+R1EJccWCxO7Z82oSjs0+Knb0KoAA test@example";
        let fp = parse_host_key_fingerprint(line).expect("fingerprint");
        assert_eq!(
            fp.as_str(),
            "d427b935052747c277d2b09d8d6c39ded174e244b2d032c9032b1c994b0689e4"
        );
        let scan = "host.example ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOr8qXRxd07w1+T+R1EJccWCxO7Z82oSjs0+Knb0KoAA";
        assert_eq!(
            parse_host_key_fingerprint(scan).expect("scan").as_str(),
            fp.as_str()
        );
    }

    #[test]
    fn host_key_fingerprint_rejects_malformed() {
        assert!(parse_host_key_fingerprint("").is_err());
        assert!(parse_host_key_fingerprint("ssh-ed25519").is_err());
        assert!(parse_host_key_fingerprint("ssh-ed25519 #### comment").is_err());
        assert!(parse_host_key_fingerprint("ssh-ed25519 TWF comment").is_err());
        assert!(parse_host_key_fingerprint("notakey AAAA comment").is_err());
    }
}
