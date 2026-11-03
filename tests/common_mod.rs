#![allow(dead_code)]

use pairproof::connection::{Endpoint, EphemeralUsername, Fingerprint};
use pairproof::core::{Language, QuestionRef};
use pairproof::leetcode::Difficulty;
use std::path::PathBuf;

pub fn creds() -> (Endpoint, EphemeralUsername) {
    (
        Endpoint::new("203.0.113.10", 2222).expect("endpoint"),
        EphemeralUsername::parse("pp-abcd1234").expect("user"),
    )
}

pub fn endpoint() -> Endpoint {
    Endpoint::new("203.0.113.10", 2222).expect("endpoint")
}

pub fn username() -> EphemeralUsername {
    EphemeralUsername::parse("pp-a1b2c3d4").expect("username")
}

pub fn fingerprint() -> Fingerprint {
    Fingerprint::parse(&"a".repeat(64)).expect("fingerprint")
}

pub fn q(slug: &str, difficulty: Difficulty) -> QuestionRef {
    QuestionRef {
        title_slug: slug.into(),
        title: slug.into(),
        difficulty,
        acceptance_pct: None,
    }
}

/// Single-arg variant for tests that only use easy questions.
pub fn q_easy(slug: &str) -> QuestionRef {
    q(slug, Difficulty::Easy)
}

pub fn qs(prefix: &str, n: usize, difficulty: Difficulty) -> Vec<QuestionRef> {
    (0..n)
        .map(|i| q(&format!("{prefix}{i}"), difficulty))
        .collect()
}

pub fn easy_five() -> Vec<QuestionRef> {
    qs("e", 5, Difficulty::Easy)
}

/// Isolated temp dir for a test. Prefix keeps parallel runs apart.
pub fn tmp(prefix: &str, name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("pairproof-{prefix}-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Python session helper used by ingest/pipeline style tests.
pub fn python_session(id: &str, easy: Vec<QuestionRef>, at_ms: u64) -> pairproof::core::Session {
    pairproof::core::Session::new(id.into(), Language::Python, easy, at_ms).expect("session")
}
