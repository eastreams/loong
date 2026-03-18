use std::time::Duration;

use tokio::time::sleep;

use crate::{CliResult, config::ProviderConfig};

use super::{
    auth_profile_runtime::ProviderAuthProfile,
    http_client_runtime::build_http_client,
    policy::ProviderRequestPolicy,
    request_planner::{plan_status_retry, plan_transport_error_retry},
    shape, transport,
};

pub(super) struct ModelCatalogRequestRuntime<'a> {
    pub(super) provider: &'a ProviderConfig,
    pub(super) headers: &'a reqwest::header::HeaderMap,
    pub(super) request_policy: &'a ProviderRequestPolicy,
    pub(super) auth_profile: Option<&'a ProviderAuthProfile>,
    pub(super) auth_context: &'a transport::RequestAuthContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CatalogStatusOutcome {
    Retry { delay_ms: u64, next_backoff_ms: u64 },
    Fail,
}

fn render_catalog_status_failure_message(
    provider: &ProviderConfig,
    status_code: u16,
    attempt: usize,
    max_attempts: usize,
    response_body: &serde_json::Value,
) -> String {
    let mut message = format!(
        "provider model-list returned status {status_code} on attempt {attempt}/{max_attempts}: {response_body}"
    );
    if matches!(status_code, 401 | 403) {
        if let Some(hint) = provider.auth_guidance_hint() {
            message.push(' ');
            message.push_str(hint.as_str());
        }
        if let Some(hint) = provider.region_endpoint_failure_hint() {
            message.push(' ');
            message.push_str(hint.as_str());
        }
    }
    message
}

fn plan_catalog_status_outcome(
    attempt: usize,
    request_policy: &ProviderRequestPolicy,
    status_code: u16,
    response_headers: &reqwest::header::HeaderMap,
    backoff_ms: u64,
) -> CatalogStatusOutcome {
    match plan_status_retry(
        attempt,
        request_policy,
        status_code,
        response_headers,
        backoff_ms,
    ) {
        Some((delay_ms, next_backoff_ms)) => CatalogStatusOutcome::Retry {
            delay_ms,
            next_backoff_ms,
        },
        None => CatalogStatusOutcome::Fail,
    }
}

pub(super) async fn fetch_available_models_with_policy(
    runtime: ModelCatalogRequestRuntime<'_>,
) -> CliResult<Vec<String>> {
    let endpoint = runtime.provider.models_endpoint();
    let client = build_http_client(runtime.request_policy)?;

    let mut attempt = 0usize;
    let mut backoff_ms = runtime.request_policy.initial_backoff_ms;
    loop {
        attempt += 1;
        let request_endpoint = transport::resolve_request_url(
            runtime.provider,
            endpoint.as_str(),
            runtime.auth_context,
        )?;
        let mut headers = runtime.headers.clone();
        transport::apply_auth_profile_headers(&mut headers, runtime.auth_profile)?;
        let req = client
            .get(request_endpoint.as_str())
            .headers(headers)
            .build()
            .map_err(|error| format!("provider model-list request setup failed: {error}"))?;

        match transport::execute_request(
            &client,
            req,
            None,
            runtime.auth_context,
            Some(transport::BedrockService::ModelCatalog),
        )
        .await
        {
            Ok(response) => {
                let status = response.status();
                let response_headers = response.headers().clone();
                let response_body = transport::decode_response_body(response)
                    .await
                    .map_err(|error| {
                        format!(
                            "provider model-list decode failed on attempt {attempt}/{max_attempts}: {error}",
                            max_attempts = runtime.request_policy.max_attempts
                        )
                    })?;

                if status.is_success() {
                    let models = shape::extract_model_ids(&response_body);
                    if models.is_empty() {
                        return Err(format!(
                            "provider model-list returned no models from endpoint `{endpoint}`"
                        ));
                    }
                    return Ok(models);
                }

                let status_code = status.as_u16();
                match plan_catalog_status_outcome(
                    attempt,
                    runtime.request_policy,
                    status_code,
                    &response_headers,
                    backoff_ms,
                ) {
                    CatalogStatusOutcome::Retry {
                        delay_ms,
                        next_backoff_ms,
                    } => {
                        sleep(Duration::from_millis(delay_ms)).await;
                        backoff_ms = next_backoff_ms;
                        continue;
                    }
                    CatalogStatusOutcome::Fail => {}
                }

                return Err(render_catalog_status_failure_message(
                    runtime.provider,
                    status_code,
                    attempt,
                    runtime.request_policy.max_attempts,
                    &response_body,
                ));
            }
            Err(transport::RequestExecutionError::Transport(error)) => {
                if let Some((retry_delay_ms, next_backoff_ms)) =
                    plan_transport_error_retry(attempt, runtime.request_policy, &error, backoff_ms)
                {
                    sleep(Duration::from_millis(retry_delay_ms)).await;
                    backoff_ms = next_backoff_ms;
                    continue;
                }
                return Err(format!(
                    "provider model-list request failed on attempt {attempt}/{max_attempts}: {error}",
                    max_attempts = runtime.request_policy.max_attempts
                ));
            }
            Err(transport::RequestExecutionError::Setup(error)) => {
                return Err(format!(
                    "provider model-list request setup failed on attempt {attempt}/{max_attempts}: {error}",
                    max_attempts = runtime.request_policy.max_attempts
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CatalogStatusCase {
        attempt: usize,
        status_code: u16,
        expected: CatalogStatusOutcome,
    }

    #[test]
    fn plan_catalog_status_outcome_matrix_is_stable() {
        let provider = ProviderConfig::default();
        let request_policy = ProviderRequestPolicy::from_config(&provider);
        let headers = reqwest::header::HeaderMap::new();
        let backoff_ms = request_policy.initial_backoff_ms;

        let cases = vec![
            CatalogStatusCase {
                attempt: 1,
                status_code: 429,
                expected: CatalogStatusOutcome::Retry {
                    delay_ms: backoff_ms,
                    next_backoff_ms: super::super::policy::next_backoff_ms(
                        backoff_ms,
                        request_policy.max_backoff_ms,
                    ),
                },
            },
            CatalogStatusCase {
                attempt: request_policy.max_attempts,
                status_code: 503,
                expected: CatalogStatusOutcome::Fail,
            },
            CatalogStatusCase {
                attempt: 1,
                status_code: 401,
                expected: CatalogStatusOutcome::Fail,
            },
        ];

        for case in cases {
            let observed = plan_catalog_status_outcome(
                case.attempt,
                &request_policy,
                case.status_code,
                &headers,
                backoff_ms,
            );
            assert_eq!(
                observed, case.expected,
                "unexpected catalog status outcome for status={}, attempt={}",
                case.status_code, case.attempt
            );
        }
    }
}
