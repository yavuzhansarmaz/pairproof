use std::collections::BTreeMap;

use pairproof::core::{ApproachChoice, Language, QuestionRef, Session};
use pairproof::leetcode::Difficulty;
use pairproof::scoring::{
    live_rating, ApproachQuality, RatingBreakdown, ScoreComponent, ScoreLine,
};
use ApproachQuality::*;
use ScoreComponent::*;

fn q(slug: &str, difficulty: Difficulty) -> QuestionRef {
    QuestionRef {
        title_slug: slug.to_string(),
        title: slug.to_string(),
        difficulty,
        acceptance_pct: None,
    }
}

fn qs(prefix: &str, n: usize, d: Difficulty) -> Vec<QuestionRef> {
    (0..n).map(|i| q(&format!("{prefix}{i}"), d)).collect()
}

fn q_acceptance(slug: &str, difficulty: Difficulty, acceptance_pct: Option<u32>) -> QuestionRef {
    let mut question = q(slug, difficulty);
    question.acceptance_pct = acceptance_pct;
    question
}

fn started_easy_acceptance(acceptance_pct: Option<u32>) -> Session {
    let mut questions = qs("e", 5, Difficulty::Easy);
    questions[0] = q_acceptance("e0", Difficulty::Easy, acceptance_pct);
    let mut s = Session::new("s1".into(), Language::Python, questions, 0).expect("session");
    s.mark_waiting_for_candidate(1).expect("waiting");
    s.candidate_connected(2).expect("connected");
    s.start(3).expect("start");
    s
}

fn started_easy() -> Session {
    let mut s = Session::new(
        "s1".into(),
        Language::Python,
        qs("e", 5, Difficulty::Easy),
        0,
    )
    .expect("session");
    s.mark_waiting_for_candidate(1).expect("waiting");
    s.candidate_connected(2).expect("connected");
    s.start(3).expect("start");
    s
}

fn started_expert() -> Session {
    let mut s = Session::new(
        "s1".into(),
        Language::Python,
        qs("e", 5, Difficulty::Easy),
        0,
    )
    .expect("session");
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

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}

fn find_line(b: &RatingBreakdown, component: ScoreComponent) -> &ScoreLine {
    b.lines
        .iter()
        .find(|l| l.slot_index == 0 && l.component == component)
        .expect("slot 0 line")
}

fn triples(b: &RatingBreakdown) -> Vec<(usize, ScoreComponent, f64)> {
    b.lines
        .iter()
        .map(|l: &ScoreLine| (l.slot_index, l.component, l.delta))
        .collect()
}

#[test]
fn approach_steps_per_quality_and_multiplier() {
    let targets = [(0usize, 0.5f64), (5, 1.0), (8, 1.5)];
    let qualities = [(Best, 0.30f64), (Workable, 0.10), (Weak, -0.20)];
    for (target, mult) in targets {
        for (quality, step) in qualities {
            let mut s = started_expert();
            for i in 0..target {
                s.finish_question(10 + i as u64).expect("finish prior");
            }
            assert_eq!(s.current_index(), Some(target));
            let map = BTreeMap::from([(target, vec![quality])]);
            let b = live_rating(&s, &map);
            assert_eq!(triples(&b), vec![(target, Approach, step * mult)]);
            close(b.rating, 6.0 + step * mult);
            close(b.unclamped, 6.0 + step * mult);
        }
    }
}

#[test]
fn rating_clamps_at_bounds() {
    let mut s = started_expert();
    let mut map = BTreeMap::new();
    for i in 0..10usize {
        let at = 10 + (i as u64) * 10;
        s.select_approach(ApproachChoice::Preset(0), 1000, at)
            .unwrap();
        s.add_tokens(10).unwrap();
        s.redirect("tighten the loop", at + 1).unwrap();
        s.finish_question(at + 2).unwrap();
        map.insert(i, vec![Best]);
    }
    let b = live_rating(&s, &map);
    close(b.unclamped, 12.25);
    close(b.rating, 10.0);
    let mut s = started_easy();
    for i in 0..20u64 {
        s.record_paste_flag(100, 10 + i).unwrap();
    }
    let b = live_rating(&s, &BTreeMap::new());
    close(b.unclamped, 0.0);
    close(b.rating, 3.0);
}

#[test]
fn golden_scripted_session_live_rating() {
    let mut s = started_expert();
    s.select_approach(ApproachChoice::Preset(0), 1000, 10)
        .unwrap();
    s.add_tokens(700).unwrap();
    s.finish_question(11).unwrap();
    s.select_approach(ApproachChoice::Preset(1), 500, 12)
        .unwrap();
    s.add_tokens(1000).unwrap();
    s.redirect("use a heap", 13).unwrap();
    s.finish_question(14).unwrap();
    s.select_approach(ApproachChoice::Preset(2), 400, 15)
        .unwrap();
    s.add_tokens(100).unwrap();
    s.change_approach(16).unwrap();
    s.select_approach(ApproachChoice::Preset(0), 600, 17)
        .unwrap();
    s.add_tokens(200).unwrap();
    s.record_paste_flag(800, 18).unwrap();
    s.finish_question(19).unwrap();
    s.skip_question(20).unwrap();
    s.select_approach(ApproachChoice::Preset(3), 800, 21)
        .unwrap();
    s.add_tokens(800).unwrap();
    s.record_test_result(2, 1, 22).unwrap();
    s.record_chat_boundary_violation(23).unwrap();
    s.finish_question(24).unwrap();
    s.select_approach(ApproachChoice::Preset(0), 1000, 25)
        .unwrap();
    s.add_tokens(2000).unwrap();
    s.redirect("mind the edge case", 26).unwrap();
    let map = BTreeMap::from([
        (0, vec![Best]),
        (1, vec![Workable]),
        (2, vec![Best, Weak]),
        (4, vec![Workable]),
        (5, vec![Best]),
        (6, vec![Best]),
    ]);
    let b = live_rating(&s, &map);
    close(b.rating, 6.50);
    close(b.unclamped, 6.50);
    let expected = vec![
        (0, Approach, 0.15),
        (0, Tokens, 0.10),
        (1, Approach, 0.05),
        (1, Tokens, -0.15),
        (1, Intervention, 0.20),
        (2, Approach, -0.10),
        (2, Tokens, 0.10),
        (2, Intervention, 0.20),
        (2, UnnecessaryChange, -0.10),
        (2, Paste, -0.30),
        (4, Approach, 0.05),
        (5, Approach, 0.30),
    ];
    assert_eq!(triples(&b), expected);
    let token_line = b
        .lines
        .iter()
        .find(|l| l.slot_index == 0 && l.component == Tokens)
        .expect("slot 0 token line");
    assert!(
        token_line.detail.contains("700/1000"),
        "{}",
        token_line.detail
    );
    assert!(token_line.detail.contains("0.700"), "{}", token_line.detail);
}

#[test]
fn acceptance_scales_approach_and_token_lines() {
    let run = |acceptance_pct: Option<u32>| {
        let mut s = started_easy_acceptance(acceptance_pct);
        s.select_approach(ApproachChoice::Preset(0), 1000, 10)
            .unwrap();
        s.add_tokens(700).unwrap();
        s.finish_question(11).unwrap();
        live_rating(&s, &BTreeMap::from([(0, vec![Best])]))
    };
    let base = run(None);
    let scaled = run(Some(30));
    close(find_line(&base, Approach).delta, 0.30 * 0.5);
    close(find_line(&scaled, Approach).delta, 0.30 * 0.5 * 1.25);
    close(find_line(&base, Tokens).delta, 0.20 * 0.5);
    close(find_line(&scaled, Tokens).delta, 0.20 * 0.5 * 1.25);
    assert!(
        find_line(&scaled, Approach).detail.contains("ac 30%"),
        "{}",
        find_line(&scaled, Approach).detail
    );
    assert!(
        find_line(&scaled, Tokens).detail.contains("ac 30%"),
        "{}",
        find_line(&scaled, Tokens).detail
    );
    assert!(!find_line(&base, Approach).detail.contains(" ac "));
    assert!(!find_line(&base, Tokens).detail.contains(" ac "));
}
