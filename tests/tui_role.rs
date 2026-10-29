use std::process::Command;

use clap::Parser;
use pairproof::tui::args::Args;
use pairproof::tui::candidate::{candidate_sync_message, placeholder_message};
use pairproof::tui::dispatch_role;
use pairproof::tui::shared::{
    destination_for, parse_role, sanitize_display, Role, RoleDestination,
};

#[test]
fn role_selection_dispatches_expert() {
    assert_eq!(parse_role("expert"), Some(Role::Expert));
    assert_eq!(destination_for(Role::Expert), RoleDestination::Expert);
    assert_eq!(dispatch_role(Role::Expert), RoleDestination::Expert);
}

#[test]
fn candidate_role_does_not_enter_expert_flow() {
    assert_eq!(parse_role("candidate"), Some(Role::Candidate));
    assert_eq!(destination_for(Role::Candidate), RoleDestination::Candidate);
    assert_eq!(dispatch_role(Role::Candidate), RoleDestination::Candidate);
    assert_ne!(dispatch_role(Role::Candidate), RoleDestination::Expert);
    assert!(!placeholder_message().contains("Phase 9"));
    assert!(!placeholder_message().contains("not implemented"));
}

#[test]
fn candidate_sync_error_is_generic() {
    assert_eq!(
        candidate_sync_message(),
        "Sync problem \u{2014} ask the expert"
    );
    let msg = candidate_sync_message();
    assert!(!msg.contains("ssh"));
    assert!(!msg.contains("/home"));
    assert!(!msg.contains("session"));
    assert!(!msg.contains("203."));
    assert!(!msg.contains("192.168"));
    assert!(!msg.contains("target"));
}

#[test]
fn candidate_placeholder_state() {
    assert_eq!(
        placeholder_message(),
        "Candidate: run pairproof --role candidate in a terminal, \
         then paste the connection string from the expert."
    );
}

#[test]
fn parse_role_edge_cases() {
    assert_eq!(parse_role("expert"), Some(Role::Expert));
    assert_eq!(parse_role(" Expert "), Some(Role::Expert));
    assert_eq!(parse_role("EXPERT"), Some(Role::Expert));
    assert_eq!(parse_role("e"), Some(Role::Expert));
    assert_eq!(parse_role("E"), Some(Role::Expert));
    assert_eq!(parse_role("1"), Some(Role::Expert));
    assert_eq!(parse_role("candidate"), Some(Role::Candidate));
    assert_eq!(parse_role(" Candidate "), Some(Role::Candidate));
    assert_eq!(parse_role("CANDIDATE"), Some(Role::Candidate));
    assert_eq!(parse_role("c"), Some(Role::Candidate));
    assert_eq!(parse_role("C"), Some(Role::Candidate));
    assert_eq!(parse_role("2"), Some(Role::Candidate));
    assert_eq!(parse_role(""), None);
    assert_eq!(parse_role("   "), None);
    assert_eq!(parse_role("bogus"), None);
    assert_eq!(parse_role("experts"), None);
    assert_eq!(parse_role("3"), None);
    assert_eq!(parse_role("0"), None);
}

#[test]
fn invalid_role_handling() {
    assert_eq!(parse_role("admin"), None);
    assert_eq!(parse_role("expert candidate"), None);
    let err = Args::try_parse_from(["pairproof", "--role", "bogus"]);
    assert!(err.is_err());
    let err2 = Args::try_parse_from(["pairproof", "--role", "admin"]);
    assert!(err2.is_err());
    // Valid roles parse without panic.
    let ok = Args::try_parse_from(["pairproof", "--role", "expert"]).expect("expert parses");
    assert_eq!(ok.role, Some(Role::Expert));
    let ok = Args::try_parse_from(["pairproof", "--role", "candidate"]).expect("candidate parses");
    assert_eq!(ok.role, Some(Role::Candidate));
    let none = Args::try_parse_from(["pairproof"]).expect("no role parses");
    assert_eq!(none.role, None);
}

#[test]
fn sanitize_display_strips_escape_and_control() {
    assert_eq!(sanitize_display("\u{1b}[31mred\u{1b}[0m"), "red");
    assert_eq!(sanitize_display("hello\u{7}world"), "helloworld");
    assert_eq!(sanitize_display("a\u{0}b\u{1f}c\u{7f}d"), "abcd");
    assert_eq!(sanitize_display("\u{1b}[2Jcleared"), "cleared");
    assert!(!sanitize_display("\u{1b}[31mx").contains('\u{1b}'));
    assert_eq!(sanitize_display("a\rb"), "ab");
}

#[test]
fn sanitize_display_preserves_printable() {
    assert_eq!(sanitize_display("hello world"), "hello world");
    assert_eq!(sanitize_display("line1\nline2\ttab"), "line1\nline2\ttab");
    assert_eq!(
        sanitize_display("caf\u{e9} \u{2014} ok"),
        "caf\u{e9} \u{2014} ok"
    );
    assert_eq!(sanitize_display(""), "");
}

#[test]
fn binary_candidate_role_shows_candidate_hint() {
    let output = Command::new(env!("CARGO_BIN_EXE_pairproof"))
        .args(["--role", "candidate"])
        .output()
        .expect("binary runs");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
    assert!(stdout.contains("pairproof"));
    assert!(stdout.contains("Candidate"));
    assert!(stdout.contains("connection string"));
    assert!(!stdout.contains("Phase 9"));
    assert!(!stdout.contains("not implemented"));
    assert_eq!(stdout.lines().count(), 1);
}

#[test]
fn binary_expert_role_shows_expert_hint() {
    let output = Command::new(env!("CARGO_BIN_EXE_pairproof"))
        .args(["--role", "expert"])
        .output()
        .expect("binary runs");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
    assert!(stdout.contains("pairproof"));
    assert!(stdout.contains("expert"));
}

#[test]
fn binary_invalid_role_fails() {
    let output = Command::new(env!("CARGO_BIN_EXE_pairproof"))
        .args(["--role", "bogus"])
        .output()
        .expect("binary runs");
    assert!(!output.status.success());
}

#[test]
fn binary_no_args_still_prints_pairproof() {
    let output = Command::new(env!("CARGO_BIN_EXE_pairproof"))
        .output()
        .expect("binary runs");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
    assert!(stdout.contains("pairproof"));
}
