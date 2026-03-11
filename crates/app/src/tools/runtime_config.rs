use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalSkillsRuntimePolicy {
    pub enabled: bool,
    pub require_download_approval: bool,
    pub allowed_domains: BTreeSet<String>,
    pub blocked_domains: BTreeSet<String>,
}

impl Default for ExternalSkillsRuntimePolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            require_download_approval: true,
            allowed_domains: BTreeSet::new(),
            blocked_domains: BTreeSet::new(),
        }
    }
}

/// Typed runtime configuration for tool executors.
///
/// Replaces per-call `std::env::var` lookups with a single read from a
/// process-wide singleton that is populated once at startup.
#[derive(Debug, Clone, Default)]
pub struct ToolRuntimeConfig {
    pub shell_allowlist: BTreeSet<String>,
    pub file_root: Option<PathBuf>,
    pub external_skills: ExternalSkillsRuntimePolicy,
}

impl ToolRuntimeConfig {
    /// Build a config by reading the legacy environment variables.
    ///
    /// Keeps full backward compatibility for callers that still rely on
    /// `LOONGCLAW_SHELL_ALLOWLIST` / `LOONGCLAW_FILE_ROOT`.
    pub fn from_env() -> Self {
        let shell_allowlist = std::env::var("LOONGCLAW_SHELL_ALLOWLIST")
            .ok()
            .unwrap_or_else(|| "echo,cat,ls,pwd".to_owned())
            .split([',', ';', ' '])
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_ascii_lowercase)
            .collect();

        let file_root = std::env::var("LOONGCLAW_FILE_ROOT").ok().map(PathBuf::from);
        let enabled = parse_env_bool("LOONGCLAW_EXTERNAL_SKILLS_ENABLED").unwrap_or(false);
        let require_download_approval =
            parse_env_bool("LOONGCLAW_EXTERNAL_SKILLS_REQUIRE_DOWNLOAD_APPROVAL").unwrap_or(true);
        let allowed_domains = parse_env_domain_list("LOONGCLAW_EXTERNAL_SKILLS_ALLOWED_DOMAINS");
        let blocked_domains = parse_env_domain_list("LOONGCLAW_EXTERNAL_SKILLS_BLOCKED_DOMAINS");

        Self {
            shell_allowlist,
            file_root,
            external_skills: ExternalSkillsRuntimePolicy {
                enabled,
                require_download_approval,
                allowed_domains,
                blocked_domains,
            },
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
        assert!(config.shell_allowlist.is_empty());
        assert!(config.file_root.is_none());
        assert!(!config.external_skills.enabled);
        assert!(config.external_skills.require_download_approval);
        assert!(config.external_skills.allowed_domains.is_empty());
        assert!(config.external_skills.blocked_domains.is_empty());
    }

    #[test]
    fn shell_allowlist_uses_injected_config_not_env() {
        // Build a ToolRuntimeConfig with an explicit allowlist that differs
        // from any env var that might be set.
        let config = ToolRuntimeConfig {
            shell_allowlist: BTreeSet::from(["git".to_owned(), "cargo".to_owned()]),
            file_root: Some(PathBuf::from("/tmp/test-root")),
            external_skills: ExternalSkillsRuntimePolicy {
                enabled: true,
                require_download_approval: false,
                allowed_domains: BTreeSet::from(["skills.sh".to_owned()]),
                blocked_domains: BTreeSet::new(),
            },
        };
        assert!(config.shell_allowlist.contains("git"));
        assert!(config.shell_allowlist.contains("cargo"));
        assert!(!config.shell_allowlist.contains("echo"));
        assert_eq!(config.file_root, Some(PathBuf::from("/tmp/test-root")));
        assert!(config.external_skills.enabled);
        assert!(!config.external_skills.require_download_approval);
        assert!(config.external_skills.allowed_domains.contains("skills.sh"));
    }

    #[test]
    fn from_env_parses_default_allowlist() {
        // When the env var is not set, from_env falls back to the hardcoded
        // defaults: echo, cat, ls, pwd.
        let config = ToolRuntimeConfig::from_env();
        // We can't guarantee the env var is unset in all CI environments,
        // but the parser itself should produce a non-empty set either way.
        assert!(!config.shell_allowlist.is_empty());
    }

    #[cfg(feature = "tool-shell")]
    #[test]
    fn injected_config_overrides_global() {
        let config = ToolRuntimeConfig {
            shell_allowlist: BTreeSet::from(["echo".to_owned()]),
            file_root: Some(PathBuf::from("/tmp/injected-root")),
            external_skills: ExternalSkillsRuntimePolicy::default(),
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

        crate::process_env::remove_var("LOONGCLAW_EXTERNAL_SKILLS_ENABLED");
        crate::process_env::remove_var("LOONGCLAW_EXTERNAL_SKILLS_REQUIRE_DOWNLOAD_APPROVAL");
        crate::process_env::remove_var("LOONGCLAW_EXTERNAL_SKILLS_ALLOWED_DOMAINS");
        crate::process_env::remove_var("LOONGCLAW_EXTERNAL_SKILLS_BLOCKED_DOMAINS");
    }
}
