#[cfg(feature = "tool-shell")]
use crate::config::GovernedToolApprovalMode;
use crate::config::LoongConfig;

use super::runtime_config::SkillsRuntimePolicy;
use super::runtime_config::{ToolRuntimeConfig, WebFetchRuntimePolicy};
#[cfg(feature = "tool-shell")]
use super::shell_policy_ext::ShellPolicyDefault;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "tool-shell")]
pub struct ShellExecutionSecurityPosture {
    pub default_mode: ShellPolicyDefault,
    pub allow_count: usize,
    pub deny_count: usize,
    pub approval_mode: GovernedToolApprovalMode,
    pub autonomy_profile: crate::config::AutonomyProfile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolFileRootSecurityPosture {
    pub explicit_root: Option<String>,
    pub effective_root: String,
    pub root_exists: bool,
    pub uses_current_working_directory_fallback: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebFetchSecurityPosture {
    pub enabled: bool,
    pub allow_private_hosts: bool,
    pub enforce_allowed_domains: bool,
    pub allowed_domain_count: usize,
    pub blocked_domain_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserSurfaceSecurityPosture {
    pub enabled: bool,
    pub execution_tier: loong_contracts::ExecutionSecurityTier,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillsSecurityPosture {
    pub enabled: bool,
    pub override_active: bool,
    pub require_download_approval: bool,
    pub allowed_domain_count: usize,
    pub blocked_domain_count: usize,
    pub auto_expose_installed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillsSecurityPostureProbeFailure {
    pub enabled: bool,
    pub require_download_approval: bool,
    pub auto_expose_installed: bool,
    pub error: String,
}

#[cfg(feature = "tool-shell")]
pub fn shell_execution_security_posture(
    config: &LoongConfig,
    runtime: &ToolRuntimeConfig,
) -> ShellExecutionSecurityPosture {
    ShellExecutionSecurityPosture {
        default_mode: runtime.shell_default_mode,
        allow_count: runtime.shell_allow.len(),
        deny_count: runtime.shell_deny.len(),
        approval_mode: config.tools.approval.mode,
        autonomy_profile: config.tools.autonomy_profile,
    }
}

pub fn tool_file_root_security_posture(config: &LoongConfig) -> ToolFileRootSecurityPosture {
    let explicit_root = config.tools.file_root.as_deref().map(str::to_owned);
    let file_root_resolution = config.tools.file_root_resolution();
    let effective_root = file_root_resolution.path().display().to_string();
    let root_exists = file_root_resolution.path().exists();
    let uses_current_working_directory_fallback =
        file_root_resolution.uses_current_working_directory_fallback();

    ToolFileRootSecurityPosture {
        explicit_root,
        effective_root,
        root_exists,
        uses_current_working_directory_fallback,
    }
}

pub fn web_fetch_security_posture(policy: &WebFetchRuntimePolicy) -> WebFetchSecurityPosture {
    WebFetchSecurityPosture {
        enabled: policy.enabled,
        allow_private_hosts: policy.allow_private_hosts,
        enforce_allowed_domains: policy.enforce_allowed_domains,
        allowed_domain_count: policy.allowed_domains.len(),
        blocked_domain_count: policy.blocked_domains.len(),
    }
}

pub fn browser_surface_security_posture(
    runtime: &ToolRuntimeConfig,
) -> BrowserSurfaceSecurityPosture {
    BrowserSurfaceSecurityPosture {
        enabled: runtime.browser.enabled,
        execution_tier: runtime.browser_execution_security_tier(),
    }
}

pub fn skills_security_posture(
    policy: &SkillsRuntimePolicy,
    override_active: bool,
) -> SkillsSecurityPosture {
    SkillsSecurityPosture {
        enabled: policy.enabled,
        override_active,
        require_download_approval: policy.require_download_approval,
        allowed_domain_count: policy.allowed_domains.len(),
        blocked_domain_count: policy.blocked_domains.len(),
        auto_expose_installed: policy.auto_expose_installed,
    }
}

pub fn skills_security_posture_probe_failure(
    policy: &SkillsRuntimePolicy,
    error: String,
) -> SkillsSecurityPostureProbeFailure {
    SkillsSecurityPostureProbeFailure {
        enabled: policy.enabled,
        require_download_approval: policy.require_download_approval,
        auto_expose_installed: policy.auto_expose_installed,
        error,
    }
}
