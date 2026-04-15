use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::shared::{default_loongclaw_home, expand_path};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditConfig {
    #[serde(default)]
    pub mode: AuditMode,
    #[serde(default = "default_audit_path")]
    pub path: String,
    #[serde(default = "default_true")]
    pub retain_in_memory: bool,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            mode: AuditMode::default(),
            path: default_audit_path(),
            retain_in_memory: default_true(),
        }
    }
}

impl AuditConfig {
    pub fn resolved_path(&self) -> PathBuf {
        let trimmed = self.path.trim();
        if trimmed.is_empty() {
            return expand_path(&default_audit_path());
        }
        expand_path(trimmed)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuditMode {
    InMemory,
    Jsonl,
    #[default]
    Fanout,
}

impl AuditMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InMemory => "in_memory",
            Self::Jsonl => "jsonl",
            Self::Fanout => "fanout",
        }
    }
}

fn default_audit_path() -> String {
    default_loongclaw_home()
        .join("audit")
        .join("events.jsonl")
        .display()
        .to_string()
}

const fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScopedLoongClawHome;

    #[test]
    fn audit_config_defaults_to_fanout_under_loongclaw_home() {
        let _home = ScopedLoongClawHome::new("loongclaw-audit-config-home");
        let config = AuditConfig::default();

        assert_eq!(config.mode, AuditMode::Fanout);
        assert!(config.retain_in_memory);
        assert_eq!(
            PathBuf::from(&config.path),
            default_loongclaw_home().join("audit").join("events.jsonl")
        );
    }

    #[test]
    fn audit_mode_ids_are_stable() {
        assert_eq!(AuditMode::InMemory.as_str(), "in_memory");
        assert_eq!(AuditMode::Jsonl.as_str(), "jsonl");
        assert_eq!(AuditMode::Fanout.as_str(), "fanout");
    }

    #[test]
    fn audit_config_empty_path_falls_back_to_default_location() {
        let _home = ScopedLoongClawHome::new("loongclaw-audit-path-home");
        let config = AuditConfig {
            path: "   ".to_owned(),
            ..AuditConfig::default()
        };

        assert_eq!(
            config.resolved_path(),
            default_loongclaw_home().join("audit").join("events.jsonl")
        );
    }
}
