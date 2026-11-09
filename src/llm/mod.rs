//! LLM provider clients (Meta default) and their prompts.

pub mod approaches;
pub mod chat;
pub mod client;
pub mod coding;
pub mod common;
pub mod config;
pub mod extraction;
pub mod grading;
pub mod shadow_eval;

pub const DEFAULT_BASE_URL: &str = "https://api.meta.ai/v1";
pub const STRONG_MODEL: &str = "muse-wonder-1.0";
pub const LIGHT_MODEL: &str = "muse-ember-1.0";
pub const API_KEY_ENV: &str = "MODEL_API_KEY";

pub use client::{
    ChatOptions, ChatOutcome, LlmClient, Message, MetaApiError, MetaClient, ModelInfo,
    ReasoningEffort, Usage,
};
pub use common::{LlmError, LlmOutcome};
/// Alias for the provider-config error type.
pub use config::ProviderError as ProviderConfigError;
pub use config::{
    provider_preset, ProviderConfig, ProviderError, ProviderPreset, BASE_URL_ENV,
    GENERIC_PROVIDERS, LIGHT_MODEL_ENV, META_PROVIDER, PROVIDER_API_KEY_ENV, PROVIDER_ENV,
    PROVIDER_PRESETS, STRONG_MODEL_ENV,
};
