use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde_json::{Value, json};

#[cfg(feature = "tool-websearch")]
use regex::Regex;

#[cfg(feature = "tool-websearch")]
const MAX_QUERY_LENGTH: usize = 500;

pub(super) fn execute_web_search_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    #[cfg(not(feature = "tool-websearch"))]
    {
        let _ = (request, config);
        return Err(
            "web.search tool is disabled in this build (enable feature `tool-websearch`)"
                .to_owned(),
        );
    }

    #[cfg(feature = "tool-websearch")]
    {
        execute_web_search_tool_enabled(request, config)
    }
}

#[cfg(feature = "tool-websearch")]
fn execute_web_search_tool_enabled(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    if !config.web_search.enabled {
        return Err("web.search is disabled by config.tools.web_search.enabled=false".to_owned());
    }

    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "web.search payload must be an object".to_owned())?;

    let query = payload
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "web.search requires payload.query".to_owned())?;
    if query.len() > MAX_QUERY_LENGTH {
        return Err(format!(
            "web.search payload.query exceeds maximum length ({MAX_QUERY_LENGTH} characters)"
        ));
    }

    let provider_override = payload.get("provider").and_then(Value::as_str);

    let max_results = payload
        .get("max_results")
        .and_then(Value::as_u64)
        .map(|v| v as usize)
        .unwrap_or(config.web_search.max_results)
        .clamp(1, 10);

    let provider = provider_override.unwrap_or(&config.web_search.default_provider);

    let result = super::web_http::run_async(async {
        match provider {
            "duckduckgo" | "ddg" => {
                search_duckduckgo(query, max_results, config.web_search.timeout_seconds).await
            }
            "brave" => {
                search_brave(
                    query,
                    max_results,
                    config.web_search.timeout_seconds,
                    config.web_search.brave_api_key.as_deref(),
                )
                .await
            }
            "tavily" => {
                search_tavily(
                    query,
                    max_results,
                    config.web_search.timeout_seconds,
                    config.web_search.tavily_api_key.as_deref(),
                )
                .await
            }
            _ => Err(format!(
                "Unknown search provider: '{}'. Use 'duckduckgo', 'brave', or 'tavily'.",
                provider
            )),
        }
    })??;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: result,
    })
}

#[cfg(feature = "tool-websearch")]
async fn search_duckduckgo(
    query: &str,
    max_results: usize,
    timeout_seconds: u64,
) -> Result<Value, String> {
    let url = reqwest::Url::parse_with_params("https://html.duckduckgo.com/html/", &[("q", query)])
        .map_err(|e| format!("Failed to build DuckDuckGo URL: {e}"))?;

    let client = super::web_http::build_ssrf_safe_client(
        false, // deny private hosts by default
        timeout_seconds,
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
    )?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("DuckDuckGo request failed: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("DuckDuckGo returned status {}", response.status()));
    }

    let html = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response: {e}"))?;

    parse_duckduckgo_html(&html, query, max_results)
}

#[cfg(feature = "tool-websearch")]
fn parse_duckduckgo_html(html: &str, query: &str, max_results: usize) -> Result<Value, String> {
    let link_regex = ddg_link_regex();
    let snippet_regex = ddg_snippet_regex();

    let links: Vec<_> = link_regex
        .captures_iter(html)
        .take(max_results + 2)
        .collect();
    let snippets: Vec<_> = snippet_regex
        .captures_iter(html)
        .take(max_results + 2)
        .collect();

    if links.is_empty() {
        return Ok(json!({
            "query": query,
            "provider": "duckduckgo",
            "results": []
        }));
    }

    let mut results = Vec::new();
    for (i, caps) in links.iter().take(max_results).enumerate() {
        let url = decode_ddg_url(&caps[1]);
        let title = strip_html_tags(&caps[2]);
        let snippet = snippets
            .get(i)
            .and_then(|caps| caps.get(1))
            .map(|m| strip_html_tags(m.as_str()))
            .unwrap_or_default();

        results.push(json!({
            "title": title.trim(),
            "url": url.trim(),
            "snippet": snippet.trim()
        }));
    }

    Ok(json!({
        "query": query,
        "provider": "duckduckgo",
        "results": results
    }))
}

#[cfg(feature = "tool-websearch")]
fn decode_ddg_url(raw: &str) -> String {
    if let Some(idx) = raw.find("uddg=") {
        let encoded = &raw[idx + 5..];
        let encoded = encoded.split('&').next().unwrap_or(encoded);
        if let Ok(decoded) = urlencoding_decode(encoded) {
            return decoded;
        }
    }
    raw.to_string()
}

#[cfg(feature = "tool-websearch")]
fn urlencoding_decode(s: &str) -> Result<String, String> {
    let mut bytes = Vec::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '+' {
            bytes.push(b' ');
        } else if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if hex.len() != 2 {
                return Err("Invalid percent encoding: truncated escape sequence".to_owned());
            }
            let byte = u8::from_str_radix(&hex, 16)
                .map_err(|e| format!("Invalid percent encoding: {e}"))?;
            bytes.push(byte);
        } else {
            let mut buf = [0u8; 4];
            bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
    }
    String::from_utf8(bytes).map_err(|e| format!("Invalid UTF-8 in decoded value: {e}"))
}

#[cfg(feature = "tool-websearch")]
#[allow(clippy::expect_used)]
fn ddg_link_regex() -> &'static Regex {
    static LINK_REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    LINK_REGEX.get_or_init(|| {
        Regex::new(r#"<a[^>]*class="[^"]*result__a[^"]*"[^>]*href="([^"]+)"[^>]*>([\s\S]*?)</a>"#)
            .expect("static regex should always compile")
    })
}

#[cfg(feature = "tool-websearch")]
#[allow(clippy::expect_used)]
fn ddg_snippet_regex() -> &'static Regex {
    static SNIPPET_REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    SNIPPET_REGEX.get_or_init(|| {
        Regex::new(r#"<a class="result__snippet[^"]*"[^>]*>([\s\S]*?)</a>"#)
            .expect("static regex should always compile")
    })
}

#[cfg(feature = "tool-websearch")]
#[allow(clippy::expect_used)]
fn strip_html_tags(s: &str) -> String {
    static TAG_REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let tag_regex = TAG_REGEX
        .get_or_init(|| Regex::new(r"<[^>]+>").expect("static regex should always compile"));
    tag_regex.replace_all(s, "").to_string()
}

#[cfg(feature = "tool-websearch")]
async fn search_brave(
    query: &str,
    max_results: usize,
    timeout_seconds: u64,
    api_key: Option<&str>,
) -> Result<Value, String> {
    let api_key = api_key.ok_or(
        "Brave API key not configured. Set tools.web_search.brave_api_key in config or BRAVE_API_KEY environment variable.",
    )?;

    let url = reqwest::Url::parse_with_params(
        "https://api.search.brave.com/res/v1/web/search",
        &[("q", query), ("count", &max_results.to_string())],
    )
    .map_err(|e| format!("Failed to build Brave URL: {e}"))?;

    let client = super::web_http::build_ssrf_safe_client(
        false, // deny private hosts by default
        timeout_seconds,
        "LoongClaw-WebSearch/0.1",
    )?;

    let response = client
        .get(url)
        .header("Accept", "application/json")
        .header("X-Subscription-Token", api_key)
        .send()
        .await
        .map_err(|error| format_request_error("Brave request failed", &error))?;

    if !response.status().is_success() {
        return Err(format!("Brave returned status {}", response.status()));
    }

    let json: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Brave response: {e}"))?;

    parse_brave_response(&json, query, max_results)
}

#[cfg(feature = "tool-websearch")]
fn parse_brave_response(json: &Value, query: &str, max_results: usize) -> Result<Value, String> {
    let results = json
        .get("web")
        .and_then(|w| w.get("results"))
        .and_then(|r| r.as_array())
        .ok_or("Invalid Brave API response format")?;

    let results: Vec<Value> = results
        .iter()
        .take(max_results)
        .map(|r| {
            json!({
                "title": r.get("title").and_then(|t| t.as_str()).unwrap_or(""),
                "url": r.get("url").and_then(|u| u.as_str()).unwrap_or(""),
                "snippet": r.get("description").and_then(|d| d.as_str()).unwrap_or("")
            })
        })
        .collect();

    Ok(json!({
        "query": query,
        "provider": "brave",
        "results": results
    }))
}

#[cfg(feature = "tool-websearch")]
async fn search_tavily(
    query: &str,
    max_results: usize,
    timeout_seconds: u64,
    api_key: Option<&str>,
) -> Result<Value, String> {
    let api_key = api_key.ok_or(
        "Tavily API key not configured. Set tools.web_search.tavily_api_key in config or TAVILY_API_KEY environment variable.",
    )?;

    let client = super::web_http::build_ssrf_safe_client(
        false, // deny private hosts by default
        timeout_seconds,
        "LoongClaw-WebSearch/0.1",
    )?;

    let response = client
        .post("https://api.tavily.com/search")
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&json!({
            "query": query,
            "max_results": max_results,
            "include_answer": false,
            "include_raw_content": false,
        }))
        .send()
        .await
        .map_err(|error| format_request_error("Tavily request failed", &error))?;

    if !response.status().is_success() {
        return Err(format!("Tavily returned status {}", response.status()));
    }

    let json: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Tavily response: {e}"))?;

    parse_tavily_response(&json, query, max_results)
}

#[cfg(feature = "tool-websearch")]
fn parse_tavily_response(json: &Value, query: &str, max_results: usize) -> Result<Value, String> {
    let results = json
        .get("results")
        .and_then(|r| r.as_array())
        .ok_or("Invalid Tavily API response format")?;

    let results: Vec<Value> = results
        .iter()
        .take(max_results)
        .map(|r| {
            json!({
                "title": r.get("title").and_then(|t| t.as_str()).unwrap_or(""),
                "url": r.get("url").and_then(|u| u.as_str()).unwrap_or(""),
                "snippet": r.get("content").and_then(|c| c.as_str()).unwrap_or("")
            })
        })
        .collect();

    Ok(json!({
        "query": query,
        "provider": "tavily",
        "results": results
    }))
}

#[cfg(feature = "tool-websearch")]
fn format_request_error(prefix: &str, error: &reqwest::Error) -> String {
    let kind = if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connect"
    } else if error.is_request() {
        "request"
    } else if error.is_redirect() {
        "redirect"
    } else if error.is_decode() {
        "decode"
    } else if error.is_body() {
        "body"
    } else {
        "unknown"
    };
    format!("{prefix} ({kind} error)")
}

#[cfg(all(test, feature = "tool-websearch"))]
#[allow(clippy::panic)]
mod tests {
    use super::super::runtime_config::ToolRuntimeConfig;
    use super::*;

    fn request(payload: Value) -> ToolCoreRequest {
        ToolCoreRequest {
            tool_name: "web.search".to_owned(),
            payload,
        }
    }

    fn test_config() -> ToolRuntimeConfig {
        ToolRuntimeConfig::default()
    }

    #[test]
    fn web_search_requires_object_payload() {
        let error = execute_web_search_tool_with_config(request(json!("query")), &test_config())
            .expect_err("should reject non-object payload");
        assert!(error.contains("payload must be an object"));
    }

    #[test]
    fn web_search_requires_query() {
        let error = execute_web_search_tool_with_config(request(json!({})), &test_config())
            .expect_err("should reject missing query");
        assert!(error.contains("requires payload.query"));
    }

    #[test]
    fn web_search_rejects_empty_query() {
        let error =
            execute_web_search_tool_with_config(request(json!({"query": ""})), &test_config())
                .expect_err("should reject empty query");
        assert!(error.contains("requires payload.query"));
    }

    #[test]
    fn web_search_rejects_unknown_provider() {
        let error = execute_web_search_tool_with_config(
            request(json!({"query": "test", "provider": "unknown"})),
            &test_config(),
        )
        .expect_err("should reject unknown provider");
        assert!(error.contains("Unknown search provider"));
    }

    #[test]
    fn web_search_rejects_overlong_query() {
        let long_query = "x".repeat(MAX_QUERY_LENGTH + 1);
        let error = execute_web_search_tool_with_config(
            request(json!({"query": long_query})),
            &test_config(),
        )
        .expect_err("should reject too-long query");
        assert!(error.contains("exceeds maximum length"));
    }

    #[test]
    fn parse_duckduckgo_html_extracts_results() {
        let html = r#"
            <a class="result__a" href="https://example.com">Example Title</a>
            <a class="result__snippet">Example snippet</a>
        "#;
        let result = parse_duckduckgo_html(html, "test", 5).unwrap();
        assert_eq!(result["provider"], "duckduckgo");
        assert!(!result["results"].as_array().unwrap().is_empty());
    }

    #[test]
    fn parse_duckduckgo_html_handles_empty() {
        let html = "<html><body>No results</body></html>";
        let result = parse_duckduckgo_html(html, "test", 5).unwrap();
        assert!(result["results"].as_array().unwrap().is_empty());
    }

    #[test]
    fn strip_html_tags_removes_tags() {
        let input = "<b>Hello</b> <i>World</i>";
        assert_eq!(strip_html_tags(input), "Hello World");
    }

    #[test]
    fn parse_tavily_response_extracts_results() {
        let json = json!({
            "results": [
                {
                    "title": "Example Title",
                    "url": "https://example.com",
                    "content": "Example content"
                }
            ]
        });
        let result = parse_tavily_response(&json, "test", 5).unwrap();
        assert_eq!(result["provider"], "tavily");
        assert!(!result["results"].as_array().unwrap().is_empty());
    }

    #[test]
    fn parse_tavily_response_handles_empty() {
        let json = json!({"results": []});
        let result = parse_tavily_response(&json, "test", 5).unwrap();
        assert!(result["results"].as_array().unwrap().is_empty());
    }

    #[test]
    fn urlencoding_decode_handles_utf8_sequences() {
        let decoded = urlencoding_decode("caf%C3%A9").expect("valid encoded utf8 should decode");
        assert_eq!(decoded, "café");
    }

    #[test]
    fn urlencoding_decode_rejects_truncated_escape() {
        let error = urlencoding_decode("abc%").expect_err("truncated escape should fail");
        assert!(error.contains("Invalid percent encoding"));
    }
}
