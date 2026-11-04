//! Deterministic live rating from session state and approach qualities.

use std::collections::BTreeMap;

use crate::core::types::Difficulty;
use crate::core::{EventKind, QuestionRef, QuestionStatus, Session};
use crate::scoring::types::ApproachQuality;

/// One component of a rating breakdown line, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ScoreComponent {
    Approach,
    Tokens,
    Intervention,
    UnnecessaryChange,
    Paste,
}

/// One explained rating movement for a single question slot.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoreLine {
    pub slot_index: usize,
    pub component: ScoreComponent,
    pub delta: f64,
    pub detail: String,
}

/// Live rating with its unclamped value and per-slot breakdown.
#[derive(Debug, Clone, PartialEq)]
pub struct RatingBreakdown {
    pub rating: f64,
    pub unclamped: f64,
    pub lines: Vec<ScoreLine>,
}

/// Returns the approach/token multiplier for a difficulty.
pub fn difficulty_multiplier(difficulty: Difficulty) -> f64 {
    match difficulty {
        Difficulty::Easy => 0.5,
        Difficulty::Medium => 1.0,
        Difficulty::Hard => 1.5,
    }
}

/// Lower acceptance means a harder real-world question; returns its factor.
pub fn acceptance_factor(acceptance_pct: Option<u32>) -> f64 {
    match acceptance_pct {
        Some(pct) if pct < 35 => 1.25,
        Some(pct) if pct < 50 => 1.1,
        _ => 1.0,
    }
}

/// Returns the effective multiplier and acceptance detail suffix for a question.
pub(crate) fn multiplier_and_detail(question: &QuestionRef) -> (f64, String) {
    let mult =
        difficulty_multiplier(question.difficulty) * acceptance_factor(question.acceptance_pct);
    let ac = question
        .acceptance_pct
        .map_or(String::new(), |pct| format!(" ac {pct}%"));
    (mult, ac)
}

/// Returns the unmultiplied token step for an actual/locked ratio.
pub fn token_band_step(ratio: f64) -> f64 {
    if ratio <= 0.8 {
        0.20
    } else if ratio <= 1.2 {
        0.0
    } else if ratio <= 1.8 {
        -0.15
    } else {
        -0.30
    }
}

/// Positive points are kept at this fraction. A full clear of the
/// standard 10 questions on the best approach is about +4.25 before
/// this scale, which used to sit on the cap of 10. At 0.75 that clear
/// lands near 9.2. Penalties are not scaled, so a weak session still
/// drops by the full amount. The displayed rating stays inside 3..=10.
pub const POSITIVE_GAIN_SCALE: f64 = 0.75;

/// Turns positive and negative point totals into `(rating, unclamped)`.
/// Only the positive total is scaled. `negative` is zero or below.
pub fn rate_from_parts(positive: f64, negative: f64) -> (f64, f64) {
    let unclamped = 6.0 + positive.max(0.0) * POSITIVE_GAIN_SCALE + negative.min(0.0);
    (unclamped.clamp(3.0, 10.0), unclamped)
}

fn split_deltas(deltas: impl Iterator<Item = f64>, ai_correction: f64) -> (f64, f64) {
    let (mut positive, mut negative) = (0.0, 0.0);
    for delta in deltas {
        if delta > 0.0 {
            positive += delta;
        } else {
            negative += delta;
        }
    }
    if ai_correction > 0.0 {
        positive += ai_correction;
    } else {
        negative += ai_correction;
    }
    (positive, negative)
}

fn line(slot_index: usize, component: ScoreComponent, delta: f64, detail: String) -> ScoreLine {
    ScoreLine {
        slot_index,
        component,
        delta,
        detail,
    }
}

/// Computes the provisional live rating.
pub fn live_rating(
    session: &Session,
    approach_qualities: &BTreeMap<usize, Vec<ApproachQuality>>,
) -> RatingBreakdown {
    let mut lines: Vec<ScoreLine> = Vec::new();
    for slot in session.slots() {
        let index = slot.index;
        let (mult, ac) = multiplier_and_detail(&slot.question);
        let empty: &[ApproachQuality] = &[];
        let qualities = approach_qualities
            .get(&index)
            .map(Vec::as_slice)
            .unwrap_or(empty);
        // Navigation parks a worked question as Pending. Its selected
        // approach still belongs to the session score. An unopened
        // question has no start time, so it adds nothing.
        let approach_counts =
            slot.status != QuestionStatus::Pending || slot.started_at_ms.is_some();
        if approach_counts {
            if let Some(quality) = qualities.last() {
                let step = quality.step();
                lines.push(line(
                    index,
                    ScoreComponent::Approach,
                    step * mult,
                    format!("approach {quality:?} step {step:+.2} x{mult:.2}{ac}"),
                ));
            }
        }
        if slot.status == QuestionStatus::Finished {
            if let Some(lock) = session.token_locks(index).last().copied() {
                let ratio = slot.actual_tokens as f64 / lock as f64;
                let step = token_band_step(ratio);
                if step != 0.0 {
                    lines.push(line(
                        index,
                        ScoreComponent::Tokens,
                        step * mult,
                        format!(
                            "tokens {}/{} ratio {ratio:.3} step {step:+.2} x{mult:.2}{ac}",
                            slot.actual_tokens, lock
                        ),
                    ));
                }
            }
        }
        let mut redirects = 0usize;
        let mut changes = 0usize;
        let mut abandoned_best: Vec<usize> = Vec::new();
        for event in session.events() {
            match event.kind {
                EventKind::Redirected { index: i, .. } if i == index => redirects += 1,
                EventKind::ApproachChanged { index: i } if i == index => {
                    if qualities.get(changes) == Some(&ApproachQuality::Best) {
                        abandoned_best.push(changes);
                    }
                    changes += 1;
                }
                _ => {}
            }
        }
        if slot.status == QuestionStatus::Finished && (redirects > 0 || changes > 0) {
            lines.push(line(
                index,
                ScoreComponent::Intervention,
                0.20,
                format!(
                    "interventions {} redirects {redirects} changes {changes}",
                    redirects + changes
                ),
            ));
        }
        for k in abandoned_best {
            lines.push(line(
                index,
                ScoreComponent::UnnecessaryChange,
                -0.10,
                format!("change {} abandoned Best selection {k}", k + 1),
            ));
        }
        if slot.paste_flags > 0 {
            let count = slot.paste_flags;
            lines.push(line(
                index,
                ScoreComponent::Paste,
                -0.30 * f64::from(count),
                format!("paste flags {count} x-0.30"),
            ));
        }
    }
    let (positive, negative) = split_deltas(lines.iter().map(|l| l.delta), 0.0);
    let (rating, unclamped) = rate_from_parts(positive, negative);
    RatingBreakdown {
        rating,
        unclamped,
        lines,
    }
}

#[cfg(test)]
mod tests {
    use super::ScoreComponent::*;
    use super::*;
    use crate::core::test_support::*;
    use crate::core::types::ApproachChoice;
    use ApproachQuality::*;

    fn close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }

    fn find(b: &RatingBreakdown, slot: usize, c: ScoreComponent) -> Vec<&ScoreLine> {
        b.lines
            .iter()
            .filter(|l| l.slot_index == slot && l.component == c)
            .collect()
    }

    #[test]
    fn navigating_away_keeps_the_selected_approach_in_the_live_rating() {
        let mut s = at_expert();
        s.select_approach(ApproachChoice::Preset(0), 100, 20)
            .unwrap();
        let map = BTreeMap::from([(5, vec![Workable])]);
        close(
            live_rating(&s, &map).rating,
            6.0 + 0.10 * POSITIVE_GAIN_SCALE,
        );
        s.navigate_to(6, 21).unwrap();
        assert_eq!(
            s.slot(5).expect("left behind").status,
            crate::core::types::QuestionStatus::Pending
        );
        let after = live_rating(&s, &map);
        close(after.rating, 6.0 + 0.10 * POSITIVE_GAIN_SCALE);
        assert_eq!(find(&after, 5, Approach).len(), 1);
        let unopened = BTreeMap::from([(7, vec![Workable])]);
        close(live_rating(&at_expert(), &unopened).rating, 6.0);
    }

    #[test]
    fn rating_starts_at_six_with_no_lines() {
        let b = live_rating(&started(), &BTreeMap::new());
        close(b.rating, 6.0);
        close(b.unclamped, 6.0);
        assert!(b.lines.is_empty());
    }

    #[test]
    fn token_bands_at_exact_boundaries() {
        close(token_band_step(0.0), 0.20);
        close(token_band_step(0.8), 0.20);
        close(token_band_step(0.800_001), 0.0);
        close(token_band_step(1.2), 0.0);
        close(token_band_step(1.200_001), -0.15);
        close(token_band_step(1.8), -0.15);
        close(token_band_step(1.800_001), -0.30);
        close(token_band_step(9.5), -0.30);
    }

    #[test]
    fn acceptance_factor_bands_at_boundaries() {
        close(acceptance_factor(None), 1.0);
        close(acceptance_factor(Some(100)), 1.0);
        close(acceptance_factor(Some(50)), 1.0);
        close(acceptance_factor(Some(49)), 1.1);
        close(acceptance_factor(Some(35)), 1.1);
        close(acceptance_factor(Some(34)), 1.25);
        close(acceptance_factor(Some(0)), 1.25);
    }

    #[test]
    fn token_ratio_uses_last_lock_after_change_approach() {
        let mut s = started();
        s.select_approach(ApproachChoice::Preset(0), 100, 5)
            .unwrap();
        s.add_tokens(150).unwrap();
        s.change_approach(6).unwrap();
        s.select_approach(ApproachChoice::Preset(1), 200, 7)
            .unwrap();
        s.add_tokens(150).unwrap();
        s.finish_question(8).unwrap();
        let b = live_rating(&s, &BTreeMap::new());
        let lines = find(&b, 0, Tokens);
        assert_eq!(lines.len(), 1);
        close(lines[0].delta, -0.15 * 0.5);
        assert!(lines[0].detail.contains("300/200"), "{}", lines[0].detail);
        assert!(lines[0].detail.contains("1.500"), "{}", lines[0].detail);
    }

    #[test]
    fn live_token_line_only_for_finished() {
        let mut s = started();
        s.select_approach(ApproachChoice::Preset(0), 100, 5)
            .unwrap();
        s.add_tokens(300).unwrap();
        assert!(find(&live_rating(&s, &BTreeMap::new()), 0, Tokens).is_empty());
        s.finish_question(6).unwrap();
        let b = live_rating(&s, &BTreeMap::new());
        let lines = find(&b, 0, Tokens);
        assert_eq!(lines.len(), 1);
        close(lines[0].delta, -0.30 * 0.5);
    }

    #[test]
    fn intervention_once_per_finished_slot() {
        let mut s = started();
        s.select_approach(ApproachChoice::Preset(0), 100, 5)
            .unwrap();
        s.redirect("use a heap", 6).unwrap();
        s.redirect("handle empty input", 7).unwrap();
        s.change_approach(8).unwrap();
        s.select_approach(ApproachChoice::Preset(1), 100, 9)
            .unwrap();
        s.finish_question(10).unwrap();
        s.redirect("not finished yet", 11).unwrap();
        let b = live_rating(&s, &BTreeMap::new());
        let lines = find(&b, 0, Intervention);
        assert_eq!(lines.len(), 1);
        close(lines[0].delta, 0.20);
        assert!(find(&b, 1, Intervention).is_empty());
    }

    #[test]
    fn unnecessary_change_only_when_abandoning_best() {
        let mut s = started();
        s.select_approach(ApproachChoice::Preset(0), 100, 5)
            .unwrap();
        s.change_approach(6).unwrap();
        s.select_approach(ApproachChoice::Preset(1), 100, 7)
            .unwrap();
        s.change_approach(8).unwrap();
        s.select_approach(ApproachChoice::Preset(2), 100, 9)
            .unwrap();
        s.finish_question(10).unwrap();
        let map = BTreeMap::from([(0, vec![Best, Workable, Weak])]);
        let b = live_rating(&s, &map);
        let lines = find(&b, 0, UnnecessaryChange);
        assert_eq!(lines.len(), 1);
        close(lines[0].delta, -0.10);
        let map = BTreeMap::from([(0, vec![Workable, Best, Weak])]);
        assert_eq!(find(&live_rating(&s, &map), 0, UnnecessaryChange).len(), 1);
        let map = BTreeMap::from([(0, vec![Best, Best, Weak])]);
        assert_eq!(find(&live_rating(&s, &map), 0, UnnecessaryChange).len(), 2);
        assert!(find(&live_rating(&s, &BTreeMap::new()), 0, UnnecessaryChange).is_empty());
        let short = BTreeMap::from([(0, vec![Workable])]);
        assert!(find(&live_rating(&s, &short), 0, UnnecessaryChange).is_empty());
    }

    #[test]
    fn paste_flags_scale_without_multiplier() {
        let mut s = with_expert();
        for i in 0..8u64 {
            s.finish_question(10 + i).unwrap();
        }
        assert_eq!(s.current_index(), Some(8));
        s.record_paste_flag(500, 30).unwrap();
        s.record_paste_flag(900, 31).unwrap();
        let b = live_rating(&s, &BTreeMap::new());
        let lines = find(&b, 8, Paste);
        assert_eq!(lines.len(), 1);
        close(lines[0].delta, -0.60);
        close(b.rating, 5.40);
    }

    #[test]
    fn chat_and_test_events_do_not_move_rating() {
        let mut s = started();
        s.select_approach(ApproachChoice::Preset(0), 100, 5)
            .unwrap();
        s.add_tokens(50).unwrap();
        let map = BTreeMap::from([(0, vec![Best])]);
        let before = live_rating(&s, &map);
        s.record_test_result(0, 5, 6).unwrap();
        s.record_chat_boundary_violation(7).unwrap();
        s.request_tests(8).unwrap();
        s.continue_work(9).unwrap();
        let after = live_rating(&s, &map);
        close(after.rating, before.rating);
        close(after.unclamped, before.unclamped);
        assert_eq!(after.lines, before.lines);
    }
}
