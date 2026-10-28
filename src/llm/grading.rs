//! Per-question end-of-session grading, one request per question.

use serde::Deserialize;

use crate::core::{ApproachChoice, EventKind, QuestionStatus, Session, TestSummary};
use crate::llm::common::{strip_code_fences, LlmError, LlmOutcome};
use crate::llm::{Message, MetaClient};
use crate::scoring::{GradeOutcome, QuestionGrade};

/// Everything one per-question grading request needs for one slot.
#[derive(Debug, Clone, PartialEq)]
pub struct QuestionScoringInput {
    pub slot_index: usize,
    pub question_statement: String,
    pub approach_text: String,
    pub solution_code: String,
    pub test_summary: Option<TestSummary>,
    pub tests_requested: bool,
    pub expected_tokens: Option<u64>,
    pub actual_tokens: u64,
    pub actions: Vec<String>,
}

/// Describes one approach choice the way grading prompts present it.
fn describe_choice(choice: &ApproachChoice) -> String {
    match choice {
        ApproachChoice::Preset(n) => format!("preset option {n}"),
        ApproachChoice::Other(text) => text.clone(),
    }
}

/// Summarises one slot's candidate actions from the session event log.
fn slot_actions(session: &Session, slot_index: usize) -> Vec<String> {
    let mut actions = Vec::new();
    for event in session.events() {
        let summary = match &event.kind {
            EventKind::ApproachSelected { index, choice } if *index == slot_index => {
                format!("approach selected: {}", describe_choice(choice))
            }
            EventKind::Continued { index } if *index == slot_index => "continued".to_string(),
            EventKind::Redirected { index, instruction } if *index == slot_index => {
                format!("redirected: {instruction}")
            }
            EventKind::ApproachChanged { index } if *index == slot_index => {
                "approach changed".to_string()
            }
            EventKind::TestsRequested { index } if *index == slot_index => {
                "tests requested".to_string()
            }
            EventKind::TestRunCompleted {
                index,
                passed,
                failed,
            } if *index == slot_index => {
                format!("test run completed: {passed} passed, {failed} failed")
            }
            EventKind::QuestionFinished { index } if *index == slot_index => {
                "question finished".to_string()
            }
            EventKind::QuestionSkipped { index } if *index == slot_index => {
                "question skipped".to_string()
            }
            _ => continue,
        };
        actions.push(summary);
    }
    actions
}

impl QuestionScoringInput {
    /// Builds grading input for one slot, or `None` if no such slot exists.
    pub fn from_session(
        session: &Session,
        slot_index: usize,
        question_statement: &str,
        solution_code: &str,
    ) -> Option<Self> {
        let slot = session.slots().iter().find(|s| s.index == slot_index)?;
        let approach_text = match &slot.approach {
            Some(choice) => describe_choice(choice),
            None => "none".to_string(),
        };
        let tests_requested = session
            .events()
            .iter()
            .any(|e| matches!(e.kind, EventKind::TestsRequested { index } if index == slot_index));
        Some(Self {
            slot_index,
            question_statement: question_statement.to_string(),
            approach_text,
            solution_code: solution_code.to_string(),
            test_summary: slot.latest_test,
            tests_requested,
            expected_tokens: session.token_locks(slot_index).last().copied(),
            actual_tokens: slot.actual_tokens,
            actions: slot_actions(session, slot_index),
        })
    }
}

/// Returns the slot indexes that get a grading request: finished only.
pub fn scorable_slots(session: &Session) -> Vec<usize> {
    session
        .slots()
        .iter()
        .filter(|s| s.status == QuestionStatus::Finished)
        .map(|s| s.index)
        .collect()
}

/// Builds system and user messages grading exactly one question.
pub fn build_question_grading_messages(input: &QuestionScoringInput) -> Vec<Message> {
    let system = concat!(
        "You grade one candidate's work on a single coding interview question. ",
        "Reply with JSON only, no prose outside the JSON, using exactly this schema: ",
        r#"{"grade":7.5,"rationale":"...","correction":0.2}"#,
        ". grade is the 0-10 solution quality for this one question: judge the ",
        "code against the question statement. Whether the candidate requested ",
        "tests and how they directed the AI is judgment evidence. Failing tests ",
        "alone are the AI's code fault; how the candidate reacted to them is ",
        "what matters. rationale briefly explains the grade. correction is a ",
        "small per-question rating adjustment, typically within -0.4..=0.4; ",
        "totals are capped later, so never exceed -0.8..=0.8."
    );
    let test_summary = match input.test_summary {
        Some(t) => format!("{} passed, {} failed", t.passed, t.failed),
        None => "none".to_string(),
    };
    let expected_tokens = match input.expected_tokens {
        Some(n) => n.to_string(),
        None => "none".to_string(),
    };
    let actions = if input.actions.is_empty() {
        "none".to_string()
    } else {
        input.actions.join("\n")
    };
    let user = format!(
        "Slot index: {}\n\nQuestion:\n{}\n\nChosen approach:\n{}\n\nSolution code:\n{}\n\n\
         Test summary: {}\nTests requested by candidate: {}\n\
         Expected tokens: {}\nActual tokens: {}\n\nCandidate actions:\n{}",
        input.slot_index,
        input.question_statement,
        input.approach_text,
        input.solution_code,
        test_summary,
        input.tests_requested,
        expected_tokens,
        input.actual_tokens,
        actions,
    );
    vec![Message::new("system", system), Message::new("user", &user)]
}

#[derive(Debug, Deserialize)]
struct QuestionGradeWire {
    grade: f64,
    rationale: String,
    correction: f64,
}

/// Parses a model reply into a validated grade for `slot_index`.
pub fn parse_question_grade(content: &str, slot_index: usize) -> Result<QuestionGrade, LlmError> {
    let wire: QuestionGradeWire = serde_json::from_str(strip_code_fences(content))
        .map_err(|e| LlmError::Parse(e.to_string()))?;
    Ok(QuestionGrade::new(
        slot_index,
        wire.grade,
        wire.rationale,
        wire.correction,
    )?)
}

/// Grades exactly one question in one strong-model request.
pub async fn grade_question(
    client: &MetaClient,
    input: &QuestionScoringInput,
) -> Result<LlmOutcome<QuestionGrade>, LlmError> {
    let model = client.strong_model.clone();
    let messages = build_question_grading_messages(input);
    let outcome = client.chat(&model, messages).await?;
    let value = parse_question_grade(&outcome.content, input.slot_index)?;
    Ok(LlmOutcome {
        value,
        usage: outcome.usage,
    })
}

/// Grades each input in its own request; failures become `NotScored`.
pub async fn grade_questions(
    client: &MetaClient,
    inputs: &[QuestionScoringInput],
) -> Vec<GradeOutcome> {
    let mut outcomes = Vec::with_capacity(inputs.len());
    for input in inputs {
        match grade_question(client, input).await {
            Ok(outcome) => outcomes.push(GradeOutcome::Graded(outcome.value)),
            Err(_) => outcomes.push(GradeOutcome::NotScored {
                slot_index: input.slot_index,
            }),
        }
    }
    outcomes
}
