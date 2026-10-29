# PairProof — Implementation Phases

This file tracks the implementation phases for the PairProof MVP defined in [structure.md](structure.md) (the authoritative build specification), with product rationale and P0/P1/P2 priorities in [initial.md](initial.md) and hackathon deadlines plus internal engineering standards in [must.md](must.md). It records what is actually built, phase by phase, with evidence, so progress toward the submission deadline is visible at a glance. On any disagreement between this file and the code, the code wins; on any disagreement about intended behaviour, [structure.md](structure.md) wins.

## Status legend

- **Done** — exit criteria met, verified in code and tests.
- **In progress** — implementation exists but exit criteria are not fully met.
- **Not started** — no implementation beyond stubs or prerequisites credited elsewhere.
- **Blocked** — cannot proceed without an external decision, access, or dependency. Name the blocker in the phase section.

## Summary

Snapshot at time of writing: `cargo test` reports **476 passed, 0 failed, 40 ignored** (85 unit tests in `src/lib.rs`; integration suites: 25 in `tests/connection_string.rs`; 16 in `tests/connection_lifecycle.rs`; 10 in `tests/connection_reissue.rs`; 17 in `tests/llm_approaches.rs`; 10 in `tests/llm_chat.rs`; 15 in `tests/llm_extraction.rs`; 9 in `tests/llm_grading.rs`; 11 in `tests/core_artifacts.rs`; 7 in `tests/core_coverage.rs`; 12 in `tests/core_event_log.rs`; 9 in `tests/core_records.rs`; 15 in `tests/ingest_pipeline.rs`; 17 in `tests/ingest_raw.rs`; 8 in `tests/ingest_text.rs`; 12 in `tests/leetcode_selection.rs`; 6 in `tests/scoring_llm_compose.rs`; 8 in `tests/scoring_final.rs`; 4 in `tests/scoring_live.rs`; 2 in `tests/smoke.rs`; new Phase 7 suites: 10 in `tests/sandbox_argv.rs`, 8 in `tests/sandbox_validate.rs`, 11 in `tests/sandbox_preflight.rs`, 11 in `tests/sandbox_lifecycle.rs` (+1 ignored), 4 in `tests/sandbox_workspace.rs` (+4 ignored), 3 in `tests/sandbox_exec.rs` (+8 ignored), 9 in `tests/sync_argv.rs`, 2 in `tests/sync_ensure.rs` (+6 ignored), 3 in `tests/sync_lifecycle.rs` (+5 ignored), 8 in `tests/sync_paste.rs`; new Phase 8 suites: 10 in `tests/sync_failure.rs`, 7 in `tests/tui_preflight.rs`, 12 in `tests/tui_role.rs`, 11 in `tests/tui_config.rs`, 12 in `tests/tui_setup.rs`, 15 in `tests/tui_connection.rs`, 10 in `tests/tui_selection.rs`, 15 in `tests/tui_monitor.rs`, 17 in `tests/tui_app.rs`). Of the 40 ignored, 16 are the pre-existing API live tests in `tests/llm_live_smoke.rs`, `tests/leetcode_live.rs`, and `tests/meta_api_live.rs` (last run and passed on 2026-10-28, NOT re-run in this phase) and 24 are new Phase 7 live tests, all run and passed live on 2026-10-29 on Docker 29.9.2 + Mutagen 0.18.1 (13 Docker: 1 container lifecycle incl. rotation/shadow-hash check, 8 exec incl. timeout heartbeat kill and SQL-isolation, 4 workspace; 11 Mutagen: 5 lifecycle, 6 sync ensure/resume/conflict-mode); Phase 8 ran no live checks (all deterministic, no Docker/Meta network). `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` are clean. The largest source file is 399 lines (`src/llm/client.rs`, under the 400 cap).

| Phase | Name | Status | Evidence | Depends on |
| --- | --- | --- | --- | --- |
| 0 | Concept and specification | Done | [structure.md](structure.md) "Final"; [initial.md](initial.md); [must.md](must.md); commits `fe9f15c` (skeleton), `570a5ec`, `c2395a9` | — |
| 1 | Validation spikes (LeetCode, Meta API) | Done | [src/leetcode/mod.rs](../src/leetcode/mod.rs), [src/llm/client.rs](../src/llm/client.rs); commit `281cd95`; model choice in commit `645581f` and [must.md](must.md) | 0 |
| 2 | Core domain (session, events, artifacts) | Done | [src/core/](../src/core/) (8 modules); 60 core-related tests green at phase close (now 61) | 0, 1 |
| 3 | Scoring engine | Done | [src/scoring/types.rs](../src/scoring/types.rs), [src/scoring/live.rs](../src/scoring/live.rs), [src/scoring/final_merge.rs](../src/scoring/final_merge.rs); 31 tests green (19 unit + 12 integration) + composition tests in `tests/scoring_llm_compose.rs` | 2 (merge also needs 4) |
| 4 | LLM orchestration and prompts | Done | [src/llm/](../src/llm/) (client, common, approaches, coding, chat, extraction, grading), [src/leetcode/selection.rs](../src/leetcode/selection.rs); 9 unit + 57 integration tests green at phase close (now 13 unit; extraction integration now 15 with SQL/coherence tests, approaches now 17, grading now 9), 7 live smoke at phase close (now 8) run and passed live on 2026-10-28 | 1, 2 |
| 5 | Ingest (JD / additional notes) | Done | [src/ingest/mod.rs](../src/ingest/mod.rs), [src/ingest/pipeline.rs](../src/ingest/pipeline.rs); 38 ingest tests green at phase close (now 40: 8 in `tests/ingest_text.rs`, 17 in `tests/ingest_raw.rs`, 15 in `tests/ingest_pipeline.rs`) plus `tests/llm_extraction.rs` now 15; product decision: paste-only, file-upload dropped, no new dependency | 2, 4 |
| 6 | Connection (rotating-secret string) | Done | [src/connection/](../src/connection/) (`error`, `secret`, `string`, `rotator`); 51 tests green (25 in `tests/connection_string.rs`, 16 in `tests/connection_lifecycle.rs`, 10 in `tests/connection_reissue.rs`) | 2 |
| 7 | Sandbox and sync (Docker, Mutagen) | Done | [src/sandbox/](../src/sandbox/) (`command`, `error`, `exec`, `image`, `lifecycle`, `parse`, `preflight`, `process`, `validate`, `workspace` + `mod.rs`), [src/sync/](../src/sync/) (`command`, `error`, `lifecycle`, `paste` + `mod.rs`), [sandbox/](../sandbox/) (`Dockerfile`, `entrypoint.sh`, `sshd_config`); 97 new tests green (69 integration + 28 unit across 10 new suites) + 24 new live tests run and passed on 2026-10-29 | 2, 6 |
| 8 | TUI — expert | Done | [src/tui/](../src/tui/) (`app`, `shared`, `config`, `setup`, `connection`, `selection`, `monitor`, `expert`, `candidate`) + [src/sync/failure.rs](../src/sync/failure.rs); 109 Phase 8 tests green (10 `sync_failure`, 7 `tui_preflight`, 12 `tui_role`, 11 `tui_config`, 12 `tui_setup`, 15 `tui_connection`, 10 `tui_selection`, 15 `tui_monitor`, 17 `tui_app`) | 2–7 |
| 9 | TUI — candidate | Not started | [src/tui/candidate.rs](../src/tui/candidate.rs) stub | 2, 4, 6, 7 |
| 10 | End-to-end integration and failure handling | Not started | No cross-module flow wired; only per-module tests exist | 3–9 |
| 11 | Hardening and polish (P1) | Not started | fmt/clippy clean today, but no P1 items built | 10 |
| 12 | Demo and submission | Not started | No demo or submission artifacts in repo | 10, 11 |

**Phases 0–8 are Done.** The expert-side controllers plus the interactive Ratatui loop and their 109 maintained tests are landed and green (`cargo test` 476/0/40). The Phase 10 production seams (SSH accept → core glue, API cancellation wire-up, Markdown report generation) stay tracked in Phase 10 and were not faked in Phase 8. Phases 9–12 are Not started. See [Current state](#current-state).

Every phase inherits two standing rules from [must.md](must.md) (Project Engineering Standards): each endpoint — user-facing flow, external integration wrapper, and lifecycle operation — ships with maintained tests covering success, error, and edge cases, and an endpoint is not done until its tests pass; and no file may exceed 400 lines, with `rustfmt`/`clippy` clean before merging. The per-phase exit criteria name the concrete tests each phase owes.

## Phase 0 — Concept and specification

**Goal.** Settle the product shape and record an authoritative build spec before code.

**Scope.**

- Product rationale, assessment model, demo narrative, P0/P1/P2 priorities: [initial.md](initial.md).
- Authoritative implementation spec (sections 1–21): [structure.md](structure.md). Hackathon rules/deadlines and internal standards: [must.md](must.md).
- Single-crate skeleton with the section 19 module layout ([Cargo.toml](../Cargo.toml), `src/` tree), license, [README.md](../README.md).

**Exit criteria.**

- [x] structure.md marked Final, consistent with initial.md/must.md.
- [x] Skeleton builds; `tests/smoke.rs` covers binary startup and `--help` (2 tests).

**Status: Done.** Evidence: [structure.md](structure.md) header says "Status: Final"; skeleton in commit `fe9f15c`; license commits `c2395a9`, `570a5ec`. Note: the P0/P1/P2 checkboxes in [initial.md](initial.md) section 11 are all still unchecked, including items already delivered (spikes, skeleton) — the code, not those checkboxes, is the source of truth. [README.md](../README.md) likewise still says "Project skeleton only", stale since Phase 1; fixing it belongs to Phase 12 at the latest.

**Remaining work.**

- [ ] None. Stale README text is tracked in Phase 12.

## Phase 1 — Validation spikes (LeetCode GraphQL, Meta Model API)

**Goal.** Prove the two external dependencies work from Rust before building on them (structure.md sections 4, 20; initial.md P0).

**Scope.**

- LeetCode GraphQL client: topic/difficulty catalog, statement + snippet fetch, premium-locked detection, free filtering ([src/leetcode/mod.rs](../src/leetcode/mod.rs): `LeetCodeClient::list_questions`, `fetch_question`).
- Meta Model API client: model listing, chat, streaming chat with token usage ([src/llm/client.rs](../src/llm/client.rs): `MetaClient::list_models`, `chat`, `chat_stream`, re-exported by [src/llm/mod.rs](../src/llm/mod.rs)); key from `MODEL_API_KEY`.
- Settled model IDs: strong `muse-wonder-1.0`, light `muse-spark-1.3` (`STRONG_MODEL`/`LIGHT_MODEL`).

**Exit criteria.**

- [x] Live tests for: free catalog, statement+snippets, premium rejection ([tests/leetcode_live.rs](../tests/leetcode_live.rs)); model visibility, strong chat usage, light streaming usage ([tests/meta_api_live.rs](../tests/meta_api_live.rs)). `#[ignore]`d (network/key); run with `-- --ignored`.
- [x] Offline parsing/error unit tests: 3 in `src/leetcode/mod.rs`, 8 in `src/llm/client.rs` (including `debug_redacts_api_key`), green in the current `cargo test` run.
- [x] Model IDs confirmed via `GET /v1/models`, recorded with live token measurements in [must.md](must.md) (2026-10-28) and commit `645581f`.

**Status: Done.** Evidence: commit `281cd95` "Add LeetCode and Meta Model API clients (validation spikes)"; [must.md](must.md) records live validation on 2026-10-28. Two caveats: (1) the live `#[ignore]`d tests were not re-run when this section was first written (network + key required), so "Done" rested on the committed clients, green unit tests, and that live-validation record — they have since been re-run and passed on 2026-10-28; (2) rate-limit behaviour (section 20 item 1) has no written findings and no retry/429 handling in code — it is carried as a residual risk into Phase 10. Randomized topic-pool selection (section 4) is **not** shipped here: the client lists/fetches, nothing selects yet.

**Remaining work.**

- [ ] Randomized selection flow over the topic pool (Phase 4/8).
- [ ] Characterize rate limiting; feed findings into Phase 10.

## Phase 2 — Core domain (session state machine, event log, artifacts)

**Goal.** UI-independent interview state: session lifecycle, per-question actions, append-only JSONL event log, on-disk artifact layout (structure.md sections 3, 11, 15, 19; `core/`).

**Scope.**

- Types/errors: `SessionStatus`, `QuestionSlot`, `ApproachChoice` (4 presets + `Other`), `Language` (Python/JavaScript/SQL), typed `CoreError` ([src/core/types.rs](../src/core/types.rs)).
- State machine: exactly 5 easy at create; waiting/connect/start; expert 3+2 set once (validated); 1 extra (max 11); `WaitingForExpert` after the fifth question; expert-end / technical-failure / completed terminals ([src/core/session.rs](../src/core/session.rs)).
- Candidate actions: select approach + lock expected tokens, continue, redirect, request tests, change approach, add tokens, re-question request/resolve (expert part only, before approach, same-difficulty replacement), finish, skip ([src/core/actions.rs](../src/core/actions.rs)).
- Typed sequenced, timestamped events ([src/core/events.rs](../src/core/events.rs)); JSONL writer (create-new, sequence-checked, flush per event) + validating reader ([src/core/event_log.rs](../src/core/event_log.rs)).
- Artifacts: session layout (`events.jsonl`, `report.md`, `tokens.json`, `context.json`, `solutions/`, `raw/`), token/context records, atomic writes, raw-dir deletion ([src/core/artifacts.rs](../src/core/artifacts.rs)).

**Exit criteria.**

- [x] Tests per must.md (success/error/edge) for every transition and action, event-log round-trip/gaps/malformed lines, artifact writes: 22 unit tests under `src/core/` (actions 9, session 6, types 5, events 2) plus [tests/core_artifacts.rs](../tests/core_artifacts.rs) (10 at phase close; now 11 with `solution_path_uses_sql_extension`), [tests/core_coverage.rs](../tests/core_coverage.rs) (7), [tests/core_event_log.rs](../tests/core_event_log.rs) (12), [tests/core_records.rs](../tests/core_records.rs) (9) — all green.
- [x] Core work committed: initial core in `f45a6c4`, completion work in the commit carrying this update.
- [x] Event schema covers everything later phases must log (test runs, paste flags, disconnect/reconnect, chat boundary violations).

**Status: Done.** Implemented and fully green. `EventKind` now covers test-run results, paste flags (sections 13, 15, 16), disconnect/resume (section 17), and chat boundary violations (section 10); `Session` has a disconnected state with a reconnect path that resumes at the same question, and [src/core/records.rs](../src/core/records.rs) records test results, paste flags, and chat boundary violations. Token-lock decision (settled): a token lock belongs to an approach selection; `change_approach` clears the current selection so a re-selection locks anew; `slot.locked_expected_tokens` is the current selection's lock; the complete lock history is preserved in `TokensLocked` events and exposed via `Session::token_locks()`. `write_report` stores Markdown verbatim — report *generation* is Phase 10.

**Remaining work.**

- [ ] None.

## Phase 3 — Scoring engine

**Goal.** Deterministic live rating and final merge (structure.md sections 13–14; initial.md section 6; `scoring/`).

**Scope.**

- Live rating from 6.0, clamped 3.0–10.0, with a live per-component breakdown.
- Step table: difficulty multipliers (easy ×0.5, medium ×1.0, hard ×1.5) times an acceptance factor from each question's LeetCode acceptance percent (>=50% ×1.0, 35–49% ×1.1, <35% ×1.25, unknown ×1.0); approach steps (+0.30/+0.10/−0.20, `Other` graded at selection); token-ratio bands vs the locked expectation (≤0.8 → +0.20, 0.8–1.2 → 0, 1.2–1.8 → −0.15, >1.8 → −0.30); intervention (+0.20/−0.10); paste flag −0.30; skip 0 with its tokens settled in the final token line; chat and test results score-neutral.
- Hidden best-approach marker recorded at option generation (data from Phase 4; storage settled here — it is not in `core` today).
- Final merge: per-question AI grades (Phase 4) → total correction capped at ±0.8, scaled down proportionally if over; skipped questions excluded from AI scoring; "not scored" = 0, never invented.
- Final breakdown data for both screens: solved count, test/time summaries, skip/paste counts, per-dimension lines, evidence sentences (section 13).

**Exit criteria.**

- [x] Table-driven tests for every step, band boundary (0.8/1.2/1.8), multiplier, clamp, cap scaling, skip settlement, and neutral chat/tests, per must.md.
- [x] Golden scoring test over a scripted `Session` event history.

**Status: Done.** Implemented in [src/scoring/types.rs](../src/scoring/types.rs) (shared Step-0 contract types: `ApproachQuality`/`ApproachSet` with hidden `best_index` and no `Serialize`, `ApproachGrade`, `QuestionGrade`, `GradeOutcome`, `ScoringError`), [src/scoring/live.rs](../src/scoring/live.rs) (`live_rating` step table), and [src/scoring/final_merge.rs](../src/scoring/final_merge.rs) (`final_rating`, ±0.8 cap with proportional scaling, `FinalBreakdown`/`PerQuestionFinal`). Tests: 19 unit (types 4, live 9, final_merge 6) plus 12 integration ([tests/scoring_live.rs](../tests/scoring_live.rs) 4, [tests/scoring_final.rs](../tests/scoring_final.rs) 8) = 31, all green, including the acceptance tests `acceptance_factor_bands_at_boundaries`, `acceptance_scales_approach_and_token_lines`, `skipped_token_settlement_uses_acceptance_factor`, and `to_question_ref_maps_and_clamps_acceptance` (in [tests/leetcode_selection.rs](../tests/leetcode_selection.rs)); the `None` acceptance default (×1.0) keeps prior golden scores unchanged. Settled decisions: the token ratio uses the **last** lock from `Session::token_locks`; skipped-token settlement maps the positive band to 0 (a skip is never rewarded); `solved_count` = finished slots graded >= 5.0; `final_rating` revalidates grades (struct-literal safe). Evidence: uncommitted working tree on branch `dev` — no commit hash exists for this work yet.

**Remaining work.**

- [ ] None.

## Phase 4 — LLM orchestration and prompts

**Goal.** Everything above the raw client that makes the two models do interview work (structure.md sections 10, 11, 14; `llm/`).

**Scope.**

- Strong model: 4+1 approach generation (shuffled, best approach marked internally, expected-token lock on selection); coding loop that writes solution code and checks runnability only, terminating only on the defined events (AI declares ready; candidate finish/skip/change; expert end) — no token cap. Tests created only on `Request tests / verify`.
- Light-model chat per section 10: fixed no-solution system prompt; PairProof live-state block (question, approach/stage, size-bounded solution file, latest test result, recent actions; labelled as data; never marker, token expectation, or score); short history; verbatim candidate message. Boundary violations shown but logged, score-neutral. Low `reasoning_effort` per [must.md](must.md).
- Extraction calls for Phase 5 (topics, level, language proposal: JD and additional notes taken together, Python only when neither names one, section 5) and per-question scoring calls (one separate request per non-skipped question; 0–10 grade, rationale, correction share; retry that question only, section 14).
- Cancellation plumbing so the expert kill switch (section 17) stops all in-flight calls; API key stays outside the container (section 9).
- Randomized selection from the LeetCode topic pool on the Phase 1 client (section 4): auto-pick 5 easy; lists for expert 3+2 selection.

**Exit criteria.**

- [x] Tests per must.md for each Meta API wrapper (approaches, coding step, chat, extraction, scoring): request assembly and response parsing vs fixtures/mocks, malformed-response and API-failure paths; live smoke tests `#[ignore]`d like Phase 1.
- [x] Tests that hidden data never enters the chat live-state block and that chat sends low `reasoning_effort`.

**Status: Done.** `src/llm` is split into [src/llm/client.rs](../src/llm/client.rs) (`ReasoningEffort`/`ChatOptions::low`, `chat*_with_options`, old signatures preserved; manual `Debug` for `MetaClient` redacts the API key as `[REDACTED]`), [src/llm/common.rs](../src/llm/common.rs) (`LlmError`/`LlmOutcome`), [src/llm/approaches.rs](../src/llm/approaches.rs) (4+1 generation with seeded-shuffle parsing, `Other` grading), [src/llm/coding.rs](../src/llm/coding.rs) (single coding-step prompt/interface, `apply_usage` via `Session::add_tokens`, tokens booked before parsing; **no** sandbox execution), [src/llm/chat.rs](../src/llm/chat.rs) (section 10 assembly, hidden-data-free live-state block, boundary-violation logging via `ChatBoundaryViolated`), [src/llm/extraction.rs](../src/llm/extraction.rs) (section 5 combined JD + additional-notes language priority, `KNOWN_TOPICS` allowlist with unknown slugs dropped in `parse_extraction`, plus normalization/unsupported surfacing), and [src/llm/grading.rs](../src/llm/grading.rs) (section 14 one request per question with per-question retry: `GRADE_MAX_ATTEMPTS` = 3 via `retry_grade_attempts` (200 ms/400 ms backoff, last error wins); `grade_questions` still turns exhausted failures into `NotScored` per question); randomized topic-pool selection is in [src/leetcode/selection.rs](../src/leetcode/selection.rs) (`pick_random`/`expert_pool`/`select_questions`/`auto_pick_easy`). Tests: 13 unit (client 8 incl. 3 moved from Phase 1 plus `debug_redacts_api_key`, common 2, grading 3 `retry_*`) plus integration [tests/llm_approaches.rs](../tests/llm_approaches.rs) (16; now 17 with `sql_language_names_mysql_in_prompts`), [tests/llm_chat.rs](../tests/llm_chat.rs) (10), [tests/llm_extraction.rs](../tests/llm_extraction.rs) (7 at phase close; now 15 with topic-filter and SQL/coherence tests), [tests/llm_grading.rs](../tests/llm_grading.rs) (9 with `grade_questions_retries_per_question_in_isolation`), [tests/leetcode_selection.rs](../tests/leetcode_selection.rs) (11 at phase close; now 12 with the acceptance-mapping test) — all green; live smoke tests in [tests/llm_live_smoke.rs](../tests/llm_live_smoke.rs) (7 at phase close; now 8 with the live SQL-role extraction test) are `#[ignore]`d by default and were run and passed live on 2026-10-28. A live ML-engineer JD extraction check on 2026-10-28 produced exactly `matrix`, `math`, `dynamic-programming`, `graph`, `heap-priority-queue`. Evidence: uncommitted working tree on branch `dev` — no commit hash exists for this work yet.

**Remaining work.**

- [ ] Coding-loop sandbox execution + loop orchestration (Phase 7/10) — the single-step interface is delivered; execution was **not** faked here.
- [ ] Expert kill-switch cancellation plumbing over in-flight calls (Phase 8/10).

## Phase 5 — Ingest (JD / additional notes)

**Goal.** Turn expert pastes into the non-personal context summary (structure.md sections 5, 9; `ingest/`).

**Scope.**

- Paste-only input by product decision: the expert pastes a job description (required) plus optional additional notes as a second source. No file upload anywhere; no new dependency was added and [Cargo.toml](../Cargo.toml) is unchanged. Disclosure that bounded text goes to the Meta API is surfaced by Phase 8.
- Each field is bounded to 50,000 chars (`BoundedText` in [src/ingest/mod.rs](../src/ingest/mod.rs)), truncated on a char boundary with a visible `truncated` flag. A blank job description is rejected; blank additional notes are allowed.
- Result: topics, level, proposed language via the strong-model extraction call in [src/llm/extraction.rs](../src/llm/extraction.rs) (prompt now says additional notes), wired through `propose_context`/`propose_from_extraction` → `ProposedContext` in [src/ingest/pipeline.rs](../src/ingest/pipeline.rs). Language priority (section 5): JD and additional notes taken together, Python only when neither names one. An explicit unsupported language surfaces as a typed warning (`LanguageProposal::Unsupported` / `IngestError::UnsupportedLanguage`), never a silent fallback.
- Storage: raw pastes stored as fixed names `raw/jd.txt` + `raw/additional.txt`, deleted at session end via `ArtifactLayout::delete_raw_dir`; leftover scan/cleanup on next launch via `find_leftover_raw_dirs` / `delete_leftover_raw_dir` (symlink-safe); documents never reach candidate or container. Once a `Session` exists, `write_context_summary` writes the surviving `ContextSummary` via `ArtifactLayout::write_context`.

**Exit criteria.**

- [x] Tests per must.md for the paste path incl. blank-JD rejection and blank additional notes allowed ([tests/ingest_text.rs](../tests/ingest_text.rs) 8; [tests/ingest_pipeline.rs](../tests/ingest_pipeline.rs) 13 at phase close, now 15, incl. `propose_context_rejects_blank_jd_without_api_call`).
- [x] 50,000-char bound per field with char-boundary truncation and visible flag (`bound_*` tests in [tests/ingest_text.rs](../tests/ingest_text.rs), truncation-flag propagation in [tests/ingest_pipeline.rs](../tests/ingest_pipeline.rs)).
- [x] Fixed filenames `jd.txt`/`additional.txt` in `raw/` and end-of-session deletion via `delete_raw_dir` ([tests/ingest_raw.rs](../tests/ingest_raw.rs) 17, incl. `stored_raw_documents_removed_by_delete_raw_dir`; `raw_documents_gone_after_cleanup_but_summary_stays` in [tests/ingest_pipeline.rs](../tests/ingest_pipeline.rs)).
- [x] Leftover scan plus confirmed cleanup incl. symlink probes (symlinked raw dir/file/session rejected or link-only removal, outside-root rejection) in [tests/ingest_raw.rs](../tests/ingest_raw.rs).
- [x] Extraction wiring, language priority (combined JD + additional notes, Python only when neither names one) and unsupported-language typed warning in [tests/ingest_pipeline.rs](../tests/ingest_pipeline.rs), with extraction parsing covered by [tests/llm_extraction.rs](../tests/llm_extraction.rs) (now 15, including the topic-filter tests `parse_extraction_filters_unknown_topics`, `parse_extraction_all_unknown_topics_yield_empty`, and `known_topics_are_normalized_slugs`).
- [x] `ContextSummary` write once a `Session` exists, refusing an unsupported language (`write_context_summary_*` in [tests/ingest_pipeline.rs](../tests/ingest_pipeline.rs)).

**Status: Done.** Implemented in [src/ingest/mod.rs](../src/ingest/mod.rs) (bounds, fixed filenames, raw storage, symlink-safe leftover scan/cleanup) and [src/ingest/pipeline.rs](../src/ingest/pipeline.rs) (`propose_context`/`propose_from_extraction` → `ProposedContext`, `write_context_summary` via `ArtifactLayout::write_context`); the extraction call stays on the strong model in [src/llm/extraction.rs](../src/llm/extraction.rs). Tests: 38 new ingest tests green at phase close (now 40: [tests/ingest_text.rs](../tests/ingest_text.rs) 8, [tests/ingest_raw.rs](../tests/ingest_raw.rs) 17, [tests/ingest_pipeline.rs](../tests/ingest_pipeline.rs) 15 with `propose_maps_sql_language` and `write_context_summary_with_sql_session`) plus one added test in [tests/llm_extraction.rs](../tests/llm_extraction.rs) (8 at phase close; now 15 with the later topic-filter and SQL/coherence tests). Manager probe findings: a symlinked-file write escape was caught and fixed (store rejects symlinked `jd.txt`/`additional.txt`/raw dir), and blank-topics/blank-JD guards were added (`propose_rejects_empty_topics_and_blank_level`, `propose_rejects_all_blank_topics`, blank-JD rejection without an API call). Product decision recorded: paste-only ingest, file-upload dropped, no new dependency. Evidence: uncommitted working tree on branch `dev` — no commit hash exists for this work yet.

**Remaining work.**

- [ ] None. Phase 8 displays the Meta API disclosure and the unsupported-language warning UI.

## Phase 6 — Connection (rotating-secret string)

**Goal.** The disposable, single-use candidate→expert link (structure.md section 6; `connection/`).

**Scope.**

- String encodes exactly: endpoint address+port, session-scoped ephemeral username, strong one-time secret, expiry, machine fingerprint — never API key, real username/home path, personal data, or permanent credentials.
- Secret rotates every 60 s with countdown; on connect it dies, rotation stops, string locks (no second connection); on expiry the old string is invalid and the candidate sees "expired, ask the expert for the current string."
- Accept/lock handling; re-issue after accidental disconnect (rotation restarts; finality only on deliberate expert end, section 17).
- Direct mode only, without baking Direct-only assumptions into shared session logic (VPS relay is P2, below).

**Exit criteria.**

- [x] Tests per must.md for the lifecycle: encode/decode round-trip, field allowlist, rotation/expiry, connect kills secret, second-connect and expired-string rejection, re-issue ([tests/connection_string.rs](../tests/connection_string.rs) 25, [tests/connection_lifecycle.rs](../tests/connection_lifecycle.rs) 16, [tests/connection_reissue.rs](../tests/connection_reissue.rs) 10 = 51, all green).

**Status: Done.** Implemented in [src/connection/error.rs](../src/connection/error.rs) (`ConnectionError`), [src/connection/secret.rs](../src/connection/secret.rs) (`Secret`), [src/connection/string.rs](../src/connection/string.rs) (`Endpoint`/`EphemeralUsername`/`Fingerprint`/`ConnectionString`), and [src/connection/rotator.rs](../src/connection/rotator.rs) (`SecretRotator`/`ConnectionState`/`ROTATION_WINDOW_MS`), declared and re-exported by [src/connection/mod.rs](../src/connection/mod.rs); no new dependencies. Canonical single-line format: `pairproof:v1:<host>:<port>:<username>:<secret_hex>:<expires_at_ms>:<fingerprint_hex>` (exactly 8 colon-separated fields). The section 6 allowlist holds by construction — the component newtypes make forbidden fields (API key, real username/home path, personal data, permanent credentials) unrepresentable — and strict parsing rejects extra/missing fields, a wrong version, any whitespace or control characters, and non-canonical numbers. `Secret` is 16 bytes (128-bit) from `rand`'s `OsRng`, encoded as 32 lowercase hex chars, compared in constant time, with `Debug` redacted (a test proves `Debug` never contains the hex). Host grammar: `Endpoint::new` is syntactic (IPv4 or a lowercased DNS hostname; `0.0.0.0` rejected, IPv6 rejected since `:` is the field separator) and `Endpoint::public` is the issuance-grade constructor that rejects non-global IPv4. `Fingerprint` is caller-supplied opaque lowercase hex, even length 32–128 (Phase 7 supplies the hex-encoded sandbox SSH host-key hash). Lifecycle runs in explicit milliseconds (`now_ms`, the core convention): `ROTATION_WINDOW_MS` = 60,000; states are `Rotating` (countdown in seconds, ceiling, capped at 60), `Locked` (the first successful `verify` kills the secret), and `Ended` (`terminate`, final). `verify` rejects, in precedence order, with typed errors: `SessionEnded`, `AlreadyLocked`, `FingerprintMismatch`, `EndpointMismatch`, `UsernameMismatch`, `Expired` (judged only against the rotator's own window — the string's expiry field is never trusted; validity is half-open, [issue, expiry)), `WrongSecret`. `reissue()` from `Rotating`/`Locked` mints a fresh secret and a full window with endpoint/username/fingerprint unchanged and all earlier secrets dead; it is impossible after `Ended`. There is no networking or SSH in this phase (the Phase 7 sandbox owns SSH) and no production connection→core glue — the seam is proven only by the integration test `core_seam_reconnect_story` in [tests/connection_reissue.rs](../tests/connection_reissue.rs), which drives a real core `Session`; wiring belongs to Phases 8/10. Manager attack-probe rounds (temporary, removed after passing) covered unicode/invisible characters, swapped fields, `u64`/`u16` overflow inputs, tampered expiry/fingerprint/endpoint/username, verify-does-not-rotate, double reissue, disconnect-without-reissue, and `u64::MAX` time saturation; an endpoint audit found the `Malformed`/`InvalidExpiry` variants unasserted, closed with `parse_malformed_variant_for_shape_errors` and `parse_invalid_expiry_variant_for_bad_expiry`. Evidence: uncommitted working tree on branch `dev` — no commit hash exists for this work yet.

**Remaining work.**

- [ ] None. SSH provisioning/accept wiring belongs to Phase 7, the connection screens to Phases 8/9, and production connection→core glue to Phases 8/10.

## Phase 7 — Sandbox and sync (Docker, Mutagen)

**Goal.** The disposable execution environment and file sync (structure.md sections 6–9; `sandbox/`, `sync/`).

**Scope.**

- Docker as external processes: build the shipped image once (cached), one container per session (SSH, Python+Node plus a MySQL runner for SQL sessions, empty workspace), destroy on expert end (ephemeral user/string die with it). SSH only in-container; no host mounts (never home), no forwarded ports; API key never in the container.
- Preflight computations owned here, displayed by Phase 8: Docker ready, image ready, port listening, external IP visible, CGNAT suspicion; refuse to start without Docker; other failures show guidance, expert may proceed at own risk.
- In-container execution: runnability checks for the coding loop; test runs on candidate request (Python/JavaScript/SQL). SQL execution is not built yet: this phase must add a MySQL runner to `sandbox/Dockerfile` and wire runnability/test execution for `.sql` solutions.
- Mutagen: availability check, both-ways session sync, removal at end; workspace layout `questions/01/question.md` + `solution.py|.js|.sql` (section 8); candidate keeps their local copy.
- Bulk-insert detection on synced changes feeding paste flags (sections 13, 16); artifacts copied out before destroy (Phase 2 layout).

**Exit criteria.**

- [x] Tests per must.md for each lifecycle operation (image build/cache, container create/destroy, Mutagen start/stop, preflight computation): success, missing-Docker/Mutagen, and failure paths — command construction/error mapping at minimum, live Docker runs where feasible.

**Status: Done.** Implemented in [src/sandbox/](../src/sandbox/) (`command`, `error`, `exec`, `image`, `lifecycle`, `parse`, `preflight`, `process`, `validate`, `workspace`, declared by `mod.rs`) and [src/sync/](../src/sync/) (`command`, `error`, `lifecycle`, `paste`, declared by `mod.rs`), with the container definition in [sandbox/](../sandbox/) (`Dockerfile`, `entrypoint.sh`, `sshd_config`); [src/sandbox/lifecycle.rs](../src/sandbox/lifecycle.rs) is 373 lines (under the 400 cap). Tests: 97 new tests green — 69 integration (10 in `tests/sandbox_argv.rs`, 8 in `tests/sandbox_validate.rs`, 11 in `tests/sandbox_preflight.rs`, 11 in `tests/sandbox_lifecycle.rs`, 4 in `tests/sandbox_workspace.rs`, 3 in `tests/sandbox_exec.rs`, 9 in `tests/sync_argv.rs`, 2 in `tests/sync_ensure.rs`, 3 in `tests/sync_lifecycle.rs`, 8 in `tests/sync_paste.rs`) plus 28 new unit tests under `src/sandbox` + `src/sync` (lib total now 81) — and 24 new `#[ignore]`d live tests, all run and passed live on 2026-10-29 on Docker 29.9.2 + Mutagen 0.18.1 (13 Docker: 1 container lifecycle incl. rotation/shadow-hash check, 8 exec, 4 workspace; 11 Mutagen: 5 lifecycle, 6 sync ensure/resume/conflict-mode); `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` are clean. Settled design: one disposable container per session with exactly one published port (->22); per-session bridge network plus in-container iptables default-DROP; limits 512m/1cpu/256pids with no mounts and no privileged flag; the Phase 6 secret reaches the container only via docker-exec stdin (`chpasswd`); the image is built from the embedded build context and tagged `pairproof-sandbox:<version>-<content-hash>`; the paste heuristic flags candidates only at inserted_bytes>=2048 OR inserted_lines>=50, with confirmation UI deferred to Phase 8+; there is NO passed/failed count parser (Phase 4 defined no test-output format — the seam is flagged for Phase 4/10); SSH-beta `mutagen create` is covered at argv level only (the interactive password cannot go live non-interactively — full SSH sync awaits Phase 9/10 wiring); timeout kill is real: `run_in_container` wraps workload argv with in-container GNU `timeout -s KILL` (`timeout_argv` in [src/sandbox/exec.rs](../src/sandbox/exec.rs)), the outer runner gets a 5s grace, and an inner-timeout exit maps to `SandboxError::Timeout` only when elapsed reaches the requested timeout; SQL runnability is isolated: provisioning creates a dedicated `ppcheck` OS+DB user (unix_socket auth, granted ONLY `pp_runnable.*`) and `check_runnability` for Sql executes the file as `ppcheck` in `pp_runnable` via `runuser`, so qualified writes to the session DB `pairproof` are denied (`runnable=false`) and cannot mutate it, while `run_tests` Sql still runs as the session user against `pairproof` (the session user no longer holds a `pp_runnable` grant); the host-key fingerprint cats the `.pub` line and `parse_host_key_fingerprint` (replacing `parse_sha256sum_fingerprint`) derives hex SHA-256 of the base64-decoded key blob — independently reproducible from an `ssh-keyscan` line, with unit tests covering a known `ssh-keygen` vector plus hand-rolled SHA-256/base64 vectors in [src/sandbox/parse.rs](../src/sandbox/parse.rs) and the `tests/sandbox_argv.rs` test renamed accordingly. Manager review: every slice went through review → independent probes → fix rounds → re-verification; probes caught a read_file infra-vs-notfound misclassification plus silent truncation and scratch-setup misattribution (all fixed and re-verified); manager live probes verified the timeout heartbeat stops (incl. a spawned grandchild), a `DROP TABLE pairproof` attempt during runnability leaves the table intact, and `read_fingerprint` matches an independent python `hashlib` computation of the same key. Evidence: uncommitted working tree on branch `dev` — no commit hash exists for this work yet.

**7E addendum.** Slice 7E lands the sync status/ensure runners on top of the Done scope above: session status is queried via `mutagen sync list --template <STATUS_TEMPLATE>` (`sync_list_template` in [src/sync/command.rs](../src/sync/command.rs), rows parsed by `parse_sync_template` into name/status/paused/conflicts with malformed lines skipped and conflicts summing reported plus excluded counts), with a human-list fallback (`sync_list` + `parse_sync_list`/`find_sync`, which yield paused=false/conflicts=0 since the human output carries no such detail); raw template states render through the `describe_status` table (paused renders as `[Paused]`, unknown states pass through). The runners are `resume_sync` (a failed resume that still leaves the session unpaused returns Ok, verified against a template-status re-query) and `ensure_sync` returning `Created`/`Resumed`/`AlreadyRunning` (create when missing, resume when paused, report when running) in [src/sync/lifecycle.rs](../src/sync/lifecycle.rs). The conflict-mode live experiment (`live_conflict_modes_compared` in [tests/sync_ensure.rs](../tests/sync_ensure.rs)) showed `two-way-resolved` silently discards one conflicting version with conflicts=0 while `two-way-safe` keeps both versions and reports conflicts>=1, so the manager decision is to ship `two-way-safe`: `sync_create` builds the two-way-safe session and this slice aligns the `src/sync/lifecycle.rs` doc line with it, while SSH-beta `mutagen create` stays argv-only. Tests: `tests/sync_ensure.rs` (2 passed + 6 ignored live) is new and `tests/sync_argv.rs` grows 8 to 9. Item D (the SSH-failure classifier) is deferred to Phase 8 scope, which already lists it under Sync/SSH failure guidance — referenced here, not duplicated.

**Remaining work.**

- [ ] None. Full SSH sync wiring belongs to Phases 9/10, paste-confirmation UI to Phase 8+, and the test-output-format seam to Phase 4/10.

## Phase 8 — TUI, expert side

**Goal.** Expert screens: preflight, setup, live selection and monitoring (structure.md sections 3, 7, 9, 12; `tui/` args/startup/role dispatch, `tui/expert`).

**Scope.**

- Role selection (`Expert`) in [src/tui/](../src/tui/); preflight checklist with concrete failure guidance.
- Sync/SSH failure guidance (deferred 7E item D): display the sync layer's SSH-failure classification as an actionable hint (e.g. "cannot reach the remote; check that `ssh <target>` works non-interactively") on the EXPERT preflight/connection screens only; the classifier itself lives in `sync/`, Phase 8 only displays it. Never on candidate screens: candidate-side sync trouble shows only a generic "sync problem, ask the expert" message with no SSH target, path, session, or daemon detail.
- API key: user-private `0600` config, masked, deletable, env-var override, never logged (section 9).
- Setup: JD/additional-notes entry (Phase 5), topics/level/language proposal with confirm/override, unsupported-language warning, Meta API disclosure; system auto-selects the 5 easy (no expert preview).
- Connection screen with 60 s countdown (Phase 6); on connect, live selection: 3 medium then 2 hard on separate screens while the candidate works; approve/reject re-question and pick the same-difficulty replacement.
- Live monitoring: current question, chosen approach, test results, elapsed time, live rating breakdown (Phase 3), paste-flag alerts; no live chat content.
- Actions: add 1 extra question; global kill switch (cancel API calls, close connection, destroy container, section 17); fresh string after accidental disconnect; final breakdown + artifacts.

**Exit criteria.**

- [x] Tests per must.md for each screen transition/expert flow (preflight outcomes, setup, selection submit, re-question resolve, extra, kill switch) at the state/logic level beneath rendering, incl. error paths.

**Status: Done.** Landed in [src/tui/](../src/tui/): pure navigation state machine in [src/tui/app.rs](../src/tui/app.rs) (`ExpertApp`/`AppScreen`/`AppEvent`/`AppError`, gated transitions — preflight `Fail` blocks, disclosure before proposal, `ConnectionLocked` only via `set_connected`, disconnect stays on Monitor, `Back`/`Retry`/`Quit` typed, never panics) wired by a thin Ratatui/crossterm loop in [src/tui/mod.rs](../src/tui/mod.rs) (`run()` branches on `stdout().is_terminal()`: TTY drives role → preflight → API-key → setup → proposal → connection → 3-medium/2-hard → monitor → final with `q`/`b`/`r`/Enter, `TtyGuard` restores the terminal on every exit, non-TTY keeps the exact one-line hints so smoke tests stay green); role dispatch with `Role`/`parse_role`/`destination_for`/`dispatch_role` and `--role` in [src/tui/args.rs](../src/tui/args.rs) (`Candidate` → Phase 9 placeholder with generic sync message only in [src/tui/candidate.rs](../src/tui/candidate.rs), no expert diagnostics), `sanitize_display` control/escape stripping in [src/tui/shared.rs](../src/tui/shared.rs), `PreflightView`/`can_proceed`/`expert_ssh_hint` in [src/tui/expert.rs](../src/tui/expert.rs) rendering the typed [src/sandbox/preflight.rs](../src/sandbox/preflight.rs) `PreflightReport` verbatim (Docker `Fail` = hard stop via `can_start()`, `Warn` may proceed when allowed); typed SSH-failure classifier in [src/sync/failure.rs](../src/sync/failure.rs) (`SshFailureKind`/`classify_ssh_failure`, expert `ssh <target>` guidance vs candidate `"Sync problem — ask the expert"`) re-exported by [src/sync/mod.rs](../src/sync/mod.rs); secure API-key storage in [src/tui/config.rs](../src/tui/config.rs) (`MODEL_API_KEY` env wins over stored `$HOME/.config/pairproof/api_key`, `0600`/`0700`, full-ancestor symlink rejection, `mask_api_key`, `ApiKeyConfig`, `RawModeGuard`); setup/context controller in [src/tui/setup.rs](../src/tui/setup.rs) (disclosure gate before `propose_context`, paste-only 50k bounds/truncation, `LanguageProposal::Unsupported` warning, confirm/override, auto-5-easy without preview); connection controller in [src/tui/connection.rs](../src/tui/connection.rs) (`SecretRotator` countdown/expiry/reissue/terminate seam, no timer-inferred connection); expert selection in [src/tui/selection.rs](../src/tui/selection.rs) (3 medium → 2 hard, same-difficulty re-question approve/reject, one-extra via core); live monitor/kill-switch/final in [src/tui/monitor.rs](../src/tui/monitor.rs) (permitted data only, no chat, `TestSummary` verbatim, paste alerts, ordered idempotent cleanup with typed `CancellationSeam` `Unsupported` by default, `final_view` via `final_rating` without fabricated report). Tests: 109 new Phase 8 tests green — 10 in [tests/sync_failure.rs](../tests/sync_failure.rs), 7 in [tests/tui_preflight.rs](../tests/tui_preflight.rs), 12 in [tests/tui_role.rs](../tests/tui_role.rs), 11 in [tests/tui_config.rs](../tests/tui_config.rs), 12 in [tests/tui_setup.rs](../tests/tui_setup.rs), 15 in [tests/tui_connection.rs](../tests/tui_connection.rs), 10 in [tests/tui_selection.rs](../tests/tui_selection.rs), 15 in [tests/tui_monitor.rs](../tests/tui_monitor.rs), 17 in [tests/tui_app.rs](../tests/tui_app.rs) — covering every named Phase 8 flow/error path (hard-stop vs warnings, disclosure gate, truncation, unsupported language, countdown half-open boundary, reissue invalidation, 3+2 order, re-question difficulty, extra-once, monitor chat-exclusion, test-summary, paste alert, disconnect-vs-end, kill-switch failure/order/idempotence, not-scored, app navigation/back/quit/illegal-transition) plus symlink-chain and secret-redaction probes. `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` are clean; largest `tui` file is 380 lines ([src/tui/monitor.rs](../src/tui/monitor.rs)). Manager review: 8A–8E each went diff → gates → independent probes → fix; the 8B probe caught a deep-ancestor symlink bypass in `save_api_key`, fixed by full-chain `reject_ancestor_symlinks` with regression `symlink_ancestor_is_rejected` and re-verified; the 8E probe covered illegal events, the connection gate, disconnect-stays-live, sanitization, and bounded history. No live Meta/Docker checks were run in Phase 8 (all deterministic). Phase 10 seams were not faked and stay tracked in Phase 10: production connection→core glue (SSH accept detection feeding `SecretRotator::verify` + `Session::candidate_connected`) is a tested seam only; API cancellation over in-flight requests is a typed `Unsupported` seam; Markdown report generation/copy-before-destroy is Phase 10, not claimed (`FinalView::artefact_note` says so). Evidence: uncommitted working tree on branch `dev` — no commit hash exists for this work yet.

**Remaining work.**

- [ ] None in Phase 8. Phase 10 seams (SSH accept → `verify`/`candidate_connected`, transport cancellation, report generation) are tracked in Phase 10, not faked here.

## Phase 9 — TUI, candidate side

**Goal.** Candidate screens: connect, rules, split question/chat work, final score (structure.md sections 3, 10, 11, 13, 16; `tui/candidate`).

**Scope.**

- Connect by pasting the connection string (incl. expired-string message, Phase 6); rules/consent screen (single workspace, no external resources needed, bulk pastes flagged).
- Split screen: left = statement, 4+1 approach selection (`Other` free text), solution progress, actions (`Continue`, `Redirect`, `Change approach`, `Request tests / verify`, `Finish question` with latest test summary shown, `Skip`, `Re-question` where allowed); right = chat pane (Phase 4). Solution files editable in the candidate's editor via Phase 7 sync.
- Paste blocked in PairProof inputs (chat, `Other` box), section 16.
- Waiting screen "Expert is preparing questions" (core `WaitingForExpert` already models this).
- Score hidden until the final screen; final breakdown per section 13.

**Exit criteria.**

- [ ] Tests per must.md for each candidate flow/transition (connect incl. expired string, rules accept, every action wired to core, waiting state, score only at end) at the state/logic level.

**Status: Not started.** [src/tui/candidate.rs](../src/tui/candidate.rs) is a 1-line stub.

**Remaining work.**

- [ ] Candidate screen set wired to core actions, Phase 4 loops, Phase 7 sync.

## Phase 10 — End-to-end integration and failure handling

**Goal.** One real session, expert setup to final report, with honest failure endings (structure.md sections 3, 14, 15, 17, 18; initial.md sections 12–13).

**Scope.**

- Wire all modules into the section 3 flows; end conditions: expert end, or candidate completes 10/11. Elapsed and per-question time recorded/reported.
- Service failure (section 18): LeetCode/Meta health checks; both screens name the unreachable service(s) with a 60 s retry countdown; resume where it stopped if back, else terminate as technical failure with a reschedule message and **no candidate evaluation**. Consumes the Phase 1 rate-limit risk.
- Disconnect/resume (section 17): container + state survive; fresh string resumes at the same question; disconnects logged, never score-affecting.
- Close-out: per-question scoring (Phase 4); failed-after-retries = "not scored", 0 correction, stated in the report; final merge (Phase 3); both sides see the breakdown.
- Report generation from a real session: Markdown report (breakdown, per-question grades/rationales, follow-up suggestions, relevant chat excerpts only), solutions, `events.jsonl`, token record, context summary — copied out before container destruction; raw pastes, API key, string, secret never retained (section 15).

**Exit criteria.**

- [ ] Integration tests per must.md for the full arc and each failure path (service-down pause/resume/terminate, disconnect/resume, kill switch mid-request, scoring failure → "not scored", report from a scripted session) in [tests/](../tests/).
- [ ] One scripted session runs end to end producing all section 15 artifacts.

**Status: Not started.** Nothing is wired across modules: the binary prints the not-implemented message ([src/tui/mod.rs](../src/tui/mod.rs)). Core models `TechnicalFailure`/`EndedByExpert`, but no health check, countdown, resume, or report generation exists.

**Remaining work.**

- [ ] Session orchestration over Phases 3–9; sections 17–18 failure handling; report generator; integration tests.

## Phase 11 — Hardening and polish (P1)

**Goal.** Make the working MVP presentable and defensible (initial.md P1; must.md standards).

**Scope.**

- TUI polish: help, clear error messages, countdown/waiting states.
- Evidence links from each score line to the underlying `events.jsonl` event.
- Scoring-consistency runs: grade the same recorded session twice, compare.
- Limitations footer in the report (validity scope; decision support, never automated hiring decisions — [must.md](must.md) section 6).
- Gates: `cargo fmt --check`, `cargo clippy -- -D warnings`, full `cargo test`, 400-line limit re-checked.

**Exit criteria.**

- [ ] All gates green; P1 items present, with tests where they add behaviour (evidence-link generation, consistency harness).

**Status: Not started.** The standing gates are incidentally green today (fmt/clippy clean; largest source file 399 lines, [src/llm/client.rs](../src/llm/client.rs)); no P1 item is built.

**Remaining work.**

- [ ] All scope items, after Phase 10.

## Phase 12 — Demo and submission

**Goal.** Ship before the deadline ([must.md](must.md) sections 3, 5, 7, 8; initial.md sections 13, 15).

**Scope.**

- Rehearse the 3-minute demo (initial.md section 13) end to end from a clean machine; record the required 3-minute video; verify the link works.
- Repo public and verified accessible; hackathon-built work clearly identified; stale README status fixed; problem statement chosen/stated; no "job application screener" positioning.
- Team ≤ 4, one Representative, all contributors listed, one submission per team.
- Submit via Cerebral Valley **before Nov 4, 9:00 PM EST** ([must.md](must.md) section 7); Nov 3 is the last full day for submission questions.

**Exit criteria.**

- [ ] [must.md](must.md) Pre-Submission Compliance Checklist fully checked, demo run from a clean checkout as final verification.

**Status: Not started.** No demo assets, video, or submission record in the repo; repo publicity not verified from this workspace.

**Remaining work.**

- [ ] All scope items.

## Out of MVP / future (initial.md P2)

Explicitly **not** MVP phases (structure.md section 2; initial.md P2). Do not build before submission; design only must not preclude them:

- VPS reverse-tunnel relay mode (expert-supplied VPS).
- More solution languages (Java first); MVP is Python, JavaScript, and SQL only.
- Additional difficulty/topic strategies and role-specific rubrics.
- Multi-challenge comparability, team analytics, multi-candidate comparison.
- Also out per structure.md: authored/snapshotted questions, in-app screen sharing, edit-authorship tracking, hard token caps, web UI/accounts/cloud backend.

## Current state

**Phases 0–8 are Done.** The spec is Final (`fe9f15c` skeleton), both clients landed in `281cd95` with models settled in `645581f`, the core landed in `f45a6c4` and its completion work after that, the scoring engine (Phase 3) is in [src/scoring/](../src/scoring/), LLM orchestration plus LeetCode selection (Phase 4) are in [src/llm/](../src/llm/) and [src/leetcode/selection.rs](../src/leetcode/selection.rs), paste-only ingest (Phase 5) is in [src/ingest/mod.rs](../src/ingest/mod.rs) and [src/ingest/pipeline.rs](../src/ingest/pipeline.rs), and the rotating-secret connection string and its lifecycle (Phase 6) are in [src/connection/](../src/connection/) (`error`, `secret`, `string`, `rotator`; 51 integration tests: 25 in `tests/connection_string.rs`, 16 in `tests/connection_lifecycle.rs`, 10 in `tests/connection_reissue.rs`); the disposable sandbox plus Mutagen sync (Phase 7) are in [src/sandbox/](../src/sandbox/) (`command`, `error`, `exec`, `image`, `lifecycle`, `parse`, `preflight`, `process`, `validate`, `workspace` + `mod.rs`), [src/sync/](../src/sync/) (`command`, `error`, `lifecycle`, `paste` + `mod.rs`), and [sandbox/](../sandbox/) (`Dockerfile`, `entrypoint.sh`, `sshd_config`); Phase 8 adds the expert TUI in [src/tui/](../src/tui/) (`app`, `args`, `shared`, `config`, `setup`, `connection`, `selection`, `monitor`, `expert`, `candidate`) plus the SSH-failure classifier in [src/sync/failure.rs](../src/sync/failure.rs), all uncommitted on branch `dev`, so no commit hash exists for this work yet. Phase 6 ships no networking/SSH and no production connection→core glue (the seam is proven by an integration test only); SSH provisioning landed in Phase 7 and wiring belongs to Phase 10 — Phase 8 keeps that as a tested `ConnectionController::on_candidate_connected` seam and does not fake SSH accept detection. Two mechanical refactors are also recorded: the former `cli` module was merged into `tui/` (args in [src/tui/args.rs](../src/tui/args.rs), `run()` in [src/tui/mod.rs](../src/tui/mod.rs)), and the former `ai` module was renamed to `llm/` (including the `LlmError`/`LlmOutcome` types, `IngestError::Llm`, and the `tests/llm_*.rs` test files). The paste-only product decision is recorded in Phase 5: a pasted job description (required) plus optional additional notes, file-upload dropped, no new dependency. Phase 3 × Phase 4 composition is proven offline by `tests/scoring_llm_compose.rs` (6 tests, no adapters). Three later refinements are also landed: extraction now chooses the language from the JD and additional notes taken together (Python only when neither names one), maps the role domain to the closest slugs from the `KNOWN_TOPICS` allowlist (25 topics, including `database`) and drops unknown slugs (a live ML-engineer JD check on 2026-10-28 produced exactly `matrix`, `math`, `dynamic-programming`, `graph`, `heap-priority-queue`), and scoring applies an acceptance factor on top of the difficulty multiplier. SQL is now a supported solution language (`Language::Sql`, `.sql` files, LeetCode slug `mysql`; MSSQL/T-SQL stay unsupported): extraction maps the aliases `sql`/`mysql`/`postgresql`/`postgres`/`psql` to it, and `parse_extraction` enforces coherence — SQL sessions draw only from the `database` topic (forced to `["database"]` if needed) and non-SQL results drop `database`. Both live SQL checks passed on 2026-10-28: database questions carry a MySQL snippet and no `python3` snippet (`live_database_question_has_mysql_snippet` in `tests/leetcode_live.rs`), and a live MySQL data-role JD proposed `Sql` with `["database"]` (`live_extract_sql_role_proposes_sql_and_database` in `tests/llm_live_smoke.rs`). A third live check, `live_database_full_composition_selectable` in `tests/leetcode_live.rs`, proves the full 5 easy + 3 medium + 2 hard database composition is selectable live (free database pool: easy 51+, medium 23+, hard 7). `cargo test` reports 476 passed, 0 failed, 40 ignored (85 lib unit tests; 109 new Phase 8 integration tests across 9 new suites); of the 40 ignored, 16 are the pre-existing API live tests (last run and passed on 2026-10-28, NOT re-run in this phase) and 24 are new Phase 7 live tests, all run and passed live on 2026-10-29 on Docker 29.9.2 + Mutagen 0.18.1; Phase 8 ran no live checks. `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` are clean, and the largest source file is 399 lines ([src/llm/client.rs](../src/llm/client.rs), under the 400 cap). Phases 9–12 are Not started; the Phase 10 seams (connection→core glue, API cancellation, report generation) remain open as listed in the Phase 8 section. The stale README status remains tracked in Phase 12. No phase is Blocked; external risks (LeetCode rate limits, CGNAT in Direct mode) are carried in Phases 7 and 10. Next: Phase 9 (TUI — candidate).

## How to update this file

When a phase's exit criteria pass, flip its status in **both** the summary table and its section, check off the exit-criteria boxes, and add the evidence (file paths, test files with counts, commit hashes). Keep statuses honest: implemented but untested/uncommitted is In progress; a stub plus a prerequisite from an earlier phase is Not started. Refresh the snapshot numbers whenever test totals move, and rewrite [Current state](#current-state) whenever the active phase changes.
