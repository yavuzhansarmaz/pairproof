# PairProof — Project Structure

> Status: **Final**. This document is the authoritative build specification for the MVP.
> `initial.md` describes product rationale. `must.md` records mandatory hackathon rules and internal engineering standards. On conflict about implementation, this document wins.

## 1. Product Shape

- PairProof evolves the LeetCode-style technical interview instead of replacing it. Questions are not authored by PairProof; live LeetCode questions are the input to a better interview experience.
- Two-sided system: **Expert** (runs the interview) and **Candidate**. Both use the same Rust binary; the start screen asks for the role.
- Expert and candidate are already on an online meeting (Google Meet or similar). Screen sharing happens there, never inside PairProof. It is fully out of scope.
- The candidate screen is split: left side is the question flow and engineering decisions, right side is an AI chat limited to the current question and progress. Solution code is also visible and editable in the candidate's own editor.

## 2. Scope

### In scope (MVP)

- One binary with expert and candidate modes.
- Live question sourcing from LeetCode.
- JD/additional-notes-driven topic and language selection.
- Direct candidate-to-expert-machine connection with disposable Docker sandbox and Mutagen sync.
- 10-question flow (5 easy + 3 medium + 2 hard) plus at most 1 expert extra question.
- Two Meta models: strong (left side) and light (chat).
- Live football-style rating for the expert, hidden from the candidate until the end.
- Per-question AI quality scoring at the end and a Markdown report with artifacts on the expert machine.

### Out of scope (MVP)

- Authoring original question content or a bundled question snapshot (fallback or otherwise).
- VPS reverse-tunnel relay (structure must not preclude it; it is not built).
- Screen-share transport or requests inside PairProof.
- Multi-language support beyond Python, JavaScript, and SQL.
- Edit authorship tracking (who changed which line, AI or candidate).
- Hard per-question or per-session token caps.
- Web UI, accounts, cloud backend, database server.

## 3. Roles and End-to-End Flow

### Expert flow

1. Start PairProof, select `Expert`.
2. Preflight checklist (Docker, sandbox image, port, external IP, CGNAT suspicion).
3. Enter Meta Model API key. Charges run on the expert's key.
4. Paste the job description; optionally paste additional notes as a second source.
5. The system extracts topics, level, and a proposed language; the expert confirms or overrides the language.
6. The system auto-selects the first 5 easy questions. The expert does not preview or change them.
7. PairProof shows a connection string with a 60-second rotating secret.
8. Once the candidate connects, the expert selects 3 medium questions, then 2 hard questions, from separate selection screens while the candidate works on the first 5.
9. The expert monitors live progress and rating, can add 1 extra question, and can end the session at any time (global kill switch).
10. At the end the expert sees the final score breakdown and receives the session artifacts.

### Candidate flow

1. Open any editor (e.g. VS Code) on an empty folder; run `pairproof` in a terminal there.
2. Select `Candidate`, paste the connection string.
3. Accept the rules screen (single workspace, no external resources needed, bulk pastes are flagged).
4. Work through questions on the split screen: left side is questions, approach selection, and actions; right side is chat. Solution files appear in the local folder via sync and can be edited manually.
5. If the expert has not submitted the 3+2 selection when the first 5 are done, a waiting screen shows: "Expert is preparing questions."
6. At the end the candidate sees the final score (never before).

### Question set and order

- Fixed composition: **5 easy (system) + 3 medium (expert) + 2 hard (expert) = 10**. There is no "very hard" tier. The expert may add at most 1 extra question of any difficulty, for a maximum of 11.
- Fixed order: 5 easy, then 3 medium, then 2 hard, then the optional extra. The candidate does not choose question order.
- The first 5 require no expert intervention; the expert sees them only live while the candidate solves.

## 4. Question Source (LeetCode)

- Question catalog and statements are fetched **live** from the unofficial GraphQL endpoint used by LeetCode's own web UI. There is no official public API.
- No bundled snapshot fallback in the MVP. This is an accepted risk: if the endpoint changes or rate-limits, the flow breaks. A GraphQL access spike is therefore the first validation task.
- LeetCode has no position filter. The AI derives topics (array, graph, DP, etc.) and level from JD + additional notes; the catalog query uses those topics. Domains are mapped to the closest real LeetCode topic slugs from a fixed known-topic list: LeetCode has no domain topics such as machine-learning. Database (SQL) topics are in scope only for SQL sessions, enforced in code as a coherence rule: SQL sessions draw only from the `database` topic pool, and `database` is dropped from Python/JavaScript sessions. The rule exists because LeetCode snippets are language-exclusive (a database question carries a MySQL snippet and no algorithm-language snippet, and vice versa). Unknown slugs are dropped in code because the endpoint silently ignores them, which would return the unfiltered pool.
- Selection from the relevant topic pool is randomized to reduce memorization.

## 5. Language Selection

- Supported solution languages in the MVP: **Python, JavaScript, and SQL (MySQL dialect; solution files `.sql`, LeetCode snippet slug "mysql")**.
- The AI proposes a language from the JD and additional notes taken together: if either text names a language, that language is used; Python is the default only when neither names one. If the two texts conflict, the language stated as a role requirement is preferred, and the proposal must cite which source(s) decided and any conflict. The expert gives the final confirmation and can override.
- If an unsupported language is required, the expert is explicitly warned and must pick a supported one. There is no silent fallback. MSSQL/T-SQL dialects (`mssql`/`tsql`) stay unsupported in the MVP — only the MySQL dialect is supported.

## 6. Connection (Candidate to Expert Machine)

- MVP connection mode is **Direct**: the candidate connects straight to the expert's machine. It requires a public IP and port forwarding; behind CGNAT it may not work. This is a known limitation, surfaced by the preflight checklist.
- A VPS reverse-tunnel relay is a later option for managed/firm setups where the expert supplies VPS details. It is not implemented in the MVP, but the connection module must not hard-code Direct-only assumptions into shared session logic.
- Sharing unit is a single **connection string**, not a short code. It contains only: endpoint address and port, a session-scoped ephemeral username, a strong one-time secret, expiry, and the machine fingerprint.
- It must never contain: Meta API key, the expert's real username or home path, personal information, or any permanent credential. The string is assumed decodable; security comes from the credentials being temporary and powerless, not from hiding the format.
- The secret **rotates every 60 seconds** (authenticator-style) with a visible countdown on the expert screen. When the candidate connects in time, that secret dies immediately, rotation stops, and the expert moves to question selection. On expiry a new string is issued and the old one is invalid; the candidate sees "expired, ask the expert for the current string."
- The first successful connection locks the string. It cannot open a second connection.
- SSH terminates inside the Docker sandbox only, never on the host OS. Port forwarding and host filesystem access are disabled.

## 7. Sandbox and Sync

- A session is one **disposable Docker container** on the expert machine: SSH server, Python and Node runtimes plus a MySQL runner for SQL sessions, and an empty session workspace.
- Prerequisite: Docker installed. PairProof checks it in preflight and refuses to start without it.
- Preflight checklist (expert screen, before start): Docker ready, sandbox image ready, port listening, external IP visible, CGNAT suspicion. Failures show concrete guidance; the expert may proceed anyway at their own risk.
- Sandbox image: built once on the expert machine from a Dockerfile shipped with PairProof, then cached. The image is identical for everyone and contains no keys or personal data.
- The expert's home directory is never mounted into the container.
- When the expert ends the session, the container is destroyed; the ephemeral user and connection string die permanently with it.
- File sync uses **Mutagen**. The AI writes code in the sandbox; it appears in the candidate's local empty folder. The candidate may also edit files manually in their editor. Authorship of edits is not tracked in the MVP.
- PairProof verifies Mutagen availability and manages the sync for the session lifetime; sync is removed at session end.
- The Meta API key never enters the container. AI calls are made from a layer outside the container.

## 8. Candidate Workspace Layout

- One folder per question: `questions/01/question.md` plus `questions/01/solution.py` (or `solution.js` / `solution.sql`), then `02/`, and so on.
- The candidate edits only `solution.*` files. `question.md` is reference.
- PairProof tracking data (selected approach, token counters, actions) does not live in candidate-editable files; it is kept in PairProof's own records.
- The candidate's local sync folder is not deleted after the session; the candidate keeps their copy of the solutions.

## 9. Expert-Side Privacy and Storage

- Meta API key: stored only on the expert machine in a user-private config file with `0600` permissions. Shown masked; deletable; overridable via environment variable. It is never logged, never put into the connection string, never sent to the candidate or the container.
- Both inputs are pastes: the job description (required) and optional additional notes. Each field is bounded to 50,000 characters, truncated on a character boundary with a visible truncated flag. The bounded text is sent to the Meta API for analysis, and this is disclosed on the setup screen.
- Raw pastes are stored as `jd.txt` and `additional.txt` in the expert's session `raw/` folder for the session duration and are automatically deleted at the end. Only the non-personal extraction remains (topics, level, language, question list). After an abnormal exit, leftover `raw/` folders are offered for cleanup on next launch.
- Documents never reach the candidate or the container.

## 10. AI Models and Chat

- Two models, both via the Meta Model API (exact model IDs and key type are confirmed by the API spike, not assumed here):
  - **Strong model (left side):** 4+1 approach generation, code writing in the sandbox, and end-of-session scoring. Quality is critical; cost is secondary.
  - **Light model (right side):** chat. Latency and cost matter most; instant response beats depth.
- Chat exists for thinking support about the question and current progress, not for handing over the solution.
- Chat request assembly on every message:
  1. Fixed system prompt: no full solution or full code, never reveal the correct approach; give concepts, hints, and Socratic questions.
  2. PairProof-generated live state block: current question, selected approach and stage, current solution file content (size-bounded), latest test result, and recent left-side actions. It never contains hidden data (correct-approach marker, locked token expectation, score). The block is labelled as data, so content inside solution files cannot instruct the model.
  3. The candidate's message verbatim.
  Chat history is kept short (last few turns) to protect latency and cost.
- Chat boundary violations (a leaked full solution or code): the answer is shown, but the event is logged as a boundary violation. It has no score effect.

## 11. Left Side: Per-Question Mechanics

- For each question the strong model offers 4 ready-made solution approaches plus a 5th `Other` option where the candidate writes their own approach.
- On selection, the strong model starts writing code in the sandbox.
- Defined candidate actions (all timestamped in the event log): `Continue`, `Redirect` (short instruction), `Change approach` (back to selection; tokens already spent stay spent), `Request tests / verify`, `Finish question`, `Skip`.
- Question closing: only the candidate finishes a question. At finish, the latest test summary is shown for information; the candidate may finish anyway. Finished questions cannot be reopened; tokens and actions lock at that moment.
- Test model: the AI does not write tests on its own; its default loop writes code and checks syntax/runnability only. Test creation and execution happen when the candidate requests them via `Request tests / verify`. Asking or not asking is itself judgment evidence in the final per-question grade. A failing test never directly reduces the score, because the code is the AI's.
- There is no token cap. The AI loop terminates only on defined events: the AI declares the solution ready, the candidate finishes/skips/changes approach, or the expert ends the session. Infinite-loop prevention is a property of these termination conditions, not of a budget cutoff.

### Re-question (change my question)

- Available only in the expert-selected 3+2 part, never in the first 5.
- The candidate may send one re-question request per question, only when the question has landed and before an approach is chosen.
- On expert approval, the expert picks 1 replacement from the same difficulty category, excluding questions already selected, and the question is swapped.
- On rejection, that question's re-question right is burned; the skip right is unaffected.

### Skip

- Skip is available on every question including the first 5. A skipped question is not replaced.
- Skip itself is neutral: no plus, no minus. Tokens burned up to the skip still enter the final token-efficiency line and affect the final score there. Early skip with near-zero burn therefore costs almost nothing; stalling before skipping costs tokens.

## 12. Expert Live Screen

- The expert sees live: current question, chosen approach, test results, elapsed time, and the live broken-down rating.
- Chat content is not shown live; it is recorded for evaluation context.
- From this screen the expert can add the 1 extra question or end the session.
- The candidate never sees any score until the end, to avoid panic-driven behaviour.

## 13. Scoring

### Live rating (football-player model)

- Starts at **6.0**, clamped to 3.0-10.0. It updates live with a visible breakdown of which component moved it. The breakdown is live, not only final.
- Only the left side affects it. **Chat has zero score effect**, by design, so the candidate can ask freely.
- Solved/quality judgement belongs to the AI, not to tests: the strong model reviews each question against its solution codebase at the end (Section 14).
- Step table. Difficulty multipliers: easy x0.5, medium x1.0, hard x1.5, applied to approach and token lines. Each question also carries its LeetCode acceptance percent; the effective multiplier is the difficulty multiplier x an acceptance factor (>=50% x1.0, 35-49% x1.1, <35% x1.25, unknown x1.0), applied to approach and token lines, including skipped-token settlement.
  - Approach: best +0.30, workable +0.10, weak/wrong -0.20. `Other` is graded on the same scale at selection time. The best-approach marker is recorded internally when options are generated; the candidate never sees it and option order is shuffled.
  - Token ratio (actual / locked expectation): up to 0.8 gives +0.20; 0.8-1.2 gives 0; 1.2-1.8 gives -0.15; above 1.8 gives -0.30.
  - Intervention: a redirect or approach change that leads to a solution gives +0.20; an unnecessary approach change gives -0.10.
  - Paste flag: -0.30 per confirmed bulk paste.
  - Skip: 0. Test result alone: no effect.
- Token expectation: when an approach is selected, the strong model locks an expected token range for that question and approach. It cannot be revised later. Actual usage is compared against it.
- Because skipped-question tokens settle only in the final token line, the live rating is provisional. The final score may differ slightly from the last live value.
- AI final correction: the sum of per-question quality adjustments is capped at **±0.8** total; if it would exceed the cap it is scaled down proportionally. The strong model never moves the live rating arbitrarily.

### Final presentation

- Both sides see the final score at the end: solved count, test summary, time summary, skip and paste-flag counts, per-dimension breakdown, and evidence sentences.

## 14. Per-Question AI Scoring (Final)

- Each question is scored in its **own separate request**. Questions are never bundled into one scoring call.
- One request contains: question statement, chosen approach, that question's solution file, test results, locked expected vs actual tokens, and the action summary for that question.
- Output per question: a 0-10 quality grade, a short rationale, and that question's share of the capped final correction. Requests are independent; a failed request is retried for that question only.
- A skipped question gets no scoring request; its only effect is the final token line already described.

## 15. Session Artifacts (Expert Side)

- Copied out before the container is destroyed, into a per-session folder on the expert machine:
  - Final report (Markdown): score breakdown, per-question AI grades and rationales, follow-up suggestions.
  - Final solution code, one file per question.
  - Event log (JSONL): approach selections, actions, test runs, paste flags, timestamps.
  - Token record: locked expectation vs actual per question.
  - Context summary: extracted topics, level, language, and the selected question list.
- Raw chat is retained for the evidence chain; the report quotes only relevant excerpts.
- Never retained: raw JD/additional-notes pastes (deleted at end), API key, connection string, and secret.

## 16. Copy-Paste and Rules

- PairProof blocks pasting in its own input fields (chat input, `Other` approach box).
- Pasting into solution code happens in the candidate's own editor and cannot be blocked there. Sudden large synced blocks are flagged as probable external paste and shown to the expert live and in the final breakdown. No absolute prevention is promised.
- Before the session the expert states the rules; the candidate also sees a rules/consent screen at session start: work only in this environment, the system is designed to need no external resources, and bulk pastes are flagged.

## 17. Time, Ending, and Reconnection

- No automatic time limit. Elapsed time is recorded; the expert sees it live, and the final report summarises total and per-question time.
- End conditions: the expert ends manually, or the candidate completes all 10 questions (11 with the extra).
- Expert end is a global kill switch: all in-flight API requests for that session are cancelled, no new requests are issued, the connection is closed, and the container is destroyed. An already-dispatched single request cannot be un-billed, but no further spend continues.
- Accidental disconnect is not an ending. The container and full state stay alive on the expert side. The expert may issue a fresh connection string (60-second rotation restarts) and the candidate resumes at the same question, or end everything instead. The no-reconnection rule applies only to a deliberate expert termination. Disconnect events and durations are logged and never reduce the score by themselves.

## 18. Service Failure

- Dependent services: LeetCode GraphQL and the Meta Model API. If either is down, the system technically cannot run an interview; the failure is attributed to neither side.
- On failure the flow pauses. Both screens immediately show which service(s) are unreachable and a 60-second countdown to the next retry.
- During the countdown the services are health-checked. At retry: if a service is back, the flow resumes where it stopped.
- If it is still down, the session is terminated as a technical failure. Both screens state that the services are currently unavailable and that the interview can be rescheduled or retried later. No candidate evaluation is produced from such a session.
- Separately, if a final per-question scoring request fails after retries, that question is marked "not scored", its correction is 0, and the report states this explicitly. No grade is invented.

## 19. Technology and Module Layout

- Single Rust crate, single binary `pairproof`. No workspace in the MVP.
- Stack: Ratatui + crossterm for the split-screen TUI; tokio for concurrent chat streaming, Meta API calls, the 60-second rotation, and live rating updates. Docker and Mutagen are driven as external processes.
- Module layout (one responsibility per module; max 400 lines per file per internal standards):

```text
src/
  main.rs          # entry point only
  tui/             # UI: args, startup, role dispatch (thin); expert/ and candidate/ screen sets, shared widgets/rendering
  core/            # interview flow state, session lifecycle, event log, artifacts
  leetcode/        # GraphQL catalog and question fetching
  llm/             # Meta Model API client (strong/light) and prompts
  connection/      # connection string, rotating secret, accept/lock handling
  sandbox/         # Docker lifecycle and in-container test execution
  sync/            # Mutagen checks and session sync management
  ingest/          # JD/additional-notes paste intake, bounds, and context summary
  scoring/         # fixed step table, token comparison, per-question grade merge
tests/             # integration tests per flow
```

- UI layer = `tui/` only. It calls downward only. `core` and every service module must never depend on `tui/`. Checks such as Docker availability are computed in their owning module; the UI only displays results.

## 20. Validation Spikes (before full build)

1. LeetCode GraphQL: confirm topic/difficulty catalog queries and statement fetching work from Rust, including rate limits and premium-locked questions.
2. Meta Model API: confirm key type, endpoints, streaming, and the exact strong/light model IDs to use.

## 21. Relationship to Other Documents

- This document supersedes the challenge definition, command surface, and single-sided flow sketched in earlier drafts of `initial.md`. `initial.md` now reflects the same design at product level.
- Hackathon compliance (Meta model usage, public repo, no automated hiring-decision positioning) and internal engineering standards (tests, 400-line limit, formatting) are governed by `must.md`.
