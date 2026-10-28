# PairProof — AI-Native Technical Interviewing Platform

| Field | Value |
| --- | --- |
| Project Name | PairProof (CLI binary: `pairproof`) |
| Event | Meta Global AI Developer Hackathon |
| Status | Accepted |
| Category | AI / Product / Engineering |
| Priority | High |
| Project Stage | Concept Validation → MVP Development |
| Delivery | Rust CLI with a split-screen terminal UI (no web UI for the MVP) |
| Implementation Language | Rust |
| Authoritative Build Spec | [structure.md](structure.md) |

---

## 1. Project Overview

PairProof rethinks how engineering talent should be evaluated in a world where AI coding assistants are an integral part of software development.

Traditional technical interviews assume engineers should solve algorithmic problems alone, under artificial constraints, without the tools they use at work. Modern engineering instead involves working alongside AI: exploring solutions, generating and reviewing code, debugging, evaluating alternatives, and making judgment calls under uncertainty.

PairProof does not replace LeetCode-style questions. It **evolves** them. Live LeetCode questions become the input to a two-sided, AI-native interview experience that observes how a candidate reasons, directs an AI coding agent, verifies its output, responds to failure, and spends a finite resource (tokens) wisely.

> **Core thesis:** As AI changes how software is built, technical interviews should evolve from measuring unaided coding ability toward measuring engineering judgment, AI collaboration, verification discipline, and resource-aware problem solving.

The detailed, final build specification lives in [structure.md](structure.md). This document states the product rationale, assessment model, and demo intent.

## 2. Problem Statement

### Limitations of traditional technical interviews

- **Artificial working conditions:** Candidates solve problems without the tools they would normally use, including AI.
- **Overemphasis on final answers:** A correct solution does not prove strong judgment; a failed first attempt does not prove weak engineering.
- **Limited visibility into reasoning:** Interviewers cannot easily distinguish understanding from memorized patterns. LeetCode preparation is heavily memorization-driven.
- **No evaluation of AI skills:** Traditional formats do not measure how well someone directs, checks, and corrects an AI.
- **Weak verification assessment:** Generating code is easy now; noticing wrong assumptions, subtle bugs, and missing edge cases is the scarce skill.
- **Passive interviewer:** In most formats the interviewer watches silently. Their expertise in picking the right follow-up questions is underused.
- **Unbounded AI cost is invisible:** In real teams, wasteful AI usage costs money. No interview measures token efficiency.

### Why this matters now

The important distinction is no longer whether someone can produce code. It is whether they can use AI to produce correct, maintainable, reliable software, understand the decisions behind it, and do so without burning unlimited tokens. Hiring needs assessment methods that reflect this shift.

## 3. Product Vision

A CLI-based, two-sided interview environment:

- The **expert** (interviewer) sets up the session with a job description and the candidate's resume, is charged on their own Meta Model API key, and actively steers the interview while it runs.
- The **candidate** connects to a disposable sandbox on the expert's machine, works inside a split-screen coding environment, chooses solution approaches, directs an AI coding agent, asks a support chat for thinking help, and is scored on judgment — not on typing code.

The result is an evidence-based, football-player-style live rating the expert watches in real time, followed by a per-question AI evaluation and a human-readable report. The system supports the expert's decision; it never makes an automated hiring decision.

## 4. Core Product Principles

### 4.1 Evolve, do not replace

LeetCode questions, difficulties, and topics stay. PairProof changes everything around them: who selects them, how they are solved, what is observed, and how performance is scored.

### 4.2 AI-native assessment

AI is an intentional, disclosed part of the environment. A strong model writes code; a light model chats. The candidate is evaluated on how they direct and check these models.

### 4.3 Judgment over typing

The AI writes the code, so code authorship is not the signal. The signal is: approach selection, timely intervention when the AI goes wrong, verification requests, skip decisions, and token efficiency.

### 4.4 The expert is an active participant

The expert selects part of the question set live, can replace a question on candidate request, can add one extra question, and can end the session at any time. Interviewers should not sit idle.

### 4.5 Evidence-based, explainable scoring

Every score movement has a named cause. Objective events use fixed steps; AI quality judgement is per-question, evidence-backed, and capped. No single opaque number is produced without its breakdown.

### 4.6 Privacy and safety by construction

Candidate code runs in a disposable Docker sandbox on the expert's machine. The expert's API key, files, and identity never reach the candidate. Raw resumes are deleted when the session ends.

## 5. How It Works

### Setup (expert)

The expert starts `pairproof`, selects `Expert`, passes a preflight checklist (Docker, sandbox image, port, connectivity), enters their Meta Model API key, and provides the job description (paste or PDF) plus the candidate's resume (PDF).

The system extracts topics, level, and a proposed solution language (JD-first, resume-second, Python default; the expert confirms). It then auto-selects 5 easy questions matching the position. No expert preview or editing of this set is needed: they are warm-up questions the AI can solve and the candidate should navigate.

PairProof displays a **connection string** whose secret rotates every 60 seconds, authenticator-style. The expert shares it with the candidate.

### Connection (candidate)

The candidate opens any editor on an empty folder, runs `pairproof`, selects `Candidate`, and pastes the connection string. This connects them, over SSH, into a disposable Docker sandbox on the expert machine. Files are kept in sync with Mutagen: code written by the AI in the sandbox appears in the candidate's local folder, and the candidate may also edit manually.

The first successful connection locks the string. If the candidate never connects in time, the secret simply rotates. If the connection drops accidentally, the expert can issue a fresh string and the candidate resumes where they left off. Only a deliberate expert termination is final: it cancels all API requests and destroys the container.

### Interview (candidate, left and right)

The candidate's screen is split:

- **Left side — the scored arena.** For each question the AI offers 4 solution approaches plus a 5th `Other` option for the candidate's own approach. On selection, the strong model starts coding in the sandbox. An expected token budget for that question and approach is locked at that moment. The candidate can continue, redirect the AI, change approach, request tests, finish the question, or skip. Every action is timestamped.
- **Right side — the unscored coach.** A fast, cheap chat model that knows the live state of the left side (current question, chosen approach, latest code and test results). It gives concepts, hints, and questions — never the full solution. Chat has **zero effect on the score**, so candidates use it without fear.

Meanwhile the expert, in parallel, picks 3 medium and then 2 hard questions from live LeetCode lists. If the candidate finishes the easy set first, they see a waiting screen. In the expert-selected part, the candidate may ask once per question to change it (`Re-question`); the expert approves or rejects and picks the replacement. The expert may also add one extra question. Total: 10 questions, at most 11.

### Score (expert, live; candidate, at the end)

The expert watches a live rating that starts at **6.0**, like a football player's match rating. It moves only on candidate-controlled events: approach quality, intervention quality, token efficiency versus the locked expectation, and confirmed paste flags. Failing tests do not reduce the score — the AI wrote the code. What matters is what the candidate does next. Skip is neutral, except that tokens burned before skipping are settled at the end.

The candidate never sees the score during the interview.

### Close-out

The session ends when the expert ends it or the candidate completes all questions. The strong model then grades **each question in a separate request**, reviewing the question, chosen approach, final code, test results, actions, and token usage. These grades apply a capped correction to the live rating. Both sides see the final breakdown, and the expert receives a Markdown report plus all session artifacts.

If LeetCode or the Meta Model API goes down mid-session, both screens say so, a 60-second retry countdown runs, and the flow either resumes or ends as a technical failure with a clear reschedule message — never as a candidate result.

## 6. Assessment Framework

Four dimensions, each tied to observable events. Exact step values and caps are defined in [structure.md](structure.md) Section 13.

### A. Approach and Engineering Reasoning

Did the candidate pick the approach that fits the problem, recognize a weak option, or write a better one under `Other`? Did they understand constraints and trade-offs before code was written?

### B. Direction and Intervention

When the AI stalls, loops, or heads the wrong way, did the candidate notice and act — redirect, change approach, or cut losses? Or did they watch tokens burn?

### C. Verification and Reliability

Did the candidate ask for tests, probe edge cases, and react to failures? The AI never tests unprompted beyond basic runnability; requesting verification is itself the measured behaviour. A failing test is the AI's fault; ignoring it is the candidate's.

### D. Resource Judgment (Token Efficiency)

Each question locks an expected token budget when the approach is chosen. Finishing well under budget signals a clean approach and crisp direction. Burning multiples of the budget signals poor approach choice or late intervention. Tokens burned before a skip are settled at the end, so early, decisive skips are nearly free and late, wasteful ones are not.

Chat usage is deliberately excluded from all four dimensions. It is a support tool, not evidence.

## 7. MVP Scope

### In scope

- Single Rust binary, expert and candidate modes, split-screen TUI (Ratatui/crossterm/tokio).
- Live LeetCode sourcing via its unofficial GraphQL endpoint (topic and difficulty catalog, statements). No bundled fallback.
- JD/resume ingestion (JD paste or PDF, resume PDF), topic/level/language extraction.
- Direct connection with rotating-secret connection string, disposable Docker sandbox, Mutagen sync.
- Solution languages: Python and JavaScript.
- Fixed question composition 5 easy + 3 medium + 2 hard, plus 1 optional expert extra. Re-question once per question in the expert-selected part.
- Two Meta models: strong (approaches, code, scoring) and light (chat with live left-side context).
- Live rating from 6.0 with fixed steps and difficulty multipliers; per-question AI grading in separate requests; capped AI correction; final Markdown report and artifacts.
- Honesty controls: paste blocked in PairProof inputs, bulk code insertions flagged, candidate score hidden until the end.

### Out of scope

- Authored questions or question snapshots.
- VPS relay/tunnel mode (expert-supplied VPS) — designed to remain possible, not built.
- Screen sharing inside PairProof (the meeting tool handles it).
- Edit authorship tracking, token hard caps, automatic time limits.
- Web UI, accounts, hosted backend, multi-candidate comparison.

## 8. Architecture Summary

Full detail, module contracts, and every settled rule are in [structure.md](structure.md). In brief:

- One Rust crate. `cli/` (startup, role dispatch) and `tui/` (expert/candidate screens) form the thin UI layer.
- `core/` owns interview flow, session lifecycle, event log (JSONL), and artifacts. It never depends on the UI.
- Service modules: `leetcode/` (GraphQL), `ai/` (Meta client and prompts), `connection/` (string, rotating secret), `sandbox/` (Docker), `sync/` (Mutagen), `ingest/` (JD/resume), `scoring/` (step table, token comparison, grade merge).
- Candidate code runs only inside the Docker sandbox. The Meta API key stays on the expert machine, outside the container, in a `0600` config file, masked everywhere.
- Storage is local files only on the expert machine. Raw JD/resume are deleted at session end.

## 9. Evaluation Methodology

- Objective events (approach pick, token ratio bands, interventions, paste flags) use a fixed step table, so the same behaviour scores the same way every time.
- AI judgement is used only where interpretation is needed: approach quality for `Other`, per-question solution quality, and rationales. It is always per-question, always evidence-referenced, and capped at ±0.8 total correction.
- Tests inform humans and the AI grader; they never score directly.
- Missing evidence stays missing: a failed scoring request marks the question "not scored". Nothing is invented.
- The rubric is a hypothesis about job-relevant judgment, not a proven predictor. Reports state their limitations and suggest follow-up topics instead of making a hiring decision.

## 10. Differentiation

- **LeetCode, evolved:** Familiar questions, entirely new selection, solving, and scoring dynamics. Nothing to memorize end-to-end anymore: the expert's live picks and the AI's randomized approach options change every session.
- **Two-sided by design:** The expert selects, replaces, extends, and ends. Interviewer expertise becomes part of the instrument.
- **Judgment is the test:** Code authorship is given to the AI on purpose. What remains — and what is scored — is exactly the skill AI-era teams need.
- **Cost-aware:** Token efficiency against a pre-locked expectation is a first-class, visible signal.
- **Explainable:** A live rating with a named breakdown, plus per-question graded evidence. The expert can defend every number.

## 11. Implementation Priorities

### P0 — Essential for the demo

- [ ] Validation spikes: LeetCode GraphQL from Rust; Meta Model API key type, endpoints, strong/light model IDs.
- [ ] Single-crate skeleton with module boundaries from structure.md; `rustfmt`/`clippy` clean.
- [ ] Expert setup: preflight, API key handling, JD/resume ingestion and extraction.
- [ ] Connection string with 60-second rotating secret; Docker sandbox; Mutagen sync.
- [ ] Candidate split-screen: question flow, 4+1 approaches, defined actions, chat with live state.
- [ ] Expert live selection (3 medium + 2 hard), waiting state, re-question, extra question, kill switch.
- [ ] Live rating engine with fixed step table and locked token expectations.
- [ ] Per-question AI grading (separate requests), final breakdown, Markdown report and artifacts.
- [ ] Failure handling: service-down countdown and technical-failure ending; accidental-disconnect resume.
- [ ] One rehearsed end-to-end demo session.

### P1 — Important improvements

- [ ] Clearer TUI polish: help, error messages, countdown and waiting states.
- [ ] Richer evidence links from each score line to the underlying event.
- [ ] Scoring-consistency runs: same session graded twice, compare.
- [ ] Documented limitations in the report footer.

### P2 — Future extensions

- [ ] VPS relay connection mode for firm-managed setups.
- [ ] More solution languages (Java first).
- [ ] Additional difficulty/topic strategies and role-specific rubrics.
- [ ] Multi-challenge comparability and team analytics.

## 12. Success Criteria

### Functional

- An expert can set up and run a full 10-question session end to end.
- A candidate can connect with a rotating-secret string, work in the sandbox, and finish.
- Live rating moves traceably; final per-question grades and report are produced.

### Product

- The demo makes obvious why the AI era changes interviewing.
- The rating visibly rewards judgment, not typing speed or memorization.
- Every number in the final report can be explained from recorded evidence.

### Technical

- The core flow is reproducible from a clean machine using documented steps.
- Untrusted candidate code never escapes the Docker sandbox.
- Service failures end honestly as technical failures, never as fake results.

## 13. Demo Narrative (3 minutes)

1. Frame the problem: memorized LeetCode answers tell us little; AI-era engineering is judgment.
2. Expert view: enter API key, paste a JD, upload a resume. The system proposes topics and language and auto-picks 5 easy questions.
3. The connection string counts down from 60 seconds. The candidate pastes it and is inside a sandbox on the expert's machine.
4. Candidate view: pick an approach (or write `Other`), watch the AI code, request a test, redirect it when it drifts.
5. Expert view, in parallel: choose 3 medium and 2 hard questions. The live rating ticks up and down with named reasons.
6. The candidate hits a bad question in the hard set and requests a change; the expert approves a replacement.
7. Token overrun on one question: the rating shows exactly where the budget went.
8. Expert ends the session. Per-question grades roll in, separately, each with a rationale.
9. Final screen: rating, breakdown, and a report the expert can defend line by line.

## 14. Risks and Considerations

- **LeetCode dependency:** An unofficial GraphQL endpoint can change or rate-limit. Mitigation: spike first, health checks, honest technical-failure ending. No silent fake data.
- **Assessment validity:** One session does not measure overall competence. Conclusions stay scoped to observed evidence; the product is decision support, never automated screening or hiring decisions.
- **AI grader consistency:** Fixed steps carry the score; AI correction is capped and per-question. Repeated grading runs are a P1 check.
- **Prompt gaming:** Candidates may try to extract solutions via chat. Mitigation: system-prompt boundary, violations logged, chat excluded from score.
- **Privacy:** Resume handling is disclosed, minimized, and deleted at session end. API key never leaves the expert machine.
- **Execution security:** Candidate code is container-confined with no host mounts and no forwarded ports.
- **Scope creep:** One binary, two languages, one connection mode, ten questions. Everything else is P2.

## 15. Final Deliverable

A working Rust CLI prototype of an AI-native technical interview: an expert configures a session from a JD and a resume; a candidate connects into a disposable sandbox and solves live LeetCode questions by directing an AI, not by typing every line; the expert actively steers question selection; and the system produces a live, explainable rating plus per-question graded evidence an interviewer can review and defend.
