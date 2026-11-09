//! Phase 1: ProviderConfig / LlmClient canonical API, Meta default.

use std::collections::HashMap;

use pairproof::llm::{
    LlmClient, MetaApiError, MetaClient, ProviderConfig, ProviderError, API_KEY_ENV,
    DEFAULT_BASE_URL, LIGHT_MODEL, STRONG_MODEL,
};

fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn meta_default_preset_exact_values() {
    let c = ProviderConfig::meta();
    assert_eq!(c.provider, "meta");
    assert_eq!(c.base_url, "https://api.meta.ai/v1");
    assert_eq!(c.base_url, DEFAULT_BASE_URL);
    assert_eq!(c.strong_model, "muse-wonder-1.0");
    assert_eq!(c.light_model, "muse-spark-1.3");
    assert_eq!(c.strong_model, STRONG_MODEL);
    assert_eq!(c.light_model, LIGHT_MODEL);
    assert!(c.requires_api_key);
    assert_eq!(API_KEY_ENV, "MODEL_API_KEY");
}

#[test]
fn env_overrides_and_legacy_fallback() {
    let legacy = ProviderConfig::from_env_map(&map(&[("MODEL_API_KEY", "legacy-key")])).unwrap();
    assert_eq!(legacy.api_key, "legacy-key");
    assert_eq!(legacy.provider, "meta");
    let over = ProviderConfig::from_env_map(&map(&[
        ("PAIRPROOF_PROVIDER", "meta"),
        ("PAIRPROOF_API_KEY", "new-key"),
        ("MODEL_API_KEY", "legacy-key"),
        ("PAIRPROOF_BASE_URL", "https://gw.example/v1/"),
        ("PAIRPROOF_STRONG_MODEL", "s-x"),
        ("PAIRPROOF_LIGHT_MODEL", "l-x"),
    ]))
    .unwrap();
    assert_eq!(over.api_key, "new-key");
    assert_eq!(over.base_url, "https://gw.example/v1");
    assert_eq!(over.strong_model, "s-x");
    assert_eq!(over.light_model, "l-x");
}

#[test]
fn blank_and_unknown_handling_is_typed_no_panic() {
    assert!(matches!(
        ProviderConfig::from_env_map(&map(&[])),
        Err(ProviderError::MissingApiKey)
    ));
    assert!(matches!(
        ProviderConfig::from_env_map(&map(&[
            ("PAIRPROOF_PROVIDER", "nope"),
            ("PAIRPROOF_API_KEY", "k")
        ])),
        Err(ProviderError::UnknownProvider(_))
    ));
    assert!(matches!(
        ProviderConfig::from_env_map(&map(&[
            ("PAIRPROOF_API_KEY", "k"),
            ("PAIRPROOF_BASE_URL", "notaurl")
        ])),
        Err(ProviderError::InvalidBaseUrl(_))
    ));
    // Alias error is the same type.
    let _: Option<MetaApiError> = None;
}

#[test]
fn phase2_presets_precedence_ollama_and_custom() {
    use pairproof::llm::{provider_preset, PROVIDER_PRESETS};
    assert_eq!(PROVIDER_PRESETS.len(), 8);
    for (id, base) in [
        ("openai", "https://api.openai.com/v1"),
        ("openrouter", "https://openrouter.ai/api/v1"),
        ("groq", "https://api.groq.com/openai/v1"),
        ("together", "https://api.together.xyz/v1"),
        ("deepseek", "https://api.deepseek.com/v1"),
        ("ollama", "http://localhost:11434/v1"),
    ] {
        assert_eq!(provider_preset(id).unwrap().base_url, Some(base), "{id}");
    }
    // Provider env key, then PAIRPROOF_* wins.
    let c = ProviderConfig::from_env_map(&map(&[
        ("PAIRPROOF_PROVIDER", "openai"),
        ("OPENAI_API_KEY", "provider-key"),
    ]))
    .unwrap();
    assert_eq!(c.api_key, "provider-key");
    assert_eq!(c.strong_model, "gpt-4o");
    let client = LlmClient::from_config(c);
    assert_eq!(client.provider(), "openai");
    // Ollama is keyless: config and client construction succeed empty.
    let c = ProviderConfig::from_env_map(&map(&[("PAIRPROOF_PROVIDER", "ollama")])).unwrap();
    assert!(!c.requires_api_key && c.api_key.is_empty());
    let client = LlmClient::from_config(c);
    assert_eq!(client.provider(), "ollama");
    assert_eq!(client.base_url(), "http://localhost:11434/v1");
    // Custom without a base url errors cleanly (typed, no panic).
    assert!(matches!(
        ProviderConfig::from_env_map(&map(&[
            ("PAIRPROOF_PROVIDER", "custom"),
            ("PAIRPROOF_API_KEY", "k")
        ])),
        Err(ProviderError::MissingBaseUrl(_))
    ));
}

#[test]
fn redaction_and_alias_client() {
    let cfg = ProviderConfig::meta_with_key("secret-key-9999");
    assert!(!format!("{cfg:?}").contains("secret-key-9999"));
    let client = LlmClient::from_config(cfg);
    let debug = format!("{client:?}");
    assert!(!debug.contains("secret-key-9999"), "{debug}");
    assert!(debug.contains("[REDACTED]"), "{debug}");
    assert_eq!(client.provider(), "meta");
    // Backwards-compat alias constructs the same client type.
    let alias: MetaClient = MetaClient::new("http://127.0.0.1:9", "k");
    assert_eq!(alias.strong_model, STRONG_MODEL);
    assert_eq!(alias.light_model, LIGHT_MODEL);
}
