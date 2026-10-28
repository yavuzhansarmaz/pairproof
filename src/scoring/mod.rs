//! Fixed rating step table, token comparison, and per-question grade merge.

pub mod final_merge;
pub mod live;
pub mod types;

pub use final_merge::{final_rating, FinalBreakdown, PerQuestionFinal};
pub use live::{
    difficulty_multiplier, live_rating, token_band_step, RatingBreakdown, ScoreComponent, ScoreLine,
};
pub use types::{
    ApproachGrade, ApproachOption, ApproachOptionView, ApproachQuality, ApproachSet, GradeOutcome,
    QuestionGrade, ScoringError,
};
