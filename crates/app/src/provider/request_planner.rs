use super::{
    contracts::{
        ProviderApiError, ProviderCapabilityContract, ProviderRuntimeContract,
        classify_payload_adaptation_axis, should_disable_tool_schema_for_error,
        should_try_next_model_on_error,
    },
    failover::ProviderFailoverReason,
    policy,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ModelRequestStatusPlan {
    Retry { delay_ms: u64, next_backoff_ms: u64 },
    TryNextModel,
    Fail,
}

pub(super) fn plan_status_retry(
    attempt: usize,
    request_policy: &policy::ProviderRequestPolicy,
    status_code: u16,
    response_headers: &reqwest::header::HeaderMap,
    backoff_ms: u64,
) -> Option<(u64, u64)> {
    if attempt >= request_policy.max_attempts || !policy::should_retry_status(status_code) {
        return None;
    }

    let retry_delay_ms = policy::resolve_status_retry_delay_ms(
        status_code,
        response_headers,
        backoff_ms,
        request_policy.max_backoff_ms,
    );
    let next_backoff_ms = policy::next_backoff_ms(retry_delay_ms, request_policy.max_backoff_ms);
    Some((retry_delay_ms, next_backoff_ms))
}

pub(super) fn plan_transport_error_retry(
    attempt: usize,
    request_policy: &policy::ProviderRequestPolicy,
    error: &reqwest::Error,
    backoff_ms: u64,
) -> Option<(u64, u64)> {
    if attempt >= request_policy.max_attempts || !policy::should_retry_error(error) {
        return None;
    }

    let retry_delay_ms = backoff_ms.min(request_policy.max_backoff_ms);
    let next_backoff_ms = policy::next_backoff_ms(retry_delay_ms, request_policy.max_backoff_ms);
    Some((retry_delay_ms, next_backoff_ms))
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn plan_model_request_status(
    status_code: u16,
    response_headers: &reqwest::header::HeaderMap,
    api_error: &ProviderApiError,
    attempt: usize,
    request_policy: &policy::ProviderRequestPolicy,
    backoff_ms: u64,
    auto_model_mode: bool,
    runtime_contract: ProviderRuntimeContract,
) -> ModelRequestStatusPlan {
    plan_model_request_status_with_capability(
        status_code,
        response_headers,
        api_error,
        attempt,
        request_policy,
        backoff_ms,
        auto_model_mode,
        runtime_contract,
        runtime_contract.capability,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn plan_model_request_status_with_capability(
    status_code: u16,
    response_headers: &reqwest::header::HeaderMap,
    api_error: &ProviderApiError,
    attempt: usize,
    request_policy: &policy::ProviderRequestPolicy,
    backoff_ms: u64,
    auto_model_mode: bool,
    runtime_contract: ProviderRuntimeContract,
    capability: ProviderCapabilityContract,
) -> ModelRequestStatusPlan {
    if let Some((delay_ms, next_backoff_ms)) = plan_status_retry(
        attempt,
        request_policy,
        status_code,
        response_headers,
        backoff_ms,
    ) {
        return ModelRequestStatusPlan::Retry {
            delay_ms,
            next_backoff_ms,
        };
    }

    if auto_model_mode
        && should_try_next_model_for_status_with_capability(
            status_code,
            api_error,
            runtime_contract,
            capability,
        )
    {
        return ModelRequestStatusPlan::TryNextModel;
    }

    ModelRequestStatusPlan::Fail
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn classify_model_status_failure_reason(
    status_code: u16,
    api_error: &ProviderApiError,
    runtime_contract: ProviderRuntimeContract,
) -> ProviderFailoverReason {
    classify_model_status_failure_reason_with_capability(
        status_code,
        api_error,
        runtime_contract,
        runtime_contract.capability,
    )
}

pub(super) fn classify_model_status_failure_reason_with_capability(
    status_code: u16,
    api_error: &ProviderApiError,
    runtime_contract: ProviderRuntimeContract,
    capability: ProviderCapabilityContract,
) -> ProviderFailoverReason {
    if status_code == 429 {
        return ProviderFailoverReason::RateLimited;
    }
    if matches!(status_code, 500 | 502 | 503 | 504) {
        return ProviderFailoverReason::ProviderOverloaded;
    }
    if matches!(status_code, 401 | 403) {
        return ProviderFailoverReason::AuthRejected;
    }
    if should_try_next_model_for_status_with_capability(
        status_code,
        api_error,
        runtime_contract,
        capability,
    ) {
        return ProviderFailoverReason::ModelMismatch;
    }
    if is_payload_incompatible_error(api_error, runtime_contract) {
        return ProviderFailoverReason::PayloadIncompatible;
    }
    ProviderFailoverReason::RequestRejected
}

fn should_try_next_model_for_status_with_capability(
    status_code: u16,
    api_error: &ProviderApiError,
    runtime_contract: ProviderRuntimeContract,
    capability: ProviderCapabilityContract,
) -> bool {
    matches!(status_code, 400 | 404 | 410 | 422)
        && (should_try_next_model_on_error(api_error, runtime_contract)
            || should_switch_model_for_strict_tool_schema(api_error, runtime_contract, capability))
}

fn should_switch_model_for_strict_tool_schema(
    api_error: &ProviderApiError,
    runtime_contract: ProviderRuntimeContract,
    capability: ProviderCapabilityContract,
) -> bool {
    capability.turn_tool_schema_enabled()
        && !capability.tool_schema_downgrade_on_unsupported()
        && should_disable_tool_schema_for_error(api_error, runtime_contract)
}

fn is_payload_incompatible_error(
    api_error: &ProviderApiError,
    runtime_contract: ProviderRuntimeContract,
) -> bool {
    should_disable_tool_schema_for_error(api_error, runtime_contract)
        || classify_payload_adaptation_axis(api_error, &runtime_contract.payload_adaptation)
            .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProviderConfig;
    use crate::provider::contracts::ProviderToolSchemaMode;

    #[test]
    fn classify_model_status_failure_reason_prioritizes_status_over_error_text() {
        let runtime_contract =
            crate::provider::contracts::provider_runtime_contract(&ProviderConfig::default());
        let cases = vec![
            (
                "rate limit beats model mismatch",
                429,
                ProviderApiError {
                    code: Some("model_not_found".to_owned()),
                    message: Some("this endpoint only supports /v1/responses".to_owned()),
                    ..ProviderApiError::default()
                },
                ProviderFailoverReason::RateLimited,
            ),
            (
                "overload beats model mismatch",
                503,
                ProviderApiError {
                    code: Some("unsupported_model".to_owned()),
                    message: Some("model does not exist".to_owned()),
                    ..ProviderApiError::default()
                },
                ProviderFailoverReason::ProviderOverloaded,
            ),
            (
                "auth beats payload incompatibility",
                401,
                ProviderApiError {
                    param: Some("tools".to_owned()),
                    message: Some("unsupported parameter: tools".to_owned()),
                    ..ProviderApiError::default()
                },
                ProviderFailoverReason::AuthRejected,
            ),
            (
                "payload incompatibility classified for schema errors",
                400,
                ProviderApiError {
                    param: Some("tools".to_owned()),
                    message: Some("unsupported parameter: tools".to_owned()),
                    ..ProviderApiError::default()
                },
                ProviderFailoverReason::PayloadIncompatible,
            ),
            (
                "model mismatch classified for model semantic failures",
                404,
                ProviderApiError {
                    code: Some("model_not_found".to_owned()),
                    ..ProviderApiError::default()
                },
                ProviderFailoverReason::ModelMismatch,
            ),
        ];

        for (name, status_code, api_error, expected_reason) in cases {
            assert_eq!(
                classify_model_status_failure_reason(status_code, &api_error, runtime_contract),
                expected_reason,
                "unexpected failure reason for case `{name}` with status={status_code}, error={api_error:?}",
            );
        }
    }

    #[test]
    fn plan_model_request_status_keeps_retry_switch_and_fail_boundaries() {
        let provider = ProviderConfig::default();
        let request_policy = policy::ProviderRequestPolicy::from_config(&provider);
        let runtime_contract = crate::provider::contracts::provider_runtime_contract(&provider);
        let headers = reqwest::header::HeaderMap::new();
        let backoff_ms = request_policy.initial_backoff_ms;

        let retry_case = plan_model_request_status(
            429,
            &headers,
            &ProviderApiError::default(),
            1,
            &request_policy,
            backoff_ms,
            true,
            runtime_contract,
        );
        assert!(matches!(retry_case, ModelRequestStatusPlan::Retry { .. }));

        let switch_case = plan_model_request_status(
            404,
            &headers,
            &ProviderApiError {
                code: Some("model_not_found".to_owned()),
                ..ProviderApiError::default()
            },
            request_policy.max_attempts,
            &request_policy,
            backoff_ms,
            true,
            runtime_contract,
        );
        assert_eq!(switch_case, ModelRequestStatusPlan::TryNextModel);

        let fail_case = plan_model_request_status(
            404,
            &headers,
            &ProviderApiError {
                code: Some("model_not_found".to_owned()),
                ..ProviderApiError::default()
            },
            request_policy.max_attempts,
            &request_policy,
            backoff_ms,
            false,
            runtime_contract,
        );
        assert_eq!(fail_case, ModelRequestStatusPlan::Fail);
    }

    #[test]
    fn plan_model_request_status_uses_capability_to_switch_on_strict_tool_schema_errors() {
        let provider = ProviderConfig::default();
        let request_policy = policy::ProviderRequestPolicy::from_config(&provider);
        let runtime_contract = crate::provider::contracts::provider_runtime_contract(&provider);
        let strict_capability = ProviderCapabilityContract {
            tool_schema_mode: ProviderToolSchemaMode::EnabledStrict,
            ..runtime_contract.capability
        };
        let tool_schema_error = ProviderApiError {
            param: Some("tools".to_owned()),
            message: Some("unsupported parameter: tools".to_owned()),
            ..ProviderApiError::default()
        };

        let strict_plan = plan_model_request_status_with_capability(
            400,
            &reqwest::header::HeaderMap::new(),
            &tool_schema_error,
            request_policy.max_attempts,
            &request_policy,
            request_policy.initial_backoff_ms,
            true,
            runtime_contract,
            strict_capability,
        );
        assert_eq!(strict_plan, ModelRequestStatusPlan::TryNextModel);

        let strict_reason = classify_model_status_failure_reason_with_capability(
            400,
            &tool_schema_error,
            runtime_contract,
            strict_capability,
        );
        assert_eq!(strict_reason, ProviderFailoverReason::ModelMismatch);
    }

    #[test]
    fn planner_status_matrix_remains_stable_across_capability_modes() {
        struct PlannerStatusCase {
            name: &'static str,
            status_code: u16,
            auto_model_mode: bool,
            capability: ProviderCapabilityContract,
            api_error: ProviderApiError,
            expected_plan: ModelRequestStatusPlan,
            expected_reason: ProviderFailoverReason,
        }

        let provider = ProviderConfig::default();
        let request_policy = policy::ProviderRequestPolicy::from_config(&provider);
        let runtime_contract = crate::provider::contracts::provider_runtime_contract(&provider);
        let strict_capability = ProviderCapabilityContract {
            tool_schema_mode: ProviderToolSchemaMode::EnabledStrict,
            ..runtime_contract.capability
        };
        let headers = reqwest::header::HeaderMap::new();

        let cases = vec![
            PlannerStatusCase {
                name: "rate_limited_keeps_status_reason_priority",
                status_code: 429,
                auto_model_mode: true,
                capability: runtime_contract.capability,
                api_error: ProviderApiError::default(),
                expected_plan: ModelRequestStatusPlan::Fail,
                expected_reason: ProviderFailoverReason::RateLimited,
            },
            PlannerStatusCase {
                name: "provider_overloaded_keeps_status_reason_priority",
                status_code: 503,
                auto_model_mode: true,
                capability: runtime_contract.capability,
                api_error: ProviderApiError {
                    code: Some("model_not_found".to_owned()),
                    ..ProviderApiError::default()
                },
                expected_plan: ModelRequestStatusPlan::Fail,
                expected_reason: ProviderFailoverReason::ProviderOverloaded,
            },
            PlannerStatusCase {
                name: "auth_rejected_keeps_status_reason_priority",
                status_code: 401,
                auto_model_mode: true,
                capability: runtime_contract.capability,
                api_error: ProviderApiError {
                    param: Some("tools".to_owned()),
                    message: Some("unsupported parameter: tools".to_owned()),
                    ..ProviderApiError::default()
                },
                expected_plan: ModelRequestStatusPlan::Fail,
                expected_reason: ProviderFailoverReason::AuthRejected,
            },
            PlannerStatusCase {
                name: "strict_tool_schema_switches_model_in_auto_mode",
                status_code: 400,
                auto_model_mode: true,
                capability: strict_capability,
                api_error: ProviderApiError {
                    param: Some("tools".to_owned()),
                    message: Some("unsupported parameter: tools".to_owned()),
                    ..ProviderApiError::default()
                },
                expected_plan: ModelRequestStatusPlan::TryNextModel,
                expected_reason: ProviderFailoverReason::ModelMismatch,
            },
            PlannerStatusCase {
                name: "strict_tool_schema_without_auto_mode_fails_as_model_mismatch",
                status_code: 400,
                auto_model_mode: false,
                capability: strict_capability,
                api_error: ProviderApiError {
                    param: Some("tools".to_owned()),
                    message: Some("unsupported parameter: tools".to_owned()),
                    ..ProviderApiError::default()
                },
                expected_plan: ModelRequestStatusPlan::Fail,
                expected_reason: ProviderFailoverReason::ModelMismatch,
            },
            PlannerStatusCase {
                name: "downgrade_tool_schema_keeps_payload_incompatible_reason",
                status_code: 400,
                auto_model_mode: true,
                capability: runtime_contract.capability,
                api_error: ProviderApiError {
                    param: Some("tools".to_owned()),
                    message: Some("unsupported parameter: tools".to_owned()),
                    ..ProviderApiError::default()
                },
                expected_plan: ModelRequestStatusPlan::Fail,
                expected_reason: ProviderFailoverReason::PayloadIncompatible,
            },
        ];

        for case in cases {
            let observed_plan = plan_model_request_status_with_capability(
                case.status_code,
                &headers,
                &case.api_error,
                request_policy.max_attempts,
                &request_policy,
                request_policy.initial_backoff_ms,
                case.auto_model_mode,
                runtime_contract,
                case.capability,
            );
            assert_eq!(
                observed_plan, case.expected_plan,
                "unexpected plan for `{}` with status={}, auto_mode={}, error={:?}",
                case.name, case.status_code, case.auto_model_mode, case.api_error
            );

            let observed_reason = classify_model_status_failure_reason_with_capability(
                case.status_code,
                &case.api_error,
                runtime_contract,
                case.capability,
            );
            assert_eq!(
                observed_reason, case.expected_reason,
                "unexpected failure reason for `{}` with status={}, error={:?}",
                case.name, case.status_code, case.api_error
            );
        }
    }
}
