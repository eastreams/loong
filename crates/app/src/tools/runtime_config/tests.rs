use super::*;
use crate::test_utils::{ScopedEnv, ScopedLoongHome};
#[cfg(feature = "feishu-integration")]
use std::collections::BTreeMap;
use std::path::PathBuf;

fn clear_tool_runtime_env(env: &mut ScopedEnv) {
    for key in [
        "LOONG_HOME",
        "LOONG_CONFIG_PATH",
        "LOONG_CONFIG_PATH",
        "LOONG_FILE_ROOT",
        "LOONG_WORKSPACE_ROOT",
        "LOONG_SQLITE_PATH",
        "LOONG_TOOL_SESSIONS_ENABLED",
        "LOONG_TOOL_SESSIONS_ALLOW_MUTATION",
        "LOONG_TOOL_MESSAGES_ENABLED",
        "LOONG_TOOL_DELEGATE_ENABLED",
        "LOONG_RUNTIME_SELF_MAX_SOURCE_CHARS",
        "LOONG_RUNTIME_SELF_MAX_TOTAL_CHARS",
        "LOONG_BROWSER_ENABLED",
        "LOONG_BROWSER_MAX_SESSIONS",
        "LOONG_BROWSER_MAX_LINKS",
        "LOONG_BROWSER_MAX_TEXT_CHARS",
        "LOONG_WEB_FETCH_ENABLED",
        "LOONG_WEB_FETCH_ALLOW_PRIVATE_HOSTS",
        "LOONG_WEB_FETCH_ALLOWED_DOMAINS",
        "LOONG_WEB_FETCH_BLOCKED_DOMAINS",
        "LOONG_WEB_FETCH_TIMEOUT_SECONDS",
        "LOONG_WEB_FETCH_MAX_BYTES",
        "LOONG_WEB_FETCH_MAX_REDIRECTS",
        "LOONG_WEB_SEARCH_ENABLED",
        "LOONG_WEB_SEARCH_PROVIDER",
        "LOONG_WEB_SEARCH_TIMEOUT_SECONDS",
        "LOONG_WEB_SEARCH_MAX_RESULTS",
        "LOONG_AUTONOMY_PROFILE",
        "BRAVE_API_KEY",
        "TAVILY_API_KEY",
        "PERPLEXITY_API_KEY",
        "EXA_API_KEY",
        "FIRECRAWL_API_KEY",
        "JINA_API_KEY",
        "JINA_AUTH_TOKEN",
    ] {
        env.remove(key);
    }
}

#[cfg(feature = "feishu-integration")]
fn clear_feishu_runtime_env(env: &mut ScopedEnv) {
    env.remove("FEISHU_APP_ID");
    env.remove("FEISHU_APP_SECRET");
}

#[test]
fn tool_runtime_config_from_env_defaults() {
    let config = ToolRuntimeConfig::default();
    assert!(config.file_root.is_none());
    assert!(config.workspace_root.is_none());
    assert!(config.config_path.is_none());
    assert!(config.sessions_enabled);
    assert!(config.sessions_allow_mutation);
    assert!(!config.messages_enabled);
    assert!(config.delegate_enabled);
    assert_eq!(
        config.runtime_self.max_source_chars,
        crate::config::DEFAULT_RUNTIME_SELF_MAX_SOURCE_CHARS
    );
    assert_eq!(
        config.runtime_self.max_total_chars,
        crate::config::DEFAULT_RUNTIME_SELF_MAX_TOTAL_CHARS
    );
    assert!(config.browser.enabled);
    assert_eq!(config.browser.max_sessions, 8);
    assert_eq!(config.browser.max_links, 40);
    assert_eq!(config.browser.max_text_chars, 6000);
    assert!(config.web_fetch.enabled);
    assert!(config.web_fetch.allow_private_hosts);
    assert!(config.web_fetch.allowed_domains.is_empty());
    assert!(config.web_fetch.blocked_domains.is_empty());
    assert_eq!(config.web_fetch.timeout_seconds, 15);
    assert_eq!(config.web_fetch.max_bytes, 1_048_576);
    assert_eq!(config.web_fetch.max_redirects, 3);
    assert!(config.web_search.enabled);
    assert_eq!(
        config.web_search.default_provider,
        crate::config::DEFAULT_WEB_SEARCH_PROVIDER
    );
    assert!(config.web_search.brave_api_key.is_none());
    assert!(config.web_search.tavily_api_key.is_none());
    assert!(config.web_search.perplexity_api_key.is_none());
    assert!(config.web_search.exa_api_key.is_none());
    assert!(config.web_search.jina_api_key.is_none());
    assert_eq!(
        config.web_search.timeout_seconds,
        crate::config::DEFAULT_WEB_SEARCH_TIMEOUT_SECONDS
    );
    assert_eq!(
        config.web_search.max_results,
        crate::config::DEFAULT_WEB_SEARCH_MAX_RESULTS
    );
    assert!(config.skills.enabled);
    assert!(!config.skills.require_download_approval);
    assert!(config.skills.allowed_domains.is_empty());
    assert!(config.skills.blocked_domains.is_empty());
    assert!(config.skills.install_root.is_none());
    assert!(!config.skills.auto_expose_installed);
}

#[test]
fn autonomy_profile_runtime_config_defaults_to_discovery_only() {
    let config = ToolRuntimeConfig::default();
    let snapshot = config.autonomy_policy_snapshot();

    assert_eq!(config.autonomy_profile, AutonomyProfile::DiscoveryOnly);
    assert_eq!(snapshot.profile, AutonomyProfile::DiscoveryOnly);
    assert_eq!(
        snapshot.capability_acquisition_mode,
        AutonomyOperationMode::Deny
    );
    assert_eq!(snapshot.provider_switch_mode, AutonomyOperationMode::Deny);
    assert_eq!(snapshot.topology_mutation_mode, AutonomyOperationMode::Deny);
    assert_eq!(snapshot.budget.max_capability_acquisitions_per_turn, 0);
    assert_eq!(snapshot.budget.max_provider_switches_per_turn, 0);
    assert_eq!(snapshot.budget.max_topology_mutations_per_turn, 0);
    assert_eq!(AutonomyProfile::DiscoveryOnly.as_str(), "discovery_only");
    assert_eq!(
        AutonomyProfile::GuidedAcquisition.as_str(),
        "guided_acquisition"
    );
    assert_eq!(
        AutonomyProfile::BoundedAutonomous.as_str(),
        "bounded_autonomous"
    );
}

#[test]
fn autonomy_profile_runtime_config_from_loong_config_uses_explicit_profile() {
    let mut config = crate::config::LoongConfig::default();
    config.tools.autonomy_profile = AutonomyProfile::GuidedAcquisition;

    let runtime = ToolRuntimeConfig::from_loong_config(&config, None);
    let snapshot = runtime.autonomy_policy_snapshot();

    assert_eq!(runtime.autonomy_profile, AutonomyProfile::GuidedAcquisition);
    assert_eq!(
        snapshot.capability_acquisition_mode,
        AutonomyOperationMode::ApprovalRequired
    );
    assert_eq!(
        snapshot.provider_switch_mode,
        AutonomyOperationMode::ApprovalRequired
    );
    assert_eq!(
        snapshot.topology_mutation_mode,
        AutonomyOperationMode::ApprovalRequired
    );
    assert_eq!(snapshot.budget.max_capability_acquisitions_per_turn, 1);
    assert_eq!(snapshot.budget.max_provider_switches_per_turn, 1);
    assert_eq!(snapshot.budget.max_topology_mutations_per_turn, 1);
}

#[cfg(feature = "tool-shell")]
#[test]
fn tool_runtime_config_default_marks_bash_exec_unavailable() {
    let config = ToolRuntimeConfig::default();

    assert!(!config.bash_exec.is_runtime_ready());
    assert!(!config.bash_exec.is_discoverable());
    assert!(config.bash_exec.command.is_none());
    assert!(config.bash_exec.warning.is_none());
    assert!(!config.bash_exec.login_shell);
}

#[cfg(feature = "tool-shell")]
#[test]
fn bash_exec_discoverability_requires_runtime_ready_and_governance_load_success() {
    let unavailable = BashExecRuntimePolicy::default();
    assert!(!unavailable.is_runtime_ready());
    assert!(!unavailable.is_discoverable());

    let runtime_ready = BashExecRuntimePolicy {
        available: true,
        command: Some(PathBuf::from("bash")),
        ..BashExecRuntimePolicy::default()
    };
    assert!(runtime_ready.is_runtime_ready());
    assert!(runtime_ready.is_discoverable());

    let governance_failed = BashExecRuntimePolicy {
        governance: BashGovernanceRuntimePolicy {
            load_error: Some("broken rules".to_owned()),
            ..BashGovernanceRuntimePolicy::default()
        },
        ..runtime_ready
    };
    assert!(governance_failed.is_runtime_ready());
    assert!(!governance_failed.is_discoverable());
}

#[cfg(feature = "tool-shell")]
#[test]
fn tool_runtime_config_projects_bash_login_shell_flag() {
    let config: crate::config::ToolConfig =
        toml::from_str("[bash]\nlogin_shell = true\n").expect("bash tool config");
    let loong = crate::config::LoongConfig {
        tools: config,
        ..crate::config::LoongConfig::default()
    };

    let runtime = ToolRuntimeConfig::from_loong_config(&loong, None);

    assert!(runtime.bash_exec.login_shell);
}

#[cfg(feature = "tool-shell")]
#[test]
fn tool_runtime_config_uses_loong_home_rules_dir_when_unset() {
    let home = ScopedLoongHome::new("loong-runtime-config-home");

    let runtime = ToolRuntimeConfig::from_loong_config(
        &LoongConfig::default(),
        Some(std::path::Path::new("/tmp/work/loong.toml")),
    );

    assert_eq!(
        runtime.bash_exec.governance.rules_dir,
        home.path().join("rules")
    );
}

#[cfg(feature = "tool-shell")]
#[test]
fn tool_runtime_config_keeps_relative_bash_rules_dir_override_relative() {
    let config: crate::config::ToolConfig =
        toml::from_str("[bash]\nrules_dir = \"custom/rules\"\n").expect("bash tool config");
    let loong = crate::config::LoongConfig {
        tools: config,
        ..crate::config::LoongConfig::default()
    };

    let runtime = ToolRuntimeConfig::from_loong_config(
        &loong,
        Some(std::path::Path::new("/tmp/work/loong.toml")),
    );

    assert_eq!(
        runtime.bash_exec.governance.rules_dir,
        PathBuf::from("custom/rules")
    );
}

#[cfg(feature = "tool-shell")]
#[test]
fn bash_governance_runtime_treats_missing_rules_dir_as_empty_rule_set() {
    let home = ScopedLoongHome::new("loong-runtime-governance-home");
    let config_path = home.path().join("loong.toml");

    let runtime =
        ToolRuntimeConfig::from_loong_config(&LoongConfig::default(), Some(config_path.as_path()));

    assert!(runtime.bash_exec.governance.load_error.is_none());
    assert!(runtime.bash_exec.governance.rules.is_empty());
}

#[cfg(feature = "tool-shell")]
#[test]
fn bash_governance_runtime_preserves_rule_load_error_for_broken_rule_file() {
    let home = ScopedLoongHome::new("loong-runtime-governance-broken-home");
    let rules_dir = home.path().join("rules");
    std::fs::create_dir_all(&rules_dir).expect("create rules dir");
    std::fs::write(rules_dir.join("broken.rules"), "not valid starlark")
        .expect("write broken rule file");
    let config_path = home.path().join("loong.toml");

    let runtime =
        ToolRuntimeConfig::from_loong_config(&LoongConfig::default(), Some(config_path.as_path()));

    assert!(runtime.bash_exec.governance.load_error.is_some());
}

#[test]
fn autonomy_profile_runtime_config_compiles_bounded_autonomous_snapshot() {
    let config = ToolRuntimeConfig {
        autonomy_profile: AutonomyProfile::BoundedAutonomous,
        ..ToolRuntimeConfig::default()
    };

    let snapshot = config.autonomy_policy_snapshot();

    assert_eq!(snapshot.profile, AutonomyProfile::BoundedAutonomous);
    assert_eq!(
        snapshot.capability_acquisition_mode,
        AutonomyOperationMode::Allow
    );
    assert_eq!(
        snapshot.provider_switch_mode,
        AutonomyOperationMode::ApprovalRequired
    );
    assert_eq!(
        snapshot.topology_mutation_mode,
        AutonomyOperationMode::ApprovalRequired
    );
    assert_eq!(snapshot.budget.max_capability_acquisitions_per_turn, 2);
    assert_eq!(snapshot.budget.max_provider_switches_per_turn, 1);
    assert_eq!(snapshot.budget.max_topology_mutations_per_turn, 1);
}

#[test]
fn autonomy_profile_runtime_config_from_env_invalid_value_fails_closed() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);
    env.set("LOONG_AUTONOMY_PROFILE", "chaos");

    let runtime = ToolRuntimeConfig::from_env();
    let snapshot = runtime.autonomy_policy_snapshot();

    assert_eq!(runtime.autonomy_profile, AutonomyProfile::DiscoveryOnly);
    assert_eq!(snapshot.profile, AutonomyProfile::DiscoveryOnly);
}

#[test]
fn autonomy_profile_runtime_config_from_env_uses_valid_value() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);
    env.set("LOONG_AUTONOMY_PROFILE", "guided_acquisition");

    let runtime = ToolRuntimeConfig::from_env();
    let snapshot = runtime.autonomy_policy_snapshot();

    assert_eq!(runtime.autonomy_profile, AutonomyProfile::GuidedAcquisition);
    assert_eq!(snapshot.profile, AutonomyProfile::GuidedAcquisition);
}

/// Deny starts empty so users are not forced to carry
/// any hardcoded restriction they did not opt into.
#[test]
fn default_deny_is_empty() {
    let config = ToolRuntimeConfig::default();
    assert!(config.shell_deny.is_empty());
}

/// Explicit config injection overrides defaults — verifies that
/// non-default values survive construction without env-var leakage.
#[test]
fn explicit_config_injection_overrides_defaults() {
    let config = ToolRuntimeConfig {
        sessions_enabled: false,
        sessions_allow_mutation: true,
        messages_enabled: true,
        delegate_enabled: false,
        shell_allow: BTreeSet::from(["git".to_owned(), "cargo".to_owned()]),
        file_root: Some(PathBuf::from("/tmp/test-root")),
        config_path: Some(PathBuf::from("/tmp/test-root/loong.toml")),
        runtime_self: RuntimeSelfRuntimePolicy::from_limits(4_096, 32_768),
        browser: BrowserRuntimePolicy {
            enabled: false,
            max_sessions: 4,
            max_links: 12,
            max_text_chars: 2_048,
        },
        web_fetch: WebFetchRuntimePolicy {
            enabled: false,
            allow_private_hosts: true,
            enforce_allowed_domains: true,
            allowed_domains: BTreeSet::from(["docs.example.com".to_owned()]),
            blocked_domains: BTreeSet::from(["internal.example".to_owned()]),
            timeout_seconds: 9,
            max_bytes: 262_144,
            max_redirects: 1,
        },
        skills: SkillsRuntimePolicy {
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
    assert_eq!(
        config.config_path,
        Some(PathBuf::from("/tmp/test-root/loong.toml"))
    );
    assert!(!config.sessions_enabled);
    assert!(config.sessions_allow_mutation);
    assert!(config.messages_enabled);
    assert!(!config.delegate_enabled);
    assert_eq!(config.runtime_self.max_source_chars, 4_096);
    assert_eq!(config.runtime_self.max_total_chars, 32_768);
    assert!(!config.browser.enabled);
    assert_eq!(config.browser.max_sessions, 4);
    assert_eq!(config.browser.max_links, 12);
    assert_eq!(config.browser.max_text_chars, 2_048);
    assert!(!config.web_fetch.enabled);
    assert!(config.web_fetch.allow_private_hosts);
    assert!(
        config
            .web_fetch
            .allowed_domains
            .contains("docs.example.com")
    );
    assert!(
        config
            .web_fetch
            .blocked_domains
            .contains("internal.example")
    );
    assert_eq!(config.web_fetch.timeout_seconds, 9);
    assert_eq!(config.web_fetch.max_bytes, 262_144);
    assert_eq!(config.web_fetch.max_redirects, 1);
    assert!(config.skills.enabled);
    assert!(!config.skills.require_download_approval);
    assert!(config.skills.allowed_domains.contains("skills.sh"));
    assert_eq!(
        config.skills.install_root,
        Some(PathBuf::from("/tmp/test-root/skills"))
    );
    assert!(!config.skills.auto_expose_installed);
}

#[test]
fn file_root_uses_injected_config() {
    let config = ToolRuntimeConfig {
        file_root: Some(PathBuf::from("/tmp/test-root")),
        ..ToolRuntimeConfig::default()
    };
    assert_eq!(config.file_root, Some(PathBuf::from("/tmp/test-root")));
}

#[test]
fn tool_runtime_config_from_loong_config_reads_fs_policy() {
    let mut config = crate::config::LoongConfig::default();
    config.tools.fs.deny_read_filenames = vec![
        "clippy.toml".to_owned(),
        "CLIPPY.TOML".to_owned(),
        " ".to_owned(),
    ];

    let runtime = ToolRuntimeConfig::from_loong_config(&config, None);

    assert_eq!(
        runtime.fs.deny_read_filenames,
        BTreeSet::from(["clippy.toml".to_owned()])
    );
}

#[test]
fn tool_runtime_config_from_loong_config_keeps_file_root_unset_when_not_configured() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);
    let config = crate::config::LoongConfig::default();

    let runtime = ToolRuntimeConfig::from_loong_config(&config, None);

    assert_eq!(runtime.file_root, None);
}

#[test]
fn memory_sqlite_path_uses_injected_config() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);
    let config = crate::config::LoongConfig::default();
    let runtime = ToolRuntimeConfig::from_loong_config(&config, None);

    assert_eq!(
        runtime.memory_sqlite_path,
        Some(config.memory.resolved_sqlite_path())
    );
}

#[test]
fn memory_sqlite_path_from_loong_config_ignores_env_override() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);
    env.set("LOONG_SQLITE_PATH", "/tmp/env-tool-runtime-memory.sqlite3");

    let config = crate::config::LoongConfig::default();
    let runtime = ToolRuntimeConfig::from_loong_config(&config, None);

    assert_eq!(
        runtime.memory_sqlite_path,
        Some(config.memory.resolved_sqlite_path())
    );
}

#[test]
fn selected_memory_system_id_uses_injected_config() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);
    let mut config = crate::config::LoongConfig::default();
    config.memory.system = crate::config::MemorySystemKind::WorkspaceRecall;

    let runtime = ToolRuntimeConfig::from_loong_config(&config, None);

    assert_eq!(runtime.selected_memory_system_id, "workspace_recall");
}

#[test]
fn selected_memory_system_id_from_loong_config_ignores_env_override() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);
    env.set(crate::memory::MEMORY_SYSTEM_ENV, "recall_first");

    let mut config = crate::config::LoongConfig::default();
    config.memory.system = crate::config::MemorySystemKind::WorkspaceRecall;

    let runtime = ToolRuntimeConfig::from_loong_config(&config, None);

    assert_eq!(runtime.selected_memory_system_id, "workspace_recall");
}

#[test]
fn selected_memory_system_id_supports_recall_first_from_config() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);
    let mut config = crate::config::LoongConfig::default();
    config.memory.system = crate::config::MemorySystemKind::RecallFirst;

    let runtime = ToolRuntimeConfig::from_loong_config(&config, None);

    assert_eq!(runtime.selected_memory_system_id, "recall_first");
}

#[test]
fn memory_sqlite_path_from_env_uses_legacy_override() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);
    env.set("LOONG_SQLITE_PATH", "/tmp/tool-runtime-memory.sqlite3");

    let runtime = ToolRuntimeConfig::from_env();

    assert_eq!(
        runtime.memory_sqlite_path,
        Some(PathBuf::from("/tmp/tool-runtime-memory.sqlite3"))
    );
}

#[test]
fn selected_memory_system_id_from_env_uses_registered_override() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);
    env.set(crate::memory::MEMORY_SYSTEM_ENV, "workspace_recall");

    let runtime = ToolRuntimeConfig::from_env();

    assert_eq!(runtime.selected_memory_system_id, "workspace_recall");
}

#[test]
fn selected_memory_system_id_from_env_supports_recall_first() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);
    env.set(crate::memory::MEMORY_SYSTEM_ENV, "recall_first");

    let runtime = ToolRuntimeConfig::from_env();

    assert_eq!(runtime.selected_memory_system_id, "recall_first");
}

#[test]
fn selected_memory_system_id_from_env_falls_back_on_unknown_value() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);
    env.set(crate::memory::MEMORY_SYSTEM_ENV, "unknown_memory_system");

    let runtime = ToolRuntimeConfig::from_env();

    assert_eq!(
        runtime.selected_memory_system_id,
        crate::memory::DEFAULT_MEMORY_SYSTEM_ID
    );
}

#[test]
fn memory_sqlite_path_from_env_falls_back_to_loong_home() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);
    let runtime_home = ScopedLoongHome::new("loong-tool-runtime-memory-home");

    let runtime = ToolRuntimeConfig::from_env();

    assert_eq!(
        runtime.memory_sqlite_path,
        Some(runtime_home.path().join("memory.sqlite3"))
    );
}

#[test]
fn empty_legacy_memory_sqlite_path_falls_back_to_loong_home() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);
    let runtime_home = ScopedLoongHome::new("loong-tool-runtime-empty-sqlite-home");
    env.set("LOONG_SQLITE_PATH", "");

    let runtime = ToolRuntimeConfig::from_env();

    assert_eq!(
        runtime.memory_sqlite_path,
        Some(runtime_home.path().join("memory.sqlite3"))
    );
}

#[test]
fn from_env_defaults_to_empty_allowlist() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);
    #[cfg(feature = "feishu-integration")]
    clear_feishu_runtime_env(&mut env);

    let config = ToolRuntimeConfig::from_env();
    assert!(config.shell_allow.is_empty());
}

#[test]
fn from_loong_config_projects_runtime_self_policy() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);
    #[cfg(feature = "feishu-integration")]
    clear_feishu_runtime_env(&mut env);

    let mut config = crate::config::LoongConfig::default();
    config.tools.runtime_self.max_source_chars = 12_345;
    config.tools.runtime_self.max_total_chars = 67_890;

    let runtime = ToolRuntimeConfig::from_loong_config(&config, None);

    assert_eq!(runtime.runtime_self.max_source_chars, 12_345);
    assert_eq!(runtime.runtime_self.max_total_chars, 67_890);
}

#[test]
fn tool_runtime_config_from_env_reads_workspace_root_override() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);

    env.set("LOONG_FILE_ROOT", "/tmp/loong-tool-root");
    env.set("LOONG_WORKSPACE_ROOT", "/tmp/loong-workspace-root");

    let runtime = ToolRuntimeConfig::from_env();

    assert_eq!(
        runtime.file_root.as_deref(),
        Some(Path::new("/tmp/loong-tool-root"))
    );
    assert_eq!(
        runtime.workspace_root.as_deref(),
        Some(Path::new("/tmp/loong-workspace-root"))
    );
    assert_eq!(
        runtime.effective_workspace_root(),
        Some(Path::new("/tmp/loong-workspace-root"))
    );
}

#[test]
fn tool_runtime_config_workspace_root_override_preserves_tool_root_truth() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);

    let mut config = crate::config::LoongConfig::default();
    config.tools.file_root = Some("/tmp/loong-tool-root".to_owned());

    let runtime = ToolRuntimeConfig::from_loong_config(&config, None);
    let runtime =
        runtime.with_workspace_root_override(PathBuf::from("/tmp/loong-runtime-workspace"));

    assert_eq!(
        runtime.file_root.as_deref(),
        Some(Path::new("/tmp/loong-tool-root"))
    );
    assert_eq!(
        runtime.workspace_root.as_deref(),
        Some(Path::new("/tmp/loong-runtime-workspace"))
    );
    assert_eq!(
        runtime.effective_workspace_root(),
        Some(Path::new("/tmp/loong-runtime-workspace"))
    );
}

#[test]
fn from_loong_config_projects_session_mutation_toggle() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);
    #[cfg(feature = "feishu-integration")]
    clear_feishu_runtime_env(&mut env);

    let mut config = crate::config::LoongConfig::default();
    config.tools.sessions.allow_mutation = true;

    let runtime = ToolRuntimeConfig::from_loong_config(&config, None);

    assert!(runtime.sessions_enabled);
    assert!(runtime.sessions_allow_mutation);
}

#[test]
fn from_loong_config_canonicalizes_web_search_provider_alias() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);
    #[cfg(feature = "feishu-integration")]
    clear_feishu_runtime_env(&mut env);

    let mut config = crate::config::LoongConfig::default();
    config.tools.web_search.default_provider = "ddg".to_owned();

    let runtime = ToolRuntimeConfig::from_loong_config(&config, None);

    assert_eq!(
        runtime.web_search.default_provider,
        crate::config::DEFAULT_WEB_SEARCH_PROVIDER
    );
}

#[cfg(feature = "tool-shell")]
#[test]
fn injected_config_overrides_global() {
    let _env = ScopedEnv::new();
    let injected_root = tempfile::tempdir().expect("create injected file root");
    let injected_root_path = injected_root.path().to_path_buf();
    let config_path = injected_root_path.join("loong.toml");
    let config = ToolRuntimeConfig {
        file_root: Some(injected_root_path),
        shell_allow: BTreeSet::from(["echo".to_owned()]),
        config_path: Some(config_path),
        ..ToolRuntimeConfig::default()
    };
    let result = crate::tools::execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
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
fn from_env_parses_skills_policy() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);
    #[cfg(feature = "feishu-integration")]
    clear_feishu_runtime_env(&mut env);
    env.set("LOONG_TOOL_SESSIONS_ENABLED", "false");
    env.set("LOONG_TOOL_SESSIONS_ALLOW_MUTATION", "true");
    env.set("LOONG_TOOL_MESSAGES_ENABLED", "true");
    env.set("LOONG_TOOL_DELEGATE_ENABLED", "false");
    env.set("LOONG_BROWSER_ENABLED", "false");
    env.set("LOONG_BROWSER_MAX_SESSIONS", "4");
    env.set("LOONG_BROWSER_MAX_LINKS", "12");
    env.set("LOONG_BROWSER_MAX_TEXT_CHARS", "2048");
    env.set("LOONG_WEB_FETCH_ENABLED", "false");
    env.set("LOONG_WEB_FETCH_ALLOW_PRIVATE_HOSTS", "true");
    env.set(
        "LOONG_WEB_FETCH_ALLOWED_DOMAINS",
        "docs.example.com,api.example.com",
    );
    env.set("LOONG_WEB_FETCH_BLOCKED_DOMAINS", "internal.example");
    env.set("LOONG_WEB_FETCH_TIMEOUT_SECONDS", "9");
    env.set("LOONG_WEB_FETCH_MAX_BYTES", "262144");
    env.set("LOONG_WEB_FETCH_MAX_REDIRECTS", "1");
    env.set("LOONG_SKILLS_ENABLED", "true");
    env.set("LOONG_SKILLS_REQUIRE_DOWNLOAD_APPROVAL", "false");
    env.set("LOONG_SKILLS_AUTO_EXPOSE_INSTALLED", "false");

    let config = ToolRuntimeConfig::from_env();
    assert!(!config.sessions_enabled);
    assert!(config.sessions_allow_mutation);
    assert!(config.messages_enabled);
    assert!(!config.delegate_enabled);
    assert!(!config.browser.enabled);
    assert_eq!(config.browser.max_sessions, 4);
    assert_eq!(config.browser.max_links, 12);
    assert_eq!(config.browser.max_text_chars, 2_048);
    assert!(!config.web_fetch.enabled);
    assert!(config.web_fetch.allow_private_hosts);
    assert!(
        config
            .web_fetch
            .allowed_domains
            .contains("docs.example.com")
    );
    assert!(config.web_fetch.allowed_domains.contains("api.example.com"));
    assert!(
        config
            .web_fetch
            .blocked_domains
            .contains("internal.example")
    );
    assert_eq!(config.web_fetch.timeout_seconds, 9);
    assert_eq!(config.web_fetch.max_bytes, 262_144);
    assert_eq!(config.web_fetch.max_redirects, 1);
    assert!(config.web_search.enabled);
    assert_eq!(
        config.web_search.default_provider,
        crate::config::DEFAULT_WEB_SEARCH_PROVIDER
    );
    assert!(config.web_search.brave_api_key.is_none());
    assert!(config.web_search.tavily_api_key.is_none());
    assert_eq!(
        config.web_search.timeout_seconds,
        crate::config::DEFAULT_WEB_SEARCH_TIMEOUT_SECONDS
    );
    assert_eq!(
        config.web_search.max_results,
        crate::config::DEFAULT_WEB_SEARCH_MAX_RESULTS
    );
    assert!(config.skills.enabled);
    assert!(!config.skills.require_download_approval);
    assert!(config.skills.allowed_domains.is_empty());
    assert!(config.skills.blocked_domains.is_empty());
    assert!(config.skills.install_root.is_none());
    assert!(!config.skills.auto_expose_installed);
}

#[test]
fn from_env_parses_canonical_skills_env_names() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);
    #[cfg(feature = "feishu-integration")]
    clear_feishu_runtime_env(&mut env);

    env.set("LOONG_SKILLS_ENABLED", "true");
    env.set("LOONG_SKILLS_REQUIRE_DOWNLOAD_APPROVAL", "false");
    env.set("LOONG_SKILLS_INSTALL_ROOT", "/tmp/.loong/skills");
    env.set("LOONG_SKILLS_AUTO_EXPOSE_INSTALLED", "true");

    let config = ToolRuntimeConfig::from_env();
    assert!(config.skills.enabled);
    assert!(!config.skills.require_download_approval);
    assert!(config.skills.allowed_domains.is_empty());
    assert!(config.skills.blocked_domains.is_empty());
    assert_eq!(
        config.skills.install_root,
        Some(PathBuf::from("/tmp/.loong/skills"))
    );
    assert!(config.skills.auto_expose_installed);
}

#[test]
fn from_env_clamps_runtime_self_policy() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);
    #[cfg(feature = "feishu-integration")]
    clear_feishu_runtime_env(&mut env);
    env.set("LOONG_RUNTIME_SELF_MAX_SOURCE_CHARS", "999999");
    env.set("LOONG_RUNTIME_SELF_MAX_TOTAL_CHARS", "1");

    let config = ToolRuntimeConfig::from_env();

    assert_eq!(
        config.runtime_self.max_source_chars,
        crate::config::MAX_RUNTIME_SELF_MAX_SOURCE_CHARS
    );
    assert_eq!(
        config.runtime_self.max_total_chars,
        crate::config::MIN_RUNTIME_SELF_MAX_TOTAL_CHARS
    );
}

#[test]
fn from_env_canonicalizes_and_clamps_web_search_policy() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);
    #[cfg(feature = "feishu-integration")]
    clear_feishu_runtime_env(&mut env);
    env.set("LOONG_WEB_SEARCH_PROVIDER", "DDG");
    env.set("LOONG_WEB_SEARCH_TIMEOUT_SECONDS", "999");
    env.set("LOONG_WEB_SEARCH_MAX_RESULTS", "42");
    env.set(
        crate::config::WEB_SEARCH_BRAVE_API_KEY_ENV,
        "brave-test-key",
    );
    env.set(
        crate::config::WEB_SEARCH_TAVILY_API_KEY_ENV,
        "tavily-test-key",
    );
    env.set(
        crate::config::WEB_SEARCH_PERPLEXITY_API_KEY_ENV,
        "perplexity-test-key",
    );
    env.set(crate::config::WEB_SEARCH_EXA_API_KEY_ENV, "exa-test-key");
    env.set(
        crate::config::WEB_SEARCH_FIRECRAWL_API_KEY_ENV,
        "firecrawl-test-key",
    );
    env.set(
        crate::config::WEB_SEARCH_JINA_AUTH_TOKEN_ENV,
        "jina-test-key",
    );

    let config = ToolRuntimeConfig::from_env();

    assert_eq!(
        config.web_search.default_provider,
        crate::config::DEFAULT_WEB_SEARCH_PROVIDER
    );
    assert_eq!(config.web_search.timeout_seconds, 60);
    assert_eq!(config.web_search.max_results, 10);
    assert_eq!(
        config.web_search.brave_api_key.as_deref(),
        Some("brave-test-key")
    );
    assert_eq!(
        config.web_search.tavily_api_key.as_deref(),
        Some("tavily-test-key")
    );
    assert_eq!(
        config.web_search.perplexity_api_key.as_deref(),
        Some("perplexity-test-key")
    );
    assert_eq!(
        config.web_search.exa_api_key.as_deref(),
        Some("exa-test-key")
    );
    assert_eq!(
        config.web_search.firecrawl_api_key.as_deref(),
        Some("firecrawl-test-key")
    );
    assert_eq!(
        config.web_search.jina_api_key.as_deref(),
        Some("jina-test-key")
    );
}

#[test]
fn from_env_preserves_legacy_web_search_env_fallbacks() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);
    #[cfg(feature = "feishu-integration")]
    clear_feishu_runtime_env(&mut env);
    env.set("LOONG_WEB_SEARCH_PROVIDER", "tavily");
    env.set("LOONG_WEB_SEARCH_TIMEOUT_SECONDS", "41");
    env.set("LOONG_WEB_SEARCH_MAX_RESULTS", "7");

    let config = ToolRuntimeConfig::from_env();

    assert_eq!(config.web_search.default_provider, "tavily");
    assert_eq!(config.web_search.timeout_seconds, 41);
    assert_eq!(config.web_search.max_results, 7);
}

#[test]
fn from_loong_config_resolves_inline_env_refs_for_web_search_credentials() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);
    #[cfg(feature = "feishu-integration")]
    clear_feishu_runtime_env(&mut env);
    env.set("TEAM_EXA_KEY", "exa-inline-env");

    let mut config = LoongConfig::default();
    config.tools.web_search.default_provider = crate::config::WEB_SEARCH_PROVIDER_EXA.to_owned();
    config.tools.web_search.exa_api_key = Some("${TEAM_EXA_KEY}".to_owned());

    let runtime = ToolRuntimeConfig::from_loong_config(&config, None);

    assert_eq!(
        runtime.web_search.default_provider,
        crate::config::WEB_SEARCH_PROVIDER_EXA
    );
    assert_eq!(
        runtime.web_search.exa_api_key.as_deref(),
        Some("exa-inline-env")
    );
}

#[test]
fn from_loong_config_resolves_inline_env_refs_for_firecrawl_web_search_credentials() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);
    #[cfg(feature = "feishu-integration")]
    clear_feishu_runtime_env(&mut env);
    env.set("TEAM_FIRECRAWL_KEY", "firecrawl-inline-env");

    let mut config = LoongConfig::default();
    let provider_id = crate::config::WEB_SEARCH_PROVIDER_FIRECRAWL.to_owned();
    let credential_ref = "${TEAM_FIRECRAWL_KEY}".to_owned();

    config.tools.web_search.default_provider = provider_id;
    config.tools.web_search.firecrawl_api_key = Some(credential_ref);

    let runtime = ToolRuntimeConfig::from_loong_config(&config, None);
    let runtime_provider = runtime.web_search.default_provider.as_str();
    let runtime_credential = runtime.web_search.firecrawl_api_key.as_deref();

    assert_eq!(
        runtime_provider,
        crate::config::WEB_SEARCH_PROVIDER_FIRECRAWL
    );
    assert_eq!(runtime_credential, Some("firecrawl-inline-env"));
}

#[test]
fn skills_policy_struct_construction() {
    let policy = SkillsRuntimePolicy {
        enabled: true,
        require_download_approval: false,
        allowed_domains: BTreeSet::from(["skills.sh".to_owned(), "clawhub.ai".to_owned()]),
        blocked_domains: BTreeSet::from([
            "malicious.example".to_owned(),
            "*.clawhub.io".to_owned(),
        ]),
        install_root: Some(PathBuf::from("/tmp/managed-skills")),
        auto_expose_installed: false,
    };

    assert!(policy.enabled);
    assert!(!policy.require_download_approval);
    assert!(policy.allowed_domains.contains("skills.sh"));
    assert!(policy.allowed_domains.contains("clawhub.ai"));
    assert!(policy.blocked_domains.contains("malicious.example"));
    assert!(policy.blocked_domains.contains("*.clawhub.io"));
    assert_eq!(
        policy.install_root,
        Some(PathBuf::from("/tmp/managed-skills"))
    );
    assert!(!policy.auto_expose_installed);
}

#[test]
fn browser_policy_struct_construction() {
    let policy = BrowserRuntimePolicy {
        enabled: false,
        max_sessions: 4,
        max_links: 12,
        max_text_chars: 2_048,
    };

    assert!(!policy.enabled);
    assert_eq!(policy.max_sessions, 4);
    assert_eq!(policy.max_links, 12);
    assert_eq!(policy.max_text_chars, 2_048);
}

#[test]
fn web_fetch_policy_struct_construction() {
    let policy = WebFetchRuntimePolicy {
        enabled: false,
        allow_private_hosts: true,
        enforce_allowed_domains: true,
        allowed_domains: BTreeSet::from(["docs.example.com".to_owned()]),
        blocked_domains: BTreeSet::from(["internal.example".to_owned()]),
        timeout_seconds: 9,
        max_bytes: 262_144,
        max_redirects: 1,
    };

    assert!(!policy.enabled);
    assert!(policy.allow_private_hosts);
    assert!(policy.enforce_allowed_domains);
    assert!(policy.allowed_domains.contains("docs.example.com"));
    assert!(policy.blocked_domains.contains("internal.example"));
    assert_eq!(policy.timeout_seconds, 9);
    assert_eq!(policy.max_bytes, 262_144);
    assert_eq!(policy.max_redirects, 1);
}

#[test]
fn tool_runtime_config_narrowed_intersects_web_domains_and_clamps_browser_limits() {
    let base = ToolRuntimeConfig {
        browser: BrowserRuntimePolicy {
            enabled: true,
            max_sessions: 4,
            max_links: 12,
            max_text_chars: 2_048,
        },
        web_fetch: WebFetchRuntimePolicy {
            enabled: true,
            allow_private_hosts: true,
            enforce_allowed_domains: true,
            allowed_domains: BTreeSet::from([
                "docs.example.com".to_owned(),
                "api.example.com".to_owned(),
            ]),
            blocked_domains: BTreeSet::from(["blocked.example.com".to_owned()]),
            timeout_seconds: 15,
            max_bytes: 8_192,
            max_redirects: 4,
        },
        ..ToolRuntimeConfig::default()
    };
    let narrowing = ToolRuntimeNarrowing {
        browser: BrowserRuntimeNarrowing {
            max_sessions: Some(1),
            max_links: Some(6),
            max_text_chars: Some(512),
        },
        web_fetch: WebFetchRuntimeNarrowing {
            allow_private_hosts: Some(false),
            enforce_allowed_domains: false,
            allowed_domains: BTreeSet::from(["docs.example.com".to_owned()]),
            blocked_domains: BTreeSet::from(["deny.example.com".to_owned()]),
            timeout_seconds: Some(5),
            max_bytes: Some(4_096),
            max_redirects: Some(2),
        },
    };

    let effective = base.narrowed(&narrowing);

    assert_eq!(effective.browser.max_sessions, 1);
    assert_eq!(effective.browser.max_links, 6);
    assert_eq!(effective.browser.max_text_chars, 512);
    assert!(!effective.web_fetch.allow_private_hosts);
    assert_eq!(
        effective.web_fetch.allowed_domains,
        BTreeSet::from(["docs.example.com".to_owned()])
    );
    assert!(effective.web_fetch.enforce_allowed_domains);
    assert_eq!(
        effective.web_fetch.blocked_domains,
        BTreeSet::from([
            "blocked.example.com".to_owned(),
            "deny.example.com".to_owned(),
        ])
    );
    assert_eq!(effective.web_fetch.timeout_seconds, 5);
    assert_eq!(effective.web_fetch.max_bytes, 4_096);
    assert_eq!(effective.web_fetch.max_redirects, 2);
}

#[test]
fn tool_runtime_config_narrowed_uses_child_allowlist_when_parent_has_none() {
    let base = ToolRuntimeConfig {
        web_fetch: WebFetchRuntimePolicy {
            enabled: true,
            allow_private_hosts: false,
            enforce_allowed_domains: false,
            allowed_domains: BTreeSet::new(),
            blocked_domains: BTreeSet::new(),
            timeout_seconds: 15,
            max_bytes: 8_192,
            max_redirects: 4,
        },
        ..ToolRuntimeConfig::default()
    };
    let narrowing = ToolRuntimeNarrowing {
        web_fetch: WebFetchRuntimeNarrowing {
            allow_private_hosts: None,
            enforce_allowed_domains: false,
            allowed_domains: BTreeSet::from(["docs.example.com".to_owned()]),
            blocked_domains: BTreeSet::new(),
            timeout_seconds: None,
            max_bytes: None,
            max_redirects: None,
        },
        ..ToolRuntimeNarrowing::default()
    };

    let effective = base.narrowed(&narrowing);

    assert_eq!(
        effective.web_fetch.allowed_domains,
        BTreeSet::from(["docs.example.com".to_owned()])
    );
    assert!(effective.web_fetch.enforce_allowed_domains);
}

#[test]
fn tool_runtime_config_narrowed_fail_closes_disjoint_allowlists() {
    let base = ToolRuntimeConfig {
        web_fetch: WebFetchRuntimePolicy {
            enabled: true,
            allow_private_hosts: false,
            enforce_allowed_domains: true,
            allowed_domains: BTreeSet::from(["api.example.com".to_owned()]),
            blocked_domains: BTreeSet::new(),
            timeout_seconds: 15,
            max_bytes: 8_192,
            max_redirects: 4,
        },
        ..ToolRuntimeConfig::default()
    };
    let narrowing = ToolRuntimeNarrowing {
        web_fetch: WebFetchRuntimeNarrowing {
            allowed_domains: BTreeSet::from(["docs.example.com".to_owned()]),
            ..WebFetchRuntimeNarrowing::default()
        },
        ..ToolRuntimeNarrowing::default()
    };

    let effective = base.narrowed(&narrowing);

    assert!(effective.web_fetch.enforce_allowed_domains);
    assert!(
        effective.web_fetch.allowed_domains.is_empty(),
        "disjoint allowlists should preserve an enforced empty intersection"
    );
}

#[test]
fn tool_runtime_config_narrowed_preserves_existing_deny_all_allowlist() {
    let base = ToolRuntimeConfig {
        web_fetch: WebFetchRuntimePolicy {
            enabled: true,
            allow_private_hosts: false,
            enforce_allowed_domains: true,
            allowed_domains: BTreeSet::new(),
            blocked_domains: BTreeSet::new(),
            timeout_seconds: 15,
            max_bytes: 8_192,
            max_redirects: 4,
        },
        ..ToolRuntimeConfig::default()
    };
    let narrowing = ToolRuntimeNarrowing {
        web_fetch: WebFetchRuntimeNarrowing {
            allowed_domains: BTreeSet::from(["docs.example.com".to_owned()]),
            ..WebFetchRuntimeNarrowing::default()
        },
        ..ToolRuntimeNarrowing::default()
    };

    let effective = base.narrowed(&narrowing);

    assert!(effective.web_fetch.enforce_allowed_domains);
    assert!(
        effective.web_fetch.allowed_domains.is_empty(),
        "an existing fail-closed allowlist should not be widened by later narrowing"
    );
}

#[test]
fn tool_runtime_narrowing_intersect_fail_closes_disjoint_allowlists() {
    let left = ToolRuntimeNarrowing {
        browser: BrowserRuntimeNarrowing {
            max_sessions: Some(1),
            ..BrowserRuntimeNarrowing::default()
        },
        web_fetch: WebFetchRuntimeNarrowing {
            allow_private_hosts: Some(false),
            enforce_allowed_domains: false,
            allowed_domains: BTreeSet::from(["docs.example.com".to_owned()]),
            blocked_domains: BTreeSet::from(["deny-left.example.com".to_owned()]),
            timeout_seconds: Some(5),
            max_bytes: Some(4_096),
            max_redirects: Some(2),
        },
    };
    let right = ToolRuntimeNarrowing {
        browser: BrowserRuntimeNarrowing {
            max_sessions: Some(3),
            ..BrowserRuntimeNarrowing::default()
        },
        web_fetch: WebFetchRuntimeNarrowing {
            allow_private_hosts: None,
            enforce_allowed_domains: false,
            allowed_domains: BTreeSet::from(["api.example.com".to_owned()]),
            blocked_domains: BTreeSet::from(["deny-right.example.com".to_owned()]),
            timeout_seconds: Some(9),
            max_bytes: Some(8_192),
            max_redirects: Some(4),
        },
    };

    let effective = left.intersect(&right);

    assert_eq!(effective.browser.max_sessions, Some(1));
    assert_eq!(effective.web_fetch.allow_private_hosts, Some(false));
    assert!(effective.web_fetch.enforce_allowed_domains);
    assert!(
        effective.web_fetch.allowed_domains.is_empty(),
        "disjoint allowlists should collapse to an enforced empty intersection"
    );
    assert_eq!(
        effective.web_fetch.blocked_domains,
        BTreeSet::from([
            "deny-left.example.com".to_owned(),
            "deny-right.example.com".to_owned(),
        ])
    );
    assert_eq!(effective.web_fetch.timeout_seconds, Some(5));
    assert_eq!(effective.web_fetch.max_bytes, Some(4_096));
    assert_eq!(effective.web_fetch.max_redirects, Some(2));
}

#[test]
fn tool_runtime_narrowing_intersection_is_idempotent_for_implicit_allowlist() {
    let narrowing = ToolRuntimeNarrowing {
        web_fetch: WebFetchRuntimeNarrowing {
            allowed_domains: BTreeSet::from(["docs.example.com".to_owned()]),
            ..WebFetchRuntimeNarrowing::default()
        },
        ..ToolRuntimeNarrowing::default()
    };

    assert_eq!(narrowing.intersect(&narrowing), narrowing);
}

#[test]
fn tool_runtime_narrowing_intersect_preserves_explicit_private_host_allowance() {
    let left = ToolRuntimeNarrowing {
        web_fetch: WebFetchRuntimeNarrowing {
            allow_private_hosts: Some(true),
            ..WebFetchRuntimeNarrowing::default()
        },
        ..ToolRuntimeNarrowing::default()
    };
    let right = left.clone();

    let effective = left.intersect(&right);

    assert_eq!(effective.web_fetch.allow_private_hosts, Some(true));
}

#[test]
fn tool_runtime_narrowing_intersect_preserves_single_explicit_private_host_allowance() {
    let left = ToolRuntimeNarrowing {
        web_fetch: WebFetchRuntimeNarrowing {
            allow_private_hosts: Some(true),
            ..WebFetchRuntimeNarrowing::default()
        },
        ..ToolRuntimeNarrowing::default()
    };
    let right = ToolRuntimeNarrowing {
        browser: BrowserRuntimeNarrowing {
            max_sessions: Some(1),
            ..BrowserRuntimeNarrowing::default()
        },
        ..ToolRuntimeNarrowing::default()
    };

    let effective = left.intersect(&right);

    assert_eq!(effective.web_fetch.allow_private_hosts, Some(true));
}

#[test]
fn merge_runtime_narrowing_sources_intersects_delegate_and_policy_inputs() {
    let delegate_runtime_narrowing = ToolRuntimeNarrowing {
        browser: BrowserRuntimeNarrowing {
            max_sessions: Some(1),
            ..BrowserRuntimeNarrowing::default()
        },
        web_fetch: WebFetchRuntimeNarrowing {
            allowed_domains: BTreeSet::from(["docs.example.com".to_owned()]),
            blocked_domains: BTreeSet::from(["deny-left.example.com".to_owned()]),
            ..WebFetchRuntimeNarrowing::default()
        },
    };
    let policy_runtime_narrowing = ToolRuntimeNarrowing {
        browser: BrowserRuntimeNarrowing {
            max_sessions: Some(3),
            ..BrowserRuntimeNarrowing::default()
        },
        web_fetch: WebFetchRuntimeNarrowing {
            allowed_domains: BTreeSet::from(["api.example.com".to_owned()]),
            blocked_domains: BTreeSet::from(["deny-right.example.com".to_owned()]),
            ..WebFetchRuntimeNarrowing::default()
        },
    };

    let effective_runtime_narrowing = merge_runtime_narrowing_sources(
        Some(delegate_runtime_narrowing),
        Some(policy_runtime_narrowing),
    )
    .expect("effective runtime narrowing");

    assert_eq!(effective_runtime_narrowing.browser.max_sessions, Some(1));
    assert!(
        effective_runtime_narrowing
            .web_fetch
            .enforce_allowed_domains
    );
    assert!(
        effective_runtime_narrowing
            .web_fetch
            .allowed_domains
            .is_empty()
    );
    assert_eq!(
        effective_runtime_narrowing.web_fetch.blocked_domains,
        BTreeSet::from([
            "deny-left.example.com".to_owned(),
            "deny-right.example.com".to_owned(),
        ])
    );
}

#[test]
fn merge_runtime_narrowing_sources_handles_empty_and_single_source_inputs() {
    let primary_runtime_narrowing = ToolRuntimeNarrowing {
        browser: BrowserRuntimeNarrowing {
            max_sessions: Some(2),
            ..BrowserRuntimeNarrowing::default()
        },
        ..ToolRuntimeNarrowing::default()
    };
    let empty_runtime_narrowing = ToolRuntimeNarrowing::default();

    let none_result = merge_runtime_narrowing_sources(None, None);
    let primary_only_result =
        merge_runtime_narrowing_sources(Some(primary_runtime_narrowing.clone()), None);
    let secondary_only_result =
        merge_runtime_narrowing_sources(None, Some(primary_runtime_narrowing.clone()));
    let empty_primary_result =
        merge_runtime_narrowing_sources(Some(empty_runtime_narrowing.clone()), None);
    let empty_primary_with_secondary_result = merge_runtime_narrowing_sources(
        Some(empty_runtime_narrowing),
        Some(primary_runtime_narrowing.clone()),
    );

    assert!(none_result.is_none());
    assert_eq!(primary_only_result, Some(primary_runtime_narrowing.clone()));
    assert_eq!(
        secondary_only_result,
        Some(primary_runtime_narrowing.clone())
    );
    assert!(empty_primary_result.is_none());
    assert_eq!(
        empty_primary_with_secondary_result,
        Some(primary_runtime_narrowing)
    );
}

#[cfg(feature = "feishu-integration")]
#[test]
fn from_env_enables_feishu_runtime_when_credentials_exist() {
    let mut env = ScopedEnv::new();
    clear_tool_runtime_env(&mut env);
    clear_feishu_runtime_env(&mut env);
    let _home = ScopedLoongHome::new("loong-feishu-runtime-home");
    env.set("FEISHU_APP_ID", "cli_env_a1b2c3");
    env.set("FEISHU_APP_SECRET", "env-secret");

    let config = ToolRuntimeConfig::from_env();
    let feishu = config
        .feishu
        .as_ref()
        .expect("feishu runtime should be enabled from env");

    assert!(feishu.channel.enabled);
    assert_eq!(feishu.channel.app_id_env.as_deref(), Some("FEISHU_APP_ID"));
    assert_eq!(
        feishu.channel.app_secret_env.as_deref(),
        Some("FEISHU_APP_SECRET")
    );
    assert_eq!(
        feishu.integration.resolved_sqlite_path(),
        crate::config::default_loong_home().join("feishu.sqlite3")
    );
}

#[cfg(feature = "feishu-integration")]
#[test]
fn from_loong_config_ignores_disabled_feishu_channel_even_when_root_credentials_exist() {
    let config = crate::config::LoongConfig {
        feishu: crate::config::FeishuChannelConfig {
            enabled: false,
            app_id: Some(loong_contracts::SecretRef::Inline(
                "cli_disabled_root".to_owned(),
            )),
            app_secret: Some(loong_contracts::SecretRef::Inline(
                "disabled-root-secret".to_owned(),
            )),
            ..crate::config::FeishuChannelConfig::default()
        },
        ..crate::config::LoongConfig::default()
    };

    assert!(
        FeishuToolRuntimeConfig::from_loong_config(&config).is_none(),
        "disabled Feishu channel should not expose Feishu tools through runtime config"
    );
}

#[cfg(feature = "feishu-integration")]
#[test]
fn from_loong_config_ignores_disabled_feishu_accounts_when_detecting_runtime() {
    let mut env = ScopedEnv::new();
    env.set("FEISHU_APP_ID", "cli_env_a1b2c3");
    env.set("FEISHU_APP_SECRET", "env-secret");

    let config = crate::config::LoongConfig {
        feishu: crate::config::FeishuChannelConfig {
            enabled: true,
            app_id_env: None,
            app_secret_env: None,
            accounts: BTreeMap::from([(
                "disabled_account".to_owned(),
                crate::config::FeishuAccountConfig {
                    enabled: Some(false),
                    app_id: Some(loong_contracts::SecretRef::Inline(
                        "cli_disabled_account".to_owned(),
                    )),
                    app_secret: Some(loong_contracts::SecretRef::Inline(
                        "disabled-account-secret".to_owned(),
                    )),
                    ..crate::config::FeishuAccountConfig::default()
                },
            )]),
            ..crate::config::FeishuChannelConfig::default()
        },
        ..crate::config::LoongConfig::default()
    };

    assert!(
        FeishuToolRuntimeConfig::from_loong_config(&config).is_none(),
        "disabled Feishu accounts should not enable Feishu tool runtime on their own"
    );
}

#[cfg(feature = "feishu-integration")]
#[test]
fn from_loong_config_requires_resolved_env_values_for_typed_feishu_secret_refs() {
    let mut env = ScopedEnv::new();
    clear_feishu_runtime_env(&mut env);

    let config = crate::config::LoongConfig {
        feishu: crate::config::FeishuChannelConfig {
            enabled: true,
            app_id: Some(loong_contracts::SecretRef::Env {
                env: "FEISHU_APP_ID".to_owned(),
            }),
            app_secret: Some(loong_contracts::SecretRef::Env {
                env: "FEISHU_APP_SECRET".to_owned(),
            }),
            app_id_env: None,
            app_secret_env: None,
            ..crate::config::FeishuChannelConfig::default()
        },
        ..crate::config::LoongConfig::default()
    };

    assert!(
        FeishuToolRuntimeConfig::from_loong_config(&config).is_none(),
        "missing env values should not enable Feishu runtime for typed env refs"
    );

    env.set("FEISHU_APP_ID", "cli_env_a1b2c3");
    env.set("FEISHU_APP_SECRET", "env-secret");

    assert!(
        FeishuToolRuntimeConfig::from_loong_config(&config).is_some(),
        "resolved env values should enable Feishu runtime for typed env refs"
    );
}
