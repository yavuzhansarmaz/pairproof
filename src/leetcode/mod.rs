//! Live LeetCode GraphQL catalog and question fetching.

pub mod selection;

pub use selection::{auto_pick_easy, expert_pool, pick_random, select_questions, SelectionError};

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

const GRAPHQL_URL: &str = "https://leetcode.com/graphql";
const USER_AGENT: &str = concat!("pairproof/", env!("CARGO_PKG_VERSION"));

const CATALOG_QUERY: &str = r#"
query problemsetQuestionList($categorySlug: String, $limit: Int, $skip: Int, $filters: QuestionListFilterInput) {
  problemsetQuestionList: questionList(categorySlug: $categorySlug limit: $limit skip: $skip filters: $filters) {
    total: totalNum
    questions: data { acRate difficulty isPaidOnly title titleSlug topicTags { name slug } }
  }
}"#;

const QUESTION_QUERY: &str = r#"
query questionData($titleSlug: String!) {
  question(titleSlug: $titleSlug) {
    questionId title titleSlug difficulty content exampleTestcases isPaidOnly
    topicTags { name slug }
    codeSnippets { lang langSlug code }
  }
}"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Difficulty {
    Easy,
    Medium,
    Hard,
}

impl Difficulty {
    /// Returns the GraphQL filter value for this difficulty.
    pub fn as_filter(self) -> &'static str {
        match self {
            Difficulty::Easy => "EASY",
            Difficulty::Medium => "MEDIUM",
            Difficulty::Hard => "HARD",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopicTag {
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    pub title: String,
    pub title_slug: String,
    pub difficulty: String,
    pub ac_rate: f64,
    pub is_paid_only: bool,
    pub topic_tags: Vec<TopicTag>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CatalogPage {
    /// Server-reported match count; includes premium questions.
    pub total_reported: u64,
    /// Free questions only; premium entries are removed.
    pub questions: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeSnippet {
    pub lang: String,
    pub lang_slug: String,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub question_id: String,
    pub title: String,
    pub title_slug: String,
    pub difficulty: String,
    pub content: String,
    pub example_testcases: String,
    pub is_paid_only: bool,
    pub topic_tags: Vec<TopicTag>,
    pub code_snippets: Vec<CodeSnippet>,
}

impl Question {
    /// Returns the starter code for a language slug such as "python3".
    pub fn snippet_for(&self, lang_slug: &str) -> Option<&str> {
        self.code_snippets
            .iter()
            .find(|s| s.lang_slug == lang_slug)
            .map(|s| s.code.as_str())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LeetCodeError {
    #[error("leetcode request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("leetcode returned an error: {0}")]
    GraphQl(String),
    #[error("question not found: {0}")]
    NotFound(String),
    #[error("question content requires LeetCode Premium: {0}")]
    PremiumOnly(String),
    #[error("unexpected leetcode response shape")]
    BadResponse,
}

#[derive(Debug, Deserialize)]
struct GraphQlResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GraphQlErrorItem>>,
}

#[derive(Debug, Deserialize)]
struct GraphQlErrorItem {
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogData {
    problemset_question_list: CatalogList,
}

#[derive(Debug, Deserialize)]
struct CatalogList {
    total: u64,
    questions: Vec<CatalogEntry>,
}

#[derive(Debug, Deserialize)]
struct QuestionData {
    question: Option<QuestionWire>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QuestionWire {
    question_id: String,
    title: String,
    title_slug: String,
    difficulty: String,
    content: Option<String>,
    example_testcases: Option<String>,
    is_paid_only: bool,
    topic_tags: Vec<TopicTag>,
    code_snippets: Option<Vec<CodeSnippet>>,
}

#[derive(Debug, Clone)]
pub struct LeetCodeClient {
    http: reqwest::Client,
    base_url: String,
}

impl Default for LeetCodeClient {
    /// Creates a client for the public LeetCode GraphQL endpoint.
    fn default() -> Self {
        Self::new(GRAPHQL_URL)
    }
}

impl LeetCodeClient {
    /// Creates a client for the given GraphQL endpoint URL.
    pub fn new(base_url: &str) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(20))
            .build()
            .expect("http client builds");
        Self {
            http,
            base_url: base_url.to_string(),
        }
    }

    /// Lists free questions for a difficulty and any of the given topic slugs.
    pub async fn list_questions(
        &self,
        difficulty: Difficulty,
        topics: &[&str],
        limit: u32,
        skip: u32,
    ) -> Result<CatalogPage, LeetCodeError> {
        let mut filters = serde_json::json!({ "difficulty": difficulty.as_filter() });
        if !topics.is_empty() {
            filters["tags"] = serde_json::json!(topics);
        }
        let variables = serde_json::json!({
            "categorySlug": "all-code-essentials",
            "limit": limit,
            "skip": skip,
            "filters": filters,
        });
        let data: CatalogData = self.graphql(CATALOG_QUERY, variables).await?;
        let list = data.problemset_question_list;
        Ok(CatalogPage {
            total_reported: list.total,
            questions: list
                .questions
                .into_iter()
                .filter(|q| !q.is_paid_only)
                .collect(),
        })
    }

    /// Fetches one free question with its statement and starter snippets.
    pub async fn fetch_question(&self, title_slug: &str) -> Result<Question, LeetCodeError> {
        let variables = serde_json::json!({ "titleSlug": title_slug });
        let data: QuestionData = self.graphql(QUESTION_QUERY, variables).await?;
        let wire = data
            .question
            .ok_or_else(|| LeetCodeError::NotFound(title_slug.to_string()))?;
        if wire.is_paid_only {
            return Err(LeetCodeError::PremiumOnly(title_slug.to_string()));
        }
        let content = wire
            .content
            .ok_or_else(|| LeetCodeError::PremiumOnly(title_slug.to_string()))?;
        Ok(Question {
            question_id: wire.question_id,
            title: wire.title,
            title_slug: wire.title_slug,
            difficulty: wire.difficulty,
            content,
            example_testcases: wire.example_testcases.unwrap_or_default(),
            is_paid_only: wire.is_paid_only,
            topic_tags: wire.topic_tags,
            code_snippets: wire.code_snippets.unwrap_or_default(),
        })
    }

    /// Posts one GraphQL operation and decodes its data payload.
    async fn graphql<T: serde::de::DeserializeOwned>(
        &self,
        query: &str,
        variables: serde_json::Value,
    ) -> Result<T, LeetCodeError> {
        let mut body = BTreeMap::new();
        body.insert("query", serde_json::json!(query));
        body.insert("variables", variables);
        let response: GraphQlResponse<T> = self
            .http
            .post(&self.base_url)
            .header("Referer", "https://leetcode.com/problemset/")
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if let Some(errors) = response.errors {
            let message = errors
                .into_iter()
                .map(|e| e.message)
                .collect::<Vec<_>>()
                .join("; ");
            return Err(LeetCodeError::GraphQl(message));
        }
        response.data.ok_or(LeetCodeError::BadResponse)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a question with python3 and javascript snippets.
    fn sample_question() -> Question {
        Question {
            question_id: "1".into(),
            title: "Two Sum".into(),
            title_slug: "two-sum".into(),
            difficulty: "Easy".into(),
            content: "<p>statement</p>".into(),
            example_testcases: "[2,7]\n9".into(),
            is_paid_only: false,
            topic_tags: vec![TopicTag {
                name: "Array".into(),
                slug: "array".into(),
            }],
            code_snippets: vec![
                CodeSnippet {
                    lang: "Python3".into(),
                    lang_slug: "python3".into(),
                    code: "class Solution: pass".into(),
                },
                CodeSnippet {
                    lang: "JavaScript".into(),
                    lang_slug: "javascript".into(),
                    code: "var twoSum = function() {};".into(),
                },
            ],
        }
    }

    #[test]
    fn difficulty_filters_match_graphql_values() {
        assert_eq!(Difficulty::Easy.as_filter(), "EASY");
        assert_eq!(Difficulty::Medium.as_filter(), "MEDIUM");
        assert_eq!(Difficulty::Hard.as_filter(), "HARD");
    }

    #[test]
    fn snippet_lookup_finds_supported_languages() {
        let question = sample_question();
        assert_eq!(
            question.snippet_for("python3"),
            Some("class Solution: pass")
        );
        assert!(question.snippet_for("javascript").is_some());
        assert_eq!(question.snippet_for("rust"), None);
    }

    #[test]
    fn catalog_entries_deserialize_from_camel_case() {
        let raw = r#"{"title":"Two Sum","titleSlug":"two-sum","difficulty":"Easy",
            "acRate":58.06,"isPaidOnly":false,
            "topicTags":[{"name":"Array","slug":"array"}]}"#;
        let entry: CatalogEntry = serde_json::from_str(raw).expect("entry parses");
        assert_eq!(entry.title_slug, "two-sum");
        assert!(!entry.is_paid_only);
        assert_eq!(entry.topic_tags[0].slug, "array");
    }
}
