use crate::core::session::Session;
use crate::core::types::{Language, QuestionRef};
use crate::leetcode::Difficulty;

pub fn q(slug: &str, difficulty: Difficulty) -> QuestionRef {
    QuestionRef {
        title_slug: slug.to_string(),
        title: slug.to_string(),
        difficulty,
    }
}

pub fn qs(prefix: &str, n: usize, d: Difficulty) -> Vec<QuestionRef> {
    (0..n).map(|i| q(&format!("{prefix}{i}"), d)).collect()
}

pub fn new_session() -> Session {
    Session::new(
        "s1".into(),
        Language::Python,
        qs("e", 5, Difficulty::Easy),
        0,
    )
    .expect("session")
}

pub fn started() -> Session {
    let mut s = new_session();
    s.mark_waiting_for_candidate(1).expect("waiting");
    s.candidate_connected(2).expect("connected");
    s.start(3).expect("start");
    s
}

pub fn with_expert() -> Session {
    let mut s = new_session();
    s.set_expert_questions(
        qs("m", 3, Difficulty::Medium),
        qs("h", 2, Difficulty::Hard),
        1,
    )
    .expect("expert set");
    s.mark_waiting_for_candidate(2).expect("waiting");
    s.candidate_connected(3).expect("connected");
    s.start(4).expect("start");
    s
}

pub fn at_expert() -> Session {
    let mut s = with_expert();
    for i in 0..5u64 {
        s.finish_question(10 + i).expect("finish easy");
    }
    s
}
