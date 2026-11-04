use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn manifest_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn read_file(relative: &str) -> String {
    let path = manifest_path(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
}

fn workflow() -> String {
    read_file(".github/workflows/release.yml")
}

fn script() -> String {
    read_file("scripts/check_release_assets.sh")
}

fn cargo_toml() -> String {
    read_file("Cargo.toml")
}

fn runner_for_target(workflow_text: &str, target: &str) -> String {
    let lines: Vec<&str> = workflow_text.lines().collect();
    let target_line = format!("- target: {target}");
    let index = lines
        .iter()
        .position(|line| line.trim() == target_line)
        .unwrap_or_else(|| panic!("missing matrix entry for target: {target}"));
    lines
        .get(index + 1)
        .and_then(|line| line.trim().strip_prefix("runner:"))
        .map(|runner| runner.trim().to_string())
        .unwrap_or_else(|| panic!("matrix entry for {target} must have a runner on the next line"))
}

fn publish_section(workflow_text: &str) -> &str {
    let start = workflow_text
        .find("\n  publish:")
        .expect("workflow must contain a publish job");
    &workflow_text[start..]
}

const EXPECTED_ARCHIVES: [&str; 4] = [
    "pairproof-linux-x86_64-unknown-linux-musl.tar.gz",
    "pairproof-linux-aarch64-unknown-linux-musl.tar.gz",
    "pairproof-darwin-x86_64-apple-darwin.tar.gz",
    "pairproof-darwin-aarch64-apple-darwin.tar.gz",
];

#[test]
fn release_workflow_triggers_on_version_tags() {
    let text = workflow();
    assert!(text.contains("tags:"), "workflow must trigger on tags");
    assert!(
        text.contains("'v*.*.*'"),
        "workflow must trigger on v*.*.* tags"
    );
    assert!(
        text.contains("workflow_dispatch:"),
        "workflow must allow workflow_dispatch for validation"
    );
    assert!(
        text.contains("startsWith(github.ref, 'refs/tags/v')"),
        "publish must be gated to tag refs"
    );
    assert!(
        text.contains("workflow_run:") && text.contains("workflows: [\"CI\"]"),
        "a main release must wait for the CI workflow"
    );
    assert!(
        text.contains("workflow_run.conclusion == 'success'"),
        "a main release must not start when CI failed"
    );
    assert!(
        text.contains("scripts/plan_release_version.sh"),
        "a main release must plan the version from Cargo.toml and the latest release"
    );
    assert!(
        text.contains("scripts/set_cargo_version.sh"),
        "a main release must rewrite Cargo.toml before building and publishing"
    );
    assert!(
        text.contains("release not needed"),
        "a main release must stop when the planned version is already published"
    );
    assert!(
        !text.contains("outputs.bump == 'true'") && !text.contains("PLANNED_BUMP\" = \"true\""),
        "workflow must not treat plan bump output as the string true"
    );
    assert!(
        text.contains("PLANNED_BUMP\" = \"bump\"") || text.contains("PLANNED_BUMP\" = 'bump'") || text.contains("$PLANNED_BUMP\" = \"bump\""),
        "publish must commit a Cargo.toml bump when the planner prints bump"
    );
    assert!(
        text.contains("built binary --version mismatch")
            || text.contains("release asset --version mismatch"),
        "release must refuse assets whose --version does not match the tag"
    );
    let publish = publish_section(&text);
    assert!(
        publish.contains("startsWith(github.ref, 'refs/tags/v')"),
        "publish must run for tag refs"
    );
    assert!(
        publish.contains("github.event_name == 'workflow_run'"),
        "publish must run after CI on main"
    );
}

#[test]
fn release_workflow_validates_tag_matches_cargo() {
    let text = workflow();
    assert!(
        text.contains("TAG_VERSION"),
        "validate job must extract TAG_VERSION"
    );
    assert!(
        text.contains("${TAG#v}"),
        "TAG_VERSION must strip the leading v from the tag"
    );
    assert!(
        text.contains("CARGO_VERSION"),
        "validate job must extract CARGO_VERSION"
    );
    assert!(
        text.contains("Cargo.toml"),
        "validate job must read Cargo.toml"
    );
    assert!(
        text.contains("^v[0-9]+\\.[0-9]+\\.[0-9]+$"),
        "validate job must enforce the v<semver> tag pattern"
    );
    assert!(
        text.contains("does not match Cargo.toml version"),
        "validate job must fail when tag and Cargo.toml versions differ"
    );
    assert!(
        text.contains("cargo fmt --check"),
        "validate job must run cargo fmt --check"
    );
    assert!(
        text.contains("cargo clippy --all-targets -- -D warnings"),
        "validate job must run cargo clippy with -D warnings"
    );
    assert!(
        text.contains("cargo test"),
        "validate job must run cargo test"
    );
    let mutagen = text
        .find("mutagen-io/mutagen/releases/download/v0.18.1/mutagen_linux_amd64_v0.18.1.tar.gz")
        .expect("validate job must install Mutagen v0.18.1 before cargo test");
    let cargo_test = text
        .find("cargo test")
        .expect("validate job must run cargo test");
    assert!(
        mutagen < cargo_test,
        "Mutagen must be installed before cargo test"
    );

    let cargo = cargo_toml();
    let cargo_version = cargo
        .lines()
        .find_map(|line| {
            let trimmed = line.trim();
            trimmed
                .strip_prefix("version")
                .and_then(|rest| rest.split('"').nth(1))
        })
        .expect("Cargo.toml must declare a package version");
    assert_eq!(
        cargo_version, "0.1.3",
        "Cargo.toml version must match the published release"
    );
}

#[test]
fn release_asset_names_match_distribution_contract() {
    let workflow_text = workflow();
    let script_text = script();
    let combined = format!("{workflow_text}\n{script_text}");

    for archive in EXPECTED_ARCHIVES {
        assert!(
            combined.contains(archive),
            "missing archive asset name: {archive}"
        );
        let checksum = format!("{archive}.sha256");
        assert!(
            combined.contains(&checksum),
            "missing checksum asset name: {checksum}"
        );
        assert!(
            script_text.contains(archive),
            "check_release_assets.sh must list archive: {archive}"
        );
        assert!(
            script_text.contains(&checksum),
            "check_release_assets.sh must list checksum: {checksum}"
        );
    }

    assert!(
        script_text.contains("pairproof"),
        "verifier must check the single executable name"
    );
    assert!(
        script_text.contains("tar -tzf"),
        "verifier must list archive contents"
    );
}

#[test]
fn release_workflow_builds_four_targets() {
    let text = workflow();

    let targets = [
        "x86_64-unknown-linux-musl",
        "aarch64-unknown-linux-musl",
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
    ];
    for target in targets {
        assert!(text.contains(target), "missing build target: {target}");
    }

    for archive in EXPECTED_ARCHIVES {
        assert!(
            text.contains(archive),
            "workflow matrix must map a target to asset: {archive}"
        );
    }

    assert_eq!(
        text.matches("asset: pairproof-").count(),
        4,
        "build matrix must contain exactly four asset entries"
    );
    assert!(
        text.contains("ubuntu-22.04"),
        "linux builds must use ubuntu-22.04"
    );
    assert!(
        !text.contains("macos-13"),
        "workflow must not use the retired macos-13 runner"
    );
    assert_eq!(
        runner_for_target(&text, "x86_64-apple-darwin"),
        "macos-14",
        "x86_64 darwin build must use macos-14"
    );
    assert_eq!(
        runner_for_target(&text, "aarch64-apple-darwin"),
        "macos-14",
        "aarch64 darwin build must use macos-14"
    );
    assert!(
        text.contains("matrix.runner"),
        "build job must run on the matrix runner"
    );
    assert!(
        text.contains("cargo build --release --target"),
        "build job must build the release binary for the matrix target"
    );
    assert!(
        text.contains("--locked"),
        "build job must build with --locked"
    );
    assert!(
        text.contains("actions/upload-artifact@"),
        "build job must upload artifacts"
    );
}

#[test]
fn release_workflow_publish_requires_all_builds() {
    let text = workflow();
    let publish = publish_section(&text);

    assert!(
        publish.contains("needs: [gate, validate, build, prepare]"),
        "publish must need CI, validation, the builds, and the version plan"
    );
    assert!(
        publish.contains("contents: write"),
        "publish job must be the only job with contents: write"
    );
    assert!(
        text.contains("permissions:\n  contents: read"),
        "top-level permissions must be contents: read"
    );
    assert!(
        publish.contains("actions/download-artifact@"),
        "publish must download all artifacts"
    );
    assert!(
        publish.contains("scripts/check_release_assets.sh"),
        "publish must verify assets with check_release_assets.sh before publishing"
    );
    assert!(
        publish.contains("gh release create"),
        "publish must create the release with gh release create"
    );
    assert!(
        publish.find("check_release_assets.sh") < publish.find("gh release create"),
        "asset verification must run before gh release create"
    );
}

#[test]
fn release_workflow_pins_third_party_actions() {
    let text = workflow();
    let mut uses_count = 0usize;

    for line in text.lines() {
        let trimmed = line.trim();
        let rest = if let Some(rest) = trimmed.strip_prefix("- uses:") {
            rest.trim()
        } else if let Some(rest) = trimmed.strip_prefix("uses:") {
            rest.trim()
        } else {
            continue;
        };
        uses_count += 1;
        let reference = rest.split_whitespace().next().unwrap_or("");
        let sha = reference
            .split('@')
            .nth(1)
            .unwrap_or_else(|| panic!("uses line is not pinned to a SHA: {line}"));
        assert_eq!(
            sha.len(),
            40,
            "uses line must pin a full 40-character SHA: {line}"
        );
        assert!(
            sha.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "uses line must pin a lowercase hex SHA: {line}"
        );
    }

    assert!(
        uses_count >= 3,
        "workflow must use at least checkout, upload-artifact, and download-artifact"
    );
    assert!(
        text.contains("actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683"),
        "checkout must be pinned to the v4.2.2 SHA"
    );
    assert!(
        text.contains("actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02"),
        "upload-artifact must be pinned to the v4.6.2 SHA"
    );
    assert!(
        text.contains("actions/download-artifact@d3f86a106a0bac45b974a628896c90dbdf5c8093"),
        "download-artifact must be pinned to the v4.3.0 SHA"
    );
}

fn plan(current: &str, latest: &str) -> String {
    let output = Command::new("sh")
        .arg(manifest_path("scripts/plan_release_version.sh"))
        .arg(current)
        .arg(latest)
        .output()
        .expect("plan_release_version.sh runs");
    assert!(
        output.status.success(),
        "planner failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("planner output is utf-8")
        .trim()
        .to_string()
}

#[test]
fn main_push_bumps_patch_unless_cargo_version_changed() {
    assert_eq!(plan("0.1.2", "0.1.2"), "0.1.3 bump");
    assert_eq!(plan("0.2.0", "0.1.2"), "0.2.0 keep");
    assert_eq!(plan("0.1.3", "0.1.2"), "0.1.3 keep");
    assert_eq!(plan("0.1.2", ""), "0.1.2 keep");
    // Cargo.toml lagged a published tag (e.g. tag without version rewrite):
    // advance past the published latest, do not re-ship the stale cargo version.
    assert_eq!(plan("0.1.2", "0.1.3"), "0.1.4 bump");
    assert_eq!(plan("0.1.1", "0.1.3"), "0.1.4 bump");
}
