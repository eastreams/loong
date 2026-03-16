use std::collections::BTreeMap;
use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_TYPE, LOCATION};
use scraper::{Html, Selector};
use serde_json::{Map, Value, json};

const DEFAULT_BROWSER_SCOPE_ID: &str = "__global";

#[derive(Clone)]
struct BrowserLink {
    id: usize,
    text: String,
    url: String,
}

#[derive(Clone)]
struct BrowserPage {
    requested_url: String,
    final_url: String,
    host: String,
    title: Option<String>,
    content_type: Option<String>,
    raw_html: String,
    page_text: String,
    links: Vec<BrowserLink>,
    bytes_downloaded: usize,
    redirect_count: usize,
}

struct BrowserSession {
    client: Client,
    page: BrowserPage,
    sequence: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowserExtractMode {
    PageText,
    Title,
    Links,
    SelectorText,
}

impl BrowserExtractMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::PageText => "page_text",
            Self::Title => "title",
            Self::Links => "links",
            Self::SelectorText => "selector_text",
        }
    }
}

static NEXT_BROWSER_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static BROWSER_SESSIONS: OnceLock<Mutex<BTreeMap<String, BTreeMap<String, BrowserSession>>>> =
    OnceLock::new();

fn browser_sessions() -> &'static Mutex<BTreeMap<String, BTreeMap<String, BrowserSession>>> {
    BROWSER_SESSIONS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn next_browser_sequence() -> u64 {
    NEXT_BROWSER_SEQUENCE.fetch_add(1, Ordering::Relaxed)
}

pub(super) fn execute_browser_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    if !config.browser.enabled {
        return Err("browser tools are disabled by config.tools.browser.enabled=false".to_owned());
    }

    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| format!("{} payload must be an object", request.tool_name))?;

    match request.tool_name.as_str() {
        "browser.open" => execute_browser_open(payload, config),
        "browser.extract" => execute_browser_extract(payload, config),
        "browser.click" => execute_browser_click(payload, config),
        _ => Err(format!(
            "tool_not_found: unknown browser tool `{}`",
            request.tool_name
        )),
    }
}

fn execute_browser_open(
    payload: &Map<String, Value>,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let raw_url = parse_required_string(payload, "url", "browser.open")?;
    reject_caller_supplied_session_id(payload, "browser.open")?;
    let scope_id = browser_scope_id_from_payload(payload);
    let sequence = next_browser_sequence();
    let session_id = format!("browser-{}", sequence);
    let max_bytes = parse_max_bytes(payload, config.web_fetch.max_bytes, "browser.open")?;
    let client = build_browser_client(config)?;
    let page = fetch_browser_page(&client, raw_url.as_str(), max_bytes, config)?;
    let response_payload = browser_page_payload("browser.open", &session_id, &page, None);
    store_browser_session(
        scope_id,
        session_id,
        BrowserSession {
            client,
            page,
            sequence,
        },
        config.browser.max_sessions,
    )?;
    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: response_payload,
    })
}

fn execute_browser_extract(
    payload: &Map<String, Value>,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let scope_id = browser_scope_id_from_payload(payload);
    let session_id = parse_required_string(payload, "session_id", "browser.extract")?;
    let mode = parse_extract_mode(payload)?;
    let limit = parse_optional_limit(
        payload,
        "limit",
        config.browser.max_links,
        "browser.extract",
    )?;
    let selector = parse_optional_string(payload, "selector");

    let (page, sequence) = {
        let sessions = browser_sessions()
            .lock()
            .map_err(|error| format!("browser session store lock poisoned: {error}"))?;
        let session = sessions
            .get(scope_id.as_str())
            .and_then(|scope_sessions| scope_sessions.get(session_id.as_str()))
            .ok_or_else(|| format!("browser.extract unknown session `{session_id}`"))?;
        (session.page.clone(), session.sequence)
    };
    touch_browser_session(scope_id.as_str(), session_id.as_str(), sequence)?;

    let payload = match mode {
        BrowserExtractMode::PageText => json!({
            "adapter": "core-tools",
            "tool_name": "browser.extract",
            "session_id": session_id,
            "mode": mode.as_str(),
            "final_url": page.final_url,
            "title": page.title,
            "content": truncate_chars(&page.page_text, config.browser.max_text_chars),
        }),
        BrowserExtractMode::Title => json!({
            "adapter": "core-tools",
            "tool_name": "browser.extract",
            "session_id": session_id,
            "mode": mode.as_str(),
            "final_url": page.final_url,
            "content": page.title.unwrap_or_default(),
        }),
        BrowserExtractMode::Links => json!({
            "adapter": "core-tools",
            "tool_name": "browser.extract",
            "session_id": session_id,
            "mode": mode.as_str(),
            "final_url": page.final_url,
            "links": page.links.iter().take(limit).map(browser_link_json).collect::<Vec<_>>(),
        }),
        BrowserExtractMode::SelectorText => {
            let selector = selector.ok_or_else(|| {
                "browser.extract selector_text mode requires payload.selector".to_owned()
            })?;
            let selector = Selector::parse(selector.as_str()).map_err(|error| {
                format!("browser.extract selector_text mode could not parse selector: {error}")
            })?;
            let document = Html::parse_document(&page.raw_html);
            let items = document
                .select(&selector)
                .map(|node| collapse_whitespace(&node.text().collect::<Vec<_>>().join(" ")))
                .filter(|text| !text.is_empty())
                .take(limit)
                .map(|text| truncate_chars(&text, config.browser.max_text_chars))
                .collect::<Vec<_>>();
            json!({
                "adapter": "core-tools",
                "tool_name": "browser.extract",
                "session_id": session_id,
                "mode": mode.as_str(),
                "final_url": page.final_url,
                "items": items,
            })
        }
    };

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload,
    })
}

fn execute_browser_click(
    payload: &Map<String, Value>,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let scope_id = browser_scope_id_from_payload(payload);
    let session_id = parse_required_string(payload, "session_id", "browser.click")?;
    let link_id = parse_required_link_id(payload)?;

    let (client, selected_link) = {
        let sessions = browser_sessions()
            .lock()
            .map_err(|error| format!("browser session store lock poisoned: {error}"))?;
        let session = sessions
            .get(scope_id.as_str())
            .and_then(|scope_sessions| scope_sessions.get(session_id.as_str()))
            .ok_or_else(|| format!("browser.click unknown session `{session_id}`"))?;
        let selected_link = session
            .page
            .links
            .iter()
            .find(|link| link.id == link_id)
            .cloned()
            .ok_or_else(|| {
                format!("browser.click could not find link_id {link_id} in session `{session_id}`")
            })?;
        (session.client.clone(), selected_link)
    };

    let page = fetch_browser_page(
        &client,
        selected_link.url.as_str(),
        config.web_fetch.max_bytes,
        config,
    )?;
    let response_payload = browser_page_payload(
        "browser.click",
        &session_id,
        &page,
        Some(json!({
            "id": selected_link.id,
            "text": selected_link.text,
            "url": selected_link.url,
        })),
    );
    store_browser_session(
        scope_id,
        session_id,
        BrowserSession {
            client,
            page,
            sequence: next_browser_sequence(),
        },
        config.browser.max_sessions,
    )?;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: response_payload,
    })
}

fn build_browser_client(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<Client, String> {
    Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(config.web_fetch.timeout_seconds))
        .user_agent("LoongClaw-Browser/0.1")
        .build()
        .map_err(|error| format!("failed to build HTTP client for browser tools: {error}"))
}

fn fetch_browser_page(
    client: &Client,
    raw_url: &str,
    max_bytes: usize,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<BrowserPage, String> {
    let mut current_url = reqwest::Url::parse(raw_url)
        .map_err(|error| format!("invalid browser url `{raw_url}`: {error}"))?;
    let mut current_host =
        super::web_fetch::validate_web_target(&current_url, &config.web_fetch, "browser")?;
    let mut redirect_count = 0usize;

    loop {
        let response = client
            .get(current_url.clone())
            .send()
            .map_err(|error| format!("browser request failed: {error}"))?;

        if response.status().is_redirection() {
            if redirect_count >= config.web_fetch.max_redirects {
                return Err(format!(
                    "browser exceeded redirect limit ({})",
                    config.web_fetch.max_redirects
                ));
            }
            let location = response
                .headers()
                .get(LOCATION)
                .ok_or_else(|| {
                    format!(
                        "browser received redirect status {} without Location header",
                        response.status()
                    )
                })?
                .to_str()
                .map_err(|error| {
                    format!("browser redirect Location header was invalid: {error}")
                })?;
            let next_url = current_url
                .join(location)
                .map_err(|error| format!("browser failed to resolve redirect target: {error}"))?;
            current_host =
                super::web_fetch::validate_web_target(&next_url, &config.web_fetch, "browser")?;
            current_url = next_url;
            redirect_count += 1;
            continue;
        }

        if !response.status().is_success() {
            return Err(format!(
                "browser returned non-success status {} for `{}`",
                response.status(),
                current_url
            ));
        }

        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.to_owned());
        let mut body = Vec::new();
        let mut limited_reader = response.take((max_bytes as u64).saturating_add(1));
        limited_reader
            .read_to_end(&mut body)
            .map_err(|error| format!("failed to read browser response body: {error}"))?;
        if body.len() > max_bytes {
            return Err(format!(
                "browser response exceeded max_bytes limit ({max_bytes} bytes)"
            ));
        }

        let raw_text = String::from_utf8_lossy(&body).into_owned();
        let is_html = super::web_fetch::looks_like_html(content_type.as_deref(), raw_text.as_str());
        if !is_html && super::web_fetch::response_is_probably_binary(content_type.as_deref(), &body)
        {
            return Err(
                "browser tools only support text-like responses; binary bodies are not returned"
                    .to_owned(),
            );
        }

        let title = is_html
            .then(|| super::web_fetch::extract_html_title(raw_text.as_str()))
            .flatten();
        let page_text = if is_html {
            super::web_fetch::extract_readable_text_from_html(&raw_text)
        } else {
            raw_text.trim().to_owned()
        };
        let page_text = truncate_chars(&page_text, config.browser.max_text_chars);
        let links = if is_html {
            discover_page_links(
                &current_url,
                raw_text.as_str(),
                &config.web_fetch,
                config.browser.max_links,
            )?
        } else {
            Vec::new()
        };

        return Ok(BrowserPage {
            requested_url: raw_url.to_owned(),
            final_url: current_url.to_string(),
            host: current_host,
            title,
            content_type,
            raw_html: raw_text,
            page_text,
            links,
            bytes_downloaded: body.len(),
            redirect_count,
        });
    }
}

fn discover_page_links(
    current_url: &reqwest::Url,
    html: &str,
    policy: &super::runtime_config::WebFetchRuntimePolicy,
    max_links: usize,
) -> Result<Vec<BrowserLink>, String> {
    let document = Html::parse_document(html);
    let selector = Selector::parse("a[href]")
        .map_err(|error| format!("browser could not parse anchor selector: {error}"))?;
    let mut seen = BTreeMap::<String, BrowserLink>::new();

    for element in document.select(&selector) {
        let Some(raw_href) = element.value().attr("href") else {
            continue;
        };
        let href = raw_href.trim();
        if href.is_empty() {
            continue;
        }
        let Ok(resolved_url) = current_url.join(href) else {
            continue;
        };
        if !matches!(resolved_url.scheme(), "http" | "https") {
            continue;
        }
        if super::web_fetch::validate_web_target(&resolved_url, policy, "browser.click").is_err() {
            continue;
        }

        let absolute_url = resolved_url.to_string();
        if seen.contains_key(&absolute_url) {
            continue;
        }
        let text = collapse_whitespace(&element.text().collect::<Vec<_>>().join(" "));
        seen.insert(
            absolute_url.clone(),
            BrowserLink {
                id: 0,
                text: if text.is_empty() {
                    absolute_url.clone()
                } else {
                    text
                },
                url: absolute_url,
            },
        );
        if seen.len() >= max_links {
            break;
        }
    }

    Ok(seen
        .into_values()
        .enumerate()
        .map(|(index, mut link)| {
            link.id = index + 1;
            link
        })
        .collect())
}

fn store_browser_session(
    scope_id: String,
    session_id: String,
    session: BrowserSession,
    max_sessions: usize,
) -> Result<(), String> {
    let mut scopes = browser_sessions()
        .lock()
        .map_err(|error| format!("browser session store lock poisoned: {error}"))?;
    let scope_sessions = scopes.entry(scope_id).or_default();
    if !scope_sessions.contains_key(session_id.as_str()) && scope_sessions.len() >= max_sessions {
        let evict_id = scope_sessions
            .iter()
            .min_by_key(|(_, session)| session.sequence)
            .map(|(id, _)| id.clone())
            .ok_or_else(|| "browser session eviction failed".to_owned())?;
        let _ = scope_sessions.remove(evict_id.as_str());
    }
    scope_sessions.insert(session_id, session);
    Ok(())
}

fn touch_browser_session(scope_id: &str, session_id: &str, sequence: u64) -> Result<(), String> {
    let mut scopes = browser_sessions()
        .lock()
        .map_err(|error| format!("browser session store lock poisoned: {error}"))?;
    if let Some(session) = scopes
        .get_mut(scope_id)
        .and_then(|scope_sessions| scope_sessions.get_mut(session_id))
        && session.sequence == sequence
    {
        session.sequence = next_browser_sequence();
    }
    Ok(())
}

fn browser_page_payload(
    tool_name: &str,
    session_id: &str,
    page: &BrowserPage,
    clicked_link: Option<Value>,
) -> Value {
    json!({
        "adapter": "core-tools",
        "tool_name": tool_name,
        "session_id": session_id,
        "requested_url": page.requested_url,
        "final_url": page.final_url,
        "host": page.host,
        "title": page.title,
        "content_type": page.content_type,
        "page_text": page.page_text,
        "available_links": page.links.iter().map(browser_link_json).collect::<Vec<_>>(),
        "bytes_downloaded": page.bytes_downloaded,
        "redirect_count": page.redirect_count,
        "clicked_link": clicked_link,
    })
}

fn browser_link_json(link: &BrowserLink) -> Value {
    json!({
        "id": link.id,
        "text": link.text,
        "url": link.url,
    })
}

fn parse_required_string(
    payload: &Map<String, Value>,
    key: &str,
    tool_name: &str,
) -> Result<String, String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{tool_name} requires payload.{key}"))
}

fn parse_optional_string(payload: &Map<String, Value>, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn browser_scope_id_from_payload(payload: &Map<String, Value>) -> String {
    parse_optional_string(payload, super::BROWSER_SESSION_SCOPE_FIELD)
        .unwrap_or_else(|| DEFAULT_BROWSER_SCOPE_ID.to_owned())
}

fn reject_caller_supplied_session_id(
    payload: &Map<String, Value>,
    tool_name: &str,
) -> Result<(), String> {
    if payload.contains_key("session_id") {
        return Err(format!(
            "{tool_name} does not accept payload.session_id; use the opaque session_id returned by browser.open"
        ));
    }
    Ok(())
}

fn parse_required_link_id(payload: &Map<String, Value>) -> Result<usize, String> {
    let parsed = payload
        .get("link_id")
        .and_then(Value::as_u64)
        .ok_or_else(|| "browser.click requires payload.link_id".to_owned())?;
    usize::try_from(parsed)
        .map_err(|error| format!("browser.click payload.link_id is invalid `{parsed}`: {error}"))
}

fn parse_max_bytes(
    payload: &Map<String, Value>,
    configured_max: usize,
    tool_name: &str,
) -> Result<usize, String> {
    let Some(value) = payload.get("max_bytes") else {
        return Ok(configured_max);
    };
    let parsed = value
        .as_u64()
        .ok_or_else(|| format!("{tool_name} payload.max_bytes must be an integer"))?;
    if parsed == 0 {
        return Err(format!("{tool_name} payload.max_bytes must be >= 1"));
    }
    let parsed = usize::try_from(parsed)
        .map_err(|error| format!("{tool_name} payload.max_bytes is invalid `{parsed}`: {error}"))?;
    if parsed > configured_max {
        return Err(format!(
            "{tool_name} payload.max_bytes exceeds configured limit ({configured_max} bytes)"
        ));
    }
    Ok(parsed)
}

fn parse_optional_limit(
    payload: &Map<String, Value>,
    key: &str,
    configured_max: usize,
    tool_name: &str,
) -> Result<usize, String> {
    let Some(value) = payload.get(key) else {
        return Ok(configured_max);
    };
    let parsed = value
        .as_u64()
        .ok_or_else(|| format!("{tool_name} payload.{key} must be an integer"))?;
    if parsed == 0 {
        return Err(format!("{tool_name} payload.{key} must be >= 1"));
    }
    let parsed = usize::try_from(parsed)
        .map_err(|error| format!("{tool_name} payload.{key} is invalid `{parsed}`: {error}"))?;
    if parsed > configured_max {
        return Err(format!(
            "{tool_name} payload.{key} exceeds configured limit ({configured_max})"
        ));
    }
    Ok(parsed)
}

fn parse_extract_mode(payload: &Map<String, Value>) -> Result<BrowserExtractMode, String> {
    let Some(value) = payload.get("mode") else {
        return Ok(BrowserExtractMode::PageText);
    };
    let raw = value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "browser.extract payload.mode must be a string".to_owned())?;
    match raw {
        "page_text" => Ok(BrowserExtractMode::PageText),
        "title" => Ok(BrowserExtractMode::Title),
        "links" => Ok(BrowserExtractMode::Links),
        "selector_text" => Ok(BrowserExtractMode::SelectorText),
        _ => Err(
            "browser.extract payload.mode must be one of `page_text`, `title`, `links`, or `selector_text`"
                .to_owned(),
        ),
    }
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    let total_chars = input.chars().count();
    if total_chars <= max_chars {
        return input.to_owned();
    }
    input.chars().take(max_chars).collect::<String>()
}

fn collapse_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;
    use std::io;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    fn request(tool_name: &str, payload: Value) -> ToolCoreRequest {
        ToolCoreRequest {
            tool_name: tool_name.to_owned(),
            payload,
        }
    }

    /// Create a request with a test-specific browser scope to isolate from parallel tests.
    fn scoped_request(tool_name: &str, mut payload: Value, scope: &str) -> ToolCoreRequest {
        payload.as_object_mut().unwrap().insert(
            super::super::BROWSER_SESSION_SCOPE_FIELD.to_owned(),
            json!(scope),
        );
        request(tool_name, payload)
    }

    fn local_browser_config() -> super::super::runtime_config::ToolRuntimeConfig {
        super::super::runtime_config::ToolRuntimeConfig {
            browser: super::super::runtime_config::BrowserRuntimePolicy {
                enabled: true,
                max_sessions: 4,
                max_links: 8,
                max_text_chars: 2048,
            },
            web_fetch: super::super::runtime_config::WebFetchRuntimePolicy {
                enabled: true,
                allow_private_hosts: true,
                allowed_domains: std::collections::BTreeSet::new(),
                blocked_domains: std::collections::BTreeSet::new(),
                timeout_seconds: 5,
                max_bytes: 64 * 1024,
                max_redirects: 2,
            },
            ..super::super::runtime_config::ToolRuntimeConfig::default()
        }
    }

    fn spawn_browser_fixture_server(expected_requests: usize) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        let address = listener.local_addr().expect("listener addr");
        let handle = thread::spawn(move || {
            let mut served_requests = 0_usize;
            while served_requests < expected_requests {
                let mut stream = listener
                    .accept()
                    .map(|(stream, _)| stream)
                    .expect("accept stream");
                stream
                    .set_read_timeout(Some(Duration::from_millis(200)))
                    .expect("set read timeout");
                loop {
                    let mut buffer = [0_u8; 4096];
                    let size = match stream.read(&mut buffer) {
                        Ok(size) => size,
                        Err(error)
                            if matches!(
                                error.kind(),
                                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                            ) =>
                        {
                            break;
                        }
                        Err(error) => panic!("read request failed: {error}"),
                    };
                    if size == 0 {
                        break;
                    }
                    served_requests += 1;
                    let request = String::from_utf8_lossy(&buffer[..size]).into_owned();
                    let mut lines = request.lines();
                    let request_line = lines.next().unwrap_or_default().to_owned();
                    let path = request_line.split_whitespace().nth(1).unwrap_or("/");

                    let response = match path {
                        "/" => build_http_response(
                            "200 OK",
                            "text/html; charset=utf-8",
                            "<html><head><title>Fixture Home</title></head>\
                             <body>\
                             <h1>Fixture Home</h1>\
                             <div class=\"feature\">Alpha</div>\
                             <div class=\"feature\">Beta</div>\
                             <a href=\"/next\">Continue</a>\
                             <a href=\"javascript:void(0)\">Ignored</a>\
                             </body></html>",
                            Some("Set-Cookie: lc_session=ready; Path=/"),
                        ),
                        "/next" => build_http_response(
                            "200 OK",
                            "text/html; charset=utf-8",
                            "<html><head><title>Next Page</title></head>\
                             <body><p>Page followed.</p></body></html>",
                            None,
                        ),
                        _ => build_http_response(
                            "404 Not Found",
                            "text/plain; charset=utf-8",
                            "not found",
                            None,
                        ),
                    };
                    stream
                        .write_all(response.as_bytes())
                        .expect("write response");
                    if served_requests >= expected_requests {
                        break;
                    }
                }
            }
        });
        (format!("http://127.0.0.1:{}", address.port()), handle)
    }

    fn build_http_response(
        status: &str,
        content_type: &str,
        body: &str,
        extra_header: Option<&str>,
    ) -> String {
        let mut response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        if let Some(extra_header) = extra_header {
            response.push_str(extra_header);
            response.push_str("\r\n");
        }
        response.push_str("\r\n");
        response.push_str(body);
        response
    }

    #[test]
    fn browser_open_requires_enabled_runtime() {
        let mut config = local_browser_config();
        config.browser.enabled = false;

        let error = execute_browser_tool_with_config(
            request("browser.open", json!({"url": "https://example.com"})),
            &config,
        )
        .expect_err("disabled browser tools should fail");

        assert!(error.contains("config.tools.browser.enabled=false"));
    }

    #[test]
    fn browser_open_rejects_private_hosts_by_default() {
        let config = super::super::runtime_config::ToolRuntimeConfig::default();
        let base_url = "http://127.0.0.1:6553".to_owned();

        let error = execute_browser_tool_with_config(
            request("browser.open", json!({"url": base_url})),
            &config,
        )
        .expect_err("private hosts should be blocked by default");

        assert!(error.contains("private or special-use"));
    }

    #[test]
    fn browser_open_rejects_caller_supplied_session_id() {
        let config = local_browser_config();

        let error = execute_browser_tool_with_config(
            request(
                "browser.open",
                json!({"url": "https://example.com", "session_id": "fixture"}),
            ),
            &config,
        )
        .expect_err("caller-supplied browser session ids should fail");

        assert!(error.contains("does not accept payload.session_id"));
    }

    #[test]
    fn browser_open_discovers_safe_links_and_page_text() {
        let (base_url, handle) = spawn_browser_fixture_server(1);
        let config = local_browser_config();

        let outcome = execute_browser_tool_with_config(
            scoped_request("browser.open", json!({"url": base_url}), "test-open-links"),
            &config,
        )
        .expect("browser.open should succeed");

        assert_eq!(outcome.status, "ok");
        assert!(
            outcome.payload["session_id"]
                .as_str()
                .expect("session id")
                .starts_with("browser-")
        );
        assert_eq!(outcome.payload["title"], json!("Fixture Home"));
        assert!(
            outcome.payload["page_text"]
                .as_str()
                .expect("page text")
                .contains("Fixture Home")
        );
        let links = outcome.payload["available_links"]
            .as_array()
            .expect("available links");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0]["id"], json!(1));
        assert_eq!(links[0]["text"], json!("Continue"));
        handle.join().expect("server thread");
    }

    #[test]
    fn browser_extract_returns_links_and_selector_text() {
        let (base_url, handle) = spawn_browser_fixture_server(1);
        let config = local_browser_config();

        let opened = execute_browser_tool_with_config(
            scoped_request("browser.open", json!({"url": base_url}), "test-extract"),
            &config,
        )
        .expect("browser.open should succeed");
        let session_id = opened.payload["session_id"]
            .as_str()
            .expect("session id")
            .to_owned();

        let links = execute_browser_tool_with_config(
            scoped_request(
                "browser.extract",
                json!({"session_id": &session_id, "mode": "links"}),
                "test-extract",
            ),
            &config,
        )
        .expect("browser.extract links should succeed");
        assert_eq!(links.payload["links"][0]["text"], json!("Continue"));

        let selector_text = execute_browser_tool_with_config(
            scoped_request(
                "browser.extract",
                json!({
                    "session_id": session_id,
                    "mode": "selector_text",
                    "selector": ".feature"
                }),
                "test-extract",
            ),
            &config,
        )
        .expect("browser.extract selector_text should succeed");
        assert_eq!(selector_text.payload["items"], json!(["Alpha", "Beta"]));
        handle.join().expect("server thread");
    }

    #[test]
    fn browser_click_follows_discovered_link_with_cookie_session() {
        let (base_url, handle) = spawn_browser_fixture_server(2);
        let config = local_browser_config();

        let opened = execute_browser_tool_with_config(
            scoped_request("browser.open", json!({"url": base_url}), "test-click"),
            &config,
        )
        .expect("browser.open should succeed");
        let session_id = opened.payload["session_id"]
            .as_str()
            .expect("session id")
            .to_owned();

        let clicked = execute_browser_tool_with_config(
            scoped_request(
                "browser.click",
                json!({"session_id": session_id, "link_id": 1}),
                "test-click",
            ),
            &config,
        )
        .expect("browser.click should succeed");

        assert_eq!(clicked.payload["title"], json!("Next Page"));
        assert_eq!(clicked.payload["clicked_link"]["id"], json!(1));
        assert!(
            clicked.payload["page_text"]
                .as_str()
                .expect("page text")
                .contains("Page followed")
        );
        handle.join().expect("server thread");
    }

    #[test]
    fn browser_sessions_are_isolated_by_internal_scope() {
        let (base_url, handle) = spawn_browser_fixture_server(1);
        let config = local_browser_config();

        let opened = execute_browser_tool_with_config(
            request(
                "browser.open",
                json!({
                    "url": base_url,
                    super::super::BROWSER_SESSION_SCOPE_FIELD: "scope-a"
                }),
            ),
            &config,
        )
        .expect("browser.open should succeed");
        let session_id = opened.payload["session_id"]
            .as_str()
            .expect("session id")
            .to_owned();

        let error = execute_browser_tool_with_config(
            request(
                "browser.extract",
                json!({
                    "session_id": session_id,
                    super::super::BROWSER_SESSION_SCOPE_FIELD: "scope-b"
                }),
            ),
            &config,
        )
        .expect_err("different browser scope should not be able to reuse the session");

        assert!(error.contains("unknown session"));
        handle.join().expect("server thread");
    }

    #[test]
    fn browser_scope_eviction_is_limited_per_scope() {
        let (base_url, handle) = spawn_browser_fixture_server(2);
        let mut config = local_browser_config();
        config.browser.max_sessions = 1;

        let opened_a = execute_browser_tool_with_config(
            request(
                "browser.open",
                json!({
                    "url": base_url,
                    super::super::BROWSER_SESSION_SCOPE_FIELD: "scope-a"
                }),
            ),
            &config,
        )
        .expect("scope-a browser.open should succeed");
        let session_a = opened_a.payload["session_id"]
            .as_str()
            .expect("scope-a session id")
            .to_owned();

        execute_browser_tool_with_config(
            request(
                "browser.open",
                json!({
                    "url": base_url,
                    super::super::BROWSER_SESSION_SCOPE_FIELD: "scope-b"
                }),
            ),
            &config,
        )
        .expect("scope-b browser.open should succeed");

        let extract_a = execute_browser_tool_with_config(
            request(
                "browser.extract",
                json!({
                    "session_id": session_a,
                    super::super::BROWSER_SESSION_SCOPE_FIELD: "scope-a"
                }),
            ),
            &config,
        )
        .expect("scope-a session should survive scope-b eviction activity");

        assert!(
            extract_a.payload["content"]
                .as_str()
                .expect("page text")
                .contains("Fixture Home")
        );
        handle.join().expect("server thread");
    }
}
