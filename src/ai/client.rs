//! Meta Model API client implementation.

use super::{API_KEY_ENV, DEFAULT_BASE_URL, LIGHT_MODEL, STRONG_MODEL};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Provider reasoning effort, serialised lowercase on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChatOptions {
    pub reasoning_effort: Option<ReasoningEffort>,
}

impl ChatOptions {
    /// Options requesting low reasoning effort, used for chat calls.
    pub fn low() -> Self {
        Self {
            reasoning_effort: Some(ReasoningEffort::Low),
        }
    }
}

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
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<ReasoningEffort>,
}

#[derive(Debug, Clone, Serialize)]
struct StreamOptions {
    include_usage: bool,
}

/// Builds a chat request, omitting unset optional fields on the wire.
fn build_request(
    model: &str,
    messages: Vec<Message>,
    stream: bool,
    options: ChatOptions,
) -> ChatRequest {
    ChatRequest {
        model: model.to_string(),
        messages,
        stream,
        stream_options: stream.then_some(StreamOptions {
            include_usage: true,
        }),
        reasoning_effort: options.reasoning_effort,
    }
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
    let found = value.pointer("/choices/0/delta/content");
    let delta = found.and_then(|v| v.as_str()).unwrap_or_default();
    let usage = value
        .get("usage")
        .filter(|u| !u.is_null())
        .and_then(|u| serde_json::from_value::<UsageWire>(u.clone()).ok())
        .map(Usage::from);
    Some((delta.to_string(), usage))
}

/// Accumulates streamed deltas into a complete chat outcome.
fn accumulate_stream(payloads: &[&str], model: &str) -> Result<ChatOutcome, MetaApiError> {
    let mut content = String::new();
    let mut usage = Usage::default();
    for payload in payloads {
        if let Some((delta, maybe_usage)) = parse_stream_payload(payload) {
            content.push_str(&delta);
            usage = maybe_usage.unwrap_or(usage);
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
        let builder = reqwest::Client::builder().timeout(Duration::from_secs(120));
        let http = builder.build().expect("http client builds");
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
        let builder = self.http.get(&url).bearer_auth(&self.api_key);
        let response = builder.send().await?;
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
        self.chat_with_options(model, messages, ChatOptions::default())
            .await
    }

    /// Runs one non-streaming chat completion with explicit options.
    pub async fn chat_with_options(
        &self,
        model: &str,
        messages: Vec<Message>,
        options: ChatOptions,
    ) -> Result<ChatOutcome, MetaApiError> {
        let url = format!("{}/chat/completions", self.base_url);
        let request = build_request(model, messages, false, options);
        let builder = self.http.post(&url).bearer_auth(&self.api_key);
        let response = builder.json(&request).send().await?;
        let response = Self::ensure_success(response).await?;
        let parsed: ChatResponse = response.json().await?;
        let mut choices = parsed.choices.into_iter();
        let choice = choices.next().ok_or(MetaApiError::EmptyChoices)?;
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
        self.chat_stream_with_options(model, messages, ChatOptions::default())
            .await
    }

    /// Runs one streaming chat completion with explicit options.
    pub async fn chat_stream_with_options(
        &self,
        model: &str,
        messages: Vec<Message>,
        options: ChatOptions,
    ) -> Result<ChatOutcome, MetaApiError> {
        let url = format!("{}/chat/completions", self.base_url);
        let request = build_request(model, messages, true, options);
        let builder = self.http.post(&url).bearer_auth(&self.api_key);
        let response = builder.json(&request).send().await?;
        let mut response = Self::ensure_success(response).await?;
        let mut buffer = String::new();
        let mut payloads: Vec<String> = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(index) = buffer.find('\n') {
                let line: String = buffer.drain(..=index).collect();
                if let Some(payload) = line.trim().strip_prefix("data:") {
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

    fn request_json(options: ChatOptions) -> String {
        let req = build_request("m", vec![Message::new("user", "hi")], false, options);
        serde_json::to_string(&req).expect("request serialises")
    }

    #[test]
    fn chat_request_omits_reasoning_effort_by_default() {
        let json = request_json(ChatOptions::default());
        assert!(!json.contains("reasoning_effort"), "{json}");
    }

    #[test]
    fn chat_request_serializes_low_reasoning_effort() {
        let json = request_json(ChatOptions::low());
        assert!(json.contains("\"reasoning_effort\":\"low\""), "{json}");
    }

    #[test]
    fn reasoning_effort_variants_serialize_lowercase() {
        use ReasoningEffort::*;
        for (effort, want) in [(Low, "\"low\""), (Medium, "\"medium\""), (High, "\"high\"")] {
            let got = serde_json::to_string(&effort).expect("effort serialises");
            assert_eq!(got, want);
        }
    }

    #[test]
    fn chat_options_low_sets_low() {
        let got = ChatOptions::low().reasoning_effort;
        assert_eq!(got, Some(ReasoningEffort::Low));
        assert_eq!(ChatOptions::default().reasoning_effort, None);
    }
}
