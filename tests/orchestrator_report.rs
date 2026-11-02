use std::collections::BTreeMap;

use pairproof::core::{Language, QuestionRef};
use pairproof::leetcode::Difficulty;
use pairproof::orchestration::report::{generate_report, ChatExcerpt};
use pairproof::orchestration::Orchestrator;
use pairproof::scoring::{final_rating, GradeOutcome, QuestionGrade};

fn q(slug: &str, d: Difficulty) -> QuestionRef {
    QuestionRef {
        title_slug: slug.into(),
        title: slug.into(),
        difficulty: d,
        acceptance_pct: None,
    }
}
fn qs(p: &str, n: usize, d: Difficulty) -> Vec<QuestionRef> {
    (0..n).map(|i| q(&format!("{p}{i}"), d)).collect()
}
fn graded(slot: usize, grade: f64, correction: f64) -> GradeOutcome {
    GradeOutcome::Graded(
        QuestionGrade::new(slot, grade, format!("rationale {slot}"), correction).expect("g"),
    )
}
fn session() -> (Orchestrator, Vec<GradeOutcome>) {
    let mut o = Orchestrator::create(
        "s1".into(),
        Language::Python,
        qs("e", 5, Difficulty::Easy),
        0,
    )
    .expect("create");
    o.set_expert_questions(
        qs("m", 3, Difficulty::Medium),
        qs("h", 2, Difficulty::Hard),
        1,
    )
    .expect("expert");
    o.mark_waiting_for_candidate(2).expect("waiting");
    o.attach_candidate(3).expect("attach");
    o.start(100).expect("start");
    o.select_approach(pairproof::core::ApproachChoice::Preset(0), 100, 110)
        .expect("select");
    o.add_tokens(150).expect("tokens");
    o.record_test_result(3, 1, 120).expect("tests");
    o.finish_question(200).expect("finish 0");
    o.finish_question(300).expect("finish 1");
    o.skip_question(400).expect("skip 2");
    let grades = vec![
        GradeOutcome::Graded(
            QuestionGrade::new(0, 3.5, "needs work on edge cases".into(), -0.2).expect("g0"),
        ),
        GradeOutcome::NotScored { slot_index: 1 },
    ];
    let _ = graded(9, 1.0, 0.0);
    (o, grades)
}

#[test]
fn report_contains_required_final_breakdown_and_rationales() {
    let (o, grades) = session();
    let b = final_rating(o.session(), &BTreeMap::new(), &grades).expect("final");
    let report = generate_report(o.session(), &b, &grades, &[]);
    let headings = [
        "# PairProof Session Report",
        "## Final breakdown",
        "## Per-question grades",
        "## Follow-up suggestions",
        "## Notes",
    ];
    let mut pos = 0usize;
    for h in headings {
        let found = report[pos..].find(h).map(|i| pos + i);
        assert!(found.is_some(), "missing heading {h} in:\n{report}");
        assert!(report.contains(h));
        pos = found.expect("found");
    }
    assert!(report.contains("Final rating:"));
    assert!(report.contains("grade 3.5/10"));
    assert!(report.contains("needs work on edge cases"));
    assert!(report.contains("not scored"));
    assert!(report.contains("correction +0.000"));
    // Follow-ups name only questions with recorded evidence:
    // slot 0 grade < 5 and token ratio 1.5; slot 1 not scored.
    let follow = report
        .split("## Follow-up suggestions")
        .nth(1)
        .expect("follow section");
    assert!(follow.contains("e0"), "{follow}");
    assert!(follow.contains("e1"), "{follow}");
    assert!(report
        .contains("This report supports a human interviewer; it does not make a hiring decision."));
    assert_eq!(report.matches("hiring decision").count(), 1);
    // Deterministic: same inputs render identically.
    assert_eq!(report, generate_report(o.session(), &b, &grades, &[]));
}

#[test]
fn report_includes_only_permitted_chat_excerpts() {
    let (o, grades) = session();
    let b = final_rating(o.session(), &BTreeMap::new(), &grades).expect("final");
    let permitted = vec![ChatExcerpt {
        slot_index: 0,
        text: "candidate asked about the heap invariant".into(),
    }];
    let report = generate_report(o.session(), &b, &grades, &permitted);
    assert!(report.contains("candidate asked about the heap invariant"));
    // Nothing else is pulled from raw chat: unpermitted text stays out.
    assert!(!report.contains("unpermitted raw chat canary"));
    let bare = generate_report(o.session(), &b, &grades, &[]);
    assert!(!bare.contains("candidate asked about the heap invariant"));
    assert!(bare.contains("No chat excerpts were included."));
}

#[test]
fn report_does_not_invent_unavailable_grades_or_test_counts() {
    let (o, grades) = session();
    let b = final_rating(o.session(), &BTreeMap::new(), &grades).expect("final");
    let report = generate_report(o.session(), &b, &grades, &[]);
    let q1 = report
        .lines()
        .find(|l| l.starts_with("- Q1 e0 ("))
        .expect("q1 line");
    assert!(q1.contains("3 passed, 1 failed"), "{q1}");
    let q2 = report
        .lines()
        .find(|l| l.starts_with("- Q2 e1 ("))
        .expect("q2 line");
    assert!(q2.contains("not scored"), "{q2}");
    assert!(q2.contains("no test summary recorded"), "{q2}");
    assert!(!q2.contains("/10"), "{q2}");
    assert!(!q2.contains("passed"), "{q2}");
    let q3 = report
        .lines()
        .find(|l| l.starts_with("- Q3 e2 ("))
        .expect("q3 line");
    // Skipped: no grade, no invented test counts or time beyond record.
    assert!(!q3.contains("/10"), "{q3}");
    assert!(q3.contains("no test summary recorded"), "{q3}");
}
