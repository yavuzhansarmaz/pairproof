use std::process::Command;

#[test]
fn binary_prints_startup_message() {
    let output = Command::new(env!("CARGO_BIN_EXE_pairproof"))
        .output()
        .expect("binary runs");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf8");
    assert!(stdout.contains("pairproof"));
}

#[test]
fn binary_help_succeeds() {
    let output = Command::new(env!("CARGO_BIN_EXE_pairproof"))
        .arg("--help")
        .output()
        .expect("binary runs");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf8");
    assert!(stdout.contains("--role"), "help lists --role: {stdout}");
    assert!(
        stdout.contains("expert"),
        "help names expert role: {stdout}"
    );
    assert!(
        stdout.contains("candidate"),
        "help names candidate role: {stdout}"
    );
}
