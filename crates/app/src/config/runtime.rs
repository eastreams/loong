use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::CliResult;

use super::{
    channels::{CliChannelConfig, FeishuChannelConfig, TelegramChannelConfig},
    provider::ProviderConfig,
    shared::{
        default_loongclaw_home as shared_default_loongclaw_home, expand_path, DEFAULT_CONFIG_FILE,
    },
    tools_memory::{MemoryConfig, ToolConfig},
};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LoongClawConfig {
    #[serde(default)]
    pub provider: ProviderConfig,
    #[serde(default)]
    pub cli: CliChannelConfig,
    #[serde(default)]
    pub telegram: TelegramChannelConfig,
    #[serde(default)]
    pub feishu: FeishuChannelConfig,
    #[serde(default)]
    pub tools: ToolConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub conversation: ConversationConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConversationConfig {
    #[serde(default)]
    pub turn_loop: ConversationTurnLoopConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationTurnLoopConfig {
    #[serde(default = "default_turn_loop_max_rounds")]
    pub max_rounds: usize,
    #[serde(default = "default_turn_loop_max_tool_steps_per_round")]
    pub max_tool_steps_per_round: usize,
    #[serde(default = "default_turn_loop_max_repeated_tool_call_rounds")]
    pub max_repeated_tool_call_rounds: usize,
    #[serde(default = "default_turn_loop_max_ping_pong_cycles")]
    pub max_ping_pong_cycles: usize,
    #[serde(default = "default_turn_loop_max_same_tool_failure_rounds")]
    pub max_same_tool_failure_rounds: usize,
    #[serde(default = "default_turn_loop_max_followup_tool_payload_chars")]
    pub max_followup_tool_payload_chars: usize,
}

impl Default for ConversationTurnLoopConfig {
    fn default() -> Self {
        Self {
            max_rounds: default_turn_loop_max_rounds(),
            max_tool_steps_per_round: default_turn_loop_max_tool_steps_per_round(),
            max_repeated_tool_call_rounds: default_turn_loop_max_repeated_tool_call_rounds(),
            max_ping_pong_cycles: default_turn_loop_max_ping_pong_cycles(),
            max_same_tool_failure_rounds: default_turn_loop_max_same_tool_failure_rounds(),
            max_followup_tool_payload_chars: default_turn_loop_max_followup_tool_payload_chars(),
        }
    }
}

pub fn load(path: Option<&str>) -> CliResult<(PathBuf, LoongClawConfig)> {
    let config_path = path.map(expand_path).unwrap_or_else(default_config_path);
    let raw = fs::read_to_string(&config_path).map_err(|error| {
        format!(
            "failed to read config {}: {error}. run `loongclawd setup` first",
            config_path.display()
        )
    })?;
    parse_toml_config(&raw).map(|config| (config_path, config))
}

pub fn write_template(path: Option<&str>, force: bool) -> CliResult<PathBuf> {
    let output_path = path.map(expand_path).unwrap_or_else(default_config_path);
    if output_path.exists() && !force {
        return Err(format!(
            "config {} already exists (use --force to overwrite)",
            output_path.display()
        ));
    }

    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create config directory: {error}"))?;
        }
    }

    let encoded = encode_toml_config(&LoongClawConfig::default())?;
    fs::write(&output_path, encoded).map_err(|error| {
        format!(
            "failed to write config file {}: {error}",
            output_path.display()
        )
    })?;
    Ok(output_path)
}

pub fn default_config_path() -> PathBuf {
    default_loongclaw_home().join(DEFAULT_CONFIG_FILE)
}

pub fn default_loongclaw_home() -> PathBuf {
    shared_default_loongclaw_home()
}

#[cfg(feature = "config-toml")]
fn parse_toml_config(raw: &str) -> CliResult<LoongClawConfig> {
    toml::from_str(raw).map_err(|error| format!("failed to parse TOML config: {error}"))
}

#[cfg(not(feature = "config-toml"))]
fn parse_toml_config(_raw: &str) -> CliResult<LoongClawConfig> {
    Err("config-toml feature is disabled for this build".to_owned())
}

#[cfg(feature = "config-toml")]
fn encode_toml_config(config: &LoongClawConfig) -> CliResult<String> {
    toml::to_string_pretty(config).map_err(|error| format!("failed to encode TOML config: {error}"))
}

#[cfg(not(feature = "config-toml"))]
fn encode_toml_config(_config: &LoongClawConfig) -> CliResult<String> {
    Err("config-toml feature is disabled for this build".to_owned())
}

const fn default_turn_loop_max_rounds() -> usize {
    4
}

const fn default_turn_loop_max_tool_steps_per_round() -> usize {
    1
}

const fn default_turn_loop_max_repeated_tool_call_rounds() -> usize {
    2
}

const fn default_turn_loop_max_ping_pong_cycles() -> usize {
    2
}

const fn default_turn_loop_max_same_tool_failure_rounds() -> usize {
    3
}

const fn default_turn_loop_max_followup_tool_payload_chars() -> usize {
    8_000
}
