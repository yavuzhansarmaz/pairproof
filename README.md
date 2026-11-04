# PairProof

**AI writes the code. You score how the engineer directs it.**

Traditional interviews ban the tools people use at work. PairProof flips that: live LeetCode-style questions, a disposable Docker sandbox, and a candidate scored on approach selection, intervention, verification, and token efficiency — not typing speed.

One Rust binary. Two roles. Expert hosts. Candidate joins.

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

**Expert** (`e`; needs [Docker](https://docs.docker.com/get-docker/) + a Meta Model API key):

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

---

## How it works

```
 Expert machine                              Candidate machine
 ──────────────                              ─────────────────
 pairproof, then e                           pairproof, then c
       │                                            │
       ├─ Meta extracts JD topics                   ├─ paste connection string
       ├─ Docker sandbox + NAT endpoint             ├─ SSH fingerprint pin
       ├─ rotating connection string ─────────────► ├─ version handshake
       │                                            ├─ Mutagen ↔ /workspace
       ├─ LeetCode questions + AI approaches        ├─ pick approach / steer AI
       ├─ live rating (hidden from candidate)       ├─ edit qN/solution.*
       └─ grade → report.md → cleanup               └─ final breakdown
```

| | Expert | Candidate |
|---|---|---|
| Runs | Interview host, Meta calls, Docker sandbox | Client only |
| Needs | [Docker](https://docs.docker.com/get-docker/), Meta API key, network reachability | [Mutagen](https://mutagen.io/documentation/introduction/installation/), SSH tools |
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
| Expert machine | Trusted — holds the Meta key and Docker daemon (root-equivalent; protect it) |
| Candidate machine | Untrusted caller — never needs Docker or a Meta key |
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

**Expert:** `pairproof`, [Docker](https://docs.docker.com/get-docker/) (daemon running), Meta API key, network to Meta + LeetCode, reachable endpoint for the candidate.

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

Early open-source (`0.1.5`) — started as a hackathon project. Core flow (setup → connect → live scoring → report) is implemented and covered by a large automated suite.

**Known limits**

- No relay yet for networks that cannot auto-map (CGNAT fallback needs a manual tunnel)
- Candidate re-question has no expert resolve action yet — skip it for now
- Live score does not yet include counted test pass/fail or bulk-paste signals
- Pause/resume for arbitrary outages is not available yet

---

## License

Apache-2.0. See [LICENSE](LICENSE).
