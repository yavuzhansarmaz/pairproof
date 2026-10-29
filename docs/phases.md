# PairProof — Implementation Phases

This file tracks the implementation phases for the PairProof MVP defined in [structure.md](structure.md) (the authoritative build specification), with product rationale and P0/P1/P2 priorities in [initial.md](initial.md) and hackathon deadlines plus internal engineering standards in [must.md](must.md). It records what is actually built, phase by phase, with evidence, so progress toward the submission deadline is visible at a glance. On any disagreement between this file and the code, the code wins; on any disagreement about intended behaviour, [structure.md](structure.md) wins.

## Status legend

- **Done** — exit criteria met, verified in code and tests.
- **In progress** — implementation exists but exit criteria are not fully met.
- **Not started** — no implementation beyond stubs or prerequisites credited elsewhere.
- **Blocked** — cannot proceed without an external decision, access, or dependency. Name the blocker in the phase section.

## Summary

Snapshot at time of writing: `cargo test` reports **261 passed, 0 failed, 16 ignored** (53 unit tests in `src/lib.rs`; 21 in `tests/connection_string.rs`; 16 in `tests/connection_lifecycle.rs`; 10 in `tests/connection_reissue.rs`; 17 in `tests/llm_approaches.rs`; 10 in `tests/llm_chat.rs`; 15 in `tests/llm_extraction.rs`; 8 in `tests/llm_grading.rs`; 11 in `tests/core_artifacts.rs`; 7 in `tests/core_coverage.rs`; 12 in `tests/core_event_log.rs`; 9 in `tests/core_records.rs`; 15 in `tests/ingest_pipeline.rs`; 17 in `tests/ingest_raw.rs`; 8 in `tests/ingest_text.rs`; 12 in `tests/leetcode_selection.rs`; 6 in `tests/phase3_phase4_compose.rs`; 8 in `tests/scoring_final.rs`; 4 in `tests/scoring_live.rs`; 2 in `tests/smoke.rs`; 8 + 5 + 3 live tests ignored in `tests/llm_live_smoke.rs`, `tests/leetcode_live.rs`, and `tests/meta_api_live.rs`). All 16 ignored live tests were run and passed on 2026-10-28, including the SQL ones (`live_database_question_has_mysql_snippet` and `live_database_full_composition_selectable` in `tests/leetcode_live.rs`, `live_extract_sql_role_proposes_sql_and_database` in `tests/llm_live_smoke.rs`). `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` are clean. The largest source file is 379 lines (`src/llm/client.rs`).

| Phase | Name | Status | Evidence | Depends on |
| --- | --- | --- | --- | --- |
| 0 | Concept and specification | Done | [structure.md](structure.md) "Final"; [initial.md](initial.md); [must.md](must.md); commits `fe9f15c` (skeleton), `570a5ec`, `c2395a9` | — |
| 1 | Validation spikes (LeetCode, Meta API) | Done | [src/leetcode/mod.rs](../src/leetcode/mod.rs), [src/llm/mod.rs](../src/llm/mod.rs); commit `281cd95`; model choice in commit `645581f` and [must.md](must.md) | 0 |
| 2 | Core domain (session, events, artifacts) | Done | [src/core/](../src/core/) (8 modules); 60 core-related tests green at phase close (now 61) | 0, 1 |
| 3 | Scoring engine | Done | [src/scoring/types.rs](../src/scoring/types.rs), [src/scoring/live.rs](../src/scoring/live.rs), [src/scoring/final_merge.rs](../src/scoring/final_merge.rs); 31 tests green (19 unit + 12 integration) + composition tests in `tests/phase3_phase4_compose.rs` | 2 (merge also needs 4) |
| 4 | LLM orchestration and prompts | Done | [src/llm/](../src/llm/) (client, common, approaches, coding, chat, extraction, grading), [src/leetcode/selection.rs](../src/leetcode/selection.rs); 9 unit + 57 integration tests green at phase close (extraction integration now 15 with SQL/coherence tests; approaches now 17), 7 live smoke at phase close (now 8) run and passed live on 2026-10-28 | 1, 2 |
| 5 | Ingest (JD / additional notes) | Done | [src/ingest/mod.rs](../src/ingest/mod.rs), [src/ingest/pipeline.rs](../src/ingest/pipeline.rs); 38 ingest tests green at phase close (now 40: 8 in `tests/ingest_text.rs`, 17 in `tests/ingest_raw.rs`, 15 in `tests/ingest_pipeline.rs`) plus `tests/llm_extraction.rs` now 15; product decision: paste-only, file-upload dropped, no new dependency | 2, 4 |
| 6 | Connection (rotating-secret string) | Done | [src/connection/](../src/connection/) (`error`, `secret`, `string`, `rotator`); 47 tests green (21 in `tests/connection_string.rs`, 16 in `tests/connection_lifecycle.rs`, 10 in `tests/connection_reissue.rs`) | 2 |
| 7 | Sandbox and sync (Docker, Mutagen) | Not started | [sandbox/Dockerfile](../sandbox/Dockerfile) exists; [src/sandbox/mod.rs](../src/sandbox/mod.rs), [src/sync/mod.rs](../src/sync/mod.rs) are 1-line stubs | 2, 6 |
| 8 | TUI — expert | Not started | [src/tui/expert.rs](../src/tui/expert.rs) stub; [src/tui/mod.rs](../src/tui/mod.rs) prints "role selection is not implemented yet" | 2–7 |
| 9 | TUI — candidate | Not started | [src/tui/candidate.rs](../src/tui/candidate.rs) stub | 2, 4, 6, 7 |
| 10 | End-to-end integration and failure handling | Not started | No cross-module flow wired; only per-module tests exist | 3–9 |
| 11 | Hardening and polish (P1) | Not started | fmt/clippy clean today, but no P1 items built | 10 |
| 12 | Demo and submission | Not started | No demo or submission artifacts in repo | 10, 11 |

**Phases 0–6 are Done; the project is ready for Phase 7.** No later phase has implementation beyond stubs and one Dockerfile. Phase 7 (sandbox and sync) is the next step; its Phase 6 dependency is now satisfied. See [Current state](#current-state).

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
- Meta Model API client: model listing, chat, streaming chat with token usage ([src/llm/mod.rs](../src/llm/mod.rs): `MetaClient::list_models`, `chat`, `chat_stream`); key from `MODEL_API_KEY`.
- Settled model IDs: strong `muse-wonder-1.0`, light `muse-spark-1.3` (`STRONG_MODEL`/`LIGHT_MODEL`).

**Exit criteria.**

- [x] Live tests for: free catalog, statement+snippets, premium rejection ([tests/leetcode_live.rs](../tests/leetcode_live.rs)); model visibility, strong chat usage, light streaming usage ([tests/meta_api_live.rs](../tests/meta_api_live.rs)). `#[ignore]`d (network/key); run with `-- --ignored`.
- [x] Offline parsing/error unit tests: 3 in `src/leetcode/mod.rs`, 3 in `src/llm/mod.rs`, green in the current `cargo test` run.
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

**Status: Done.** `src/llm` is split into [src/llm/client.rs](../src/llm/client.rs) (`ReasoningEffort`/`ChatOptions::low`, `chat*_with_options`, old signatures preserved), [src/llm/common.rs](../src/llm/common.rs) (`LlmError`/`LlmOutcome`), [src/llm/approaches.rs](../src/llm/approaches.rs) (4+1 generation with seeded-shuffle parsing, `Other` grading), [src/llm/coding.rs](../src/llm/coding.rs) (single coding-step prompt/interface, `apply_usage` via `Session::add_tokens`, tokens booked before parsing; **no** sandbox execution), [src/llm/chat.rs](../src/llm/chat.rs) (section 10 assembly, hidden-data-free live-state block, boundary-violation logging via `ChatBoundaryViolated`), [src/llm/extraction.rs](../src/llm/extraction.rs) (section 5 combined JD + additional-notes language priority, `KNOWN_TOPICS` allowlist with unknown slugs dropped in `parse_extraction`, plus normalization/unsupported surfacing), and [src/llm/grading.rs](../src/llm/grading.rs) (section 14 one request per question; `grade_questions` turns failures into `NotScored`); randomized topic-pool selection is in [src/leetcode/selection.rs](../src/leetcode/selection.rs) (`pick_random`/`expert_pool`/`select_questions`/`auto_pick_easy`). Tests: 9 new unit (client 7 incl. 3 moved from Phase 1, common 2) plus integration [tests/llm_approaches.rs](../tests/llm_approaches.rs) (16; now 17 with `sql_language_names_mysql_in_prompts`), [tests/llm_chat.rs](../tests/llm_chat.rs) (10), [tests/llm_extraction.rs](../tests/llm_extraction.rs) (7 at phase close; now 15 with topic-filter and SQL/coherence tests), [tests/llm_grading.rs](../tests/llm_grading.rs) (8), [tests/leetcode_selection.rs](../tests/leetcode_selection.rs) (11 at phase close; now 12 with the acceptance-mapping test) — all green; live smoke tests in [tests/llm_live_smoke.rs](../tests/llm_live_smoke.rs) (7 at phase close; now 8 with the live SQL-role extraction test) are `#[ignore]`d by default and were run and passed live on 2026-10-28. A live ML-engineer JD extraction check on 2026-10-28 produced exactly `matrix`, `math`, `dynamic-programming`, `graph`, `heap-priority-queue`. Evidence: uncommitted working tree on branch `dev` — no commit hash exists for this work yet.

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

- [x] Tests per must.md for the lifecycle: encode/decode round-trip, field allowlist, rotation/expiry, connect kills secret, second-connect and expired-string rejection, re-issue ([tests/connection_string.rs](../tests/connection_string.rs) 21, [tests/connection_lifecycle.rs](../tests/connection_lifecycle.rs) 16, [tests/connection_reissue.rs](../tests/connection_reissue.rs) 10 = 47, all green).

**Status: Done.** Implemented in [src/connection/error.rs](../src/connection/error.rs) (`ConnectionError`), [src/connection/secret.rs](../src/connection/secret.rs) (`Secret`), [src/connection/string.rs](../src/connection/string.rs) (`Endpoint`/`EphemeralUsername`/`Fingerprint`/`ConnectionString`), and [src/connection/rotator.rs](../src/connection/rotator.rs) (`SecretRotator`/`ConnectionState`/`ROTATION_WINDOW_MS`), declared and re-exported by [src/connection/mod.rs](../src/connection/mod.rs); no new dependencies. Canonical single-line format: `pairproof:v1:<host>:<port>:<username>:<secret_hex>:<expires_at_ms>:<fingerprint_hex>` (exactly 8 colon-separated fields). The section 6 allowlist holds by construction — the component newtypes make forbidden fields (API key, real username/home path, personal data, permanent credentials) unrepresentable — and strict parsing rejects extra/missing fields, a wrong version, any whitespace or control characters, and non-canonical numbers. `Secret` is 16 bytes (128-bit) from `rand`'s `OsRng`, encoded as 32 lowercase hex chars, compared in constant time, with `Debug` redacted (a test proves `Debug` never contains the hex). Host grammar: a public IPv4 address (`0.0.0.0` rejected) or a lowercased DNS hostname; IPv6 is explicitly rejected with a typed error (documented Direct-mode limitation, since `:` is the field separator). `Fingerprint` is caller-supplied opaque lowercase hex, even length 32–128 (Phase 7 supplies the hex-encoded sandbox SSH host-key hash). Lifecycle runs in explicit milliseconds (`now_ms`, the core convention): `ROTATION_WINDOW_MS` = 60,000; states are `Rotating` (countdown in seconds, ceiling, capped at 60), `Locked` (the first successful `verify` kills the secret), and `Ended` (`terminate`, final). `verify` rejects, in precedence order, with typed errors: `SessionEnded`, `AlreadyLocked`, `FingerprintMismatch`, `EndpointMismatch`, `UsernameMismatch`, `Expired` (judged only against the rotator's own window — the string's expiry field is never trusted; validity is half-open, [issue, expiry)), `WrongSecret`. `reissue()` from `Rotating`/`Locked` mints a fresh secret and a full window with endpoint/username/fingerprint unchanged and all earlier secrets dead; it is impossible after `Ended`. There is no networking or SSH in this phase (the Phase 7 sandbox owns SSH) and no production connection→core glue — the seam is proven only by the integration test `core_seam_reconnect_story` in [tests/connection_reissue.rs](../tests/connection_reissue.rs), which drives a real core `Session`; wiring belongs to Phases 8/10. Manager attack-probe rounds (temporary, removed after passing) covered unicode/invisible characters, swapped fields, `u64`/`u16` overflow inputs, tampered expiry/fingerprint/endpoint/username, verify-does-not-rotate, double reissue, disconnect-without-reissue, and `u64::MAX` time saturation; an endpoint audit found the `Malformed`/`InvalidExpiry` variants unasserted, closed with `parse_malformed_variant_for_shape_errors` and `parse_invalid_expiry_variant_for_bad_expiry`. Evidence: uncommitted working tree on branch `dev` — no commit hash exists for this work yet.

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

- [ ] Tests per must.md for each lifecycle operation (image build/cache, container create/destroy, Mutagen start/stop, preflight computation): success, missing-Docker/Mutagen, and failure paths — command construction/error mapping at minimum, live Docker runs where feasible.

**Status: Not started.** Only [sandbox/Dockerfile](../sandbox/Dockerfile) exists (debian-slim + openssh-server, python3, nodejs, `/workspace`, `sshd` CMD); [src/sandbox/mod.rs](../src/sandbox/mod.rs) and [src/sync/mod.rs](../src/sync/mod.rs) are 1-line stubs. The Dockerfile creates no ephemeral user and bakes in no host keys (correct for a shared image) — user/credential provisioning at container start is part of this phase.

**Remaining work.**

- [ ] Implement `sandbox/` lifecycle + preflight + in-container test execution.
- [ ] Add a MySQL runner to `sandbox/Dockerfile` and SQL (`.sql`) runnability/test execution in the sandbox — SQL language/coherence support is landed (see Current state), sandbox SQL execution is not.
- [ ] Implement `sync/` Mutagen management + workspace layout + paste heuristic.

## Phase 8 — TUI, expert side

**Goal.** Expert screens: preflight, setup, live selection and monitoring (structure.md sections 3, 7, 9, 12; `tui/` args/startup/role dispatch, `tui/expert`).

**Scope.**

- Role selection (`Expert`) in [src/tui/](../src/tui/); preflight checklist with concrete failure guidance.
- API key: user-private `0600` config, masked, deletable, env-var override, never logged (section 9).
- Setup: JD/additional-notes entry (Phase 5), topics/level/language proposal with confirm/override, unsupported-language warning, Meta API disclosure; system auto-selects the 5 easy (no expert preview).
- Connection screen with 60 s countdown (Phase 6); on connect, live selection: 3 medium then 2 hard on separate screens while the candidate works; approve/reject re-question and pick the same-difficulty replacement.
- Live monitoring: current question, chosen approach, test results, elapsed time, live rating breakdown (Phase 3), paste-flag alerts; no live chat content.
- Actions: add 1 extra question; global kill switch (cancel API calls, close connection, destroy container, section 17); fresh string after accidental disconnect; final breakdown + artifacts.

**Exit criteria.**

- [ ] Tests per must.md for each screen transition/expert flow (preflight outcomes, setup, selection submit, re-question resolve, extra, kill switch) at the state/logic level beneath rendering, incl. error paths.

**Status: Not started.** [src/tui/mod.rs](../src/tui/mod.rs) (`run()`, args in [src/tui/args.rs](../src/tui/args.rs)) parses empty args and prints "pairproof: role selection is not implemented yet"; [src/tui/expert.rs](../src/tui/expert.rs) and [src/tui/shared.rs](../src/tui/shared.rs) are 1-line stubs.

**Remaining work.**

- [ ] Role dispatch in `tui/`; expert screens + shared widgets; API key storage (0600, masking, env override).

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

**Status: Not started.** The standing gates are incidentally green today (fmt/clippy clean; largest source file 379 lines, [src/llm/client.rs](../src/llm/client.rs)); no P1 item is built.

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

**Phases 0–6 are Done.** The spec is Final (`fe9f15c` skeleton), both clients landed in `281cd95` with models settled in `645581f`, the core landed in `f45a6c4` and its completion work after that, the scoring engine (Phase 3) is in [src/scoring/](../src/scoring/), LLM orchestration plus LeetCode selection (Phase 4) are in [src/llm/](../src/llm/) and [src/leetcode/selection.rs](../src/leetcode/selection.rs), paste-only ingest (Phase 5) is in [src/ingest/mod.rs](../src/ingest/mod.rs) and [src/ingest/pipeline.rs](../src/ingest/pipeline.rs), and the rotating-secret connection string and its lifecycle (Phase 6) are in [src/connection/](../src/connection/) (`error`, `secret`, `string`, `rotator`; 47 integration tests: 21 in `tests/connection_string.rs`, 16 in `tests/connection_lifecycle.rs`, 10 in `tests/connection_reissue.rs`); the Phase 3–6 work is uncommitted on branch `dev`, so no commit hash exists for it yet. Phase 6 ships no networking/SSH and no production connection→core glue (the seam is proven by an integration test only); SSH belongs to Phase 7 and wiring to Phases 8/10. Two mechanical refactors are also recorded: the former `cli` module was merged into `tui/` (args in [src/tui/args.rs](../src/tui/args.rs), `run()` in [src/tui/mod.rs](../src/tui/mod.rs)), and the former `ai` module was renamed to `llm/` (including the `LlmError`/`LlmOutcome` types, `IngestError::Llm`, and the `tests/llm_*.rs` test files). The paste-only product decision is recorded in Phase 5: a pasted job description (required) plus optional additional notes, file-upload dropped, no new dependency. Phase 3 × Phase 4 composition is proven offline by `tests/phase3_phase4_compose.rs` (6 tests, no adapters). Three later refinements are also landed: extraction now chooses the language from the JD and additional notes taken together (Python only when neither names one), maps the role domain to the closest slugs from the `KNOWN_TOPICS` allowlist (25 topics, including `database`) and drops unknown slugs (a live ML-engineer JD check on 2026-10-28 produced exactly `matrix`, `math`, `dynamic-programming`, `graph`, `heap-priority-queue`), and scoring applies an acceptance factor on top of the difficulty multiplier. SQL is now a supported solution language (`Language::Sql`, `.sql` files, LeetCode slug `mysql`; MSSQL/T-SQL stay unsupported): extraction maps the aliases `sql`/`mysql`/`postgresql`/`postgres`/`psql` to it, and `parse_extraction` enforces coherence — SQL sessions draw only from the `database` topic (forced to `["database"]` if needed) and non-SQL results drop `database`. Both live SQL checks passed on 2026-10-28: database questions carry a MySQL snippet and no `python3` snippet (`live_database_question_has_mysql_snippet` in `tests/leetcode_live.rs`), and a live MySQL data-role JD proposed `Sql` with `["database"]` (`live_extract_sql_role_proposes_sql_and_database` in `tests/llm_live_smoke.rs`). A third live check, `live_database_full_composition_selectable` in `tests/leetcode_live.rs`, proves the full 5 easy + 3 medium + 2 hard database composition is selectable live (free database pool: easy 51+, medium 23+, hard 7). `cargo test` reports 261 passed, 0 failed, 16 ignored; all 16 ignored live tests were run and passed on 2026-10-28, including the SQL ones; `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` are clean, and the largest source file is 379 lines ([src/llm/client.rs](../src/llm/client.rs)). Sandbox SQL execution is not built: Phase 7 must add a MySQL runner to `sandbox/Dockerfile`, so no SQL session can execute in-container yet. Phases 7–12 are Not started: the sandbox and sync modules and the tui screens are stubs, the binary still prints "role selection is not implemented yet" ([src/tui/mod.rs](../src/tui/mod.rs)), and the only later-phase artifact beyond stubs is [sandbox/Dockerfile](../sandbox/Dockerfile). The stale README status remains tracked in Phase 12. No phase is Blocked; external risks (LeetCode rate limits, CGNAT in Direct mode) are carried in Phases 7 and 10. Next: Phase 7 (sandbox and sync), which consumes Phase 6's `Endpoint`, `EphemeralUsername`, and `Fingerprint` types.

## How to update this file

When a phase's exit criteria pass, flip its status in **both** the summary table and its section, check off the exit-criteria boxes, and add the evidence (file paths, test files with counts, commit hashes). Keep statuses honest: implemented but untested/uncommitted is In progress; a stub plus a prerequisite from an earlier phase is Not started. Refresh the snapshot numbers whenever test totals move, and rewrite [Current state](#current-state) whenever the active phase changes.
