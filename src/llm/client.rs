//! OpenAI-compatible LLM client (Meta is the default provider preset).

use super::config::{ProviderConfig, ProviderError};

/// Backwards-compat alias for the canonical [`ProviderError`].
pub type MetaApiError = ProviderError;
use serde::{Deserialize, Serialize};
use std::{fmt, time::Duration};

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
    /// True when tokens were estimated locally because the provider
    /// returned all-zero usage. Defaults to false for old payloads.
    #[serde(default)]
    pub estimated: bool,
}

fn approx_tokens(chars: usize) -> u64 {
    chars.div_ceil(4) as u64
}

impl Usage {
    /// True when every token count is zero.
    pub fn is_zero(&self) -> bool {
        self.prompt_tokens == 0 && self.completion_tokens == 0 && self.total_tokens == 0
    }

    /// Estimates tokens as ceil(chars/4) over prompt and completion text.
    pub fn estimated_for(messages: &[Message], completion: &str) -> Self {
        let prompt_chars: usize = messages.iter().map(|m| m.content.chars().count()).sum();
        let prompt_tokens = approx_tokens(prompt_chars);
        let completion_tokens = approx_tokens(completion.chars().count());
        Self {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
            estimated: true,
        }
    }

    /// Replaces all-zero usage with an estimate; keeps real usage untouched.
    pub fn with_fallback(self, messages: &[Message], completion: &str) -> Self {
        if self.is_zero() {
            Self::estimated_for(messages, completion)
        } else {
            self
        }
    }

    /// Sums two usages (e.g. an initial attempt plus its repair attempt).
    pub fn combined(&self, other: &Usage) -> Usage {
        Usage {
            prompt_tokens: self.prompt_tokens.saturating_add(other.prompt_tokens),
            completion_tokens: self
                .completion_tokens
                .saturating_add(other.completion_tokens),
            total_tokens: self.total_tokens.saturating_add(other.total_tokens),
            estimated: self.estimated || other.estimated,
        }
    }
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
            estimated: false,
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

/// Folds one payload into the running content and usage.
#[rustfmt::skip]
fn fold_stream_payload(payload: &str, content: &mut String, usage: &mut Usage, on_chunk: &mut (dyn FnMut(&str) + Send)) {
    if let Some((delta, maybe_usage)) = parse_stream_payload(payload) {
        if !delta.is_empty() { content.push_str(&delta); on_chunk(&delta); }
        if let Some(next) = maybe_usage { *usage = next; }
    }
}
/// Accumulates streamed deltas into a complete chat outcome.
#[cfg(test)] #[rustfmt::skip]
fn accumulate_stream(payloads: &[&str], model: &str) -> Result<ChatOutcome, ProviderError> { accumulate_stream_each(payloads, model, &mut |_| {}) }
/// Accumulates streamed deltas, reporting each non-empty delta in order.
#[cfg(test)] #[rustfmt::skip]
fn accumulate_stream_each(payloads: &[&str], model: &str, on_chunk: &mut (dyn FnMut(&str) + Send)) -> Result<ChatOutcome, ProviderError> {
    let (mut content, mut usage) = (String::new(), Usage::default());
    for payload in payloads { fold_stream_payload(payload, &mut content, &mut usage, on_chunk); }
    if content.is_empty() { return Err(ProviderError::EmptyStream); }
    Ok(ChatOutcome { content, usage, model: model.to_string() })
}
#[derive(Clone)]
pub struct LlmClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    provider: String,
    requires_api_key: bool,
    pub strong_model: String,
    pub light_model: String,
}

/// Backwards-compat alias for [`LlmClient`].
pub type MetaClient = LlmClient;

impl fmt::Debug for LlmClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LlmClient")
            .field("provider", &self.provider)
            .field("base_url", &self.base_url)
            .field("api_key", &"[REDACTED]")
            .field("strong_model", &self.strong_model)
            .field("light_model", &self.light_model)
            .finish_non_exhaustive()
    }
}

impl LlmClient {
    /// Builds a client from provider env vars (see [`ProviderConfig`]).
    pub fn from_env() -> Result<Self, ProviderError> {
        Ok(Self::from_config(ProviderConfig::from_env()?))
    }

    /// Builds a client from a resolved provider config.
    pub fn from_config(config: ProviderConfig) -> Self {
        let builder = reqwest::Client::builder().timeout(Duration::from_secs(120));
        let http = builder.build().expect("http client builds");
        Self {
            http,
            base_url: config.base_url.trim_end_matches('/').to_string(),
            api_key: config.api_key,
            provider: config.provider,
            requires_api_key: config.requires_api_key,
            strong_model: config.strong_model,
            light_model: config.light_model,
        }
    }

    /// Alias for [`Self::from_config`].
    pub fn new_with_config(config: ProviderConfig) -> Self {
        Self::from_config(config)
    }

    /// Reconstructs the provider config (key included; Debug redacts it).
    pub fn config(&self) -> ProviderConfig {
        ProviderConfig {
            provider: self.provider.clone(),
            base_url: self.base_url.clone(),
            api_key: self.api_key.clone(),
            strong_model: self.strong_model.clone(),
            light_model: self.light_model.clone(),
            requires_api_key: self.requires_api_key,
        }
    }
    pub fn provider(&self) -> &str {
        &self.provider
    }
    pub fn provider_id(&self) -> &str {
        &self.provider
    }
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Builds a Meta-preset client for an explicit base URL and API key.
    pub fn new(base_url: &str, api_key: &str) -> Self {
        let mut cfg = ProviderConfig::meta_with_key(api_key);
        cfg.base_url = base_url.trim_end_matches('/').to_string();
        Self::from_config(cfg)
    }

    /// Lists model IDs visible to this API key.
    pub async fn list_models(&self) -> Result<Vec<String>, ProviderError> {
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
        let request = build_request(model, messages.clone(), false, options);
        let builder = self.http.post(&url).bearer_auth(&self.api_key);
        let response = builder.json(&request).send().await?;
        let response = Self::ensure_success(response).await?;
        let parsed: ChatResponse = response.json().await?;
        let mut choices = parsed.choices.into_iter();
        let choice = choices.next().ok_or(ProviderError::EmptyChoices)?;
        let message = choice.message.ok_or(ProviderError::EmptyChoices)?;
        let usage = parsed
            .usage
            .map(Usage::from)
            .unwrap_or_default()
            .with_fallback(&messages, &message.content);
        Ok(ChatOutcome {
            content: message.content,
            usage,
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
    #[rustfmt::skip]
    pub async fn chat_stream_with_options(&self, model: &str, messages: Vec<Message>, options: ChatOptions) -> Result<ChatOutcome, MetaApiError> { self.chat_stream_each(model, messages, options, &mut |_| {}).await }

    /// Runs one streaming chat completion, reporting each delta as it arrives.
    #[rustfmt::skip]
    pub async fn chat_stream_each(&self, model: &str, messages: Vec<Message>, options: ChatOptions, on_chunk: &mut (dyn FnMut(&str) + Send)) -> Result<ChatOutcome, MetaApiError> {
        let url = format!("{}/chat/completions", self.base_url);
        let request = build_request(model, messages.clone(), true, options);
        let response = Self::ensure_success(self.http.post(&url).bearer_auth(&self.api_key).json(&request).send().await?).await?;
        let (mut response, mut buffer, mut content, mut usage) = (response, String::new(), String::new(), Usage::default());
        while let Some(chunk) = response.chunk().await? {
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(index) = buffer.find('\n') { let line: String = buffer.drain(..=index).collect(); if let Some(p) = line.trim().strip_prefix("data:") { fold_stream_payload(p.trim(), &mut content, &mut usage, on_chunk); } }
        }
        if let Some(p) = buffer.trim().strip_prefix("data:") { fold_stream_payload(p.trim(), &mut content, &mut usage, on_chunk); }
        if content.is_empty() { return Err(ProviderError::EmptyStream); }
        let usage = usage.with_fallback(&messages, &content);
        Ok(ChatOutcome { content, usage, model: model.to_string() })
    }

    /// Converts a non-success HTTP response into a typed API error.
    async fn ensure_success(
        response: reqwest::Response,
    ) -> Result<reqwest::Response, ProviderError> {
        if response.status().is_success() {
            return Ok(response);
        }
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        Err(ProviderError::Status { status, body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_env_key_is_typed_error() {
        for k in [
            crate::llm::API_KEY_ENV,
            "PAIRPROOF_API_KEY",
            "PAIRPROOF_PROVIDER",
            "PAIRPROOF_BASE_URL",
        ] {
            std::env::remove_var(k);
        }
        assert!(matches!(
            LlmClient::from_env(),
            Err(ProviderError::MissingApiKey)
        ));
        // Alias resolves to the same canonical types.
        assert!(matches!(
            MetaClient::from_env(),
            Err(MetaApiError::MissingApiKey)
        ));
    }

    #[test]
    fn llm_client_from_config_carries_provider_and_models() {
        let cfg = ProviderConfig::meta_with_key("k").with_api_key("k2");
        let client = LlmClient::from_config(cfg.clone());
        assert_eq!(client.provider(), "meta");
        assert_eq!(client.base_url(), crate::llm::DEFAULT_BASE_URL);
        assert_eq!(client.config(), cfg);
        assert_eq!(client.strong_model, crate::llm::STRONG_MODEL);
        assert_eq!(client.light_model, crate::llm::LIGHT_MODEL);
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
    #[rustfmt::skip]
    fn stream_each_reports_deltas_in_order() {
        let payloads = [r#"{"choices":[{"delta":{"content":"Hel"}}]}"#, r#"{"choices":[{"delta":{"content":"lo"}}]}"#, "[DONE]"];
        let mut seen: Vec<String> = Vec::new();
        let outcome = accumulate_stream_each(&payloads, "m", &mut |d| seen.push(d.to_string())).expect("stream assembles");
        assert_eq!(seen, vec!["Hel".to_string(), "lo".to_string()]); assert_eq!(outcome.content, "Hello");
    }

    #[test]
    fn zero_usage_is_estimated_and_real_usage_is_kept() {
        let msgs = vec![Message::new("user", "12345678")];
        let est = Usage::default().with_fallback(&msgs, "abcd");
        assert!(est.estimated);
        assert_eq!(
            (est.prompt_tokens, est.completion_tokens, est.total_tokens),
            (2, 1, 3)
        );
        let real = Usage {
            prompt_tokens: 5,
            completion_tokens: 2,
            total_tokens: 7,
            estimated: false,
        };
        assert_eq!(real.clone().with_fallback(&msgs, "abcd"), real);
        // Back-compat: missing estimated field deserializes as false.
        let old: Usage =
            serde_json::from_str(r#"{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}"#)
                .unwrap();
        assert!(!old.estimated);
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
    #[test]
    fn debug_redacts_api_key() {
        let client = MetaClient::new("http://127.0.0.1:9", "test-secret-key-123");
        let debug = format!("{client:?}");
        assert!(!debug.contains("test-secret-key-123"), "{debug}");
        assert!(debug.contains("[REDACTED]"), "{debug}");
        assert!(debug.contains("127.0.0.1:9"), "{debug}");
        assert!(debug.contains(&client.strong_model), "{debug}");
        assert!(debug.contains(&client.light_model), "{debug}");
    }
}
