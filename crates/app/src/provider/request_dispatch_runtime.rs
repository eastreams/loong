use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

use crate::config::LoongClawConfig;
use crate::tools;

use super::auth_profile_runtime::ProviderAuthProfile;
use super::capability_profile_runtime::ProviderCapabilityProfile;
use super::contracts::{ProviderRuntimeContract, should_disable_tool_schema_for_error};
use super::failover::ModelRequestError;
use super::policy;
use super::request_executor::{ModelRequestRuntime, execute_model_request};
use super::request_payload_runtime::{
    build_completion_request_body_with_capability, build_turn_request_body_with_capability,
};
use super::shape;

#[allow(clippy::too_many_arguments)]
pub(super) async fn request_completion_with_model(
    config: &LoongClawConfig,
    messages: &[Value],
    model: String,
    runtime_contract: ProviderRuntimeContract,
    capability_profile: &ProviderCapabilityProfile,
    auto_model_mode: bool,
    auth_profile: ProviderAuthProfile,
    endpoint: &str,
    headers: &reqwest::header::HeaderMap,
    request_policy: &policy::ProviderRequestPolicy,
    client: &reqwest::Client,
    auth_context: &super::transport::RequestAuthContext,
) -> Result<String, ModelRequestError> {
    let capability = capability_profile.resolve_for_model(model.as_str());
    let runtime = ModelRequestRuntime {
        provider: &config.provider,
        model: model.as_str(),
        runtime_contract,
        capability,
        auto_model_mode,
        auth_profile: &auth_profile,
        endpoint,
        headers,
        request_policy,
        client,
        auth_context,
    };

    execute_model_request(
        runtime,
        |payload_mode| {
            build_completion_request_body_with_capability(
                config,
                messages,
                model.as_str(),
                payload_mode,
                capability,
            )
        },
        shape::extract_message_content,
        "choices[0].message.content",
        |_| false,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn request_turn_with_model(
    config: &LoongClawConfig,
    messages: &[Value],
    model: String,
    runtime_contract: ProviderRuntimeContract,
    capability_profile: &ProviderCapabilityProfile,
    auto_model_mode: bool,
    auth_profile: ProviderAuthProfile,
    endpoint: &str,
    headers: &reqwest::header::HeaderMap,
    request_policy: &policy::ProviderRequestPolicy,
    client: &reqwest::Client,
    auth_context: &super::transport::RequestAuthContext,
) -> Result<crate::conversation::turn_engine::ProviderTurn, ModelRequestError> {
    let tool_definitions = tools::provider_tool_definitions();
    let capability = capability_profile.resolve_for_model(model.as_str());
    let include_tool_schema =
        AtomicBool::new(capability.turn_tool_schema_enabled() && !tool_definitions.is_empty());
    let runtime = ModelRequestRuntime {
        provider: &config.provider,
        model: model.as_str(),
        runtime_contract,
        capability,
        auto_model_mode,
        auth_profile: &auth_profile,
        endpoint,
        headers,
        request_policy,
        client,
        auth_context,
    };

    execute_model_request(
        runtime,
        |payload_mode| {
            build_turn_request_body_with_capability(
                config,
                messages,
                model.as_str(),
                payload_mode,
                capability,
                include_tool_schema.load(Ordering::Relaxed),
                &tool_definitions,
            )
        },
        shape::extract_provider_turn,
        "choices[0].message",
        |api_error| {
            if include_tool_schema.load(Ordering::Relaxed)
                && capability.tool_schema_downgrade_on_unsupported()
                && should_disable_tool_schema_for_error(api_error, runtime_contract)
            {
                include_tool_schema.store(false, Ordering::Relaxed);
                return true;
            }
            false
        },
    )
    .await
}
