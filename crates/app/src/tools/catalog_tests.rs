use super::*;
use tempfile::tempdir;

#[cfg(feature = "tool-browser")]
#[test]
fn browser_companion_visibility_surface_requires_runtime_readiness_for_all_companion_tools() {
    let catalog = tool_catalog();
    let expected = [
        ("browser.companion.session.start", ToolExecutionKind::Core),
        ("browser.companion.navigate", ToolExecutionKind::Core),
        ("browser.companion.snapshot", ToolExecutionKind::Core),
        ("browser.companion.wait", ToolExecutionKind::Core),
        ("browser.companion.session.stop", ToolExecutionKind::Core),
        ("browser.companion.click", ToolExecutionKind::App),
        ("browser.companion.type", ToolExecutionKind::App),
    ];

    let mut hidden = ToolRuntimeConfig::default();
    hidden.browser_companion.enabled = true;
    hidden.browser_companion.ready = false;
    hidden.browser_companion.command = Some("browser-companion".to_owned());
    let hidden_view = runtime_tool_view_for_runtime_config(&hidden);

    let mut visible = ToolRuntimeConfig::default();
    visible.browser_companion.enabled = true;
    visible.browser_companion.ready = true;
    visible.browser_companion.command = Some("browser-companion".to_owned());
    let visible_view = runtime_tool_view_for_runtime_config(&visible);

    for (tool_name, execution_kind) in expected {
        let descriptor = catalog
            .resolve(tool_name)
            .unwrap_or_else(|| panic!("missing browser companion descriptor `{tool_name}`"));
        assert_eq!(
            descriptor.visibility_gate,
            ToolVisibilityGate::BrowserCompanion
        );
        assert_eq!(descriptor.execution_kind, execution_kind);
        assert!(
            !hidden_view.contains(tool_name),
            "tool should stay hidden until runtime-ready: {tool_name}"
        );
        assert!(
            visible_view.contains(tool_name),
            "tool should appear once runtime-ready: {tool_name}"
        );
    }
}

#[test]
fn browser_companion_visibility_gate_requires_runtime_readiness() {
    let mut config = ToolRuntimeConfig::default();
    config.browser_companion.enabled = true;
    config.browser_companion.ready = false;
    config.browser_companion.command = Some("browser-companion".to_owned());

    assert!(!tool_visibility_gate_enabled_for_runtime_policy(
        ToolVisibilityGate::BrowserCompanion,
        &config
    ));

    config.browser_companion.ready = true;

    assert!(tool_visibility_gate_enabled_for_runtime_policy(
        ToolVisibilityGate::BrowserCompanion,
        &config
    ));
}

#[test]
fn browser_companion_visibility_gate_stays_hidden_for_config_only_views() {
    let mut config = ToolConfig::default();
    config.browser_companion.enabled = true;

    assert!(!tool_visibility_gate_enabled_for_runtime_view(
        ToolVisibilityGate::BrowserCompanion,
        &config,
        false
    ));
}

#[test]
fn memory_file_root_visibility_gate_requires_safe_root_configuration() {
    let hidden_config = ToolConfig::default();
    assert!(!tool_visibility_gate_enabled_for_runtime_view(
        ToolVisibilityGate::MemoryFileRoot,
        &hidden_config,
        false
    ));

    let visible_config = ToolConfig {
        file_root: Some("/tmp/workspace".to_owned()),
        ..ToolConfig::default()
    };
    assert!(tool_visibility_gate_enabled_for_runtime_view(
        ToolVisibilityGate::MemoryFileRoot,
        &visible_config,
        false
    ));

    let hidden_runtime = ToolRuntimeConfig::default();
    assert!(!tool_visibility_gate_enabled_for_runtime_policy(
        ToolVisibilityGate::MemoryFileRoot,
        &hidden_runtime
    ));

    let empty_runtime_dir = tempdir().expect("tempdir");
    let empty_runtime = ToolRuntimeConfig {
        file_root: Some(empty_runtime_dir.path().to_path_buf()),
        ..ToolRuntimeConfig::default()
    };
    assert!(!tool_visibility_gate_enabled_for_runtime_policy(
        ToolVisibilityGate::MemoryFileRoot,
        &empty_runtime
    ));

    let visible_runtime_dir = tempdir().expect("tempdir");
    let visible_memory_path = visible_runtime_dir.path().join("MEMORY.md");
    std::fs::write(
        &visible_memory_path,
        "# Durable Notes\nDeploy freeze window is Friday.\n",
    )
    .expect("write root memory");

    let visible_runtime = ToolRuntimeConfig {
        file_root: Some(visible_runtime_dir.path().to_path_buf()),
        ..ToolRuntimeConfig::default()
    };
    assert!(tool_visibility_gate_enabled_for_runtime_policy(
        ToolVisibilityGate::MemoryFileRoot,
        &visible_runtime
    ));
}

#[test]
fn memory_file_root_visibility_gate_rejects_whitespace_only_paths() {
    let view_config = ToolConfig {
        file_root: Some("   ".to_owned()),
        ..ToolConfig::default()
    };
    let runtime_config = ToolRuntimeConfig {
        file_root: Some(std::path::PathBuf::from("   ")),
        ..ToolRuntimeConfig::default()
    };

    assert!(!tool_visibility_gate_enabled_for_runtime_view(
        ToolVisibilityGate::MemoryFileRoot,
        &view_config,
        false
    ));
    assert!(!tool_visibility_gate_enabled_for_runtime_policy(
        ToolVisibilityGate::MemoryFileRoot,
        &runtime_config
    ));
    assert!(!tool_visibility_gate_enabled_for_runtime_policy(
        ToolVisibilityGate::MemorySearchCorpus,
        &runtime_config
    ));
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn memory_search_corpus_visibility_gate_allows_canonical_memory_without_workspace_files() {
    let runtime_dir = tempdir().expect("tempdir");
    let db_path = runtime_dir.path().join("memory.sqlite3");
    let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path.clone()),
        ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
    };
    crate::memory::append_turn_direct(
        "canonical-search-gate-session",
        "assistant",
        "Rollback checklist includes smoke tests and release notes.",
        &memory_config,
    )
    .expect("append canonical turn");

    let runtime = ToolRuntimeConfig {
        file_root: None,
        memory_sqlite_path: Some(db_path),
        ..ToolRuntimeConfig::default()
    };
    assert!(tool_visibility_gate_enabled_for_runtime_policy(
        ToolVisibilityGate::MemorySearchCorpus,
        &runtime
    ));
    assert!(!tool_visibility_gate_enabled_for_runtime_policy(
        ToolVisibilityGate::MemoryFileRoot,
        &runtime
    ));
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn runtime_tool_view_includes_memory_search_for_canonical_memory_without_workspace_files() {
    let runtime_dir = tempdir().expect("tempdir");
    let db_path = runtime_dir.path().join("memory.sqlite3");
    let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path.clone()),
        ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
    };
    crate::memory::append_turn_direct(
        "canonical-view-session",
        "assistant",
        "Rollback checklist includes smoke tests and release notes.",
        &memory_config,
    )
    .expect("append canonical turn");

    let runtime = ToolRuntimeConfig {
        file_root: None,
        memory_sqlite_path: Some(db_path),
        ..ToolRuntimeConfig::default()
    };
    let tool_view = runtime_tool_view_for_runtime_config(&runtime);

    assert!(tool_view.contains("memory_search"));
    assert!(!tool_view.contains("memory_get"));
}

#[test]
fn browser_visibility_gate_is_independent_from_companion_settings() {
    let mut config = ToolRuntimeConfig::default();
    config.browser.enabled = true;
    config.browser_companion.enabled = false;
    config.browser_companion.ready = false;

    assert!(tool_visibility_gate_enabled_for_runtime_policy(
        ToolVisibilityGate::Browser,
        &config
    ));
}

#[cfg(feature = "feishu-integration")]
#[test]
fn feishu_visibility_gate_requires_runtime_configuration() {
    let hidden_runtime = ToolRuntimeConfig::default();
    let hidden_view = runtime_tool_view_for_runtime_config(&hidden_runtime);

    assert!(!tool_visibility_gate_enabled_for_runtime_policy(
        ToolVisibilityGate::Feishu,
        &hidden_runtime
    ));
    assert!(!hidden_view.contains("feishu.card.update"));

    let visible_runtime = ToolRuntimeConfig {
        feishu: Some(crate::tools::runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                app_id: Some(loong_contracts::SecretRef::Inline("cli_a1b2c3".to_owned())),
                app_secret: Some(loong_contracts::SecretRef::Inline("app-secret".to_owned())),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig::default(),
        }),
        ..ToolRuntimeConfig::default()
    };
    let visible_view = runtime_tool_view_for_runtime_config(&visible_runtime);
    let descriptor = tool_catalog()
        .resolve("feishu_card_update")
        .expect("feishu card update descriptor");

    assert!(tool_visibility_gate_enabled_for_runtime_policy(
        ToolVisibilityGate::Feishu,
        &visible_runtime
    ));
    assert_eq!(descriptor.name, "feishu.card.update");
    assert_eq!(descriptor.visibility_gate, ToolVisibilityGate::Feishu);
    assert!(visible_view.contains("feishu.card.update"));
}

#[test]
fn delegate_child_tool_view_respects_visibility_gates() {
    let mut config = ToolConfig::default();
    config.web.enabled = false;
    config.delegate.child_tool_allowlist = vec!["web.fetch".to_owned()];

    let child_view = delegate_child_tool_view_for_config(&config);

    assert!(!child_view.contains("web.fetch"));
}

#[test]
fn delegate_child_tool_view_for_contract_fails_closed_without_profile() {
    let config = ToolConfig::default();
    let child_view = delegate_child_tool_view_for_contract(&config, None);

    assert!(!child_view.contains("delegate"));
    assert!(!child_view.contains("delegate_async"));
}

#[test]
fn delegate_child_tool_view_for_contract_allows_nested_delegate_when_profile_permits() {
    let config = ToolConfig::default();
    let contract = ConstrainedSubagentContractView::from_profile(ConstrainedSubagentProfile {
        role: crate::conversation::ConstrainedSubagentRole::Orchestrator,
        control_scope: crate::conversation::ConstrainedSubagentControlScope::Children,
    });
    let child_view = delegate_child_tool_view_for_contract(&config, Some(&contract));

    assert!(child_view.contains("delegate"));
    assert!(child_view.contains("delegate_async"));
}

#[cfg(feature = "tool-shell")]
#[test]
fn delegate_child_tool_view_hides_allowlisted_bash_exec_without_runtime_visibility() {
    let mut config = ToolConfig::default();
    config.delegate.child_tool_allowlist = vec!["bash.exec".to_owned()];

    let child_view = delegate_child_tool_view_for_config(&config);

    assert!(!child_view.contains("bash.exec"));
}

#[cfg(feature = "tool-shell")]
#[test]
fn bash_runtime_visibility_gate_hides_bash_exec_when_governance_rules_failed_to_load() {
    let runtime = ToolRuntimeConfig {
        bash_exec: crate::tools::runtime_config::BashExecRuntimePolicy {
            available: true,
            command: Some(std::path::PathBuf::from("bash")),
            governance: crate::tools::runtime_config::BashGovernanceRuntimePolicy {
                load_error: Some("broken rules".to_owned()),
                ..crate::tools::runtime_config::BashGovernanceRuntimePolicy::default()
            },
            ..crate::tools::runtime_config::BashExecRuntimePolicy::default()
        },
        ..ToolRuntimeConfig::default()
    };

    assert!(!tool_visibility_gate_enabled_for_runtime_policy(
        ToolVisibilityGate::BashRuntime,
        &runtime
    ));
    assert!(!runtime_tool_view_for_runtime_config(&runtime).contains("bash.exec"));
}

#[cfg(feature = "tool-shell")]
#[test]
fn delegate_child_tool_view_hides_allowlisted_bash_exec_when_governance_rules_failed_to_load() {
    let mut config = ToolConfig::default();
    config.delegate.child_tool_allowlist = vec!["bash.exec".to_owned()];
    let runtime = ToolRuntimeConfig {
        bash_exec: crate::tools::runtime_config::BashExecRuntimePolicy {
            available: true,
            command: Some(std::path::PathBuf::from("bash")),
            governance: crate::tools::runtime_config::BashGovernanceRuntimePolicy {
                load_error: Some("broken rules".to_owned()),
                ..crate::tools::runtime_config::BashGovernanceRuntimePolicy::default()
            },
            ..crate::tools::runtime_config::BashExecRuntimePolicy::default()
        },
        ..ToolRuntimeConfig::default()
    };

    let child_view = delegate_child_tool_view_for_runtime_config(&config, &runtime);

    assert!(!child_view.contains("bash.exec"));
}

#[cfg(feature = "tool-shell")]
#[test]
fn delegate_child_tool_view_exposes_allowlisted_bash_exec_when_runtime_ready() {
    let mut config = ToolConfig::default();
    config.delegate.child_tool_allowlist = vec!["bash.exec".to_owned()];
    let runtime = ToolRuntimeConfig {
        bash_exec: crate::tools::runtime_config::BashExecRuntimePolicy {
            available: true,
            command: Some(std::path::PathBuf::from("bash")),
            ..crate::tools::runtime_config::BashExecRuntimePolicy::default()
        },
        ..ToolRuntimeConfig::default()
    };

    let child_view = delegate_child_tool_view_for_runtime_config(&config, &runtime);

    assert!(child_view.contains("bash.exec"));
}

#[test]
fn scheduling_class_marks_parallel_safe_subset() {
    let catalog = tool_catalog();
    assert_eq!(
        catalog
            .descriptor("tool.search")
            .expect("tool.search descriptor")
            .scheduling_class(),
        ToolSchedulingClass::ParallelSafe
    );
    #[cfg(feature = "tool-file")]
    assert_eq!(
        catalog
            .descriptor("file.read")
            .expect("file.read descriptor")
            .scheduling_class(),
        ToolSchedulingClass::ParallelSafe
    );
    #[cfg(feature = "tool-file")]
    assert_eq!(
        catalog
            .descriptor("memory_search")
            .expect("memory_search descriptor")
            .scheduling_class(),
        ToolSchedulingClass::ParallelSafe
    );
    #[cfg(feature = "tool-file")]
    assert_eq!(
        catalog
            .descriptor("memory_get")
            .expect("memory_get descriptor")
            .scheduling_class(),
        ToolSchedulingClass::ParallelSafe
    );
    #[cfg(feature = "tool-webfetch")]
    assert_eq!(
        catalog
            .descriptor("web.fetch")
            .expect("web.fetch descriptor")
            .scheduling_class(),
        ToolSchedulingClass::ParallelSafe
    );
    assert_eq!(
        catalog
            .descriptor("sessions_list")
            .expect("sessions_list descriptor")
            .scheduling_class(),
        ToolSchedulingClass::ParallelSafe
    );
    assert_eq!(
        catalog
            .descriptor("session_search")
            .expect("session_search descriptor")
            .scheduling_class(),
        ToolSchedulingClass::ParallelSafe
    );
    assert_eq!(
        catalog
            .descriptor("delegate_async")
            .expect("delegate_async descriptor")
            .scheduling_class(),
        ToolSchedulingClass::SerialOnly
    );
}

#[test]
fn tool_catalog_entries_expose_concurrency_class() {
    let search = find_tool_catalog_entry("tool.search").expect("tool.search catalog entry");
    assert_eq!(search.scheduling_class, ToolSchedulingClass::ParallelSafe);
    assert_eq!(search.concurrency_class, ToolConcurrencyClass::ReadOnly);

    let invoke = find_tool_catalog_entry("tool.invoke").expect("tool.invoke catalog entry");
    assert_eq!(invoke.scheduling_class, ToolSchedulingClass::SerialOnly);
    assert_eq!(invoke.concurrency_class, ToolConcurrencyClass::Unknown);

    let delegate_async =
        find_tool_catalog_entry("delegate_async").expect("delegate_async catalog entry");
    assert_eq!(
        delegate_async.scheduling_class,
        ToolSchedulingClass::SerialOnly
    );
    assert_eq!(
        delegate_async.concurrency_class,
        ToolConcurrencyClass::Mutating
    );

    #[cfg(feature = "tool-http")]
    {
        let http_request =
            find_tool_catalog_entry("http.request").expect("http.request catalog entry");
        assert_eq!(
            http_request.scheduling_class,
            ToolSchedulingClass::SerialOnly
        );
        assert_eq!(
            http_request.concurrency_class,
            ToolConcurrencyClass::Mutating
        );
    }

    let file_write = find_tool_catalog_entry("file.write").expect("file.write catalog entry");
    assert_eq!(file_write.scheduling_class, ToolSchedulingClass::SerialOnly);
    assert_eq!(file_write.concurrency_class, ToolConcurrencyClass::Mutating);
    assert_eq!(file_write.surface_id, Some("write"));
    assert!(
        file_write
            .usage_guidance
            .is_some_and(|guidance| guidance.contains("normal patching and file creation"))
    );

    let bash_exec = find_tool_catalog_entry("bash.exec").expect("bash.exec catalog entry");
    assert_eq!(bash_exec.scheduling_class, ToolSchedulingClass::SerialOnly);
    assert_eq!(bash_exec.concurrency_class, ToolConcurrencyClass::Mutating);
    assert_eq!(bash_exec.surface_id, Some("exec"));
}

#[test]
fn tool_catalog_resolve_preserves_canonical_provider_and_alias_lookup() {
    let catalog = tool_catalog();

    let canonical = catalog.resolve("tool.search").expect("canonical lookup");
    let provider_name = catalog.resolve("tool_search").expect("provider lookup");
    let alias = catalog.resolve("shell").expect("alias lookup");

    assert_eq!(canonical.name, "tool.search");
    assert_eq!(provider_name.name, "tool.search");
    assert_eq!(alias.name, "shell.exec");
}

#[test]
fn cached_catalog_entry_partitions_match_descriptor_filters() {
    let catalog = tool_catalog();

    let expected_all_entries = descriptor_identity_list(catalog.descriptors().iter());
    let expected_provider_exposed_entries = descriptor_identity_list(
        catalog
            .descriptors()
            .iter()
            .filter(|descriptor| descriptor.is_provider_exposed()),
    );

    let actual_all_entries = entry_identity_list(all_tool_catalog().iter());
    let actual_provider_exposed_entries =
        entry_identity_list(provider_exposed_tool_catalog().iter());

    assert_eq!(actual_all_entries, expected_all_entries);
    assert_eq!(
        actual_provider_exposed_entries,
        expected_provider_exposed_entries
    );
}

fn descriptor_identity_list<'a>(
    descriptors: impl Iterator<Item = &'a ToolDescriptor>,
) -> Vec<(&'static str, &'static str, ToolExposureClass)> {
    let mut identities = Vec::new();

    for descriptor in descriptors {
        let identity = (
            descriptor.name,
            descriptor.provider_name,
            descriptor.exposure,
        );
        identities.push(identity);
    }

    identities
}

fn entry_identity_list<'a>(
    entries: impl Iterator<Item = &'a ToolCatalogEntry>,
) -> Vec<(&'static str, &'static str, ToolExposureClass)> {
    let mut identities = Vec::new();

    for entry in entries {
        let identity = (
            entry.canonical_name,
            entry.provider_function_name,
            entry.exposure,
        );
        identities.push(identity);
    }

    identities
}

#[cfg(feature = "feishu-integration")]
#[test]
fn feishu_tool_catalog_entries_expose_explicit_concurrency_class() {
    let catalog = tool_catalog();
    let feishu_descriptors: Vec<&ToolDescriptor> = catalog
        .descriptors()
        .iter()
        .filter(|descriptor| descriptor.name.starts_with("feishu."))
        .collect();

    assert!(!feishu_descriptors.is_empty());

    for descriptor in feishu_descriptors {
        assert_ne!(
            descriptor.concurrency_class(),
            ToolConcurrencyClass::Unknown,
            "{} should expose an explicit concurrency class",
            descriptor.name
        );
    }

    let calendar_list =
        find_tool_catalog_entry("feishu.calendar.list").expect("feishu.calendar.list entry");
    assert_eq!(
        calendar_list.concurrency_class,
        ToolConcurrencyClass::ReadOnly
    );

    let messages_send =
        find_tool_catalog_entry("feishu.messages.send").expect("feishu.messages.send entry");
    assert_eq!(
        messages_send.concurrency_class,
        ToolConcurrencyClass::Mutating
    );
}

#[cfg(all(feature = "feishu-integration", feature = "tool-file"))]
#[test]
fn feishu_resource_download_catalog_entry_is_mutating() {
    let entry = find_tool_catalog_entry("feishu.messages.resource.get")
        .expect("feishu.messages.resource.get entry");

    assert_eq!(entry.concurrency_class, ToolConcurrencyClass::Mutating);
}

#[test]
fn governance_profile_follows_descriptor_declared_policy() {
    let catalog = tool_catalog();

    let delegate_async = catalog
        .descriptor("delegate_async")
        .expect("delegate_async descriptor");
    let delegate_async_policy = governance_profile_for_descriptor(delegate_async);

    assert_eq!(
        delegate_async_policy.scope,
        ToolGovernanceScope::TopologyMutation
    );
    assert_eq!(delegate_async_policy.risk_class, ToolRiskClass::High);
    assert_eq!(
        delegate_async_policy.approval_mode,
        ToolApprovalMode::PolicyDriven
    );

    let sessions_send_policy = governance_profile_for_tool_name("sessions_send");

    assert_eq!(sessions_send_policy.scope, ToolGovernanceScope::Routine);
    assert_eq!(sessions_send_policy.risk_class, ToolRiskClass::Elevated);
    assert_eq!(
        sessions_send_policy.approval_mode,
        ToolApprovalMode::PolicyDriven
    );

    let external_skills_policy = governance_profile_for_tool_name("external_skills.policy");

    assert_eq!(external_skills_policy.scope, ToolGovernanceScope::Routine);
    assert_eq!(external_skills_policy.risk_class, ToolRiskClass::High);
    assert_eq!(
        external_skills_policy.approval_mode,
        ToolApprovalMode::PolicyDriven
    );

    let unknown_policy = governance_profile_for_tool_name("unknown.tool");

    assert_eq!(unknown_policy, FAIL_CLOSED_GOVERNANCE_PROFILE);
}

#[cfg(feature = "tool-browser")]
#[test]
fn governance_profile_resolves_alias_backed_tool_metadata() {
    let policy = governance_profile_for_tool_name("browser_companion_click");

    assert_eq!(policy.scope, ToolGovernanceScope::Routine);
    assert_eq!(policy.risk_class, ToolRiskClass::High);
    assert_eq!(policy.approval_mode, ToolApprovalMode::PolicyDriven);
}

#[cfg(feature = "tool-shell")]
#[test]
fn governance_profile_resolves_alias_distinct_from_provider_name() {
    let catalog = tool_catalog();
    let descriptor = catalog
        .descriptor("shell.exec")
        .expect("shell.exec descriptor");
    let expected_policy = governance_profile_for_descriptor(descriptor);
    let alias_policy = governance_profile_for_tool_name("shell");

    assert_ne!(descriptor.provider_name, "shell");
    assert!(descriptor.aliases.contains(&"shell"));
    assert_eq!(alias_policy, expected_policy);
}

#[cfg(feature = "tool-shell")]
#[test]
fn bash_exec_uses_high_risk_governance_profile() {
    let policy = governance_profile_for_tool_name("bash.exec");

    assert_eq!(policy.scope, ToolGovernanceScope::Routine);
    assert_eq!(policy.risk_class, ToolRiskClass::High);
    assert_eq!(policy.approval_mode, ToolApprovalMode::PolicyDriven);
}

#[test]
fn config_import_alias_resolves_descriptor_governance() {
    let catalog = tool_catalog();
    let descriptor = catalog
        .descriptor("config.import")
        .expect("config.import descriptor");
    let expected_policy = governance_profile_for_descriptor(descriptor);
    let legacy_alias_policy = governance_profile_for_tool_name("claw.migrate");

    assert!(descriptor.aliases.contains(&"claw.migrate"));
    assert!(descriptor.aliases.contains(&"claw_migrate"));
    assert_eq!(legacy_alias_policy, expected_policy);
}

#[test]
fn autonomy_capability_action_is_independent_from_governance_profile() {
    let catalog = tool_catalog();
    let migrate = catalog
        .descriptor("config.import")
        .expect("config.import descriptor");
    let provider_switch = catalog
        .descriptor("provider.switch")
        .expect("provider.switch descriptor");
    let migrate_policy = governance_profile_for_descriptor(migrate);
    let provider_switch_policy = governance_profile_for_descriptor(provider_switch);

    assert_eq!(migrate_policy, provider_switch_policy);
    assert_eq!(
        migrate.scheduling_class(),
        provider_switch.scheduling_class()
    );
    assert_eq!(
        capability_action_class_for_descriptor(migrate),
        CapabilityActionClass::ExecuteExisting
    );
    assert_eq!(
        capability_action_class_for_descriptor(provider_switch),
        CapabilityActionClass::RuntimeSwitch
    );
    assert_ne!(
        migrate.capability_action_class(),
        provider_switch.capability_action_class()
    );
}

#[test]
fn autonomy_capability_action_classifies_representative_tool_families() {
    let expectations = [
        ("tool.search", CapabilityActionClass::Discover),
        ("tool_search", CapabilityActionClass::Discover),
        ("tool.invoke", CapabilityActionClass::ExecuteExisting),
        ("config.import", CapabilityActionClass::ExecuteExisting),
        (
            "external_skills.fetch",
            CapabilityActionClass::CapabilityFetch,
        ),
        (
            "external_skills.install",
            CapabilityActionClass::CapabilityInstall,
        ),
        (
            "external_skills.invoke",
            CapabilityActionClass::CapabilityLoad,
        ),
        ("provider.switch", CapabilityActionClass::RuntimeSwitch),
        ("delegate", CapabilityActionClass::TopologyExpand),
        ("delegate_async", CapabilityActionClass::TopologyExpand),
        (
            "approval_request_resolve",
            CapabilityActionClass::ExecuteExisting,
        ),
        (
            "external_skills.policy",
            CapabilityActionClass::PolicyMutation,
        ),
        ("session_archive", CapabilityActionClass::SessionMutation),
        ("session_cancel", CapabilityActionClass::SessionMutation),
        ("session_continue", CapabilityActionClass::SessionMutation),
        ("session_events", CapabilityActionClass::ExecuteExisting),
        (
            "session_tool_policy_status",
            CapabilityActionClass::ExecuteExisting,
        ),
        (
            "session_tool_policy_set",
            CapabilityActionClass::PolicyMutation,
        ),
        (
            "session_tool_policy_clear",
            CapabilityActionClass::PolicyMutation,
        ),
        ("session_search", CapabilityActionClass::ExecuteExisting),
        ("task_status", CapabilityActionClass::ExecuteExisting),
        ("task_wait", CapabilityActionClass::ExecuteExisting),
        ("task_history", CapabilityActionClass::ExecuteExisting),
        ("tasks_list", CapabilityActionClass::ExecuteExisting),
        ("tasks_search", CapabilityActionClass::ExecuteExisting),
        ("session_recover", CapabilityActionClass::SessionMutation),
    ];

    for (tool_name, expected_action_class) in expectations {
        let resolved_action_class = capability_action_class_for_tool_name(tool_name)
            .unwrap_or_else(|| panic!("missing action class for `{tool_name}`"));

        assert_eq!(resolved_action_class, expected_action_class);
    }
}

#[test]
fn autonomy_capability_action_catalog_entries_expose_serializable_metadata() {
    let delegate_async =
        find_tool_catalog_entry("delegate_async").expect("delegate_async catalog entry");
    let delegate_async_value =
        serde_json::to_value(delegate_async).expect("serialize delegate_async catalog entry");
    let search = find_tool_catalog_entry("tool.search").expect("tool.search catalog entry");
    let search_value = serde_json::to_value(search).expect("serialize tool.search catalog entry");
    let invoke = find_tool_catalog_entry("tool.invoke").expect("tool.invoke catalog entry");
    let invoke_value = serde_json::to_value(invoke).expect("serialize tool.invoke catalog entry");

    assert_eq!(
        delegate_async.capability_action_class,
        CapabilityActionClass::TopologyExpand
    );
    assert_eq!(
        delegate_async_value["capability_action_class"],
        "topology_expand"
    );
    assert_eq!(delegate_async_value["concurrency_class"], "mutating");
    assert_eq!(search_value["concurrency_class"], "read_only");
    assert_eq!(invoke_value["concurrency_class"], "unknown");
}

#[test]
fn autonomy_capability_action_returns_none_for_unknown_tools() {
    let action_class = capability_action_class_for_tool_name("unknown.tool");

    assert_eq!(action_class, None);
}

#[test]
fn tool_catalog_lookup_tokens_are_globally_unambiguous() {
    let catalog = tool_catalog();
    let mut token_owners = std::collections::BTreeMap::new();

    for descriptor in catalog.descriptors() {
        let owner = descriptor.name;
        let mut lookup_tokens = BTreeSet::new();

        lookup_tokens.insert(descriptor.name);
        lookup_tokens.insert(descriptor.provider_name);

        for alias in descriptor.aliases {
            lookup_tokens.insert(*alias);
        }

        for token in lookup_tokens {
            let previous_owner = token_owners.insert(token, owner);

            if let Some(previous_owner) = previous_owner {
                assert_eq!(
                    previous_owner, owner,
                    "lookup token `{token}` resolves to both `{previous_owner}` and `{owner}`"
                );
            }
        }
    }
}

#[test]
fn sessions_send_definition_mentions_generic_channel_sessions() {
    let catalog = tool_catalog();
    let descriptor = catalog
        .descriptor("sessions_send")
        .expect("sessions_send descriptor");
    let definition = descriptor.provider_definition();
    let description =
        definition["function"]["parameters"]["properties"]["session_id"]["description"]
            .as_str()
            .expect("session_id description");

    assert!(description.contains("channel-backed"));
    assert!(description.contains("Matrix"));
}

#[test]
fn session_tool_policy_set_definition_surfaces_runtime_narrowing_shape() {
    let descriptor = tool_catalog()
        .descriptor("session_tool_policy_set")
        .expect("session_tool_policy_set descriptor");
    let definition = descriptor.provider_definition();
    let runtime_narrowing =
        &definition["function"]["parameters"]["properties"]["runtime_narrowing"];

    assert_eq!(runtime_narrowing["type"], "object");
    assert!(runtime_narrowing["properties"]["browser"]["properties"]["max_sessions"].is_object());
    assert!(
        runtime_narrowing["properties"]["web_fetch"]["properties"]["allowed_domains"].is_object()
    );
}

#[test]
fn delegate_definitions_surface_shared_and_worktree_isolation_modes() {
    let catalog = tool_catalog();

    for tool_name in ["delegate", "delegate_async"] {
        let descriptor = catalog.descriptor(tool_name).expect("delegate descriptor");
        let definition = descriptor.provider_definition();
        let isolation = &definition["function"]["parameters"]["properties"]["isolation"]["enum"];

        assert_eq!(*isolation, json!(["shared", "worktree"]));
    }
}

#[test]
fn external_skills_policy_definition_surfaces_update_controls() {
    let descriptor = tool_catalog()
        .descriptor("external_skills.policy")
        .expect("external_skills.policy descriptor");
    let definition = descriptor.provider_definition();
    let properties = &definition["function"]["parameters"]["properties"];

    assert_eq!(properties["action"]["enum"], json!(["get", "set", "reset"]));
    assert!(properties["policy_update_approved"].is_object());
    assert!(properties["allowed_domains"].is_object());
    assert!(properties["blocked_domains"].is_object());
}

#[cfg(feature = "tool-websearch")]
#[test]
fn web_search_definition_requires_query_and_exposes_provider_override() {
    let descriptor = tool_catalog()
        .descriptor("web.search")
        .expect("web.search descriptor");
    let definition = descriptor.provider_definition();
    let parameters = &definition["function"]["parameters"];

    assert_eq!(parameters["required"], json!(["query"]));
    assert!(parameters["properties"]["provider"]["enum"].is_array());
    assert!(parameters["properties"]["max_results"].is_object());
}

#[cfg(feature = "tool-browser")]
#[test]
fn browser_companion_type_definition_requires_session_selector_and_text() {
    let descriptor = tool_catalog()
        .descriptor("browser.companion.type")
        .expect("browser.companion.type descriptor");
    let definition = descriptor.provider_definition();
    let required = &definition["function"]["parameters"]["required"];

    assert_eq!(required, &json!(["session_id", "selector", "text"]));
}

#[cfg(feature = "feishu-integration")]
#[test]
fn feishu_bitable_record_search_catalog_metadata_includes_automatic_fields() {
    let descriptor = tool_catalog()
        .descriptor("feishu.bitable.record.search")
        .expect("feishu bitable record search descriptor");

    assert!(
        descriptor
            .argument_hint()
            .contains("automatic_fields?:boolean")
    );
    assert!(
        descriptor
            .parameter_types()
            .contains(&("automatic_fields", "boolean"))
    );
}

#[test]
fn sessions_list_definition_and_hint_surface_offset_pagination() {
    let catalog = tool_catalog();
    let descriptor = catalog
        .descriptor("sessions_list")
        .expect("sessions_list descriptor");
    let definition = descriptor.provider_definition();
    let function_definition = &definition["function"];
    let parameter_definition = &function_definition["parameters"];
    let property_definition = &parameter_definition["properties"];
    let offset_definition = &property_definition["offset"];
    let offset_description_value = &offset_definition["description"];
    let offset_description = offset_description_value
        .as_str()
        .expect("offset description");
    let parameter_types = descriptor.parameter_types();
    let has_offset_parameter = parameter_types.contains(&("offset", "integer"));

    assert!(offset_description.contains("skip"));
    assert_eq!(
        descriptor.argument_hint(),
        "limit?:integer,offset?:integer,state?:string"
    );
    assert!(has_offset_parameter);
}

#[test]
fn read_definitions_surface_line_window_fields() {
    let catalog = tool_catalog();
    let direct_descriptor = catalog.descriptor("read").expect("read descriptor");
    let direct_definition = direct_descriptor.provider_definition();
    let direct_properties = &direct_definition["function"]["parameters"]["properties"];
    let direct_parameter_types = direct_descriptor.parameter_types();

    assert!(direct_properties.get("offset").is_some());
    assert!(direct_properties.get("limit").is_some());
    assert!(
        direct_descriptor
            .argument_hint()
            .contains("offset?:integer")
    );
    assert!(direct_descriptor.argument_hint().contains("limit?:integer"));
    assert!(direct_parameter_types.contains(&("offset", "integer")));
    assert!(direct_parameter_types.contains(&("limit", "integer")));

    let file_descriptor = catalog
        .descriptor("file.read")
        .expect("file.read descriptor");
    let file_definition = file_descriptor.provider_definition();
    let file_properties = &file_definition["function"]["parameters"]["properties"];
    let file_parameter_types = file_descriptor.parameter_types();

    assert!(file_properties.get("offset").is_some());
    assert!(file_properties.get("limit").is_some());
    assert!(file_descriptor.argument_hint().contains("offset?:integer"));
    assert!(file_descriptor.argument_hint().contains("limit?:integer"));
    assert!(file_parameter_types.contains(&("offset", "integer")));
    assert!(file_parameter_types.contains(&("limit", "integer")));
}

#[test]
fn exec_definition_supports_script_mode() {
    let catalog = tool_catalog();
    let descriptor = catalog.descriptor("exec").expect("exec descriptor");
    let definition = descriptor.provider_definition();
    let properties = &definition["function"]["parameters"]["properties"];
    let any_of = &definition["function"]["parameters"]["anyOf"];

    assert!(properties.get("script").is_some());
    assert!(descriptor.argument_hint().contains("script?:string"));
    assert!(descriptor.parameter_types().contains(&("script", "string")));
    assert_eq!(descriptor.required_fields(), Vec::<&str>::new());
    assert!(any_of.is_array());
}
