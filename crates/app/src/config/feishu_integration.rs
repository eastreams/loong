use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::shared::{
    ConfigValidationIssue, DEFAULT_FEISHU_SQLITE_FILE, default_loongclaw_home, expand_path,
    validate_numeric_range,
};

const MIN_FEISHU_OAUTH_STATE_TTL_S: usize = 60;
const MAX_FEISHU_OAUTH_STATE_TTL_S: usize = 86_400;
const MIN_FEISHU_REQUEST_TIMEOUT_S: usize = 3;
const MAX_FEISHU_REQUEST_TIMEOUT_S: usize = 120;
const MIN_FEISHU_RETRY_MAX_ATTEMPTS: usize = 1;
const MAX_FEISHU_RETRY_MAX_ATTEMPTS: usize = 8;
const MIN_FEISHU_RETRY_INITIAL_BACKOFF_MS: usize = 0;
const MAX_FEISHU_RETRY_INITIAL_BACKOFF_MS: usize = 30_000;
const MAX_FEISHU_RETRY_MAX_BACKOFF_MS: usize = 60_000;
const FEISHU_GROUP_MESSAGE_READ_SCOPE: &str = "im:message.group_msg";
const FEISHU_GROUP_MESSAGE_READ_SCOPE_LEGACY: &str = "im:message.group_msg:readonly";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeishuCapabilityConfig {
    #[serde(default = "default_true")]
    pub docs: bool,
    #[serde(default = "default_true")]
    pub messages: bool,
    #[serde(default = "default_true")]
    pub calendar: bool,
    #[serde(default)]
    pub bitable: bool,
}

impl Default for FeishuCapabilityConfig {
    fn default() -> Self {
        Self {
            docs: true,
            messages: true,
            calendar: true,
            bitable: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeishuIntegrationConfig {
    pub sqlite_path: String,
    pub oauth_state_ttl_s: usize,
    pub request_timeout_s: usize,
    pub retry_max_attempts: usize,
    pub retry_initial_backoff_ms: usize,
    pub retry_max_backoff_ms: usize,
    pub default_scopes: Vec<String>,
    pub capabilities: FeishuCapabilityConfig,
    pub capabilities_explicitly_configured: bool,
}

impl Serialize for FeishuIntegrationConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut state = serializer.serialize_struct(
            "FeishuIntegrationConfig",
            7 + usize::from(self.capabilities_explicitly_configured),
        )?;
        state.serialize_field("sqlite_path", &self.sqlite_path)?;
        state.serialize_field("oauth_state_ttl_s", &self.oauth_state_ttl_s)?;
        state.serialize_field("request_timeout_s", &self.request_timeout_s)?;
        state.serialize_field("retry_max_attempts", &self.retry_max_attempts)?;
        state.serialize_field("retry_initial_backoff_ms", &self.retry_initial_backoff_ms)?;
        state.serialize_field("retry_max_backoff_ms", &self.retry_max_backoff_ms)?;
        state.serialize_field("default_scopes", &self.default_scopes)?;
        if self.capabilities_explicitly_configured {
            state.serialize_field("capabilities", &self.capabilities)?;
        }
        state.end()
    }
}

#[derive(Debug, Deserialize)]
struct FeishuIntegrationConfigSerde {
    #[serde(default = "default_feishu_sqlite_path")]
    sqlite_path: String,
    #[serde(default = "default_oauth_state_ttl_s")]
    oauth_state_ttl_s: usize,
    #[serde(default = "default_request_timeout_s")]
    request_timeout_s: usize,
    #[serde(default = "default_retry_max_attempts")]
    retry_max_attempts: usize,
    #[serde(default = "default_retry_initial_backoff_ms")]
    retry_initial_backoff_ms: usize,
    #[serde(default = "default_retry_max_backoff_ms")]
    retry_max_backoff_ms: usize,
    #[serde(default = "default_scopes")]
    default_scopes: Vec<String>,
    #[serde(default)]
    capabilities: Option<FeishuCapabilityConfig>,
}

impl<'de> Deserialize<'de> for FeishuIntegrationConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = FeishuIntegrationConfigSerde::deserialize(deserializer)?;
        let capabilities_explicitly_configured = raw.capabilities.is_some();

        Ok(Self {
            sqlite_path: raw.sqlite_path,
            oauth_state_ttl_s: raw.oauth_state_ttl_s,
            request_timeout_s: raw.request_timeout_s,
            retry_max_attempts: raw.retry_max_attempts,
            retry_initial_backoff_ms: raw.retry_initial_backoff_ms,
            retry_max_backoff_ms: raw.retry_max_backoff_ms,
            default_scopes: raw.default_scopes,
            capabilities: raw.capabilities.unwrap_or_default(),
            capabilities_explicitly_configured,
        })
    }
}

impl Default for FeishuIntegrationConfig {
    fn default() -> Self {
        Self {
            sqlite_path: default_feishu_sqlite_path(),
            oauth_state_ttl_s: default_oauth_state_ttl_s(),
            request_timeout_s: default_request_timeout_s(),
            retry_max_attempts: default_retry_max_attempts(),
            retry_initial_backoff_ms: default_retry_initial_backoff_ms(),
            retry_max_backoff_ms: default_retry_max_backoff_ms(),
            default_scopes: default_scopes(),
            capabilities: FeishuCapabilityConfig::default(),
            capabilities_explicitly_configured: false,
        }
    }
}

impl FeishuIntegrationConfig {
    pub fn resolved_sqlite_path(&self) -> PathBuf {
        expand_path(&self.sqlite_path)
    }

    pub fn trimmed_default_scopes(&self) -> Vec<String> {
        let mut normalized = Vec::new();
        for raw in &self.default_scopes {
            let Some(scope) = normalize_scope_alias(raw) else {
                continue;
            };
            if normalized.iter().any(|existing| existing == &scope) {
                continue;
            }
            normalized.push(scope);
        }
        normalized
    }

    pub fn has_explicit_capability_config(&self) -> bool {
        self.capabilities_explicitly_configured
    }

    pub(super) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        if let Err(issue) = validate_numeric_range(
            "feishu_integration.oauth_state_ttl_s",
            self.oauth_state_ttl_s,
            MIN_FEISHU_OAUTH_STATE_TTL_S,
            MAX_FEISHU_OAUTH_STATE_TTL_S,
        ) {
            issues.push(*issue);
        }
        if let Err(issue) = validate_numeric_range(
            "feishu_integration.request_timeout_s",
            self.request_timeout_s,
            MIN_FEISHU_REQUEST_TIMEOUT_S,
            MAX_FEISHU_REQUEST_TIMEOUT_S,
        ) {
            issues.push(*issue);
        }
        if let Err(issue) = validate_numeric_range(
            "feishu_integration.retry_max_attempts",
            self.retry_max_attempts,
            MIN_FEISHU_RETRY_MAX_ATTEMPTS,
            MAX_FEISHU_RETRY_MAX_ATTEMPTS,
        ) {
            issues.push(*issue);
        }
        if let Err(issue) = validate_numeric_range(
            "feishu_integration.retry_initial_backoff_ms",
            self.retry_initial_backoff_ms,
            MIN_FEISHU_RETRY_INITIAL_BACKOFF_MS,
            MAX_FEISHU_RETRY_INITIAL_BACKOFF_MS,
        ) {
            issues.push(*issue);
        }
        if let Err(issue) = validate_numeric_range(
            "feishu_integration.retry_max_backoff_ms",
            self.retry_max_backoff_ms,
            self.retry_initial_backoff_ms,
            MAX_FEISHU_RETRY_MAX_BACKOFF_MS,
        ) {
            issues.push(*issue);
        }
        issues
    }
}

fn default_feishu_sqlite_path() -> String {
    default_loongclaw_home()
        .join(DEFAULT_FEISHU_SQLITE_FILE)
        .display()
        .to_string()
}

const fn default_oauth_state_ttl_s() -> usize {
    600
}

const fn default_request_timeout_s() -> usize {
    20
}

const fn default_retry_max_attempts() -> usize {
    4
}

const fn default_retry_initial_backoff_ms() -> usize {
    200
}

const fn default_retry_max_backoff_ms() -> usize {
    2_000
}

const fn default_true() -> bool {
    true
}

fn default_scopes() -> Vec<String> {
    vec![
        "offline_access".to_owned(),
        "docx:document:readonly".to_owned(),
        "im:message:readonly".to_owned(),
        "im:message.group_msg".to_owned(),
        "search:message".to_owned(),
        "calendar:calendar:readonly".to_owned(),
    ]
}

fn normalize_scope_alias(raw: &str) -> Option<String> {
    let scope = raw.trim();
    if scope.is_empty() {
        return None;
    }

    Some(match scope {
        FEISHU_GROUP_MESSAGE_READ_SCOPE_LEGACY => FEISHU_GROUP_MESSAGE_READ_SCOPE.to_owned(),
        _ => scope.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScopedLoongClawHome;

    #[test]
    fn feishu_integration_defaults_use_dedicated_runtime_db() {
        let _home = ScopedLoongClawHome::new("loongclaw-feishu-integration-home");
        let config = FeishuIntegrationConfig::default();
        assert_eq!(
            config.resolved_sqlite_path(),
            crate::config::default_loongclaw_home().join("feishu.sqlite3")
        );
        assert_eq!(config.oauth_state_ttl_s, 600);
        assert_eq!(config.request_timeout_s, 20);
        assert_eq!(config.retry_max_attempts, 4);
        assert_eq!(config.retry_initial_backoff_ms, 200);
        assert_eq!(config.retry_max_backoff_ms, 2_000);
        assert!(
            config
                .trimmed_default_scopes()
                .iter()
                .any(|scope| scope == "offline_access")
        );
        assert_eq!(
            config.capabilities,
            FeishuCapabilityConfig {
                docs: true,
                messages: true,
                calendar: true,
                bitable: false,
            }
        );
    }

    #[test]
    fn runtime_config_loads_feishu_integration_block() {
        let raw = r#"
            [feishu_integration]
            sqlite_path = "~/runtime/feishu.sqlite3"
            oauth_state_ttl_s = 900
            retry_max_attempts = 5
            retry_initial_backoff_ms = 150
            retry_max_backoff_ms = 900
            default_scopes = ["offline_access", "docs:document:readonly", "offline_access"]
        "#;

        let config: crate::config::LoongClawConfig = toml::from_str(raw).expect("parse config");

        assert_eq!(config.feishu_integration.oauth_state_ttl_s, 900);
        assert_eq!(config.feishu_integration.retry_max_attempts, 5);
        assert_eq!(config.feishu_integration.retry_initial_backoff_ms, 150);
        assert_eq!(config.feishu_integration.retry_max_backoff_ms, 900);
        assert_eq!(
            config.feishu_integration.resolved_sqlite_path(),
            crate::config::expand_path("~/runtime/feishu.sqlite3")
        );
        assert_eq!(
            config.feishu_integration.trimmed_default_scopes(),
            vec![
                "offline_access".to_owned(),
                "docs:document:readonly".to_owned()
            ]
        );
        assert_eq!(
            config.feishu_integration.capabilities,
            FeishuCapabilityConfig::default()
        );
    }

    #[test]
    fn runtime_config_loads_feishu_capabilities_block() {
        let raw = r#"
            [feishu_integration.capabilities]
            docs = true
            messages = true
            calendar = false
            bitable = true
        "#;

        let config: crate::config::LoongClawConfig = toml::from_str(raw).expect("parse config");

        assert!(config.feishu_integration.capabilities.docs);
        assert!(config.feishu_integration.capabilities.messages);
        assert!(!config.feishu_integration.capabilities.calendar);
        assert!(config.feishu_integration.capabilities.bitable);
    }

    #[test]
    fn runtime_config_marks_explicit_feishu_capabilities_block_even_when_values_match_defaults() {
        let raw = r#"
            [feishu_integration.capabilities]
            docs = true
            messages = true
            calendar = true
            bitable = false
        "#;

        let config: crate::config::LoongClawConfig = toml::from_str(raw).expect("parse config");

        assert!(config.feishu_integration.has_explicit_capability_config());
    }

    #[test]
    fn runtime_config_keeps_capabilities_implicit_when_block_is_absent() {
        let raw = r#"
            [feishu_integration]
            request_timeout_s = 20
        "#;

        let config: crate::config::LoongClawConfig = toml::from_str(raw).expect("parse config");

        assert!(!config.feishu_integration.has_explicit_capability_config());
    }

    #[test]
    fn serializing_implicit_capabilities_omits_capabilities_block() {
        let config = FeishuIntegrationConfig::default();

        let serialized = toml::to_string(&config).expect("serialize config");

        assert!(!serialized.contains("[capabilities]"));
        assert!(!serialized.contains("capabilities"));
    }

    #[test]
    fn feishu_integration_tracks_explicit_capability_configuration() {
        let default_config = FeishuIntegrationConfig::default();
        assert!(!default_config.has_explicit_capability_config());

        let custom_config = FeishuIntegrationConfig {
            capabilities_explicitly_configured: true,
            capabilities: FeishuCapabilityConfig {
                docs: true,
                messages: true,
                calendar: true,
                bitable: true,
            },
            ..FeishuIntegrationConfig::default()
        };

        assert!(custom_config.has_explicit_capability_config());
    }

    #[test]
    fn trimmed_default_scopes_normalizes_legacy_group_message_scope() {
        let config = FeishuIntegrationConfig {
            default_scopes: vec![
                "offline_access".to_owned(),
                "im:message.group_msg:readonly".to_owned(),
                "im:message.group_msg".to_owned(),
            ],
            ..FeishuIntegrationConfig::default()
        };

        assert_eq!(
            config.trimmed_default_scopes(),
            vec![
                "offline_access".to_owned(),
                "im:message.group_msg".to_owned()
            ]
        );
    }

    #[test]
    fn feishu_integration_validation_rejects_inverted_retry_backoff_window() {
        let config = FeishuIntegrationConfig {
            retry_initial_backoff_ms: 500,
            retry_max_backoff_ms: 100,
            ..FeishuIntegrationConfig::default()
        };

        let issues = config.validate();

        assert!(
            issues
                .iter()
                .any(|issue| issue.field_path == "feishu_integration.retry_max_backoff_ms"),
            "expected retry_max_backoff_ms validation issue, got {issues:?}"
        );
    }
}
