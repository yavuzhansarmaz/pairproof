# Testing guide

How to run and understand the PairProof test suites. Every command below is verified against [Cargo.toml](../../Cargo.toml) and [.github/workflows/ci.yml](../../.github/workflows/ci.yml). User-facing build instructions are in [README.md](../../README.md).

## Standard gates (hermetic, run on pull requests and pushes to main)

```sh
cargo build
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

`cargo test` runs the unit tests embedded in `src/` plus the integration suites in [tests/](../../tests/). Those suites are modules of one `integration` test binary (`tests/common_mod.rs` is a shared helper, not a suite). Cargo would otherwise compile every file as its own crate, which dominated CI. The default run is hermetic: it needs no network, Docker daemon, Meta API key, or LeetCode access. CI's `check` job runs exactly `cargo test`, `cargo fmt --check`, and `cargo clippy --all-targets -- -D warnings` on stable Rust, after installing Mutagen v0.18.1 (three hermetic `orchestrator_sync` tests shell out to the real `mutagen` binary for absent-session paths). The release workflow's validate job installs that same Mutagen build before `cargo test`, then builds `cargo build --release --target <target> --locked` for the four release targets. After CI succeeds on `main`, the release workflow plans a version and stops if that version is already published. If `Cargo.toml` differs from the latest release, that version is published; otherwise the patch number is incremented in `Cargo.toml` and `Cargo.lock`.

Run a single suite by its module name, for example:

```sh
cargo test scoring_live
cargo test tui_expert_recovery
```

## Test structure

- **Unit tests** live in `src/` modules (scoring step tables, connection-string parsing, config/key handling, lifecycle and recovery state machines, and similar pure logic).
- **Integration suites** in [tests/](../../tests/) are grouped by area: `connection_*` (connection string, lifecycle, reissue), `core_*` (session, artifacts, event log, records), `ingest_*`, `leetcode_*`, `llm_*` (approaches, chat, extraction, grading), `orchestrator_*` (session, control, connection gate, coding, chat, lifecycle, health, recovery, finalize, report, evidence, cleanup, SSH/Mutagen auth, sync, e2e, failures), `sandbox_*` (argv, validate, preflight, lifecycle, workspace, exec), `scoring_*` (live, final, consistency, LLM composition), `sync_*` (argv, ensure, lifecycle, failure, paste), `tui_*` (app, role, config, setup, connection, selection, monitor, candidate flow/runtime/sync/chat/final, expert runtime/recovery, polish/viewport, reconnect, live sessions), `distribution_*` (startup, update, uninstall, version, docs), plus `release_workflow.rs`, `version_handshake.rs`, and `smoke.rs`.
- Suites named `*_live`, `meta_api_live`, and `llm_live_smoke`, and individual tests marked `#[ignore = "live ..."]`, are **live tests** (below) and are skipped by the default `cargo test`.

## Live tests (manual, ignored by default)

```sh
cargo test -- --ignored
```

Live tests require real external resources and are marked `#[ignore]` with a reason naming what they need, for example `live docker+ssh`, `live mutagen`, `requires ssh-keyscan on PATH`, or `requires MODEL_API_KEY`. Depending on the suite they need: a running Docker daemon, Mutagen and `ssh`/`ssh-keyscan` on `PATH`, network access to LeetCode and the Meta Model API, and `MODEL_API_KEY` set in the process environment (source or export it first; the binary and tests do not parse a `.env` file).

In CI the `live` job is manual-only (`workflow_dispatch`): it checks that the `MODEL_API_KEY` repository secret is present, Docker and Mutagen are available, and then runs `cargo test -- --ignored`. It never runs on push or pull request. A full live end-to-end run (Docker + SSH + Mutagen + Meta + LeetCode together) is therefore **unverified** by automated CI; treat live results as manual evidence only.

## Conventions

- No test may require a real API key, Docker daemon, or network unless it is `#[ignore]`d as a live test.
- New behavior ships with maintained tests covering success, error, and edge cases; an endpoint is not done until its tests pass.
- Before submitting changes, all four standard gates above must pass.
