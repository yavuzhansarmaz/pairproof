//! Widgets and rendering shared by expert and candidate screens.

use crate::orchestration::{Clock, SystemClock};

/// Current time in milliseconds; the single definition both the
/// expert and candidate loops delegate to.
pub fn now_ms() -> u64 {
    SystemClock.now_ms()
}

/// Returns `text` without its last character; empty stays empty.
pub(crate) fn pop_last(text: &str) -> String {
    let mut owned = text.to_string();
    owned.pop();
    owned
}

/// Role selected at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Expert,
    Candidate,
}

/// Destination a role is routed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleDestination {
    Expert,
    Candidate,
}

/// Parses a role from trimmed case-insensitive text.
pub fn parse_role(input: &str) -> Option<Role> {
    match input.trim().to_ascii_lowercase().as_str() {
        "expert" | "e" | "1" => Some(Role::Expert),
        "candidate" | "c" | "2" => Some(Role::Candidate),
        _ => None,
    }
}

/// Returns the destination for a role.
pub fn destination_for(role: Role) -> RoleDestination {
    match role {
        Role::Expert => RoleDestination::Expert,
        Role::Candidate => RoleDestination::Candidate,
    }
}

/// Standard base64 encoding (with padding), for OSC 52 clipboard
/// payloads.
pub fn base64_encode(data: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = data.as_bytes();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from_be_bytes([0, b[0], b[1], b[2]]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Strips terminal control and escape sequences for display.
/// Ownership rule: each line producer sanitizes exactly once at its
/// own boundary; aggregators and TTY loops must not re-sanitize lines
/// they only collect from other producers.
pub fn sanitize_display(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    // Consume parameter and intermediate bytes.
                    while let Some(&n) = chars.peek() {
                        if ('\u{30}'..='\u{3f}').contains(&n) || ('\u{20}'..='\u{2f}').contains(&n)
                        {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    // Consume the final byte of the CSI sequence.
                    if let Some(&n) = chars.peek() {
                        if ('\u{40}'..='\u{7e}').contains(&n) {
                            chars.next();
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    // Consume until BEL or ESC backslash terminator.
                    let mut prev_esc = false;
                    for n in chars.by_ref() {
                        if n == '\u{7}' {
                            break;
                        }
                        if prev_esc && n == '\\' {
                            break;
                        }
                        prev_esc = n == '\u{1b}';
                    }
                }
                Some(_) => {
                    // Two-character escape sequence.
                    chars.next();
                }
                None => {}
            }
            continue;
        }
        if c == '\n' || c == '\t' {
            out.push(c);
            continue;
        }
        if ('\u{0}'..='\u{1f}').contains(&c) || c == '\u{7f}' {
            continue;
        }
        out.push(c);
    }
    out
}
