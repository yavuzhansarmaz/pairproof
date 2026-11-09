//! Canonical LLM provider configuration (OpenAI-compatible wire).
//!
//! Meta is the exact default preset. Other provider ids are accepted
//! only as generic OpenAI-compatible endpoints with an explicit
//! base URL; there are no native Anthropic/Gemini adapters here.

use std::collections::{BTreeMap, HashMap};
use std::fmt;

use super::{API_KEY_ENV, DEFAULT_BASE_URL, LIGHT_MODEL, STRONG_MODEL};

pub const PROVIDER_ENV: &str = "PAIRPROOF_PROVIDER";
pub const PROVIDER_API_KEY_ENV: &str = "PAIRPROOF_API_KEY";
pub const BASE_URL_ENV: &str = "PAIRPROOF_BASE_URL";
pub const STRONG_MODEL_ENV: &str = "PAIRPROOF_STRONG_MODEL";
pub const LIGHT_MODEL_ENV: &str = "PAIRPROOF_LIGHT_MODEL";
pub const META_PROVIDER: &str = "meta";
/// Provider ids that denote a generic OpenAI-compatible endpoint.
pub const GENERIC_PROVIDERS: &[&str] = &[
    "openai-compatible",
    "openai_compatible",
    "custom",
    "generic",
];

/// One OpenAI-compatible provider preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderPreset {
    pub id: &'static str,
    /// None only for `custom`, which requires `PAIRPROOF_BASE_URL`.
    pub base_url: Option<&'static str>,
    /// Provider-specific key env; None for keyless/custom providers.
    pub api_key_env: Option<&'static str>,
    pub requires_api_key: bool,
    pub strong_model: &'static str,
    pub light_model: &'static str,
}

/// All supported presets (OpenAI-compatible wire only, no native adapters).
pub const PROVIDER_PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        id: "meta",
        base_url: Some("https://api.meta.ai/v1"),
        api_key_env: Some("MODEL_API_KEY"),
        requires_api_key: true,
        strong_model: "muse-wonder-1.0",
        light_model: "muse-ember-1.0",
    },
    ProviderPreset {
        id: "openai",
        base_url: Some("https://api.openai.com/v1"),
        api_key_env: Some("OPENAI_API_KEY"),
        requires_api_key: true,
        strong_model: "gpt-4o",
        light_model: "gpt-4o-mini",
    },
    ProviderPreset {
        id: "openrouter",
        base_url: Some("https://openrouter.ai/api/v1"),
        api_key_env: Some("OPENROUTER_API_KEY"),
        requires_api_key: true,
        strong_model: "openai/gpt-4o",
        light_model: "openai/gpt-4o-mini",
    },
    ProviderPreset {
        id: "groq",
        base_url: Some("https://api.groq.com/openai/v1"),
        api_key_env: Some("GROQ_API_KEY"),
        requires_api_key: true,
        strong_model: "llama-3.3-70b-versatile",
        light_model: "llama-3.1-8b-instant",
    },
    ProviderPreset {
        id: "together",
        base_url: Some("https://api.together.xyz/v1"),
        api_key_env: Some("TOGETHER_API_KEY"),
        requires_api_key: true,
        strong_model: "meta-llama/Llama-3.3-70B-Instruct-Turbo",
        light_model: "meta-llama/Meta-Llama-3.1-8B-Instruct-Turbo",
    },
    ProviderPreset {
        id: "deepseek",
        base_url: Some("https://api.deepseek.com/v1"),
        api_key_env: Some("DEEPSEEK_API_KEY"),
        requires_api_key: true,
        strong_model: "deepseek-chat",
        light_model: "deepseek-chat",
    },
    ProviderPreset {
        id: "ollama",
        base_url: Some("http://localhost:11434/v1"),
        api_key_env: None,
        requires_api_key: false,
        strong_model: "llama3.1",
        light_model: "llama3.2",
    },
    ProviderPreset {
        id: "custom",
        base_url: None,
        api_key_env: None,
        requires_api_key: true,
        strong_model: "gpt-4o",
        light_model: "gpt-4o-mini",
    },
];

/// Looks up a preset by (case-insensitive) provider id.
pub fn provider_preset(id: &str) -> Option<&'static ProviderPreset> {
    let id = id.trim().to_ascii_lowercase();
    PROVIDER_PRESETS.iter().find(|p| p.id == id)
}

/// Canonical provider error. `MetaApiError` is a backwards-compat alias.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("provider api key is not set (set PAIRPROOF_API_KEY or the provider key env, e.g. MODEL_API_KEY)")]
    MissingApiKey,
    #[error("provider api request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("provider api returned status {status}: {body}")]
    Status { status: u16, body: String },
    #[error("provider api response contained no choices")]
    EmptyChoices,
    #[error("provider api stream ended without content")]
    EmptyStream,
    #[error("unknown provider: {0}")]
    UnknownProvider(String),
    #[error("provider {0} requires PAIRPROOF_BASE_URL")]
    MissingBaseUrl(String),
    #[error("invalid provider base url: {0}")]
    InvalidBaseUrl(String),
}

/// Alias kept so provider-config failures share the client error type.
pub type ConfigError = ProviderError;

fn clean(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn validate_base_url(raw: &str) -> Result<String, ProviderError> {
    let trimmed = raw.trim().trim_end_matches('/');
    if !(trimmed.starts_with("https://") || trimmed.starts_with("http://"))
        || trimmed.len() <= "https://".len()
    {
        return Err(ProviderError::InvalidBaseUrl(raw.trim().to_string()));
    }
    Ok(trimmed.to_string())
}

/// Provider configuration resolved before constructing [`super::LlmClient`].
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderConfig {
    pub provider: String,
    pub base_url: String,
    pub api_key: String,
    pub strong_model: String,
    pub light_model: String,
    pub requires_api_key: bool,
}

impl fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderConfig")
            .field("provider", &self.provider)
            .field("base_url", &self.base_url)
            .field("api_key", &"[REDACTED]")
            .field("strong_model", &self.strong_model)
            .field("light_model", &self.light_model)
            .field("requires_api_key", &self.requires_api_key)
            .finish()
    }
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self::meta()
    }
}

impl ProviderConfig {
    /// Exact Meta preset, without a key.
    pub fn meta() -> Self {
        Self {
            provider: META_PROVIDER.to_string(),
            base_url: DEFAULT_BASE_URL.to_string(),
            api_key: String::new(),
            strong_model: STRONG_MODEL.to_string(),
            light_model: LIGHT_MODEL.to_string(),
            requires_api_key: true,
        }
    }

    /// Meta preset carrying `api_key`.
    pub fn meta_with_key(api_key: &str) -> Self {
        let mut cfg = Self::meta();
        cfg.api_key = api_key.trim().to_string();
        cfg
    }

    /// Preset for `id` without a key; unknown ids are a typed error.
    pub fn for_provider(id: &str) -> Result<Self, ProviderError> {
        let preset = provider_preset(id)
            .ok_or_else(|| ProviderError::UnknownProvider(id.trim().to_ascii_lowercase()))?;
        Ok(Self {
            provider: preset.id.to_string(),
            base_url: preset.base_url.unwrap_or_default().to_string(),
            api_key: String::new(),
            strong_model: preset.strong_model.to_string(),
            light_model: preset.light_model.to_string(),
            requires_api_key: preset.requires_api_key,
        })
    }

    /// The preset backing this config, when the provider is a known preset.
    pub fn preset(&self) -> Option<&'static ProviderPreset> {
        provider_preset(&self.provider)
    }

    /// Provider-specific api-key env name, when conventional.
    pub fn api_key_env(&self) -> Option<&'static str> {
        self.preset().and_then(|p| p.api_key_env)
    }

    /// Human label for the provider (`Meta` for meta, else the id).
    pub fn display_name(&self) -> String {
        if self.provider == META_PROVIDER {
            "Meta".to_string()
        } else {
            self.provider.clone()
        }
    }

    /// Provider id (alias accessor for callers that call it `id`).
    pub fn id(&self) -> &str {
        &self.provider
    }
    /// Provider id.
    pub fn provider_id(&self) -> &str {
        &self.provider
    }
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
    pub fn strong_model(&self) -> &str {
        &self.strong_model
    }
    pub fn light_model(&self) -> &str {
        &self.light_model
    }

    /// Returns a copy with `api_key` set (trimmed).
    pub fn with_api_key(mut self, api_key: &str) -> Self {
        self.api_key = api_key.trim().to_string();
        self
    }

    /// Validates base URL / provider / key invariants; never panics.
    pub fn validate(&self) -> Result<(), ProviderError> {
        validate_base_url(&self.base_url)?;
        if self.provider.trim().is_empty() {
            return Err(ProviderError::UnknownProvider(String::new()));
        }
        if self.requires_api_key && self.api_key.trim().is_empty() {
            return Err(ProviderError::MissingApiKey);
        }
        Ok(())
    }

    /// Pure resolution from an env map (HashMap form).
    pub fn from_env_map(map: &HashMap<String, String>) -> Result<Self, ProviderError> {
        Self::from_lookup(|k| map.get(k).cloned())
    }
    /// Alias for [`Self::from_env_map`].
    pub fn resolve_from_env_map(map: &HashMap<String, String>) -> Result<Self, ProviderError> {
        Self::from_env_map(map)
    }
    /// Pure resolution from a BTreeMap env map.
    pub fn from_env_btree(map: &BTreeMap<String, String>) -> Result<Self, ProviderError> {
        Self::from_lookup(|k| map.get(k).cloned())
    }

    /// Pure resolution from any lookup function.
    /// Precedence: preset defaults < provider-specific key env /
    /// preset base+models < `PAIRPROOF_*` overrides (key, base, models).
    pub fn from_lookup<F: Fn(&str) -> Option<String>>(get: F) -> Result<Self, ProviderError> {
        Self::resolve(&get, true)
    }

    fn resolve<F: Fn(&str) -> Option<String>>(
        get: &F,
        require_key: bool,
    ) -> Result<Self, ProviderError> {
        let get = |k: &str| clean(get(k).as_deref());
        let provider = get(PROVIDER_ENV)
            .map(|p| p.to_ascii_lowercase())
            .unwrap_or_else(|| META_PROVIDER.to_string());
        // Generic aliases use the custom preset's defaults (id preserved).
        let preset = provider_preset(&provider).or_else(|| {
            GENERIC_PROVIDERS
                .contains(&provider.as_str())
                .then(|| provider_preset("custom"))
                .flatten()
        });
        let base_override = get(BASE_URL_ENV);
        if preset.is_none() && base_override.is_none() {
            return Err(ProviderError::UnknownProvider(provider));
        }
        let (preset_base, requires_api_key, preset_strong, preset_light, key_env) = match preset {
            Some(p) => (
                p.base_url,
                p.requires_api_key,
                p.strong_model,
                p.light_model,
                p.api_key_env,
            ),
            None => (None, true, STRONG_MODEL, LIGHT_MODEL, None),
        };
        let base_url = match base_override {
            Some(raw) => validate_base_url(&raw)?,
            None => match preset_base {
                Some(b) => b.to_string(),
                None => return Err(ProviderError::MissingBaseUrl(provider)),
            },
        };
        // PAIRPROOF_API_KEY beats the provider-specific env (MODEL_API_KEY
        // is meta's provider env, via its preset entry).
        let api_key = get(PROVIDER_API_KEY_ENV)
            .or_else(|| key_env.and_then(get))
            .or_else(|| {
                // Unknown/generic endpoints keep the phase-1 legacy fallback.
                if preset.is_none() {
                    get(API_KEY_ENV)
                } else {
                    None
                }
            });
        let api_key = match api_key {
            Some(k) => k,
            None if requires_api_key && require_key => return Err(ProviderError::MissingApiKey),
            None => String::new(),
        };
        Ok(Self {
            provider,
            base_url,
            api_key,
            strong_model: get(STRONG_MODEL_ENV).unwrap_or_else(|| preset_strong.to_string()),
            light_model: get(LIGHT_MODEL_ENV).unwrap_or_else(|| preset_light.to_string()),
            requires_api_key,
        })
    }

    /// Resolution like [`Self::from_env_map`] but without requiring a key;
    /// missing key yields the preset with an empty key.
    /// Used by stored-key fallback in the TUI (key filled in later).
    pub fn preset_from_env_map(map: &HashMap<String, String>) -> Result<Self, ProviderError> {
        Self::resolve(&|k: &str| map.get(k).cloned(), false)
    }

    /// Like [`Self::from_env`] without requiring a key (key empty when unset).
    pub fn preset_from_env() -> Result<Self, ProviderError> {
        Self::resolve(&|k: &str| std::env::var(k).ok(), false)
    }

    /// Resolves from the process environment.
    pub fn from_env() -> Result<Self, ProviderError> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }
    /// Alias for [`Self::from_env`].
    pub fn resolve_from_env() -> Result<Self, ProviderError> {
        Self::from_env()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }
    #[test]
    fn meta_preset_exact_values() {
        let c = ProviderConfig::meta();
        assert_eq!(c.provider, "meta");
        assert_eq!(c.id(), "meta");
        assert_eq!(c.base_url, "https://api.meta.ai/v1");
        assert_eq!(c.strong_model, "muse-wonder-1.0");
        assert_eq!(c.light_model, "muse-ember-1.0");
        assert!(c.requires_api_key);
        assert_eq!(ProviderConfig::default(), c);
    }
    #[test]
    fn env_map_meta_default_with_legacy_key() {
        let c = ProviderConfig::from_env_map(&map(&[("MODEL_API_KEY", " legacy ")])).unwrap();
        assert_eq!(c.provider, "meta");
        assert_eq!(c.base_url, DEFAULT_BASE_URL);
        assert_eq!(c.api_key, "legacy");
        assert_eq!(c.strong_model, STRONG_MODEL);
    }
    #[test]
    fn env_overrides_win_and_pairproof_key_beats_legacy() {
        let c = ProviderConfig::from_env_map(&map(&[
            ("PAIRPROOF_PROVIDER", " Meta "),
            ("PAIRPROOF_API_KEY", "new-key"),
            ("MODEL_API_KEY", "old-key"),
            ("PAIRPROOF_BASE_URL", "https://example.test/v1/"),
            ("PAIRPROOF_STRONG_MODEL", "strong-x"),
            ("PAIRPROOF_LIGHT_MODEL", "light-x"),
        ]))
        .unwrap();
        assert_eq!(c.provider, "meta");
        assert_eq!(c.api_key, "new-key");
        assert_eq!(c.base_url, "https://example.test/v1");
        assert_eq!(c.strong_model, "strong-x");
        assert_eq!(c.light_model, "light-x");
    }
    #[test]
    fn blank_values_are_unset_and_missing_key_is_typed() {
        let err = ProviderConfig::from_env_map(&map(&[
            ("PAIRPROOF_API_KEY", "  "),
            ("MODEL_API_KEY", ""),
        ]))
        .unwrap_err();
        assert!(matches!(err, ProviderError::MissingApiKey), "{err:?}");
        let err = ProviderConfig::from_env_map(&map(&[])).unwrap_err();
        assert!(matches!(err, ProviderError::MissingApiKey));
    }
    #[test]
    fn unknown_provider_without_base_url_is_typed() {
        let err = ProviderConfig::from_env_map(&map(&[
            ("PAIRPROOF_PROVIDER", "anthropic"),
            ("PAIRPROOF_API_KEY", "k"),
        ]))
        .unwrap_err();
        assert!(
            matches!(err, ProviderError::UnknownProvider(ref p) if p == "anthropic"),
            "{err:?}"
        );
    }
    #[test]
    fn unknown_provider_with_base_url_is_generic_compatible() {
        let c = ProviderConfig::from_env_map(&map(&[
            ("PAIRPROOF_PROVIDER", "my-gateway"),
            ("PAIRPROOF_API_KEY", "k"),
            ("PAIRPROOF_BASE_URL", "https://gw.test/v1"),
        ]))
        .unwrap();
        assert_eq!(c.provider, "my-gateway");
        assert_eq!(c.base_url, "https://gw.test/v1");
    }
    #[test]
    fn generic_provider_requires_base_url_and_valid_scheme() {
        let err = ProviderConfig::from_env_map(&map(&[
            ("PAIRPROOF_PROVIDER", "custom"),
            ("PAIRPROOF_API_KEY", "k"),
        ]))
        .unwrap_err();
        assert!(matches!(err, ProviderError::MissingBaseUrl(_)), "{err:?}");
        let err = ProviderConfig::from_env_map(&map(&[
            ("PAIRPROOF_API_KEY", "k"),
            ("PAIRPROOF_BASE_URL", "ftp://x"),
        ]))
        .unwrap_err();
        assert!(matches!(err, ProviderError::InvalidBaseUrl(_)), "{err:?}");
    }
    #[test]
    fn preset_table_exact() {
        type Case = (
            &'static str,
            &'static str,
            Option<&'static str>,
            bool,
            &'static str,
            &'static str,
        );
        let cases: &[Case] = &[
            (
                "meta",
                "https://api.meta.ai/v1",
                Some("MODEL_API_KEY"),
                true,
                "muse-wonder-1.0",
                "muse-ember-1.0",
            ),
            (
                "openai",
                "https://api.openai.com/v1",
                Some("OPENAI_API_KEY"),
                true,
                "gpt-4o",
                "gpt-4o-mini",
            ),
            (
                "openrouter",
                "https://openrouter.ai/api/v1",
                Some("OPENROUTER_API_KEY"),
                true,
                "openai/gpt-4o",
                "openai/gpt-4o-mini",
            ),
            (
                "groq",
                "https://api.groq.com/openai/v1",
                Some("GROQ_API_KEY"),
                true,
                "llama-3.3-70b-versatile",
                "llama-3.1-8b-instant",
            ),
            (
                "together",
                "https://api.together.xyz/v1",
                Some("TOGETHER_API_KEY"),
                true,
                "meta-llama/Llama-3.3-70B-Instruct-Turbo",
                "meta-llama/Meta-Llama-3.1-8B-Instruct-Turbo",
            ),
            (
                "deepseek",
                "https://api.deepseek.com/v1",
                Some("DEEPSEEK_API_KEY"),
                true,
                "deepseek-chat",
                "deepseek-chat",
            ),
            (
                "ollama",
                "http://localhost:11434/v1",
                None,
                false,
                "llama3.1",
                "llama3.2",
            ),
            ("custom", "", None, true, "gpt-4o", "gpt-4o-mini"),
        ];
        assert_eq!(PROVIDER_PRESETS.len(), cases.len());
        for (id, base, env, req, strong, light) in cases {
            let p = provider_preset(id).unwrap();
            assert_eq!(p.base_url.unwrap_or(""), *base, "{id}");
            assert_eq!(p.api_key_env, *env, "{id}");
            assert_eq!(p.requires_api_key, *req, "{id}");
            assert_eq!(p.strong_model, *strong, "{id}");
            assert_eq!(p.light_model, *light, "{id}");
            let c = ProviderConfig::from_env_map(&map(&[
                ("PAIRPROOF_PROVIDER", id),
                ("PAIRPROOF_API_KEY", "k"),
                (
                    "PAIRPROOF_BASE_URL",
                    if id == &"custom" {
                        "https://gw.test/v1"
                    } else {
                        ""
                    },
                ),
            ]));
            // empty base override is unset, so non-custom presets still resolve
            let c = c.unwrap();
            assert_eq!(c.provider, *id);
            if *id != "custom" {
                assert_eq!(c.base_url, *base, "{id}");
            }
        }
    }
    #[test]
    fn precedence_provider_env_then_pairproof_overrides() {
        let c = ProviderConfig::from_env_map(&map(&[
            ("PAIRPROOF_PROVIDER", "openai"),
            ("OPENAI_API_KEY", "provider-key"),
            ("MODEL_API_KEY", "meta-key-should-not-leak"),
        ]))
        .unwrap();
        assert_eq!(c.api_key, "provider-key");
        assert_eq!(c.base_url, "https://api.openai.com/v1");
        assert_eq!(c.strong_model, "gpt-4o");
        let c = ProviderConfig::from_env_map(&map(&[
            ("PAIRPROOF_PROVIDER", "openai"),
            ("OPENAI_API_KEY", "provider-key"),
            ("PAIRPROOF_API_KEY", "override-key"),
            ("PAIRPROOF_BASE_URL", "https://proxy.test/v1/"),
            ("PAIRPROOF_STRONG_MODEL", "s-x"),
            ("PAIRPROOF_LIGHT_MODEL", "l-x"),
        ]))
        .unwrap();
        assert_eq!(c.api_key, "override-key");
        assert_eq!(c.base_url, "https://proxy.test/v1");
        assert_eq!(
            (c.strong_model.as_str(), c.light_model.as_str()),
            ("s-x", "l-x")
        );
    }
    #[test]
    fn ollama_keyless_and_custom_requires_base() {
        let c = ProviderConfig::from_env_map(&map(&[("PAIRPROOF_PROVIDER", "ollama")])).unwrap();
        assert!(!c.requires_api_key);
        assert_eq!(c.api_key, "");
        assert_eq!(c.base_url, "http://localhost:11434/v1");
        c.validate().unwrap();
        let err = ProviderConfig::from_env_map(&map(&[
            ("PAIRPROOF_PROVIDER", "custom"),
            ("PAIRPROOF_API_KEY", "k"),
        ]))
        .unwrap_err();
        assert!(
            matches!(err, ProviderError::MissingBaseUrl(ref p) if p == "custom"),
            "{err:?}"
        );
        let c = ProviderConfig::from_env_map(&map(&[
            ("PAIRPROOF_PROVIDER", "custom"),
            ("PAIRPROOF_API_KEY", "k"),
            ("PAIRPROOF_BASE_URL", "http://localhost:9999/v1"),
        ]))
        .unwrap();
        assert_eq!(c.base_url, "http://localhost:9999/v1");
    }
    #[test]
    fn debug_redacts_api_key() {
        let c = ProviderConfig::meta_with_key("super-secret-123");
        let d = format!("{c:?}");
        assert!(!d.contains("super-secret-123"), "{d}");
        assert!(d.contains("[REDACTED]"), "{d}");
    }
}
