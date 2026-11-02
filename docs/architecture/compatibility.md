# Compatibility notes

Internal engineering notes preserved from the pre-public README. User-facing usage is in [README.md](../../README.md). Wire and source compatibility are distinguished explicitly: a source-breaking change can leave the wire format unchanged.

## Session-id validation (tightened)

The canonical rule is `validate_session_id` in [src/core/types.rs](../../src/core/types.rs). Core, sandbox, sync, and artifact boundaries all delegate to it. An id is valid only when it matches `[A-Za-z0-9_-]{1,40}` and does not start with `-` or `net-`.

Ids previously accepted in some places are now rejected everywhere, including dotted ids such as `sess.1`, ids longer than 40 characters, ids with a leading `-`, and ids starting with `net-` (reserved to avoid network-name collisions).

**There is no automatic migration.** An existing `sessions/<id>/` directory whose name no longer conforms is orphaned: PairProof will not rename or convert it. An operator must rename the directory to a conforming id or remove it.

## Expert snapshot sync label (changed default)

The default `sync_phase` label published in expert snapshots changed from `Sync: unknown` to `Sync: not started` (`SessionSyncPhase::NotStarted`). The expert runtime starts no sync itself, so this is the truthful default. `SNAPSHOT_VERSION` stays `1` in [src/orchestration/control.rs](../../src/orchestration/control.rs) (`SNAPSHOT_VERSION = 1`), the `sync_phase` wire field and format are unchanged, and the labels for the other phases are unchanged.

## Snapshot builder signature (breaking: `&str` -> `SessionSyncPhase`)

`pairproof::orchestration::control::build_snapshot` (also re-exported as `pairproof::orchestration::build_snapshot`) changed its `sync_phase` parameter from `&str` to `SessionSyncPhase` (`pairproof::orchestration::sync_gate::SessionSyncPhase`, also re-exported as `pairproof::orchestration::SessionSyncPhase`). Callers that passed a label string such as `"Sync: active"` must now pass the enum, for example `SessionSyncPhase::Active`; the published label is derived with `SessionSyncPhase::label()`.

Source-level breaking change only. The wire format is unchanged: the snapshot JSON `sync_phase` field is still a string with the same labels, and `SNAPSHOT_VERSION = 1` is unchanged.

## Expert runtime sync-phase field (breaking: `String` -> `SessionSyncPhase`)

`ExpertRuntimeDeps.sync_phase` in [src/tui/expert_runtime.rs](../../src/tui/expert_runtime.rs) changed its public field type from `String` to `SessionSyncPhase`. The `ExpertRuntime` sync state changed the same way: the stored `sync_phase` state, `ExpertRuntime::sync_phase()` return type, and `ExpertRuntime::set_sync_phase(...)` parameter are now `SessionSyncPhase`. Constructors that wrote `sync_phase: "Sync: unknown".to_string()` must now write e.g. `sync_phase: SessionSyncPhase::NotStarted`. The published snapshot wire JSON `sync_phase` string and `SNAPSHOT_VERSION = 1` are unchanged.

## Mutable session access (narrowed: `pub` -> `pub(crate)`)

`Orchestrator::session_mut` in [src/orchestration/session.rs](../../src/orchestration/session.rs) and `CandidateApp::session_mut` in [src/tui/candidate.rs](../../src/tui/candidate.rs) changed from `pub` to `pub(crate)`. External crates can no longer call them; they are no longer part of the public API. Use the immutable `session()` accessors and the typed `Orchestrator` / `CandidateApp` operations instead. In-crate pipeline, coding, and chat transports that use `session_mut` internally keep working unchanged.

See also [runtime.md](runtime.md) for lifecycle ownership and removed items.
