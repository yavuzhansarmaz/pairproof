//! Expert runtime question selection: medium/hard fetching and
//! selection backoff.

use super::build::widened_note;
use super::*;

impl ExpertRuntime {
    /// Commits expert medium and hard picks through the core session.
    pub fn expert_select(
        &mut self,
        medium: Vec<QuestionRef>,
        hard: Vec<QuestionRef>,
        now_ms: u64,
    ) -> Result<(), ExpertRuntimeError> {
        self.orch.set_expert_questions(medium, hard, now_ms)?;
        self.pending_medium = None;
        self.maybe_trigger_load();
        Ok(())
    }

    /// Fetches three real medium questions for the medium screen.
    /// A cached pending set returns without touching the pool again.
    pub async fn fetch_medium(&mut self, now_ms: u64) -> Result<usize, ExpertRuntimeError> {
        if let Some(pending) = &self.pending_medium {
            return Ok(pending.len());
        }
        let client = self
            .pool
            .as_ref()
            .ok_or_else(|| ExpertRuntimeError::Message("question pool unavailable".into()))?;
        let topics: Vec<&str> = self.topics.iter().map(String::as_str).collect();
        let mut rng = rand::thread_rng();
        let exclude: BTreeSet<String> = self
            .orch
            .session()
            .slots()
            .iter()
            .map(|s| s.question.title_slug.clone())
            .collect();
        match select_questions_with_fallback(
            client.as_ref(),
            Difficulty::Medium,
            &topics,
            3,
            &exclude,
            &mut rng,
        )
        .await
        {
            Ok((picked, report)) => {
                let count = picked.len();
                self.pending_medium = Some(picked);
                self.medium_failed_at = None;
                self.selection_note = widened_note(&report);
                Ok(count)
            }
            Err(err) => {
                self.medium_failed_at = Some(now_ms);
                Err(ExpertRuntimeError::Message(err.to_string()))
            }
        }
    }

    /// Fetches two real hard questions and commits the expert set.
    pub async fn complete_selection(&mut self, now_ms: u64) -> Result<(), ExpertRuntimeError> {
        let medium = self.pending_medium.clone().ok_or_else(|| {
            ExpertRuntimeError::Message("medium questions are not selected yet".into())
        })?;
        let client = self
            .pool
            .as_ref()
            .ok_or_else(|| ExpertRuntimeError::Message("question pool unavailable".into()))?;
        let topics: Vec<&str> = self.topics.iter().map(String::as_str).collect();
        let mut exclude: BTreeSet<String> = self
            .orch
            .session()
            .slots()
            .iter()
            .map(|s| s.question.title_slug.clone())
            .collect();
        for question in &medium {
            exclude.insert(question.title_slug.clone());
        }
        let mut rng = rand::thread_rng();
        match select_questions_with_fallback(
            client.as_ref(),
            Difficulty::Hard,
            &topics,
            2,
            &exclude,
            &mut rng,
        )
        .await
        {
            Ok((hard, report)) => {
                self.hard_failed_at = None;
                self.selection_note = widened_note(&report);
                self.expert_select(medium, hard, now_ms)
            }
            Err(err) => {
                self.hard_failed_at = Some(now_ms);
                Err(ExpertRuntimeError::Message(err.to_string()))
            }
        }
    }

    /// Returns false while the screen backs off after a failed attempt.
    /// Pure in `now_ms`: no wall-clock reads, first attempt is immediate.
    pub fn selection_allowed(&self, screen: SelectionScreen, now_ms: u64) -> bool {
        let failed = match screen {
            SelectionScreen::Medium => self.medium_failed_at,
            SelectionScreen::Hard => self.hard_failed_at,
        };
        failed.is_none_or(|at| now_ms.saturating_sub(at) >= SELECTION_RETRY_MS)
    }
    /// Expert-visible note from the last widened selection, if any.
    pub fn selection_note(&self) -> Option<&str> {
        self.selection_note.as_deref()
    }
    /// Overrides the question pool source (test seam for selection).
    pub fn set_pool(&mut self, pool: Arc<dyn PoolClient>) {
        self.pool = Some(pool);
    }
    pub fn set_secret_pusher(
        &mut self,
        pusher: Arc<dyn crate::orchestration::secret_push::SecretPush>,
    ) {
        self.secret_pusher = Some(pusher);
    }
    pub fn set_clock(&mut self, clock: Arc<dyn Clock>) {
        self.clock = Some(clock);
    }
}
