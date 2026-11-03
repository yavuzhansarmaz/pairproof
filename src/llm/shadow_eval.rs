//! Independent shadow evaluation of a generated approach set.
//! The evaluator sees only the question and candidate-visible titles
//! and summaries; it never receives hidden quality, `best_index`, or
//! expected tokens, and its result never feeds production scoring.

use serde::Deserialize;

use crate::llm::common::{decode, LlmError, LlmOutcome};
use crate::llm::{Message, MetaClient};
use crate::scoring::shadow::{
    Confidence, DimensionScores, EvalLevel, OptionEvaluation, OptionVerdict, ShadowReport,
};
use crate::scoring::ApproachSet;

#[derive(Debug, Deserialize)]
struct ShadowWire {
    options: Vec<OptionWire>,
}

#[derive(Debug, Deserialize)]
struct OptionWire {
    option_index: usize,
    verdict: String,
    correctness: String,
    constraint_fit: String,
    complexity: String,
    tradeoff: String,
    risk: String,
    rationale: String,
    confidence: String,
}

fn parse_level(text: &str) -> Result<EvalLevel, LlmError> {
    match text.trim().to_ascii_lowercase().as_str() {
        "strong" => Ok(EvalLevel::Strong),
        "adequate" => Ok(EvalLevel::Adequate),
        "weak" => Ok(EvalLevel::Weak),
        "invalid" => Ok(EvalLevel::Invalid),
        "unknown" => Ok(EvalLevel::Unknown),
        other => Err(LlmError::Parse(format!("unknown eval level: {other}"))),
    }
}

fn parse_verdict(text: &str) -> Result<OptionVerdict, LlmError> {
    match text.trim().to_ascii_lowercase().as_str() {
        "strong" => Ok(OptionVerdict::Strong),
        "viable" => Ok(OptionVerdict::Viable),
        "weak" => Ok(OptionVerdict::Weak),
        "invalid" => Ok(OptionVerdict::Invalid),
        "insufficient_evidence" => Ok(OptionVerdict::InsufficientEvidence),
        other => Err(LlmError::Parse(format!("unknown verdict: {other}"))),
    }
}

fn parse_confidence(text: &str) -> Result<Confidence, LlmError> {
    match text.trim().to_ascii_lowercase().as_str() {
        "low" => Ok(Confidence::Low),
        "medium" => Ok(Confidence::Medium),
        "high" => Ok(Confidence::High),
        other => Err(LlmError::Parse(format!("unknown confidence: {other}"))),
    }
}

/// Builds evaluator messages from candidate-visible data only.
pub fn build_shadow_messages(question_statement: &str, set: &ApproachSet) -> Vec<Message> {
    let system = concat!(
        "You independently evaluate solution approaches for a coding ",
        "interview question. Reply with JSON only, no prose outside the ",
        "JSON, using exactly this schema: ",
        r#"{"options":[{"option_index":0,"verdict":"strong|viable|weak|invalid|insufficient_evidence","correctness":"strong|adequate|weak|invalid|unknown","constraint_fit":"strong|adequate|weak|invalid|unknown","complexity":"strong|adequate|weak|invalid|unknown","tradeoff":"strong|adequate|weak|invalid|unknown","risk":"strong|adequate|weak|invalid|unknown","rationale":"...","confidence":"low|medium|high"}]}"#,
        ". Evaluate each option independently against the question ",
        "constraints, correctness, time and space complexity, trade-off ",
        "quality, and edge-case risk. Multiple approaches may be equally ",
        "strong; do not assume exactly one approach is best and do not ",
        "infer quality from option order. Use insufficient_evidence or ",
        "unknown when the summary lacks needed information rather than ",
        "guessing. Provide exactly one entry for every option index."
    );
    let mut user = format!("Question:\n{question_statement}\n\nApproaches:\n");
    for (index, option) in set.options.iter().enumerate() {
        user.push_str(&format!("{index}. {} — {}\n", option.title, option.summary));
    }
    vec![Message::new("system", system), Message::new("user", &user)]
}

/// Parses an evaluator reply into a complete shadow report.
pub fn parse_shadow_response(
    content: &str,
    slot_index: usize,
    model: &str,
    set: &ApproachSet,
) -> Result<ShadowReport, LlmError> {
    let wire: ShadowWire = decode(content)?;
    if wire.options.len() != 4 {
        return Err(LlmError::Parse(format!(
            "expected 4 option evaluations, got {}",
            wire.options.len()
        )));
    }
    let mut seen = [false; 4];
    let mut options = Vec::with_capacity(4);
    for item in wire.options {
        if item.option_index >= 4 || seen[item.option_index] {
            return Err(LlmError::Parse(format!(
                "duplicate or out-of-range option_index {}",
                item.option_index
            )));
        }
        seen[item.option_index] = true;
        options.push(OptionEvaluation::new(
            item.option_index,
            parse_verdict(&item.verdict)?,
            DimensionScores {
                correctness: parse_level(&item.correctness)?,
                constraint_fit: parse_level(&item.constraint_fit)?,
                complexity: parse_level(&item.complexity)?,
                tradeoff: parse_level(&item.tradeoff)?,
                risk: parse_level(&item.risk)?,
            },
            item.rationale,
            parse_confidence(&item.confidence)?,
        )?);
    }
    options.sort_by_key(|o| o.option_index);
    Ok(ShadowReport::complete(
        slot_index,
        model.to_string(),
        set,
        options,
    ))
}

/// Evaluates a set in one strong-model request. Failure is returned
/// as an error; callers map it to `ShadowStatus::Failed` and must not
/// alter any production score or session state on failure.
pub async fn evaluate_shadow_set(
    client: &MetaClient,
    question_statement: &str,
    set: &ApproachSet,
    slot_index: usize,
) -> Result<LlmOutcome<ShadowReport>, LlmError> {
    let model = client.strong_model.clone();
    let messages = build_shadow_messages(question_statement, set);
    let outcome = client.chat(&model, messages).await?;
    let value = parse_shadow_response(&outcome.content, slot_index, &model, set)?;
    Ok(LlmOutcome {
        value,
        usage: outcome.usage,
    })
}

/// Maps an evaluation result to a report, preserving failure honestly.
pub fn report_or_failed(
    result: Result<ShadowReport, LlmError>,
    slot_index: usize,
    model: &str,
) -> ShadowReport {
    match result {
        Ok(report) => report,
        Err(err) => ShadowReport::failed(slot_index, model.to_string(), err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scoring::{ApproachOption, ApproachQuality};

    fn set() -> ApproachSet {
        let mk = |title: &str, summary: &str, quality: ApproachQuality| ApproachOption {
            title: title.into(),
            summary: summary.into(),
            quality,
            expected_tokens: 999,
        };
        ApproachSet::new(vec![
            mk("Hash map", "single pass lookup", ApproachQuality::Best),
            mk("Sort", "sort then scan", ApproachQuality::Workable),
            mk("Brute force", "compare all pairs", ApproachQuality::Weak),
            mk(
                "Two pointers",
                "sorted two pointers",
                ApproachQuality::Workable,
            ),
        ])
        .expect("set")
    }

    const GOOD: &str = r#"{"options":[
      {"option_index":0,"verdict":"strong","correctness":"strong","constraint_fit":"strong","complexity":"strong","tradeoff":"adequate","risk":"adequate","rationale":"linear and correct","confidence":"high"},
      {"option_index":1,"verdict":"viable","correctness":"strong","constraint_fit":"adequate","complexity":"adequate","tradeoff":"adequate","risk":"adequate","rationale":"works with extra log factor","confidence":"medium"},
      {"option_index":2,"verdict":"weak","correctness":"adequate","constraint_fit":"weak","complexity":"weak","tradeoff":"weak","risk":"adequate","rationale":"quadratic may time out","confidence":"high"},
      {"option_index":3,"verdict":"strong","correctness":"strong","constraint_fit":"strong","complexity":"strong","tradeoff":"strong","risk":"adequate","rationale":"also linear after sort for this input","confidence":"medium"}]}"#;

    #[test]
    fn prompt_hides_quality_best_and_tokens() {
        let messages = build_shadow_messages("Find a pair.", &set());
        let text = messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Hash map"));
        assert!(!text.contains("999"), "tokens leaked: {text}");
        assert!(!text.contains("best_index"), "best marker leaked");
        assert!(text.contains("Multiple approaches may be equally strong"));
    }

    #[test]
    fn parse_allows_tied_strong_options() {
        let report = parse_shadow_response(GOOD, 2, "m", &set()).expect("parse");
        assert_eq!(report.slot_index, 2);
        assert_eq!(report.options.len(), 4);
        assert_eq!(report.options[0].verdict, OptionVerdict::Strong);
        assert_eq!(report.options[3].verdict, OptionVerdict::Strong);
        assert!(report.set_evaluation.is_some());
    }

    #[test]
    fn parse_rejects_malformed_duplicate_and_unknown() {
        assert!(parse_shadow_response("not json", 0, "m", &set()).is_err());
        let dup = GOOD.replacen("\"option_index\":3", "\"option_index\":2", 1);
        assert!(parse_shadow_response(&dup, 0, "m", &set()).is_err());
        let unknown = GOOD.replacen("\"viable\"", "\"excellent\"", 1);
        assert!(parse_shadow_response(&unknown, 0, "m", &set()).is_err());
        let short = r#"{"options":[]}"#;
        assert!(parse_shadow_response(short, 0, "m", &set()).is_err());
    }

    #[test]
    fn failed_mapping_preserves_error_without_options() {
        let report = report_or_failed(Err(LlmError::Parse("bad json".into())), 5, "strong-model");
        assert_eq!(report.slot_index, 5);
        assert!(report.options.is_empty());
        assert!(matches!(
            report.status,
            crate::scoring::ShadowStatus::Failed { .. }
        ));
    }
}
