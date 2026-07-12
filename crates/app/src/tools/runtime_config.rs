use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::path::PathBuf;
use std::sync::OnceLock;

#[cfg(feature = "tool-shell")]
use super::bash;
use super::shell_policy_ext::ShellPolicyDefault;

use crate::config::{AutonomyProfile, LoongConfig};
#[cfg(feature = "feishu-integration")]
use crate::config::{FeishuChannelConfig, FeishuIntegrationConfig};
use crate::conversation::{
    ConstrainedSubagentContractView, ConstrainedSubagentControlScope, ConstrainedSubagentMode,
    ConstrainedSubagentRole, ConstrainedSubagentRuntimeBinding,
};
#[cfg(feature = "feishu-integration")]
use crate::secrets::has_configured_secret_ref;
use loong_contracts::{ExecutionSecurityTier, SecretRef};
use serde::{Deserialize, Serialize};

#[path = "runtime_config_narrowing.rs"]
mod runtime_narrowing;
#[path = "runtime_config_web_search.rs"]
mod web_search_runtime;
pub(crate) use runtime_narrowing::merge_runtime_narrowing_sources;
pub use runtime_narrowing::{
    BrowserRuntimeNarrowing, ToolRuntimeNarrowing, WebFetchRuntimeNarrowing,
};
pub use web_search_runtime::WebSearchRuntimePolicy;

pub const SKILLS_ENABLED_ENV: &str = "LOONG_SKILLS_ENABLED";
pub const SKILLS_REQUIRE_DOWNLOAD_APPROVAL_ENV: &str = "LOONG_SKILLS_REQUIRE_DOWNLOAD_APPROVAL";
pub const SKILLS_ALLOWED_DOMAINS_ENV: &str = "LOONG_SKILLS_ALLOWED_DOMAINS";
pub const SKILLS_BLOCKED_DOMAINS_ENV: &str = "LOONG_SKILLS_BLOCKED_DOMAINS";
pub const SKILLS_INSTALL_ROOT_ENV: &str = "LOONG_SKILLS_INSTALL_ROOT";
pub const SKILLS_AUTO_EXPOSE_INSTALLED_ENV: &str = "LOONG_SKILLS_AUTO_EXPOSE_INSTALLED";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillsRuntimePolicy {
    pub enabled: bool,
    pub require_download_approval: bool,
    pub allowed_domains: BTreeSet<String>,
    pub blocked_domains: BTreeSet<String>,
    pub install_root: Option<PathBuf>,
    pub auto_expose_installed: bool,
}

impl Default for SkillsRuntimePolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            require_download_approval: false,
            allowed_domains: BTreeSet::new(),
            blocked_domains: BTreeSet::new(),
            install_root: None,
            auto_expose_installed: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserRuntimePolicy {
    pub enabled: bool,
    pub max_sessions: usize,
    pub max_links: usize,
    pub max_text_chars: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FsRuntimePolicy {
    pub deny_read_filenames: BTreeSet<String>,
}

impl Default for BrowserRuntimePolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            max_sessions: crate::config::DEFAULT_BROWSER_MAX_SESSIONS,
            max_links: crate::config::DEFAULT_BROWSER_MAX_LINKS,
            max_text_chars: crate::config::DEFAULT_BROWSER_MAX_TEXT_CHARS,
        }
    }
}

impl BrowserRuntimePolicy {
    #[must_use]
    pub const fn execution_security_tier(&self) -> ExecutionSecurityTier {
        let _ = self;
        ExecutionSecurityTier::Restricted
    }
}

#[cfg(feature = "tool-shell")]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BashGovernanceRuntimePolicy {
    pub rules_dir: PathBuf,
    pub rules: Vec<bash::rules::CompiledPrefixRule>,
    pub load_error: Option<String>,
}

#[cfg(feature = "tool-shell")]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BashExecRuntimePolicy {
    pub available: bool,
    pub command: Option<PathBuf>,
    pub warning: Option<String>,
    pub login_shell: bool,
    pub governance: BashGovernanceRuntimePolicy,
}

#[cfg(feature = "tool-shell")]
impl BashExecRuntimePolicy {
    #[must_use]
    pub fn is_runtime_ready(&self) -> bool {
        self.available && self.command.is_some()
    }

    #[must_use]
    pub fn is_discoverable(&self) -> bool {
        self.is_runtime_ready() && self.governance.load_error.is_none()
    }
}

#[allow(clippy::print_stderr)]
fn emit_runtime_warning(warning: &str) {
    eprintln!("warning: {warning}");
}

#[cfg(feature = "tool-shell")]
fn cached_bash_exec_runtime_probe() -> BashExecRuntimePolicy {
    static BASH_RUNTIME_PROBE: OnceLock<BashExecRuntimePolicy> = OnceLock::new();

    BASH_RUNTIME_PROBE
        .get_or_init(super::bash::detect_bash_runtime_policy)
        .clone()
}

#[cfg(feature = "tool-shell")]
fn emit_bash_runtime_warning_once(warning: &str) {
    static BASH_RUNTIME_WARNING: OnceLock<()> = OnceLock::new();

    BASH_RUNTIME_WARNING.get_or_init(|| emit_runtime_warning(warning));
}

#[cfg(feature = "tool-shell")]
fn translate_legacy_shell_rules<'a>(
    source: &str,
    decision: bash::rules::PrefixRuleDecision,
    commands: impl IntoIterator<Item = &'a String>,
) -> Vec<bash::rules::CompiledPrefixRule> {
    commands
        .into_iter()
        .filter_map(|command| {
            let normalized = command.trim().to_ascii_lowercase();
            if normalized.is_empty() {
                return None;
            }

            Some(bash::rules::CompiledPrefixRule {
                source: format!("{source}:{normalized}"),
                prefix: vec![normalized],
                decision,
                origin: bash::rules::CompiledRuleOrigin::LegacyShellCompatibility,
            })
        })
        .collect()
}

#[cfg(feature = "tool-shell")]
fn build_bash_governance_runtime_policy<'a>(
    rules_dir: PathBuf,
    shell_allow: impl IntoIterator<Item = &'a String>,
    shell_deny: impl IntoIterator<Item = &'a String>,
) -> BashGovernanceRuntimePolicy {
    let mut rules = translate_legacy_shell_rules(
        "shell_allow",
        bash::rules::PrefixRuleDecision::Allow,
        shell_allow,
    );
    rules.extend(translate_legacy_shell_rules(
        "shell_deny",
        bash::rules::PrefixRuleDecision::Deny,
        shell_deny,
    ));

    let load_error = match bash::rules::load_rules_from_dir(&rules_dir) {
        Ok(loaded_rules) => {
            rules.extend(loaded_rules);
            None
        }
        Err(error) => Some(error),
    };

    BashGovernanceRuntimePolicy {
        rules_dir,
        rules,
        load_error,
    }
}

#[cfg(feature = "tool-shell")]
fn build_bash_exec_runtime_policy(
    login_shell: bool,
    governance: BashGovernanceRuntimePolicy,
) -> BashExecRuntimePolicy {
    #[cfg(feature = "tool-shell")]
    {
        let mut policy = cached_bash_exec_runtime_probe();
        if let Some(warning) = policy.warning.as_deref() {
            emit_bash_runtime_warning_once(warning);
        }
        policy.login_shell = login_shell;
        policy.governance = governance;
        policy
    }

    #[cfg(not(feature = "tool-shell"))]
    {
        BashExecRuntimePolicy {
            login_shell,
            governance,
            ..BashExecRuntimePolicy::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSelfRuntimePolicy {
    pub max_source_chars: usize,
    pub max_total_chars: usize,
}

impl RuntimeSelfRuntimePolicy {
    #[must_use]
    pub fn from_limits(max_source_chars: usize, max_total_chars: usize) -> Self {
        let clamped_max_source_chars = max_source_chars.clamp(
            crate::config::MIN_RUNTIME_SELF_MAX_SOURCE_CHARS,
            crate::config::MAX_RUNTIME_SELF_MAX_SOURCE_CHARS,
        );
        let clamped_max_total_chars = max_total_chars.clamp(
            crate::config::MIN_RUNTIME_SELF_MAX_TOTAL_CHARS,
            crate::config::MAX_RUNTIME_SELF_MAX_TOTAL_CHARS,
        );

        Self {
            max_source_chars: clamped_max_source_chars,
            max_total_chars: clamped_max_total_chars,
        }
    }
}

impl Default for RuntimeSelfRuntimePolicy {
    fn default() -> Self {
        Self::from_limits(
            crate::config::DEFAULT_RUNTIME_SELF_MAX_SOURCE_CHARS,
            crate::config::DEFAULT_RUNTIME_SELF_MAX_TOTAL_CHARS,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutonomyOperationMode {
    #[default]
    Deny,
    ApprovalRequired,
    Allow,
}

impl AutonomyOperationMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Deny => "deny",
            Self::ApprovalRequired => "approval_required",
            Self::Allow => "allow",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AutonomyBudgetPolicy {
    pub max_capability_acquisitions_per_turn: usize,
    pub max_provider_switches_per_turn: usize,
    pub max_topology_mutations_per_turn: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomyPolicySnapshot {
    pub profile: AutonomyProfile,
    pub capability_acquisition_mode: AutonomyOperationMode,
    pub provider_switch_mode: AutonomyOperationMode,
    pub topology_mutation_mode: AutonomyOperationMode,
    pub requires_kernel_binding: bool,
    pub budget: AutonomyBudgetPolicy,
}

impl AutonomyPolicySnapshot {
    #[must_use]
    pub fn from_profile(profile: AutonomyProfile) -> Self {
        match profile {
            AutonomyProfile::DiscoveryOnly => Self {
                profile,
                capability_acquisition_mode: AutonomyOperationMode::Deny,
                provider_switch_mode: AutonomyOperationMode::Deny,
                topology_mutation_mode: AutonomyOperationMode::Deny,
                requires_kernel_binding: false,
                budget: AutonomyBudgetPolicy::default(),
            },
            AutonomyProfile::GuidedAcquisition => Self {
                profile,
                capability_acquisition_mode: AutonomyOperationMode::ApprovalRequired,
                provider_switch_mode: AutonomyOperationMode::ApprovalRequired,
                topology_mutation_mode: AutonomyOperationMode::ApprovalRequired,
                requires_kernel_binding: true,
                budget: AutonomyBudgetPolicy {
                    max_capability_acquisitions_per_turn: 1,
                    max_provider_switches_per_turn: 1,
                    max_topology_mutations_per_turn: 1,
                },
            },
            AutonomyProfile::BoundedAutonomous => Self {
                profile,
                capability_acquisition_mode: AutonomyOperationMode::Allow,
                provider_switch_mode: AutonomyOperationMode::ApprovalRequired,
                topology_mutation_mode: AutonomyOperationMode::ApprovalRequired,
                requires_kernel_binding: true,
                budget: AutonomyBudgetPolicy {
                    max_capability_acquisitions_per_turn: 2,
                    max_provider_switches_per_turn: 1,
                    max_topology_mutations_per_turn: 1,
                },
            },
        }
    }
}

// General network fetch/request policy for `web { url }`, low-level HTTP requests,
// and shared SSRF helpers reused by browser. This is separate from web-search
// provider selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebFetchRuntimePolicy {
    pub enabled: bool,
    pub allow_private_hosts: bool,
    pub enforce_allowed_domains: bool,
    pub allowed_domains: BTreeSet<String>,
    pub blocked_domains: BTreeSet<String>,
    pub timeout_seconds: u64,
    pub max_bytes: usize,
    pub max_redirects: usize,
}

impl Default for WebFetchRuntimePolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_private_hosts: true,
            enforce_allowed_domains: false,
            allowed_domains: BTreeSet::new(),
            blocked_domains: BTreeSet::new(),
            timeout_seconds: crate::config::DEFAULT_WEB_FETCH_TIMEOUT_SECONDS,
            max_bytes: crate::config::DEFAULT_WEB_FETCH_MAX_BYTES,
            max_redirects: crate::config::DEFAULT_WEB_FETCH_MAX_REDIRECTS,
        }
    }
}

#[cfg(feature = "feishu-integration")]
#[derive(Debug, Clone)]
pub struct FeishuToolRuntimeConfig {
    pub channel: FeishuChannelConfig,
    pub integration: FeishuIntegrationConfig,
}

#[cfg(feature = "feishu-integration")]
impl FeishuToolRuntimeConfig {
    pub fn from_loong_config(config: &LoongConfig) -> Option<Self> {
        has_enabled_feishu_runtime_credentials(&config.feishu).then(|| Self {
            channel: config.feishu.clone(),
            integration: config.feishu_integration.clone(),
        })
    }

    fn from_env() -> Option<Self> {
        has_feishu_runtime_credentials(&FeishuChannelConfig::default()).then(|| Self {
            channel: FeishuChannelConfig {
                enabled: true,
                ..FeishuChannelConfig::default()
            },
            integration: FeishuIntegrationConfig::default(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolExecutionConfig {
    pub default_timeout_seconds: Option<u64>,
    pub per_tool_timeout: BTreeMap<String, u64>,
}

impl ToolExecutionConfig {
    pub fn timeout_for_tool(&self, tool_name: &str) -> Option<u64> {
        for key in tool_timeout_lookup_keys(tool_name) {
            if let Some(timeout) = self.per_tool_timeout.get(key.as_str()).copied() {
                return Some(timeout);
            }
        }
        self.default_timeout_seconds
    }
}

fn tool_timeout_lookup_keys(tool_name: &str) -> Vec<String> {
    let canonical_tool_name = super::canonical_tool_name(tool_name);
    let visible_tool_name = super::legacy_display_tool_name(canonical_tool_name);
    let mut keys = Vec::new();

    for candidate in [
        tool_name.to_lowercase(),
        canonical_tool_name.to_lowercase(),
        visible_tool_name.to_lowercase(),
    ] {
        if !keys.contains(&candidate) {
            keys.push(candidate);
        }
    }

    match visible_tool_name.as_str() {
        "read" => push_timeout_lookup_key(&mut keys, "file.read"),
        "write" => push_timeout_lookup_key(&mut keys, "file.write"),
        "edit" => push_timeout_lookup_key(&mut keys, "file.edit"),
        _ => {}
    }

    keys
}

fn push_timeout_lookup_key(keys: &mut Vec<String>, candidate: &str) {
    let candidate = candidate.to_owned();
    if !keys.contains(&candidate) {
        keys.push(candidate);
    }
}

/// Typed runtime configuration for tool executors.
///
/// Replaces per-call `std::env::var` lookups with a single read from a
/// process-wide singleton that is populated once at startup.
#[derive(Debug, Clone)]
pub struct ToolRuntimeConfig {
    pub file_root: Option<PathBuf>,
    pub workspace_root: Option<PathBuf>,
    pub memory_sqlite_path: Option<PathBuf>,
    pub selected_memory_system_id: String,
    pub shell_allow: BTreeSet<String>,
    pub shell_deny: BTreeSet<String>,
    pub shell_default_mode: ShellPolicyDefault,
    pub config_path: Option<PathBuf>,
    pub sessions_enabled: bool,
    pub sessions_allow_mutation: bool,
    pub messages_enabled: bool,
    pub delegate_enabled: bool,
    pub runtime_self: RuntimeSelfRuntimePolicy,
    pub browser: BrowserRuntimePolicy,
    #[cfg(feature = "tool-shell")]
    pub bash_exec: BashExecRuntimePolicy,
    pub web_fetch: WebFetchRuntimePolicy,
    pub web_search: WebSearchRuntimePolicy,
    pub autonomy_profile: AutonomyProfile,
    pub skills: SkillsRuntimePolicy,
    pub tool_execution: ToolExecutionConfig,
    pub fs: FsRuntimePolicy,
    #[cfg(feature = "feishu-integration")]
    pub feishu: Option<FeishuToolRuntimeConfig>,
}

impl Default for ToolRuntimeConfig {
    fn default() -> Self {
        Self {
            file_root: None,
            workspace_root: None,
            memory_sqlite_path: None,
            selected_memory_system_id: crate::memory::DEFAULT_MEMORY_SYSTEM_ID.to_owned(),
            shell_allow: crate::config::DEFAULT_SHELL_ALLOW
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
            shell_deny: BTreeSet::new(),
            shell_default_mode: ShellPolicyDefault::Allow,
            config_path: None,
            sessions_enabled: true,
            sessions_allow_mutation: true,
            messages_enabled: false,
            delegate_enabled: true,
            runtime_self: RuntimeSelfRuntimePolicy::default(),
            browser: BrowserRuntimePolicy::default(),
            #[cfg(feature = "tool-shell")]
            bash_exec: BashExecRuntimePolicy::default(),
            web_fetch: WebFetchRuntimePolicy::default(),
            web_search: WebSearchRuntimePolicy::default(),
            autonomy_profile: AutonomyProfile::default(),
            skills: SkillsRuntimePolicy::default(),
            tool_execution: ToolExecutionConfig::default(),
            fs: FsRuntimePolicy::default(),
            #[cfg(feature = "feishu-integration")]
            feishu: None,
        }
    }
}

impl ToolRuntimeConfig {
    pub fn with_file_root_override(&self, file_root: PathBuf) -> Self {
        let mut overridden = self.clone();
        overridden.file_root = Some(file_root);
        overridden
    }

    pub fn with_workspace_root_override(&self, workspace_root: PathBuf) -> Self {
        let mut overridden = self.clone();
        overridden.workspace_root = Some(workspace_root);
        overridden
    }

    pub fn effective_workspace_root(&self) -> Option<&Path> {
        let configured_workspace_root = self.workspace_root.as_deref();
        let fallback_file_root = self.file_root.as_deref();
        configured_workspace_root.or(fallback_file_root)
    }

    pub fn path_resolution_root(&self) -> Option<&Path> {
        self.workspace_root.as_deref().or(self.file_root.as_deref())
    }

    pub fn filesystem_access_root(&self) -> Option<&Path> {
        match (self.file_root.as_deref(), self.workspace_root.as_deref()) {
            (Some(file_root), Some(workspace_root)) if workspace_root.starts_with(file_root) => {
                Some(file_root)
            }
            (_, Some(workspace_root)) => Some(workspace_root),
            (Some(file_root), None) => Some(file_root),
            (None, None) => None,
        }
    }

    pub fn default_working_directory(&self) -> PathBuf {
        self.path_resolution_root()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }

    pub fn from_loong_config(config: &LoongConfig, config_path: Option<&Path>) -> Self {
        let file_root = config.tools.configured_file_root();
        let workspace_root = config
            .tools
            .configured_runtime_workspace_root()
            .or_else(|| file_root.clone());
        // Tool runtime should reflect the provided config, not ambient
        // `LOONG_MEMORY_SYSTEM` overrides that are meant for other entrypaths.
        let memory_system_selection =
            crate::memory::resolve_memory_system_selection_without_env(config);
        let selected_memory_system_id = memory_system_selection.id;
        let web_fetch_allowed_domains = config.tools.web.normalized_allowed_domains();
        let web_fetch_enforce_allowed_domains = !web_fetch_allowed_domains.is_empty();
        let shell_allow: BTreeSet<String> = config
            .tools
            .shell_allow
            .iter()
            .map(|value| value.to_ascii_lowercase())
            .collect();
        let shell_deny: BTreeSet<String> = config
            .tools
            .shell_deny
            .iter()
            .map(|value| value.to_ascii_lowercase())
            .collect();
        #[cfg(feature = "tool-shell")]
        let bash_governance = build_bash_governance_runtime_policy(
            config.tools.bash.resolved_rules_dir(),
            shell_allow.iter(),
            shell_deny.iter(),
        );
        Self {
            file_root,
            workspace_root,
            memory_sqlite_path: Some(config.memory.resolved_sqlite_path()),
            selected_memory_system_id,
            shell_allow,
            shell_deny,
            shell_default_mode: ShellPolicyDefault::parse(&config.tools.shell_default_mode),
            config_path: config_path.map(Path::to_path_buf),
            sessions_enabled: config.tools.sessions.enabled,
            sessions_allow_mutation: config.tools.sessions.allow_mutation,
            messages_enabled: config.tools.messages.enabled,
            delegate_enabled: config.tools.delegate.enabled,
            runtime_self: RuntimeSelfRuntimePolicy::from_limits(
                config.tools.runtime_self.max_source_chars,
                config.tools.runtime_self.max_total_chars,
            ),
            browser: BrowserRuntimePolicy {
                enabled: config.tools.browser.enabled,
                max_sessions: config.tools.browser.max_sessions,
                max_links: config.tools.browser.max_links,
                max_text_chars: config.tools.browser.max_text_chars,
            },
            #[cfg(feature = "tool-shell")]
            bash_exec: build_bash_exec_runtime_policy(
                config.tools.bash.login_shell,
                bash_governance,
            ),
            web_fetch: WebFetchRuntimePolicy {
                enabled: config.tools.web.enabled,
                allow_private_hosts: config.tools.web.allow_private_hosts,
                enforce_allowed_domains: web_fetch_enforce_allowed_domains,
                allowed_domains: web_fetch_allowed_domains.into_iter().collect(),
                blocked_domains: config
                    .tools
                    .web
                    .normalized_blocked_domains()
                    .into_iter()
                    .collect(),
                timeout_seconds: config.tools.web.timeout_seconds,
                max_bytes: config.tools.web.max_bytes,
                max_redirects: config.tools.web.max_redirects,
            },
            web_search: WebSearchRuntimePolicy::from_loong_config(config),
            autonomy_profile: config.tools.autonomy_profile,
            skills: SkillsRuntimePolicy {
                enabled: config.skills.enabled,
                require_download_approval: config.skills.require_download_approval,
                allowed_domains: config
                    .skills
                    .normalized_allowed_domains()
                    .into_iter()
                    .collect(),
                blocked_domains: config
                    .skills
                    .normalized_blocked_domains()
                    .into_iter()
                    .collect(),
                install_root: config.skills.resolved_install_root(),
                auto_expose_installed: config.skills.auto_expose_installed,
            },
            tool_execution: ToolExecutionConfig {
                default_timeout_seconds: config.tools.tool_execution.default_timeout_seconds,
                per_tool_timeout: config
                    .tools
                    .tool_execution
                    .per_tool_timeout
                    .iter()
                    .map(|(k, v): (&String, &u64)| (k.to_lowercase(), *v))
                    .collect(),
            },
            fs: FsRuntimePolicy {
                deny_read_filenames: config
                    .tools
                    .fs
                    .deny_read_filenames
                    .iter()
                    .filter_map(|name| normalize_policy_filename(name))
                    .collect(),
            },
            #[cfg(feature = "feishu-integration")]
            feishu: FeishuToolRuntimeConfig::from_loong_config(config),
        }
    }

    /// Build a config by reading the legacy environment variables.
    ///
    /// Keeps full backward compatibility for callers that still rely on
    /// `LOONG_FILE_ROOT`.
    pub fn from_env() -> Self {
        let file_root = parse_env_path("LOONG_FILE_ROOT");
        let workspace_root = parse_env_path("LOONG_WORKSPACE_ROOT").or_else(|| file_root.clone());
        let memory_sqlite_path = std::env::var_os("LOONG_SQLITE_PATH")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let memory_sqlite_path = memory_sqlite_path.or_else(|| {
            let default_config = crate::config::LoongConfig::default();
            Some(default_config.memory.resolved_sqlite_path())
        });
        let selected_memory_system_id = crate::memory::registered_memory_system_id_from_env()
            .unwrap_or_else(|| crate::memory::DEFAULT_MEMORY_SYSTEM_ID.to_owned());
        let config_path = std::env::var("LOONG_CONFIG_PATH").ok().map(PathBuf::from);
        let shell_allow: BTreeSet<String> = crate::config::DEFAULT_SHELL_ALLOW
            .iter()
            .map(|value| (*value).to_owned())
            .collect();
        let shell_deny = BTreeSet::new();
        let sessions_enabled = parse_env_bool("LOONG_TOOL_SESSIONS_ENABLED").unwrap_or(true);
        let sessions_allow_mutation =
            parse_env_bool("LOONG_TOOL_SESSIONS_ALLOW_MUTATION").unwrap_or(true);
        let messages_enabled = parse_env_bool("LOONG_TOOL_MESSAGES_ENABLED").unwrap_or(false);
        let delegate_enabled = parse_env_bool("LOONG_TOOL_DELEGATE_ENABLED").unwrap_or(true);
        let runtime_self_max_source_chars = parse_env_usize("LOONG_RUNTIME_SELF_MAX_SOURCE_CHARS")
            .unwrap_or(crate::config::DEFAULT_RUNTIME_SELF_MAX_SOURCE_CHARS);
        let runtime_self_max_total_chars = parse_env_usize("LOONG_RUNTIME_SELF_MAX_TOTAL_CHARS")
            .unwrap_or(crate::config::DEFAULT_RUNTIME_SELF_MAX_TOTAL_CHARS);
        let runtime_self_policy = RuntimeSelfRuntimePolicy::from_limits(
            runtime_self_max_source_chars,
            runtime_self_max_total_chars,
        );
        let browser_enabled = parse_env_bool("LOONG_BROWSER_ENABLED").unwrap_or(true);
        let browser_max_sessions = parse_env_usize("LOONG_BROWSER_MAX_SESSIONS")
            .unwrap_or(crate::config::DEFAULT_BROWSER_MAX_SESSIONS);
        let browser_max_links = parse_env_usize("LOONG_BROWSER_MAX_LINKS")
            .unwrap_or(crate::config::DEFAULT_BROWSER_MAX_LINKS);
        let browser_max_text_chars = parse_env_usize("LOONG_BROWSER_MAX_TEXT_CHARS")
            .unwrap_or(crate::config::DEFAULT_BROWSER_MAX_TEXT_CHARS);
        let web_fetch_enabled = parse_env_bool("LOONG_WEB_FETCH_ENABLED").unwrap_or(true);
        let web_fetch_allow_private_hosts =
            parse_env_bool("LOONG_WEB_FETCH_ALLOW_PRIVATE_HOSTS").unwrap_or(false);
        let web_fetch_allowed_domains = parse_env_domain_list("LOONG_WEB_FETCH_ALLOWED_DOMAINS");
        let web_fetch_blocked_domains = parse_env_domain_list("LOONG_WEB_FETCH_BLOCKED_DOMAINS");
        let web_fetch_timeout_seconds = parse_env_u64("LOONG_WEB_FETCH_TIMEOUT_SECONDS")
            .unwrap_or(crate::config::DEFAULT_WEB_FETCH_TIMEOUT_SECONDS);
        let web_fetch_max_bytes = parse_env_usize("LOONG_WEB_FETCH_MAX_BYTES")
            .unwrap_or(crate::config::DEFAULT_WEB_FETCH_MAX_BYTES);
        let web_fetch_max_redirects = parse_env_usize("LOONG_WEB_FETCH_MAX_REDIRECTS")
            .unwrap_or(crate::config::DEFAULT_WEB_FETCH_MAX_REDIRECTS);
        let autonomy_profile = resolve_autonomy_profile_from_env();
        let enabled = parse_env_bool(SKILLS_ENABLED_ENV).unwrap_or(false);
        let require_download_approval =
            parse_env_bool(SKILLS_REQUIRE_DOWNLOAD_APPROVAL_ENV).unwrap_or(false);
        let allowed_domains = parse_env_domain_list(SKILLS_ALLOWED_DOMAINS_ENV);
        let blocked_domains = parse_env_domain_list(SKILLS_BLOCKED_DOMAINS_ENV);
        let install_root = parse_env_path(SKILLS_INSTALL_ROOT_ENV);
        let auto_expose_installed =
            parse_env_bool(SKILLS_AUTO_EXPOSE_INSTALLED_ENV).unwrap_or(false);

        let tool_execution_default_timeout = parse_env_u64("LOONG_TOOL_DEFAULT_TIMEOUT_SECONDS");
        let mut tool_execution_per_tool_timeout = BTreeMap::new();
        for (key, value) in std::env::vars() {
            if let Some(tool_name) = key.strip_prefix("LOONG_TOOL_")
                && let Some(stripped) = tool_name.strip_suffix("_TIMEOUT_SECONDS")
                && !stripped.is_empty()
                && stripped != "DEFAULT"
                && let Ok(timeout) = value.parse::<u64>()
            {
                tool_execution_per_tool_timeout.insert(stripped.to_lowercase(), timeout);
            }
        }
        let tool_execution = ToolExecutionConfig {
            default_timeout_seconds: tool_execution_default_timeout,
            per_tool_timeout: tool_execution_per_tool_timeout,
        };
        #[cfg(feature = "tool-shell")]
        let bash_exec = build_bash_exec_runtime_policy(
            false,
            build_bash_governance_runtime_policy(
                crate::config::ToolConfig::default()
                    .bash
                    .resolved_rules_dir(),
                shell_allow.iter(),
                shell_deny.iter(),
            ),
        );

        Self {
            file_root,
            workspace_root,
            memory_sqlite_path,
            selected_memory_system_id,
            shell_allow,
            shell_deny,
            shell_default_mode: ShellPolicyDefault::Allow,
            config_path,
            sessions_enabled,
            sessions_allow_mutation,
            messages_enabled,
            delegate_enabled,
            runtime_self: runtime_self_policy,
            browser: BrowserRuntimePolicy {
                enabled: browser_enabled,
                max_sessions: browser_max_sessions,
                max_links: browser_max_links,
                max_text_chars: browser_max_text_chars,
            },
            #[cfg(feature = "tool-shell")]
            bash_exec,
            web_fetch: WebFetchRuntimePolicy {
                enabled: web_fetch_enabled,
                allow_private_hosts: web_fetch_allow_private_hosts,
                enforce_allowed_domains: !web_fetch_allowed_domains.is_empty(),
                allowed_domains: web_fetch_allowed_domains,
                blocked_domains: web_fetch_blocked_domains,
                timeout_seconds: web_fetch_timeout_seconds,
                max_bytes: web_fetch_max_bytes,
                max_redirects: web_fetch_max_redirects,
            },
            web_search: WebSearchRuntimePolicy::from_env(),
            autonomy_profile,
            tool_execution,
            ..Self::default()
        }
        .with_skills_policy(SkillsRuntimePolicy {
            enabled,
            require_download_approval,
            allowed_domains,
            blocked_domains,
            install_root,
            auto_expose_installed,
        })
    }

    fn with_skills_policy(mut self, skills: SkillsRuntimePolicy) -> Self {
        self.skills = skills;
        #[cfg(feature = "feishu-integration")]
        {
            self.feishu = FeishuToolRuntimeConfig::from_env();
        }
        self
    }

    #[must_use]
    pub fn narrowed(&self, narrowing: &ToolRuntimeNarrowing) -> Self {
        if narrowing.is_empty() {
            return self.clone();
        }

        let mut narrowed = self.clone();

        if let Some(max_sessions) = narrowing.browser.max_sessions {
            narrowed.browser.max_sessions = narrowed.browser.max_sessions.min(max_sessions.max(1));
        }
        if let Some(max_links) = narrowing.browser.max_links {
            narrowed.browser.max_links = narrowed.browser.max_links.min(max_links.max(1));
        }
        if let Some(max_text_chars) = narrowing.browser.max_text_chars {
            narrowed.browser.max_text_chars =
                narrowed.browser.max_text_chars.min(max_text_chars.max(1));
        }

        narrowed.web_fetch.allow_private_hosts = match narrowing.web_fetch.allow_private_hosts {
            Some(false) => false,
            Some(true) => narrowed.web_fetch.allow_private_hosts,
            None => narrowed.web_fetch.allow_private_hosts,
        };

        let preserve_deny_all = narrowed.web_fetch.enforce_allowed_domains
            && narrowed.web_fetch.allowed_domains.is_empty();
        if narrowing.web_fetch.enforces_allowed_domains() {
            narrowed.web_fetch.enforce_allowed_domains = true;
            if !preserve_deny_all {
                narrowed.web_fetch.allowed_domains =
                    if narrowing.web_fetch.allowed_domains.is_empty() {
                        BTreeSet::new()
                    } else if narrowed.web_fetch.allowed_domains.is_empty() {
                        narrowing.web_fetch.allowed_domains.clone()
                    } else {
                        narrowed
                            .web_fetch
                            .allowed_domains
                            .intersection(&narrowing.web_fetch.allowed_domains)
                            .cloned()
                            .collect()
                    };
            }
        }
        narrowed
            .web_fetch
            .blocked_domains
            .extend(narrowing.web_fetch.blocked_domains.iter().cloned());

        if let Some(timeout_seconds) = narrowing.web_fetch.timeout_seconds {
            narrowed.web_fetch.timeout_seconds = narrowed
                .web_fetch
                .timeout_seconds
                .min(timeout_seconds.max(1));
        }
        if let Some(max_bytes) = narrowing.web_fetch.max_bytes {
            narrowed.web_fetch.max_bytes = narrowed.web_fetch.max_bytes.min(max_bytes.max(1));
        }
        if let Some(max_redirects) = narrowing.web_fetch.max_redirects {
            narrowed.web_fetch.max_redirects = narrowed.web_fetch.max_redirects.min(max_redirects);
        }

        narrowed
    }

    fn visible_child_tool_allowlist(tool_names: &[String]) -> Vec<String> {
        let mut visible_tool_names = Vec::new();

        for tool_name in tool_names {
            let visible_tool_name = super::model_visible_tool_name(tool_name.as_str());
            if !visible_tool_names.contains(&visible_tool_name) {
                visible_tool_names.push(visible_tool_name);
            }
        }

        visible_tool_names
    }

    #[must_use]
    pub(crate) fn delegate_child_prompt_summary(
        &self,
        subagent_contract: Option<&ConstrainedSubagentContractView>,
    ) -> Option<String> {
        let subagent_contract = subagent_contract?;
        let narrowing = &subagent_contract.runtime_narrowing;
        let effective = self.narrowed(narrowing);
        let child_exec_label = super::model_visible_tool_name(super::SHELL_EXEC_TOOL_NAME);
        let child_allowlist =
            Self::visible_child_tool_allowlist(&subagent_contract.child_tool_allowlist);
        let web_network_label = format!("{} network", super::model_visible_tool_name("web.fetch"));
        let mut lines = vec![
            "[delegate_child_runtime_contract]".to_owned(),
            "Plan within these child-session runtime limits:".to_owned(),
        ];
        let mut rendered_any = false;

        if let Some(mode) = subagent_contract.mode {
            rendered_any = true;
            let mode = match mode {
                ConstrainedSubagentMode::Async => "async",
                ConstrainedSubagentMode::Inline => "inline",
            };
            lines.push(format!("- subagent mode: {mode}"));
        }

        if let Some(identity) = subagent_contract.resolved_identity() {
            if let Some(nickname) = identity.nickname.as_deref() {
                rendered_any = true;
                lines.push(format!("- subagent nickname: {nickname}"));
            }
            if let Some(specialization) = identity.specialization.as_deref() {
                rendered_any = true;
                lines.push(format!("- subagent specialization: {specialization}"));
            }
        }

        if let Some(depth_budget) = subagent_contract.depth_budget {
            rendered_any = true;
            lines.push(format!(
                "- subagent depth budget: {}/{}",
                depth_budget.current, depth_budget.max
            ));
        }

        if let Some(active_child_budget) = subagent_contract.active_child_budget {
            rendered_any = true;
            lines.push(format!(
                "- subagent active-child budget snapshot: {}/{}",
                active_child_budget.current, active_child_budget.max
            ));
        }

        if let Some(timeout_seconds) = subagent_contract.timeout_seconds {
            rendered_any = true;
            lines.push(format!("- child timeout seconds: {}", timeout_seconds));
        }

        if let Some(allow_shell_in_child) = subagent_contract.allow_shell_in_child {
            rendered_any = true;
            lines.push(format!(
                "- child {child_exec_label}: {}",
                if allow_shell_in_child {
                    "allowed"
                } else {
                    "denied"
                }
            ));
        }

        if !subagent_contract.child_tool_allowlist.is_empty() || subagent_contract.mode.is_some() {
            rendered_any = true;
            let tool_allowlist = if child_allowlist.is_empty() {
                "none".to_owned()
            } else {
                child_allowlist.join(", ")
            };
            lines.push(format!("- child tool allowlist: {tool_allowlist}"));
        }

        if let Some(runtime_binding) = subagent_contract.runtime_binding {
            rendered_any = true;
            lines.push(format!(
                "- child runtime binding: {}",
                match runtime_binding {
                    ConstrainedSubagentRuntimeBinding::ContextBound => "context-bound",
                    ConstrainedSubagentRuntimeBinding::Direct => "direct",
                }
            ));
        }

        if let Some(subagent_profile) = subagent_contract.profile {
            rendered_any = true;
            let role = match subagent_profile.role {
                ConstrainedSubagentRole::Orchestrator => "orchestrator",
                ConstrainedSubagentRole::Leaf => "leaf",
            };
            let control_scope = match subagent_profile.control_scope {
                ConstrainedSubagentControlScope::Children => "children",
                ConstrainedSubagentControlScope::None => "none",
            };
            lines.push(format!("- subagent role: {role}"));
            lines.push(format!("- subagent control scope: {control_scope}"));
        }

        if effective.web_fetch.enabled {
            if narrowing.web_fetch.allow_private_hosts.is_some() {
                rendered_any = true;
                lines.push(format!(
                    "- {web_network_label} private hosts: {}",
                    if effective.web_fetch.allow_private_hosts {
                        "allowed"
                    } else {
                        "denied"
                    }
                ));
            }
            if narrowing.web_fetch.enforces_allowed_domains() {
                rendered_any = true;
                if effective.web_fetch.enforce_allowed_domains
                    && effective.web_fetch.allowed_domains.is_empty()
                {
                    lines.push(
                        format!(
                            "- {web_network_label} allowed domains: none (effective intersection is empty)"
                        ),
                    );
                } else {
                    lines.push(format!(
                        "- {web_network_label} allowed domains: {}",
                        effective
                            .web_fetch
                            .allowed_domains
                            .iter()
                            .map(String::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            if !narrowing.web_fetch.blocked_domains.is_empty() {
                rendered_any = true;
                lines.push(format!(
                    "- {web_network_label} blocked domains: {}",
                    effective
                        .web_fetch
                        .blocked_domains
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            if narrowing.web_fetch.timeout_seconds.is_some() {
                rendered_any = true;
                lines.push(format!(
                    "- {web_network_label} timeout seconds: {}",
                    effective.web_fetch.timeout_seconds
                ));
            }
            if narrowing.web_fetch.max_bytes.is_some() {
                rendered_any = true;
                lines.push(format!(
                    "- {web_network_label} max bytes: {}",
                    effective.web_fetch.max_bytes
                ));
            }
            if narrowing.web_fetch.max_redirects.is_some() {
                rendered_any = true;
                lines.push(format!(
                    "- {web_network_label} max redirects: {}",
                    effective.web_fetch.max_redirects
                ));
            }
        }

        if effective.browser.enabled {
            if narrowing.browser.max_sessions.is_some() {
                rendered_any = true;
                lines.push(format!(
                    "- browser max sessions: {}",
                    effective.browser.max_sessions
                ));
            }
            if narrowing.browser.max_links.is_some() {
                rendered_any = true;
                lines.push(format!(
                    "- browser max links: {}",
                    effective.browser.max_links
                ));
            }
            if narrowing.browser.max_text_chars.is_some() {
                rendered_any = true;
                lines.push(format!(
                    "- browser max text chars: {}",
                    effective.browser.max_text_chars
                ));
            }
        }

        if !rendered_any {
            return None;
        }

        lines.push("Treat these as enforced limits for this child session.".to_owned());
        Some(lines.join("\n"))
    }

    #[must_use]
    pub const fn browser_execution_security_tier(&self) -> ExecutionSecurityTier {
        self.browser.execution_security_tier()
    }

    #[must_use]
    pub fn autonomy_policy_snapshot(&self) -> AutonomyPolicySnapshot {
        AutonomyPolicySnapshot::from_profile(self.autonomy_profile)
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

fn parse_env_u64(key: &str) -> Option<u64> {
    std::env::var(key)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
}

fn parse_env_usize(key: &str) -> Option<usize> {
    std::env::var(key)
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
}

fn normalize_optional_string(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn normalize_policy_filename(name: &str) -> Option<String> {
    let normalized = name.trim().to_ascii_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}

fn parse_env_string(key: &str) -> Option<String> {
    normalize_optional_string(std::env::var(key).ok().as_deref())
}

fn parse_env_path(key: &str) -> Option<PathBuf> {
    let raw_path = parse_env_string(key)?;
    let path = PathBuf::from(raw_path);
    Some(path)
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

fn resolve_autonomy_profile_from_env() -> AutonomyProfile {
    let raw_profile = parse_env_string("LOONG_AUTONOMY_PROFILE");
    let Some(raw_profile) = raw_profile else {
        return AutonomyProfile::default();
    };

    let parsed_profile = crate::config::parse_autonomy_profile(raw_profile.as_str());
    let Some(profile) = parsed_profile else {
        let default_profile = AutonomyProfile::default();
        let default_profile_id = default_profile.as_str();
        let valid_values = crate::config::AUTONOMY_PROFILE_VALID_VALUES;

        #[allow(clippy::print_stderr)]
        {
            eprintln!(
                "warning: invalid LOONG_AUTONOMY_PROFILE `{raw_profile}`; falling back to `{default_profile_id}`. supported values: {valid_values}"
            );
        }
        return default_profile;
    };

    profile
}

#[cfg(feature = "feishu-integration")]
fn has_enabled_feishu_runtime_credentials(config: &FeishuChannelConfig) -> bool {
    if !config.enabled {
        return false;
    }

    has_secret_binding(config.app_id.as_ref(), config.app_id_env.as_deref())
        && has_secret_binding(config.app_secret.as_ref(), config.app_secret_env.as_deref())
        || config
            .accounts
            .values()
            .any(account_has_enabled_feishu_runtime_credentials)
}

#[cfg(feature = "feishu-integration")]
fn has_feishu_runtime_credentials(config: &FeishuChannelConfig) -> bool {
    has_secret_binding(config.app_id.as_ref(), config.app_id_env.as_deref())
        && has_secret_binding(config.app_secret.as_ref(), config.app_secret_env.as_deref())
        || config
            .accounts
            .values()
            .any(account_has_feishu_runtime_credentials)
}

#[cfg(feature = "feishu-integration")]
fn account_has_enabled_feishu_runtime_credentials(
    account: &crate::config::FeishuAccountConfig,
) -> bool {
    account.enabled.unwrap_or(true) && account_has_feishu_runtime_credentials(account)
}

#[cfg(feature = "feishu-integration")]
fn account_has_feishu_runtime_credentials(account: &crate::config::FeishuAccountConfig) -> bool {
    has_secret_binding(account.app_id.as_ref(), account.app_id_env.as_deref())
        && has_secret_binding(
            account.app_secret.as_ref(),
            account.app_secret_env.as_deref(),
        )
}

#[cfg(feature = "feishu-integration")]
fn has_secret_binding(secret_ref: Option<&SecretRef>, env_name: Option<&str>) -> bool {
    if let Some(secret_ref) = secret_ref {
        let explicit_env_name = secret_ref.explicit_env_name();
        if let Some(explicit_env_name) = explicit_env_name {
            let resolved_env_value = std::env::var(explicit_env_name.as_str()).ok();
            let has_resolved_env_value = resolved_env_value
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty());
            return has_resolved_env_value;
        }

        if has_configured_secret_ref(Some(secret_ref)) {
            return true;
        }
    }

    env_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|name| std::env::var(name).ok())
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
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
mod tests;

#[cfg(test)]
#[path = "runtime_config_delegate_prompt_tests.rs"]
mod delegate_prompt_tests;
