use std::time::Duration;

use serde_json::Value;
use tokio::time::sleep;

use crate::config::ProviderConfig;

use super::{
    auth_profile_runtime::ProviderAuthProfile,
    contracts::{
        CompletionPayloadMode, ProviderApiError, ProviderCapabilityContract,
        ProviderRuntimeContract, adapt_payload_mode_for_error, parse_provider_api_error,
    },
    failover::{
        ModelRequestError, ProviderFailoverReason, ProviderFailoverStage, build_model_request_error,
    },
    policy,
    request_planner::{
        ModelRequestStatusPlan, classify_model_status_failure_reason_with_capability,
        plan_model_request_status_with_capability, plan_transport_error_retry,
    },
    transport,
};

pub(super) struct ModelRequestRuntime<'a> {
    pub(super) provider: &'a ProviderConfig,
    pub(super) model: &'a str,
    pub(super) runtime_contract: ProviderRuntimeContract,
    pub(super) capability: ProviderCapabilityContract,
    pub(super) auto_model_mode: bool,
    pub(super) auth_profile: &'a ProviderAuthProfile,
    pub(super) endpoint: &'a str,
    pub(super) headers: &'a reqwest::header::HeaderMap,
    pub(super) request_policy: &'a policy::ProviderRequestPolicy,
    pub(super) client: &'a reqwest::Client,
    pub(super) auth_context: &'a transport::RequestAuthContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelStatusOutcome {
    Retry { delay_ms: u64, next_backoff_ms: u64 },
    TryNextModel,
    Fail { reason: ProviderFailoverReason },
}

fn plan_model_status_outcome(
    status_code: u16,
    response_headers: &reqwest::header::HeaderMap,
    api_error: &ProviderApiError,
    attempt: usize,
    request_policy: &policy::ProviderRequestPolicy,
    backoff_ms: u64,
    auto_model_mode: bool,
    runtime_contract: ProviderRuntimeContract,
    capability: ProviderCapabilityContract,
) -> ModelStatusOutcome {
    match plan_model_request_status_with_capability(
        status_code,
        response_headers,
        api_error,
        attempt,
        request_policy,
        backoff_ms,
        auto_model_mode,
        runtime_contract,
        capability,
    ) {
        ModelRequestStatusPlan::Retry {
            delay_ms,
            next_backoff_ms,
        } => ModelStatusOutcome::Retry {
            delay_ms,
            next_backoff_ms,
        },
        ModelRequestStatusPlan::TryNextModel => ModelStatusOutcome::TryNextModel,
        ModelRequestStatusPlan::Fail => ModelStatusOutcome::Fail {
            reason: classify_model_status_failure_reason_with_capability(
                status_code,
                api_error,
                runtime_contract,
                capability,
            ),
        },
    }
}

pub(super) async fn execute_model_request<T, BuildBody, ParseSuccess, PreStatusError>(
    runtime: ModelRequestRuntime<'_>,
    mut build_body: BuildBody,
    mut parse_success: ParseSuccess,
    missing_shape_fragment: &'static str,
    mut pre_status_error: PreStatusError,
) -> Result<T, ModelRequestError>
where
    BuildBody: FnMut(CompletionPayloadMode) -> Value,
    ParseSuccess: FnMut(&Value) -> Option<T>,
    PreStatusError: FnMut(&ProviderApiError) -> bool,
{
    let mut attempt = 0usize;
    let mut backoff_ms = runtime.request_policy.initial_backoff_ms;
    let mut payload_mode =
        CompletionPayloadMode::default_for_contract(runtime.provider, runtime.runtime_contract);
    let mut tried_payload_modes = vec![payload_mode];

    loop {
        attempt += 1;
        let body = build_body(payload_mode);
        let request_endpoint =
            transport::resolve_request_endpoint(runtime.provider, runtime.endpoint, runtime.model);
        let request_endpoint =
            transport::resolve_request_url(runtime.provider, request_endpoint.as_str(), runtime.auth_context)
                .map_err(|error| {
                    build_model_request_error(
                        format!(
                            "provider request setup failed for model `{model}` on attempt {attempt}/{max_attempts}: {error}",
                            model = runtime.model,
                            max_attempts = runtime.request_policy.max_attempts
                        ),
                        false,
                        ProviderFailoverReason::TransportFailure,
                        ProviderFailoverStage::TransportFailure,
                        runtime.model,
                        attempt,
                        runtime.request_policy.max_attempts,
                        None,
                        None,
                    )
                })?;
        let body_bytes = transport::encode_json_request_body(&body).map_err(|error| {
            build_model_request_error(
                format!(
                    "provider request setup failed for model `{model}` on attempt {attempt}/{max_attempts}: {error}",
                    model = runtime.model,
                    max_attempts = runtime.request_policy.max_attempts
                ),
                false,
                ProviderFailoverReason::TransportFailure,
                ProviderFailoverStage::TransportFailure,
                runtime.model,
                attempt,
                runtime.request_policy.max_attempts,
                None,
                None,
            )
        })?;
        let mut headers = runtime.headers.clone();
        transport::apply_json_request_defaults(&mut headers).map_err(|error| {
            build_model_request_error(
                format!(
                    "provider request setup failed for model `{model}` on attempt {attempt}/{max_attempts}: {error}",
                    model = runtime.model,
                    max_attempts = runtime.request_policy.max_attempts
                ),
                false,
                ProviderFailoverReason::TransportFailure,
                ProviderFailoverStage::TransportFailure,
                runtime.model,
                attempt,
                runtime.request_policy.max_attempts,
                None,
                None,
            )
        })?;
        transport::apply_auth_profile_headers(&mut headers, Some(runtime.auth_profile)).map_err(
            |error| {
                build_model_request_error(
                    format!(
                        "provider request setup failed for model `{model}` on attempt {attempt}/{max_attempts}: {error}",
                        model = runtime.model,
                        max_attempts = runtime.request_policy.max_attempts
                    ),
                    false,
                    ProviderFailoverReason::TransportFailure,
                    ProviderFailoverStage::TransportFailure,
                    runtime.model,
                    attempt,
                    runtime.request_policy.max_attempts,
                    None,
                    None,
                )
            },
        )?;
        let req = runtime
            .client
            .post(request_endpoint.as_str())
            .headers(headers)
            .body(body_bytes.clone())
            .build()
            .map_err(|error| {
                build_model_request_error(
                    format!(
                        "provider request setup failed for model `{model}` on attempt {attempt}/{max_attempts}: {error}",
                        model = runtime.model,
                        max_attempts = runtime.request_policy.max_attempts
                    ),
                    false,
                    ProviderFailoverReason::TransportFailure,
                    ProviderFailoverStage::TransportFailure,
                    runtime.model,
                    attempt,
                    runtime.request_policy.max_attempts,
                    None,
                    None,
                )
            })?;

        match transport::execute_request(
            runtime.client,
            req,
            Some(body_bytes.as_slice()),
            runtime.auth_context,
            Some(transport::BedrockService::Runtime),
        )
        .await
        {
            Ok(response) => {
                let status = response.status();
                let response_headers = response.headers().clone();
                let response_body = transport::decode_response_body(response)
                    .await
                    .map_err(|error| {
                        build_model_request_error(
                            format!(
                                "provider response decode failed for model `{model}` on attempt {attempt}/{max_attempts}: {error}",
                                model = runtime.model,
                                max_attempts = runtime.request_policy.max_attempts
                            ),
                            false,
                            ProviderFailoverReason::ResponseDecodeFailure,
                            ProviderFailoverStage::ResponseDecode,
                            runtime.model,
                            attempt,
                            runtime.request_policy.max_attempts,
                            None,
                            None,
                        )
                    })?;

                if status.is_success() {
                    let parsed = parse_success(&response_body).ok_or_else(|| {
                        build_model_request_error(
                            format!(
                                "provider response missing {missing_shape_fragment} for model `{model}` on attempt {attempt}/{max_attempts}: {response_body}",
                                model = runtime.model,
                                max_attempts = runtime.request_policy.max_attempts
                            ),
                            false,
                            ProviderFailoverReason::ResponseShapeInvalid,
                            ProviderFailoverStage::ResponseShapeInvalid,
                            runtime.model,
                            attempt,
                            runtime.request_policy.max_attempts,
                            None,
                            None,
                        )
                    })?;
                    return Ok(parsed);
                }

                let api_error = parse_provider_api_error(&response_body);
                if pre_status_error(&api_error) {
                    continue;
                }
                if let Some(next_mode) = adapt_payload_mode_for_error(
                    payload_mode,
                    runtime.provider,
                    runtime.runtime_contract,
                    &api_error,
                ) && !tried_payload_modes.contains(&next_mode)
                {
                    payload_mode = next_mode;
                    tried_payload_modes.push(next_mode);
                    continue;
                }

                let status_code = status.as_u16();
                match plan_model_status_outcome(
                    status_code,
                    &response_headers,
                    &api_error,
                    attempt,
                    runtime.request_policy,
                    backoff_ms,
                    runtime.auto_model_mode,
                    runtime.runtime_contract,
                    runtime.capability,
                ) {
                    ModelStatusOutcome::Retry {
                        delay_ms,
                        next_backoff_ms,
                    } => {
                        sleep(Duration::from_millis(delay_ms)).await;
                        backoff_ms = next_backoff_ms;
                        continue;
                    }
                    ModelStatusOutcome::TryNextModel => {
                        return Err(build_model_request_error(
                            format!(
                                "model `{}` rejected by provider endpoint; trying next candidate. status {status_code}: {response_body}",
                                runtime.model
                            ),
                            true,
                            ProviderFailoverReason::ModelMismatch,
                            ProviderFailoverStage::ModelCandidateRejected,
                            runtime.model,
                            attempt,
                            runtime.request_policy.max_attempts,
                            Some(status_code),
                            Some(api_error.clone()),
                        ));
                    }
                    ModelStatusOutcome::Fail { reason } => {
                        return Err(build_model_request_error(
                            format!(
                                "provider returned status {status_code} for model `{}` on attempt {attempt}/{max_attempts}: {response_body}",
                                runtime.model,
                                max_attempts = runtime.request_policy.max_attempts
                            ),
                            false,
                            reason,
                            ProviderFailoverStage::StatusFailure,
                            runtime.model,
                            attempt,
                            runtime.request_policy.max_attempts,
                            Some(status_code),
                            Some(api_error.clone()),
                        ));
                    }
                }
            }
            Err(transport::RequestExecutionError::Transport(error)) => {
                if let Some((retry_delay_ms, next_backoff_ms)) =
                    plan_transport_error_retry(attempt, runtime.request_policy, &error, backoff_ms)
                {
                    sleep(Duration::from_millis(retry_delay_ms)).await;
                    backoff_ms = next_backoff_ms;
                    continue;
                }
                return Err(build_model_request_error(
                    format!(
                        "provider request failed for model `{}` on attempt {attempt}/{max_attempts}: {error}",
                        runtime.model,
                        max_attempts = runtime.request_policy.max_attempts
                    ),
                    false,
                    ProviderFailoverReason::TransportFailure,
                    ProviderFailoverStage::TransportFailure,
                    runtime.model,
                    attempt,
                    runtime.request_policy.max_attempts,
                    None,
                    None,
                ));
            }
            Err(transport::RequestExecutionError::Setup(error)) => {
                return Err(build_model_request_error(
                    format!(
                        "provider request setup failed for model `{}` on attempt {attempt}/{max_attempts}: {error}",
                        runtime.model,
                        max_attempts = runtime.request_policy.max_attempts
                    ),
                    false,
                    ProviderFailoverReason::TransportFailure,
                    ProviderFailoverStage::TransportFailure,
                    runtime.model,
                    attempt,
                    runtime.request_policy.max_attempts,
                    None,
                    None,
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::contracts::provider_runtime_contract;

    struct ModelStatusCase {
        status_code: u16,
        attempt: usize,
        auto_model_mode: bool,
        api_error: ProviderApiError,
        expected: ModelStatusOutcome,
    }

    #[test]
    fn plan_model_status_outcome_matrix_is_stable() {
        let provider = ProviderConfig::default();
        let request_policy = policy::ProviderRequestPolicy::from_config(&provider);
        let runtime_contract = provider_runtime_contract(&provider);
        let headers = reqwest::header::HeaderMap::new();
        let backoff_ms = request_policy.initial_backoff_ms;

        let cases = vec![
            ModelStatusCase {
                status_code: 429,
                attempt: 1,
                auto_model_mode: true,
                api_error: ProviderApiError::default(),
                expected: ModelStatusOutcome::Retry {
                    delay_ms: backoff_ms,
                    next_backoff_ms: policy::next_backoff_ms(
                        backoff_ms,
                        request_policy.max_backoff_ms,
                    ),
                },
            },
            ModelStatusCase {
                status_code: 404,
                attempt: 1,
                auto_model_mode: true,
                api_error: ProviderApiError {
                    code: Some("model_not_found".to_owned()),
                    ..ProviderApiError::default()
                },
                expected: ModelStatusOutcome::TryNextModel,
            },
            ModelStatusCase {
                status_code: 404,
                attempt: 1,
                auto_model_mode: false,
                api_error: ProviderApiError {
                    code: Some("model_not_found".to_owned()),
                    ..ProviderApiError::default()
                },
                expected: ModelStatusOutcome::Fail {
                    reason: ProviderFailoverReason::ModelMismatch,
                },
            },
            ModelStatusCase {
                status_code: 503,
                attempt: request_policy.max_attempts,
                auto_model_mode: true,
                api_error: ProviderApiError::default(),
                expected: ModelStatusOutcome::Fail {
                    reason: ProviderFailoverReason::ProviderOverloaded,
                },
            },
            ModelStatusCase {
                status_code: 400,
                attempt: 1,
                auto_model_mode: true,
                api_error: ProviderApiError {
                    message: Some("unsupported parameter: max_completion_tokens".to_owned()),
                    ..ProviderApiError::default()
                },
                expected: ModelStatusOutcome::Fail {
                    reason: ProviderFailoverReason::PayloadIncompatible,
                },
            },
        ];

        for case in cases {
            let observed = plan_model_status_outcome(
                case.status_code,
                &headers,
                &case.api_error,
                case.attempt,
                &request_policy,
                backoff_ms,
                case.auto_model_mode,
                runtime_contract,
                runtime_contract.capability,
            );
            assert_eq!(
                observed, case.expected,
                "unexpected status outcome for status={}, attempt={}, auto_mode={}, error={:?}",
                case.status_code, case.attempt, case.auto_model_mode, case.api_error
            );
        }
    }
}
