use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};

#[cfg(feature = "tool-websearch")]
use regex::Regex;

#[cfg(feature = "tool-websearch")]
use reqwest::Url;

#[cfg(feature = "tool-websearch")]
use serde_json::{Value, json};

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

    let query = match payload.get("query") {
        Some(value) => value
            .as_str()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| "web.search requires payload.query to be a string".to_owned())?,
        None => return Err("web.search requires payload.query".to_owned()),
    };
    if query.chars().count() > MAX_QUERY_LENGTH {
        return Err(format!(
            "web.search payload.query exceeds maximum length ({MAX_QUERY_LENGTH} characters)"
        ));
    }

    let provider_override = match payload.get("provider") {
        Some(value) => Some(
            value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "web.search payload.provider must be a string".to_owned())?,
        ),
        None => None,
    };

    let max_results = match payload.get("max_results") {
        Some(value) => usize::try_from(
            value
                .as_u64()
                .ok_or_else(|| "web.search payload.max_results must be an integer".to_owned())?,
        )
        .map_err(|error| format!("invalid web.search payload.max_results: {error}"))?,
        None => config.web_search.max_results,
    }
    .clamp(1, 10);

    let provider = provider_override.unwrap_or(&config.web_search.default_provider);
    let normalized_provider =
        crate::config::normalize_web_search_provider(provider).unwrap_or(provider);

    let result = super::web_http::run_async(async {
        match normalized_provider {
            crate::config::WEB_SEARCH_PROVIDER_DUCKDUCKGO => {
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
                "Unknown search provider: '{}'. Supported providers: {}.",
                provider,
                crate::config::WEB_SEARCH_PROVIDER_VALID_VALUES
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

    let link_matches: Vec<_> = link_regex.captures_iter(html).collect();

    if link_matches.is_empty() {
        return Ok(json!({
            "query": query,
            "provider": "duckduckgo",
            "results": []
        }));
    }

    let mut results = Vec::new();

    for (i, caps) in link_matches.iter().take(max_results).enumerate() {
        let Some(full_match) = caps.get(0) else {
            continue;
        };
        let url = decode_ddg_url(&caps[1]);
        let title = strip_html_tags(&caps[2]);

        let search_start = full_match.end();
        let search_end = link_matches
            .get(i + 1)
            .and_then(|next_caps| next_caps.get(0))
            .map(|m| m.start())
            .unwrap_or(html.len());

        let snippet = if search_start < search_end {
            let region = &html[search_start..search_end];
            snippet_regex
                .captures(region)
                .and_then(|c| c.get(1))
                .map(|m| strip_html_tags(m.as_str()))
                .unwrap_or_default()
        } else {
            String::new()
        };

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
    let normalized = if raw.starts_with("//") {
        format!("https:{raw}")
    } else if raw.starts_with("/l/") {
        format!("https://duckduckgo.com{raw}")
    } else {
        raw.to_owned()
    };

    if let Ok(url) = Url::parse(&normalized) {
        let is_ddg_host = url
            .host_str()
            .is_some_and(|host| host == "duckduckgo.com" || host.ends_with(".duckduckgo.com"));
        let is_ddg_redirect = is_ddg_host && url.path().starts_with("/l/");
        if is_ddg_redirect
            && let Some(encoded) = url.query_pairs().find(|(k, _)| k == "uddg").map(|(_, v)| v)
        {
            return encoded.into_owned();
        }
    }
    raw.to_string()
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
    let api_key = api_key
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            format!(
                "Brave API key not configured. Set tools.web_search.brave_api_key in config or {} environment variable.",
                crate::config::WEB_SEARCH_BRAVE_API_KEY_ENV
            )
        })?;

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
    let api_key = api_key
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            format!(
                "Tavily API key not configured. Set tools.web_search.tavily_api_key in config or {} environment variable.",
                crate::config::WEB_SEARCH_TAVILY_API_KEY_ENV
            )
        })?;

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
    fn web_search_rejects_non_string_query() {
        let error =
            execute_web_search_tool_with_config(request(json!({"query": 123})), &test_config())
                .expect_err("should reject non-string query");
        assert!(
            error.contains("payload.query to be a string"),
            "unexpected error: {error}"
        );
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
    fn web_search_rejects_overlong_multibyte_query() {
        // Test that character count, not byte count, is used for length validation
        // Each emoji is 4 bytes in UTF-8, so 126 emojis = 504 bytes but 126 characters
        // We want a query that exceeds MAX_QUERY_LENGTH (500) in characters but is under in bytes
        let multibyte_query = "😀".repeat(MAX_QUERY_LENGTH + 1); // 501 chars, 2004 bytes
        let error = execute_web_search_tool_with_config(
            request(json!({"query": multibyte_query})),
            &test_config(),
        )
        .expect_err("should reject too-long multibyte query");
        assert!(error.contains("exceeds maximum length"));
    }

    #[test]
    fn web_search_accepts_query_at_max_length() {
        // Test that a query exactly at MAX_QUERY_LENGTH is accepted
        let max_query = "x".repeat(MAX_QUERY_LENGTH);
        // Use an unknown provider to short-circuit after validation without hitting the network
        let error = execute_web_search_tool_with_config(
            request(json!({"query": max_query, "provider": "unknown"})),
            &test_config(),
        )
        .expect_err("validation should pass before provider dispatch");
        assert!(error.contains("Unknown search provider"));
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
    fn parse_duckduckgo_html_handles_missing_snippet_without_misalignment() {
        // Regression test: verify that a missing snippet in the middle result
        // doesn't cause subsequent snippets to shift onto wrong titles/URLs
        let html = r#"
            <a class="result__a" href="https://first.com">First Title</a>
            <a class="result__snippet">First snippet</a>
            <a class="result__a" href="https://second.com">Second Title</a>
            <!-- Second result has no snippet -->
            <a class="result__a" href="https://third.com">Third Title</a>
            <a class="result__snippet">Third snippet</a>
        "#;
        let result = parse_duckduckgo_html(html, "test", 5).unwrap();
        let results = result["results"]
            .as_array()
            .expect("results should be array");

        assert_eq!(results.len(), 3, "should have 3 results");

        // First result: has title, URL, and snippet
        assert_eq!(results[0]["title"], "First Title");
        assert_eq!(results[0]["url"], "https://first.com");
        assert_eq!(results[0]["snippet"], "First snippet");

        // Second result: has title and URL, but no snippet
        assert_eq!(results[1]["title"], "Second Title");
        assert_eq!(results[1]["url"], "https://second.com");
        assert_eq!(
            results[1]["snippet"], "",
            "second result should have empty snippet"
        );

        // Third result: has title, URL, and snippet (not shifted from second)
        assert_eq!(results[2]["title"], "Third Title");
        assert_eq!(results[2]["url"], "https://third.com");
        assert_eq!(
            results[2]["snippet"], "Third snippet",
            "third snippet should not be shifted"
        );
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
    fn parse_brave_response_extracts_results() {
        let json = json!({
            "web": {
                "results": [{
                    "title": "Example Title",
                    "url": "https://example.com",
                    "description": "Example description"
                }]
            }
        });
        let result = parse_brave_response(&json, "test", 5).unwrap();
        assert_eq!(result["provider"], "brave");
        assert!(!result["results"].as_array().unwrap().is_empty());
    }

    #[test]
    fn parse_brave_response_handles_empty() {
        let json = json!({"web": {"results": []}});
        let result = parse_brave_response(&json, "test", 5).unwrap();
        assert!(result["results"].as_array().unwrap().is_empty());
    }

    #[test]
    fn parse_brave_response_rejects_invalid_format() {
        let json = json!({"invalid": "structure"});
        let error =
            parse_brave_response(&json, "test", 5).expect_err("should reject invalid format");
        assert!(error.contains("Invalid Brave API response format"));
    }

    #[test]
    fn web_search_disabled_returns_error() {
        let mut config = test_config();
        config.web_search.enabled = false;
        let error = execute_web_search_tool_with_config(request(json!({"query": "test"})), &config)
            .expect_err("should reject when disabled");
        assert!(
            error.contains("disabled by config"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn decode_ddg_url_extracts_redirect() {
        // Standard DDG redirect URL
        let url = "https://duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpage";
        assert_eq!(decode_ddg_url(url), "https://example.com/page");
    }

    #[test]
    fn decode_ddg_url_returns_original_for_non_ddg_url() {
        // Non-DDG URL should be returned as-is
        let url = "https://example.com/page?uddg=https%3A%2F%2Fother.com";
        assert_eq!(decode_ddg_url(url), url);

        // DDG URL but not a redirect path
        let url = "https://duckduckgo.com/?q=test&uddg=https%3A%2F%2Fother.com";
        assert_eq!(decode_ddg_url(url), url);
    }

    #[test]
    fn decode_ddg_url_handles_malformed_url() {
        // Malformed URL should be returned as-is
        let url = "not a valid url";
        assert_eq!(decode_ddg_url(url), url);
    }

    #[test]
    fn web_search_provider_override_uses_specified_provider() {
        // Test that provider parameter overrides default
        // Use unknown provider to short-circuit after provider selection
        let error = execute_web_search_tool_with_config(
            request(json!({"query": "test", "provider": "unknown"})),
            &test_config(),
        )
        .expect_err("should fail with unknown provider");
        assert!(error.contains("Unknown search provider"));
    }

    #[test]
    fn web_search_brave_requires_api_key() {
        // Test that brave provider requires API key at runtime
        let config = test_config(); // No brave_api_key set by default
        let error = execute_web_search_tool_with_config(
            request(json!({"query": "test", "provider": "brave"})),
            &config,
        )
        .expect_err("should require brave API key");
        assert!(
            error.contains("Brave API key not configured"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn web_search_tavily_requires_api_key() {
        // Test that tavily provider requires API key at runtime
        let config = test_config(); // No tavily_api_key set by default
        let error = execute_web_search_tool_with_config(
            request(json!({"query": "test", "provider": "tavily"})),
            &config,
        )
        .expect_err("should require tavily API key");
        assert!(
            error.contains("Tavily API key not configured"),
            "unexpected error: {error}"
        );
    }
}
