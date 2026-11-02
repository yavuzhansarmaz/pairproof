# PairProof

## 1. Product introduction

PairProof is an AI-native technical interviewing tool. Traditional interviews ban the tools engineers use every day. PairProof does the opposite: the AI writes the code, and the candidate is scored on how they direct it — approach selection, timely intervention, verification discipline, and token efficiency.

PairProof runs live LeetCode-style questions as a two-sided interview that measures engineering judgment, not typing. It is a single Rust binary (`pairproof`, version `0.1.0`) with two roles: expert and candidate.

## 2. How PairProof works

One binary, two roles. The expert runs the interview from one machine; the candidate joins from another.

- The expert pastes a job description and additional notes. Both are required for extraction: the system extracts topics, level, and a proposed language (Python, JavaScript, or SQL only) only when both are present.
- The system auto-selects 5 easy warm-up questions. When the candidate is working, the system auto-selects 3 medium and 2 hard questions (random selection with topic fallback; the expert does not hand-pick question titles in the TUI).
- Per question, the candidate picks 1 of 4 AI-generated approaches (or `Other`), then continues, redirects, changes approach, requests tests, finishes, or skips. A separate unscored AI coach pane offers hints.
- The expert watches a live rating starting at 6.0. The candidate never sees the live score, approach quality, or expected token counts during the session — only the final breakdown at the end.
- At close, the expert receives a Markdown report with per-question graded evidence.

Startup note: on every interactive (TTY) launch without a subcommand, PairProof checks for an update and may restart itself before opening the TUI. Non-TTY launches and subcommand invocations (`update`, `uninstall`) skip this check.

## 3. Expert workflow

1. Start PairProof as expert (see Section 9). If you pass `--role expert`, preflight runs immediately; otherwise it runs when you enter the Preflight screen.
2. Pass the preflight checklist (Docker, daemon, sandbox image, port, external IP / CGNAT, Mutagen probe). A `Fail` blocks the session; a `Warn` (for example, a missing sandbox image that will be built on first use) does not.
3. Make sure a Meta Model API key is available before the session (see Section 9). All Meta charges run on the expert's key. Note: the TUI ApiKey screen does not capture a new key — the key must already exist in the environment or in the stored key file.
4. Paste the job description first, then paste additional notes. Both are required for topic/level/language extraction; without both, extraction yields no topics and defaults to Python. Blank pastes and pastes over 50,000 characters are rejected.
5. Confirm the extracted setup. The system creates a disposable Docker sandbox and a session.
6. Share the connection string with the candidate. The secret in the string rotates every 60 seconds until the first valid connection locks it.
7. Monitor the live rating, per-question progress, and paste alerts. The expert sees the live rating and breakdown, but not candidate chat content.
8. End the session with `e` on the Monitor screen when ready. Ending triggers grading, report generation, and cleanup (see Section 9).

## 4. Candidate workflow

1. Open an empty folder, run `pairproof`, select `Candidate` (or run `pairproof --role candidate` in a terminal), and paste the connection string from the expert. Input is masked.
2. PairProof verifies the host fingerprint, authenticates over SSH, and checks that the candidate and expert binary versions match exactly. If versions differ, the session does not start.
3. You land over SSH in a disposable Docker container on the expert's machine. Your working folder (`<current directory>/pairproof-workspace` by default) syncs to the container workspace with Mutagen. Local files are preserved when the session ends.
4. Work one question at a time in a split screen: the scored arena on one side (pick 1 of 4 approaches or `Other`, then continue / redirect / change approach / request tests / finish / skip) and an unscored AI coach on the other side. Coach hints never affect the score.
5. A disconnect is not the end of the session. The expert can reissue a fresh connection string (press `R` on the expert side) after a 30-second heartbeat timeout.
6. When the expert ends the session, you see the final breakdown. If the session ends in technical failure, the candidate view shows "No evaluation produced".

Known limitation: requesting a re-question blocks question flow, and there is currently no expert-side resolve action for it. Avoid using re-question unless the interviewer confirms how to proceed. Whether this can deadlock a question is a current limitation (see Section 11).

## 5. Scoring philosophy

Scoring is explainable by construction. It uses a fixed step table with multipliers plus a capped AI correction — it is not a learned or weighted machine-learning model.

- The live rating starts at 6.0 and is clamped to [3, 10]. It moves only on candidate-controlled events: approach quality (only the last approach quality per question counts), token efficiency versus the locked expectation (finished questions only), intervention (a flat bonus once per finished question if the candidate redirected or changed approach), unnecessary changes away from the best approach, and related recorded events. Every live line carries a human-readable detail string (quality, step, multiplier, token counts and ratio, redirect/change counts).
- Chat activity, test requests, and failing tests never move the live rating. The AI wrote the code, so failing tests alone do not lower the candidate's rating.
- At close, the strong model grades each finished question separately (grade 0–10, judged from code against the question statement; skipped questions are never sent for grading). The final rating is the live rating plus AI corrections proportionally scaled so the total AI effect is capped at ±0.8, clamped to [3, 10]. A question counts as solved when its grade is ≥ 5.0.
- The final Markdown report (`sessions/<id>/report.md`) renders the rating, solved/finished/skipped counts, AI correction total, time, per-line scoring detail, and per-question grade, token ratio, time, test outcome, and rationale. The report marks evidence provenance honestly: AI grades and some token totals are marked as not event-recorded. Do not treat AI grades as event-proven.
- The report states its own limits: it is decision support, not a hiring decision; one session is not proof of competence; the rubric is a hypothesis, not a proven predictor.

Not currently scored in the live flow (do not rely on these):

- Counted test pass/fail: requesting tests runs the solution file and shows the raw outcome verbatim, but live pass/fail counts are not derived from it.
- Bulk-paste flags: a paste detector and a scoring rule exist, but no production polling/confirmation path feeds confirmed flags into the live rating. Treat any paste-related scoring as **unverified** in the live flow.
- Extra questions: the data model supports one extra question, but no TUI/orchestration path exposes it. Do not expect an "add extra question" action.

## 6. Security model and trust boundaries

- **Expert machine (trusted):** holds the Meta Model API key, the Docker daemon (root-equivalent — protect this machine accordingly), and makes all LeetCode and Meta HTTPS calls. All Meta charges run on the expert's key. The candidate never provides a Meta key.
- **Candidate machine (untrusted caller):** needs only the `pairproof` binary, an empty folder, Mutagen, and standard SSH tools. It never needs Docker or a Meta key.
- The API key never enters the container or the connection string. The connection-string secret is an ephemeral SSH password for a per-session `pp-` user; it is delivered to the container only via `docker exec` stdin (`chpasswd`) and to Mutagen via an isolated askpass feeder. Host identity is pinned by fingerprint (`ssh-keyscan` verification with strict host-key checking).
- Sandbox isolation: one Docker container and one per-session network per session; resource limits (512 MB memory, 1 CPU, 256 PIDs); dropped capabilities; exactly one published port; no volume mounts and no privileged mode; in-container firewall rules restrict traffic; SSH inside the container disables root login, public-key auth, forwarding, X11, and tunnelling; file paths are validated (relative paths only, 1 MiB per-file cap); code execution is allowlisted to `python3`, `node`, and `mariadb` under a kill timeout.
- The candidate sees only a snapshot view (question titles and summaries, no approach quality, best-approach markers, expected tokens, or grading internals). The expert monitor shows the live rating and breakdown but no candidate chat content.
- Raw job-description/notes pastes are deleted at session end. Final artifacts never carry the API key, connection secret, or raw pastes.
- API keys at rest: the stored key file (`~/.config/pairproof/api_key`) is written with `0600` permissions in a `0700` directory via atomic rename, with symlink refusal. Keys are masked (last 4 characters only) in display and redacted in debug output.

Endpoint reachability — **unverified**: the expert currently publishes the sandbox endpoint as `127.0.0.1:<port>`. How a candidate on a different machine reaches that endpoint (for example, same host or an SSH tunnel/port-forward you arrange yourself) is **unverified** and not set up automatically by PairProof. Confirm reachability before a real interview.

## 7. Prerequisites and supported environment

Supported platforms for release binaries:

- Linux x86_64 (musl)
- Linux aarch64 (musl)
- macOS x86_64
- macOS aarch64

Windows is unsupported.

### Expert machine

- `pairproof` binary (installer or source build).
- Docker with a running daemon (required for the sandbox). The sandbox image is built from an embedded Dockerfile on first use.
- A Meta Model API key (see Section 9).
- Network access to the Meta Model API and LeetCode, and a reachable endpoint for the candidate (see the unverified note in Section 6).

### Candidate machine

- `pairproof` binary (same version as the expert — exact version match required).
- An empty folder to work in.
- Mutagen (file sync).
- `ssh`/`scp` (and `ssh-keyscan`, used for host verification).
- The connection string from the expert.
- No Docker and no Meta API key required.

### Shared notes

- Users do not need Rust or Cargo — see Section 8.
- Minimum Docker and Mutagen versions: **unverified**. No version floor is enforced beyond binary presence; field-tested versions noted in developer docs are not a guaranteed minimum.
- Whether the expert machine also needs Mutagen installed is **unverified**: preflight probes for Mutagen on the expert machine as a warning only, and guidance describes Mutagen as a candidate-machine requirement.

## 8. Installation and build

## Install (Users)

Install the latest stable release with one line — no Rust or Cargo required:

```sh
curl -fsSL https://raw.githubusercontent.com/yavuzhansarmaz/pairproof/main/install.sh | bash
```

The installer downloads the matching release archive, verifies its SHA-256 checksum before installing, and installs the single `pairproof` binary to `~/.local/bin/pairproof`. It requires no `sudo` and does not modify any shell configuration files. The installer installs neither Docker nor Mutagen — install those separately. If `~/.local/bin` is not already in your `PATH`, add it yourself, for example in `~/.bashrc` or `~/.zshrc`:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

Then start a new shell (or run the binary by its full path, `~/.local/bin/pairproof`).

## Build (Developers)

```sh
cargo build
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

Release builds use `cargo build --release --target <target> --locked`.

## 9. Running an interview

### Expert setup

```sh
pairproof
pairproof --role expert
```

With no `--role`, choose Expert on the start screen. Preflight runs automatically for `--role expert` or when you enter the Preflight screen. Follow the Expert workflow in Section 3: paste job description, then notes, confirm setup, and share the current connection string (it rotates every 60 seconds until first connection).

### Candidate connection

```sh
pairproof
pairproof --role candidate
```

With no `--role`, choose Candidate on the start screen (or press `c` in role select). Paste the connection string promptly — expired strings are rejected. See Section 4.

### API keys

PairProof uses the Meta Model API only (`https://api.meta.ai/v1`; model IDs are fixed in the binary and not user-configurable). The expert provides the key in exactly one of these ways:

- Environment variable: `MODEL_API_KEY` set in the process environment, or
- Stored key file: `~/.config/pairproof/api_key`.

The environment variable wins over the stored file when both are set. The candidate never sets a Meta key.

Important: PairProof does **not** read a `.env` file. Putting `MODEL_API_KEY` only in a `.env` file has no effect unless you export/source it into the process environment yourself, for example:

```sh
export MODEL_API_KEY="<your-key>"
pairproof --role expert
```

Never commit a real API key. Keep any local `.env` untracked (it is git-ignored).

### Docker

Docker is required on the expert machine only. Preflight fails hard if Docker, the daemon, or a free sandbox port is unavailable; a missing sandbox image is a warning (it will be built on first use, which can take several minutes). The candidate machine does not run Docker.

### Mutagen

Mutagen is required on the candidate machine for file sync. The expert-side preflight warning about Mutagen refers to the candidate machine — install Mutagen there. PairProof runs Mutagen with an isolated data directory and daemon; it does not touch your global Mutagen daemon. Sync terminates at session end and local candidate files are kept.

### Update and uninstall

```sh
pairproof update
pairproof update --check
pairproof uninstall
pairproof uninstall --yes
pairproof uninstall --keep-data
pairproof uninstall --yes --keep-data
```

- `pairproof update` updates to the latest stable release; `--check` only checks without downloading. Downloads are SHA-256 verified; on checksum mismatch the operation aborts and any existing binary is preserved.
- `pairproof uninstall` asks for confirmation, then removes the binary and `~/.config/pairproof` (including any saved API key). `--yes` skips confirmation. `--keep-data` removes only the binary and keeps `~/.config/pairproof`. Sessions and project files are never removed by uninstall.

### Completion and cleanup

Only the expert ends the session (`e` on the Monitor screen). On end, PairProof grades finished questions, computes the final rating, writes the session bundle first (`sessions/<id>/`: `events.jsonl`, `tokens.json`, `context.json`, `report.md`, `solutions/`), then cancels in-flight work, terminates the connection, stops sync, destroys the sandbox container and network, and deletes raw pastes. Every cleanup step is attempted even if an earlier step fails. The candidate acknowledges the final breakdown when it arrives; if the acknowledgement times out (60 seconds), the expert proceeds and records the timeout. Session folders are kept after uninstall.

## 10. Development and testing

Developer prerequisites: Rust (stable, only to build from source), plus Docker and Mutagen for sandbox/sync features, and a Meta Model API key for live flows (expert side).

```sh
cargo build
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

Live suites (need network, API key, Docker, and Mutagen) are ignored by default:

```sh
cargo test -- --ignored
```

Live end-to-end behaviour (real Docker sandbox + SSH + Mutagen + Meta + LeetCode together) is **unverified** by automated runs in this repository state — the live suites above are manual/ignored. CI runs `cargo test`, `cargo fmt --check`, and `cargo clippy --all-targets -- -D warnings`; the live job is manual-only. Actual CI/release run status is **unverified** from this repository alone.

Developer documentation:

- [Compatibility notes](docs/architecture/compatibility.md)
- [Runtime architecture](docs/architecture/runtime.md)
- [Testing guide](docs/development/testing.md)

## 11. Project status and limitations

PairProof is at version `0.1.0` under active development. Known limitations and unverified items:

- **Unverified:** live end-to-end interviews (Docker + SSH + Mutagen + Meta + LeetCode together) have not been executed as an automated verified run; validate in your own environment before relying on PairProof for hiring decisions.
- **Unverified:** remote-candidate endpoint reachability. The published endpoint is `127.0.0.1:<port>`; any tunnel or port-forward for a candidate on another machine must be arranged and confirmed by you.
- **Unverified:** minimum Docker/Mutagen versions; expert-machine Mutagen requirement; external download-link liveness; public CI/release status.
- Re-question has no expert-side resolve path and blocks question flow (see Section 4). Extra questions and counted live test pass/fail are not exposed in the live flow. Bulk-paste scoring has no production confirmation path (see Section 5).
- Automatic service-failure pause/resume for arbitrary Meta/LeetCode outages is not promised. Recovery handling exists for specific question-loading failures; do not assume every outage pauses the session cleanly — **unverified** beyond the implemented question-load recovery.
- The TUI ApiKey screen does not capture a new key; provision the key via environment or stored file beforehand (see Section 9). The first-run onboarding path beyond that is **unverified**.
- Scoring is decision support only, as stated in the report itself (see Section 5).

## 12. License

Apache-2.0. See [LICENSE](LICENSE).
