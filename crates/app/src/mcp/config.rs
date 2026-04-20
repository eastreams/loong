use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: BTreeMap<String, McpServerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServerConfig {
    #[serde(flatten)]
    pub transport: McpServerTransportConfig,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub startup_timeout_ms: Option<u64>,
    #[serde(default)]
    pub tool_timeout_ms: Option<u64>,
    #[serde(default)]
    pub enabled_tools: Vec<String>,
    #[serde(default)]
    pub disabled_tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged, deny_unknown_fields, rename_all = "snake_case")]
pub enum McpServerTransportConfig {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
        #[serde(default)]
        cwd: Option<PathBuf>,
    },
    StreamableHttp {
        url: String,
        #[serde(default)]
        bearer_token_env_var: Option<String>,
        #[serde(default)]
        http_headers: BTreeMap<String, String>,
        #[serde(default)]
        env_http_headers: BTreeMap<String, String>,
    },
}

const fn default_enabled() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_config_defaults_to_empty_registry() {
        let config = McpConfig::default();
        assert!(config.servers.is_empty());
    }

    #[test]
    fn deserialize_stdio_mcp_server_config() {
        let raw = r#"
        {
          "command": "uvx",
          "args": ["context7-mcp"],
          "env": {"API_TOKEN": "secret"},
          "cwd": "/workspace/repo",
          "enabled": true,
          "required": false,
          "startup_timeout_ms": 15000,
          "tool_timeout_ms": 120000
        }
        "#;
        let parsed: McpServerConfig = serde_json::from_str(raw).expect("parse stdio MCP config");

        assert_eq!(
            parsed.transport,
            McpServerTransportConfig::Stdio {
                command: "uvx".to_owned(),
                args: vec!["context7-mcp".to_owned()],
                env: BTreeMap::from([("API_TOKEN".to_owned(), "secret".to_owned())]),
                cwd: Some(PathBuf::from("/workspace/repo")),
            }
        );
        assert!(parsed.enabled);
        assert!(!parsed.required);
        assert_eq!(parsed.startup_timeout_ms, Some(15_000));
        assert_eq!(parsed.tool_timeout_ms, Some(120_000));
    }

    #[test]
    fn deserialize_streamable_http_mcp_server_config() {
        let raw = r#"
        {
          "url": "https://mcp.example.com",
          "bearer_token_env_var": "MCP_TOKEN",
          "http_headers": {"X-Test": "ok"},
          "env_http_headers": {"Authorization": "MCP_AUTH_HEADER"},
          "enabled_tools": ["search"],
          "disabled_tools": ["write"]
        }
        "#;
        let parsed: McpServerConfig =
            serde_json::from_str(raw).expect("parse streamable HTTP MCP config");

        assert_eq!(
            parsed.transport,
            McpServerTransportConfig::StreamableHttp {
                url: "https://mcp.example.com".to_owned(),
                bearer_token_env_var: Some("MCP_TOKEN".to_owned()),
                http_headers: BTreeMap::from([("X-Test".to_owned(), "ok".to_owned())]),
                env_http_headers: BTreeMap::from([(
                    "Authorization".to_owned(),
                    "MCP_AUTH_HEADER".to_owned(),
                )]),
            }
        );
        assert_eq!(parsed.enabled_tools, vec!["search".to_owned()]);
        assert_eq!(parsed.disabled_tools, vec!["write".to_owned()]);
    }
}
