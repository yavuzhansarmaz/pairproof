//! Shadow evaluation types; evidence only, never fed into ratings.

use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

use crate::core::ApproachChoice;
use crate::scoring::types::{ApproachGrade, ApproachQuality, ScoringError};
use crate::scoring::ApproachSet;

pub const RUBRIC_VERSION: &str = "shadow-v1";
const DUPLICATE_JACCARD: f64 = 0.80;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvalLevel {
    Strong,
    Adequate,
    Weak,
    Invalid,
    Unknown,
}

/// Per-dimension profile; avoids false-precision single numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DimensionScores {
    pub correctness: EvalLevel,
    pub constraint_fit: EvalLevel,
    pub complexity: EvalLevel,
    pub tradeoff: EvalLevel,
    pub risk: EvalLevel,
}

impl DimensionScores {
    /// All dimensions unknown (used for mapped `Other` grades).
    pub fn unknown() -> Self {
        Self {
            correctness: EvalLevel::Unknown,
            constraint_fit: EvalLevel::Unknown,
            complexity: EvalLevel::Unknown,
            tradeoff: EvalLevel::Unknown,
            risk: EvalLevel::Unknown,
        }
    }
}

/// Overall verdict for one option; ties between options are allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OptionVerdict {
    Strong,
    Viable,
    Weak,
    Invalid,
    InsufficientEvidence,
}

impl OptionVerdict {
    /// Ordinal tier, or `None` when evidence is insufficient.
    pub fn tier(self) -> Option<u8> {
        match self {
            Self::Strong => Some(3),
            Self::Viable => Some(2),
            Self::Weak => Some(1),
            Self::Invalid => Some(0),
            Self::InsufficientEvidence => None,
        }
    }
}

/// Evaluator confidence; low confidence blocks strong conclusions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Confidence {
    Low,
    Medium,
    High,
}

/// Independent quality evaluation of one generated option.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptionEvaluation {
    pub option_index: usize,
    pub verdict: OptionVerdict,
    pub dimensions: DimensionScores,
    pub rationale: String,
    pub confidence: Confidence,
}

impl OptionEvaluation {
    /// Validates a non-empty rationale.
    pub fn new(
        option_index: usize,
        verdict: OptionVerdict,
        dimensions: DimensionScores,
        rationale: String,
        confidence: Confidence,
    ) -> Result<Self, ScoringError> {
        if rationale.trim().is_empty() {
            return Err(ScoringError::InvalidGrade(
                "shadow rationale must not be empty".to_string(),
            ));
        }
        Ok(Self {
            option_index,
            verdict,
            dimensions,
            rationale,
            confidence,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SetVerdict {
    Useful,
    Compromised,
    Unusable,
    InsufficientEvidence,
}

/// Derived set-level evaluation (duplicates, invalid members).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetEvaluation {
    pub verdict: SetVerdict,
    pub duplicate_pairs: Vec<(usize, usize)>,
    pub invalid_count: usize,
    pub rationale: String,
}

fn tokens(text: &str) -> BTreeSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    let union = a.union(b).count();
    if union == 0 {
        return 1.0;
    }
    a.intersection(b).count() as f64 / union as f64
}

/// Derives set quality deterministically; no extra model call.
pub fn evaluate_set(set: &ApproachSet, evals: &[OptionEvaluation]) -> SetEvaluation {
    let sets: Vec<BTreeSet<String>> = set
        .options
        .iter()
        .map(|o| tokens(&format!("{} {}", o.title, o.summary)))
        .collect();
    let mut duplicate_pairs = Vec::new();
    for i in 0..sets.len() {
        for j in (i + 1)..sets.len() {
            if jaccard(&sets[i], &sets[j]) >= DUPLICATE_JACCARD {
                duplicate_pairs.push((i, j));
            }
        }
    }
    let invalid_count = evals
        .iter()
        .filter(|e| e.verdict == OptionVerdict::Invalid)
        .count();
    let weak_or_invalid = evals
        .iter()
        .filter(|e| matches!(e.verdict, OptionVerdict::Weak | OptionVerdict::Invalid))
        .count();
    let (verdict, rationale) = if evals.len() != 4 {
        (
            SetVerdict::InsufficientEvidence,
            format!("only {} of 4 options evaluated", evals.len()),
        )
    } else if invalid_count >= 2 || weak_or_invalid == 4 {
        (
            SetVerdict::Unusable,
            format!("{invalid_count} invalid options in set"),
        )
    } else if invalid_count == 1 || !duplicate_pairs.is_empty() || weak_or_invalid >= 2 {
        (
            SetVerdict::Compromised,
            format!(
                "{invalid_count} invalid, {} duplicate pairs, {weak_or_invalid} weak/invalid",
                duplicate_pairs.len()
            ),
        )
    } else {
        (
            SetVerdict::Useful,
            "four distinct options with no invalid member".to_string(),
        )
    };
    SetEvaluation {
        verdict,
        duplicate_pairs,
        invalid_count,
        rationale,
    }
}

/// How the selected option stands relative to the evaluated set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecisionStanding {
    AmongBest,
    Defensible,
    LikelySuboptimal,
    ClearlyPoor,
    InsufficientEvidence,
}

/// Assessment of a candidate decision; never a numeric score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionAssessment {
    pub choice: ApproachChoice,
    pub standing: DecisionStanding,
    pub evidence_complete: bool,
    pub rationale: String,
}

/// Maps an existing `Other` grade into a shadow profile; dimensions
/// stay `Unknown` since production grading gives no dimensions.
pub fn other_evaluation(grade: &ApproachGrade) -> OptionEvaluation {
    let verdict = match grade.quality {
        ApproachQuality::Best => OptionVerdict::Strong,
        ApproachQuality::Workable => OptionVerdict::Viable,
        ApproachQuality::Weak => OptionVerdict::Weak,
    };
    OptionEvaluation {
        option_index: 4,
        verdict,
        dimensions: DimensionScores::unknown(),
        rationale: grade.rationale.clone(),
        confidence: Confidence::Medium,
    }
}

/// Assesses a decision; incomplete/low-confidence evidence yields
/// `InsufficientEvidence`, and a one-tier gap is `Defensible`.
pub fn assess_decision(evals: &[OptionEvaluation], choice: ApproachChoice) -> DecisionAssessment {
    let insufficient = |why: &str| DecisionAssessment {
        choice: choice.clone(),
        standing: DecisionStanding::InsufficientEvidence,
        evidence_complete: false,
        rationale: why.to_string(),
    };
    if evals.len() < 4 {
        return insufficient("fewer than four options were evaluated");
    }
    let selected = match &choice {
        ApproachChoice::Preset(n) => evals.iter().find(|e| e.option_index == *n as usize),
        ApproachChoice::Other(_) => evals.iter().find(|e| e.option_index == 4),
    };
    let Some(selected) = selected else {
        return insufficient("selected option was not evaluated");
    };
    if selected.confidence == Confidence::Low {
        return insufficient("evaluator confidence for the selected option is low");
    }
    let Some(sel_tier) = selected.verdict.tier() else {
        return insufficient("selected option has insufficient evidence");
    };
    let expected = match &choice {
        ApproachChoice::Other(_) => 5,
        ApproachChoice::Preset(_) => 4,
    };
    let tiers: Vec<u8> = evals
        .iter()
        .filter(|e| match &choice {
            ApproachChoice::Preset(_) => e.option_index < 4,
            ApproachChoice::Other(_) => true,
        })
        .filter_map(|e| e.verdict.tier())
        .collect();
    if tiers.len() < expected {
        return insufficient("some options have insufficient evidence");
    }
    let max_tier = tiers.into_iter().max().unwrap_or(0);
    let (standing, rationale) = if selected.verdict == OptionVerdict::Invalid {
        (
            DecisionStanding::ClearlyPoor,
            "selected approach was independently judged invalid".to_string(),
        )
    } else if sel_tier == max_tier {
        (
            DecisionStanding::AmongBest,
            "selected approach is in the strongest evaluated tier (ties allowed)".to_string(),
        )
    } else if max_tier - sel_tier == 1 {
        (
            DecisionStanding::Defensible,
            "selected approach is one tier below the strongest; a valid trade-off may justify it"
                .to_string(),
        )
    } else {
        (
            DecisionStanding::LikelySuboptimal,
            "selected approach is more than one tier below the strongest evaluated option"
                .to_string(),
        )
    };
    DecisionAssessment {
        choice,
        standing,
        evidence_complete: true,
        rationale,
    }
}

/// Evaluation status; failure is explicit, never a fabricated score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ShadowStatus {
    Complete,
    Partial { missing: Vec<usize> },
    Failed { reason: String },
    NotEvaluated,
}

/// Full shadow report for one question slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShadowReport {
    pub slot_index: usize,
    pub rubric_version: String,
    pub evaluator_model: String,
    pub status: ShadowStatus,
    pub options: Vec<OptionEvaluation>,
    pub set_evaluation: Option<SetEvaluation>,
    pub decision: Option<DecisionAssessment>,
    pub fingerprint: String,
}

pub fn fingerprint_for(set: &ApproachSet) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for option in &set.options {
        option.title.hash(&mut hasher);
        option.summary.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

impl ShadowReport {
    /// Builds a complete report, deriving the set evaluation.
    pub fn complete(
        slot_index: usize,
        model: String,
        set: &ApproachSet,
        options: Vec<OptionEvaluation>,
    ) -> Self {
        let set_evaluation = Some(evaluate_set(set, &options));
        Self {
            slot_index,
            rubric_version: RUBRIC_VERSION.to_string(),
            evaluator_model: model,
            status: ShadowStatus::Complete,
            options,
            set_evaluation,
            decision: None,
            fingerprint: fingerprint_for(set),
        }
    }

    /// Builds an explicit failed report with no fabricated options.
    pub fn failed(slot_index: usize, model: String, reason: String) -> Self {
        Self {
            slot_index,
            rubric_version: RUBRIC_VERSION.to_string(),
            evaluator_model: model,
            status: ShadowStatus::Failed { reason },
            options: Vec::new(),
            set_evaluation: None,
            decision: None,
            fingerprint: String::new(),
        }
    }

    /// True when this report still matches the given approach set.
    pub fn is_current_for(&self, set: &ApproachSet) -> bool {
        !self.fingerprint.is_empty() && self.fingerprint == fingerprint_for(set)
    }
}
