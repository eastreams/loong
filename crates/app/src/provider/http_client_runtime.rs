use std::time::Duration;

use crate::CliResult;

use super::policy;

pub(super) fn build_http_client(
    request_policy: &policy::ProviderRequestPolicy,
) -> CliResult<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_millis(request_policy.timeout_ms))
        .build()
        .map_err(|error| format!("build provider http client failed: {error}"))
}
