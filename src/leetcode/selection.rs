//! Randomized selection of free LeetCode questions from topic pools.

use super::{CatalogEntry, CatalogPage, Difficulty, LeetCodeClient, LeetCodeError};
use crate::core::types::QuestionRef;
use rand::seq::SliceRandom;
use rand::Rng;
use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;

pub const MAX_PAGES: u32 = 5;
pub const PAGE_LIMIT: u32 = 100;

#[derive(Debug, thiserror::Error)]
pub enum SelectionError {
    #[error("leetcode api error: {0}")]
    Api(#[from] LeetCodeError),
    #[error("not enough questions: needed {needed}, available {available}")]
    NotEnoughQuestions { needed: usize, available: usize },
}

/// Boxed catalog-page read from any pool source.
pub type PoolFuture<'a> =
    Pin<Box<dyn Future<Output = Result<CatalogPage, LeetCodeError>> + Send + 'a>>;

/// Source of catalog pages for selection (live client or test fake).
pub trait PoolClient: Send + Sync {
    /// Lists one catalog page; mirrors `LeetCodeClient::list_questions`.
    fn list_questions<'a>(
        &'a self,
        difficulty: Difficulty,
        topics: &'a [&'a str],
        limit: u32,
        skip: u32,
    ) -> PoolFuture<'a>;
}

impl PoolClient for LeetCodeClient {
    /// Delegates to the live GraphQL catalog.
    fn list_questions<'a>(
        &'a self,
        difficulty: Difficulty,
        topics: &'a [&'a str],
        limit: u32,
        skip: u32,
    ) -> PoolFuture<'a> {
        Box::pin(async move {
            LeetCodeClient::list_questions(self, difficulty, topics, limit, skip).await
        })
    }
}

/// Report of which tag set satisfied a fallback selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FallbackReport {
    /// Tags of the winning pool (`[]` means unfiltered).
    pub tags_used: Vec<String>,
    /// True when the winning pool dropped at least one topic tag.
    pub widened: bool,
}

/// Maps a catalog difficulty string to [`Difficulty`].
pub fn difficulty_of(entry: &CatalogEntry) -> Option<Difficulty> {
    let value = entry.difficulty.trim().to_ascii_lowercase();
    match value.as_str() {
        "easy" => Some(Difficulty::Easy),
        "medium" => Some(Difficulty::Medium),
        "hard" => Some(Difficulty::Hard),
        _ => None,
    }
}

/// Converts a free, known-difficulty catalog entry to a session reference.
pub fn to_question_ref(entry: &CatalogEntry) -> Option<QuestionRef> {
    if entry.is_paid_only {
        return None;
    }
    let difficulty = difficulty_of(entry)?;
    Some(QuestionRef {
        title_slug: entry.title_slug.clone(),
        title: entry.title.clone(),
        difficulty,
        acceptance_pct: if entry.ac_rate.is_finite() {
            Some(entry.ac_rate.round().clamp(0.0, 100.0) as u32)
        } else {
            None
        },
    })
}

/// Picks up to `count` distinct valid entries at random order.
pub fn pick_random(
    entries: &[CatalogEntry],
    count: usize,
    exclude: &BTreeSet<String>,
    rng: &mut impl Rng,
) -> Vec<QuestionRef> {
    let mut seen = BTreeSet::new();
    let mut survivors: Vec<QuestionRef> = Vec::new();
    for entry in entries {
        if !seen.insert(entry.title_slug.clone()) || exclude.contains(&entry.title_slug) {
            continue;
        }
        if let Some(question) = to_question_ref(entry) {
            survivors.push(question);
        }
    }
    survivors.shuffle(rng);
    survivors.truncate(count.min(survivors.len()));
    survivors
}

/// Fetches the deduped topic pool for expert selection, in catalog order.
pub async fn expert_pool(
    client: &LeetCodeClient,
    difficulty: Difficulty,
    topics: &[&str],
    exclude: &BTreeSet<String>,
) -> Result<Vec<CatalogEntry>, SelectionError> {
    pooled_from(client, difficulty, topics, exclude).await
}

/// Fetches the deduped pool through any catalog source, in catalog order.
async fn pooled_from(
    client: &dyn PoolClient,
    difficulty: Difficulty,
    topics: &[&str],
    exclude: &BTreeSet<String>,
) -> Result<Vec<CatalogEntry>, SelectionError> {
    let mut seen = BTreeSet::new();
    let mut pool: Vec<CatalogEntry> = Vec::new();
    for page in 0..MAX_PAGES {
        let skip = page * PAGE_LIMIT;
        let fetched = client
            .list_questions(difficulty, topics, PAGE_LIMIT, skip)
            .await?;
        let page_len = fetched.questions.len();
        for entry in fetched.questions {
            if seen.insert(entry.title_slug.clone()) && !exclude.contains(&entry.title_slug) {
                pool.push(entry);
            }
        }
        if (page_len as u32) < PAGE_LIMIT {
            break;
        }
    }
    Ok(pool)
}

/// Tag sets tried in order: full set, then shorter prefixes
/// (trailing tags dropped first), then one unfiltered attempt.
pub fn fallback_tag_sets<'a>(topics: &[&'a str]) -> Vec<Vec<&'a str>> {
    let mut sets = Vec::with_capacity(topics.len() + 1);
    for len in (1..=topics.len()).rev() {
        sets.push(topics[..len].to_vec());
    }
    sets.push(Vec::new());
    sets
}

/// Selects `count` questions, shrinking tags while the pool is short.
/// Only `NotEnoughQuestions` falls through; API errors abort at once.
pub async fn select_questions_with_fallback(
    client: &dyn PoolClient,
    difficulty: Difficulty,
    topics: &[&str],
    count: usize,
    exclude: &BTreeSet<String>,
    rng: &mut impl Rng,
) -> Result<(Vec<QuestionRef>, FallbackReport), SelectionError> {
    let mut available = 0;
    for tags in fallback_tag_sets(topics) {
        let pool = pooled_from(client, difficulty, &tags, exclude).await?;
        let picked = pick_random(&pool, count, &BTreeSet::new(), rng);
        if picked.len() >= count {
            let report = FallbackReport {
                tags_used: tags.iter().map(|tag| tag.to_string()).collect(),
                widened: tags.len() < topics.len(),
            };
            return Ok((picked, report));
        }
        available = picked.len();
    }
    Err(SelectionError::NotEnoughQuestions {
        needed: count,
        available,
    })
}

/// Selects `count` questions at random from the topic pool.
pub async fn select_questions(
    client: &LeetCodeClient,
    difficulty: Difficulty,
    topics: &[&str],
    count: usize,
    exclude: &BTreeSet<String>,
    rng: &mut impl Rng,
) -> Result<Vec<QuestionRef>, SelectionError> {
    let pool = expert_pool(client, difficulty, topics, exclude).await?;
    let picked = pick_random(&pool, count, &BTreeSet::new(), rng);
    if picked.len() < count {
        return Err(SelectionError::NotEnoughQuestions {
            needed: count,
            available: picked.len(),
        });
    }
    Ok(picked)
}

/// Auto-picks the 5 easy questions that open a session.
/// Falls back through shorter tag sets like expert selection.
pub async fn auto_pick_easy(
    client: &dyn PoolClient,
    topics: &[&str],
    rng: &mut impl Rng,
) -> Result<Vec<QuestionRef>, SelectionError> {
    let (picked, _) =
        select_questions_with_fallback(client, Difficulty::Easy, topics, 5, &BTreeSet::new(), rng)
            .await?;
    Ok(picked)
}
