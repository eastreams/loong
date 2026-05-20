use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use loong_contracts::{SecretRef, SecretResolver};
use serde::{Deserialize, Serialize};

use crate::{CliResult, secrets::DefaultSecretResolver};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayConfig {
    #[serde(default = "default_gateway_port")]
    pub port: u16,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            port: default_gateway_port(),
        }
    }
}

impl GatewayConfig {
    pub(super) fn is_default(config: &Self) -> bool {
        *config == Self::default()
    }
}

fn default_gateway_port() -> u16 {
    26_306
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ControlPlaneConfig {
    #[serde(default)]
    pub allow_remote: bool,
    #[serde(default)]
    pub shared_token: Option<SecretRef>,
}

impl ControlPlaneConfig {
    pub(super) fn is_default(config: &Self) -> bool {
        *config == Self::default()
    }

    pub fn resolved_shared_token(&self) -> CliResult<Option<String>> {
        let Some(secret_ref) = self.shared_token.as_ref() else {
            return Ok(None);
        };
        let resolver = DefaultSecretResolver::default();
        let resolved_value = resolver
            .resolve(secret_ref)
            .map_err(|error| format!("resolve control-plane shared token failed: {error}"))?;
        let shared_token = resolved_value.map(|secret| secret.into_inner());
        Ok(shared_token)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AcpConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub dispatch: AcpDispatchConfig,
    #[serde(default)]
    pub default_agent: Option<String>,
    #[serde(default)]
    pub allowed_agents: Vec<String>,
    #[serde(default)]
    pub max_concurrent_sessions: Option<usize>,
    #[serde(default)]
    pub session_idle_ttl_ms: Option<u64>,
    #[serde(default)]
    pub startup_timeout_ms: Option<u64>,
    #[serde(default)]
    pub turn_timeout_ms: Option<u64>,
    #[serde(default)]
    pub queue_owner_ttl_ms: Option<u64>,
    #[serde(default)]
    pub bindings_enabled: bool,
    #[serde(default)]
    pub emit_runtime_events: bool,
    #[serde(default)]
    pub allow_mcp_server_injection: bool,
    #[serde(default)]
    pub backends: AcpBackendProfilesConfig,
}

impl AcpConfig {
    pub fn backend_id(&self) -> Option<String> {
        self.backend
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase())
    }

    pub fn dispatch_enabled(&self) -> bool {
        self.enabled && self.dispatch.enabled
    }

    pub fn max_concurrent_sessions(&self) -> usize {
        self.max_concurrent_sessions
            .filter(|value| *value > 0)
            .unwrap_or_else(default_acp_max_concurrent_sessions)
    }

    pub fn resolved_default_agent(&self) -> CliResult<String> {
        let raw = normalize_optional_string(self.default_agent.as_deref())
            .unwrap_or_else(|| "codex".to_owned());
        normalize_acp_agent_id(raw.as_str()).ok_or_else(|| {
            format!("ACP default agent `{raw}` is invalid; use letters, numbers, `-`, or `_`")
        })
    }

    pub fn allowed_agent_ids(&self) -> CliResult<Vec<String>> {
        let default_agent = self.resolved_default_agent()?;
        if self.allowed_agents.is_empty() {
            return Ok(vec![default_agent]);
        }

        let mut seen = BTreeSet::new();
        let mut agents = Vec::new();
        for raw in &self.allowed_agents {
            let trimmed = raw.trim();
            let normalized = normalize_acp_agent_id(trimmed).ok_or_else(|| {
                format!(
                    "ACP allowed agent `{trimmed}` is invalid; use letters, numbers, `-`, or `_`"
                )
            })?;
            if seen.insert(normalized.clone()) {
                agents.push(normalized);
            }
        }

        if !agents.iter().any(|agent| agent == &default_agent) {
            return Err(format!(
                "ACP default agent `{default_agent}` must be included in allowed_agents"
            ));
        }

        Ok(agents)
    }

    pub fn resolve_allowed_agent(&self, raw: &str) -> CliResult<String> {
        let normalized = normalize_acp_agent_id(raw).ok_or_else(|| {
            format!("ACP agent `{raw}` is invalid; use letters, numbers, `-`, or `_`")
        })?;
        let allowed = self.allowed_agent_ids()?;
        if allowed.iter().any(|agent| agent == &normalized) {
            return Ok(normalized);
        }
        Err(format!(
            "ACP agent `{normalized}` is not in the allowed ACP agents ({})",
            allowed.join(", ")
        ))
    }

    pub fn session_idle_ttl_ms(&self) -> u64 {
        self.session_idle_ttl_ms
            .filter(|value| *value > 0)
            .unwrap_or_else(default_acp_session_idle_ttl_ms)
    }

    pub fn startup_timeout_ms(&self) -> u64 {
        self.startup_timeout_ms
            .filter(|value| *value > 0)
            .unwrap_or_else(default_acp_startup_timeout_ms)
    }

    pub fn turn_timeout_ms(&self) -> u64 {
        self.turn_timeout_ms
            .filter(|value| *value > 0)
            .unwrap_or_else(default_acp_turn_timeout_ms)
    }

    pub fn queue_owner_ttl_ms(&self) -> u64 {
        self.queue_owner_ttl_ms
            .filter(|value| *value > 0)
            .unwrap_or_else(default_acp_queue_owner_ttl_ms)
    }

    pub fn acpx_profile(&self) -> Option<&AcpxBackendConfig> {
        self.backends.acpx.as_ref()
    }
}

impl Default for AcpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: None,
            dispatch: AcpDispatchConfig::default(),
            default_agent: None,
            allowed_agents: Vec::new(),
            max_concurrent_sessions: Some(default_acp_max_concurrent_sessions()),
            session_idle_ttl_ms: Some(default_acp_session_idle_ttl_ms()),
            startup_timeout_ms: Some(default_acp_startup_timeout_ms()),
            turn_timeout_ms: Some(default_acp_turn_timeout_ms()),
            queue_owner_ttl_ms: Some(default_acp_queue_owner_ttl_ms()),
            bindings_enabled: false,
            emit_runtime_events: false,
            allow_mcp_server_injection: false,
            backends: AcpBackendProfilesConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcpDispatchConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub conversation_routing: AcpConversationRoutingMode,
    #[serde(default)]
    pub allowed_channels: Vec<String>,
    #[serde(default)]
    pub allowed_account_ids: Vec<String>,
    #[serde(default)]
    pub bootstrap_mcp_servers: Vec<String>,
    #[serde(default)]
    pub working_directory: Option<String>,
    #[serde(default)]
    pub thread_routing: AcpDispatchThreadRoutingMode,
}

impl Default for AcpDispatchConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            conversation_routing: AcpConversationRoutingMode::default(),
            allowed_channels: Vec::new(),
            allowed_account_ids: Vec::new(),
            bootstrap_mcp_servers: Vec::new(),
            working_directory: None,
            thread_routing: AcpDispatchThreadRoutingMode::default(),
        }
    }
}

impl AcpDispatchConfig {
    pub fn allowed_channel_ids(&self) -> CliResult<Vec<String>> {
        let mut seen = BTreeSet::new();
        let mut channels = Vec::new();
        for raw in &self.allowed_channels {
            let trimmed = raw.trim();
            let normalized = normalize_dispatch_channel_id(trimmed).ok_or_else(|| {
                format!(
                    "ACP dispatch allowed channel `{trimmed}` is invalid; use letters, numbers, `-`, or `_`"
                )
            })?;
            if seen.insert(normalized.clone()) {
                channels.push(normalized);
            }
        }
        Ok(channels)
    }

    pub fn allows_channel_id(&self, channel_id: Option<&str>) -> CliResult<bool> {
        let allowed = self.allowed_channel_ids()?;
        if allowed.is_empty() {
            return Ok(true);
        }
        let Some(channel_id) = channel_id.and_then(normalize_dispatch_channel_id) else {
            return Ok(false);
        };
        Ok(allowed.iter().any(|channel| channel == &channel_id))
    }

    pub fn allowed_account_ids(&self) -> CliResult<Vec<String>> {
        let mut seen = BTreeSet::new();
        let mut accounts = Vec::new();
        for raw in &self.allowed_account_ids {
            let trimmed = raw.trim();
            let normalized = normalize_dispatch_account_id(trimmed).ok_or_else(|| {
                format!(
                    "ACP dispatch allowed account `{trimmed}` is invalid; use a configured account identity or label"
                )
            })?;
            if seen.insert(normalized.clone()) {
                accounts.push(normalized);
            }
        }
        Ok(accounts)
    }

    pub fn bootstrap_mcp_server_names(&self) -> CliResult<Vec<String>> {
        self.bootstrap_mcp_server_names_with_additions(&[])
    }

    pub fn resolved_working_directory(&self) -> Option<PathBuf> {
        self.working_directory
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(Path::new)
            .map(Path::to_path_buf)
    }

    pub fn bootstrap_mcp_server_names_with_additions(
        &self,
        additional: &[String],
    ) -> CliResult<Vec<String>> {
        let mut seen = BTreeSet::new();
        let mut names = Vec::new();
        for raw in self
            .bootstrap_mcp_servers
            .iter()
            .map(String::as_str)
            .chain(additional.iter().map(String::as_str))
        {
            let Some(normalized) =
                normalize_optional_string(Some(raw)).map(|value| value.to_ascii_lowercase())
            else {
                return Err(
                    "ACP dispatch bootstrap MCP server names must not contain empty entries"
                        .to_owned(),
                );
            };
            if seen.insert(normalized.clone()) {
                names.push(normalized);
            }
        }
        Ok(names)
    }

    pub fn allows_account_id(&self, account_id: Option<&str>) -> CliResult<bool> {
        let allowed = self.allowed_account_ids()?;
        if allowed.is_empty() {
            return Ok(true);
        }
        let Some(account_id) = account_id.and_then(normalize_dispatch_account_id) else {
            return Ok(false);
        };
        Ok(allowed.iter().any(|candidate| candidate == &account_id))
    }

    pub fn allows_thread_id(&self, thread_id: Option<&str>) -> bool {
        self.thread_routing.allows_thread_id(thread_id)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AcpConversationRoutingMode {
    #[default]
    AgentPrefixedOnly,
    All,
}

impl AcpConversationRoutingMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::AgentPrefixedOnly => "agent_prefixed_only",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AcpDispatchThreadRoutingMode {
    #[default]
    All,
    ThreadOnly,
    RootOnly,
}

impl AcpDispatchThreadRoutingMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::ThreadOnly => "thread_only",
            Self::RootOnly => "root_only",
        }
    }

    pub fn allows_thread_id(self, thread_id: Option<&str>) -> bool {
        let has_thread = thread_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some();
        match self {
            Self::All => true,
            Self::ThreadOnly => has_thread,
            Self::RootOnly => !has_thread,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct AcpBackendProfilesConfig {
    #[serde(default)]
    pub acpx: Option<AcpxBackendConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AcpxBackendConfig {
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub expected_version: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub non_interactive_permissions: Option<String>,
    #[serde(default)]
    pub strict_windows_cmd_wrapper: Option<bool>,
    #[serde(default)]
    pub timeout_seconds: Option<f64>,
    #[serde(default)]
    pub queue_owner_ttl_seconds: Option<f64>,
    #[serde(default)]
    pub mcp_servers: BTreeMap<String, AcpxMcpServerConfig>,
}

impl AcpxBackendConfig {
    pub fn command(&self) -> Option<String> {
        normalize_optional_string(self.command.as_deref())
    }

    pub fn expected_version(&self) -> Option<String> {
        normalize_optional_string(self.expected_version.as_deref())
    }

    pub fn cwd(&self) -> Option<String> {
        normalize_optional_string(self.cwd.as_deref())
    }

    pub fn permission_mode(&self) -> Option<String> {
        normalize_optional_string(self.permission_mode.as_deref())
    }

    pub fn non_interactive_permissions(&self) -> Option<String> {
        normalize_optional_string(self.non_interactive_permissions.as_deref())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcpxMcpServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

const fn default_acp_max_concurrent_sessions() -> usize {
    8
}

const fn default_true() -> bool {
    true
}

const fn default_acp_session_idle_ttl_ms() -> u64 {
    900_000
}

const fn default_acp_startup_timeout_ms() -> u64 {
    15_000
}

const fn default_acp_turn_timeout_ms() -> u64 {
    120_000
}

const fn default_acp_queue_owner_ttl_ms() -> u64 {
    30_000
}

fn normalize_optional_string(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_owned())
}

fn normalize_acp_agent_id(raw: &str) -> Option<String> {
    let normalized = raw.trim().to_ascii_lowercase();
    let mut chars = normalized.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphanumeric() {
        return None;
    }
    if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') {
        return None;
    }
    Some(normalized)
}

pub(crate) fn normalize_dispatch_channel_id(raw: &str) -> Option<String> {
    let normalized = raw.trim().to_ascii_lowercase();
    let mut chars = normalized.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphanumeric() {
        return None;
    }
    if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') {
        return None;
    }
    Some(normalized)
}

pub(crate) fn normalize_dispatch_account_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut normalized = String::with_capacity(trimmed.len());
    let mut last_was_separator = false;
    let mut saw_alphanumeric = false;
    for value in trimmed.chars() {
        if value.is_ascii_alphanumeric() {
            normalized.push(value.to_ascii_lowercase());
            last_was_separator = false;
            saw_alphanumeric = true;
            continue;
        }
        if matches!(value, '_' | '-') {
            if !normalized.is_empty() && !last_was_separator {
                normalized.push(value);
                last_was_separator = true;
            }
            continue;
        }
        if !normalized.is_empty() && !last_was_separator {
            normalized.push('-');
            last_was_separator = true;
        }
    }

    while matches!(normalized.chars().last(), Some('-' | '_')) {
        normalized.pop();
    }

    if !saw_alphanumeric || normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}
