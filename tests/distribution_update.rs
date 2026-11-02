use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use clap::Parser;
use pairproof::distribution::release::{
    asset_url, checksum_url, current_platform_asset, verify_checksum, ReleaseClient,
    REPO_API_LATEST,
};
use pairproof::distribution::{parse_version, DistributionError, UpdateOutcome, Updater};
use pairproof::tui::args::{Args, Command};
use sha2::{Digest, Sha256};

const NEW_SCRIPT: &[u8] = b"#!/bin/sh\necho \"pairproof 9.9.9\"\n";
const OLD_SCRIPT: &[u8] = b"#!/bin/sh\necho \"pairproof 0.1.0\"\n";

/// Recording fake: serves canned metadata/archive/checksum by URL.
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
        "pairproof-dist-{name}-{}-{nanos}",
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

/// Fake client for tag `tag` serving `archive` with a correct checksum.
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
fn checksum_verification_accepts_valid_archive() {
    let asset = current_platform_asset().expect("supported test platform");
    let archive = make_archive(NEW_SCRIPT);
    let text = format!("{}  {}\n", sha256_hex(&archive), asset);
    // Write through the same bytes path the updater verifies.
    let mut sink = Vec::new();
    sink.write_all(&archive).expect("copy archive");
    verify_checksum(&sink, &text, asset).expect("valid checksum accepted");
}

#[test]
fn checksum_verification_rejects_mismatch() {
    let asset = current_platform_asset().expect("supported test platform");
    let archive = make_archive(NEW_SCRIPT);
    let mut hex = sha256_hex(&archive);
    let replacement = if hex.starts_with('0') { "1" } else { "0" };
    hex.replace_range(0..1, replacement);
    let text = format!("{hex}  {asset}\n");
    let result = verify_checksum(&archive, &text, asset);
    assert!(
        matches!(result, Err(DistributionError::ChecksumMismatch { .. })),
        "expected ChecksumMismatch, got {result:?}"
    );
    // A checksum naming a different asset is rejected before comparison.
    let wrong_name = format!("{}  other.tar.gz\n", sha256_hex(&archive));
    assert!(matches!(
        verify_checksum(&archive, &wrong_name, asset),
        Err(DistributionError::InvalidChecksum { .. })
    ));
}

#[test]
fn checksum_verification_rejects_multibyte_checksum_without_panic() {
    let asset = current_platform_asset().expect("supported test platform");
    // 63 ASCII hex chars + 'é' (2 bytes) puts byte 64 inside the char.
    let text = format!("{}é  {}\n", "a".repeat(63), asset);
    let result = verify_checksum(b"archive", &text, asset);
    assert!(
        matches!(result, Err(DistributionError::InvalidChecksum { .. })),
        "expected InvalidChecksum, got {result:?}"
    );
}

#[test]
fn update_check_does_not_download_archive() {
    let dir = unique_dir("check");
    let exe = write_exe(&dir, OLD_SCRIPT);
    let client = fake_for("v9.9.9", make_archive(NEW_SCRIPT));
    let urls = client.urls.clone();
    let updater = Updater::new(client, exe);
    let outcome = updater.check(&current()).expect("check succeeds");
    assert_eq!(
        outcome,
        UpdateOutcome::CheckOnly {
            update_available: true,
            latest: parse_version("9.9.9").expect("latest"),
        }
    );
    let requested = urls.lock().expect("urls lock").clone();
    assert_eq!(requested, vec![REPO_API_LATEST.to_string()]);
    assert!(
        requested
            .iter()
            .all(|url| !url.ends_with(".tar.gz") && !url.ends_with(".sha256")),
        "check must not request archive/checksum URLs: {requested:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn update_reports_already_up_to_date() {
    let dir = unique_dir("uptodate");
    let exe = write_exe(&dir, OLD_SCRIPT);
    let client = fake_for("v0.1.0", make_archive(NEW_SCRIPT));
    let updater = Updater::new(client, exe.clone());
    let outcome = updater.update(&current()).expect("update check succeeds");
    assert_eq!(outcome, UpdateOutcome::UpToDate);
    assert_eq!(std::fs::read(&exe).expect("read exe"), OLD_SCRIPT);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn update_preserves_old_binary_on_download_failure() {
    let dir = unique_dir("download-failure");
    let exe = write_exe(&dir, OLD_SCRIPT);
    let mut client = fake_for("v9.9.9", make_archive(NEW_SCRIPT));
    client.fail_bytes = true;
    let updater = Updater::new(client, exe.clone());
    let result = updater.update(&current());
    assert!(result.is_err(), "expected download error, got {result:?}");
    assert_eq!(std::fs::read(&exe).expect("read exe"), OLD_SCRIPT);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn update_preserves_old_binary_on_checksum_failure() {
    let dir = unique_dir("checksum-failure");
    let exe = write_exe(&dir, OLD_SCRIPT);
    let asset = current_platform_asset().expect("supported test platform");
    let archive = make_archive(NEW_SCRIPT);
    let mut client = fake_for("v9.9.9", archive.clone());
    client.text.insert(
        checksum_url("v9.9.9", asset),
        format!("{}  {}\n", "0".repeat(64), asset),
    );
    let updater = Updater::new(client, exe.clone());
    let result = updater.update(&current());
    assert!(
        matches!(result, Err(DistributionError::ChecksumMismatch { .. })),
        "expected ChecksumMismatch, got {result:?}"
    );
    assert_eq!(std::fs::read(&exe).expect("read exe"), OLD_SCRIPT);
    let entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("list dir")
        .filter_map(Result::ok)
        .collect();
    assert_eq!(entries.len(), 1, "no staged temp file may remain");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn update_preserves_old_binary_on_replace_failure() {
    let dir = unique_dir("replace-failure");
    // The "executable" path is an existing directory: rename onto it fails.
    let exe = dir.join("pairproof");
    std::fs::create_dir(&exe).expect("create exe directory");
    let client = fake_for("v9.9.9", make_archive(NEW_SCRIPT));
    let updater = Updater::new(client, exe.clone());
    let result = updater.update(&current());
    assert!(
        matches!(
            result,
            Err(DistributionError::ReplaceFailed { .. })
                | Err(DistributionError::PermissionDenied { .. })
        ),
        "expected typed replace error, got {result:?}"
    );
    assert!(exe.is_dir(), "original path must be preserved");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn update_reports_permission_error() {
    let dir = unique_dir("permission");
    let exe = write_exe(&dir, OLD_SCRIPT);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555))
            .expect("make dir read-only");
    }
    let client = fake_for("v9.9.9", make_archive(NEW_SCRIPT));
    let updater = Updater::new(client, exe.clone());
    let result = updater.update(&current());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755));
    }
    match result {
        Err(DistributionError::PermissionDenied { .. }) => {
            assert_eq!(std::fs::read(&exe).expect("read exe"), OLD_SCRIPT);
        }
        Err(other) => panic!("expected PermissionDenied, got {other:?}"),
        // Root (or an ACL) can write despite mode bits; nothing to assert.
        Ok(_) => return,
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn update_uses_correct_platform_asset() {
    let dir = unique_dir("platform-asset");
    let exe = write_exe(&dir, OLD_SCRIPT);
    let asset = current_platform_asset().expect("supported test platform");
    let client = fake_for("v9.9.9", make_archive(NEW_SCRIPT));
    let urls = client.urls.clone();
    let updater = Updater::new(client, exe.clone());
    let outcome = updater.update(&current()).expect("update succeeds");
    assert_eq!(
        outcome,
        UpdateOutcome::Updated {
            from: current(),
            to: parse_version("9.9.9").expect("latest"),
        }
    );
    assert_eq!(std::fs::read(&exe).expect("read new exe"), NEW_SCRIPT);
    let requested = urls.lock().expect("urls lock").clone();
    assert_eq!(requested[0], REPO_API_LATEST);
    assert!(
        requested.contains(&asset_url("v9.9.9", asset)),
        "expected platform asset URL for {asset} in {requested:?}"
    );
    assert!(requested.contains(&checksum_url("v9.9.9", asset)));
    assert_eq!(
        asset_url("v9.9.9", asset),
        format!("https://github.com/yavuzhansarmaz/pairproof/releases/download/v9.9.9/{asset}")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_args_preserve_existing_and_update_forms() {
    let plain = Args::try_parse_from(["pairproof"]).expect("no args parse");
    assert!(plain.role.is_none());
    assert!(plain.command.is_none());

    let expert = Args::try_parse_from(["pairproof", "--role", "expert"]).expect("role parses");
    assert!(expert.command.is_none());

    let candidate =
        Args::try_parse_from(["pairproof", "--role", "candidate"]).expect("role parses");
    assert!(candidate.command.is_none());

    let update = Args::try_parse_from(["pairproof", "update"]).expect("update parses");
    match update.command {
        Some(Command::Update(args)) => assert!(!args.check),
        other => panic!("expected Update command, got {other:?}"),
    }

    let check = Args::try_parse_from(["pairproof", "update", "--check"]).expect("check parses");
    match check.command {
        Some(Command::Update(args)) => assert!(args.check),
        other => panic!("expected Update command, got {other:?}"),
    }

    // Role plus subcommand parses; run() handles the subcommand and
    // never enters the TUI.
    let both =
        Args::try_parse_from(["pairproof", "--role", "expert", "update"]).expect("both parse");
    assert!(matches!(both.command, Some(Command::Update(_))));

    // No `delete` alias exists in this slice.
    assert!(Args::try_parse_from(["pairproof", "delete"]).is_err());
}
