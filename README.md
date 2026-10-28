# PairProof

AI-native technical interviewing: a Rust CLI that evolves LeetCode-style interviews into a two-sided, judgment-based assessment.

An expert configures a session from a job description and a candidate resume. A candidate connects into a disposable Docker sandbox on the expert machine and solves live LeetCode questions by directing an AI coding agent. The expert selects questions live, watches an explainable rating, and receives per-question graded evidence.

## Documents

- [initial.md](docs/initial.md) — product rationale and demo narrative.
- [structure.md](docs/structure.md) — final, authoritative build specification.
- [must.md](docs/must.md) — mandatory hackathon rules and internal engineering standards.

## Status

Project skeleton only. Role selection and interview flows are not implemented yet.

## Prerequisites

- Rust (stable)
- Docker (expert machine, required for the sandbox)
- Mutagen (file sync)

## Build

```sh
cargo build
cargo test
cargo clippy -- -D warnings
```
