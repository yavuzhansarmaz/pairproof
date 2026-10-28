//! Shared scoring types for approach options and per-question grades.

/// Errors returned by scoring type validation and grade merging.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScoringError {
    #[error("invalid grade: {0}")]
    InvalidGrade(String),
    #[error("invalid approach set: {0}")]
    InvalidApproachSet(String),
    #[error("unknown question slot: {0}")]
    UnknownSlot(usize),
    #[error("grade supplied for skipped slot: {0}")]
    GradeForSkipped(usize),
    #[error("duplicate grade for slot: {0}")]
    DuplicateGrade(usize),
}

/// Quality of one solution approach on the fixed step scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApproachQuality {
    Best,
    Workable,
    Weak,
}

impl ApproachQuality {
    /// Returns the unmultiplied rating step for this quality.
    pub fn step(self) -> f64 {
        match self {
            ApproachQuality::Best => 0.30,
            ApproachQuality::Workable => 0.10,
            ApproachQuality::Weak => -0.20,
        }
    }
}

/// One generated approach option with its hidden quality grade.
#[derive(Debug, Clone, PartialEq)]
pub struct ApproachOption {
    pub title: String,
    pub summary: String,
    pub quality: ApproachQuality,
    pub expected_tokens: u64,
}

/// Candidate-visible view of an approach option (no quality or tokens).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApproachOptionView {
    pub title: String,
    pub summary: String,
}

/// A validated set of exactly 4 options with exactly one best option.
#[derive(Debug, Clone, PartialEq)]
pub struct ApproachSet {
    pub options: Vec<ApproachOption>,
    pub best_index: usize,
}

impl ApproachSet {
    /// Validates `options` and records the hidden best option index.
    pub fn new(options: Vec<ApproachOption>) -> Result<Self, ScoringError> {
        let bad = |m: String| ScoringError::InvalidApproachSet(m);
        if options.len() != 4 {
            return Err(bad(format!("expected 4 options, got {}", options.len())));
        }
        for option in &options {
            if option.title.trim().is_empty() {
                return Err(bad("option title must not be empty".to_string()));
            }
            if option.summary.trim().is_empty() {
                return Err(bad("option summary must not be empty".to_string()));
            }
            if option.expected_tokens == 0 {
                return Err(bad("expected tokens must be greater than zero".to_string()));
            }
        }
        let best: Vec<usize> = options
            .iter()
            .enumerate()
            .filter(|(_, o)| o.quality == ApproachQuality::Best)
            .map(|(i, _)| i)
            .collect();
        if best.len() != 1 {
            return Err(bad(format!(
                "expected exactly one best option, got {}",
                best.len()
            )));
        }
        Ok(Self {
            options,
            best_index: best[0],
        })
    }

    /// Returns candidate-visible options without quality or token data.
    pub fn candidate_options(&self) -> Vec<ApproachOptionView> {
        self.options
            .iter()
            .map(|o| ApproachOptionView {
                title: o.title.clone(),
                summary: o.summary.clone(),
            })
            .collect()
    }

    /// Returns the hidden quality of a preset, or `None` if out of range.
    pub fn quality_of(&self, preset: u8) -> Option<ApproachQuality> {
        self.options.get(preset as usize).map(|o| o.quality)
    }
}

/// Grade for a candidate-written `Other` approach, assigned at selection.
#[derive(Debug, Clone, PartialEq)]
pub struct ApproachGrade {
    pub quality: ApproachQuality,
    pub expected_tokens: u64,
    pub rationale: String,
}

impl ApproachGrade {
    /// Validates a positive token expectation and a non-empty rationale.
    pub fn new(
        quality: ApproachQuality,
        expected_tokens: u64,
        rationale: String,
    ) -> Result<Self, ScoringError> {
        if expected_tokens == 0 {
            return Err(ScoringError::InvalidGrade(
                "expected tokens must be greater than zero".to_string(),
            ));
        }
        if rationale.trim().is_empty() {
            return Err(ScoringError::InvalidGrade(
                "rationale must not be empty".to_string(),
            ));
        }
        Ok(Self {
            quality,
            expected_tokens,
            rationale,
        })
    }
}

/// End-of-session AI quality grade for one question slot.
#[derive(Debug, Clone, PartialEq)]
pub struct QuestionGrade {
    pub slot_index: usize,
    pub grade: f64,
    pub rationale: String,
    pub correction: f64,
}

impl QuestionGrade {
    /// Validates grade range, finite correction, and non-empty rationale.
    pub fn new(
        slot_index: usize,
        grade: f64,
        rationale: String,
        correction: f64,
    ) -> Result<Self, ScoringError> {
        if !grade.is_finite() || !(0.0..=10.0).contains(&grade) {
            return Err(ScoringError::InvalidGrade(format!(
                "grade {grade} is not finite in 0.0..=10.0"
            )));
        }
        if !correction.is_finite() {
            return Err(ScoringError::InvalidGrade(format!(
                "correction {correction} is not finite"
            )));
        }
        if rationale.trim().is_empty() {
            return Err(ScoringError::InvalidGrade(
                "rationale must not be empty".to_string(),
            ));
        }
        Ok(Self {
            slot_index,
            grade,
            rationale,
            correction,
        })
    }
}

/// Outcome of end-of-session grading for one question slot.
#[derive(Debug, Clone, PartialEq)]
pub enum GradeOutcome {
    Graded(QuestionGrade),
    NotScored { slot_index: usize },
}

#[cfg(test)]
mod tests {
    use super::ApproachQuality::*;
    use super::*;

    fn opt(title: &str, quality: ApproachQuality, tokens: u64) -> ApproachOption {
        ApproachOption {
            title: title.to_string(),
            summary: format!("{title} summary"),
            quality,
            expected_tokens: tokens,
        }
    }

    fn valid_options() -> Vec<ApproachOption> {
        vec![
            opt("a", Workable, 100),
            opt("b", Weak, 200),
            opt("c", Best, 300),
            opt("d", Workable, 400),
        ]
    }

    #[test]
    fn approach_set_new_rejects_wrong_counts_and_zero_tokens() {
        let ok = ApproachSet::new(valid_options()).expect("valid set");
        assert_eq!(ok.best_index, 2);
        let mut three = valid_options();
        three.pop();
        assert!(matches!(
            ApproachSet::new(three),
            Err(ScoringError::InvalidApproachSet(_))
        ));
        let mut five = valid_options();
        five.push(opt("e", Weak, 10));
        assert!(ApproachSet::new(five).is_err());
        let mut zero = valid_options();
        zero[1].expected_tokens = 0;
        assert!(ApproachSet::new(zero).is_err());
        let mut no_best = valid_options();
        no_best[2].quality = Weak;
        assert!(ApproachSet::new(no_best).is_err());
        let mut two_best = valid_options();
        two_best[0].quality = Best;
        assert!(ApproachSet::new(two_best).is_err());
        let mut blank_title = valid_options();
        blank_title[0].title = "  ".to_string();
        assert!(ApproachSet::new(blank_title).is_err());
        let mut blank_summary = valid_options();
        blank_summary[3].summary = String::new();
        assert!(ApproachSet::new(blank_summary).is_err());
    }

    #[test]
    fn approach_set_candidate_view_hides_best_and_quality() {
        let set = ApproachSet::new(valid_options()).expect("valid set");
        let views = set.candidate_options();
        assert_eq!(views.len(), 4);
        for (view, option) in views.iter().zip(set.options.iter()) {
            let ApproachOptionView { title, summary } = view;
            assert_eq!(title, &option.title);
            assert_eq!(summary, &option.summary);
        }
        assert_eq!(set.quality_of(2), Some(Best));
        assert_eq!(set.quality_of(1), Some(Weak));
        assert_eq!(set.quality_of(4), None);
        assert_eq!(set.quality_of(255), None);
    }

    #[test]
    fn question_grade_new_rejects_out_of_range_and_nan() {
        let ok = QuestionGrade::new(3, 10.0, "solid".into(), -0.5).expect("boundary grade");
        assert_eq!((ok.slot_index, ok.grade, ok.correction), (3, 10.0, -0.5));
        assert!(QuestionGrade::new(0, 0.0, "poor".into(), 0.0).is_ok());
        let outcome = GradeOutcome::Graded(ok);
        assert!(matches!(outcome, GradeOutcome::Graded(_)));
        assert!(matches!(
            GradeOutcome::NotScored { slot_index: 1 },
            GradeOutcome::NotScored { slot_index: 1 }
        ));
        for grade in [-0.1, 10.1, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(
                matches!(
                    QuestionGrade::new(0, grade, "r".into(), 0.0),
                    Err(ScoringError::InvalidGrade(_))
                ),
                "grade {grade}"
            );
        }
        for correction in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(QuestionGrade::new(0, 5.0, "r".into(), correction).is_err());
        }
        assert!(QuestionGrade::new(0, 5.0, "   ".into(), 0.0).is_err());
    }

    #[test]
    fn approach_grade_new_validates_tokens_and_rationale() {
        let ok = ApproachGrade::new(Best, 100, "optimal plan".into()).expect("valid grade");
        assert_eq!((ok.quality, ok.expected_tokens), (Best, 100));
        assert_eq!(Best.step(), 0.30);
        assert_eq!(Workable.step(), 0.10);
        assert_eq!(Weak.step(), -0.20);
        assert!(ApproachGrade::new(Weak, 0, "r".into()).is_err());
        assert!(ApproachGrade::new(Weak, 10, "  ".into()).is_err());
    }
}
