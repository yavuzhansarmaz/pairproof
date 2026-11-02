//! Hermetic README / CLI-help consistency tests for distribution docs.

use std::path::PathBuf;
use std::process::Command;

fn readme() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("README.md");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

fn run_help(args: &[&str]) -> String {
    let exe = env!("CARGO_BIN_EXE_pairproof");
    let output = Command::new(exe)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to run {exe} {args:?}: {e}"));
    assert!(
        output.status.success(),
        "{exe} {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("help output is UTF-8")
}

#[test]
fn readme_has_separate_user_and_developer_sections() {
    let text = readme();
    let install = text
        .find("## Install (Users)")
        .expect("README must have '## Install (Users)'");
    let build = text
        .find("## Build (Developers)")
        .expect("README must have '## Build (Developers)'");
    assert!(
        install < build,
        "Install (Users) must come before Build (Developers)"
    );
}

#[test]
fn readme_documents_install_update_uninstall() {
    let text = readme();
    assert!(
        text.contains(
            "curl -fsSL https://raw.githubusercontent.com/yavuzhansarmaz/pairproof/main/install.sh | bash"
        ),
        "README must document the one-line curl install with yavuzhansarmaz"
    );
    assert!(text.contains("pairproof update"), "missing update");
    assert!(text.contains("update --check"), "missing update --check");
    assert!(text.contains("pairproof uninstall"), "missing uninstall");
    assert!(
        text.contains("uninstall --keep-data"),
        "missing uninstall --keep-data"
    );
    assert!(text.contains("~/.local/bin"), "missing ~/.local/bin");
}

#[test]
fn readme_has_no_owner_placeholder_or_delete_alias() {
    let text = readme();
    assert!(!text.contains("<OWNER>"), "README must not contain <OWNER>");
    assert!(
        !text.contains("pairproof delete"),
        "README must not mention a delete alias"
    );
}

#[test]
fn readme_developer_instructions_preserved() {
    let text = readme();
    for needle in [
        "cargo build",
        "cargo test",
        "cargo fmt --check",
        "cargo clippy --all-targets -- -D warnings",
    ] {
        assert!(text.contains(needle), "README must preserve `{needle}`");
    }
}

#[test]
fn cli_help_lists_update_and_uninstall() {
    let help = run_help(&["--help"]);
    assert!(help.contains("update"), "top-level help: {help}");
    assert!(help.contains("uninstall"), "top-level help: {help}");

    let update_help = run_help(&["update", "--help"]);
    assert!(
        update_help.contains("--check"),
        "update help: {update_help}"
    );

    let uninstall_help = run_help(&["uninstall", "--help"]);
    assert!(
        uninstall_help.contains("--yes"),
        "uninstall help: {uninstall_help}"
    );
    assert!(
        uninstall_help.contains("--keep-data"),
        "uninstall help: {uninstall_help}"
    );
}
