# PairProof

AI-native technical interviewing: live LeetCode questions become the input to a two-sided interview that measures engineering judgment, not typing.

Traditional interviews ban the tools engineers use every day. PairProof does the opposite: the AI writes the code, and the candidate is scored on how they direct it — approach selection, timely intervention, verification discipline, and token efficiency.

## How it works

One Rust binary, two roles on the start screen.

**Expert — runs the interview:**

1. Passes the preflight checklist (Docker, sandbox image, port, external IP).
2. Enters their Meta Model API key. All charges run on the expert's key.
3. Pastes the job description, optionally plus additional notes. The system extracts topics, level, and a proposed language (Python, JavaScript, or SQL); the expert confirms.
4. Gets 5 easy warm-up questions auto-selected, then picks 3 medium + 2 hard live while the candidate works. May add 1 extra question, may end the session at any time.
5. Shares a connection string whose secret rotates every 60 seconds. First connection locks it.
6. Watches a live football-style rating starting at 6.0, then receives a Markdown report with per-question graded evidence.

**Candidate — solves in a sandbox:**

1. Opens an empty folder, runs `pairproof`, selects `Candidate`, pastes the connection string.
2. Lands over SSH in a disposable Docker container on the expert's machine. Files sync back to their folder with Mutagen.
3. Works a split screen per question: left is the scored arena (pick 1 of 4 AI approaches or `Other`, then continue / redirect / change approach / request tests / finish / skip), right is an unscored AI coach for hints that never affects the score.
4. Never sees the score during the session, only the final breakdown at the end.

**Scoring — explainable by construction:**

- Live rating moves only on candidate-controlled events (approach quality, intervention quality, token efficiency vs. the locked expectation, confirmed bulk-paste flags). Failing tests don't lower it — the AI wrote the code.
- At close, the strong model grades each question separately from code, tests, actions, and token usage, applying a capped correction to the live rating.

**Safety:** the API key never enters the container or the connection string, the expert's home is never mounted, raw pastes are deleted at session end, and ending the session destroys the container.

## Prerequisites

Users do not need Rust or Cargo — see [Install (Users)](#install-users) below.

Developer prerequisites:

- Rust (stable) — required only to build from source; see [Build (Developers)](#build-developers)
- Docker (expert machine, required for the sandbox)
- Mutagen (file sync)
- `MODEL_API_KEY` in `.env` (untracked, never committed) for the Meta Model API

Local/runtime outputs (`.env`, `.pairproof/`, `sessions/`, `artifacts/`, `connection-string*.txt`) are git-ignored.

## Install (Users)

Install the latest stable release with one line — no Rust or Cargo required:

```sh
curl -fsSL https://raw.githubusercontent.com/yavuzhansarmaz/pairproof/main/install.sh | bash
```

Supported platforms:

- Linux x86_64 (musl)
- Linux aarch64 (musl)
- macOS x86_64
- macOS aarch64

Windows is unsupported.

The installer downloads the matching release archive, verifies its SHA-256 checksum before installing, and installs the single `pairproof` binary to `~/.local/bin/pairproof`. It requires no `sudo` and does not modify any shell configuration files. If `~/.local/bin` is not already in your `PATH`, add it yourself, for example in `~/.bashrc` or `~/.zshrc`:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

Then start a new shell (or run the binary by its full path, `~/.local/bin/pairproof`).

Docker and Mutagen are prerequisites for sandbox and file-sync features. The installer warns when they are missing but does not install them — install Docker and Mutagen separately if you need those features.

Run PairProof:

```sh
pairproof
pairproof --role expert
pairproof --role candidate
```

Update to the latest stable release:

```sh
pairproof update
```

Check whether an update is available without downloading the release archive:

```sh
pairproof update --check
```

Uninstall:

```sh
pairproof uninstall
pairproof uninstall --yes
pairproof uninstall --keep-data
pairproof uninstall --yes --keep-data
```

- `pairproof uninstall` asks for confirmation, then removes the binary and `~/.config/pairproof` (including any saved API key and config).
- `pairproof uninstall --yes` skips the confirmation prompt.
- `pairproof uninstall --keep-data` removes only the binary and keeps the saved API key and config in `~/.config/pairproof`.
- Sessions and project files are never removed by uninstall.

Checksum verification: the installer and `pairproof update` verify the SHA-256 checksum of the downloaded archive before installing. If the checksum does not match, the install or update aborts and any existing (old) binary is preserved untouched.

## Build (Developers)

```sh
cargo build
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

Live suites (need network/key, Docker, Mutagen):

```sh
cargo test -- --ignored
```

## Compatibility notes

### Session-id validation (tightened)

The canonical session-id rule is `validate_session_id` in
[src/core/types.rs](src/core/types.rs). Core, sandbox, sync, and artifact
boundaries all delegate to it. An id is valid only when it matches
`[A-Za-z0-9_-]{1,40}` and does not start with `-` or `net-`.

Ids that were previously accepted in some places are now **rejected**
everywhere, including:

- dotted ids such as `sess.1` (`.` is not an allowed character);
- ids longer than 40 characters;
- ids with a leading `-`;
- ids starting with `net-` (reserved to avoid network-name collisions).

**There is no automatic migration.** An existing `sessions/<id>/`
directory whose name no longer conforms (for example a dotted or
over-long name) is orphaned: PairProof will not rename or convert it.
An operator must either rename the directory to a conforming id or
remove it.

### Monitor lifecycle types (moved)

The following public items have been **removed** from
`pairproof::tui::monitor`: `KillSwitch`, `ShutdownGate`,
`CancellationSeam`, `try_cancel_inflight`, and the monitor-defined
`CleanupReport` type. Lifecycle orchestration is the sole owner now:
the single `CleanupReport` definition lives in
`pairproof::orchestration::lifecycle`.

External consumers must import from there instead, for example
`pairproof::orchestration::lifecycle::{CancelToken, CancelOutcome,
CleanupReport, CleanupSequence, CleanupStep, RequestSupervisor,
run_cancellable, CLEANUP_STEP_NAMES}` (also re-exported at
`pairproof::orchestration`). In particular:

- use `RequestSupervisor::kill_switch` plus `CleanupSequence` instead
  of the old `KillSwitch`;
- use `CleanupSequence` (a second run is a no-op) instead of the old
  `ShutdownGate`;
- use `CancelToken` / `run_cancellable` and read
  `CleanupReport::cancel_outcome()` (`CancelOutcome::Cancelled` or
  `CancelOutcome::Unsupported`) instead of the old `CancellationSeam`
  and `try_cancel_inflight`.

`src/tui/monitor.rs` only re-exports the lifecycle cleanup types so the
monitor view shares the one definition; it defines no lifecycle type of
its own. No automatic migration is provided for the removed monitor
items either — update imports to `orchestration::lifecycle` manually.

### Expert snapshot sync label (changed default)

The default `sync_phase` label published in expert snapshots changed
from `Sync: unknown` to `Sync: not started`
(`SessionSyncPhase::NotStarted`). This is the truthful default: the
expert runtime starts no sync itself. `SNAPSHOT_VERSION` stays `1`,
the `sync_phase` wire field and its format are unchanged, and the
labels for the other phases (`Sync: active`, `Sync: paused`,
`Sync: conflict needs attention — ask the expert`, and the
candidate-safe sync-problem message) are unchanged.

### Snapshot builder signature (breaking: `&str` -> `SessionSyncPhase`)

`pairproof::orchestration::control::build_snapshot` (also re-exported
as `pairproof::orchestration::build_snapshot`) changed its
`sync_phase` parameter type: `sync_phase: &str` -> `sync_phase:
SessionSyncPhase`. `SessionSyncPhase` is
`pairproof::orchestration::sync_gate::SessionSyncPhase` (also
re-exported as `pairproof::orchestration::SessionSyncPhase`).
Callers that passed a label string such as `"Sync: active"` must now
pass the enum, for example `SessionSyncPhase::Active`; the published
label is derived with `SessionSyncPhase::label()`.

This is a source-level breaking change only. The wire format is
unchanged: the snapshot JSON `sync_phase` field is still a string
with the same labels, and `SNAPSHOT_VERSION=1` is unchanged
(`SNAPSHOT_VERSION` stays `1`).

### Mutable session access (narrowed: `pub` -> `pub(crate)`)

`Orchestrator::session_mut`
(`pairproof::orchestration::Orchestrator::session_mut`, in
[src/orchestration/session.rs](src/orchestration/session.rs)) and
`CandidateApp::session_mut`
(`pairproof::tui::candidate::CandidateApp::session_mut`, in
[src/tui/candidate.rs](src/tui/candidate.rs)) changed from `pub` to
`pub(crate)`: `pub` -> `pub(crate)`. External crates can no longer
call `Orchestrator::session_mut` or `CandidateApp::session_mut`;
they are no longer part of the public API. Use the immutable
`session()` accessors and the typed `Orchestrator` / `CandidateApp`
operations instead. In-crate pipeline transports unaffected: the
in-crate pipeline, coding, and chat transports that use
`session_mut` internally keep working unchanged.

### Expert runtime sync-phase field (breaking: `String` -> `SessionSyncPhase`)

`ExpertRuntimeDeps.sync_phase`
(`pairproof::tui::expert_runtime::ExpertRuntimeDeps::sync_phase`,
in [src/tui/expert_runtime.rs](src/tui/expert_runtime.rs)) changed
its public field type: `String` -> `SessionSyncPhase`. The
`ExpertRuntime` sync state changed the same way (`String` ->
`SessionSyncPhase`): the stored `sync_phase` state, the
`ExpertRuntime::sync_phase()` return type, and the
`ExpertRuntime::set_sync_phase(...)` parameter are now
`SessionSyncPhase`, not `String`. Constructors that wrote
`sync_phase: "Sync: unknown".to_string()` (or any label string)
must now write e.g. `sync_phase: SessionSyncPhase::NotStarted`.
As with `build_snapshot` above, the published snapshot wire JSON
`sync_phase` string and `SNAPSHOT_VERSION=1` are unchanged.

### Removed TUI controllers (accepted removals)

`pairproof::tui::connection::ConnectionController` and
`pairproof::tui::selection::SelectionController` have been **deleted**.
Behavioral coverage now lives in
`pairproof::orchestration::connection::ConnectionGate` together with
the core `Session` type.

The following named helpers have no direct equivalent in the new API:
`error_is_expired`, `cached_string`/`display_string`, and the
pending-medium accessors / `MediumRequired`. Use the gate's
`current_string` and the `ExpertRuntime` pending-medium staging
instead. No automatic migration is provided for these removals —
update call sites manually.

### Service-failure HealthGate (implemented, not wired)

`HealthGate` / `await_recovery` in
[src/orchestration/health.rs](src/orchestration/health.rs) (exposed on
the expert side only as `ExpertRuntime::health_gate`) is implemented
gate logic, **NOT wired into TTY loop; wiring pending product/design
decision on interception point + countdown propagation + non-blocking
tick**. It has no production caller: `ExpertRuntime` stores only the
gate value and no LeetCode/Meta probes, `Clock`, or `Sleeper`, and the
synchronous TTY tick in `src/tui/mod.rs` drives the runtime through
bounded `block_on` steps, so awaiting the 60-second recovery window
inline would block key input, drawing, and heartbeat handling.

Before wiring, a product/design decision must specify exactly:

- **Interception point:** which failing operation(s) pause the flow,
  how each maps to an `InterruptedStep`, and how resume versus
  `TechnicalFailure` (no candidate evaluation) is applied on both
  expert and candidate screens.
- **Countdown propagation:** how the unreachable service(s) and the
  remaining countdown reach both screens (new snapshot/control field
  or another channel) and when they clear.
- **Non-blocking tick:** how recovery runs without blocking the TTY
  loop (for example a spawned task plus channel, following the
  existing pending-load pattern), including cancellation on end/kill.

Until that decision is made there is **no behavior change**: sessions
do not pause, count down, resume, or terminate via the health gate.

## License

Apache-2.0. See [LICENSE](LICENSE).
