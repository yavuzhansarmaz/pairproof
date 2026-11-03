use std::collections::BTreeMap;

use pairproof::core::{ApproachChoice, ArtifactLayout, Difficulty, Language, Session};
use pairproof::orchestration::report::render_shadow_section;
use pairproof::scoring::shadow::{
    assess_decision, evaluate_set, other_evaluation, Confidence, DecisionStanding, DimensionScores,
    EvalLevel, OptionEvaluation, OptionVerdict, SetVerdict, ShadowReport, ShadowStatus,
};
use pairproof::scoring::{
    final_rating, live_rating, ApproachGrade, ApproachOption, ApproachQuality, ApproachSet,
};

mod common_mod;
use common_mod::qs;

fn opt(title: &str, summary: &str, quality: ApproachQuality) -> ApproachOption {
    ApproachOption {
        title: title.into(),
        summary: summary.into(),
        quality,
        expected_tokens: 100,
    }
}

fn distinct_set() -> ApproachSet {
    ApproachSet::new(vec![
        opt(
            "Hash map",
            "single pass hash lookup for pair",
            ApproachQuality::Best,
        ),
        opt(
            "Sort scan",
            "sort values then linear scan",
            ApproachQuality::Workable,
        ),
        opt(
            "Brute force",
            "compare every pair nested loops",
            ApproachQuality::Weak,
        ),
        opt(
            "Heap",
            "push values into heap and pop smallest",
            ApproachQuality::Workable,
        ),
    ])
    .expect("set")
}

fn eval(index: usize, verdict: OptionVerdict, confidence: Confidence) -> OptionEvaluation {
    OptionEvaluation::new(
        index,
        verdict,
        DimensionScores {
            correctness: EvalLevel::Adequate,
            constraint_fit: EvalLevel::Adequate,
            complexity: EvalLevel::Adequate,
            tradeoff: EvalLevel::Adequate,
            risk: EvalLevel::Adequate,
        },
        format!("rationale {index}"),
        confidence,
    )
    .expect("eval")
}

fn evals(verdicts: [OptionVerdict; 4]) -> Vec<OptionEvaluation> {
    verdicts
        .into_iter()
        .enumerate()
        .map(|(i, v)| eval(i, v, Confidence::High))
        .collect()
}

#[test]
fn tied_strong_options_both_count_as_among_best() {
    let all = evals([
        OptionVerdict::Strong,
        OptionVerdict::Viable,
        OptionVerdict::Weak,
        OptionVerdict::Strong,
    ]);
    for preset in [0u8, 3] {
        let d = assess_decision(&all, ApproachChoice::Preset(preset));
        assert_eq!(d.standing, DecisionStanding::AmongBest, "preset {preset}");
        assert!(d.evidence_complete);
    }
}

#[test]
fn one_tier_below_is_defensible_not_penalized() {
    let all = evals([
        OptionVerdict::Strong,
        OptionVerdict::Viable,
        OptionVerdict::Weak,
        OptionVerdict::Viable,
    ]);
    let d = assess_decision(&all, ApproachChoice::Preset(1));
    assert_eq!(d.standing, DecisionStanding::Defensible);
}

#[test]
fn invalid_pick_is_clearly_poor_only_with_confidence() {
    let all = evals([
        OptionVerdict::Strong,
        OptionVerdict::Viable,
        OptionVerdict::Invalid,
        OptionVerdict::Viable,
    ]);
    assert_eq!(
        assess_decision(&all, ApproachChoice::Preset(2)).standing,
        DecisionStanding::ClearlyPoor
    );
    let mut low = all.clone();
    low[2].confidence = Confidence::Low;
    assert_eq!(
        assess_decision(&low, ApproachChoice::Preset(2)).standing,
        DecisionStanding::InsufficientEvidence
    );
}

#[test]
fn incomplete_evaluation_is_insufficient_evidence() {
    let partial = evals([
        OptionVerdict::Strong,
        OptionVerdict::Viable,
        OptionVerdict::Weak,
        OptionVerdict::Viable,
    ])[..3]
        .to_vec();
    let d = assess_decision(&partial, ApproachChoice::Preset(0));
    assert_eq!(d.standing, DecisionStanding::InsufficientEvidence);
    assert!(!d.evidence_complete);
    let with_unknown = evals([
        OptionVerdict::Strong,
        OptionVerdict::InsufficientEvidence,
        OptionVerdict::Weak,
        OptionVerdict::Viable,
    ]);
    assert_eq!(
        assess_decision(&with_unknown, ApproachChoice::Preset(0)).standing,
        DecisionStanding::InsufficientEvidence
    );
}

#[test]
fn duplicate_and_invalid_sets_are_flagged() {
    let text = "identical wording same approach text";
    let dup_set = ApproachSet::new(vec![
        opt("Same", text, ApproachQuality::Best),
        opt("Same", text, ApproachQuality::Workable),
        opt("Same", text, ApproachQuality::Weak),
        opt("Same", text, ApproachQuality::Workable),
    ])
    .expect("dup set");
    let good = evals([
        OptionVerdict::Strong,
        OptionVerdict::Viable,
        OptionVerdict::Viable,
        OptionVerdict::Viable,
    ]);
    let set_eval = evaluate_set(&dup_set, &good);
    assert_eq!(set_eval.verdict, SetVerdict::Compromised);
    assert_eq!(set_eval.duplicate_pairs.len(), 6);

    let with_invalid = evals([
        OptionVerdict::Strong,
        OptionVerdict::Viable,
        OptionVerdict::Invalid,
        OptionVerdict::Viable,
    ]);
    assert_eq!(
        evaluate_set(&distinct_set(), &with_invalid).verdict,
        SetVerdict::Compromised
    );
    let all_weak = evals([OptionVerdict::Weak; 4]);
    assert_eq!(
        evaluate_set(&distinct_set(), &all_weak).verdict,
        SetVerdict::Unusable
    );
    let useful = evaluate_set(&distinct_set(), &good);
    assert_eq!(useful.verdict, SetVerdict::Useful);
    assert!(useful.duplicate_pairs.is_empty());
}

#[test]
fn other_can_outperform_generated_options() {
    let grade = ApproachGrade::new(ApproachQuality::Best, 120, "optimal custom plan".into())
        .expect("grade");
    let mut all = evals([
        OptionVerdict::Viable,
        OptionVerdict::Viable,
        OptionVerdict::Weak,
        OptionVerdict::Viable,
    ]);
    all.push(other_evaluation(&grade));
    let d = assess_decision(&all, ApproachChoice::Other("custom".into()));
    assert_eq!(d.standing, DecisionStanding::AmongBest);
}

#[test]
fn evaluator_contradicting_best_index_is_not_overruled() {
    // Generator labels index 0 Best in distinct_set; shadow may judge
    // index 3 strongest. Decision follows shadow evidence only.
    let all = evals([
        OptionVerdict::Weak,
        OptionVerdict::Viable,
        OptionVerdict::Weak,
        OptionVerdict::Strong,
    ]);
    assert_eq!(
        assess_decision(&all, ApproachChoice::Preset(3)).standing,
        DecisionStanding::AmongBest
    );
    assert_eq!(
        assess_decision(&all, ApproachChoice::Preset(0)).standing,
        DecisionStanding::LikelySuboptimal
    );
}

#[test]
fn shadow_report_persists_and_stale_detection_works() {
    let set = distinct_set();
    let report = ShadowReport::complete(0, "m".into(), &set, evals([OptionVerdict::Strong; 4]));
    assert!(report.is_current_for(&set));
    let other = distinct_set();
    let mut changed = other.options.clone();
    changed[0].summary = "completely different summary words".into();
    let changed_set = ApproachSet::new(changed).expect("changed");
    assert!(!report.is_current_for(&changed_set));

    let dir = std::env::temp_dir().join(format!("pairproof-shadow-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let layout = ArtifactLayout::create(&dir, "s1", Language::Python).expect("layout");
    layout
        .write_shadow_evaluations(std::slice::from_ref(&report))
        .expect("write shadow");
    let text = std::fs::read_to_string(layout.shadow_path()).expect("read shadow");
    assert!(text.contains("shadow-v1"));
    let back: Vec<ShadowReport> = serde_json::from_str(&text).expect("parse shadow");
    assert_eq!(back, vec![report]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn failed_report_and_render_never_fabricate_scores() {
    let failed = ShadowReport::failed(1, "m".into(), "timeout".into());
    assert!(failed.options.is_empty());
    assert!(matches!(failed.status, ShadowStatus::Failed { .. }));
    let section = render_shadow_section(&[failed]);
    assert!(section.contains("failed: timeout"));
    assert!(section.contains("diagnostic only"));
    assert!(render_shadow_section(&[]).contains("No shadow evaluation was run"));
}

#[test]
fn shadow_does_not_change_live_or_final_rating() {
    let mut s = Session::new(
        "shadow-invariance".into(),
        Language::Python,
        qs("e", 5, Difficulty::Easy),
        0,
    )
    .expect("session");
    s.mark_waiting_for_candidate(1).expect("waiting");
    s.candidate_connected(2).expect("connected");
    s.start(3).expect("start");
    s.select_approach(ApproachChoice::Preset(0), 100, 5)
        .expect("select");
    s.finish_question(10).expect("finish");
    let map = BTreeMap::from([(0, vec![ApproachQuality::Best])]);
    let live_before = live_rating(&s, &map);
    let final_before = final_rating(&s, &map, &[]).expect("final");

    let set = distinct_set();
    let mut report = ShadowReport::complete(
        0,
        "m".into(),
        &set,
        evals([
            OptionVerdict::Weak,
            OptionVerdict::Viable,
            OptionVerdict::Invalid,
            OptionVerdict::Strong,
        ]),
    );
    report.decision = Some(assess_decision(
        &report.options.clone(),
        ApproachChoice::Preset(0),
    ));
    let _section = render_shadow_section(std::slice::from_ref(&report));

    assert_eq!(live_rating(&s, &map), live_before);
    assert_eq!(final_rating(&s, &map, &[]).expect("final"), final_before);
    // Candidate view still hides all hidden/shadow data.
    let views = set.candidate_options();
    assert_eq!(views.len(), 4);
    assert_eq!(views[0].title, "Hash map");
}

#[test]
fn probe_memory_constraint_superiority_is_recognized() {
    // Evaluator judged option 1 strongest under a tight memory limit;
    // picking it is AmongBest even though generator labelled 0 Best.
    let all = evals([
        OptionVerdict::Viable,
        OptionVerdict::Strong,
        OptionVerdict::Weak,
        OptionVerdict::Viable,
    ]);
    assert_eq!(
        assess_decision(&all, ApproachChoice::Preset(1)).standing,
        DecisionStanding::AmongBest
    );
}

#[test]
fn probe_time_constraint_and_maintainability_tradeoffs_are_defensible() {
    // Faster-but-complex option is Strong; simpler maintainable option
    // one tier below must be Defensible, never auto-penalized.
    let all = evals([
        OptionVerdict::Strong,
        OptionVerdict::Viable,
        OptionVerdict::Weak,
        OptionVerdict::Viable,
    ]);
    for preset in [1u8, 3] {
        assert_eq!(
            assess_decision(&all, ApproachChoice::Preset(preset)).standing,
            DecisionStanding::Defensible
        );
    }
}

#[test]
fn probe_attractive_invalid_and_assumption_dependent_options() {
    // Option 2 looks attractive but is Invalid; option 3 is Viable only
    // under an assumption (evaluator still rates it Viable).
    let all = evals([
        OptionVerdict::Viable,
        OptionVerdict::Strong,
        OptionVerdict::Invalid,
        OptionVerdict::Viable,
    ]);
    assert_eq!(
        assess_decision(&all, ApproachChoice::Preset(2)).standing,
        DecisionStanding::ClearlyPoor
    );
    assert_eq!(
        assess_decision(&all, ApproachChoice::Preset(3)).standing,
        DecisionStanding::Defensible
    );
}

#[test]
fn probe_other_superior_and_other_with_incomplete_evidence() {
    let grade =
        ApproachGrade::new(ApproachQuality::Workable, 90, "sound custom".into()).expect("grade");
    let mut all = evals([
        OptionVerdict::Strong,
        OptionVerdict::Viable,
        OptionVerdict::Weak,
        OptionVerdict::Viable,
    ]);
    all.push(other_evaluation(&grade));
    assert_eq!(
        assess_decision(&all, ApproachChoice::Other("custom".into())).standing,
        DecisionStanding::Defensible
    );
    // Missing one generated evaluation -> no judgment at all.
    let short = all[..3].to_vec();
    assert_eq!(
        assess_decision(&short, ApproachChoice::Other("custom".into())).standing,
        DecisionStanding::InsufficientEvidence
    );
}

#[test]
fn probe_identical_wording_options_collapse_set_quality() {
    let text = "use dynamic programming table fill bottom up";
    let set = ApproachSet::new(vec![
        opt("DP", text, ApproachQuality::Best),
        opt("DP alt", text, ApproachQuality::Workable),
        opt("DP v2", text, ApproachQuality::Workable),
        opt("DP v3", text, ApproachQuality::Weak),
    ])
    .expect("set");
    let set_eval = evaluate_set(
        &set,
        &evals([
            OptionVerdict::Strong,
            OptionVerdict::Strong,
            OptionVerdict::Strong,
            OptionVerdict::Strong,
        ]),
    );
    assert!(!set_eval.duplicate_pairs.is_empty());
    assert_eq!(set_eval.verdict, SetVerdict::Compromised);
}

#[test]
fn probe_late_and_duplicate_delivery_are_detectable() {
    let set = distinct_set();
    let report = ShadowReport::complete(0, "m".into(), &set, evals([OptionVerdict::Strong; 4]));
    // Duplicate delivery: same report twice renders twice but scoring
    // inputs are untouched; staleness guards late delivery.
    let section = render_shadow_section(&[report.clone(), report.clone()]);
    assert_eq!(section.matches("diagnostic only").count(), 1);
    let mut changed = set.options.clone();
    changed[1].title = "Renamed approach".into();
    let changed_set = ApproachSet::new(changed).expect("changed set");
    assert!(
        !report.is_current_for(&changed_set),
        "late result must be stale"
    );
}

#[test]
fn probe_timeout_and_cancellation_become_failed_not_poor() {
    for reason in ["timeout after 30s", "cancelled by user"] {
        let report = ShadowReport::failed(0, "m".into(), reason.into());
        assert!(report.options.is_empty());
        assert!(report.decision.is_none());
        assert!(matches!(report.status, ShadowStatus::Failed { .. }));
    }
}

#[test]
fn probe_shadow_never_enters_candidate_report_or_view() {
    let set = distinct_set();
    let report = ShadowReport::complete(0, "m".into(), &set, evals([OptionVerdict::Strong; 4]));
    let views = set.candidate_options();
    let view_text = format!("{views:?}");
    assert!(!view_text.contains("Strong"), "shadow leaked: {view_text}");
    assert!(!view_text.contains("shadow"), "shadow leaked: {view_text}");
    drop(report);
}

#[test]
fn candidate_view_unchanged_by_shadow_types() {
    let set = distinct_set();
    assert_eq!(set.best_index, 0);
    assert_eq!(set.quality_of(0), Some(ApproachQuality::Best));
    for view in set.candidate_options() {
        assert!(!view.title.is_empty() && !view.summary.is_empty());
    }
}
