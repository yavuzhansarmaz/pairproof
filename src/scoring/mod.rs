//! Fixed rating step table, token comparison, and per-question grade merge.

pub mod final_merge;
pub mod live;
pub mod shadow;
pub mod types;

pub use final_merge::{final_rating, FinalBreakdown, PerQuestionFinal};
pub use live::{
    acceptance_factor, difficulty_multiplier, live_rating, token_band_step, RatingBreakdown,
    ScoreComponent, ScoreLine,
};
pub use shadow::{
    assess_decision, evaluate_set, fingerprint_for, other_evaluation, Confidence,
    DecisionAssessment, DecisionStanding, DimensionScores, EvalLevel, OptionEvaluation,
    OptionVerdict, SetEvaluation, SetVerdict, ShadowReport, ShadowStatus, RUBRIC_VERSION,
};
pub use types::{
    ApproachGrade, ApproachOption, ApproachOptionView, ApproachQuality, ApproachSet, GradeOutcome,
    QuestionGrade, ScoringError,
};
