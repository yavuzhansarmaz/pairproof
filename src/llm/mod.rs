//! Meta Model API clients (strong and light) and their prompts.

pub mod approaches;
pub mod chat;
pub mod client;
pub mod coding;
pub mod common;
pub mod extraction;
pub mod grading;
pub mod shadow_eval;

pub const DEFAULT_BASE_URL: &str = "https://api.meta.ai/v1";
pub const STRONG_MODEL: &str = "muse-wonder-1.0";
pub const LIGHT_MODEL: &str = "muse-spark-1.3";
pub const API_KEY_ENV: &str = "MODEL_API_KEY";

pub use client::{
    ChatOptions, ChatOutcome, Message, MetaApiError, MetaClient, ModelInfo, ReasoningEffort, Usage,
};
pub use common::{LlmError, LlmOutcome};
