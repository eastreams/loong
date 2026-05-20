use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

fn bool_is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BrowserRuntimeNarrowing {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_sessions: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_links: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_text_chars: Option<usize>,
}

impl BrowserRuntimeNarrowing {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.max_sessions.is_none() && self.max_links.is_none() && self.max_text_chars.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WebFetchRuntimeNarrowing {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_private_hosts: Option<bool>,
    #[serde(default, skip_serializing_if = "bool_is_false")]
    pub enforce_allowed_domains: bool,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub allowed_domains: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub blocked_domains: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_redirects: Option<usize>,
}

impl WebFetchRuntimeNarrowing {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.allow_private_hosts.is_none()
            && !self.enforces_allowed_domains()
            && self.allowed_domains.is_empty()
            && self.blocked_domains.is_empty()
            && self.timeout_seconds.is_none()
            && self.max_bytes.is_none()
            && self.max_redirects.is_none()
    }

    #[must_use]
    pub fn enforces_allowed_domains(&self) -> bool {
        self.enforce_allowed_domains || !self.allowed_domains.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ToolRuntimeNarrowing {
    #[serde(default)]
    pub browser: BrowserRuntimeNarrowing,
    #[serde(default)]
    pub web_fetch: WebFetchRuntimeNarrowing,
}

impl ToolRuntimeNarrowing {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.browser.is_empty() && self.web_fetch.is_empty()
    }

    #[must_use]
    pub fn intersect(&self, other: &Self) -> Self {
        if self.is_empty() {
            return other.clone();
        }
        if other.is_empty() {
            return self.clone();
        }

        let browser = BrowserRuntimeNarrowing {
            max_sessions: min_optional_limit(self.browser.max_sessions, other.browser.max_sessions),
            max_links: min_optional_limit(self.browser.max_links, other.browser.max_links),
            max_text_chars: min_optional_limit(
                self.browser.max_text_chars,
                other.browser.max_text_chars,
            ),
        };

        let left_enforces_allowed_domains = self.web_fetch.enforces_allowed_domains();
        let right_enforces_allowed_domains = other.web_fetch.enforces_allowed_domains();
        let mut allowed_domains = BTreeSet::new();
        let mut enforce_allowed_domains = false;

        if left_enforces_allowed_domains && right_enforces_allowed_domains {
            enforce_allowed_domains = true;
            let left_is_deny_all = self.web_fetch.allowed_domains.is_empty();
            let right_is_deny_all = other.web_fetch.allowed_domains.is_empty();
            if !left_is_deny_all && !right_is_deny_all {
                allowed_domains = self
                    .web_fetch
                    .allowed_domains
                    .intersection(&other.web_fetch.allowed_domains)
                    .cloned()
                    .collect();
            }
        } else if left_enforces_allowed_domains {
            enforce_allowed_domains = true;
            allowed_domains = self.web_fetch.allowed_domains.clone();
        } else if right_enforces_allowed_domains {
            enforce_allowed_domains = true;
            allowed_domains = other.web_fetch.allowed_domains.clone();
        }

        let allow_private_hosts = intersect_private_host_setting(
            self.web_fetch.allow_private_hosts,
            other.web_fetch.allow_private_hosts,
        );

        let blocked_domains = self
            .web_fetch
            .blocked_domains
            .union(&other.web_fetch.blocked_domains)
            .cloned()
            .collect();

        let web_fetch = WebFetchRuntimeNarrowing {
            allow_private_hosts,
            enforce_allowed_domains,
            allowed_domains,
            blocked_domains,
            timeout_seconds: min_optional_limit(
                self.web_fetch.timeout_seconds,
                other.web_fetch.timeout_seconds,
            ),
            max_bytes: min_optional_limit(self.web_fetch.max_bytes, other.web_fetch.max_bytes),
            max_redirects: min_optional_limit(
                self.web_fetch.max_redirects,
                other.web_fetch.max_redirects,
            ),
        };

        Self { browser, web_fetch }
    }
}

fn min_optional_limit<T>(left: Option<T>, right: Option<T>) -> Option<T>
where
    T: Ord + Copy,
{
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn intersect_private_host_setting(left: Option<bool>, right: Option<bool>) -> Option<bool> {
    if left == Some(false) || right == Some(false) {
        return Some(false);
    }
    None
}

pub(crate) fn merge_runtime_narrowing_sources(
    primary_runtime_narrowing: Option<ToolRuntimeNarrowing>,
    secondary_runtime_narrowing: Option<ToolRuntimeNarrowing>,
) -> Option<ToolRuntimeNarrowing> {
    let primary_runtime_narrowing =
        primary_runtime_narrowing.filter(|runtime_narrowing| !runtime_narrowing.is_empty());
    let secondary_runtime_narrowing =
        secondary_runtime_narrowing.filter(|runtime_narrowing| !runtime_narrowing.is_empty());

    match (primary_runtime_narrowing, secondary_runtime_narrowing) {
        (Some(primary_runtime_narrowing), Some(secondary_runtime_narrowing)) => {
            let merged_runtime_narrowing =
                primary_runtime_narrowing.intersect(&secondary_runtime_narrowing);
            Some(merged_runtime_narrowing)
        }
        (Some(primary_runtime_narrowing), None) => Some(primary_runtime_narrowing),
        (None, Some(secondary_runtime_narrowing)) => Some(secondary_runtime_narrowing),
        (None, None) => None,
    }
}
