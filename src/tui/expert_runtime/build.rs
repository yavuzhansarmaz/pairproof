//! Expert runtime construction: Meta client, context extraction,
//! and sandbox-backed runtime build.

use super::*;

/// Builds the expert-visible note for a widened selection report.
pub(crate) fn widened_note(report: &FallbackReport) -> Option<String> {
    if !report.widened {
        return None;
    }
    let tags = if report.tags_used.is_empty() {
        "unfiltered".to_string()
    } else {
        report.tags_used.join(", ")
    };
    Some(format!("selection widened beyond topics ({tags})."))
}

/// Builds an LLM client for the selected provider (Meta by default):
/// env config first, else a stored key recorded for that same
/// provider filled into the env-derived preset. A stored key for a
/// different provider is never reused. Keyless providers (ollama)
/// succeed with an empty key.
pub(crate) fn provider_client() -> Result<crate::llm::LlmClient, String> {
    if let Ok(client) = crate::llm::LlmClient::from_env() {
        return Ok(client);
    }
    let preset = crate::llm::ProviderConfig::preset_from_env().map_err(|e| e.to_string())?;
    if !preset.requires_api_key {
        return Ok(crate::llm::LlmClient::from_config(preset));
    }
    let path =
        crate::tui::config::default_api_key_path().ok_or_else(|| "no API key path".to_string())?;
    let key = crate::tui::config::resolve_api_key_for_provider(&path, &preset.provider)
        .map_err(|e| e.to_string())?;
    Ok(crate::llm::LlmClient::from_config(
        preset.with_api_key(&key),
    ))
}

/// Backwards-compat name for [`provider_client`].
pub(crate) fn meta_client() -> Result<crate::llm::LlmClient, String> {
    provider_client()
}

/// Maps an extraction result to runtime context (test seam for the build path).
pub fn expert_context_from_extraction(
    result: &ExtractionResult,
) -> (Vec<String>, String, Language) {
    (
        result.topics.clone(),
        result.level.clone(),
        result.proposed_language.unwrap_or(Language::Python),
    )
}
/// Resolves Setup pastes to stored documents plus runtime context.
async fn resolve_expert_context(
    jd: &str,
    notes: &str,
) -> Result<(Option<ExpertDocuments>, Vec<String>, String, Language), String> {
    if jd.trim().is_empty() {
        return Ok((None, Vec::new(), String::new(), Language::Python));
    }
    let docs = prepare_documents(jd, notes).map_err(|e| e.to_string())?;
    let proposed = crate::ingest::propose_context(&meta_client()?, &docs)
        .await
        .map_err(|e| format!("extraction failed: {e}"))?;
    Ok((
        Some(docs),
        proposed.topics.clone(),
        proposed.level.clone(),
        proposed.language.proposed().unwrap_or(Language::Python),
    ))
}

/// Derives runtime context from an already-reviewed proposal, so the
/// Proposal screen and the built runtime share one extraction.
fn context_from_proposal(
    proposal: &crate::ingest::ProposedContext,
) -> (Vec<String>, String, Language) {
    (
        proposal.topics.clone(),
        proposal.level.clone(),
        proposal.language.proposed().unwrap_or(Language::Python),
    )
}

/// Builds the real expert runtime from the Setup pastes.
pub(crate) async fn build_expert_runtime(
    now: u64,
    jd: &str,
    notes: &str,
    proposal: Option<&crate::ingest::ProposedContext>,
) -> Result<ExpertRuntime, String> {
    use crate::connection::{Endpoint, EphemeralUsername, Secret};
    use crate::leetcode::selection::auto_pick_easy;
    use crate::leetcode::LeetCodeClient;
    use crate::orchestration::Clock;
    use crate::orchestration::{
        ControlIo, DockerControlIo, LeetcodeStatementFetcher, MetaApproachGenerator,
        MetaChatTransport, MetaCodingTransport, MetaQuestionGrader, MetaShadowEvaluator,
        ShadowEvaluator,
    };
    use crate::sandbox::lifecycle::{create_and_provision, destroy, rotate_secret};
    let (docs, topics, level, language) = match proposal {
        Some(proposed) => {
            let docs = if jd.trim().is_empty() {
                None
            } else {
                Some(prepare_documents(jd, notes).map_err(|e| e.to_string())?)
            };
            let (topics, level, language) = context_from_proposal(proposed);
            (docs, topics, level, language)
        }
        None => resolve_expert_context(jd, notes).await?,
    };
    let leetcode = LeetCodeClient::default();
    let mut rng = rand::thread_rng();
    let topic_refs: Vec<&str> = topics.iter().map(String::as_str).collect();
    let easy = auto_pick_easy(&leetcode, &topic_refs, &mut rng)
        .await
        .map_err(|e| e.to_string())?;
    let session_id = format_session_id(std::process::id(), now);
    let port = select_sandbox_port().ok_or_else(|| "no free sandbox port".to_string())?;
    // Automatic direct access: public address, NAT-PMP, or UPnP mapping.
    // When no automatic path exists (for example CGNAT), fall back to the
    // loopback endpoint so same-machine and manual-tunnel use still work.
    let (endpoint, mut port_mapping) =
        match crate::sandbox::nat::establish_direct_access(port).await {
            Some(direct) => direct.into_parts(),
            None => (
                Endpoint::new("127.0.0.1", port).map_err(|e| e.to_string())?,
                None,
            ),
        };
    let username = EphemeralUsername::generate();
    let secret0 = Secret::generate();
    // Docker always publishes the internal port; the public endpoint
    // (possibly a different, router-assigned external port) only goes
    // into the connection string.
    let sandbox_endpoint = Endpoint::new("127.0.0.1", port).map_err(|e| e.to_string())?;
    let (handle, fingerprint) =
        match create_and_provision(&session_id, &sandbox_endpoint, &username, &secret0).await {
            Ok(pair) => pair,
            Err(err) => {
                if let Some(mapping) = port_mapping.take() {
                    mapping.close().await;
                }
                return Err(err.to_string());
            }
        };
    let built: Result<ExpertRuntime, String> = async {
        let control_io = DockerControlIo::new(handle.clone());
        control_io
            .setup_dirs(username.as_str())
            .await
            .map_err(|e| e.to_string())?;
        let deps = ExpertRuntimeDeps {
            control_io: Box::new(control_io),
            hello: Box::new(DockerHelloSource::new(handle.clone())),
            fetcher: Arc::new(LeetcodeStatementFetcher::new(LeetCodeClient::default())),
            generator: Arc::new(MetaApproachGenerator::new(meta_client()?)),
            coding: Arc::new(MetaCodingTransport::new(meta_client()?)),
            sandbox_io: Arc::new(crate::orchestration::DockerSandboxIo::new(
                handle.clone(),
                username.clone(),
            )),
            chat: Arc::new(MetaChatTransport::new(meta_client()?)),
            grader: Arc::new(MetaQuestionGrader::new(meta_client()?)),
            leetcode: Some(leetcode),
            sandbox_handle: Some(handle.clone()),
            artifact_root: std::path::PathBuf::from("sessions"),
            topics,
            level,
            sync_phase: SessionSyncPhase::NotStarted,
            shadow_evaluator: meta_client().ok().map(|client| {
                Arc::new(MetaShadowEvaluator::new(client)) as Arc<dyn ShadowEvaluator>
            }),
        };
        let spec = ExpertSessionSpec {
            session_id: session_id.clone(),
            language,
            easy,
            endpoint: endpoint.clone(),
            username: username.clone(),
            fingerprint,
        };
        let fresh_now = crate::orchestration::SystemClock.now_ms();
        let mut runtime = ExpertRuntime::new(spec, deps, fresh_now)
            .await
            .map_err(|e| e.to_string())?;
        runtime.set_clock(Arc::new(crate::orchestration::SystemClock));
        if let Ok(client) = meta_client() {
            let cfg = client.config();
            runtime.set_model_provenance(crate::core::ModelProvenance::new(
                &cfg.provider,
                &cfg.strong_model,
                &cfg.light_model,
            ));
        }
        if let Ok(meta) = meta_client() {
            runtime.set_health_probes(
                Arc::new(crate::orchestration::LeetCodeHealthProbe::new(
                    LeetCodeClient::default(),
                )),
                Arc::new(crate::orchestration::MetaHealthProbe::new(meta)),
            );
        }
        if let Some(docs) = &docs {
            store_raw_documents(runtime.layout(), docs).map_err(|e| e.to_string())?;
        }
        let current = runtime
            .current_string(fresh_now)
            .map_err(|e| e.to_string())?;
        rotate_secret(&handle, &username, current.secret())
            .await
            .map_err(|e| e.to_string())?;
        runtime.pushed_window = Some(current.expires_at_ms());
        Ok(runtime)
    }
    .await;
    match built {
        Ok(mut runtime) => {
            runtime.set_port_mapping(port_mapping.take());
            Ok(runtime)
        }
        Err(err) => {
            let _ = destroy(&handle).await;
            if let Some(mapping) = port_mapping.take() {
                mapping.close().await;
            }
            Err(err)
        }
    }
}
