use loong_contracts::ToolCoreRequest;
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

const MASCOT_BROWSER_SCOPE_ID: &str = "mascot:qoong:browser";
const MASCOT_THEME_TOGGLE_SESSION_PREFIX: &str = "qoong-theme-toggle";
const MASCOT_THEME_TOGGLE_SELECTOR: &str = r#"[data-mascot-action="toggle-theme"]"#;
const MASCOT_SEARCH_SESSION_PREFIX: &str = "qoong-search-demo";
const MASCOT_DEFAULT_SEARCH_QUERY: &str = "food";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MascotBrowserThemeToggleRequest {
    page_url: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MascotBrowserThemeTogglePayload {
    session_id: String,
    execution_tier: String,
    page_url: Option<String>,
    title: Option<String>,
    clicked: bool,
    snapshot: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MascotBrowserSearchRequest {
    #[serde(default)]
    query: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MascotBrowserSearchPayload {
    query: String,
    session_id: String,
    execution_tier: String,
    page_url: Option<String>,
    title: Option<String>,
    first_url: Option<String>,
    snapshot: String,
}

pub(super) async fn mascot_browser_theme_toggle(
    State(state): State<Arc<WebApiState>>,
    Json(request): Json<MascotBrowserThemeToggleRequest>,
) -> Result<Json<ApiEnvelope<MascotBrowserThemeTogglePayload>>, WebApiError> {
    let snapshot = load_web_snapshot(state.as_ref())?;
    let mut tool_runtime =
        mvp::tools::runtime_config::ToolRuntimeConfig::from_loong_config(&snapshot.config, None);
    tool_runtime.browser_companion.timeout_seconds =
        tool_runtime.browser_companion.timeout_seconds.min(8);
    if !tool_runtime.browser_companion.is_runtime_ready() {
        return Err(WebApiError::bad_request(
            "browser companion is enabled but not ready",
        ));
    }

    let page_url = normalize_mascot_page_url(request.page_url.as_str())?;
    let session_scope = MASCOT_BROWSER_SCOPE_ID;
    let requested_session_id = mascot_browser_session_id(MASCOT_THEME_TOGGLE_SESSION_PREFIX);
    let session_store_config =
        mvp::session::store::SessionStoreConfig::from_memory_config(&snapshot.config.memory);

    let start_payload = execute_browser_companion_core_tool(
        &tool_runtime,
        "browser.companion.session.start",
        json!({
            "url": page_url,
            "session_id": requested_session_id,
            "__loong_browser_scope": session_scope,
        }),
    )?;

    let session_id = browser_companion_payload_string(&start_payload, "session_id")?.to_owned();
    let execution_tier =
        browser_companion_payload_string(&start_payload, "execution_tier")?.to_owned();

    let action_result = (|| -> Result<MascotBrowserThemeTogglePayload, WebApiError> {
        let click_payload = execute_browser_companion_app_tool(
            &snapshot.config.tools,
            &session_store_config,
            "browser.companion.click",
            json!({
                "session_id": session_id,
                "selector": MASCOT_THEME_TOGGLE_SELECTOR,
            }),
            session_scope,
        )?;
        let clicked = browser_companion_result_bool(&click_payload, "clicked").unwrap_or(false);

        let _wait_payload = execute_browser_companion_core_tool(
            &tool_runtime,
            "browser.companion.wait",
            json!({
                "session_id": session_id,
                "__loong_browser_scope": session_scope,
                "condition": "500",
                "timeout_ms": 1500,
            }),
        )?;

        let snapshot_payload = execute_browser_companion_core_tool(
            &tool_runtime,
            "browser.companion.snapshot",
            json!({
                "session_id": session_id.as_str(),
                "__loong_browser_scope": session_scope,
                "mode": "summary",
            }),
        )?;

        Ok(MascotBrowserThemeTogglePayload {
            session_id: session_id.clone(),
            execution_tier,
            page_url: browser_companion_result_string(&snapshot_payload, "page_url")
                .map(ToOwned::to_owned),
            title: browser_companion_result_string(&snapshot_payload, "title")
                .map(ToOwned::to_owned),
            clicked,
            snapshot: browser_companion_result_string(&snapshot_payload, "snapshot")
                .unwrap_or_default()
                .to_owned(),
        })
    })();

    let _ = execute_browser_companion_core_tool(
        &tool_runtime,
        "browser.companion.session.stop",
        json!({
            "session_id": session_id,
            "__loong_browser_scope": session_scope,
        }),
    );

    let payload = action_result?;

    Ok(Json(ApiEnvelope {
        ok: true,
        data: payload,
    }))
}

pub(super) async fn mascot_browser_search(
    State(state): State<Arc<WebApiState>>,
    Json(request): Json<MascotBrowserSearchRequest>,
) -> Result<Json<ApiEnvelope<MascotBrowserSearchPayload>>, WebApiError> {
    let snapshot = load_web_snapshot(state.as_ref())?;
    let mut tool_runtime =
        mvp::tools::runtime_config::ToolRuntimeConfig::from_loong_config(&snapshot.config, None);
    tool_runtime.browser_companion.timeout_seconds =
        tool_runtime.browser_companion.timeout_seconds.min(30);
    if !tool_runtime.browser_companion.is_runtime_ready() {
        return Err(WebApiError::bad_request(
            "browser companion is enabled but not ready",
        ));
    }

    let query = normalize_search_query(request.query.as_deref());
    let search_url = build_yahoo_search_url(query.as_str())?;
    let session_scope = MASCOT_BROWSER_SCOPE_ID;
    let requested_session_id = mascot_browser_session_id(MASCOT_SEARCH_SESSION_PREFIX);

    let start_payload = execute_browser_companion_core_tool(
        &tool_runtime,
        "browser.companion.session.start",
        json!({
            "url": search_url,
            "session_id": requested_session_id,
            "__loong_browser_scope": session_scope,
        }),
    )?;

    let session_id = browser_companion_payload_string(&start_payload, "session_id")?.to_owned();
    let execution_tier =
        browser_companion_payload_string(&start_payload, "execution_tier")?.to_owned();

    let action_result = (|| -> Result<MascotBrowserSearchPayload, WebApiError> {
        let snapshot_payload = execute_browser_companion_core_tool(
            &tool_runtime,
            "browser.companion.snapshot",
            json!({
                "session_id": session_id.as_str(),
                "__loong_browser_scope": session_scope,
                "mode": "links",
            }),
        )?;
        let snapshot = browser_companion_result_string(&snapshot_payload, "snapshot")
            .unwrap_or_default()
            .to_owned();

        Ok(MascotBrowserSearchPayload {
            query,
            session_id: session_id.clone(),
            execution_tier,
            page_url: browser_companion_result_string(&snapshot_payload, "page_url")
                .map(ToOwned::to_owned),
            title: browser_companion_result_string(&snapshot_payload, "title")
                .map(ToOwned::to_owned),
            first_url: first_search_result_url(&snapshot_payload, snapshot.as_str()),
            snapshot,
        })
    })();

    let _ = execute_browser_companion_core_tool(
        &tool_runtime,
        "browser.companion.session.stop",
        json!({
            "session_id": session_id,
            "__loong_browser_scope": session_scope,
        }),
    );

    let payload = action_result?;

    Ok(Json(ApiEnvelope {
        ok: true,
        data: payload,
    }))
}

fn normalize_mascot_page_url(raw: &str) -> Result<String, WebApiError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(WebApiError::bad_request("pageUrl must not be empty"));
    }

    let parsed = reqwest::Url::parse(trimmed)
        .map_err(|error| WebApiError::bad_request(format!("pageUrl is invalid: {error}")))?;
    match parsed.scheme() {
        "http" | "https" => Ok(parsed.to_string()),
        _ => Err(WebApiError::bad_request("pageUrl must use http or https")),
    }
}

fn normalize_search_query(raw: Option<&str>) -> String {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(MASCOT_DEFAULT_SEARCH_QUERY)
        .chars()
        .take(64)
        .collect::<String>()
}

fn build_yahoo_search_url(query: &str) -> Result<String, WebApiError> {
    let query = query.trim();
    if query.is_empty() {
        return Err(WebApiError::bad_request("search query must not be empty"));
    }

    // Keep the query readable for the browser companion. On Windows the companion
    // invokes a .cmd shim, and percent-encoded non-ASCII queries can be mangled
    // before they reach the browser.
    Ok(format!("https://search.yahoo.com/search?p={query}"))
}

fn first_search_result_url(payload: &Value, snapshot: &str) -> Option<String> {
    let mut candidates = Vec::new();

    if let Some(refs) = payload
        .get("result")
        .and_then(Value::as_object)
        .and_then(|result| result.get("refs"))
    {
        collect_url_candidates(refs, &mut candidates);
    }

    for captures in snapshot.match_indices("href=\"") {
        let start = captures.0 + "href=\"".len();
        let rest = &snapshot[start..];
        if let Some(end) = rest.find('"') {
            candidates.push(rest[..end].to_owned());
        }
    }
    for line in snapshot.lines() {
        if let Some((_, rest)) = line.split_once("url=") {
            let end = rest.find(']').unwrap_or(rest.len());
            candidates.push(rest[..end].trim().to_owned());
        }
    }

    candidates
        .into_iter()
        .filter_map(|candidate| normalize_search_result_url(candidate.as_str()))
        .next()
}

fn collect_url_candidates(value: &Value, output: &mut Vec<String>) {
    match value {
        Value::String(candidate) => {
            if candidate.starts_with("http://") || candidate.starts_with("https://") {
                output.push(candidate.to_owned());
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_url_candidates(item, output);
            }
        }
        Value::Object(object) => {
            for (key, item) in object {
                if matches!(key.as_str(), "href" | "url")
                    && let Some(candidate) = item.as_str()
                {
                    output.push(candidate.to_owned());
                }
                collect_url_candidates(item, output);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn normalize_search_result_url(raw: &str) -> Option<String> {
    let candidate = raw.trim().replace("&amp;", "&");
    let parsed = reqwest::Url::parse(candidate.as_str()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }

    let host = parsed.host_str()?.to_ascii_lowercase();
    let rejected_hosts = [
        "bing.com",
        "duckduckgo.com",
        "google.com",
        "search.yahoo.com",
        "yahoo.com",
    ];
    if rejected_hosts
        .iter()
        .any(|rejected| host == *rejected || host.ends_with(format!(".{rejected}").as_str()))
    {
        return None;
    }

    Some(parsed.to_string())
}

fn mascot_browser_session_id(prefix: &str) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("{prefix}-{millis}")
}

fn execute_browser_companion_core_tool(
    runtime: &mvp::tools::runtime_config::ToolRuntimeConfig,
    tool_name: &str,
    payload: Value,
) -> Result<Value, WebApiError> {
    mvp::tools::execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: tool_name.to_owned(),
            payload,
        },
        runtime,
    )
    .map(|outcome| outcome.payload)
    .map_err(map_browser_companion_error)
}

fn execute_browser_companion_app_tool(
    tool_config: &mvp::config::ToolConfig,
    session_store_config: &mvp::session::store::SessionStoreConfig,
    tool_name: &str,
    payload: Value,
    current_session_id: &str,
) -> Result<Value, WebApiError> {
    mvp::tools::execute_app_tool_with_config(
        ToolCoreRequest {
            tool_name: tool_name.to_owned(),
            payload,
        },
        current_session_id,
        session_store_config,
        tool_config,
    )
    .map(|outcome| outcome.payload)
    .map_err(map_browser_companion_error)
}

fn map_browser_companion_error(error: String) -> WebApiError {
    let normalized = error.to_ascii_lowercase();
    if normalized.contains("requires payload")
        || normalized.contains("invalid")
        || normalized.contains("blocked host")
        || normalized.contains("not_ready")
        || normalized.contains("not ready")
        || normalized.contains("unknown_session")
        || normalized.contains("disabled")
    {
        WebApiError::bad_request(error)
    } else {
        WebApiError::internal(error)
    }
}

fn browser_companion_payload_string<'a>(
    payload: &'a Value,
    field: &str,
) -> Result<&'a str, WebApiError> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            WebApiError::internal(format!(
                "browser companion payload did not include `{field}`"
            ))
        })
}

fn browser_companion_result_string<'a>(payload: &'a Value, field: &str) -> Option<&'a str> {
    payload
        .get("result")
        .and_then(Value::as_object)
        .and_then(|result| result.get(field))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn browser_companion_result_bool(payload: &Value, field: &str) -> Option<bool> {
    payload
        .get("result")
        .and_then(Value::as_object)
        .and_then(|result| result.get(field))
        .and_then(Value::as_bool)
}
