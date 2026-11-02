use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use clap::Parser;
use pairproof::distribution::release::{
    asset_url, checksum_url, current_platform_asset, ReleaseClient, REPO_API_LATEST,
};
use pairproof::distribution::startup::{
    reexec_args, should_run_startup_check, startup_check, startup_gate_with, Reexecer, StartupGate,
    StartupOutcome, REEXEC_GUARD_ENV,
};
use pairproof::distribution::{parse_version, DistributionError};
use pairproof::tui::args::{Args, Command};
use sha2::{Digest, Sha256};

const NEW_SCRIPT: &[u8] = b"#!/bin/sh\necho \"pairproof 9.9.9\"\n";
const OLD_SCRIPT: &[u8] = b"#!/bin/sh\necho \"pairproof 0.1.0\"\n";

struct FakeClient {
    urls: Arc<Mutex<Vec<String>>>,
    text: HashMap<String, String>,
    bytes: HashMap<String, Vec<u8>>,
    fail_bytes: bool,
}

impl ReleaseClient for FakeClient {
    fn get_text(&self, url: &str) -> Result<String, DistributionError> {
        self.urls.lock().expect("urls lock").push(url.to_string());
        self.text
            .get(url)
            .cloned()
            .ok_or_else(|| DistributionError::Http {
                url: url.to_string(),
                message: "no canned text response".to_string(),
            })
    }
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, DistributionError> {
        self.urls.lock().expect("urls lock").push(url.to_string());
        if self.fail_bytes {
            return Err(DistributionError::Http {
                url: url.to_string(),
                message: "simulated download failure".to_string(),
            });
        }
        self.bytes
            .get(url)
            .cloned()
            .ok_or_else(|| DistributionError::Http {
                url: url.to_string(),
                message: "no canned bytes response".to_string(),
            })
    }
}

type ReexecCalls = Arc<Mutex<Vec<(PathBuf, Vec<String>)>>>;

#[derive(Clone, Default)]
struct RecordingReexecer {
    calls: ReexecCalls,
}

impl Reexecer for RecordingReexecer {
    fn reexec(&self, exe: &Path, args: &[String]) -> Result<(), DistributionError> {
        self.calls
            .lock()
            .expect("reexec lock")
            .push((exe.to_path_buf(), args.to_vec()));
        Ok(())
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn make_archive(contents: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    {
        let mut builder = tar::Builder::new(&mut encoder);
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, "pairproof", contents)
            .expect("build tar fixture");
        builder.finish().expect("finish tar fixture");
    }
    encoder.finish().expect("finish gzip fixture")
}

fn unique_dir(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "pairproof-startup-{name}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn write_exe(dir: &Path, contents: &[u8]) -> PathBuf {
    let exe = dir.join("pairproof");
    std::fs::write(&exe, contents).expect("write old exe");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755))
            .expect("chmod old exe");
    }
    exe
}

fn fake_for(tag: &str, archive: Vec<u8>) -> FakeClient {
    let asset = current_platform_asset().expect("supported test platform");
    let checksum = format!("{}  {}\n", sha256_hex(&archive), asset);
    let mut text = HashMap::new();
    text.insert(
        REPO_API_LATEST.to_string(),
        format!("{{\"tag_name\":\"{tag}\"}}"),
    );
    text.insert(checksum_url(tag, asset), checksum);
    let mut bytes = HashMap::new();
    bytes.insert(asset_url(tag, asset), archive);
    FakeClient {
        urls: Arc::new(Mutex::new(Vec::new())),
        text,
        bytes,
        fail_bytes: false,
    }
}

fn current() -> pairproof::distribution::Version {
    parse_version("0.1.0").expect("current version")
}

#[test]
fn startup_check_detects_latest_installed_version() {
    let dir = unique_dir("current");
    let exe = write_exe(&dir, OLD_SCRIPT);
    let client = fake_for("v0.1.0", make_archive(NEW_SCRIPT));
    let urls = client.urls.clone();
    let outcome = startup_check(client, exe.clone(), &current()).expect("startup check");
    assert_eq!(outcome, StartupOutcome::Current);
    assert_eq!(
        urls.lock().expect("urls lock").clone(),
        vec![REPO_API_LATEST.to_string()]
    );
    assert_eq!(std::fs::read(&exe).expect("read exe"), OLD_SCRIPT);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn startup_check_triggers_update_when_outdated() {
    let dir = unique_dir("outdated");
    let exe = write_exe(&dir, OLD_SCRIPT);
    let client = fake_for("v9.9.9", make_archive(NEW_SCRIPT));
    let outcome = startup_check(client, exe.clone(), &current()).expect("startup check");
    assert_eq!(outcome, StartupOutcome::Updated);
    assert_eq!(std::fs::read(&exe).expect("read exe"), NEW_SCRIPT);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn startup_check_skips_archive_download_when_current() {
    let dir = unique_dir("skip-archive");
    let exe = write_exe(&dir, OLD_SCRIPT);
    let client = fake_for("v0.1.0", make_archive(NEW_SCRIPT));
    let urls = client.urls.clone();
    let outcome = startup_check(client, exe, &current()).expect("startup check");
    assert_eq!(outcome, StartupOutcome::Current);
    let requested = urls.lock().expect("urls lock").clone();
    assert!(
        requested
            .iter()
            .all(|u| !u.ends_with(".tar.gz") && !u.ends_with(".sha256")),
        "no archive/checksum URLs: {requested:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn startup_update_preserves_original_cli_arguments() {
    assert_eq!(REEXEC_GUARD_ENV, "PAIRPROOF_AUTO_UPDATE_REEXEC");
    assert!(should_run_startup_check(None));
    assert!(should_run_startup_check(Some("0")));
    assert!(!should_run_startup_check(Some("1")));
    assert_eq!(
        reexec_args(&["--role".to_string(), "expert".to_string()]),
        vec!["--role".to_string(), "expert".to_string()]
    );
    for role in ["expert", "candidate"] {
        let dir = unique_dir(&format!("args-{role}"));
        let exe = write_exe(&dir, OLD_SCRIPT);
        let client = fake_for("v9.9.9", make_archive(NEW_SCRIPT));
        let reexecer = RecordingReexecer::default();
        let calls = reexecer.calls.clone();
        let original = vec!["--role".to_string(), role.to_string()];
        let gate =
            startup_gate_with(client, reexecer, exe, &current(), &original, None).expect("gate");
        assert_eq!(gate, StartupGate::Reexec);
        let recorded = calls.lock().expect("reexec lock").clone();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].1, vec!["--role".to_string(), role.to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn startup_update_launches_new_binary() {
    let dir = unique_dir("launches");
    let exe = write_exe(&dir, OLD_SCRIPT);
    let client = fake_for("v9.9.9", make_archive(NEW_SCRIPT));
    let reexecer = RecordingReexecer::default();
    let calls = reexecer.calls.clone();
    let original = vec!["--role".to_string(), "expert".to_string()];
    let gate = startup_gate_with(client, reexecer, exe.clone(), &current(), &original, None)
        .expect("gate");
    assert_eq!(gate, StartupGate::Reexec);
    let recorded = calls.lock().expect("reexec lock").clone();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].0, exe);
    // Updater staged-binary verification requires `pairproof 9.9.9` from --version.
    assert_eq!(std::fs::read(&exe).expect("read new exe"), NEW_SCRIPT);
    let output = std::process::Command::new(&exe)
        .arg("--version")
        .output()
        .expect("run new binary");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("pairproof 9.9.9"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn startup_update_failure_prevents_tui_launch() {
    // Checksum mismatch: gate errors, reexecer not called, exe unchanged.
    let dir = unique_dir("checksum-fail");
    let exe = write_exe(&dir, OLD_SCRIPT);
    let asset = current_platform_asset().expect("supported test platform");
    let archive = make_archive(NEW_SCRIPT);
    let mut client = fake_for("v9.9.9", archive);
    client.text.insert(
        checksum_url("v9.9.9", asset),
        format!("{}  {}\n", "0".repeat(64), asset),
    );
    let reexecer = RecordingReexecer::default();
    let calls = reexecer.calls.clone();
    let original = vec!["--role".to_string(), "expert".to_string()];
    let result = startup_gate_with(client, reexecer, exe.clone(), &current(), &original, None);
    assert!(
        matches!(result, Err(DistributionError::ChecksumMismatch { .. })),
        "got {result:?}"
    );
    assert!(calls.lock().expect("reexec lock").is_empty());
    assert_eq!(std::fs::read(&exe).expect("read exe"), OLD_SCRIPT);
    let _ = std::fs::remove_dir_all(&dir);
    // Download failure: same guarantees.
    let dir = unique_dir("download-fail");
    let exe = write_exe(&dir, OLD_SCRIPT);
    let mut client = fake_for("v9.9.9", make_archive(NEW_SCRIPT));
    client.fail_bytes = true;
    let reexecer = RecordingReexecer::default();
    let calls = reexecer.calls.clone();
    let result = startup_gate_with(client, reexecer, exe.clone(), &current(), &original, None);
    assert!(result.is_err(), "expected download error, got {result:?}");
    assert!(calls.lock().expect("reexec lock").is_empty());
    assert_eq!(std::fs::read(&exe).expect("read exe"), OLD_SCRIPT);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn startup_check_does_not_run_for_management_commands() {
    // Management commands parse to Some(command); run() dispatches them before the gate.
    // --help/--version exit inside Args::parse() before run()'s gate, so they never check either.
    for argv in [
        vec!["pairproof", "update"],
        vec!["pairproof", "update", "--check"],
        vec!["pairproof", "uninstall"],
        vec!["pairproof", "uninstall", "--yes"],
        vec!["pairproof", "uninstall", "--keep-data"],
        vec!["pairproof", "uninstall", "--yes", "--keep-data"],
    ] {
        let args =
            Args::try_parse_from(argv.clone()).unwrap_or_else(|e| panic!("{argv:?} parses: {e}"));
        assert!(
            matches!(
                args.command,
                Some(Command::Update(_)) | Some(Command::Uninstall(_))
            ),
            "{argv:?} must be a command"
        );
    }
    // Guard-set gate makes zero client calls (loop prevention after re-exec).
    let dir = unique_dir("guard");
    let exe = write_exe(&dir, OLD_SCRIPT);
    let client = fake_for("v9.9.9", make_archive(NEW_SCRIPT));
    let urls = client.urls.clone();
    let reexecer = RecordingReexecer::default();
    let calls = reexecer.calls.clone();
    let original = vec!["--role".to_string(), "expert".to_string()];
    let gate = startup_gate_with(client, reexecer, exe, &current(), &original, Some("1"))
        .expect("guard gate");
    assert_eq!(gate, StartupGate::Proceed);
    assert!(
        urls.lock().expect("urls lock").is_empty(),
        "guard must make zero client calls"
    );
    assert!(calls.lock().expect("reexec lock").is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
