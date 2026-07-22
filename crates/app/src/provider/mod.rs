#[cfg(test)]
use std::time::Duration;
#[cfg(test)]
use std::time::Instant;

use serde_json::Value;
#[cfg(test)]
use tokio::time::sleep;

use crate::CliResult;

use super::config::LoongConfig;
#[cfg(test)]
use super::config::{ProviderKind, ProviderProfileHealthModeConfig};

mod auth_profile_runtime;
mod capability_profile_runtime;
mod catalog_executor;
mod catalog_query_runtime;
mod catalog_runtime;
mod contracts;
mod copilot_auth;
mod failover;
mod failover_telemetry_runtime;
mod http_client_runtime;
#[cfg(test)]
mod mock_transport;
mod model_candidate_cooldown_runtime;
mod model_candidate_resolver_runtime;
mod model_catalog_runtime;
mod native_tool_surface;
mod policy;
mod profile_health_policy;
mod profile_health_runtime;
mod profile_state_backend;
mod profile_state_store;
mod provider_keyspace;
mod provider_runtime_status;
mod provider_validation_runtime;
mod rate_limit;
mod request_dispatch_runtime;
mod request_executor;
mod request_failover_runtime;
mod request_message_runtime;
mod request_payload_runtime;
mod request_planner;
mod request_session_runtime;
mod response_debug_context;
mod shape;
mod sse;
mod transport;
mod transport_profile_runtime;
mod transport_trait;

pub use copilot_auth::device_code_login as copilot_device_code_login;
pub use failover::parse_provider_failover_snapshot_payload;
pub use failover_telemetry_runtime::ProviderFailoverMetricsSnapshot;
pub use http_client_runtime::ProviderHttpClientRuntimeMetricsSnapshot;
pub use model_catalog_runtime::{
    ProviderModelCatalogEntry, default_reasoning_effort_for_model,
    effective_default_reasoning_effort_for_entry, effective_supported_reasoning_efforts_for_entry,
    reasoning_effort_description_for_entry, supported_reasoning_efforts_for_model,
};
pub use provider_runtime_status::{
    ProviderToolSchemaReadiness, fetch_available_models, is_auth_style_failure_message,
    provider_auth_ready, provider_failover_metrics_snapshot,
    provider_http_client_runtime_metrics_snapshot, provider_tool_schema_readiness,
    supports_turn_streaming_events,
};
pub use rate_limit::RateLimitObservation;
pub use request_executor::{
    ProviderRetryProgress, ProviderRetryProgressCallback, StreamingCallbackData,
    StreamingTokenCallback,
};
pub use response_debug_context::ProviderResponseDebugContext;
pub use shape::{extract_provider_turn, extract_provider_turn_with_scope};

#[cfg(test)]
use auth_profile_runtime::{ProviderAuthProfile, resolve_provider_auth_profiles};
use catalog_query_runtime::fetch_model_catalog_with_profiles;
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
use failover::build_model_request_error_with_rate_limit;
#[cfg(test)]
use failover::{ProviderFailoverStage, build_model_request_error};
#[cfg(test)]
use failover_telemetry_runtime::record_provider_failover_audit_event;
#[cfg(test)]
use model_candidate_cooldown_runtime::ModelCandidateCooldownCache;
#[cfg(test)]
use model_candidate_cooldown_runtime::prioritize_model_candidates_by_cooldown;
#[cfg(test)]
use model_candidate_cooldown_runtime::{
    ModelCandidateCooldownPolicy, register_model_candidate_cooldown,
    resolve_model_candidate_cooldown_duration,
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

pub async fn build_system_message(
    config: &LoongConfig,
    include_system_prompt: bool,
    ctx: &crate::Context<'_>,
) -> CliResult<Option<Value>> {
    request_message_runtime::build_system_message(config, include_system_prompt, ctx).await
}

pub fn native_query_search_label(config: &LoongConfig) -> Option<String> {
    native_tool_surface::provider_tool_surface(config).native_query_search_label()
}

pub fn native_query_search_active(config: &LoongConfig) -> bool {
    native_tool_surface::provider_tool_surface(config).native_query_search_active()
}

#[cfg(test)]
pub(crate) use request_message_runtime::build_projected_context_for_session;
#[cfg(feature = "memory-sqlite")]
pub(crate) use request_message_runtime::project_stage_envelope_with_context;

pub async fn build_messages_for_session(
    config: &LoongConfig,
    include_system_prompt: bool,
    ctx: &crate::Context<'_>,
) -> CliResult<Vec<Value>> {
    request_message_runtime::build_messages_for_session(config, include_system_prompt, ctx).await
}

pub async fn request_completion(
    config: &LoongConfig,
    messages: &[Value],
    ctx: &crate::Context<'_>,
) -> CliResult<String> {
    request_completion_with_retry_progress(config, messages, ctx, None).await
}

pub async fn request_completion_with_retry_progress(
    config: &LoongConfig,
    messages: &[Value],
    ctx: &crate::Context<'_>,
    retry_progress: ProviderRetryProgressCallback,
) -> CliResult<String> {
    let session = prepare_provider_request_session(config).await?;
    request_across_model_candidates(
        &config.provider,
        ctx,
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
                &session.request_policy,
                &session.client,
                &session.auth_context,
                retry_progress.clone(),
            )
        },
    )
    .await
}

pub async fn request_turn(
    config: &LoongConfig,
    turn_id: &str,
    messages: &[Value],
    ctx: &crate::Context<'_>,
) -> CliResult<crate::conversation::turn_engine::ProviderTurn> {
    request_turn_with_retry_progress(config, turn_id, messages, ctx, None).await
}

pub async fn request_turn_with_retry_progress(
    config: &LoongConfig,
    turn_id: &str,
    messages: &[Value],
    ctx: &crate::Context<'_>,
    retry_progress: ProviderRetryProgressCallback,
) -> CliResult<crate::conversation::turn_engine::ProviderTurn> {
    let session_id = ctx.session().session_id();
    let tool_view = &ctx.session().tool_view;
    let session = prepare_provider_request_session(config).await?;
    let provider_tool_surface = native_tool_surface::provider_tool_surface(config);
    let surface_plan = provider_tool_surface
        .materialize(ctx.runtime(), tool_view, ctx.tool_runtime_config())
        .map_err(|error| error.to_string())?;
    let tool_surface = surface_plan.request;
    request_across_model_candidates(
        &config.provider,
        ctx,
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
                &tool_surface,
                auth_profile,
                &session.request_policy,
                &session.client,
                &session.auth_context,
                retry_progress.clone(),
            )
        },
    )
    .await
}

pub async fn request_turn_streaming(
    config: &LoongConfig,
    turn_id: &str,
    messages: &[Value],
    ctx: &crate::Context<'_>,
    on_token: crate::provider::request_executor::StreamingTokenCallback,
) -> CliResult<crate::conversation::turn_engine::ProviderTurn> {
    request_turn_streaming_with_retry_progress(config, turn_id, messages, ctx, on_token, None).await
}

pub async fn request_turn_streaming_with_retry_progress(
    config: &LoongConfig,
    turn_id: &str,
    messages: &[Value],
    ctx: &crate::Context<'_>,
    on_token: crate::provider::request_executor::StreamingTokenCallback,
    retry_progress: ProviderRetryProgressCallback,
) -> CliResult<crate::conversation::turn_engine::ProviderTurn> {
    if !supports_turn_streaming_events(config) {
        return Err("provider transport does not support live turn streaming events".to_owned());
    }

    let session_id = ctx.session().session_id();
    let tool_view = &ctx.session().tool_view;
    let session = prepare_provider_request_session(config).await?;
    let provider_tool_surface = native_tool_surface::provider_tool_surface(config);
    let surface_plan = provider_tool_surface
        .materialize(ctx.runtime(), tool_view, ctx.tool_runtime_config())
        .map_err(|error| error.to_string())?;
    let tool_surface = surface_plan.request;
    request_across_model_candidates(
        &config.provider,
        ctx,
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
                &tool_surface,
                auth_profile,
                &session.request_policy,
                &session.client,
                &session.auth_context,
                on_token.clone(),
                retry_progress.clone(),
            )
        },
    )
    .await
}

pub async fn fetch_model_catalog(
    config: &LoongConfig,
) -> CliResult<Vec<ProviderModelCatalogEntry>> {
    fetch_model_catalog_with_profiles(config).await
}

#[cfg(test)]
mod tests;
