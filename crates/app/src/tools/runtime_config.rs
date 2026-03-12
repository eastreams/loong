use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::OnceLock;

use super::policy_ext::ShellPolicyDefault;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalSkillsRuntimePolicy {
    pub enabled: bool,
    pub require_download_approval: bool,
    pub allowed_domains: BTreeSet<String>,
    pub blocked_domains: BTreeSet<String>,
    pub install_root: Option<PathBuf>,
    pub auto_expose_installed: bool,
}

impl Default for ExternalSkillsRuntimePolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            require_download_approval: true,
            allowed_domains: BTreeSet::new(),
            blocked_domains: BTreeSet::new(),
            install_root: None,
            auto_expose_installed: true,
        }
    }
}

/// Typed runtime configuration for tool executors.
///
/// Replaces per-call `std::env::var` lookups with a single read from a
/// process-wide singleton that is populated once at startup.
#[derive(Debug, Clone)]
pub struct ToolRuntimeConfig {
    pub file_root: Option<PathBuf>,
    pub shell_allow: BTreeSet<String>,
    pub shell_deny: BTreeSet<String>,
    pub shell_approval_required: BTreeSet<String>,
    pub shell_default_mode: ShellPolicyDefault,
    pub external_skills: ExternalSkillsRuntimePolicy,
}

impl Default for ToolRuntimeConfig {
    fn default() -> Self {
        Self {
            file_root: None,
            shell_allow: crate::config::DEFAULT_SHELL_ALLOW
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
            shell_deny: BTreeSet::new(),
            shell_approval_required: BTreeSet::new(),
            shell_default_mode: ShellPolicyDefault::Deny,
            external_skills: ExternalSkillsRuntimePolicy::default(),
        }
    }
}

impl ToolRuntimeConfig {
    /// Build a config by reading the legacy environment variables.
    ///
    /// Keeps full backward compatibility for callers that still rely on
    /// `LOONGCLAW_FILE_ROOT`.
    pub fn from_env() -> Self {
        let file_root = std::env::var("LOONGCLAW_FILE_ROOT").ok().map(PathBuf::from);
        let enabled = parse_env_bool("LOONGCLAW_EXTERNAL_SKILLS_ENABLED").unwrap_or(false);
        let require_download_approval =
            parse_env_bool("LOONGCLAW_EXTERNAL_SKILLS_REQUIRE_DOWNLOAD_APPROVAL").unwrap_or(true);
        let allowed_domains = parse_env_domain_list("LOONGCLAW_EXTERNAL_SKILLS_ALLOWED_DOMAINS");
        let blocked_domains = parse_env_domain_list("LOONGCLAW_EXTERNAL_SKILLS_BLOCKED_DOMAINS");
        let install_root = std::env::var("LOONGCLAW_EXTERNAL_SKILLS_INSTALL_ROOT")
            .ok()
            .map(PathBuf::from);
        let auto_expose_installed =
            parse_env_bool("LOONGCLAW_EXTERNAL_SKILLS_AUTO_EXPOSE_INSTALLED").unwrap_or(true);

        Self {
            file_root,
            external_skills: ExternalSkillsRuntimePolicy {
                enabled,
                require_download_approval,
                allowed_domains,
                blocked_domains,
                install_root,
                auto_expose_installed,
            },
            ..Self::default()
        }
    }
}

fn parse_env_bool(key: &str) -> Option<bool> {
    std::env::var(key).ok().and_then(|raw| {
        let value = raw.trim().to_ascii_lowercase();
        match value.as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        }
    })
}

fn parse_env_domain_list(key: &str) -> BTreeSet<String> {
    std::env::var(key)
        .ok()
        .unwrap_or_default()
        .split([',', ';', ' '])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

static TOOL_RUNTIME_CONFIG: OnceLock<ToolRuntimeConfig> = OnceLock::new();

/// Initialise the process-wide tool runtime config.
///
/// Returns `Ok(())` on the first call.  Subsequent calls return
/// `Err` because the `OnceLock` rejects duplicate initialisation.
pub fn init_tool_runtime_config(config: ToolRuntimeConfig) -> Result<(), String> {
    TOOL_RUNTIME_CONFIG.set(config).map_err(|_err| {
        "tool runtime config already initialised (duplicate init_tool_runtime_config call)"
            .to_owned()
    })
}

/// Return the process-wide tool runtime config.
///
/// If `init_tool_runtime_config` was never called the config is lazily
/// populated from environment variables (backward-compat path).
pub fn get_tool_runtime_config() -> &'static ToolRuntimeConfig {
    TOOL_RUNTIME_CONFIG.get_or_init(ToolRuntimeConfig::from_env)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_runtime_config_from_env_defaults() {
        let config = ToolRuntimeConfig::default();
        assert!(config.file_root.is_none());
        assert!(!config.external_skills.enabled);
        assert!(config.external_skills.require_download_approval);
        assert!(config.external_skills.allowed_domains.is_empty());
        assert!(config.external_skills.blocked_domains.is_empty());
        assert!(config.external_skills.install_root.is_none());
        assert!(config.external_skills.auto_expose_installed);
    }

    /// The default allow list must mirror the serde default in `ToolConfig` so
    /// that the runtime fallback path and a freshly-parsed config file agree.
    #[test]
    fn default_shell_allow_contains_four_initial_commands() {
        let config = ToolRuntimeConfig::default();
        let mut allow: Vec<_> = config.shell_allow.iter().cloned().collect();
        allow.sort();
        assert_eq!(allow, vec!["cargo", "echo", "git", "ls"]);
    }

    /// Deny and approval-required start empty so users are not forced to carry
    /// any hardcoded restriction they did not opt into.
    #[test]
    fn default_deny_and_approval_are_empty() {
        let config = ToolRuntimeConfig::default();
        assert!(config.shell_deny.is_empty());
        assert!(config.shell_approval_required.is_empty());
    }

    /// Explicit config injection overrides defaults — verifies that
    /// non-default values survive construction without env-var leakage.
    #[test]
    fn explicit_config_injection_overrides_defaults() {
        let config = ToolRuntimeConfig {
            shell_allow: BTreeSet::from(["git".to_owned(), "cargo".to_owned()]),
            file_root: Some(PathBuf::from("/tmp/test-root")),
            external_skills: ExternalSkillsRuntimePolicy {
                enabled: true,
                require_download_approval: false,
                allowed_domains: BTreeSet::from(["skills.sh".to_owned()]),
                blocked_domains: BTreeSet::new(),
                install_root: Some(PathBuf::from("/tmp/test-root/skills")),
                auto_expose_installed: false,
            },
            ..ToolRuntimeConfig::default()
        };
        assert!(config.shell_allow.contains("git"));
        assert!(config.shell_allow.contains("cargo"));
        assert!(!config.shell_allow.contains("echo"));
        assert_eq!(config.file_root, Some(PathBuf::from("/tmp/test-root")));
        assert!(config.external_skills.enabled);
        assert!(!config.external_skills.require_download_approval);
        assert!(config.external_skills.allowed_domains.contains("skills.sh"));
        assert_eq!(
            config.external_skills.install_root,
            Some(PathBuf::from("/tmp/test-root/skills"))
        );
        assert!(!config.external_skills.auto_expose_installed);
    }

    #[test]
    fn file_root_uses_injected_config() {
        let config = ToolRuntimeConfig {
            file_root: Some(PathBuf::from("/tmp/test-root")),
            ..ToolRuntimeConfig::default()
        };
        assert_eq!(config.file_root, Some(PathBuf::from("/tmp/test-root")));
    }

    #[cfg(feature = "tool-shell")]
    #[test]
    fn injected_config_overrides_global() {
        let config = ToolRuntimeConfig {
            file_root: Some(PathBuf::from("/tmp/injected-root")),
            ..ToolRuntimeConfig::default()
        };
        let result = crate::tools::execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "shell.exec".to_owned(),
                payload: serde_json::json!({"command": "echo", "args": ["injected"]}),
            },
            &config,
        );
        let outcome = result.expect("echo should be allowed with injected config");
        assert_eq!(outcome.status, "ok");
        assert!(
            outcome.payload["stdout"]
                .as_str()
                .unwrap()
                .contains("injected")
        );
    }

    #[test]
    fn from_env_parses_external_skills_policy() {
        crate::process_env::set_var("LOONGCLAW_EXTERNAL_SKILLS_ENABLED", "true");
        crate::process_env::set_var(
            "LOONGCLAW_EXTERNAL_SKILLS_REQUIRE_DOWNLOAD_APPROVAL",
            "false",
        );
        crate::process_env::set_var(
            "LOONGCLAW_EXTERNAL_SKILLS_ALLOWED_DOMAINS",
            "skills.sh,clawhub.io",
        );
        crate::process_env::set_var(
            "LOONGCLAW_EXTERNAL_SKILLS_BLOCKED_DOMAINS",
            "malicious.example",
        );
        crate::process_env::set_var(
            "LOONGCLAW_EXTERNAL_SKILLS_INSTALL_ROOT",
            "/tmp/managed-skills",
        );
        crate::process_env::set_var("LOONGCLAW_EXTERNAL_SKILLS_AUTO_EXPOSE_INSTALLED", "false");

        let config = ToolRuntimeConfig::from_env();
        assert!(config.external_skills.enabled);
        assert!(!config.external_skills.require_download_approval);
        assert!(config.external_skills.allowed_domains.contains("skills.sh"));
        assert!(
            config
                .external_skills
                .allowed_domains
                .contains("clawhub.io")
        );
        assert!(
            config
                .external_skills
                .blocked_domains
                .contains("malicious.example")
        );
        assert_eq!(
            config.external_skills.install_root,
            Some(PathBuf::from("/tmp/managed-skills"))
        );
        assert!(!config.external_skills.auto_expose_installed);

        crate::process_env::remove_var("LOONGCLAW_EXTERNAL_SKILLS_ENABLED");
        crate::process_env::remove_var("LOONGCLAW_EXTERNAL_SKILLS_REQUIRE_DOWNLOAD_APPROVAL");
        crate::process_env::remove_var("LOONGCLAW_EXTERNAL_SKILLS_ALLOWED_DOMAINS");
        crate::process_env::remove_var("LOONGCLAW_EXTERNAL_SKILLS_BLOCKED_DOMAINS");
        crate::process_env::remove_var("LOONGCLAW_EXTERNAL_SKILLS_INSTALL_ROOT");
        crate::process_env::remove_var("LOONGCLAW_EXTERNAL_SKILLS_AUTO_EXPOSE_INSTALLED");
    }
}
