#[cfg(test)]
use std::time::Duration;
#[cfg(test)]
use std::time::Instant;

use serde_json::Value;
#[cfg(test)]
use tokio::time::sleep;

use crate::CliResult;

use super::config::LoongClawConfig;
#[cfg(test)]
use super::config::{ProviderKind, ProviderProfileHealthModeConfig};

mod copilot_auth;
mod auth_profile_runtime;
mod capability_profile_runtime;
mod catalog_executor;
mod catalog_query_runtime;
mod catalog_runtime;
mod contracts;
mod failover;
mod failover_telemetry_runtime;
mod http_client_runtime;
mod model_candidate_cooldown_runtime;
mod model_candidate_resolver_runtime;
mod policy;
mod profile_health_policy;
mod profile_health_runtime;
mod profile_state_backend;
mod profile_state_store;
mod provider_keyspace;
mod provider_validation_runtime;
mod request_dispatch_runtime;
mod request_executor;
mod request_failover_runtime;
mod request_message_runtime;
mod request_payload_runtime;
mod request_planner;
mod request_session_runtime;
mod runtime_binding;
mod shape;
mod transport;

pub(crate) use failover::parse_provider_failover_snapshot_payload;
pub use copilot_auth::device_code_login as copilot_device_code_login;
pub use request_executor::{StreamingCallbackData, StreamingTokenCallback};
pub use runtime_binding::ProviderRuntimeBinding;
pub use shape::{
    extract_provider_turn, extract_provider_turn_with_scope,
    extract_provider_turn_with_scope_and_messages,
};

pub fn is_auth_style_failure_message(message: &str) -> bool {
    matches!(
        profile_health_policy::classify_profile_failure_reason_from_message(message),
        ProviderFailoverReason::AuthRejected
    )
}

#[cfg(test)]
use auth_profile_runtime::{ProviderAuthProfile, resolve_provider_auth_profiles};
use catalog_query_runtime::fetch_available_models_with_profiles;
#[cfg(test)]
use catalog_runtime::{
    ModelCatalogCache, clear_model_catalog_singleflight_slot,
    fetch_model_catalog_singleflight_with_timeouts, has_model_catalog_singleflight_slot,
};
#[cfg(test)]
use catalog_runtime::{ModelCatalogCacheLookup, fetch_model_catalog_singleflight};
#[cfg(test)]
use contracts::ProviderApiError;
#[cfg(test)]
use contracts::ProviderFeatureFamily;
use contracts::provider_runtime_contract;
#[cfg(test)]
use contracts::should_disable_tool_schema_for_error;
#[cfg(test)]
use contracts::{CompletionPayloadMode, ReasoningField, TemperatureField, TokenLimitField};
#[cfg(test)]
use contracts::{
    PayloadAdaptationAxis, ProviderReasoningExtraBodyMode, ProviderToolSchemaMode,
    ProviderTransportMode,
};
#[cfg(test)]
use contracts::{adapt_payload_mode_for_error, parse_provider_api_error};
#[cfg(test)]
use contracts::{classify_payload_adaptation_axis, should_try_next_model_on_error};
use failover::ProviderFailoverReason;
#[cfg(test)]
use failover::ProviderFailoverSnapshot;
#[cfg(test)]
use failover::{ProviderFailoverStage, build_model_request_error};
#[cfg(test)]
use failover_telemetry_runtime::{
    provider_failover_metrics_snapshot, record_provider_failover_audit_event,
};
#[cfg(test)]
use model_candidate_cooldown_runtime::ModelCandidateCooldownCache;
#[cfg(test)]
use model_candidate_cooldown_runtime::prioritize_model_candidates_by_cooldown;
#[cfg(test)]
use model_candidate_cooldown_runtime::{
    ModelCandidateCooldownPolicy, register_model_candidate_cooldown,
};
#[cfg(test)]
use model_candidate_resolver_runtime::rank_model_candidates;
#[cfg(test)]
use profile_health_runtime::{
    ProviderProfileStatePolicy, build_provider_profile_state_policy, mark_provider_profile_failure,
    prioritize_provider_auth_profiles_by_health,
};
#[cfg(all(test, feature = "memory-sqlite"))]
use profile_state_backend::SqliteProviderProfileStateBackend;
#[cfg(test)]
use profile_state_backend::with_provider_profile_states;
#[cfg(test)]
use profile_state_backend::{
    FileProviderProfileStateBackend, ProviderProfileStateBackend,
    ProviderProfileStatePersistOutcome, provider_profile_state_backend,
    provider_profile_state_persistence_metrics_snapshot,
    record_provider_profile_state_persist_outcome,
};
#[cfg(test)]
use profile_state_store::{
    PROVIDER_PROFILE_STATE_SNAPSHOT_VERSION, ProviderProfileStateEntry,
    ProviderProfileStateSnapshotEntry,
};
use profile_state_store::{
    ProviderProfileHealthMode, ProviderProfileStateSnapshot, ProviderProfileStateStore,
    current_unix_timestamp_ms,
};
#[cfg(test)]
use provider_keyspace::build_model_catalog_cache_key;
#[cfg(test)]
use provider_keyspace::build_provider_profile_state_key;
use request_dispatch_runtime::{
    request_completion_with_model, request_turn_streaming_with_model, request_turn_with_model,
};
use request_failover_runtime::request_across_model_candidates;
#[cfg(test)]
use request_payload_runtime::{build_completion_request_body, build_turn_request_body};
use request_session_runtime::prepare_provider_request_session;

#[cfg(test)]
use request_planner::{
    ModelRequestStatusPlan, classify_model_status_failure_reason, plan_model_request_status,
};

#[cfg(test)]
const MODEL_CATALOG_CACHE_MAX_ENTRIES: usize = 32;
#[cfg(test)]
const MODEL_CANDIDATE_COOLDOWN_CACHE_MAX_ENTRIES: usize = 64;

pub fn build_system_message(
    config: &LoongClawConfig,
    include_system_prompt: bool,
) -> Option<Value> {
    request_message_runtime::build_system_message(config, include_system_prompt)
}

pub(crate) use request_message_runtime::build_projected_context_for_session_with_binding;
pub(crate) use request_message_runtime::project_hydrated_memory_context_for_view_with_binding;

pub fn build_messages_for_session(
    config: &LoongClawConfig,
    session_id: &str,
    include_system_prompt: bool,
) -> CliResult<Vec<Value>> {
    request_message_runtime::build_messages_for_session(config, session_id, include_system_prompt)
}

pub async fn request_completion(
    config: &LoongClawConfig,
    messages: &[Value],
    binding: ProviderRuntimeBinding<'_>,
) -> CliResult<String> {
    let session = prepare_provider_request_session(config).await?;
    request_across_model_candidates(
        &config.provider,
        binding,
        &session.auth_profiles,
        session.profile_state_policy.as_ref(),
        &session.model_candidates,
        session.auto_model_mode,
        session.model_candidate_cooldown_policy.as_ref(),
        |model, auto_model_mode, auth_profile| {
            request_completion_with_model(
                config,
                messages,
                model,
                auto_model_mode,
                auth_profile,
                &session.endpoint,
                &session.headers,
                &session.request_policy,
                &session.client,
                &session.auth_context,
            )
        },
    )
    .await
}

pub async fn request_turn(
    config: &LoongClawConfig,
    session_id: &str,
    turn_id: &str,
    messages: &[Value],
    binding: ProviderRuntimeBinding<'_>,
) -> CliResult<crate::conversation::turn_engine::ProviderTurn> {
    request_turn_in_view(
        config,
        session_id,
        turn_id,
        messages,
        &crate::tools::runtime_tool_view(),
        binding,
    )
    .await
}

pub async fn request_turn_in_view(
    config: &LoongClawConfig,
    session_id: &str,
    turn_id: &str,
    messages: &[Value],
    tool_view: &crate::tools::ToolView,
    binding: ProviderRuntimeBinding<'_>,
) -> CliResult<crate::conversation::turn_engine::ProviderTurn> {
    let session = prepare_provider_request_session(config).await?;
    let tool_runtime_config =
        crate::tools::runtime_config::ToolRuntimeConfig::from_loongclaw_config(config, None);
    let runtime_tool_view =
        crate::tools::runtime_tool_view_with_runtime_config(&config.tools, &tool_runtime_config);
    let tool_definitions = if tool_view == &runtime_tool_view {
        crate::tools::provider_tool_definitions_with_config(Some(&tool_runtime_config))
    } else {
        crate::tools::try_provider_tool_definitions_for_view(tool_view)?
    };
    request_across_model_candidates(
        &config.provider,
        binding,
        &session.auth_profiles,
        session.profile_state_policy.as_ref(),
        &session.model_candidates,
        session.auto_model_mode,
        session.model_candidate_cooldown_policy.as_ref(),
        |model, auto_model_mode, auth_profile| {
            request_turn_with_model(
                config,
                session_id,
                turn_id,
                messages,
                model,
                auto_model_mode,
                tool_definitions.as_slice(),
                auth_profile,
                &session.endpoint,
                &session.headers,
                &session.request_policy,
                &session.client,
                &session.auth_context,
            )
        },
    )
    .await
}

pub async fn request_turn_streaming(
    config: &LoongClawConfig,
    session_id: &str,
    turn_id: &str,
    messages: &[Value],
    binding: ProviderRuntimeBinding<'_>,
    on_token: crate::provider::request_executor::StreamingTokenCallback,
) -> CliResult<crate::conversation::turn_engine::ProviderTurn> {
    request_turn_streaming_in_view(
        config,
        session_id,
        turn_id,
        messages,
        &crate::tools::runtime_tool_view(),
        binding,
        on_token,
    )
    .await
}

pub(crate) fn supports_turn_streaming_events(config: &LoongClawConfig) -> bool {
    let runtime_contract = provider_runtime_contract(&config.provider);
    runtime_contract.supports_turn_streaming_events()
}

pub async fn request_turn_streaming_in_view(
    config: &LoongClawConfig,
    session_id: &str,
    turn_id: &str,
    messages: &[Value],
    tool_view: &crate::tools::ToolView,
    binding: ProviderRuntimeBinding<'_>,
    on_token: crate::provider::request_executor::StreamingTokenCallback,
) -> CliResult<crate::conversation::turn_engine::ProviderTurn> {
    if !supports_turn_streaming_events(config) {
        return Err("provider transport does not support live turn streaming events".to_owned());
    }

    let session = prepare_provider_request_session(config).await?;
    let tool_runtime_config =
        crate::tools::runtime_config::ToolRuntimeConfig::from_loongclaw_config(config, None);
    let runtime_tool_view =
        crate::tools::runtime_tool_view_with_runtime_config(&config.tools, &tool_runtime_config);
    let tool_definitions = if tool_view == &runtime_tool_view {
        crate::tools::provider_tool_definitions_with_config(Some(&tool_runtime_config))
    } else {
        crate::tools::try_provider_tool_definitions_for_view(tool_view)?
    };
    request_across_model_candidates(
        &config.provider,
        binding,
        &session.auth_profiles,
        session.profile_state_policy.as_ref(),
        &session.model_candidates,
        session.auto_model_mode,
        session.model_candidate_cooldown_policy.as_ref(),
        |model, auto_model_mode, auth_profile| {
            request_turn_streaming_with_model(
                config,
                session_id,
                turn_id,
                messages,
                model,
                auto_model_mode,
                tool_definitions.as_slice(),
                auth_profile,
                &session.endpoint,
                &session.headers,
                &session.request_policy,
                &session.client,
                &session.auth_context,
                on_token.clone(),
            )
        },
    )
    .await
}

pub async fn fetch_available_models(config: &LoongClawConfig) -> CliResult<Vec<String>> {
    fetch_available_models_with_profiles(config).await
}

pub async fn provider_auth_ready(config: &LoongClawConfig) -> bool {
    if config.provider.resolved_auth_secret().is_some() {
        return true;
    }

    for header_name in ["authorization", "x-api-key"] {
        if config
            .provider
            .header_value(header_name)
            .is_some_and(|value| !value.trim().is_empty())
        {
            return true;
        }
    }

    if config.provider.kind == crate::config::ProviderKind::Bedrock
        && let Ok(auth_context) = transport::resolve_request_auth_context(&config.provider).await
    {
        return auth_context.has_bedrock_sigv4_fallback();
    }

    false
}

#[cfg(test)]
mod tests;
