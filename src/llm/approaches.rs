//! Approach-set generation and candidate `Other` approach grading.

use rand::seq::SliceRandom;
use rand::Rng;
use serde::Deserialize;

use crate::core::Language;
use crate::llm::common::{
    chat_json_with_repair, decode, language_name, parse_quality, LlmError, LlmOutcome,
};
use crate::llm::{Message, MetaClient};
use crate::scoring::{ApproachGrade, ApproachOption, ApproachSet};

#[derive(Debug, Deserialize)]
struct ApproachSetWire {
    approaches: Vec<ApproachWire>,
}

#[derive(Debug, Deserialize)]
struct ApproachWire {
    title: String,
    summary: String,
    quality: String,
    expected_tokens: u64,
}

#[derive(Debug, Deserialize)]
struct ApproachGradeWire {
    quality: String,
    expected_tokens: u64,
    rationale: String,
}

/// Builds system and user messages asking for 4 graded approaches.
pub fn build_approach_messages(question_statement: &str, language: Language) -> Vec<Message> {
    let system = concat!(
        "You generate solution approaches for a coding interview question. ",
        "Reply with JSON only, no prose outside the JSON, using exactly this schema: ",
        r#"{"approaches":[{"title":"...","summary":"...","quality":"best|workable|weak","expected_tokens":123}]}"#,
        ". Provide exactly 4 distinct strategies for solving the question. ",
        "Exactly one approach must have quality \"best\"; the others must be ",
        "\"workable\" or \"weak\". ",
        "expected_tokens is the estimated total tokens needed to implement ",
        "that approach, as a positive integer. ",
        "Give each approach a short title and a one or two sentence summary. ",
        "Do not include code."
    );
    let user = format!(
        "Question:\n{question_statement}\n\nLanguage: {}",
        language_name(language)
    );
    vec![Message::new("system", system), Message::new("user", &user)]
}

/// Parses a model reply into a validated, shuffled approach set.
pub fn parse_approach_set(content: &str, rng: &mut impl Rng) -> Result<ApproachSet, LlmError> {
    let wire: ApproachSetWire = decode(content)?;
    let mut options: Vec<ApproachOption> = Vec::with_capacity(wire.approaches.len());
    for approach in wire.approaches {
        options.push(ApproachOption {
            title: approach.title,
            summary: approach.summary,
            quality: parse_quality(&approach.quality)?,
            expected_tokens: approach.expected_tokens,
        });
    }
    options.shuffle(rng);
    Ok(ApproachSet::new(options)?)
}

/// Generates the 4 approaches for a question on the strong model.
pub async fn generate_approaches(
    client: &MetaClient,
    question_statement: &str,
    language: Language,
    rng: &mut impl Rng,
) -> Result<LlmOutcome<ApproachSet>, LlmError> {
    let model = client.strong_model.clone();
    let messages = build_approach_messages(question_statement, language);
    chat_json_with_repair(
        client,
        &model,
        messages,
        |content| parse_approach_set(content, &mut *rng),
        &mut |_| Ok(()),
    )
    .await
}

/// Builds messages asking to grade a candidate-written approach.
pub fn build_other_grade_messages(question_statement: &str, other_text: &str) -> Vec<Message> {
    let system = concat!(
        "You grade a candidate's proposed solution approach for a coding ",
        "interview question. Reply with JSON only, no prose outside the JSON, ",
        "using exactly this schema: ",
        r#"{"quality":"best|workable|weak","expected_tokens":123,"rationale":"..."}"#,
        ". quality is \"best\" if the approach is an optimal strategy for this ",
        "question, \"workable\" if it can solve the question with acceptable ",
        "efficiency, and \"weak\" if it is incorrect, infeasible, or clearly ",
        "inefficient. expected_tokens is the estimated total tokens needed to ",
        "implement the approach, as a positive integer. ",
        "rationale briefly explains the grade."
    );
    let user = format!("Question:\n{question_statement}\n\nCandidate approach:\n{other_text}");
    vec![Message::new("system", system), Message::new("user", &user)]
}

/// Parses a model reply into a validated approach grade.
pub fn parse_approach_grade(content: &str) -> Result<ApproachGrade, LlmError> {
    let wire: ApproachGradeWire = decode(content)?;
    let quality = parse_quality(&wire.quality)?;
    Ok(ApproachGrade::new(
        quality,
        wire.expected_tokens,
        wire.rationale,
    )?)
}

/// Grades a candidate-written approach on the strong model.
pub async fn grade_other_approach(
    client: &MetaClient,
    question_statement: &str,
    other_text: &str,
) -> Result<LlmOutcome<ApproachGrade>, LlmError> {
    let model = client.strong_model.clone();
    let messages = build_other_grade_messages(question_statement, other_text);
    chat_json_with_repair(client, &model, messages, parse_approach_grade, &mut |_| {
        Ok(())
    })
    .await
}
