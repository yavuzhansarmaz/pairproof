# PairProof

**AI writes the code. You score how the engineer directs it.**

Traditional interviews ban the tools people use at work. PairProof flips that: live LeetCode-style questions, a disposable Docker sandbox, and a candidate scored on approach selection, intervention, verification, and token efficiency — not typing speed.

One Rust binary. Two roles. Expert hosts. Candidate joins.

PairProof was originally built for a Meta hackathon. It has since been generalised to run model-agnostically: Meta remains the default provider, but any OpenAI-compatible provider or local model can be used instead — see [LLM provider selection](#llm-provider-selection-expert).

[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org/)
[![Platforms](https://img.shields.io/badge/platform-Linux%20%7C%20macOS-lightgrey.svg)](#prerequisites)

## Why PairProof

Real engineering is directing tools, not memorizing syntax under a blank editor. PairProof measures that: judgment under AI, not whiteboard theater. Live, explainable scoring. Sandboxed remote sessions. One binary for both sides.

---

## Install (Users)

```sh
curl -fsSL https://raw.githubusercontent.com/yavuzhansarmaz/pairproof/main/install.sh | bash
```

Installs `~/.local/bin/pairproof` (no sudo, SHA-256 verified). Add `~/.local/bin` to your `PATH` if needed:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

[Docker](https://docs.docker.com/get-docker/) (expert) and [Mutagen](https://mutagen.io/documentation/introduction/installation/) (candidate) are installed separately.

---

## Quick start

Run PairProof, then pick a role on the main menu:

```sh
pairproof
```

Press `e` for Expert or `c` for Candidate.

**Expert** (`e`; needs [Docker](https://docs.docker.com/get-docker/) + an LLM provider API key (Meta by default; see provider selection below)):

```sh
export MODEL_API_KEY="<your-key>"   # or save via the TUI / ~/.config/pairproof/api_key
pairproof
```

Press `e`. Paste a job description → confirm the extracted topics/level/language → share the connection string.

**Candidate** (`c`; needs [Mutagen](https://mutagen.io/documentation/introduction/installation/) + `ssh`; same `pairproof` version as the expert). Start in an empty folder — that folder is the workspace:

```sh
mkdir interview && cd interview
pairproof
```

Press `c`. Paste the connection string. Questions sync into that folder as `q1/`, `q2/`, … — local files stay when the session ends.

```sh
pairproof update                 # latest release
pairproof update --check         # check only, no download
pairproof uninstall              # remove binary and saved config
pairproof uninstall --keep-data  # remove binary, keep config
```

> PairProof does **not** load `.env` files. Export `MODEL_API_KEY` in the shell, or use the stored key file.

### LLM provider selection (expert)

PairProof is model-agnostic. Meta is the default provider, but any
OpenAI-compatible provider can be selected with environment variables —
no code changes. There are no native Anthropic/Gemini adapters; those
would need a separate protocol implementation.

Presets: `meta` (default), `openai`, `openrouter`, `groq`, `together`,
`deepseek`, `ollama`, `custom`. Each preset supplies a base URL and
default strong/light models; override models with
`PAIRPROOF_STRONG_MODEL` / `PAIRPROOF_LIGHT_MODEL`, and the base URL with
`PAIRPROOF_BASE_URL`. `PAIRPROOF_API_KEY` + `PAIRPROOF_BASE_URL` override
any preset. Otherwise the provider-specific key env is used
(`OPENAI_API_KEY`, `OPENROUTER_API_KEY`, `GROQ_API_KEY`,
`TOGETHER_API_KEY`, `DEEPSEEK_API_KEY`; Meta uses `MODEL_API_KEY`).
Precedence: preset < provider key env < `PAIRPROOF_*` overrides.
The stored key file (`~/.config/pairproof/api_key`) records which
provider it was saved for and is reused only for that provider; an env
key for the selected provider wins over it. A key saved for one
provider is never silently sent to another. Reports record the
provider and models used (`report.md`, `context.json`, `tokens.json`),
and when a provider returns no token usage it is estimated and marked
as estimated rather than invented silently.

```sh
# Meta (default — unchanged)
export MODEL_API_KEY="<your-key>"
pairproof

# OpenAI
export PAIRPROOF_PROVIDER=openai
export OPENAI_API_KEY="<your-key>"
# optional: export PAIRPROOF_STRONG_MODEL=gpt-4o PAIRPROOF_LIGHT_MODEL=gpt-4o-mini
pairproof

# Ollama (local, no API key required)
export PAIRPROOF_PROVIDER=ollama
# optional: export PAIRPROOF_STRONG_MODEL=llama3.1 PAIRPROOF_LIGHT_MODEL=llama3.2
pairproof

# Custom OpenAI-compatible endpoint (base URL required)
export PAIRPROOF_PROVIDER=custom
export PAIRPROOF_BASE_URL="https://your-gateway.example/v1"
export PAIRPROOF_API_KEY="<your-key>"
pairproof
```

---

## How it works

```
 Expert machine                              Candidate machine
 ──────────────                              ─────────────────
 pairproof, then e                           pairproof, then c
       │                                            │
       ├─ LLM extracts JD topics                    ├─ paste connection string
       ├─ Docker sandbox + NAT endpoint             ├─ SSH fingerprint pin
       ├─ rotating connection string ─────────────► ├─ version handshake
       │                                            ├─ Mutagen ↔ /workspace
       ├─ LeetCode questions + AI approaches        ├─ pick approach / steer AI
       ├─ live rating (hidden from candidate)       ├─ edit qN/solution.*
       └─ grade → report.md → cleanup               └─ final breakdown
```

| | Expert | Candidate |
|---|---|---|
| Runs | Interview host, LLM calls, Docker sandbox | Client only |
| Needs | [Docker](https://docs.docker.com/get-docker/), LLM provider API key (Meta default), network reachability | [Mutagen](https://mutagen.io/documentation/introduction/installation/), SSH tools |
| Sees | Live rating, progress, paste alerts | Questions, approaches, coach (unscored) |
| Ends session | Yes (`e` on Monitor) | Acknowledges final breakdown |

Per question the candidate picks 1 of 4 AI-generated approaches (or Other), then continues, redirects, changes approach, requests tests, finishes, or skips. An unscored coach pane can hint without affecting the score.

Interactive launches check for updates before opening the TUI. Subcommands (`update`, `uninstall`) and non-TTY runs skip that check.

---

## Scoring

Explainable by construction: a fixed step table plus a capped AI correction — not a learned hiring model.

- Live rating starts at **6.0**, clamped to **[3, 10]**
- Moves on candidate-controlled events: approach quality, token efficiency vs expectation, intervention (redirect/change), and related recorded steps
- Chat, test requests, and failing tests do **not** move the live rating (the AI wrote the code)
- At close, a strong model grades each finished question (0–10). AI effect on the final rating is capped at **±0.8**
- Report lands in `sessions/<id>/report.md` with per-line detail and honest provenance notes

Scores are **decision support**, not an automated hiring decision. One session is not proof of competence.

---

## Security model

| Boundary | Trust |
|---|---|
| Expert machine | Trusted — holds the LLM provider key and Docker daemon (root-equivalent; protect it) |
| Candidate machine | Untrusted caller — never needs Docker or an LLM key |
| Sandbox | One container + network per session; 512 MB / 1 CPU / 256 PIDs; dropped caps; no mounts; no privileged mode; allowlisted `python3` / `node` / `mariadb` |

- API key never enters the container or connection string
- Connection secret is an ephemeral SSH password for a per-session `pp-*` user
- Host identity pinned via `ssh-keyscan` + strict host-key checking
- Mutagen uses an isolated daemon and askpass feeder
- Raw JD/notes pastes deleted at session end; artifacts never store keys or secrets
- Stored key at `~/.config/pairproof/api_key` with `0600` / `0700` permissions

Reachability is automatic (public address, NAT-PMP, or UPnP + STUN). If nothing maps (e.g. CGNAT), the session falls back to `127.0.0.1:<port>` for same-machine use or a manual tunnel.

---

## Prerequisites

**Release binaries:** Linux x86_64/aarch64 (musl), macOS x86_64/aarch64. Windows is unsupported.

**Expert:** `pairproof`, [Docker](https://docs.docker.com/get-docker/) (daemon running), LLM provider API key (Meta default; none for local Ollama), network to the provider + LeetCode, reachable endpoint for the candidate.

**Candidate:** same `pairproof` version as the expert, empty working folder, [Mutagen](https://mutagen.io/documentation/introduction/installation/), `ssh` / `ssh-keyscan`, connection string.

Exact version match is required — mismatched binaries refuse to start the session.

---

## Build (Developers)

```sh
cargo build
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings

cargo test -- --ignored   # live suites: network + API key + Docker + Mutagen
```

Docs:

- [Runtime architecture](docs/architecture/runtime.md)
- [Compatibility](docs/architecture/compatibility.md)
- [Testing](docs/development/testing.md)

---

## Status

Early open-source (`0.1.8`) — originally built for a Meta hackathon, now model-agnostic with Meta as the default provider. Core flow (setup → connect → live scoring → report) is implemented and covered by a large automated suite.

**Known limits**

- No relay yet for networks that cannot auto-map (CGNAT fallback needs a manual tunnel)
- Candidate re-question has no expert resolve action yet — skip it for now
- Live score does not yet include counted test pass/fail or bulk-paste signals
- Pause/resume for arbitrary outages is not available yet

---

## License

Apache-2.0. See [LICENSE](LICENSE).
