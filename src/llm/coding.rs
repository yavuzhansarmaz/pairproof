//! Single strong-model coding step; execution stays with the sandbox.

use serde::Deserialize;

use crate::core::{Language, Session};
use crate::llm::common::{decode, language_name, LlmError, LlmOutcome};
use crate::llm::{Message, MetaClient, Usage};

/// Inputs for one coding step on the active question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingStepInput {
    pub question_statement: String,
    pub approach_text: String,
    pub current_code: String,
    pub instruction: Option<String>,
    pub tests_requested: bool,
    pub language: Language,
}

/// Parsed result of one coding step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingStepOutput {
    pub code: String,
    pub ready: bool,
    pub summary: String,
}

#[derive(Debug, Deserialize)]
struct CodingStepWire {
    code: String,
    ready: bool,
    summary: String,
}

/// Builds system and user messages for one coding step.
pub fn build_coding_messages(input: &CodingStepInput) -> Vec<Message> {
    let tests_directive = if input.tests_requested {
        concat!(
            "Include a small self-test in the same file: asserts under a main ",
            "guard (or the language equivalent) that test the examples from ",
            "the question and a few edge cases."
        )
    } else {
        "Include no test code: write only the solution itself."
    };
    let system = format!(
        concat!(
            "You write code for a coding interview question. ",
            "Reply with JSON only, no prose outside the JSON, using exactly ",
            "this schema: ",
            r#"{{"code":"<complete file>","ready":true,"summary":"..."}}"#,
            ". code must be the complete contents of the solution file in ",
            "{language}, syntactically correct and runnable as-is. ",
            "Set ready to true only when the solution is complete and solves ",
            "the question; otherwise set ready to false. ",
            "summary briefly describes the current state of the code. ",
            "{tests_directive}"
        ),
        language = language_name(input.language),
        tests_directive = tests_directive,
    );
    let current = if input.current_code.trim().is_empty() {
        "(empty: no code written yet)"
    } else {
        &input.current_code
    };
    let mut user = format!(
        "Question:\n{}\n\nApproach:\n{}\n\nCurrent code:\n{}",
        input.question_statement, input.approach_text, current
    );
    if let Some(instruction) = &input.instruction {
        user.push_str(&format!("\n\nInstruction:\n{instruction}"));
    }
    vec![Message::new("system", &system), Message::new("user", &user)]
}

/// Parses a model reply into a coding step output.
pub fn parse_coding_step(content: &str) -> Result<CodingStepOutput, LlmError> {
    let wire: CodingStepWire = decode(content)?;
    if wire.code.trim().is_empty() {
        return Err(LlmError::Parse("coding step code must not be empty".into()));
    }
    Ok(CodingStepOutput {
        code: wire.code,
        ready: wire.ready,
        summary: wire.summary,
    })
}

/// Adds the total spent tokens to the session's active question.
pub fn apply_usage(session: &mut Session, usage: &Usage) -> Result<(), LlmError> {
    session.add_tokens(usage.total_tokens)?;
    Ok(())
}

/// Runs one coding step, booking spent tokens before parsing the reply.
pub async fn run_coding_step(
    client: &MetaClient,
    session: &mut Session,
    input: &CodingStepInput,
) -> Result<LlmOutcome<CodingStepOutput>, LlmError> {
    let model = client.strong_model.clone();
    let outcome = client.chat(&model, build_coding_messages(input)).await?;
    apply_usage(session, &outcome.usage)?;
    let value = parse_coding_step(&outcome.content)?;
    Ok(LlmOutcome {
        value,
        usage: outcome.usage,
    })
}
