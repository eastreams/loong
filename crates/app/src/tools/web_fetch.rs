use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
#[cfg(feature = "tool-webfetch")]
use serde_json::{Map, Value, json};

#[cfg_attr(not(feature = "tool-webfetch"), allow(dead_code))]
pub(super) fn execute_web_fetch_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    #[cfg(not(feature = "tool-webfetch"))]
    {
        let _ = (request, config);
        return Err(
            "web.fetch tool is disabled in this build (enable feature `tool-webfetch`)".to_owned(),
        );
    }

    #[cfg(feature = "tool-webfetch")]
    {
        execute_web_fetch_tool_enabled(request, config)
    }
}

#[cfg(feature = "tool-webfetch")]
fn execute_web_fetch_tool_enabled(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    use std::sync::Arc;
    use std::time::Duration;

    if !config.web_fetch.enabled {
        return Err("web.fetch is disabled by config.tools.web.enabled=false".to_owned());
    }

    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "web.fetch payload must be an object".to_owned())?;
    let raw_url = payload
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "web.fetch requires payload.url".to_owned())?;
    let mode = parse_render_mode(payload)?;
    let max_bytes = parse_max_bytes(payload, config.web_fetch.max_bytes)?;

    let resolver = super::web_http::SsrfSafeResolver {
        allow_private_hosts: config.web_fetch.allow_private_hosts,
    };

    let client = reqwest::Client::builder()
        .dns_resolver(Arc::new(resolver))
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(config.web_fetch.timeout_seconds))
        .user_agent("LoongClaw-WebFetch/0.1")
        .build()
        .map_err(|error| format!("failed to build HTTP client for web.fetch: {error}"))?;

    let mut current_url = reqwest::Url::parse(raw_url)
        .map_err(|error| format!("invalid web.fetch url `{raw_url}`: {error}"))?;
    let mut current_host = validate_web_target(&current_url, &config.web_fetch, "web.fetch")?;
    let mut redirect_count = 0usize;

    super::web_http::run_async(async {
        loop {
            let mut budget = super::download_guard::ByteBudget::new(max_bytes);
            let response = client
                .get(current_url.clone())
                .send()
                .await
                .map_err(|error| format!("web.fetch request failed: {error}"))?;

            if response.status().is_redirection() {
                if redirect_count >= config.web_fetch.max_redirects {
                    return Err(format!(
                        "web.fetch exceeded redirect limit ({})",
                        config.web_fetch.max_redirects
                    ));
                }

                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .ok_or_else(|| {
                        format!(
                            "web.fetch received redirect status {} without Location header",
                            response.status()
                        )
                    })?
                    .to_str()
                    .map_err(|error| {
                        format!("web.fetch redirect Location header was invalid: {error}")
                    })?;
                let next_url = current_url.join(location).map_err(|error| {
                    format!("web.fetch failed to resolve redirect target: {error}")
                })?;
                current_host = validate_web_target(&next_url, &config.web_fetch, "web.fetch")?;
                current_url = next_url;
                redirect_count += 1;
                continue;
            }

            if !response.status().is_success() {
                return Err(format!(
                    "web.fetch returned non-success status {} for `{}`",
                    response.status(),
                    current_url
                ));
            }

            let status_code = response.status().as_u16();
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(|value| value.to_owned());
            budget.reject_if_content_length_exceeds(
                response.content_length(),
                "web.fetch response",
            )?;

            let mut body = Vec::new();
            let mut stream = response.bytes_stream();
            use futures_util::StreamExt;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk
                    .map_err(|error| format!("failed to read web.fetch response body: {error}"))?;
                budget.try_consume(chunk.len(), "web.fetch response")?;
                body.extend_from_slice(&chunk);
            }

            let raw_text = String::from_utf8_lossy(&body).into_owned();
            let is_html = looks_like_html(content_type.as_deref(), raw_text.as_str());
            if mode == RenderMode::ReadableText
                && !is_html
                && response_is_probably_binary(content_type.as_deref(), &body)
            {
                return Err(
                    "web.fetch readable_text mode only supports text-like responses; binary bodies are not returned"
                        .to_owned(),
                );
            }
            let title = is_html
                .then(|| extract_html_title(raw_text.as_str()))
                .flatten();
            let content = match mode {
                RenderMode::ReadableText if is_html => extract_readable_text_from_html(&raw_text),
                RenderMode::ReadableText | RenderMode::RawText => raw_text.trim().to_owned(),
            };

            return Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({
                    "adapter": "core-tools",
                    "tool_name": request.tool_name,
                    "requested_url": raw_url,
                    "final_url": current_url.as_str(),
                    "host": current_host,
                    "status_code": status_code,
                    "content_type": content_type,
                    "mode": mode.as_str(),
                    "content": content,
                    "title": title,
                    "bytes_downloaded": budget.consumed(),
                    "redirect_count": redirect_count,
                }),
            });
        }
    })?
}

#[cfg(feature = "tool-webfetch")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenderMode {
    ReadableText,
    RawText,
}

#[cfg(feature = "tool-webfetch")]
impl RenderMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::ReadableText => "readable_text",
            Self::RawText => "raw_text",
        }
    }
}

#[cfg(feature = "tool-webfetch")]
fn parse_render_mode(payload: &Map<String, Value>) -> Result<RenderMode, String> {
    let Some(value) = payload.get("mode") else {
        return Ok(RenderMode::ReadableText);
    };

    let raw = value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "web.fetch payload.mode must be a string".to_owned())?;

    match raw {
        "readable_text" | "text" => Ok(RenderMode::ReadableText),
        "raw_text" | "raw" => Ok(RenderMode::RawText),
        _ => Err(
            "web.fetch payload.mode must be one of `readable_text`, `raw_text`, `text`, or `raw`"
                .to_owned(),
        ),
    }
}

#[cfg(feature = "tool-webfetch")]
fn parse_max_bytes(payload: &Map<String, Value>, configured_max: usize) -> Result<usize, String> {
    let Some(value) = payload.get("max_bytes") else {
        return Ok(configured_max);
    };

    let parsed = value
        .as_u64()
        .ok_or_else(|| "web.fetch payload.max_bytes must be an integer".to_owned())?;
    if parsed == 0 {
        return Err("web.fetch payload.max_bytes must be >= 1".to_owned());
    }
    let parsed = usize::try_from(parsed)
        .map_err(|error| format!("invalid web.fetch max_bytes `{parsed}`: {error}"))?;
    if parsed > configured_max {
        return Err(format!(
            "web.fetch payload.max_bytes exceeds configured limit ({configured_max} bytes)"
        ));
    }
    Ok(parsed)
}

#[cfg(any(
    feature = "tool-webfetch",
    feature = "tool-browser",
    feature = "tool-websearch"
))]
pub(crate) fn validate_web_target(
    url: &reqwest::Url,
    policy: &super::runtime_config::WebFetchRuntimePolicy,
    surface_name: &str,
) -> Result<String, String> {
    let options = super::web_http::HttpTargetValidationOptions {
        allow_private_hosts: policy.allow_private_hosts,
        reject_userinfo: false,
        resolve_dns: true,
        enforce_allowed_domains: policy.enforce_allowed_domains,
        allowed_domains: policy
            .enforce_allowed_domains
            .then_some(&policy.allowed_domains),
        blocked_domains: Some(&policy.blocked_domains),
    };

    super::web_http::validate_http_target(url, &options, surface_name)
}

#[cfg(any(
    feature = "tool-webfetch",
    feature = "tool-browser",
    feature = "tool-websearch"
))]
pub(crate) fn looks_like_html(content_type: Option<&str>, body: &str) -> bool {
    if let Some(content_type) = content_type {
        let lowered = content_type.to_ascii_lowercase();
        if lowered.contains("text/html") || lowered.contains("application/xhtml+xml") {
            return true;
        }
    }

    let lowered = body.to_ascii_lowercase();
    lowered.contains("<html") || lowered.contains("<body") || lowered.contains("<!doctype html")
}

#[cfg(any(
    feature = "tool-webfetch",
    feature = "tool-browser",
    feature = "tool-websearch"
))]
pub(crate) fn response_is_probably_binary(content_type: Option<&str>, body: &[u8]) -> bool {
    if body.is_empty() {
        return false;
    }

    if let Some(content_type) = content_type {
        let lowered = content_type.to_ascii_lowercase();
        if lowered.starts_with("text/")
            || lowered.contains("json")
            || lowered.contains("xml")
            || lowered.contains("javascript")
            || lowered.contains("x-www-form-urlencoded")
            || lowered.contains("yaml")
            || lowered.contains("csv")
        {
            return false;
        }

        if lowered.contains("octet-stream")
            || lowered.contains("pdf")
            || lowered.contains("zip")
            || lowered.starts_with("image/")
            || lowered.starts_with("audio/")
            || lowered.starts_with("video/")
        {
            return true;
        }
    }

    let sample = body.get(..body.len().min(512)).unwrap_or(body);
    if sample.contains(&0) {
        return true;
    }

    let control_count = sample
        .iter()
        .filter(|&&byte| {
            ((byte < 0x20) && !matches!(byte, b'\n' | b'\r' | b'\t' | 0x0c)) || byte == 0x7f
        })
        .count();
    control_count.saturating_mul(8) > sample.len()
}

#[cfg(any(
    feature = "tool-webfetch",
    feature = "tool-browser",
    feature = "tool-websearch"
))]
pub(crate) fn extract_html_title(html: &str) -> Option<String> {
    extract_tag_inner_text(html, "title").and_then(|value| {
        let collapsed = collapse_whitespace(&decode_basic_entities(value.trim()));
        (!collapsed.is_empty()).then_some(collapsed)
    })
}

#[cfg(any(
    feature = "tool-webfetch",
    feature = "tool-browser",
    feature = "tool-websearch"
))]
pub(crate) fn extract_readable_text_from_html(html: &str) -> String {
    let mut sanitized = strip_tag_block(html, "script");
    sanitized = strip_tag_block(&sanitized, "style");
    sanitized = strip_tag_block(&sanitized, "noscript");
    sanitized = strip_tag_block(&sanitized, "head");
    let text = strip_tags(&sanitized);
    collapse_whitespace(&decode_basic_entities(&text))
}

#[cfg(any(
    feature = "tool-webfetch",
    feature = "tool-browser",
    feature = "tool-websearch"
))]
fn strip_tag_block(input: &str, tag: &str) -> String {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut output = input.to_owned();

    loop {
        let lowered = output.to_ascii_lowercase();
        let Some(start) = lowered.find(&open) else {
            break;
        };
        let Some(close_start_rel) = lowered[start..].find(&close) else {
            break;
        };
        let close_start = start + close_start_rel;
        let Some(close_end_rel) = lowered[close_start..].find('>') else {
            break;
        };
        let end = close_start + close_end_rel + 1;
        output.replace_range(start..end, " ");
    }

    output
}

#[cfg(any(
    feature = "tool-webfetch",
    feature = "tool-browser",
    feature = "tool-websearch"
))]
fn extract_tag_inner_text<'a>(input: &'a str, tag: &str) -> Option<&'a str> {
    let lowered = input.to_ascii_lowercase();
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let start = lowered.find(&open)?;
    let open_end = start + lowered[start..].find('>')? + 1;
    let end = lowered[open_end..].find(&close)? + open_end;
    Some(&input[open_end..end])
}

#[cfg(any(
    feature = "tool-webfetch",
    feature = "tool-browser",
    feature = "tool-websearch"
))]
fn strip_tags(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut in_tag = false;

    for ch in input.chars() {
        match ch {
            '<' => {
                in_tag = true;
                if !output.ends_with(' ') {
                    output.push(' ');
                }
            }
            '>' => {
                in_tag = false;
                if !output.ends_with(' ') {
                    output.push(' ');
                }
            }
            _ if !in_tag => output.push(ch),
            _ => {}
        }
    }

    output
}

#[cfg(any(
    feature = "tool-webfetch",
    feature = "tool-browser",
    feature = "tool-websearch"
))]
fn decode_basic_entities(input: &str) -> String {
    input
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

#[cfg(any(
    feature = "tool-webfetch",
    feature = "tool-browser",
    feature = "tool-websearch"
))]
fn collapse_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(all(test, feature = "tool-webfetch"))]
#[allow(clippy::panic)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn request(payload: Value) -> ToolCoreRequest {
        ToolCoreRequest {
            tool_name: "web.fetch".to_owned(),
            payload,
        }
    }

    fn local_runtime_config() -> super::super::runtime_config::ToolRuntimeConfig {
        let mut config = super::super::runtime_config::ToolRuntimeConfig::default();
        config.web_fetch.allow_private_hosts = true;
        config
    }

    fn spawn_http_server(responder: impl Fn(String) -> String + Send + Sync + 'static) -> String {
        let listener = TcpListener::bind("127.0.0.1:0")
            .unwrap_or_else(|error| panic!("bind test server: {error}"));
        let address = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("local addr: {error}"));
        thread::spawn(move || {
            let (mut stream, _) = listener
                .accept()
                .unwrap_or_else(|error| panic!("accept request: {error}"));
            let mut buffer = [0_u8; 4096];
            let read = stream
                .read(&mut buffer)
                .unwrap_or_else(|error| panic!("read request: {error}"));
            let request =
                String::from_utf8_lossy(buffer.get(..read).unwrap_or(&buffer)).into_owned();
            let response = responder(request);
            stream
                .write_all(response.as_bytes())
                .unwrap_or_else(|error| panic!("write response: {error}"));
            stream.flush().ok();
        });

        format!("http://{}", address)
    }

    fn ok_response(content_type: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn redirect_response(location: &str) -> String {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
    }

    #[test]
    fn web_fetch_requires_enabled_runtime() {
        let mut config = super::super::runtime_config::ToolRuntimeConfig::default();
        config.web_fetch.enabled = false;

        let error = execute_web_fetch_tool_with_config(
            request(json!({"url": "https://example.com"})),
            &config,
        )
        .expect_err("disabled runtime should block web.fetch");

        assert!(error.contains("config.tools.web.enabled=false"));
    }

    #[test]
    fn web_fetch_requires_object_payload() {
        let error = execute_web_fetch_tool_with_config(
            request(json!("https://example.com")),
            &super::super::runtime_config::ToolRuntimeConfig::default(),
        )
        .expect_err("non-object payload should be rejected");

        assert!(error.contains("payload must be an object"));
    }

    #[test]
    fn web_fetch_requires_url() {
        let error = execute_web_fetch_tool_with_config(
            request(json!({})),
            &super::super::runtime_config::ToolRuntimeConfig::default(),
        )
        .expect_err("missing url should be rejected");

        assert!(error.contains("requires payload.url"));
    }

    #[test]
    fn web_fetch_rejects_non_http_scheme() {
        let error = execute_web_fetch_tool_with_config(
            request(json!({"url": "file:///etc/passwd"})),
            &super::super::runtime_config::ToolRuntimeConfig::default(),
        )
        .expect_err("non-http scheme should be rejected");

        assert!(error.contains("requires http or https"));
    }

    #[test]
    fn web_fetch_rejects_private_hosts_by_default() {
        let error = execute_web_fetch_tool_with_config(
            request(json!({"url": "http://127.0.0.1:8080"})),
            &super::super::runtime_config::ToolRuntimeConfig::default(),
        )
        .expect_err("private host should be blocked");

        assert!(error.contains("private or special-use"));
    }

    #[test]
    fn web_fetch_enforces_allow_and_block_domain_rules() {
        let mut allowlist_config = super::super::runtime_config::ToolRuntimeConfig::default();
        allowlist_config.web_fetch.enforce_allowed_domains = true;
        allowlist_config
            .web_fetch
            .allowed_domains
            .insert("docs.example.com".to_owned());

        let allowed_error = execute_web_fetch_tool_with_config(
            request(json!({"url": "https://api.example.com/reference"})),
            &allowlist_config,
        )
        .expect_err("host outside allowlist should be rejected");

        assert!(allowed_error.contains("not in allowed_domains"));

        let mut blocklist_config = super::super::runtime_config::ToolRuntimeConfig::default();
        blocklist_config
            .web_fetch
            .blocked_domains
            .insert("*.example.com".to_owned());

        let blocked_error = execute_web_fetch_tool_with_config(
            request(json!({"url": "https://docs.example.com/reference"})),
            &blocklist_config,
        )
        .expect_err("blocked host should be rejected");

        assert!(blocked_error.contains("matches blocked domain rule"));
    }

    #[test]
    fn web_fetch_allows_local_html_fixture_and_extracts_readable_text() {
        let url = spawn_http_server(|_request| {
            ok_response(
                "text/html; charset=utf-8",
                "<html><head><title>Demo Page</title><style>.hidden{display:none}</style><script>window.alert('x')</script></head><body><h1>Hello world</h1><p>LoongClaw fetches docs.</p></body></html>",
            )
        });

        let outcome = super::super::execute_tool_core_with_config(
            request(json!({"url": url})),
            &local_runtime_config(),
        )
        .expect("local HTML fixture should fetch");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["tool_name"], "web.fetch");
        assert_eq!(outcome.payload["mode"], "readable_text");
        assert_eq!(outcome.payload["title"], "Demo Page");
        let content = outcome.payload["content"]
            .as_str()
            .expect("content should be string");
        assert!(content.contains("Hello world"));
        assert!(content.contains("LoongClaw fetches docs."));
        assert!(!content.contains("window.alert"));
    }

    #[test]
    fn web_fetch_enforces_max_bytes_limit() {
        let body = "x".repeat(128);
        let url = spawn_http_server(move |_request| ok_response("text/plain", &body));
        let mut config = local_runtime_config();
        config.web_fetch.max_bytes = 32;

        let error = execute_web_fetch_tool_with_config(request(json!({"url": url})), &config)
            .expect_err("oversized body should be rejected");

        assert!(
            error.contains("max_bytes limit"),
            "expected max_bytes error, got: {error}"
        );
    }

    #[test]
    fn web_fetch_rejects_binary_body_in_readable_mode() {
        let url =
            spawn_http_server(|_request| ok_response("application/octet-stream", "\0PNG\x01\x02"));

        let error = execute_web_fetch_tool_with_config(
            request(json!({"url": url})),
            &local_runtime_config(),
        )
        .expect_err("binary body should be rejected in readable mode");

        assert!(error.contains("readable_text mode only supports text-like responses"));
    }

    #[test]
    fn web_fetch_follows_redirects_with_revalidation() {
        let target_url = spawn_http_server(|_request| ok_response("text/plain", "final body"));
        let redirect_target = target_url.clone();
        let redirect_url = spawn_http_server(move |_request| redirect_response(&redirect_target));

        let outcome = execute_web_fetch_tool_with_config(
            request(json!({"url": redirect_url})),
            &local_runtime_config(),
        )
        .expect("redirected local fetch should succeed");

        assert_eq!(outcome.payload["redirect_count"], 1);
        assert_eq!(
            outcome.payload["final_url"],
            reqwest::Url::parse(&target_url)
                .expect("target url")
                .to_string()
        );
        assert_eq!(outcome.payload["content"], "final body");
    }

    #[test]
    fn web_fetch_rejects_redirect_without_location_header() {
        let url = spawn_http_server(|_request| {
            "HTTP/1.1 302 Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()
        });

        let error = execute_web_fetch_tool_with_config(
            request(json!({"url": url})),
            &local_runtime_config(),
        )
        .expect_err("redirect without location should fail");

        assert!(error.contains("without Location header"));
    }

    #[test]
    fn web_fetch_raw_text_mode_preserves_non_html_body() {
        let url =
            spawn_http_server(|_request| ok_response("application/json", "{\n  \"ok\": true\n}"));

        let outcome = execute_web_fetch_tool_with_config(
            request(json!({"url": url, "mode": "raw_text"})),
            &local_runtime_config(),
        )
        .expect("raw_text mode should preserve body");

        assert_eq!(outcome.payload["mode"], "raw_text");
        assert_eq!(outcome.payload["content"], "{\n  \"ok\": true\n}");
    }

    #[test]
    fn web_fetch_rejects_invalid_mode() {
        let error = execute_web_fetch_tool_with_config(
            request(json!({"url": "https://example.com", "mode": "markdown"})),
            &super::super::runtime_config::ToolRuntimeConfig::default(),
        )
        .expect_err("invalid mode should fail");

        assert!(error.contains("payload.mode must be one of"));
    }

    #[test]
    fn web_fetch_policy_can_allow_localhost_targets() {
        let policy = super::super::runtime_config::WebFetchRuntimePolicy {
            allow_private_hosts: true,
            ..super::super::runtime_config::WebFetchRuntimePolicy::default()
        };
        let url = reqwest::Url::parse("http://localhost:8080").expect("url");

        let host =
            validate_web_target(&url, &policy, "web.fetch").expect("localhost should be allowed");

        assert_eq!(host, "localhost");
    }

    #[test]
    fn validate_web_target_denies_when_allowlist_is_enforced_but_empty() {
        let policy = super::super::runtime_config::WebFetchRuntimePolicy {
            enforce_allowed_domains: true,
            ..super::super::runtime_config::WebFetchRuntimePolicy::default()
        };
        let url = reqwest::Url::parse("https://example.com").expect("url");

        let error =
            validate_web_target(&url, &policy, "web.fetch").expect_err("empty enforced allowlist");

        assert!(error.contains("not in allowed_domains"), "error={error}");
    }

    #[test]
    fn ssrf_safe_resolver_blocks_private_ips() {
        let resolver = crate::tools::web_http::SsrfSafeResolver {
            allow_private_hosts: false,
        };
        let result: Result<_, String> = crate::tools::web_http::run_async(async {
            use reqwest::dns::Resolve;
            let name = "localhost".parse().expect("valid name");
            resolver.resolve(name).await
        });
        let result = result.expect("runtime should build");
        assert!(
            result.is_err(),
            "resolver should block localhost when allow_private_hosts is false"
        );
    }

    #[test]
    fn ssrf_safe_resolver_allows_private_ips_when_configured() {
        let resolver = crate::tools::web_http::SsrfSafeResolver {
            allow_private_hosts: true,
        };
        let result: Result<_, String> = crate::tools::web_http::run_async(async {
            use reqwest::dns::Resolve;
            let name = "localhost".parse().expect("valid name");
            resolver.resolve(name).await
        });
        let result = result.expect("runtime should build");
        assert!(
            result.is_ok(),
            "resolver should allow localhost when allow_private_hosts is true"
        );
    }

    #[test]
    fn ipv6_compatible_addresses_are_checked() {
        // ::7f00:1 is the IPv4-compatible form of 127.0.0.1
        let ip: std::net::Ipv6Addr = "::7f00:1".parse().expect("valid ipv6");
        assert!(
            crate::tools::web_http::is_private_or_special_ipv6(ip),
            "IPv4-compatible loopback address should be detected as private"
        );

        // ::a00:1 is the IPv4-compatible form of 10.0.0.1
        let ip: std::net::Ipv6Addr = "::a00:1".parse().expect("valid ipv6");
        assert!(
            crate::tools::web_http::is_private_or_special_ipv6(ip),
            "IPv4-compatible private address should be detected as private"
        );
    }

    #[test]
    fn ipv4_over_block_192_tightened() {
        // 192.0.0.1 (IETF Protocol Assignments) should be blocked
        let ip: std::net::Ipv4Addr = "192.0.0.1".parse().expect("valid ipv4");
        assert!(crate::tools::web_http::is_private_or_special_ipv4(ip));

        // 192.0.1.1 is normal routable space and should NOT be blocked
        let ip: std::net::Ipv4Addr = "192.0.1.1".parse().expect("valid ipv4");
        assert!(
            !crate::tools::web_http::is_private_or_special_ipv4(ip),
            "192.0.1.x should not be blocked (normal routable space)"
        );
    }
}
