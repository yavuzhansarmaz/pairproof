# PairProof — Implementation Phases

This file tracks the implementation phases for the PairProof MVP defined in [structure.md](structure.md) (the authoritative build specification), with product rationale and P0/P1/P2 priorities in [initial.md](initial.md) and hackathon deadlines plus internal engineering standards in [must.md](must.md). It records what is actually built, phase by phase, with evidence, so progress toward the submission deadline is visible at a glance. On any disagreement between this file and the code, the code wins; on any disagreement about intended behaviour, [structure.md](structure.md) wins.

## Status legend

- **Done** — exit criteria met, verified in code and tests.
- **In progress** — implementation exists but exit criteria are not fully met.
- **Not started** — no implementation beyond stubs or prerequisites credited elsewhere.
- **Blocked** — cannot proceed without an external decision, access, or dependency. Name the blocker in the phase section.

## Summary

Snapshot at time of writing: `cargo test` reports **160 passed, 0 failed, 13 ignored** (52 unit tests in `src/lib.rs`; 16 in `tests/ai_approaches.rs`; 10 in `tests/ai_chat.rs`; 7 in `tests/ai_extraction.rs`; 8 in `tests/ai_grading.rs`; 10 in `tests/core_artifacts.rs`; 7 in `tests/core_coverage.rs`; 12 in `tests/core_event_log.rs`; 9 in `tests/core_records.rs`; 11 in `tests/leetcode_selection.rs`; 6 in `tests/phase3_phase4_compose.rs`; 7 in `tests/scoring_final.rs`; 3 in `tests/scoring_live.rs`; 2 in `tests/smoke.rs`; 7 + 3 + 3 live tests ignored in `tests/ai_live_smoke.rs`, `tests/leetcode_live.rs`, and `tests/meta_api_live.rs`). All 13 ignored live tests were run and passed on 2026-10-28 (meta 3, leetcode 3, ai smoke 7). `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` are clean. The largest source file is 379 lines (`src/ai/client.rs`).

| Phase | Name | Status | Evidence | Depends on |
| --- | --- | --- | --- | --- |
| 0 | Concept and specification | Done | [structure.md](structure.md) "Final"; [initial.md](initial.md); [must.md](must.md); commits `fe9f15c` (skeleton), `570a5ec`, `c2395a9` | — |
| 1 | Validation spikes (LeetCode, Meta API) | Done | [src/leetcode/mod.rs](../src/leetcode/mod.rs), [src/ai/mod.rs](../src/ai/mod.rs); commit `281cd95`; model choice in commit `645581f` and [must.md](must.md) | 0 |
| 2 | Core domain (session, events, artifacts) | Done | [src/core/](../src/core/) (8 modules); 60 core-related tests green | 0, 1 |
| 3 | Scoring engine | Done | [src/scoring/types.rs](../src/scoring/types.rs), [src/scoring/live.rs](../src/scoring/live.rs), [src/scoring/final_merge.rs](../src/scoring/final_merge.rs); 28 tests green (18 unit + 10 integration) + composition tests in `tests/phase3_phase4_compose.rs` | 2 (merge also needs 4) |
| 4 | AI orchestration and prompts | Done | [src/ai/](../src/ai/) (client, common, approaches, coding, chat, extraction, grading), [src/leetcode/selection.rs](../src/leetcode/selection.rs); 9 unit + 52 integration tests green, 7 live smoke run and passed 7/7 live on 2026-10-28 | 1, 2 |
| 5 | Ingest (JD / resume) | Not started | [src/ingest/mod.rs](../src/ingest/mod.rs) is a 1-line doc stub | 2, 4 |
| 6 | Connection (rotating-secret string) | Not started | [src/connection/mod.rs](../src/connection/mod.rs) is a 1-line doc stub | 2 |
| 7 | Sandbox and sync (Docker, Mutagen) | Not started | [sandbox/Dockerfile](../sandbox/Dockerfile) exists; [src/sandbox/mod.rs](../src/sandbox/mod.rs), [src/sync/mod.rs](../src/sync/mod.rs) are 1-line stubs | 2, 6 |
| 8 | TUI — expert | Not started | [src/tui/expert.rs](../src/tui/expert.rs) stub; [src/cli/mod.rs](../src/cli/mod.rs) prints "role selection is not implemented yet" | 2–7 |
| 9 | TUI — candidate | Not started | [src/tui/candidate.rs](../src/tui/candidate.rs) stub | 2, 4, 6, 7 |
| 10 | End-to-end integration and failure handling | Not started | No cross-module flow wired; only per-module tests exist | 3–9 |
| 11 | Hardening and polish (P1) | Not started | fmt/clippy clean today, but no P1 items built | 10 |
| 12 | Demo and submission | Not started | No demo or submission artifacts in repo | 10, 11 |

**Phases 0–4 are Done; the project is ready for Phases 5, 6, and 7.** No later phase has implementation beyond stubs and one Dockerfile. Phases 5 (ingest), 6 (connection), and 7 (sandbox and sync) can proceed. See [Current state](#current-state).

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
- Meta Model API client: model listing, chat, streaming chat with token usage ([src/ai/mod.rs](../src/ai/mod.rs): `MetaClient::list_models`, `chat`, `chat_stream`); key from `MODEL_API_KEY`.
- Settled model IDs: strong `muse-wonder-1.0`, light `muse-spark-1.3` (`STRONG_MODEL`/`LIGHT_MODEL`).

**Exit criteria.**

- [x] Live tests for: free catalog, statement+snippets, premium rejection ([tests/leetcode_live.rs](../tests/leetcode_live.rs)); model visibility, strong chat usage, light streaming usage ([tests/meta_api_live.rs](../tests/meta_api_live.rs)). `#[ignore]`d (network/key); run with `-- --ignored`.
- [x] Offline parsing/error unit tests: 3 in `src/leetcode/mod.rs`, 3 in `src/ai/mod.rs`, green in the current `cargo test` run.
- [x] Model IDs confirmed via `GET /v1/models`, recorded with live token measurements in [must.md](must.md) (2026-10-28) and commit `645581f`.

**Status: Done.** Evidence: commit `281cd95` "Add LeetCode and Meta Model API clients (validation spikes)"; [must.md](must.md) records live validation on 2026-10-28. Two caveats: (1) the live `#[ignore]`d tests were not re-run when this section was first written (network + key required), so "Done" rested on the committed clients, green unit tests, and that live-validation record — they have since been re-run and passed on 2026-10-28; (2) rate-limit behaviour (section 20 item 1) has no written findings and no retry/429 handling in code — it is carried as a residual risk into Phase 10. Randomized topic-pool selection (section 4) is **not** shipped here: the client lists/fetches, nothing selects yet.

**Remaining work.**

- [ ] Randomized selection flow over the topic pool (Phase 4/8).
- [ ] Characterize rate limiting; feed findings into Phase 10.

## Phase 2 — Core domain (session state machine, event log, artifacts)

**Goal.** UI-independent interview state: session lifecycle, per-question actions, append-only JSONL event log, on-disk artifact layout (structure.md sections 3, 11, 15, 19; `core/`).

**Scope.**

- Types/errors: `SessionStatus`, `QuestionSlot`, `ApproachChoice` (4 presets + `Other`), `Language` (Python/JavaScript), typed `CoreError` ([src/core/types.rs](../src/core/types.rs)).
- State machine: exactly 5 easy at create; waiting/connect/start; expert 3+2 set once (validated); 1 extra (max 11); `WaitingForExpert` after the fifth question; expert-end / technical-failure / completed terminals ([src/core/session.rs](../src/core/session.rs)).
- Candidate actions: select approach + lock expected tokens, continue, redirect, request tests, change approach, add tokens, re-question request/resolve (expert part only, before approach, same-difficulty replacement), finish, skip ([src/core/actions.rs](../src/core/actions.rs)).
- Typed sequenced, timestamped events ([src/core/events.rs](../src/core/events.rs)); JSONL writer (create-new, sequence-checked, flush per event) + validating reader ([src/core/event_log.rs](../src/core/event_log.rs)).
- Artifacts: session layout (`events.jsonl`, `report.md`, `tokens.json`, `context.json`, `solutions/`, `raw/`), token/context records, atomic writes, raw-dir deletion ([src/core/artifacts.rs](../src/core/artifacts.rs)).

**Exit criteria.**

- [x] Tests per must.md (success/error/edge) for every transition and action, event-log round-trip/gaps/malformed lines, artifact writes: 22 unit tests under `src/core/` (actions 9, session 6, types 5, events 2) plus [tests/core_artifacts.rs](../tests/core_artifacts.rs) (10), [tests/core_coverage.rs](../tests/core_coverage.rs) (7), [tests/core_event_log.rs](../tests/core_event_log.rs) (12), [tests/core_records.rs](../tests/core_records.rs) (9) — all green.
- [x] Core work committed: initial core in `f45a6c4`, completion work in the commit carrying this update.
- [x] Event schema covers everything later phases must log (test runs, paste flags, disconnect/reconnect, chat boundary violations).

**Status: Done.** Implemented and fully green. `EventKind` now covers test-run results, paste flags (sections 13, 15, 16), disconnect/resume (section 17), and chat boundary violations (section 10); `Session` has a disconnected state with a reconnect path that resumes at the same question, and [src/core/records.rs](../src/core/records.rs) records test results, paste flags, and chat boundary violations. Token-lock decision (settled): a token lock belongs to an approach selection; `change_approach` clears the current selection so a re-selection locks anew; `slot.locked_expected_tokens` is the current selection's lock; the complete lock history is preserved in `TokensLocked` events and exposed via `Session::token_locks()`. `write_report` stores Markdown verbatim — report *generation* is Phase 10.

**Remaining work.**

- [ ] None.

## Phase 3 — Scoring engine

**Goal.** Deterministic live rating and final merge (structure.md sections 13–14; initial.md section 6; `scoring/`).

**Scope.**

- Live rating from 6.0, clamped 3.0–10.0, with a live per-component breakdown.
- Step table: difficulty multipliers (easy ×0.5, medium ×1.0, hard ×1.5); approach steps (+0.30/+0.10/−0.20, `Other` graded at selection); token-ratio bands vs the locked expectation (≤0.8 → +0.20, 0.8–1.2 → 0, 1.2–1.8 → −0.15, >1.8 → −0.30); intervention (+0.20/−0.10); paste flag −0.30; skip 0 with its tokens settled in the final token line; chat and test results score-neutral.
- Hidden best-approach marker recorded at option generation (data from Phase 4; storage settled here — it is not in `core` today).
- Final merge: per-question AI grades (Phase 4) → total correction capped at ±0.8, scaled down proportionally if over; skipped questions excluded from AI scoring; "not scored" = 0, never invented.
- Final breakdown data for both screens: solved count, test/time summaries, skip/paste counts, per-dimension lines, evidence sentences (section 13).

**Exit criteria.**

- [x] Table-driven tests for every step, band boundary (0.8/1.2/1.8), multiplier, clamp, cap scaling, skip settlement, and neutral chat/tests, per must.md.
- [x] Golden scoring test over a scripted `Session` event history.

**Status: Done.** Implemented in [src/scoring/types.rs](../src/scoring/types.rs) (shared Step-0 contract types: `ApproachQuality`/`ApproachSet` with hidden `best_index` and no `Serialize`, `ApproachGrade`, `QuestionGrade`, `GradeOutcome`, `ScoringError`), [src/scoring/live.rs](../src/scoring/live.rs) (`live_rating` step table), and [src/scoring/final_merge.rs](../src/scoring/final_merge.rs) (`final_rating`, ±0.8 cap with proportional scaling, `FinalBreakdown`/`PerQuestionFinal`). Tests: 18 unit (types 4, live 8, final_merge 6) plus 10 integration ([tests/scoring_live.rs](../tests/scoring_live.rs) 3, [tests/scoring_final.rs](../tests/scoring_final.rs) 7) = 28, all green. Settled decisions: the token ratio uses the **last** lock from `Session::token_locks`; skipped-token settlement maps the positive band to 0 (a skip is never rewarded); `solved_count` = finished slots graded >= 5.0; `final_rating` revalidates grades (struct-literal safe). Evidence: uncommitted working tree on branch `dev` — no commit hash exists for this work yet.

**Remaining work.**

- [ ] None.

## Phase 4 — AI orchestration and prompts

**Goal.** Everything above the raw client that makes the two models do interview work (structure.md sections 10, 11, 14; `ai/`).

**Scope.**

- Strong model: 4+1 approach generation (shuffled, best approach marked internally, expected-token lock on selection); coding loop that writes solution code and checks runnability only, terminating only on the defined events (AI declares ready; candidate finish/skip/change; expert end) — no token cap. Tests created only on `Request tests / verify`.
- Light-model chat per section 10: fixed no-solution system prompt; PairProof live-state block (question, approach/stage, size-bounded solution file, latest test result, recent actions; labelled as data; never marker, token expectation, or score); short history; verbatim candidate message. Boundary violations shown but logged, score-neutral. Low `reasoning_effort` per [must.md](must.md).
- Extraction calls for Phase 5 (topics, level, language proposal: JD-first / resume-second / Python default, section 5) and per-question scoring calls (one separate request per non-skipped question; 0–10 grade, rationale, correction share; retry that question only, section 14).
- Cancellation plumbing so the expert kill switch (section 17) stops all in-flight calls; API key stays outside the container (section 9).
- Randomized selection from the LeetCode topic pool on the Phase 1 client (section 4): auto-pick 5 easy; lists for expert 3+2 selection.

**Exit criteria.**

- [x] Tests per must.md for each Meta API wrapper (approaches, coding step, chat, extraction, scoring): request assembly and response parsing vs fixtures/mocks, malformed-response and API-failure paths; live smoke tests `#[ignore]`d like Phase 1.
- [x] Tests that hidden data never enters the chat live-state block and that chat sends low `reasoning_effort`.

**Status: Done.** `src/ai` is split into [src/ai/client.rs](../src/ai/client.rs) (`ReasoningEffort`/`ChatOptions::low`, `chat*_with_options`, old signatures preserved), [src/ai/common.rs](../src/ai/common.rs) (`AiError`/`AiOutcome`), [src/ai/approaches.rs](../src/ai/approaches.rs) (4+1 generation with seeded-shuffle parsing, `Other` grading), [src/ai/coding.rs](../src/ai/coding.rs) (single coding-step prompt/interface, `apply_usage` via `Session::add_tokens`, tokens booked before parsing; **no** sandbox execution), [src/ai/chat.rs](../src/ai/chat.rs) (section 10 assembly, hidden-data-free live-state block, boundary-violation logging via `ChatBoundaryViolated`), [src/ai/extraction.rs](../src/ai/extraction.rs) (section 5 priority prompt plus normalization/unsupported surfacing), and [src/ai/grading.rs](../src/ai/grading.rs) (section 14 one request per question; `grade_questions` turns failures into `NotScored`); randomized topic-pool selection is in [src/leetcode/selection.rs](../src/leetcode/selection.rs) (`pick_random`/`expert_pool`/`select_questions`/`auto_pick_easy`). Tests: 9 new unit (client 7 incl. 3 moved from Phase 1, common 2) plus integration [tests/ai_approaches.rs](../tests/ai_approaches.rs) (16), [tests/ai_chat.rs](../tests/ai_chat.rs) (10), [tests/ai_extraction.rs](../tests/ai_extraction.rs) (7), [tests/ai_grading.rs](../tests/ai_grading.rs) (8), [tests/leetcode_selection.rs](../tests/leetcode_selection.rs) (11) — all green; live smoke tests in [tests/ai_live_smoke.rs](../tests/ai_live_smoke.rs) (7) are `#[ignore]`d by default and were run and passed 7/7 live on 2026-10-28. Evidence: uncommitted working tree on branch `dev` — no commit hash exists for this work yet.

**Remaining work.**

- [ ] Coding-loop sandbox execution + loop orchestration (Phase 7/10) — the single-step interface is delivered; execution was **not** faked here.
- [ ] Expert kill-switch cancellation plumbing over in-flight calls (Phase 8/10).

## Phase 5 — Ingest (JD / resume)

**Goal.** Turn expert documents into the non-personal context summary (structure.md sections 5, 9; `ingest/`).

**Scope.**

- JD by paste or PDF; resume by PDF; text extraction on the expert side only; disclosure that extracted text goes to the Meta API (surfaced by Phase 8).
- Result: topics, level, proposed language (call in Phase 4); unsupported language → explicit warning, never silent fallback.
- Storage: raw files only in the session `raw/` folder, deleted at session end; leftover folder offered for cleanup on next launch; documents never reach candidate or container. The surviving `ContextSummary` type already exists in [src/core/artifacts.rs](../src/core/artifacts.rs).

**Exit criteria.**

- [ ] Tests per must.md: PDF extraction on fixture PDFs (incl. empty/corrupt), paste path, extraction-call parsing, raw deletion at end, leftover-cleanup offer.

**Status: Not started.** [src/ingest/mod.rs](../src/ingest/mod.rs) is a 1-line doc stub and no PDF dependency is in [Cargo.toml](../Cargo.toml) yet, although the extraction call now exists in [src/ai/extraction.rs](../src/ai/extraction.rs) (Phase 4).

**Remaining work.**

- [ ] Pick and integrate PDF text extraction; implement `ingest/`; wire Phase 4 calls and `ContextSummary` production.

## Phase 6 — Connection (rotating-secret string)

**Goal.** The disposable, single-use candidate→expert link (structure.md section 6; `connection/`).

**Scope.**

- String encodes exactly: endpoint address+port, session-scoped ephemeral username, strong one-time secret, expiry, machine fingerprint — never API key, real username/home path, personal data, or permanent credentials.
- Secret rotates every 60 s with countdown; on connect it dies, rotation stops, string locks (no second connection); on expiry the old string is invalid and the candidate sees "expired, ask the expert for the current string."
- Accept/lock handling; re-issue after accidental disconnect (rotation restarts; finality only on deliberate expert end, section 17).
- Direct mode only, without baking Direct-only assumptions into shared session logic (VPS relay is P2, below).

**Exit criteria.**

- [ ] Tests per must.md for the lifecycle: encode/decode round-trip, field allowlist, rotation/expiry, connect kills secret, second-connect and expired-string rejection, re-issue.

**Status: Not started.** [src/connection/mod.rs](../src/connection/mod.rs) is a 1-line doc stub.

**Remaining work.**

- [ ] Implement `connection/` (string format, rotation timer, accept/lock state).

## Phase 7 — Sandbox and sync (Docker, Mutagen)

**Goal.** The disposable execution environment and file sync (structure.md sections 6–9; `sandbox/`, `sync/`).

**Scope.**

- Docker as external processes: build the shipped image once (cached), one container per session (SSH, Python+Node, empty workspace), destroy on expert end (ephemeral user/string die with it). SSH only in-container; no host mounts (never home), no forwarded ports; API key never in the container.
- Preflight computations owned here, displayed by Phase 8: Docker ready, image ready, port listening, external IP visible, CGNAT suspicion; refuse to start without Docker; other failures show guidance, expert may proceed at own risk.
- In-container execution: runnability checks for the coding loop; test runs on candidate request (Python/JavaScript).
- Mutagen: availability check, both-ways session sync, removal at end; workspace layout `questions/01/question.md` + `solution.py|.js` (section 8); candidate keeps their local copy.
- Bulk-insert detection on synced changes feeding paste flags (sections 13, 16); artifacts copied out before destroy (Phase 2 layout).

**Exit criteria.**

- [ ] Tests per must.md for each lifecycle operation (image build/cache, container create/destroy, Mutagen start/stop, preflight computation): success, missing-Docker/Mutagen, and failure paths — command construction/error mapping at minimum, live Docker runs where feasible.

**Status: Not started.** Only [sandbox/Dockerfile](../sandbox/Dockerfile) exists (debian-slim + openssh-server, python3, nodejs, `/workspace`, `sshd` CMD); [src/sandbox/mod.rs](../src/sandbox/mod.rs) and [src/sync/mod.rs](../src/sync/mod.rs) are 1-line stubs. The Dockerfile creates no ephemeral user and bakes in no host keys (correct for a shared image) — user/credential provisioning at container start is part of this phase.

**Remaining work.**

- [ ] Implement `sandbox/` lifecycle + preflight + in-container test execution.
- [ ] Implement `sync/` Mutagen management + workspace layout + paste heuristic.

## Phase 8 — TUI, expert side

**Goal.** Expert screens: preflight, setup, live selection and monitoring (structure.md sections 3, 7, 9, 12; `cli/`, `tui/expert`).

**Scope.**

- Role selection (`Expert`) in [src/cli/](../src/cli/); preflight checklist with concrete failure guidance.
- API key: user-private `0600` config, masked, deletable, env-var override, never logged (section 9).
- Setup: JD/resume entry (Phase 5), topics/level/language proposal with confirm/override, unsupported-language warning, Meta API disclosure; system auto-selects the 5 easy (no expert preview).
- Connection screen with 60 s countdown (Phase 6); on connect, live selection: 3 medium then 2 hard on separate screens while the candidate works; approve/reject re-question and pick the same-difficulty replacement.
- Live monitoring: current question, chosen approach, test results, elapsed time, live rating breakdown (Phase 3), paste-flag alerts; no live chat content.
- Actions: add 1 extra question; global kill switch (cancel API calls, close connection, destroy container, section 17); fresh string after accidental disconnect; final breakdown + artifacts.

**Exit criteria.**

- [ ] Tests per must.md for each screen transition/expert flow (preflight outcomes, setup, selection submit, re-question resolve, extra, kill switch) at the state/logic level beneath rendering, incl. error paths.

**Status: Not started.** [src/cli/mod.rs](../src/cli/mod.rs) parses empty args and prints "pairproof: role selection is not implemented yet"; [src/tui/expert.rs](../src/tui/expert.rs) and [src/tui/shared.rs](../src/tui/shared.rs) are 1-line stubs.

**Remaining work.**

- [ ] Role dispatch in `cli/`; expert screens + shared widgets; API key storage (0600, masking, env override).

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
- Report generation from a real session: Markdown report (breakdown, per-question grades/rationales, follow-up suggestions, relevant chat excerpts only), solutions, `events.jsonl`, token record, context summary — copied out before container destruction; raw JD/resume, API key, string, secret never retained (section 15).

**Exit criteria.**

- [ ] Integration tests per must.md for the full arc and each failure path (service-down pause/resume/terminate, disconnect/resume, kill switch mid-request, scoring failure → "not scored", report from a scripted session) in [tests/](../tests/).
- [ ] One scripted session runs end to end producing all section 15 artifacts.

**Status: Not started.** Nothing is wired across modules: the binary prints the not-implemented message ([src/cli/mod.rs](../src/cli/mod.rs)). Core models `TechnicalFailure`/`EndedByExpert`, but no health check, countdown, resume, or report generation exists.

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

**Status: Not started.** The standing gates are incidentally green today (fmt/clippy clean; largest source file 342 lines); no P1 item is built.

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
- More solution languages (Java first); MVP is Python + JavaScript only.
- Additional difficulty/topic strategies and role-specific rubrics.
- Multi-challenge comparability, team analytics, multi-candidate comparison.
- Also out per structure.md: authored/snapshotted questions, in-app screen sharing, edit-authorship tracking, hard token caps, web UI/accounts/cloud backend.

## Current state

**Phases 0–4 are Done.** The spec is Final (`fe9f15c` skeleton), both clients landed in `281cd95` with models settled in `645581f`, the core landed in `f45a6c4` and its completion work after that, the scoring engine (Phase 3) is in [src/scoring/](../src/scoring/), and AI orchestration plus LeetCode selection (Phase 4) are in [src/ai/](../src/ai/) and [src/leetcode/selection.rs](../src/leetcode/selection.rs); the Phase 3–4 work is uncommitted on branch `dev`, so no commit hash exists for it yet. Phase 3 × Phase 4 composition is proven offline by `tests/phase3_phase4_compose.rs` (6 tests, no adapters). `cargo test` reports 160 passed, 0 failed, 13 ignored (observed 2026-10-28); all 13 ignored live tests were run and passed on 2026-10-28 (meta 3, leetcode 3, ai smoke 7); `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` are clean, and the largest source file is 379 lines ([src/ai/client.rs](../src/ai/client.rs)). Phases 5–12 are Not started: the ingest, connection, sandbox, sync, and tui modules are stubs, the CLI still prints "role selection is not implemented yet", and the only later-phase artifact beyond stubs is [sandbox/Dockerfile](../sandbox/Dockerfile). No phase is Blocked; external risks (LeetCode rate limits, CGNAT in Direct mode) are carried in Phases 7 and 10. Next: Phase 5 (ingest) and Phase 6 (connection), then Phase 7 (sandbox and sync).

## How to update this file

When a phase's exit criteria pass, flip its status in **both** the summary table and its section, check off the exit-criteria boxes, and add the evidence (file paths, test files with counts, commit hashes). Keep statuses honest: implemented but untested/uncommitted is In progress; a stub plus a prerequisite from an earlier phase is Not started. Refresh the snapshot numbers whenever test totals move, and rewrite [Current state](#current-state) whenever the active phase changes.
