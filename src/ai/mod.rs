//! Meta Model API clients (strong and light) and their prompts.

use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const DEFAULT_BASE_URL: &str = "https://api.meta.ai/v1";
pub const STRONG_MODEL: &str = "muse-wonder-1.0";
pub const LIGHT_MODEL: &str = "muse-spark-1.3";
pub const API_KEY_ENV: &str = "MODEL_API_KEY";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    /// Creates a message with the given role and content.
    pub fn new(role: &str, content: &str) -> Self {
        Self {
            role: role.to_string(),
            content: content.to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatOutcome {
    pub content: String,
    pub usage: Usage,
    pub model: String,
}

#[derive(Debug, Clone, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
}

#[derive(Debug, Clone, Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    model: String,
    choices: Vec<Choice>,
    usage: Option<UsageWire>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: Option<Message>,
}

#[derive(Debug, Deserialize)]
struct UsageWire {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    total_tokens: Option<u64>,
}

impl From<UsageWire> for Usage {
    /// Normalises optional wire usage fields into a totalled usage.
    fn from(wire: UsageWire) -> Self {
        let prompt_tokens = wire.prompt_tokens.unwrap_or(0);
        let completion_tokens = wire.completion_tokens.unwrap_or(0);
        let total_tokens = wire
            .total_tokens
            .unwrap_or(prompt_tokens + completion_tokens);
        Self {
            prompt_tokens,
            completion_tokens,
            total_tokens,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<ModelInfo>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ModelInfo {
    pub id: String,
}

#[derive(Debug, thiserror::Error)]
pub enum MetaApiError {
    #[error("MODEL_API_KEY is not set")]
    MissingApiKey,
    #[error("meta api request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("meta api returned status {status}: {body}")]
    Status { status: u16, body: String },
    #[error("meta api response contained no choices")]
    EmptyChoices,
    #[error("meta api stream ended without content")]
    EmptyStream,
}

/// Parses one SSE data payload into text delta and optional usage.
fn parse_stream_payload(payload: &str) -> Option<(String, Option<Usage>)> {
    if payload.trim() == "[DONE]" {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    let delta = value
        .pointer("/choices/0/delta/content")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let usage = value.get("usage").and_then(|u| {
        if u.is_null() {
            None
        } else {
            serde_json::from_value::<UsageWire>(u.clone())
                .ok()
                .map(Usage::from)
        }
    });
    Some((delta, usage))
}

/// Accumulates streamed deltas into a complete chat outcome.
fn accumulate_stream(payloads: &[&str], model: &str) -> Result<ChatOutcome, MetaApiError> {
    let mut content = String::new();
    let mut usage = Usage::default();
    for payload in payloads {
        if let Some((delta, maybe_usage)) = parse_stream_payload(payload) {
            content.push_str(&delta);
            if let Some(found) = maybe_usage {
                usage = found;
            }
        }
    }
    if content.is_empty() {
        return Err(MetaApiError::EmptyStream);
    }
    Ok(ChatOutcome {
        content,
        usage,
        model: model.to_string(),
    })
}

#[derive(Debug, Clone)]
pub struct MetaClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    pub strong_model: String,
    pub light_model: String,
}

impl MetaClient {
    /// Builds a client from the MODEL_API_KEY environment variable.
    pub fn from_env() -> Result<Self, MetaApiError> {
        let api_key = std::env::var(API_KEY_ENV).map_err(|_| MetaApiError::MissingApiKey)?;
        Ok(Self::new(DEFAULT_BASE_URL, &api_key))
    }

    /// Builds a client for an explicit base URL and API key.
    pub fn new(base_url: &str, api_key: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("http client builds");
        Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            strong_model: STRONG_MODEL.to_string(),
            light_model: LIGHT_MODEL.to_string(),
        }
    }

    /// Lists model IDs visible to this API key.
    pub async fn list_models(&self) -> Result<Vec<String>, MetaApiError> {
        let url = format!("{}/models", self.base_url);
        let response = self
            .http
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await?;
        let response = Self::ensure_success(response).await?;
        let parsed: ModelsResponse = response.json().await?;
        Ok(parsed.data.into_iter().map(|m| m.id).collect())
    }

    /// Runs one non-streaming chat completion on the given model.
    pub async fn chat(
        &self,
        model: &str,
        messages: Vec<Message>,
    ) -> Result<ChatOutcome, MetaApiError> {
        let url = format!("{}/chat/completions", self.base_url);
        let request = ChatRequest {
            model: model.to_string(),
            messages,
            stream: false,
            stream_options: None,
        };
        let response = self
            .http
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&request)
            .send()
            .await?;
        let response = Self::ensure_success(response).await?;
        let parsed: ChatResponse = response.json().await?;
        let choice = parsed
            .choices
            .into_iter()
            .next()
            .ok_or(MetaApiError::EmptyChoices)?;
        let message = choice.message.ok_or(MetaApiError::EmptyChoices)?;
        Ok(ChatOutcome {
            content: message.content,
            usage: parsed.usage.map(Usage::from).unwrap_or_default(),
            model: parsed.model,
        })
    }

    /// Runs one streaming chat completion and assembles the full text.
    pub async fn chat_stream(
        &self,
        model: &str,
        messages: Vec<Message>,
    ) -> Result<ChatOutcome, MetaApiError> {
        let url = format!("{}/chat/completions", self.base_url);
        let request = ChatRequest {
            model: model.to_string(),
            messages,
            stream: true,
            stream_options: Some(StreamOptions {
                include_usage: true,
            }),
        };
        let response = self
            .http
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&request)
            .send()
            .await?;
        let mut response = Self::ensure_success(response).await?;
        let mut buffer = String::new();
        let mut payloads: Vec<String> = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(index) = buffer.find('\n') {
                let line: String = buffer.drain(..=index).collect();
                let line = line.trim();
                if let Some(payload) = line.strip_prefix("data:") {
                    payloads.push(payload.trim().to_string());
                }
            }
        }
        let refs: Vec<&str> = payloads.iter().map(|s| s.as_str()).collect();
        accumulate_stream(&refs, model)
    }

    /// Converts a non-success HTTP response into a typed API error.
    async fn ensure_success(
        response: reqwest::Response,
    ) -> Result<reqwest::Response, MetaApiError> {
        if response.status().is_success() {
            return Ok(response);
        }
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        Err(MetaApiError::Status { status, body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_env_key_is_typed_error() {
        std::env::remove_var(API_KEY_ENV);
        assert!(matches!(
            MetaClient::from_env(),
            Err(MetaApiError::MissingApiKey)
        ));
    }

    #[test]
    fn stream_payloads_assemble_content_and_usage() {
        let payloads = [
            r#"{"choices":[{"delta":{"content":"Hel"}}]}"#,
            r#"{"choices":[{"delta":{"content":"lo"}}]}"#,
            r#"{"choices":[{"delta":{}}],"usage":{"prompt_tokens":10,"completion_tokens":2,"total_tokens":12}}"#,
            "[DONE]",
        ];
        let outcome = accumulate_stream(&payloads, "m").expect("stream assembles");
        assert_eq!(outcome.content, "Hello");
        assert_eq!(outcome.usage.total_tokens, 12);
    }

    #[test]
    fn usage_wire_defaults_total_from_parts() {
        let usage = Usage::from(UsageWire {
            prompt_tokens: Some(7),
            completion_tokens: Some(3),
            total_tokens: None,
        });
        assert_eq!(usage.total_tokens, 10);
    }
}
