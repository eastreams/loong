use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::{CliResult, config::LoongClawConfig};

const DEFAULT_OUTBOUND_HTTP_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct ChannelOutboundHttpPolicy {
    pub allow_private_hosts: bool,
}

pub(super) fn outbound_http_policy_from_config(
    config: &LoongClawConfig,
) -> ChannelOutboundHttpPolicy {
    ChannelOutboundHttpPolicy {
        allow_private_hosts: config.outbound_http.allow_private_hosts,
    }
}

pub(super) fn build_outbound_http_client(
    context: &str,
    policy: ChannelOutboundHttpPolicy,
) -> CliResult<reqwest::Client> {
    let resolver = crate::tools::web_http::SsrfSafeResolver {
        allow_private_hosts: policy.allow_private_hosts,
    };
    reqwest::Client::builder()
        .dns_resolver(Arc::new(resolver))
        .redirect(reqwest::redirect::Policy::none())
        .timeout(DEFAULT_OUTBOUND_HTTP_TIMEOUT)
        .no_proxy()
        .build()
        .map_err(|error| format!("build {context} http client failed: {error}"))
}

pub(super) fn validate_outbound_http_target(
    field_name: &str,
    raw_url: &str,
    policy: ChannelOutboundHttpPolicy,
) -> Result<reqwest::Url, String> {
    let trimmed_url = raw_url.trim();
    if trimmed_url.is_empty() {
        return Err(format!("{field_name} is empty"));
    }

    let parsed_url = reqwest::Url::parse(trimmed_url)
        .map_err(|error| format!("{field_name} is invalid: {error}"))?;
    let options = crate::tools::web_http::HttpTargetValidationOptions {
        allow_private_hosts: policy.allow_private_hosts,
        reject_userinfo: true,
        resolve_dns: false,
        enforce_allowed_domains: false,
        allowed_domains: None,
        blocked_domains: None,
    };
    crate::tools::web_http::validate_http_target(&parsed_url, &options, field_name)?;
    Ok(parsed_url)
}

pub(super) async fn read_json_or_text_response(
    response: reqwest::Response,
    context: &str,
) -> CliResult<(reqwest::StatusCode, String, Value)> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("read {context} response failed: {error}"))?;
    let payload =
        serde_json::from_str::<Value>(&body).unwrap_or_else(|_| Value::String(body.clone()));
    Ok((status, body, payload))
}

pub(super) fn response_body_detail(body: &str) -> String {
    let trimmed_body = body.trim();
    if trimmed_body.is_empty() {
        return "empty response body".to_owned();
    }

    trimmed_body.to_owned()
}

pub(super) fn redact_endpoint_status_url(raw_url: &str) -> Option<String> {
    let trimmed_url = raw_url.trim();
    let parsed_url = reqwest::Url::parse(trimmed_url).ok()?;
    let has_userinfo = !parsed_url.username().is_empty() || parsed_url.password().is_some();
    let has_query = parsed_url.query().is_some();
    let has_fragment = parsed_url.fragment().is_some();
    if !has_userinfo && !has_query && !has_fragment {
        return Some(trimmed_url.to_owned());
    }

    let mut redacted_url = parsed_url;
    let _ = redacted_url.set_username("");
    let _ = redacted_url.set_password(None);
    redacted_url.set_query(None);
    redacted_url.set_fragment(None);
    Some(redacted_url.to_string())
}

pub(super) fn redact_generic_webhook_status_url(raw_url: &str) -> Option<String> {
    let trimmed_url = raw_url.trim();
    let parsed_url = reqwest::Url::parse(trimmed_url).ok()?;
    let mut redacted_url = parsed_url;
    let _ = redacted_url.set_username("");
    let _ = redacted_url.set_password(None);
    redacted_url.set_path("/");
    redacted_url.set_query(None);
    redacted_url.set_fragment(None);
    Some(redacted_url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    fn spawn_test_http_server(
        response: String,
        accept_timeout: Duration,
    ) -> Result<(String, std::thread::JoinHandle<Result<bool, String>>), String> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|error| format!("bind test server: {error}"))?;
        let address = listener
            .local_addr()
            .map_err(|error| format!("read test server address: {error}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("set test server nonblocking mode: {error}"))?;

        let handle = std::thread::spawn(move || -> Result<bool, String> {
            let deadline = std::time::Instant::now() + accept_timeout;

            loop {
                match listener.accept() {
                    Ok((mut stream, _peer)) => {
                        let mut request_buffer = [0_u8; 1024];
                        let _ = stream.read(&mut request_buffer);
                        stream
                            .write_all(response.as_bytes())
                            .map_err(|error| format!("write test server response: {error}"))?;
                        stream
                            .flush()
                            .map_err(|error| format!("flush test server response: {error}"))?;
                        return Ok(true);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if std::time::Instant::now() >= deadline {
                            return Ok(false);
                        }
                        std::thread::park_timeout(Duration::from_millis(10));
                    }
                    Err(error) => {
                        return Err(format!("accept test server connection: {error}"));
                    }
                }
            }
        });

        let url = format!("http://127.0.0.1:{}/", address.port());
        Ok((url, handle))
    }

    #[test]
    fn outbound_http_target_rejects_userinfo() {
        let policy = ChannelOutboundHttpPolicy::default();
        let error = validate_outbound_http_target(
            "webhook.endpoint_url",
            "https://user:pass@example.com/hook",
            policy,
        )
        .expect_err("credential-bearing urls should be rejected");

        assert!(error.contains("must not embed credentials"));
    }

    #[test]
    fn outbound_http_target_blocks_private_hosts_by_default() {
        let policy = ChannelOutboundHttpPolicy::default();
        let error =
            validate_outbound_http_target("signal.service_url", "http://127.0.0.1:8080", policy)
                .expect_err("private hosts should be blocked by default");

        assert!(error.contains("private or special-use"));
    }

    #[test]
    fn outbound_http_target_allows_private_hosts_when_policy_is_enabled() {
        let policy = ChannelOutboundHttpPolicy {
            allow_private_hosts: true,
        };
        let url =
            validate_outbound_http_target("signal.service_url", "http://127.0.0.1:8080", policy)
                .expect("private hosts should be allowed when the policy is widened");

        assert_eq!(url.as_str(), "http://127.0.0.1:8080/");
    }

    #[test]
    fn outbound_http_client_does_not_follow_redirects() {
        let policy = ChannelOutboundHttpPolicy {
            allow_private_hosts: true,
        };

        let final_response =
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_owned();
        let (final_url, final_handle) =
            spawn_test_http_server(final_response, Duration::from_millis(250))
                .expect("spawn final test server");

        let redirect_response = format!(
            "HTTP/1.1 302 Found\r\nLocation: {final_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let (redirect_url, redirect_handle) =
            spawn_test_http_server(redirect_response, Duration::from_secs(2))
                .expect("spawn redirect test server");

        let client =
            build_outbound_http_client("redirect test", policy).expect("build outbound client");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime");
        let response = runtime
            .block_on(async { client.get(redirect_url).send().await })
            .expect("send redirect test request");

        assert_eq!(response.status(), reqwest::StatusCode::FOUND);

        let redirect_requested = redirect_handle
            .join()
            .expect("join redirect server thread")
            .expect("redirect server result");
        let final_requested = final_handle
            .join()
            .expect("join final server thread")
            .expect("final server result");

        assert!(redirect_requested);
        assert!(!final_requested);
    }
}
