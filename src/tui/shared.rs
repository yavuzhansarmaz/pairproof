//! Widgets and rendering shared by expert and candidate screens.

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

/// Strips terminal control and escape sequences for display.
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
