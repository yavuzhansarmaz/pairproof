use std::fs;
use std::path::PathBuf;

use pairproof::core::{ArtifactLayout, ContextSummary, Language, QuestionRef, Session};
use pairproof::ingest::{
    prepare_documents, propose_context, propose_from_extraction, store_raw_documents,
    write_context_summary, BoundedText, ExpertDocuments, IngestError, LanguageProposal,
    ProposedContext, MAX_TEXT_CHARS,
};
use pairproof::leetcode::Difficulty;
use pairproof::llm::extraction::{parse_extraction, ExtractionResult};
use pairproof::llm::{LlmError, MetaClient};

use crate::common_mod;
use common_mod::easy_five as easy_questions;

struct TestDir(PathBuf);

impl TestDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "pairproof-ingest-pipeline-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create test dir");
        Self(dir)
    }
    fn path(&self, file: &str) -> PathBuf {
        self.0.join(file)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn docs() -> ExpertDocuments {
    prepare_documents("Backend role", "team notes").expect("prepare docs")
}

fn hand_built(
    proposed_language: Option<Language>,
    unsupported_language: Option<String>,
) -> ExtractionResult {
    ExtractionResult {
        topics: vec!["array".to_string()],
        level: "mid".to_string(),
        proposed_language,
        unsupported_language,
        language_reason: "reason".to_string(),
    }
}

fn session(language: Language) -> Session {
    Session::new("s1".to_string(), language, easy_questions(), 0).expect("session")
}

fn proposed_context(language: LanguageProposal) -> ProposedContext {
    ProposedContext {
        topics: vec!["array".to_string(), "graph".to_string()],
        level: "mid".to_string(),
        language,
        language_reason: "from jd".to_string(),
        jd_truncated: false,
        additional_truncated: false,
    }
}

#[test]
fn propose_maps_jd_language_win() {
    let json = r#"{"topics":["array","graph"],"level":"senior","language":"Python","language_reason":"required by the job description"}"#;
    let result = parse_extraction(json).expect("parse");
    let proposed = propose_from_extraction(&docs(), result).expect("propose");
    assert_eq!(proposed.topics, ["array", "graph"]);
    assert_eq!(proposed.level, "senior");
    assert_eq!(
        proposed.language,
        LanguageProposal::Proposed(Language::Python)
    );
    assert_eq!(proposed.language.proposed(), Some(Language::Python));
    assert_eq!(proposed.language.unsupported(), None);
    assert_eq!(proposed.language_reason, "required by the job description");
    assert!(!proposed.jd_truncated);
    assert!(!proposed.additional_truncated);
}

#[test]
fn propose_maps_additional_fallback() {
    let json = r#"{"topics":["array"],"level":"mid","language":"JavaScript","language_reason":"named in additional notes"}"#;
    let result = parse_extraction(json).expect("parse");
    let proposed = propose_from_extraction(&docs(), result).expect("propose");
    assert_eq!(
        proposed.language,
        LanguageProposal::Proposed(Language::JavaScript)
    );
    assert_eq!(proposed.language.proposed(), Some(Language::JavaScript));
    assert_eq!(proposed.language.unsupported(), None);
    assert_eq!(proposed.topics, ["array"]);
    assert_eq!(proposed.level, "mid");
}

#[test]
fn propose_defaults_to_python_when_neither_specifies() {
    let json =
        r#"{"topics":["array"],"level":"mid","language":"","language_reason":"Python default"}"#;
    let parsed = parse_extraction(json).expect("parse empty language");
    let from_parse = propose_from_extraction(&docs(), parsed).expect("propose parsed");
    assert_eq!(
        from_parse.language,
        LanguageProposal::Proposed(Language::Python)
    );
    let hand = hand_built(None, None);
    let from_hand = propose_from_extraction(&docs(), hand).expect("propose hand-built");
    assert_eq!(
        from_hand.language,
        LanguageProposal::Proposed(Language::Python)
    );
    assert_eq!(from_hand.language.proposed(), Some(Language::Python));
    assert_eq!(from_hand.language.unsupported(), None);
    // Blank unsupported name alone also falls through to the default.
    let blank = hand_built(None, Some("   ".to_string()));
    let from_blank = propose_from_extraction(&docs(), blank).expect("propose blank");
    assert_eq!(
        from_blank.language,
        LanguageProposal::Proposed(Language::Python)
    );
}

#[test]
fn propose_surfaces_unsupported_language_not_defaulted() {
    let json = r#"{"topics":["array"],"level":"mid","language":"Java","language_reason":"jd"}"#;
    let parsed = parse_extraction(json).expect("parse java");
    let from_parse = propose_from_extraction(&docs(), parsed).expect("propose parsed");
    assert_eq!(
        from_parse.language,
        LanguageProposal::Unsupported("Java".to_string())
    );
    assert_eq!(from_parse.language.proposed(), None);
    assert_eq!(from_parse.language.unsupported(), Some("Java"));

    let contradictory = hand_built(Some(Language::Python), Some("Go".to_string()));
    let from_hand = propose_from_extraction(&docs(), contradictory).expect("propose hand");
    assert_eq!(
        from_hand.language,
        LanguageProposal::Unsupported("Go".to_string())
    );
    assert_eq!(from_hand.language.proposed(), None);
    assert_eq!(from_hand.language.unsupported(), Some("Go"));

    // Surrounding whitespace on the unsupported name is trimmed.
    let padded = hand_built(Some(Language::JavaScript), Some("  Rust  ".to_string()));
    let from_padded = propose_from_extraction(&docs(), padded).expect("propose padded");
    assert_eq!(from_padded.language.unsupported(), Some("Rust"));
}

#[test]
fn propose_rejects_empty_topics_and_blank_level() {
    let empty_topics = ExtractionResult {
        topics: vec![],
        level: "mid".to_string(),
        proposed_language: Some(Language::Python),
        unsupported_language: None,
        language_reason: "r".to_string(),
    };
    let err = propose_from_extraction(&docs(), empty_topics).expect_err("empty topics");
    assert!(matches!(err, IngestError::EmptyTopics), "got {err:?}");

    let blank_level = ExtractionResult {
        topics: vec!["array".to_string()],
        level: "  ".to_string(),
        proposed_language: Some(Language::Python),
        unsupported_language: None,
        language_reason: "r".to_string(),
    };
    let err = propose_from_extraction(&docs(), blank_level).expect_err("blank level");
    assert!(matches!(err, IngestError::EmptyLevel), "got {err:?}");

    // A padded non-blank level is stored trimmed.
    let padded_level = ExtractionResult {
        topics: vec!["array".to_string()],
        level: "  senior  ".to_string(),
        proposed_language: Some(Language::Python),
        unsupported_language: None,
        language_reason: "r".to_string(),
    };
    let ok = propose_from_extraction(&docs(), padded_level).expect("padded level");
    assert_eq!(ok.level, "senior");
}

#[test]
fn propose_propagates_truncation_flags() {
    let long_jd = "x".repeat(MAX_TEXT_CHARS + 1);
    let truncated_docs = prepare_documents(&long_jd, "short notes").expect("prepare long");
    assert!(truncated_docs.jd.truncated);
    let proposed =
        propose_from_extraction(&truncated_docs, hand_built(Some(Language::Python), None))
            .expect("propose");
    assert!(proposed.jd_truncated);
    assert!(!proposed.additional_truncated);
}

#[test]
fn propose_rejects_all_blank_topics() {
    let mut blank = hand_built(Some(Language::Python), None);
    blank.topics = vec!["  ".into(), "".into()];
    let err = propose_from_extraction(&docs(), blank).expect_err("all blank topics");
    assert!(matches!(err, IngestError::EmptyTopics), "got {err:?}");

    let mut mixed = hand_built(Some(Language::Python), None);
    mixed.topics = vec!["array".into(), " ".into()];
    let proposed = propose_from_extraction(&docs(), mixed).expect("propose mixed");
    assert_eq!(proposed.topics, vec!["array".to_string(), " ".to_string()]);
}

#[tokio::test]
async fn propose_context_rejects_blank_jd_without_api_call() {
    let blank_docs = ExpertDocuments {
        jd: BoundedText {
            text: "   ".to_string(),
            truncated: false,
            original_chars: 3,
        },
        additional: BoundedText {
            text: String::new(),
            truncated: false,
            original_chars: 0,
        },
    };
    let client = MetaClient::new("http://127.0.0.1:9/v1", "k");
    let err = propose_context(&client, &blank_docs).await;
    assert!(
        matches!(err, Err(IngestError::EmptyJobDescription)),
        "got {err:?}"
    );
}

#[tokio::test]
async fn propose_context_unreachable_api_is_typed_error() {
    let client = MetaClient::new("http://127.0.0.1:9/v1", "k");
    let err = propose_context(&client, &docs()).await;
    assert!(
        matches!(err, Err(IngestError::Llm(LlmError::Api(_)))),
        "got {err:?}"
    );
}

#[test]
fn write_context_summary_writes_context_json() {
    let dir = TestDir::new("write-context");
    let layout =
        ArtifactLayout::create(dir.path("root"), "s1", Language::JavaScript).expect("layout");
    let sess = session(Language::JavaScript);
    let proposed = proposed_context(LanguageProposal::Proposed(Language::Python));
    let summary = write_context_summary(&layout, &sess, &proposed).expect("write summary");
    assert_eq!(summary.language, Language::JavaScript);
    assert_eq!(summary.session_id, "s1");
    assert_eq!(summary.topics, ["array", "graph"]);
    assert_eq!(summary.level, "mid");
    assert_eq!(summary.questions.len(), 5);
    assert!(layout.context_path().is_file());
    let text = fs::read_to_string(layout.context_path()).expect("read context");
    let back: ContextSummary = serde_json::from_str(&text).expect("parse context");
    assert_eq!(back, summary);
}

#[test]
fn write_context_summary_refuses_unsupported() {
    let dir = TestDir::new("refuse-unsupported");
    let layout = ArtifactLayout::create(dir.path("root"), "s1", Language::Python).expect("layout");
    let sess = session(Language::Python);
    let proposed = proposed_context(LanguageProposal::Unsupported("Java".to_string()));
    let err = write_context_summary(&layout, &sess, &proposed).expect_err("unsupported");
    assert!(
        matches!(err, IngestError::UnsupportedLanguage(ref n) if n == "Java"),
        "got {err:?}"
    );
    assert!(!layout.context_path().exists());
}

#[test]
fn write_context_summary_surfaces_artifact_error() {
    let dir = TestDir::new("artifact-error");
    let layout = ArtifactLayout::create(dir.path("root"), "s1", Language::Python).expect("layout");
    let sess = session(Language::Python);
    let proposed = proposed_context(LanguageProposal::Proposed(Language::Python));
    fs::remove_dir_all(layout.session_dir()).expect("remove session dir");
    let err = write_context_summary(&layout, &sess, &proposed).expect_err("artifact error");
    assert!(matches!(err, IngestError::Artifact(_)), "got {err:?}");
    assert!(!layout.context_path().exists());
}

#[test]
fn raw_documents_gone_after_cleanup_but_summary_stays() {
    let dir = TestDir::new("cleanup-summary");
    let layout = ArtifactLayout::create(dir.path("root"), "s1", Language::Python).expect("layout");
    let prepared = docs();
    store_raw_documents(&layout, &prepared).expect("store raw");
    assert!(layout.raw_dir().join("jd.txt").is_file());
    let sess = session(Language::Python);
    let proposed = proposed_context(LanguageProposal::Proposed(Language::Python));
    write_context_summary(&layout, &sess, &proposed).expect("write summary");
    assert!(layout.context_path().is_file());
    layout.delete_raw_dir().expect("delete raw");
    assert!(!layout.raw_dir().exists());
    assert!(layout.context_path().is_file());
    let text = fs::read_to_string(layout.context_path()).expect("read context");
    let back: ContextSummary = serde_json::from_str(&text).expect("parse context");
    assert_eq!(back.topics, ["array", "graph"]);
}

#[test]
fn propose_maps_sql_language() {
    let json = r#"{"topics":["database"],"level":"mid","language":"SQL","language_reason":"database role"}"#;
    let result = parse_extraction(json).expect("parse");
    let proposed = propose_from_extraction(&docs(), result).expect("propose");
    assert_eq!(proposed.language, LanguageProposal::Proposed(Language::Sql));
    assert_eq!(proposed.language.proposed(), Some(Language::Sql));
}

#[test]
fn write_context_summary_with_sql_session() {
    let dir = TestDir::new("write-context-sql");
    let layout = ArtifactLayout::create(dir.path("root"), "s1", Language::Sql).expect("layout");
    let slugs = [
        "combine-two-tables",
        "employees-earning-more-than-their-managers",
        "duplicate-emails",
        "customers-who-never-order",
        "delete-duplicate-emails",
    ];
    let questions = slugs
        .iter()
        .map(|slug| QuestionRef {
            title_slug: slug.to_string(),
            title: slug.to_string(),
            difficulty: Difficulty::Easy,
            acceptance_pct: None,
        })
        .collect();
    let sess = Session::new("s1".to_string(), Language::Sql, questions, 0).expect("session");
    let proposed = proposed_context(LanguageProposal::Proposed(Language::Sql));
    let summary = write_context_summary(&layout, &sess, &proposed).expect("write summary");
    assert_eq!(summary.language, Language::Sql);
    assert!(layout.context_path().is_file());
    let text = fs::read_to_string(layout.context_path()).expect("read context");
    let back: ContextSummary = serde_json::from_str(&text).expect("parse context");
    assert_eq!(back, summary);
}
