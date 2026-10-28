# MUST — Meta Global AI Developer Hackathon Mandatory Rules

> Source: Meta Global AI Developer Hackathon Participant Guide
> This file contains only mandatory (MUST / MUST NOT) rules. Violations risk disqualification or exclusion from judging.

---

## 1. Project Topic — MUST

- [ ] The project must build toward **one** of the two problem statements:
  - **Build for Your World** — Bring Meta's models into your domain. Solve a problem you know firsthand.
  - **Push the Frontier** — Use Meta's models to tackle challenges in software, robotics, manufacturing, energy, materials, and beyond.
- [ ] Stage 1 judging is pass/fail on theme fit. A project that does not fit a problem statement is eliminated.

## 2. Meta Model Usage — MUST

- [ ] The project **must incorporate Meta's models**. This is a Stage 1 pass/fail criterion: the Meta Model API and/or Meta open-weight models must be reasonably used.
- Available models / tools:
  - Via Meta Model API: Muse Spark series, Muse Wonder, Muse Image, Muse Voice Transcribe, SAM 3.1
  - Open weights: Muse Glimmer, Muse Spark Open
  - Coding tool: Muse Code
- [ ] If Baseten is used, the model name must be `meta-models/muse-spark-open-1026`, **not** the `meta-models/muse-spark-open` name used in the docs.
- The guide imposes **no programming-language or framework restriction**. Rust is not banned. The requirement is Meta model usage, not a specific SDK or language. Cookbook examples in Python/JavaScript are examples, not mandates; the Meta Model API can be called over HTTPS from any language.

## 3. Repository and Open Source — MUST

- [ ] The repository **must be public**.
- [ ] Verify the repository is publicly accessible before submission.
- [ ] You must hold the rights to all code, data, and assets used. Unlicensed content = disqualification.

## 4. Team — MUST

- [ ] Team size is **maximum 4 participants**. Solo participation is allowed.
- [ ] **One submission per team/startup**, not one per person.
- [ ] The team must appoint **one Representative** to enter the submission.
- [ ] **Everyone** who worked on the project must be listed on the submission page.

## 5. New Work and Originality — MUST

- [ ] The presented work must have been built **during the Hackathon** (building period: **Oct 27 – Nov 4**).
- [ ] The demo must highlight **only** the specific features, code, and functionality built during the Hackathon.
- [ ] Judges must be able to clearly identify what was created during the Hackathon. Failure to clearly identify original contributions risks **immediate disqualification** at the judges' sole discretion.
- [ ] Startups may build on existing products or codebases, but what was **newly developed during the Hackathon must be clearly identified**.

## 6. Banned Projects — MUST NOT

The following result in disqualification:

- [ ] Projects that violate legal, ethical, or platform policies.
- [ ] Use of code, data, or assets you do not have the rights to.

Generic "anti-project" examples to avoid (non-exhaustive list):

- AI generating medical advice (AI Mental Health Advisor, skin blemish detector, etc.)
- Basic RAG applications
- Simple Image Analyzers
- Simple "AI for Education" chatbots
- **AI Job Application Screener**
- AI Nutrition Coach
- Personality Analyzers
- Sports analyzers or coaches
- Anything political or involving war

> ⚠️ **Critical note for this project:** This project is a technical interviewing platform. To stay clear of the "AI Job Application Screener" ban, it must **not** be positioned as a tool that screens applications or makes automated hiring decisions. Emphasis: AI-native engineering assessment, process-based evidence, decision support for a human interviewer — not automated screening or decisions.

## 7. Submission — MUST

- [ ] Submit via the Cerebral Valley platform: https://cerebralvalley.ai/e/meta-ai-developer-hackathon/hackathon/submit
- [ ] **Deadline: Nov 4, 9:00 PM EST.** Late submissions will not be considered.
- [ ] A **three-minute demo video is required**. It must highlight the specific features, code, and functionality built during the Hackathon.
- [ ] Verify the video link is accessible. The video is one of the primary judging artifacts, along with the code and the live product.
- [ ] Double-check before submitting: repository is public, video link works, all contributors are listed.

## 8. Schedule — Key Dates (ET)

| Date | Event |
| --- | --- |
| Oct 26, 12:00 PM | Kickoff |
| Oct 27 – Nov 4 | Building period |
| Nov 3 | Last full day for submission questions and technical troubleshooting |
| **Nov 4, 9:00 PM EST** | **Submission deadline** + API credit claim deadline |
| Nov 5–8 | Judging period |
| Nov 9, 12:00 PM | Closing ceremony, Top 10 announced (5 per track), live final-round judging |
| Nov 16 | Community vote winner announced on Discord |

- Daily Discord #office-hours on weekdays, 10:00–11:00 AM ET (Meta team, text only).

## 9. Judging — What to Know

- **Stage 1 (pass/fail):** Theme fit + reasonable use of the Meta Model API and/or Meta open-weight models.
- **Stage 2:** Submissions passing Stage 1 are scored 0–10 against a rubric. *(The rubric table was not included in the guide — confirm details separately.)*
- **Stage 3:** Top 5 per track move to a live final round; final judges watch video demos live and rank them with the same rubric.
- Tracks: **Indie Developer** ($400K total) and **Startups** ($550K total), plus $50K in bonus prizes. Total pool: $1,000,000.

## 10. Credits — Conditions

- [ ] Claim $150 in Meta Model API credits via the form: https://docs.google.com/forms/d/e/1FAIpQLScam78z7tHsNYibiFyJQVFzEgr0coyBTiY568SxxNbsPa3jiw/viewform
- [ ] A **credit card must be on the account** to use credits and make API calls (a temporary pre-authorization charge is released within a few business days).
- [ ] Claim deadline: **Nov 4, 9:00 PM ET**. Each participant must claim **their own** credits individually; credits cannot be claimed on someone else's behalf.
- [ ] Set a "Spend Limit" notification on the Billing page to avoid spending beyond the $150 credit.
- Partner credits are not guaranteed and are distributed at each partner's sole discretion (Baseten $100 / Modal $100 / Fireworks $100 — via application forms, subject to selection).

---

## Project Decisions (Not Hackathon Rules)

These are this project's own choices, recorded here so they are not confused with the mandatory rules above:

- **Project name:** PairProof (CLI binary: `pairproof`). Name signals pairing with AI + producing proof/evidence, not screening. Preliminary availability check (Oct 28): crates.io, GitHub org name, npm, PyPI free; `pairproof.io`/`.dev`/`.ai`/`.app` unregistered per RDAP; `pairproof.com` registered (2025, parked); unrelated uses exist ("Institute Pairproof" partner-analysis app, a small Solana repo). Not a formal trademark search.
- **Delivery:** Rust CLI with a split-screen TUI (Ratatui/crossterm/tokio), no web UI for the MVP. Two-sided: Expert and Candidate modes in one binary.
- **Implementation language:** Rust — chosen for low memory footprint, fast startup, and single-binary distribution with no runtime dependency.
- **Fallback:** Python only if Rust proves infeasible during the hackathon. (Solution languages inside the product are Python, JavaScript, and SQL; this fallback concerns the implementation language only.)
- **Meta integration:** Meta Model API called over HTTPS from the Rust binary (`https://api.meta.ai/v1`, key env: `MODEL_API_KEY`). Two models, chosen from the Section 2 available list and validated live on 2026-10-28: **strong = `muse-wonder-1.0`** (Muse Wonder; approach generation, code writing, scoring) and **light = `muse-spark-1.3`** (Muse Spark series; chat), both confirmed visible to the project API key via `GET /v1/models` and exercised in live tests (defaults in `src/llm/mod.rs`). Contributor variants are intentionally not used. Chat (light model) calls must run with a low `reasoning_effort`: observed live on 2026-10-28, a trivial "OK" reply cost `muse-spark-1.3` 161 tokens (138 reasoning) versus `muse-wonder-1.0` at 32, so default reasoning would inflate chat latency, cost, and token-efficiency scoring. Baseten, Modal, and Fireworks are not used in the MVP; the Baseten model-name rule above stays on record only if that ever changes.
- **Question source:** Live LeetCode questions via the unofficial GraphQL endpoint used by LeetCode's web UI. No authored questions and no bundled snapshot in the MVP.
- **Session model:** The candidate connects directly into a disposable Docker sandbox on the expert's machine (Mutagen sync). A VPS relay mode is a future option, not MVP.
- **Authoritative spec:** `structure.md` (Final). `initial.md` states product rationale; on implementation conflicts, `structure.md` wins.

---

## Project Engineering Standards — Internal MUST

These are binding internal rules for this codebase, not hackathon rules.

### Tests

- [ ] Every endpoint must ship with detailed, maintained tests in the repo's test layout. "Endpoint" means every user-facing flow and screen transition (expert setup and preflight, candidate connect, per-question actions, chat, expert live selection/monitoring, session end and scoring, report generation), every external integration wrapper (each Meta Model API call for the strong and light models, and each LeetCode GraphQL query), and every lifecycle operation (connection-string rotation and locking, Docker sandbox create/destroy, Mutagen sync start/stop).
- [ ] Tests must cover the success path, error/failure paths, and relevant edge cases (invalid input, missing session, API/network failure, timeout). Happy-path-only coverage is not enough.
- [ ] No endpoint/command is considered done until its tests pass. Do not weaken or delete a failing test to get green; fix the code.

### Comments and Documentation

- [ ] No unnecessary comment lines. No commented-out code, no comments that restate what the code already says.
- [ ] Function documentation is **at most 2 lines**. Prefer self-descriptive names over explanations.
- [ ] Comment only non-obvious intent, constraints, or trade-offs that the code cannot express.

### Modularity and File Size

- [ ] The codebase is modular: one responsibility per module.
- [ ] **No file may exceed 400 lines.** If a file approaches or exceeds 400 lines, split it into smaller modules before merging.
- [ ] UI and functionality are separate modules:
  - UI layer (`tui/` only): argument parsing, startup/role dispatch, expert and candidate screens, terminal rendering, prompts, formatting. Thin layer only.
  - Core/functionality layer (`core/`, `leetcode/`, `llm/`, `connection/`, `sandbox/`, `sync/`, `ingest/`, `scoring/`): interview flow, session lifecycle, event log, question sourcing, Meta API clients, connection and sandbox lifecycles, document ingestion, scoring, report logic.
  - The core layer must not depend on the UI layer (`tui/`).

### Language

- [ ] The codebase language is **English**: identifiers, comments, documentation, CLI text, and error messages.

### Clean Code

- [ ] Meaningful names; small functions that do one thing.
- [ ] No dead code, no duplicated logic that should be shared, no premature abstraction.
- [ ] Explicit, typed error handling; no silent failures.
- [ ] `rustfmt` formatting and `clippy` with no warnings are required before merging.

---

## Pre-Submission Compliance Checklist

- [ ] Build follows `structure.md` (Final); spikes validated first (LeetCode GraphQL access from Rust; Meta Model API key type, endpoints, strong/light model IDs)
- [ ] Candidate code runs only inside the disposable Docker sandbox; the Meta API key never leaves the expert machine; raw JD/additional-notes pastes are deleted at session end
- [ ] Problem statement selected and clearly stated (Build for Your World / Push the Frontier)
- [ ] Meta model genuinely integrated and visibly used in the demo (from the Rust CLI, via Meta Model API over HTTPS)
- [ ] Repository is public
- [ ] Work built during the Hackathon clearly identified in README/demo
- [ ] Not in a banned / anti-project category (in particular, no "job application screener" positioning)
- [ ] 3-minute demo video ready, link accessible
- [ ] All contributors listed on the submission, Representative appointed
- [ ] Submission sent via the CV platform **before** Nov 4, 9:00 PM EST
