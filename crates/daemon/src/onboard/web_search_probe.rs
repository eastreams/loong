use std::env;

use loong_app as mvp;

use crate::onboard::web_search::WebSearchEnvironmentSignals;

pub(crate) async fn detect_web_search_environment_signals() -> WebSearchEnvironmentSignals {
    let domestic_locale_hint = onboarding_locale_looks_domestic_cn();
    let duckduckgo_reachable = probe_duckduckgo_route().await;
    let tavily_reachable = probe_tavily_route().await;
    WebSearchEnvironmentSignals {
        domestic_locale_hint,
        duckduckgo_reachable,
        tavily_reachable,
    }
}

fn onboarding_locale_looks_domestic_cn() -> bool {
    let locale_matches = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|key| env::var(key).ok())
        .any(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            normalized.contains("zh_cn")
                || normalized.contains("zh-hans")
                || normalized.starts_with("zh-cn")
        });
    if locale_matches {
        return true;
    }

    let timezone = env::var("TZ").ok();
    let Some(timezone) = timezone else {
        return false;
    };
    let normalized = timezone.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "asia/shanghai" | "asia/chongqing" | "asia/harbin" | "asia/urumqi" | "asia/beijing"
    )
}

async fn probe_duckduckgo_route() -> bool {
    let Some(client) = build_onboard_probe_client() else {
        return false;
    };
    let request = client.get("https://html.duckduckgo.com/html/?q=loong");
    let response = request.send().await;
    match response {
        Ok(response) => response.status().is_success() || response.status().is_redirection(),
        Err(_) => false,
    }
}

async fn probe_tavily_route() -> bool {
    let Some(client) = build_onboard_probe_client() else {
        return false;
    };
    let request = client
        .post("https://api.tavily.com/search")
        .header("Content-Type", "application/json")
        .body(r#"{"query":"loong","max_results":1}"#);
    let response = request.send().await;
    match response {
        Ok(response) => {
            let status = response.status();
            status.is_success() || status.is_redirection() || status.is_client_error()
        }
        Err(_) => false,
    }
}

fn build_onboard_probe_client() -> Option<reqwest::Client> {
    build_onboard_probe_client_with_user_agent("Loong-Onboard/0.1")
}

#[cfg(test)]
pub(crate) fn build_onboard_probe_client_with_user_agent(
    user_agent: &str,
) -> Option<reqwest::Client> {
    let client = mvp::tools::build_ssrf_safe_client(false, 2, user_agent);
    client.ok()
}

#[cfg(not(test))]
pub(crate) fn build_onboard_probe_client_with_user_agent(
    user_agent: &str,
) -> Option<reqwest::Client> {
    let client = mvp::tools::build_ssrf_safe_client(false, 2, user_agent);
    client.ok()
}
