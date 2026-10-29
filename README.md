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

- Rust (stable)
- Docker (expert machine, required for the sandbox)
- Mutagen (file sync)
- `MODEL_API_KEY` in `.env` (untracked, never committed) for the Meta Model API

Local/runtime outputs (`.env`, `.pairproof/`, `sessions/`, `artifacts/`, `connection-string*.txt`) are git-ignored.

## Build

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

## License

Apache-2.0. See [LICENSE](LICENSE).
