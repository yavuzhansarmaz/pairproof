use pairproof::core::{Language, QuestionRef};
use pairproof::ingest::{LanguageProposal, ProposedContext};
use pairproof::leetcode::Difficulty;
use pairproof::llm::extraction::ExtractionResult;
use pairproof::llm::MetaClient;
use pairproof::tui::setup::{ExpertSetupFocus, ExpertSetupPastes, SetupController, SetupError};

fn easy(slug: &str) -> QuestionRef {
    QuestionRef {
        title_slug: slug.to_string(),
        title: format!("Title {slug}"),
        difficulty: Difficulty::Easy,
        acceptance_pct: Some(50),
    }
}

fn five_easy() -> Vec<QuestionRef> {
    vec![easy("e0"), easy("e1"), easy("e2"), easy("e3"), easy("e4")]
}

fn proposal(language: LanguageProposal) -> ProposedContext {
    ProposedContext {
        topics: vec!["array".to_string(), "hash-table".to_string()],
        level: "mid".to_string(),
        language,
        language_reason: "job description names it".to_string(),
        jd_truncated: false,
        additional_truncated: false,
    }
}

#[test]
fn setup_discloses_meta_transfer_before_proposal_call() {
    let controller = SetupController::new();
    let message = SetupController::disclosure_message();
    assert!(message.contains("Meta API"));
    assert!(message.contains("job description"));
    assert!(!controller.can_request_proposal());
    assert!(!controller.draft().disclosure_acknowledged);
}

#[tokio::test]
async fn setup_rejects_blank_jd_without_api_call() {
    let mut controller = SetupController::new();
    controller.acknowledge_disclosure();
    // Client points nowhere; a blank JD must fail before any network use.
    let client = MetaClient::new("http://127.0.0.1:9", "test-key-not-real");
    let err = controller.request_proposal(&client).await.unwrap_err();
    assert!(matches!(err, SetupError::Ingest(_)));
    assert!(controller.validate_for_proposal().is_err());
    assert!(!controller.can_request_proposal());
}

#[tokio::test]
async fn disclosure_gate_blocks_proposal_request() {
    let mut controller = SetupController::new();
    controller.set_jd("Senior backend engineer, Python, arrays.");
    assert!(!controller.can_request_proposal());
    let client = MetaClient::new("http://127.0.0.1:9", "test-key-not-real");
    let err = controller.request_proposal(&client).await.unwrap_err();
    assert!(matches!(err, SetupError::DisclosureNotAcknowledged));
    controller.acknowledge_disclosure();
    assert!(controller.can_request_proposal());
}

#[test]
fn setup_preserves_truncation_warning() {
    let mut controller = SetupController::new();
    controller.set_jd(&"x".repeat(50_001));
    controller.set_additional(&"y".repeat(50_001));
    assert_eq!(controller.truncation_flags(), (true, true));
    let docs = controller.validate_for_proposal().unwrap();
    assert!(docs.jd.truncated);
    assert!(docs.additional.truncated);
    let mut proposed = proposal(LanguageProposal::Proposed(Language::Python));
    proposed.jd_truncated = true;
    proposed.additional_truncated = true;
    controller.apply_proposal(proposed);
    let lines = controller.proposal_lines();
    assert!(lines.iter().any(|l| l.contains("truncated")));
    assert!(lines.iter().any(|l| l.contains("Job description")));
}

#[test]
fn setup_surfaces_unsupported_language() {
    let mut controller = SetupController::new();
    controller.set_jd("Java backend role.");
    controller.apply_proposal(proposal(LanguageProposal::Unsupported("Java".to_string())));
    let warning = controller.language_warning().unwrap();
    assert!(warning.contains("Unsupported language"));
    assert!(warning.contains("Python, JavaScript, SQL"));
    assert_eq!(controller.selected_language(), None);
    // Overriding with a supported language clears the warning.
    controller.override_language(Language::Python);
    assert_eq!(controller.selected_language(), Some(Language::Python));
    assert_eq!(controller.language_warning(), None);
}

#[test]
fn setup_auto_selects_five_easy_without_preview() {
    let mut controller = SetupController::new();
    assert!(!controller.has_auto_easy());
    assert!(controller.easy_questions().is_empty());
    controller.accept_auto_easy(five_easy()).unwrap();
    assert!(controller.has_auto_easy());
    assert_eq!(controller.easy_questions().len(), 5);
    assert!(controller
        .easy_questions()
        .iter()
        .all(|q| q.difficulty == Difficulty::Easy));
}

#[test]
fn auto_easy_rejects_wrong_count_difficulty_and_duplicates() {
    let mut controller = SetupController::new();
    assert!(matches!(
        controller.accept_auto_easy(vec![easy("a")]),
        Err(SetupError::InvalidQuestionCount {
            expected: 5,
            got: 1
        })
    ));
    let mut medium = five_easy();
    medium[0].difficulty = Difficulty::Medium;
    assert!(matches!(
        controller.accept_auto_easy(medium),
        Err(SetupError::WrongDifficulty { .. })
    ));
    let mut duplicate = five_easy();
    duplicate[1] = easy("e0");
    assert!(matches!(
        controller.accept_auto_easy(duplicate),
        Err(SetupError::DuplicateSlug(_))
    ));
    let mut blank = five_easy();
    blank[0].title_slug = "  ".to_string();
    assert!(matches!(
        controller.accept_auto_easy(blank),
        Err(SetupError::InvalidQuestion(_))
    ));
    assert!(!controller.has_auto_easy());
}

#[test]
fn blank_additional_notes_are_allowed() {
    let mut controller = SetupController::new();
    controller.set_jd("Python engineer working on arrays.");
    controller.set_additional("   ");
    let docs = controller.validate_for_proposal().unwrap();
    assert_eq!(docs.additional_text(), None);
    assert_eq!(controller.truncation_flags(), (false, false));
}

#[test]
fn language_confirm_and_override() {
    let mut controller = SetupController::new();
    controller.apply_proposal(proposal(LanguageProposal::Proposed(Language::Python)));
    assert_eq!(controller.selected_language(), Some(Language::Python));
    controller.confirm_language(Language::Python);
    assert_eq!(controller.selected_language(), Some(Language::Python));
    controller.override_language(Language::Sql);
    assert_eq!(controller.selected_language(), Some(Language::Sql));
    assert_eq!(controller.language_warning(), None);
}

#[test]
fn proposal_error_path_via_injected_extraction() {
    let mut controller = SetupController::new();
    controller.set_jd("Python engineer.");
    controller.acknowledge_disclosure();
    let empty_topics = ExtractionResult {
        topics: vec![],
        level: "mid".to_string(),
        proposed_language: Some(Language::Python),
        unsupported_language: None,
        language_reason: "default".to_string(),
    };
    assert!(controller.apply_extraction(empty_topics).is_err());
    assert!(controller.proposal().is_none());
    let good = ExtractionResult {
        topics: vec!["array".to_string()],
        level: "mid".to_string(),
        proposed_language: Some(Language::Python),
        unsupported_language: None,
        language_reason: "default".to_string(),
    };
    let proposed = controller.apply_extraction(good).unwrap();
    assert_eq!(proposed.topics, vec!["array".to_string()]);
    assert!(controller.proposal().is_some());
}

#[test]
fn proposal_lines_are_sanitized_and_hide_raw_jd() {
    let secret_jd = "SECRETJD array engineer";
    let mut controller = SetupController::new();
    controller.set_jd(secret_jd);
    let mut proposed = proposal(LanguageProposal::Proposed(Language::Python));
    proposed.topics = vec!["\u{1b}[31marray".to_string()];
    proposed.level = "m\u{1b}id".to_string();
    controller.apply_proposal(proposed);
    let lines = controller.proposal_lines();
    assert!(!lines.iter().any(|l| l.contains('\u{1b}')));
    assert!(!lines.iter().any(|l| l.contains("SECRETJD")));
    assert!(lines.iter().any(|l| l.contains("array")));
    assert!(lines.iter().any(|l| l.contains("Python")));
}

#[test]
fn focused_pastes_make_notes_optional_and_keep_texts_on_back() {
    let mut pastes = ExpertSetupPastes::new();
    assert_eq!(pastes.focus(), ExpertSetupFocus::JobDescription);
    assert!(!pastes.confirm_job_description());
    pastes
        .apply_focused_paste("Senior Rust role, async systems.")
        .expect("jd paste");
    assert!(pastes.is_ready());
    assert!(!pastes.is_complete(), "notes are optional, not required");
    assert!(pastes.confirm_job_description());
    assert_eq!(pastes.focus(), ExpertSetupFocus::AdditionalNotes);
    // Going back keeps both texts so a wrong paste can be corrected.
    pastes.back_to_job_description();
    assert_eq!(pastes.jd(), "Senior Rust role, async systems.");
    assert_eq!(pastes.focus(), ExpertSetupFocus::JobDescription);
    pastes
        .apply_focused_paste("Corrected JD text.")
        .expect("corrected jd");
    assert_eq!(pastes.jd(), "Corrected JD text.");
    assert!(pastes.confirm_job_description());
    pastes.scroll_focused(3);
    assert_eq!(pastes.notes_scroll(), 3);
    pastes.scroll_focused(-10);
    assert_eq!(pastes.notes_scroll(), 0);
    // Empty notes stay empty and ready: Enter on notes skips them.
    assert_eq!(pastes.notes(), "");
    assert!(pastes.is_ready());
}

#[test]
fn injected_unsupported_extraction_surfaces_warning() {
    let mut controller = SetupController::new();
    controller.set_jd("Rust systems role.");
    let result = ExtractionResult {
        topics: vec!["array".to_string()],
        level: "senior".to_string(),
        proposed_language: None,
        unsupported_language: Some("Rust".to_string()),
        language_reason: "job description names Rust".to_string(),
    };
    let proposed = controller.apply_extraction(result).unwrap();
    assert!(matches!(
        proposed.language,
        LanguageProposal::Unsupported(_)
    ));
    assert!(controller.language_warning().is_some());
}
