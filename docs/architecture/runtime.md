# Runtime architecture

Internal runtime and lifecycle notes preserved from the pre-public README, updated to current wiring. User-facing run instructions are in [README.md](../../README.md). Source-level compatibility breaks are in [compatibility.md](compatibility.md).

## Startup flow (internals)

The binary entry point is `pairproof::tui::run()` from [src/main.rs](../../src/main.rs). Startup order:

1. Collect original args, then `Args::parse()` (clap exits on `--help`/`--version` during parse) — [src/tui/mod.rs](../../src/tui/mod.rs), [src/tui/args.rs](../../src/tui/args.rs).
2. If a subcommand (`update`, `uninstall`) is present, dispatch it immediately and never enter the TUI or the auto-update gate.
3. On a TTY only, run the startup auto-update gate (`distribution::startup::startup_gate` in [src/distribution/startup.rs](../../src/distribution/startup.rs)): parse `CARGO_PKG_VERSION`, check `current_exe` and the `PAIRPROOF_AUTO_UPDATE_REEXEC` re-exec guard (skipped only when exactly `"1"`), run the updater check, and re-exec with the original args plus the guard set on `Updated`. Non-TTY launches skip this gate.
4. Role dispatch: `Candidate` + TTY runs the candidate interactive loop; `Expert` + TTY runs the expert interactive loop (pre-selecting Expert and running preflight immediately when `--role expert` is passed, otherwise preflight runs on entering the Preflight screen); no role + TTY opens role select. Non-TTY launches print a one-line placeholder/hint and exit 0.

The expert interactive loop creates `ExpertApp` (default screen `RoleSelect`), enables raw mode with alternate screen and bracketed paste, and restores the terminal via a guard on drop. Expert screens proceed `RoleSelect > Preflight > ApiKey > Setup > Proposal > ConnectionWait > SelectMedium > SelectHard > Monitor > Final > Exited` ([src/tui/app.rs](../../src/tui/app.rs)).

## Monitor lifecycle types (moved/removed)

The following public items have been removed from `pairproof::tui::monitor`: `KillSwitch`, `ShutdownGate`, `CancellationSeam`, `try_cancel_inflight`, and the monitor-defined `CleanupReport` type. Lifecycle orchestration is the sole owner now: the single `CleanupReport` definition lives in `pairproof::orchestration::lifecycle` ([src/orchestration/lifecycle.rs](../../src/orchestration/lifecycle.rs)).

External consumers must import from there, for example `pairproof::orchestration::lifecycle::{CancelToken, CancelOutcome, CleanupReport, CleanupSequence, CleanupStep, RequestSupervisor, run_cancellable, CLEANUP_STEP_NAMES}` (also re-exported at `pairproof::orchestration`). In particular:

- use `RequestSupervisor::kill_switch` plus `CleanupSequence` instead of the old `KillSwitch`;
- use `CleanupSequence` (a second run is a no-op) instead of the old `ShutdownGate`;
- use `CancelToken` / `run_cancellable` and read `CleanupReport::cancel_outcome()` (`CancelOutcome::Cancelled` or `CancelOutcome::Unsupported`) instead of the old `CancellationSeam` and `try_cancel_inflight`.

[src/tui/monitor.rs](../../src/tui/monitor.rs) only re-exports the lifecycle cleanup types so the monitor view shares the one definition; it defines no lifecycle type of its own. No automatic migration is provided — update imports manually.

## Removed TUI controllers (accepted removals)

`pairproof::tui::connection::ConnectionController` and `pairproof::tui::selection::SelectionController` have been deleted. Behavioral coverage now lives in `pairproof::orchestration::connection::ConnectionGate` together with the core `Session` type. Migration coverage is exercised by [tests/tui_connection.rs](../../tests/tui_connection.rs) and [tests/tui_selection.rs](../../tests/tui_selection.rs).

The following named helpers have no direct equivalent in the new API: `error_is_expired`, `cached_string`/`display_string`, and the pending-medium accessors / `MediumRequired`. Use the gate's `current_string` and the `ExpertRuntime` pending-medium staging instead. No automatic migration is provided — update call sites manually.

## Service-failure recovery: HealthGate vs RecoveryController (wiring status)

Two distinct mechanisms exist; do not conflate them.

- **`HealthGate`** in [src/orchestration/health.rs](../../src/orchestration/health.rs) (exposed as `ExpertRuntime::health_gate`) now drives service probing inside recovery: `begin_recovery` in [src/tui/expert_runtime_recovery.rs](../../src/tui/expert_runtime_recovery.rs) captures the gate and spawns a non-blocking loop calling `probe_both` with the LeetCode/Meta probes until both are up or the gate's policy window elapses. The TTY tick stays non-blocking: it only polls the outcome via `poll_recovery`. The blocking `await_recovery` form (injected `Clock`/`Sleeper`) remains available but is not used on the TTY path.

- **`RecoveryController`** in [src/orchestration/recovery.rs](../../src/orchestration/recovery.rs), driven from [src/tui/expert_runtime_recovery.rs](../../src/tui/expert_runtime_recovery.rs), **is wired**. The TTY loop polls it (`tick_runtime` in [src/tui/expert_loop.rs](../../src/tui/expert_loop.rs)), renders `recovery_lines()` on the expert screen, can pause the session timer, retries question-load `Fetch`/`Generate` steps on resume, and expires to `TechnicalFailure` when the recovery window lapses. Snapshots carry recovery state via `build_snapshot_with_recovery` / `snapshot_recovery()`. This wired recovery covers specific question-loading failures only; it is not a promise that arbitrary Meta/LeetCode outages pause the session. See the limitation stated in [README.md](../../README.md) Section 11.

## Session lifecycle and cleanup (production path)

Session status proceeds `Setup > WaitingForCandidate > InProgress > WaitingForExpert > Scoring > Completed | EndedByExpert | TechnicalFailure` (terminal), in [src/core/types.rs](../../src/core/types.rs) and [src/core/session.rs](../../src/core/session.rs). The production end path is only the expert pressing `e` on the Monitor screen: cancel in-flight work, terminate the connection gate (ending by expert, not marking completed), grade and compute the final rating, generate the report (or a `TechnicalFailure` stub report), publish the final snapshot, wait for the candidate final-ack (60 s poll, timeout proceeds and is recorded), then run cleanup.

Cleanup writes the session bundle first (`events.jsonl`, `tokens.json`, `context.json`, `report.md`, `solutions/` under `sessions/<id>/`), then runs an ordered sequence — cancel in-flight, terminate connection, stop sync, destroy sandbox (container and network), delete raw pastes — attempting every step, collecting failures, and treating a second run as already finalized. See [src/orchestration/cleanup.rs](../../src/orchestration/cleanup.rs) and [src/orchestration/lifecycle.rs](../../src/orchestration/lifecycle.rs).
