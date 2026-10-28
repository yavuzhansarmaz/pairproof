//! Phase 3 (scoring) x Phase 4 (LLM orchestration) composition tests, fully offline.
//! LLM parser outputs feed scoring/core inputs directly with no adapter code.

use std::collections::{BTreeMap, BTreeSet};

use pairproof::core::{ApproachChoice, Language, QuestionRef, Session, SessionStatus};
use pairproof::leetcode::{pick_random, CatalogEntry, Difficulty};
use pairproof::llm::approaches::{parse_approach_grade, parse_approach_set};
use pairproof::llm::grading::{
    grade_questions, parse_question_grade, scorable_slots, QuestionScoringInput,
};
use pairproof::llm::MetaClient;
use pairproof::scoring::{
    difficulty_multiplier, final_rating, live_rating, ApproachQuality, ApproachSet, GradeOutcome,
    ScoreComponent,
};
use rand::rngs::StdRng;
use rand::SeedableRng;

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}

fn q(slug: &str, difficulty: Difficulty) -> QuestionRef {
    QuestionRef {
        title_slug: slug.to_string(),
        title: slug.to_string(),
        difficulty,
        acceptance_pct: None,
    }
}

fn started_easy() -> Session {
    let easy = (0..5)
        .map(|i| q(&format!("e{i}"), Difficulty::Easy))
        .collect();
    let mut s = Session::new("s1".into(), Language::Python, easy, 0).expect("session");
    s.mark_waiting_for_candidate(1).expect("waiting");
    s.candidate_connected(2).expect("connected");
    s.start(3).expect("start");
    s
}

// Best locks 1000 tokens, Weak 800, both Workable options 500 (shuffle-safe).
fn approaches_json() -> String {
    concat!(
        r#"{"approaches":["#,
        r#"{"title":"Brute force","summary":"try all pairs","quality":"weak","expected_tokens":800},"#,
        r#"{"title":"Hash map","summary":"one pass lookup","quality":"best","expected_tokens":1000},"#,
        r#"{"title":"Sort first","summary":"sort then scan","quality":"workable","expected_tokens":500},"#,
        r#"{"title":"Two pointers","summary":"converge","quality":"workable","expected_tokens":500}]}"#
    )
    .to_string()
}

fn parsed_set(rng: &mut StdRng) -> ApproachSet {
    parse_approach_set(&approaches_json(), rng).expect("approach set parses")
}

fn line_delta(
    lines: &[pairproof::scoring::ScoreLine],
    slot: usize,
    component: ScoreComponent,
) -> f64 {
    let line = lines
        .iter()
        .find(|l| l.slot_index == slot && l.component == component)
        .expect("score line exists");
    line.delta
}

#[test]
fn difficulty_multiplier_direct_values() {
    close(difficulty_multiplier(Difficulty::Easy), 0.5);
    close(difficulty_multiplier(Difficulty::Medium), 1.0);
    close(difficulty_multiplier(Difficulty::Hard), 1.5);
}

#[test]
fn parsed_approach_set_drives_live_rating() {
    let mut rng = StdRng::seed_from_u64(7);
    let set = parsed_set(&mut rng);
    let best = set.best_index;
    assert_eq!(set.options[best].quality, ApproachQuality::Best);
    let mut s = started_easy();
    s.select_approach(
        ApproachChoice::Preset(best as u8),
        set.options[best].expected_tokens,
        10,
    )
    .expect("select best preset");
    s.add_tokens(700).expect("tokens");
    s.finish_question(11).expect("finish");
    let quality = set.quality_of(best as u8).expect("quality of best");
    let map = BTreeMap::from([(0usize, vec![quality])]);
    let b = live_rating(&s, &map);
    let mult = difficulty_multiplier(Difficulty::Easy);
    close(
        line_delta(&b.lines, 0, ScoreComponent::Approach),
        0.30 * mult,
    );
    close(line_delta(&b.lines, 0, ScoreComponent::Tokens), 0.20 * mult);
    close(b.rating, 6.0 + 0.30 * mult + 0.20 * mult);
    close(b.unclamped, 6.25);
}

#[test]
fn other_grade_feeds_selection_and_scoring() {
    let grade =
        parse_approach_grade(r#"{"quality":"workable","expected_tokens":250,"rationale":"sound"}"#)
            .expect("approach grade parses");
    assert_eq!(grade.quality, ApproachQuality::Workable);
    let text = "sliding window over the array".to_string();
    let mut s = started_easy();
    s.select_approach(ApproachChoice::Other(text), grade.expected_tokens, 10)
        .expect("select other");
    s.add_tokens(250).expect("tokens");
    s.finish_question(11).expect("finish");
    let map = BTreeMap::from([(0usize, vec![grade.quality])]);
    let b = live_rating(&s, &map);
    let mult = difficulty_multiplier(Difficulty::Easy);
    close(
        line_delta(&b.lines, 0, ScoreComponent::Approach),
        grade.quality.step() * mult,
    );
    assert!(b
        .lines
        .iter()
        .all(|l| l.component != ScoreComponent::Tokens));
    close(b.rating, 6.0 + 0.10 * mult);
}

// Slot 0: Best, 700/1000. Slot 1: Workable, 1000/500 plus one redirect. Basis 6.35.
fn script_two_finished() -> (Session, BTreeMap<usize, Vec<ApproachQuality>>) {
    use ApproachQuality::*;
    let mut s = started_easy();
    s.select_approach(ApproachChoice::Preset(0), 1000, 10)
        .expect("select 0");
    s.add_tokens(700).expect("tokens 0");
    s.finish_question(11).expect("finish 0");
    s.select_approach(ApproachChoice::Preset(1), 500, 12)
        .expect("select 1");
    s.add_tokens(1000).expect("tokens 1");
    s.redirect("use a heap", 13).expect("redirect");
    s.finish_question(14).expect("finish 1");
    let map = BTreeMap::from([(0usize, vec![Best]), (1usize, vec![Workable])]);
    (s, map)
}

#[test]
fn parsed_grades_merge_in_final_rating() {
    let (s, map) = script_two_finished();
    let g0 = parse_question_grade(
        r#"{"grade":9,"rationale":"optimal and clean","correction":1.0}"#,
        0,
    )
    .expect("grade 0 parses");
    let g1 = parse_question_grade(
        r#"{"grade":4,"rationale":"correct but wasteful","correction":0.6}"#,
        1,
    )
    .expect("grade 1 parses");
    let grades = [GradeOutcome::Graded(g0), GradeOutcome::Graded(g1)];
    let b = final_rating(&s, &map, &grades).expect("final rating");
    close(b.ai_correction_total, 0.8);
    close(b.per_question[0].correction, 0.5);
    close(b.per_question[1].correction, 0.3);
    close(b.rating, 7.15);
    close(b.unclamped, 7.15);
    assert_eq!(b.per_question[0].grade, Some(9.0));
    assert_eq!(b.per_question[1].grade, Some(4.0));
    assert_eq!(
        b.per_question[0].rationale.as_deref(),
        Some("optimal and clean")
    );
    assert_eq!(
        b.per_question[1].rationale.as_deref(),
        Some("correct but wasteful")
    );
    assert_eq!((b.solved_count, b.finished_count), (1, 2));
    assert!(b.not_scored.is_empty());
}

#[tokio::test]
async fn grade_questions_failure_path_composes_with_final_rating() {
    let (s, map) = script_two_finished();
    let slots = scorable_slots(&s);
    assert_eq!(slots, vec![0, 1]);
    let inputs: Vec<QuestionScoringInput> = slots
        .iter()
        .map(|&i| QuestionScoringInput::from_session(&s, i, "Q?", "code").expect("input"))
        .collect();
    assert_eq!(inputs[0].expected_tokens, Some(1000));
    assert_eq!(inputs[1].actual_tokens, 1000);
    let client = MetaClient::new("http://127.0.0.1:9/v1", "k");
    let outcomes = grade_questions(&client, &inputs).await;
    assert_eq!(
        outcomes,
        vec![
            GradeOutcome::NotScored { slot_index: 0 },
            GradeOutcome::NotScored { slot_index: 1 }
        ]
    );
    let b = final_rating(&s, &map, &outcomes).expect("final with not scored");
    let empty = final_rating(&s, &map, &[]).expect("final with empty grades");
    assert_eq!(b.not_scored, vec![0, 1]);
    close(b.rating, empty.rating);
    close(b.unclamped, empty.unclamped);
    close(b.ai_correction_total, 0.0);
    close(b.rating, 6.35);
}

fn entries(prefix: &str, n: usize, difficulty: &str) -> Vec<CatalogEntry> {
    (0..n)
        .map(|i| CatalogEntry {
            title: format!("{prefix}{i}"),
            title_slug: format!("{prefix}{i}"),
            difficulty: difficulty.to_string(),
            ac_rate: 50.0,
            is_paid_only: false,
            topic_tags: Vec::new(),
        })
        .collect()
}

#[test]
fn golden_full_pipeline_compose() {
    use ApproachQuality::*;
    let mut rng = StdRng::seed_from_u64(99);
    let empty = BTreeSet::new();
    let easy = pick_random(&entries("e", 7, "Easy"), 5, &empty, &mut rng);
    let medium = pick_random(&entries("m", 5, "Medium"), 3, &empty, &mut rng);
    let hard = pick_random(&entries("h", 4, "Hard"), 2, &empty, &mut rng);
    assert_eq!((easy.len(), medium.len(), hard.len()), (5, 3, 2));
    assert!(easy.iter().all(|x| x.difficulty == Difficulty::Easy));
    assert!(medium.iter().all(|x| x.difficulty == Difficulty::Medium));
    assert!(hard.iter().all(|x| x.difficulty == Difficulty::Hard));
    let mut s = Session::new("golden".into(), Language::Python, easy, 0).expect("session");
    s.set_expert_questions(medium, hard, 1).expect("expert set");
    s.mark_waiting_for_candidate(2).expect("waiting");
    s.candidate_connected(3).expect("connected");
    s.start(4).expect("start");
    let wants = [
        Best, Workable, Weak, Workable, Best, Best, Weak, Workable, Best, Workable,
    ];
    let actuals = [700u64, 1000, 800, 250, 1500, 1000, 1600, 400, 700, 750];
    let mut map: BTreeMap<usize, Vec<ApproachQuality>> = BTreeMap::new();
    let mut expected = Vec::new();
    let mut at = 10u64;
    for slot in 0..10usize {
        assert_eq!(s.current_index(), Some(slot));
        let set = parsed_set(&mut rng);
        let preset = set
            .options
            .iter()
            .position(|o| o.quality == wants[slot])
            .expect("quality present");
        let lock = set.options[preset].expected_tokens;
        expected.push(lock);
        s.select_approach(ApproachChoice::Preset(preset as u8), lock, at)
            .expect("select");
        map.insert(slot, vec![set.quality_of(preset as u8).expect("quality")]);
        s.add_tokens(actuals[slot]).expect("tokens");
        at += 1;
        if slot == 1 {
            s.redirect("use a heap", at).expect("redirect");
            at += 1;
        }
        if slot == 3 {
            s.skip_question(at).expect("skip");
        } else {
            s.finish_question(at).expect("finish");
        }
        at += 1;
    }
    assert_eq!(s.status(), SessionStatus::Scoring);
    let slots = scorable_slots(&s);
    assert_eq!(slots, vec![0, 1, 2, 4, 5, 6, 7, 8, 9]);
    for &i in &slots {
        let input = QuestionScoringInput::from_session(&s, i, "Q?", "code").expect("input");
        assert_eq!(input.slot_index, i);
        assert_eq!(input.expected_tokens, Some(expected[i]), "slot {i}");
        assert_eq!(input.actual_tokens, actuals[i], "slot {i}");
    }
    // Basis +0.25+0.10-0.10+0.05(skip approach)+0.075+0.30-0.50+0.30+0.75-0.075=+1.15 over 6.0.
    // Parsed corrections sum +0.50 uncapped, so final is 7.65; slot 2 NotScored, slot 3 skipped.
    let table = [
        (0usize, 9.0, 0.2),
        (1, 4.0, -0.1),
        (4, 6.0, 0.1),
        (5, 8.0, 0.2),
        (6, 3.0, -0.2),
        (7, 7.0, 0.1),
        (8, 9.0, 0.2),
        (9, 5.0, 0.0),
    ];
    let mut grades: Vec<GradeOutcome> = table
        .iter()
        .map(|&(slot, grade, correction)| {
            let json = format!(
                r#"{{"grade":{grade},"rationale":"golden rationale {slot}","correction":{correction}}}"#
            );
            GradeOutcome::Graded(parse_question_grade(&json, slot).expect("grade parses"))
        })
        .collect();
    grades.push(GradeOutcome::NotScored { slot_index: 2 });
    let b = final_rating(&s, &map, &grades).expect("final rating");
    close(b.rating, 7.65);
    close(b.unclamped, 7.65);
    close(b.ai_correction_total, 0.5);
    assert_eq!(
        (b.solved_count, b.finished_count, b.skipped_count),
        (6, 9, 1)
    );
    assert_eq!(b.not_scored, vec![2]);
    assert_eq!(b.per_question[8].grade, Some(9.0));
    assert_eq!(
        b.per_question[8].rationale.as_deref(),
        Some("golden rationale 8")
    );
    assert_eq!(b.per_question[3].expected_tokens, Some(expected[3]));
}
