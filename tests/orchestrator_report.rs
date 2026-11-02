use std::collections::BTreeMap;

use pairproof::core::{Language, QuestionRef};
use pairproof::leetcode::Difficulty;
use pairproof::orchestration::report::{generate_report, limitations_footer, ChatExcerpt};
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
    assert_eq!(report.matches("hiring decision").count(), 2);
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

#[test]
fn report_includes_required_limitations_footer() {
    let (o, grades) = session();
    let b = final_rating(o.session(), &BTreeMap::new(), &grades).expect("final");
    let report = generate_report(o.session(), &b, &grades, &[]);
    assert!(report.contains("## Limitations"), "{report}");
    assert!(report.ends_with(limitations_footer()), "{report}");
    for s in [
        "PairProof is a decision-support tool for a human interviewer. It does not make automated hiring decisions and must not be used to screen applications automatically.",
        "One session does not measure overall competence. Conclusions are scoped to the evidence recorded in this session.",
        "The scoring rubric is a hypothesis about job-relevant judgment, not a proven predictor of job performance.",
        "Per-question AI grades are capped in their effect on the final rating; the rest of the score comes from fixed, event-based steps recorded in events.jsonl.",
        "A question marked not scored contributed no AI grade and no correction; nothing was invented in its place.",
    ] {
        assert!(report.contains(s), "missing: {s}");
        assert!(limitations_footer().contains(s), "footer missing: {s}");
    }
}

#[test]
fn limitations_footer_matches_project_validity_scope() {
    let footer = limitations_footer();
    assert!(footer.contains("decision-support tool for a human interviewer"));
    assert!(footer.contains("does not make automated hiring decisions"));
    assert!(footer.contains("must not be used to screen applications automatically"));
    assert!(footer.contains("not a proven predictor of job performance"));
    assert!(footer.contains("One session does not measure overall competence"));
    for forbidden in [
        "scientifically validated",
        "bias-free",
        "guarantees",
        "legally compliant",
    ] {
        assert!(
            !footer.to_lowercase().contains(forbidden),
            "forbidden claim {forbidden} in footer"
        );
        let (o, grades) = session();
        let b = final_rating(o.session(), &BTreeMap::new(), &grades).expect("final");
        let report = generate_report(o.session(), &b, &grades, &[]);
        assert!(
            !report.to_lowercase().contains(forbidden),
            "forbidden claim {forbidden} in report"
        );
    }
}

#[test]
fn limitations_footer_preserves_existing_report_sections() {
    let (o, grades) = session();
    let b = final_rating(o.session(), &BTreeMap::new(), &grades).expect("final");
    let report = generate_report(o.session(), &b, &grades, &[]);
    let headings = [
        "# PairProof Session Report",
        "## Final breakdown",
        "## Per-question grades",
        "## Follow-up suggestions",
        "## Notes",
        "## Limitations",
    ];
    let mut pos = 0usize;
    for h in headings {
        let found = report[pos..].find(h).map(|i| pos + i);
        assert!(
            found.is_some(),
            "missing/out-of-order heading {h} in:\n{report}"
        );
        pos = found.expect("found") + h.len();
    }
    assert!(report.contains("Final rating:"));
    assert!(report.contains("Solved:"));
    assert!(report.contains("AI correction total:"));
    // Footer is the last section: no heading after it.
    let tail = report.split("## Limitations").nth(1).expect("limitations");
    assert!(!tail.contains("\n## "), "footer not last:\n{tail}");
    assert!(report.ends_with(limitations_footer()));
}

#[test]
fn limitations_footer_present_with_not_scored_questions() {
    let (o, grades) = session();
    let b = final_rating(o.session(), &BTreeMap::new(), &grades).expect("final");
    // Session fixture includes a NotScored grade (slot 1) and a skipped question.
    assert!(
        b.not_scored.contains(&1)
            || grades
                .iter()
                .any(|g| matches!(g, GradeOutcome::NotScored { slot_index: 1 }))
    );
    let report = generate_report(o.session(), &b, &grades, &[]);
    assert!(report.contains("not scored"), "{report}");
    assert!(report.contains("## Limitations"), "{report}");
    assert!(report.ends_with(limitations_footer()), "{report}");
    assert!(report.contains(
        "A question marked not scored contributed no AI grade and no correction; nothing was invented in its place."
    ));
    // Empty / no-question breakdown also ends with the footer (single exit path).
    let empty = pairproof::scoring::FinalBreakdown {
        rating: 6.0,
        unclamped: 6.0,
        ai_correction_total: 0.0,
        solved_count: 0,
        finished_count: 0,
        skipped_count: 0,
        paste_flag_count: 0,
        total_time_ms: 0,
        lines: Vec::new(),
        per_question: Vec::new(),
        not_scored: Vec::new(),
    };
    let empty_report = generate_report(o.session(), &empty, &[], &[]);
    assert!(empty_report.contains("- No questions recorded."));
    assert!(
        empty_report.ends_with(limitations_footer()),
        "{empty_report}"
    );
}

#[test]
fn limitations_footer_does_not_change_score_values() {
    let (o, grades) = session();
    let b = final_rating(o.session(), &BTreeMap::new(), &grades).expect("final");
    let report = generate_report(o.session(), &b, &grades, &[]);
    let prefix = report.split("## Limitations").next().expect("prefix");
    assert!(
        prefix.contains(&format!(
            "Final rating: {:.2} / 10 (unclamped {:.2})",
            b.rating, b.unclamped
        )),
        "{prefix}"
    );
    assert!(
        prefix.contains(&format!(
            "Solved: {}; finished: {}; skipped: {}",
            b.solved_count, b.finished_count, b.skipped_count
        )),
        "{prefix}"
    );
    assert!(
        prefix.contains(&format!(
            "AI correction total: {:+.3}",
            b.ai_correction_total
        )),
        "{prefix}"
    );
    assert!(
        prefix.contains(&format!("Total time: {} ms", b.total_time_ms)),
        "{prefix}"
    );
    assert!(
        prefix.contains(&format!("Paste flags: {}", b.paste_flag_count)),
        "{prefix}"
    );
    // Deterministic: generating twice yields identical output including footer.
    assert_eq!(report, generate_report(o.session(), &b, &grades, &[]));
    // Footer itself carries no score values that could alter the breakdown.
    assert!(!limitations_footer().contains("Final rating"));
}
