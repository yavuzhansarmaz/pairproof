//! Question statement fetch and approach generation for the current slot.
//! Hidden approach quality and expected tokens never leave this module
//! except as the single locked value passed to the core at selection.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;

use crate::core::{ApproachChoice, CoreError, Language};
use crate::leetcode::{LeetCodeClient, Question};
use crate::llm::approaches::{generate_approaches, grade_other_approach};
use crate::llm::MetaClient;
use crate::orchestration::error::OrchestratorError;
use crate::orchestration::session::Orchestrator;
use crate::sandbox::TestOutcome;
use crate::scoring::{ApproachGrade, ApproachOptionView, ApproachQuality, ApproachSet};
use rand::SeedableRng as _;

/// Errors surfaced by the question, coding, and chat pipeline.
/// Payload strings are redacted summaries and never carry an API key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PipelineError {
    /// Wraps an orchestrator or core session error unchanged.
    #[error(transparent)]
    Core(#[from] OrchestratorError),
    /// Redacted large-language-model failure summary.
    #[error("llm error: {0}")]
    Llm(String),
    /// Redacted question-fetch failure summary.
    #[error("fetch error: {0}")]
    Fetch(String),
    /// Redacted sandbox failure summary.
    #[error("sandbox error: {0}")]
    Sandbox(String),
    /// No approach set has been generated for the current question.
    #[error("no approaches generated for the current question")]
    NoApproaches,
    /// The requested preset is outside the generated approach set.
    #[error("invalid preset")]
    InvalidPreset,
    /// A chat submit is already in flight for this pipeline.
    #[error("chat submit already in flight")]
    ChatBusy,
    /// The chat transport is unavailable.
    #[error("chat unavailable")]
    ChatUnavailable,
}

impl From<CoreError> for PipelineError {
    fn from(err: CoreError) -> Self {
        Self::Core(err.into())
    }
}

/// A fetched question statement plus starter code for the session language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedStatement {
    pub statement: String,
    pub starter_code: String,
}

/// Raw sandbox test outcome stored verbatim; no counts are derived from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestRunRecord {
    pub index: usize,
    pub at_ms: u64,
    pub outcome: TestOutcome,
}

/// Boxed pipeline future returned by every narrow transport trait.
pub type PipelineFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, PipelineError>> + Send + 'a>>;

/// Narrow boundary for fetching one question statement.
pub trait StatementFetcher: Send + Sync {
    /// Fetches the statement and starter code for `slug` and `language`.
    fn fetch_statement<'a>(
        &'a self,
        slug: &'a str,
        language: Language,
    ) -> PipelineFuture<'a, FetchedStatement>;
}

/// Narrow boundary for Phase 4 approach generation and grading.
pub trait ApproachGenerator: Send + Sync {
    /// Generates the four graded approaches for a question statement.
    fn generate<'a>(
        &'a self,
        statement: &'a str,
        language: Language,
    ) -> PipelineFuture<'a, ApproachSet>;
    /// Grades a candidate-written `Other` approach before any selection.
    fn grade_other<'a>(
        &'a self,
        statement: &'a str,
        text: &'a str,
    ) -> PipelineFuture<'a, ApproachGrade>;
}

/// Extracts pipeline data from a fetched question for `language`.
/// A missing snippet yields empty starter code, not an error.
pub fn fetched_from_question(question: &Question, language: Language) -> FetchedStatement {
    FetchedStatement {
        statement: question.content.clone(),
        starter_code: question
            .snippet_for(language.leetcode_slug())
            .unwrap_or("")
            .to_string(),
    }
}

/// Production statement fetcher backed by the real LeetCode client.
#[derive(Debug)]
pub struct LeetcodeStatementFetcher {
    client: LeetCodeClient,
}

impl LeetcodeStatementFetcher {
    /// Creates a fetcher wrapping `client`.
    pub fn new(client: LeetCodeClient) -> Self {
        Self { client }
    }
}

impl StatementFetcher for LeetcodeStatementFetcher {
    fn fetch_statement<'a>(
        &'a self,
        slug: &'a str,
        language: Language,
    ) -> PipelineFuture<'a, FetchedStatement> {
        Box::pin(async move {
            let question = self
                .client
                .fetch_question(slug)
                .await
                .map_err(|e| PipelineError::Fetch(e.to_string()))?;
            Ok(fetched_from_question(&question, language))
        })
    }
}

/// Production approach generator backed by the real Meta client.
#[derive(Debug)]
pub struct MetaApproachGenerator {
    client: MetaClient,
}

impl MetaApproachGenerator {
    /// Creates a generator wrapping `client`.
    pub fn new(client: MetaClient) -> Self {
        Self { client }
    }
}

impl ApproachGenerator for MetaApproachGenerator {
    fn generate<'a>(
        &'a self,
        statement: &'a str,
        language: Language,
    ) -> PipelineFuture<'a, ApproachSet> {
        Box::pin(async move {
            let mut rng = rand::rngs::StdRng::from_entropy();
            let outcome = generate_approaches(&self.client, statement, language, &mut rng)
                .await
                .map_err(|e| PipelineError::Llm(e.to_string()))?;
            Ok(outcome.value)
        })
    }

    fn grade_other<'a>(
        &'a self,
        statement: &'a str,
        text: &'a str,
    ) -> PipelineFuture<'a, ApproachGrade> {
        Box::pin(async move {
            let outcome = grade_other_approach(&self.client, statement, text)
                .await
                .map_err(|e| PipelineError::Llm(e.to_string()))?;
            Ok(outcome.value)
        })
    }
}

/// Pipeline state for the current question: statements, approaches, code.
#[derive(Debug)]
pub struct QuestionPipeline {
    pub(crate) approaches: Option<ApproachSet>,
    pub(crate) approaches_index: Option<usize>,
    pub(crate) statements: HashMap<String, FetchedStatement>,
    pub(crate) codes: HashMap<usize, String>,
    pub(crate) test_records: Vec<TestRunRecord>,
    pub(crate) chat_busy: AtomicBool,
    pub(crate) selected_qualities: BTreeMap<usize, Vec<ApproachQuality>>,
    pub(crate) other_grades: BTreeMap<usize, ApproachGrade>,
}

impl Default for QuestionPipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl QuestionPipeline {
    /// Creates an empty pipeline with no statement or approaches loaded.
    pub fn new() -> Self {
        Self {
            approaches: None,
            approaches_index: None,
            statements: HashMap::new(),
            codes: HashMap::new(),
            test_records: Vec::new(),
            chat_busy: AtomicBool::new(false),
            selected_qualities: BTreeMap::new(),
            other_grades: BTreeMap::new(),
        }
    }

    /// Fetches and stores the statement for the orchestrator's current
    /// question. A failure mutates neither the session nor stored state.
    pub async fn load_current_statement(
        &mut self,
        orch: &Orchestrator,
        fetcher: &dyn StatementFetcher,
    ) -> Result<FetchedStatement, PipelineError> {
        let slug = orch
            .current_question()
            .map(|q| q.title_slug.clone())
            .ok_or(CoreError::NoActiveQuestion)?;
        let fetched = fetcher
            .fetch_statement(&slug, orch.session().language())
            .await?;
        self.statements.insert(slug, fetched.clone());
        Ok(fetched)
    }

    /// Returns the stored statement for the current question, if loaded.
    pub fn current_statement(&self, orch: &Orchestrator) -> Option<FetchedStatement> {
        let slug = orch.current_question()?.title_slug.as_str();
        self.statements.get(slug).cloned()
    }

    /// Returns the stored statement text for the current question.
    pub(crate) fn statement_text(&self, orch: &Orchestrator) -> Result<String, PipelineError> {
        let slug = orch
            .current_question()
            .map(|q| q.title_slug.clone())
            .ok_or(CoreError::NoActiveQuestion)?;
        self.statements
            .get(&slug)
            .map(|f| f.statement.clone())
            .ok_or_else(|| PipelineError::Fetch(format!("statement not loaded for {slug}")))
    }

    /// Generates and privately stores approaches for the current question.
    pub async fn generate_current_approaches(
        &mut self,
        orch: &Orchestrator,
        generator: &dyn ApproachGenerator,
    ) -> Result<Vec<ApproachOptionView>, PipelineError> {
        let index = orch.current_index().ok_or(CoreError::NoActiveQuestion)?;
        let statement = self.statement_text(orch)?;
        let set = generator
            .generate(&statement, orch.session().language())
            .await?;
        let views = set.candidate_options();
        self.approaches = Some(set);
        self.approaches_index = Some(index);
        Ok(views)
    }

    /// Snapshot of the stored approach set with its slot index. The
    /// clone stays valid after the live set is cleared, so a shadow
    /// evaluation triggered from it is never tied to the live set.
    pub fn approaches_snapshot(&self) -> Option<(usize, ApproachSet)> {
        Some((self.approaches_index?, self.approaches.clone()?))
    }

    /// Cloned `Other` grade stashed for `index` at selection, if any.
    pub fn other_grade(&self, index: usize) -> Option<ApproachGrade> {
        self.other_grades.get(&index).cloned()
    }

    /// Takes the `Other` grade stashed for `index`, if any.
    pub fn take_other_grade(&mut self, index: usize) -> Option<ApproachGrade> {
        self.other_grades.remove(&index)
    }

    /// Every stashed `Other` grade as `(slot, grade)` pairs.
    pub fn other_grades(&self) -> Vec<(usize, ApproachGrade)> {
        self.other_grades
            .iter()
            .map(|(index, grade)| (*index, grade.clone()))
            .collect()
    }

    /// Returns candidate-visible approach options only, hiding quality,
    /// expected tokens, and which option is best.
    pub fn candidate_options(&self) -> Vec<ApproachOptionView> {
        self.approaches
            .as_ref()
            .map(|set| set.candidate_options())
            .unwrap_or_default()
    }

    /// Returns whether an approach set is stored for the current question.
    pub fn has_current_approaches(&self, orch: &Orchestrator) -> bool {
        self.approaches.is_some() && self.approaches_index == orch.current_index()
    }

    /// Hidden quality of the current preset selection, if any.
    fn current_selection_quality(&self, orch: &Orchestrator) -> Option<(usize, ApproachQuality)> {
        let index = orch.current_index()?;
        if self.approaches_index != Some(index) {
            return None;
        }
        let slot = orch.session().current_slot()?;
        match slot.approach {
            Some(ApproachChoice::Preset(preset)) => self
                .approaches
                .as_ref()
                .and_then(|set| set.quality_of(preset))
                .map(|quality| (index, quality)),
            _ => None,
        }
    }

    /// Records the current preset quality before its set is cleared.
    fn record_current_selection(&mut self, selected: Option<(usize, ApproachQuality)>) {
        if let Some((index, quality)) = selected {
            self.selected_qualities
                .entry(index)
                .or_default()
                .push(quality);
        }
    }

    /// Approach qualities in selection order, keyed by slot index.
    /// Selections already cleared (finished, skipped or changed) come
    /// from the recorded history; the still-current selection is read
    /// from its live approach set. `Other` selections carry no set
    /// quality and are omitted.
    pub fn approach_qualities(&self, orch: &Orchestrator) -> BTreeMap<usize, Vec<ApproachQuality>> {
        let mut qualities = self.selected_qualities.clone();
        if let Some((index, quality)) = self.current_selection_quality(orch) {
            qualities.entry(index).or_default().push(quality);
        }
        qualities
    }

    /// Returns the latest stored solution code for a question index.
    pub fn current_code(&self, index: usize) -> Option<&str> {
        self.codes.get(&index).map(String::as_str)
    }

    /// Returns every raw test-run record stored verbatim, in run order.
    pub fn test_records(&self) -> &[TestRunRecord] {
        &self.test_records
    }

    pub(crate) fn clear_approaches(&mut self) {
        self.approaches = None;
        self.approaches_index = None;
    }

    /// Finishes the active question via the orchestrator and clears the
    /// stored approaches so a stale set can never be selected again.
    pub fn finish_question(
        &mut self,
        orch: &mut Orchestrator,
        now_ms: u64,
    ) -> Result<(), PipelineError> {
        let selected = self.current_selection_quality(orch);
        orch.finish_question(now_ms)?;
        self.record_current_selection(selected);
        self.clear_approaches();
        Ok(())
    }

    /// Skips the active question via the orchestrator and clears approaches.
    pub fn skip_question(
        &mut self,
        orch: &mut Orchestrator,
        now_ms: u64,
    ) -> Result<(), PipelineError> {
        let selected = self.current_selection_quality(orch);
        orch.skip_question(now_ms)?;
        self.record_current_selection(selected);
        self.clear_approaches();
        Ok(())
    }

    /// Changes the selected approach via the orchestrator and clears the
    /// stored approaches; a fresh generation is required to reselect.
    pub fn change_approach(
        &mut self,
        orch: &mut Orchestrator,
        now_ms: u64,
    ) -> Result<(), PipelineError> {
        let selected = self.current_selection_quality(orch);
        orch.change_approach(now_ms)?;
        self.record_current_selection(selected);
        self.clear_approaches();
        Ok(())
    }

    /// Explicit counted path: records genuinely parsed passed and failed
    /// counts through the core exactly once. Raw sandbox outcomes carry
    /// no counts and never take this path implicitly.
    pub fn record_counted_test_result(
        &self,
        orch: &mut Orchestrator,
        passed: u32,
        failed: u32,
        now_ms: u64,
    ) -> Result<(), PipelineError> {
        orch.record_test_result(passed, failed, now_ms)?;
        Ok(())
    }
}
