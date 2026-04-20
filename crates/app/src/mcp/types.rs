use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpServerOriginKind {
    Config,
    Plugin,
    Managed,
    AcpBackendProfile,
    AcpBootstrapSelection,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServerOrigin {
    pub kind: McpServerOriginKind,
    pub source_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum McpAuthStatus {
    #[default]
    Unknown,
    Unsupported,
    NotLoggedIn,
    BearerToken,
    OAuth,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpServerStatusKind {
    Pending,
    Connected,
    NeedsAuth,
    Failed,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServerStatus {
    pub kind: McpServerStatusKind,
    pub auth: McpAuthStatus,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "transport", rename_all = "snake_case")]
pub enum McpTransportSnapshot {
    Stdio {
        command: String,
        args: Vec<String>,
        cwd: Option<String>,
        env_var_names: Vec<String>,
    },
    StreamableHttp {
        url: String,
        bearer_token_env_var: Option<String>,
        http_header_names: Vec<String>,
        env_http_header_names: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpRuntimeServerSnapshot {
    pub name: String,
    pub enabled: bool,
    pub required: bool,
    pub selected_for_acp_bootstrap: bool,
    pub origins: Vec<McpServerOrigin>,
    pub status: McpServerStatus,
    pub transport: McpTransportSnapshot,
    pub enabled_tools: Vec<String>,
    pub disabled_tools: Vec<String>,
    pub startup_timeout_ms: Option<u64>,
    pub tool_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpStdioServerLaunchSpec {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: Option<String>,
    pub startup_timeout_ms: Option<u64>,
    pub tool_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct McpRuntimeSnapshot {
    pub servers: Vec<McpRuntimeServerSnapshot>,
    pub missing_selected_servers: Vec<String>,
}
