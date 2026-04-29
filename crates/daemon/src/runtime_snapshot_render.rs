use serde_json::{Value, json};

use crate::{
    RuntimeSnapshotCliState, RuntimeSnapshotExternalSkillsState,
    RuntimeSnapshotProviderProfileState, RuntimeSnapshotProviderState,
    RuntimeSnapshotRuntimePluginsState, acp_backend_metadata_json, acp_control_plane_json,
    context_engine_metadata_json, format_capability_names, memory_system_metadata_json,
    memory_system_policy_json, mvp, push_channel_surface_managed_plugin_bridge_discovery,
    render_string_list,
};

pub fn render_runtime_snapshot_text(snapshot: &RuntimeSnapshotCliState) -> String {
    let mut lines = vec![
        format!("config={}", snapshot.config),
        format!(
            "provider active_profile={} active_label=\"{}\" last_provider={}",
            snapshot.provider.active_profile_id,
            snapshot.provider.active_label,
            snapshot.provider.last_provider_id.as_deref().unwrap_or("-")
        ),
        format!(
            "provider saved_profiles={}",
            render_string_list(
                snapshot
                    .provider
                    .saved_profile_ids
                    .iter()
                    .map(String::as_str)
            )
        ),
        format!(
            "provider transport cache_entries={} cache_hits={} cache_misses={} built_clients={}",
            snapshot
                .provider
                .transport_runtime
                .http_client_cache_entries,
            snapshot.provider.transport_runtime.http_client_cache_hits,
            snapshot.provider.transport_runtime.http_client_cache_misses,
            snapshot.provider.transport_runtime.built_http_clients
        ),
    ];

    for profile in &snapshot.provider.profiles {
        lines.push(format!(
            "  profile {} active={} default_for_kind={} kind={} model={} wire_api={} credential_resolved={} auth_env={} endpoint={} models_endpoint={} temperature={} max_tokens={} timeout_ms={} retries={} headers={} preferred_models={}",
            profile.profile_id,
            profile.is_active,
            profile.default_for_kind,
            profile.kind.as_str(),
            profile.model,
            profile.wire_api.as_str(),
            profile.credential_resolved,
            profile.auth_env.as_deref().unwrap_or("-"),
            profile.endpoint,
            profile.models_endpoint,
            profile.temperature,
            profile
                .max_tokens
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            profile.request_timeout_ms,
            profile.retry_max_attempts,
            render_string_list(profile.header_names.iter().map(String::as_str)),
            render_string_list(profile.preferred_models.iter().map(String::as_str))
        ));
    }

    lines.push(format!(
        "context_engine selected={} source={} api_version={} capabilities={}",
        snapshot.context_engine.selected_metadata.id,
        snapshot.context_engine.selected.source.as_str(),
        snapshot.context_engine.selected_metadata.api_version,
        format_capability_names(&snapshot.context_engine.selected_metadata.capability_names())
    ));
    lines.push(format!(
        "context_engine compaction=enabled:{} min_messages:{} trigger_estimated_tokens:{} fail_open:{}",
        snapshot.context_engine.compaction.enabled,
        snapshot
            .context_engine
            .compaction
            .min_messages
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_owned()),
        snapshot
            .context_engine
            .compaction
            .trigger_estimated_tokens
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_owned()),
        snapshot.context_engine.compaction.fail_open
    ));
    lines.push(format!(
        "memory selected={} source={} api_version={} capabilities={} summary={}",
        snapshot.memory_system.selected_metadata.id,
        snapshot.memory_system.selected.source.as_str(),
        snapshot.memory_system.selected_metadata.api_version,
        format_capability_names(&snapshot.memory_system.selected_metadata.capability_names()),
        snapshot.memory_system.selected_metadata.summary
    ));
    lines.push(format!(
        "memory policy=backend:{} profile:{} mode:{} ingest_mode:{} fail_open:{} strict_mode_requested:{} strict_mode_active:{} effective_fail_open:{}",
        snapshot.memory_system.policy.backend.as_str(),
        snapshot.memory_system.policy.profile.as_str(),
        snapshot.memory_system.policy.mode.as_str(),
        snapshot.memory_system.policy.ingest_mode.as_str(),
        snapshot.memory_system.policy.fail_open,
        snapshot.memory_system.policy.strict_mode_requested,
        snapshot.memory_system.policy.strict_mode_active,
        snapshot.memory_system.policy.effective_fail_open
    ));
    lines.push(format!(
        "acp enabled={} selected={} source={} api_version={} capabilities={} dispatch_enabled={} routing={} thread_routing={} default_agent={} allowed_agents={} allowed_channels={} allowed_account_ids={} bootstrap_mcp_servers={} working_directory={}",
        snapshot.acp.control_plane.enabled,
        snapshot.acp.selected_metadata.id,
        snapshot.acp.selected.source.as_str(),
        snapshot.acp.selected_metadata.api_version,
        format_capability_names(&snapshot.acp.selected_metadata.capability_names()),
        snapshot.acp.control_plane.dispatch_enabled,
        snapshot.acp.control_plane.conversation_routing.as_str(),
        snapshot.acp.control_plane.thread_routing.as_str(),
        snapshot.acp.control_plane.default_agent,
        render_string_list(snapshot.acp.control_plane.allowed_agents.iter().map(String::as_str)),
        render_string_list(snapshot.acp.control_plane.allowed_channels.iter().map(String::as_str)),
        render_string_list(
            snapshot
                .acp
                .control_plane
                .allowed_account_ids
                .iter()
                .map(String::as_str)
        ),
        render_string_list(
            snapshot
                .acp
                .control_plane
                .bootstrap_mcp_servers
                .iter()
                .map(String::as_str)
        ),
        snapshot
            .acp
            .control_plane
            .working_directory
            .as_deref()
            .unwrap_or("-")
    ));
    crate::mcp_cli::append_mcp_runtime_snapshot_lines(&mut lines, &snapshot.acp.mcp);
    let runtime_backed_surface_count = snapshot
        .channels
        .channel_surfaces
        .iter()
        .filter(|surface| {
            surface.catalog.implementation_status
                == mvp::channel::ChannelCatalogImplementationStatus::RuntimeBacked
        })
        .count();
    let config_backed_surface_count = snapshot
        .channels
        .channel_surfaces
        .iter()
        .filter(|surface| {
            surface.catalog.implementation_status
                == mvp::channel::ChannelCatalogImplementationStatus::ConfigBacked
        })
        .count();
    let plugin_backed_surface_count = snapshot
        .channels
        .channel_surfaces
        .iter()
        .filter(|surface| {
            surface.catalog.implementation_status
                == mvp::channel::ChannelCatalogImplementationStatus::PluginBacked
        })
        .count();
    let catalog_only_surface_count = snapshot
        .channels
        .channel_surfaces
        .iter()
        .filter(|surface| {
            surface.catalog.implementation_status
                == mvp::channel::ChannelCatalogImplementationStatus::Stub
        })
        .count();
    lines.push(format!(
        "channels enabled={} runtime_backed_enabled={} service_enabled={} plugin_backed_enabled={} outbound_only_enabled={} configured_accounts={} surfaces={} runtime_backed={} config_backed={} plugin_backed={} catalog_only={}",
        render_string_list(snapshot.enabled_channel_ids.iter().map(String::as_str)),
        render_string_list(
            snapshot
                .enabled_runtime_backed_channel_ids
                .iter()
                .map(String::as_str)
        ),
        render_string_list(
            snapshot
                .enabled_service_channel_ids
                .iter()
                .map(String::as_str)
        ),
        render_string_list(
            snapshot
                .enabled_plugin_backed_channel_ids
                .iter()
                .map(String::as_str)
        ),
        render_string_list(
            snapshot
                .enabled_outbound_only_channel_ids
                .iter()
                .map(String::as_str)
        ),
        snapshot.channels.channels.len(),
        snapshot.channels.channel_surfaces.len(),
        runtime_backed_surface_count,
        config_backed_surface_count,
        plugin_backed_surface_count,
        catalog_only_surface_count
    ));
    for surface in &snapshot.channels.channel_surfaces {
        lines.push(format!(
            "  channel {} implementation_status={} configured_accounts={} default_configured_account={} aliases={}",
            surface.catalog.id,
            surface.catalog.implementation_status.as_str(),
            surface.configured_accounts.len(),
            surface
                .default_configured_account_id
                .as_deref()
                .unwrap_or("-"),
            render_string_list(surface.catalog.aliases.iter().copied())
        ));
        push_channel_surface_managed_plugin_bridge_discovery(&mut lines, surface);
    }
    lines.push(format!(
        "tool_runtime shell_default={} shell_allow={} shell_deny={} sessions_enabled={} messages_enabled={} delegate_enabled={}",
        shell_policy_default_str(snapshot.tool_runtime.shell_default_mode),
        render_string_list(snapshot.tool_runtime.shell_allow.iter().map(String::as_str)),
        render_string_list(snapshot.tool_runtime.shell_deny.iter().map(String::as_str)),
        snapshot.tool_runtime.sessions_enabled,
        snapshot.tool_runtime.messages_enabled,
        snapshot.tool_runtime.delegate_enabled
    ));
    lines.push(format!(
        "tool_runtime browser enabled={} tier={} max_sessions={} max_links={} max_text_chars={}",
        snapshot.tool_runtime.browser.enabled,
        snapshot.tool_runtime.browser_execution_security_tier(),
        snapshot.tool_runtime.browser.max_sessions,
        snapshot.tool_runtime.browser.max_links,
        snapshot.tool_runtime.browser.max_text_chars
    ));
    lines.push(format!(
        "tool_runtime browser_companion enabled={} ready={} tier={} command={} expected_version={}",
        snapshot.tool_runtime.browser_companion.enabled,
        snapshot.tool_runtime.browser_companion.ready,
        snapshot
            .tool_runtime
            .browser_companion_execution_security_tier(),
        snapshot
            .tool_runtime
            .browser_companion
            .command
            .as_deref()
            .unwrap_or("-"),
        snapshot
            .tool_runtime
            .browser_companion
            .expected_version
            .as_deref()
            .unwrap_or("-")
    ));
    lines.push(format!(
        "tool_runtime web_fetch enabled={} allow_private_hosts={} timeout_seconds={} max_bytes={} max_redirects={} allowed_domains={} blocked_domains={}",
        snapshot.tool_runtime.web_fetch.enabled,
        snapshot.tool_runtime.web_fetch.allow_private_hosts,
        snapshot.tool_runtime.web_fetch.timeout_seconds,
        snapshot.tool_runtime.web_fetch.max_bytes,
        snapshot.tool_runtime.web_fetch.max_redirects,
        render_string_list(snapshot.tool_runtime.web_fetch.allowed_domains.iter().map(String::as_str)),
        render_string_list(snapshot.tool_runtime.web_fetch.blocked_domains.iter().map(String::as_str))
    ));
    let web_access_summary = crate::runtime_web_access_summary(&snapshot.tool_runtime);
    lines.push(format!(
        "tool_runtime web_search enabled={} default_provider={} credential_ready={} separation_note=\"{}\"",
        snapshot.tool_runtime.web_search.enabled,
        snapshot.tool_runtime.web_search.default_provider,
        web_access_summary.query_search_credential_ready,
        web_access_summary.separation_note
    ));
    lines.push(format!(
        "tool_runtime web_access ordinary_network_enabled={} query_search_enabled={} query_search_default_provider={} query_search_credential_ready={} separation_note=\"{}\"",
        web_access_summary.ordinary_network_access_enabled,
        web_access_summary.query_search_enabled,
        web_access_summary.query_search_default_provider,
        web_access_summary.query_search_credential_ready,
        web_access_summary.separation_note
    ));
    lines.push(format!(
        "tools visible_count={} hidden_count={} capability_snapshot_sha256={} visible_names={} visible_direct_names={}",
        snapshot.visible_tool_names.len(),
        snapshot.discoverable_tool_summary.hidden_tool_count,
        snapshot.capability_snapshot_sha256,
        render_string_list(snapshot.visible_tool_names.iter().map(String::as_str)),
        render_string_list(
            snapshot
                .discoverable_tool_summary
                .visible_direct_tools
                .iter()
                .map(String::as_str)
        )
    ));
    lines.push(format!(
        "tools hidden_tags={} hidden_surfaces={}",
        render_string_list(
            snapshot
                .discoverable_tool_summary
                .hidden_tags
                .iter()
                .map(String::as_str)
        ),
        render_tool_surface_summary(
            snapshot
                .discoverable_tool_summary
                .hidden_surfaces
                .as_slice()
        )
    ));
    lines.extend(render_runtime_plugins_lines(&snapshot.runtime_plugins));
    lines.push(format!(
        "external_skills inventory_status={} override_active={} enabled={} require_download_approval={} auto_expose_installed={} install_root={} resolved_skills={} shadowed_skills={} inventory_error={}",
        snapshot.external_skills.inventory_status.as_str(),
        snapshot.external_skills.override_active,
        snapshot.external_skills.policy.enabled,
        snapshot.external_skills.policy.require_download_approval,
        snapshot.external_skills.policy.auto_expose_installed,
        snapshot
            .external_skills
            .policy
            .install_root
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "-".to_owned()),
        snapshot.external_skills.resolved_skill_count,
        snapshot.external_skills.shadowed_skill_count,
        snapshot
            .external_skills
            .inventory_error
            .as_deref()
            .unwrap_or("-")
    ));

    if let Some(skills) = snapshot
        .external_skills
        .inventory
        .get("skills")
        .and_then(Value::as_array)
    {
        for skill in skills {
            lines.push(format!(
                "  external_skill {} scope={} active={} sha256={}",
                json_string_field(skill, "skill_id"),
                json_string_field(skill, "scope"),
                skill
                    .get("active")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                json_string_field(skill, "sha256")
            ));
        }
    }

    let body_lines = lines
        .into_iter()
        .chain([
            "capability_snapshot:".to_owned(),
            snapshot.capability_snapshot.clone(),
        ])
        .collect::<Vec<_>>();
    crate::render_operator_shell_surface(
        "runtime snapshot",
        "operator runtime snapshot",
        Vec::new(),
        body_lines,
        Vec::new(),
    )
}

fn render_tool_surface_summary(surfaces: &[crate::mvp::tools::ToolSurfaceState]) -> String {
    if surfaces.is_empty() {
        return "-".to_owned();
    }

    surfaces
        .iter()
        .map(|surface| format!("{}:{}", surface.surface_id, surface.tool_count()))
        .collect::<Vec<_>>()
        .join(",")
}

fn render_runtime_plugins_lines(snapshot: &RuntimeSnapshotRuntimePluginsState) -> Vec<String> {
    let mut lines = vec![format!(
        "runtime_plugins inventory_status={} enabled={} roots_source={} readiness_evaluation={} supported_bridges={} supported_adapter_families={} roots={} scanned_roots={} scanned_files={} discovered={} translated={} ready={} setup_incomplete={} blocked={} shadowed_plugins={}",
        snapshot.inventory_status.as_str(),
        snapshot.enabled,
        crate::render_line_safe_text_value(&snapshot.roots_source),
        snapshot.readiness_evaluation,
        crate::render_line_safe_text_values(
            snapshot.supported_bridges.iter().map(String::as_str),
            ","
        ),
        crate::render_line_safe_text_values(
            snapshot
                .supported_adapter_families
                .iter()
                .map(String::as_str),
            ",",
        ),
        crate::render_line_safe_text_values(snapshot.roots.iter().map(String::as_str), ","),
        snapshot.scanned_root_count,
        snapshot.scanned_file_count,
        snapshot.discovered_plugin_count,
        snapshot.translated_plugin_count,
        snapshot.ready_plugin_count,
        snapshot.setup_incomplete_plugin_count,
        snapshot.blocked_plugin_count,
        snapshot.shadowed_plugin_ids.len(),
    )];
    if let Some(authoring_summary) = snapshot.native_extension_authoring_summary.as_ref() {
        let action_roles = authoring_summary
            .action_roles
            .iter()
            .map(|(key, value)| format!("{key}:{value}"))
            .collect::<Vec<_>>()
            .join(",");
        let action_execution_kinds = authoring_summary
            .action_execution_kinds
            .iter()
            .map(|(key, value)| format!("{key}:{value}"))
            .collect::<Vec<_>>()
            .join(",");
        lines.push(format!(
            "  authoring_summary guided_plugins={} plugins_with_metadata_issues={} total_remediation_actions={} action_roles={} action_execution_kinds={} runnable_actions={} allow_command_gated_actions={}",
            authoring_summary.guided_plugins,
            authoring_summary.plugins_with_metadata_issues,
            authoring_summary.total_remediation_actions,
            crate::render_line_safe_text_value(&action_roles),
            crate::render_line_safe_text_value(&action_execution_kinds),
            authoring_summary.runnable_action_count,
            authoring_summary.allow_command_gated_action_count,
        ));
    }
    if !snapshot.shadowed_plugin_ids.is_empty() {
        lines.push(format!(
            "  shadowed_plugin_ids={}",
            crate::render_line_safe_text_values(
                snapshot.shadowed_plugin_ids.iter().map(String::as_str),
                ",",
            )
        ));
    }
    if let Some(discovery_guidance) = snapshot.discovery_guidance.as_ref() {
        let recommended_action = crate::render_line_safe_optional_text_value(
            discovery_guidance.recommended_action.as_deref(),
        );
        let discovery_action_kinds = crate::render_line_safe_text_values(
            discovery_guidance
                .discovery_actions
                .iter()
                .map(|action| action.kind.as_str()),
            ",",
        );
        let first_conflict = discovery_guidance.shadowed_conflicts.first();
        let effective_source_path = first_conflict
            .map(|conflict| crate::render_line_safe_text_value(&conflict.effective_source_path))
            .unwrap_or_else(|| "-".to_owned());
        let shadowed_source_paths = first_conflict
            .map(|conflict| {
                crate::render_line_safe_text_values(
                    conflict.shadowed_source_paths.iter().map(String::as_str),
                    ",",
                )
            })
            .unwrap_or_else(|| "-".to_owned());
        lines.push(format!(
            "  discovery_guidance precedence_rule={} precedence_roots={}>{} recommended_action={} discovery_action_kinds={} effective_source_path={} shadowed_source_paths={}",
            crate::render_line_safe_text_value(&discovery_guidance.precedence_rule),
            crate::render_line_safe_text_value(&discovery_guidance.project_local_root),
            crate::render_line_safe_text_value(&discovery_guidance.global_root),
            recommended_action,
            discovery_action_kinds,
            effective_source_path,
            shadowed_source_paths,
        ));
    }

    if let Some(error) = snapshot.inventory_error.as_deref() {
        let rendered_error = crate::render_line_safe_text_value(error);

        lines.push(format!("  runtime_plugin_error {rendered_error}"));
    }

    for plugin in &snapshot.plugins {
        let plugin_id = crate::render_line_safe_text_value(&plugin.plugin_id);
        let manifest_api_version =
            crate::render_line_safe_optional_text_value(plugin.manifest_api_version.as_deref());
        let plugin_version =
            crate::render_line_safe_optional_text_value(plugin.plugin_version.as_deref());
        let dialect = crate::render_line_safe_text_value(&plugin.dialect);
        let dialect_version =
            crate::render_line_safe_optional_text_value(plugin.dialect_version.as_deref());
        let compatibility_mode = crate::render_line_safe_text_value(&plugin.compatibility_mode);
        let compatibility_shim =
            crate::render_line_safe_optional_text_value(plugin.compatibility_shim.as_deref());
        let compatibility_shim_support_version = crate::render_line_safe_optional_text_value(
            plugin.compatibility_shim_support_version.as_deref(),
        );
        let compatibility_shim_supported_dialects = crate::render_line_safe_text_values(
            plugin
                .compatibility_shim_supported_dialects
                .iter()
                .map(String::as_str),
            ",",
        );
        let compatibility_shim_supported_bridges = crate::render_line_safe_text_values(
            plugin
                .compatibility_shim_supported_bridges
                .iter()
                .map(String::as_str),
            ",",
        );
        let compatibility_shim_supported_adapter_families = crate::render_line_safe_text_values(
            plugin
                .compatibility_shim_supported_adapter_families
                .iter()
                .map(String::as_str),
            ",",
        );
        let compatibility_shim_supported_source_languages = crate::render_line_safe_text_values(
            plugin
                .compatibility_shim_supported_source_languages
                .iter()
                .map(String::as_str),
            ",",
        );
        let compatibility_shim_mismatch_reasons = crate::render_line_safe_text_values(
            plugin
                .compatibility_shim_mismatch_reasons
                .iter()
                .map(String::as_str),
            ",",
        );
        let source_path = crate::render_line_safe_text_value(plugin.source_path.as_str());
        let package_root = crate::render_line_safe_text_value(plugin.package_root.as_str());
        let summary = crate::render_line_safe_optional_text_value(plugin.summary.as_deref());
        let tags = crate::render_line_safe_text_values(plugin.tags.iter().map(String::as_str), ",");
        let capabilities = crate::render_line_safe_text_values(
            plugin.capabilities.iter().map(String::as_str),
            ",",
        );
        let provider_id = crate::render_line_safe_text_value(&plugin.provider_id);
        let connector_name = crate::render_line_safe_text_value(&plugin.connector_name);
        let bridge_kind = crate::render_line_safe_text_value(&plugin.bridge_kind);
        let adapter_family = crate::render_line_safe_text_value(&plugin.adapter_family);
        let source_language = crate::render_line_safe_text_value(&plugin.source_language);
        let entrypoint_hint = crate::render_line_safe_text_value(&plugin.entrypoint_hint);
        let status = crate::render_line_safe_text_value(&plugin.status);
        let setup_mode = crate::render_line_safe_optional_text_value(plugin.setup_mode.as_deref());
        let setup_surface =
            crate::render_line_safe_optional_text_value(plugin.setup_surface.as_deref());
        let reason = crate::render_line_safe_text_value(&plugin.reason);
        let bootstrap_hint =
            crate::render_line_safe_optional_text_value(plugin.bootstrap_hint.as_deref());
        let diagnostic_codes = crate::render_line_safe_text_values(
            plugin.diagnostic_codes.iter().map(String::as_str),
            ",",
        );
        let missing_required_env_vars = crate::render_line_safe_text_values(
            plugin.missing_required_env_vars.iter().map(String::as_str),
            ",",
        );
        let missing_required_config_keys = crate::render_line_safe_text_values(
            plugin
                .missing_required_config_keys
                .iter()
                .map(String::as_str),
            ",",
        );
        let extension_contract =
            crate::render_line_safe_optional_text_value(plugin.extension_contract.as_deref());
        let extension_family =
            crate::render_line_safe_optional_text_value(plugin.extension_family.as_deref());
        let extension_trust_lane =
            crate::render_line_safe_optional_text_value(plugin.extension_trust_lane.as_deref());
        let extension_facets = crate::render_line_safe_text_values(
            plugin.extension_facets.iter().map(String::as_str),
            ",",
        );
        let extension_methods = crate::render_line_safe_text_values(
            plugin.extension_methods.iter().map(String::as_str),
            ",",
        );
        let extension_events = crate::render_line_safe_text_values(
            plugin.extension_events.iter().map(String::as_str),
            ",",
        );
        let extension_host_actions = crate::render_line_safe_text_values(
            plugin.extension_host_actions.iter().map(String::as_str),
            ",",
        );
        let extension_metadata_issues = crate::render_line_safe_text_values(
            plugin.extension_metadata_issues.iter().map(String::as_str),
            ",",
        );
        let slot_claims =
            crate::render_line_safe_text_values(plugin.slot_claims.iter().map(String::as_str), ",");
        let conflicting_slot_claims = crate::render_line_safe_text_values(
            plugin.conflicting_slot_claims.iter().map(String::as_str),
            ",",
        );

        lines.push(format!(
            "  runtime_plugin {} manifest_api_version={} plugin_version={} dialect={} dialect_version={} compatibility_mode={} compatibility_shim={} compatibility_shim_support_version={} compatibility_shim_supported_dialects={} compatibility_shim_supported_bridges={} compatibility_shim_supported_adapter_families={} compatibility_shim_supported_source_languages={} compatibility_shim_mismatch_reasons={} source_path={} package_root={} summary={} tags={} capabilities={} provider={} connector={} bridge={} adapter_family={} source_language={} entrypoint_hint={} status={} setup_mode={} setup_surface={} reason={} bootstrap_hint={} diagnostic_codes={} missing_env_vars={} missing_config_keys={} extension_contract={} extension_family={} extension_trust_lane={} extension_facets={} extension_methods={} extension_events={} extension_host_actions={} extension_metadata_issues={} slot_claims={} conflicting_slot_claims={}",
            plugin_id,
            manifest_api_version,
            plugin_version,
            dialect,
            dialect_version,
            compatibility_mode,
            compatibility_shim,
            compatibility_shim_support_version,
            compatibility_shim_supported_dialects,
            compatibility_shim_supported_bridges,
            compatibility_shim_supported_adapter_families,
            compatibility_shim_supported_source_languages,
            compatibility_shim_mismatch_reasons,
            source_path,
            package_root,
            summary,
            tags,
            capabilities,
            provider_id,
            connector_name,
            bridge_kind,
            adapter_family,
            source_language,
            entrypoint_hint,
            status,
            setup_mode,
            setup_surface,
            reason,
            bootstrap_hint,
            diagnostic_codes,
            missing_required_env_vars,
            missing_required_config_keys,
            extension_contract,
            extension_family,
            extension_trust_lane,
            extension_facets,
            extension_methods,
            extension_events,
            extension_host_actions,
            extension_metadata_issues,
            slot_claims,
            conflicting_slot_claims,
        ));
        if let Some(authoring_guidance) = plugin.authoring_guidance.as_ref() {
            let action_roles = crate::render_line_safe_text_values(
                authoring_guidance
                    .author_remediation_actions
                    .iter()
                    .map(|action| action.role.as_str()),
                ",",
            );
            let action_kinds = crate::render_line_safe_text_values(
                authoring_guidance
                    .author_remediation_actions
                    .iter()
                    .map(|action| action.kind.as_str()),
                ",",
            );
            let runnable_action_kinds = crate::render_line_safe_text_values(
                authoring_guidance
                    .author_remediation_actions
                    .iter()
                    .filter(|action| action.agent_runnable)
                    .map(|action| action.kind.as_str()),
                ",",
            );
            let allow_command_action_kinds = crate::render_line_safe_text_values(
                authoring_guidance
                    .author_remediation_actions
                    .iter()
                    .filter(|action| action.requires_allow_command)
                    .map(|action| action.kind.as_str()),
                ",",
            );
            let reference_example =
                crate::render_line_safe_text_value(&authoring_guidance.reference_example_path);
            let smoke_allow_command =
                crate::render_line_safe_text_value(&authoring_guidance.smoke_allow_command);
            lines.push(format!(
                "    authoring reference_example={} smoke_allow_command={} action_roles={} action_kinds={} runnable_action_kinds={} allow_command_action_kinds={}",
                reference_example,
                smoke_allow_command,
                action_roles,
                action_kinds,
                runnable_action_kinds,
                allow_command_action_kinds,
            ));
        }
    }

    lines
}

pub(crate) fn runtime_snapshot_provider_json(snapshot: &RuntimeSnapshotProviderState) -> Value {
    json!({
        "active_profile_id": snapshot.active_profile_id,
        "active_label": snapshot.active_label,
        "last_provider_id": snapshot.last_provider_id,
        "saved_profile_ids": snapshot.saved_profile_ids,
        "transport_runtime": {
            "http_client_cache_entries": snapshot.transport_runtime.http_client_cache_entries,
            "http_client_cache_hits": snapshot.transport_runtime.http_client_cache_hits,
            "http_client_cache_misses": snapshot.transport_runtime.http_client_cache_misses,
            "built_http_clients": snapshot.transport_runtime.built_http_clients,
        },
        "profiles": snapshot
            .profiles
            .iter()
            .map(runtime_snapshot_provider_profile_json)
            .collect::<Vec<_>>(),
    })
}

fn runtime_snapshot_provider_profile_json(profile: &RuntimeSnapshotProviderProfileState) -> Value {
    let descriptor = runtime_snapshot_provider_descriptor_json(&profile.descriptor);

    json!({
        "profile_id": profile.profile_id,
        "is_active": profile.is_active,
        "default_for_kind": profile.default_for_kind,
        "descriptor": descriptor,
        "kind": profile.kind.as_str(),
        "model": profile.model,
        "wire_api": profile.wire_api.as_str(),
        "base_url": profile.base_url,
        "endpoint": profile.endpoint,
        "models_endpoint": profile.models_endpoint,
        "protocol_family": profile.protocol_family,
        "credential_resolved": profile.credential_resolved,
        "auth_env": profile.auth_env,
        "reasoning_effort": profile.reasoning_effort,
        "temperature": profile.temperature,
        "max_tokens": profile.max_tokens,
        "request_timeout_ms": profile.request_timeout_ms,
        "retry_max_attempts": profile.retry_max_attempts,
        "header_names": profile.header_names,
        "preferred_models": profile.preferred_models,
    })
}

fn runtime_snapshot_provider_descriptor_json(
    descriptor: &mvp::config::ProviderDescriptorDocument,
) -> Value {
    serde_json::to_value(descriptor).expect("provider descriptor document should serialize")
}

pub(crate) fn runtime_snapshot_context_engine_json(
    snapshot: &mvp::conversation::ContextEngineRuntimeSnapshot,
) -> Value {
    json!({
        "selected": context_engine_metadata_json(
            &snapshot.selected_metadata,
            Some(snapshot.selected.source.as_str())
        ),
        "available": snapshot
            .available
            .iter()
            .map(|metadata| context_engine_metadata_json(metadata, None))
            .collect::<Vec<_>>(),
        "compaction": {
            "enabled": snapshot.compaction.enabled,
            "min_messages": snapshot.compaction.min_messages,
            "trigger_estimated_tokens": snapshot.compaction.trigger_estimated_tokens,
            "fail_open": snapshot.compaction.fail_open,
        },
    })
}

pub(crate) fn runtime_snapshot_memory_system_json(
    snapshot: &mvp::memory::MemorySystemRuntimeSnapshot,
) -> Value {
    json!({
        "selected": memory_system_metadata_json(
            &snapshot.selected_metadata,
            Some(snapshot.selected.source.as_str())
        ),
        "available": snapshot
            .available
            .iter()
            .map(|metadata| memory_system_metadata_json(metadata, None))
            .collect::<Vec<_>>(),
        "policy": memory_system_policy_json(&snapshot.policy),
    })
}

pub(crate) fn runtime_snapshot_acp_json(snapshot: &mvp::acp::AcpRuntimeSnapshot) -> Value {
    json!({
        "enabled": snapshot.control_plane.enabled,
        "selected": acp_backend_metadata_json(
            &snapshot.selected_metadata,
            Some(snapshot.selected.source.as_str())
        ),
        "available": snapshot
            .available
            .iter()
            .map(|metadata| acp_backend_metadata_json(metadata, None))
            .collect::<Vec<_>>(),
        "control_plane": acp_control_plane_json(&snapshot.control_plane),
        "mcp": crate::mcp_cli::mcp_runtime_snapshot_json(&snapshot.mcp),
    })
}

pub(crate) fn runtime_snapshot_tool_runtime_json(
    runtime: &mvp::tools::runtime_config::ToolRuntimeConfig,
) -> Value {
    let web_access_summary = crate::runtime_web_access_summary(runtime);
    json!({
        "file_root": runtime
            .file_root
            .as_ref()
            .map(|path| path.display().to_string()),
        "shell": {
            "default_mode": shell_policy_default_str(runtime.shell_default_mode),
            "allow": runtime.shell_allow.iter().collect::<Vec<_>>(),
            "deny": runtime.shell_deny.iter().collect::<Vec<_>>(),
        },
        "sessions_enabled": runtime.sessions_enabled,
        "messages_enabled": runtime.messages_enabled,
        "delegate_enabled": runtime.delegate_enabled,
        "browser": {
            "enabled": runtime.browser.enabled,
            "execution_tier": runtime.browser_execution_security_tier().as_str(),
            "max_sessions": runtime.browser.max_sessions,
            "max_links": runtime.browser.max_links,
            "max_text_chars": runtime.browser.max_text_chars,
        },
        "browser_companion": {
            "enabled": runtime.browser_companion.enabled,
            "ready": runtime.browser_companion.ready,
            "execution_tier": runtime.browser_companion_execution_security_tier().as_str(),
            "command": runtime.browser_companion.command,
            "expected_version": runtime.browser_companion.expected_version,
        },
        "web_fetch": {
            "enabled": runtime.web_fetch.enabled,
            "allow_private_hosts": runtime.web_fetch.allow_private_hosts,
            "allowed_domains": runtime.web_fetch.allowed_domains.iter().collect::<Vec<_>>(),
            "blocked_domains": runtime.web_fetch.blocked_domains.iter().collect::<Vec<_>>(),
            "timeout_seconds": runtime.web_fetch.timeout_seconds,
            "max_bytes": runtime.web_fetch.max_bytes,
            "max_redirects": runtime.web_fetch.max_redirects,
        },
        "web_search": {
            "enabled": runtime.web_search.enabled,
            "default_provider": runtime.web_search.default_provider,
            "credential_ready": web_access_summary.query_search_credential_ready,
            "separation_note": web_access_summary.separation_note,
        },
        "web_access": {
            "ordinary_network_access_enabled": web_access_summary.ordinary_network_access_enabled,
            "query_search_enabled": web_access_summary.query_search_enabled,
            "query_search_default_provider": web_access_summary.query_search_default_provider,
            "query_search_credential_ready": web_access_summary.query_search_credential_ready,
            "separation_note": web_access_summary.separation_note,
        },
    })
}

pub(crate) fn runtime_snapshot_external_skills_json(
    snapshot: &RuntimeSnapshotExternalSkillsState,
) -> Value {
    json!({
        "policy": {
            "enabled": snapshot.policy.enabled,
            "require_download_approval": snapshot.policy.require_download_approval,
            "allowed_domains": snapshot.policy.allowed_domains.iter().collect::<Vec<_>>(),
            "blocked_domains": snapshot.policy.blocked_domains.iter().collect::<Vec<_>>(),
            "install_root": snapshot
                .policy
                .install_root
                .as_ref()
                .map(|path| path.display().to_string()),
            "auto_expose_installed": snapshot.policy.auto_expose_installed,
        },
        "override_active": snapshot.override_active,
        "inventory_status": snapshot.inventory_status.as_str(),
        "inventory_error": snapshot.inventory_error,
        "resolved_skill_count": snapshot.resolved_skill_count,
        "shadowed_skill_count": snapshot.shadowed_skill_count,
        "inventory": snapshot.inventory,
    })
}

pub(crate) fn runtime_snapshot_runtime_plugins_json(
    snapshot: &RuntimeSnapshotRuntimePluginsState,
) -> Value {
    json!({
        "enabled": snapshot.enabled,
        "roots_source": snapshot.roots_source,
        "roots": snapshot.roots,
        "supported_bridges": snapshot.supported_bridges,
        "supported_adapter_families": snapshot.supported_adapter_families,
        "inventory_status": snapshot.inventory_status.as_str(),
        "inventory_error": snapshot.inventory_error,
        "readiness_evaluation": snapshot.readiness_evaluation,
        "scanned_root_count": snapshot.scanned_root_count,
        "scanned_file_count": snapshot.scanned_file_count,
        "discovered_plugin_count": snapshot.discovered_plugin_count,
        "translated_plugin_count": snapshot.translated_plugin_count,
        "ready_plugin_count": snapshot.ready_plugin_count,
        "setup_incomplete_plugin_count": snapshot.setup_incomplete_plugin_count,
        "blocked_plugin_count": snapshot.blocked_plugin_count,
        "shadowed_plugin_ids": snapshot.shadowed_plugin_ids,
        "discovery_guidance": snapshot.discovery_guidance,
        "native_extension_authoring_summary": snapshot.native_extension_authoring_summary,
        "plugins": snapshot
            .plugins
            .iter()
            .map(runtime_snapshot_runtime_plugin_json)
            .collect::<Vec<_>>(),
    })
}

fn runtime_snapshot_runtime_plugin_json(
    plugin: &crate::RuntimeSnapshotRuntimePluginState,
) -> Value {
    let mut object = serde_json::Map::new();
    object.insert(
        "manifest_api_version".to_owned(),
        serde_json::to_value(&plugin.manifest_api_version).unwrap_or(Value::Null),
    );
    object.insert(
        "plugin_version".to_owned(),
        serde_json::to_value(&plugin.plugin_version).unwrap_or(Value::Null),
    );
    object.insert("dialect".to_owned(), Value::String(plugin.dialect.clone()));
    object.insert(
        "dialect_version".to_owned(),
        serde_json::to_value(&plugin.dialect_version).unwrap_or(Value::Null),
    );
    object.insert(
        "compatibility_mode".to_owned(),
        Value::String(plugin.compatibility_mode.clone()),
    );
    object.insert(
        "compatibility_shim".to_owned(),
        serde_json::to_value(&plugin.compatibility_shim).unwrap_or(Value::Null),
    );
    object.insert(
        "compatibility_shim_support_version".to_owned(),
        serde_json::to_value(&plugin.compatibility_shim_support_version).unwrap_or(Value::Null),
    );
    object.insert(
        "compatibility_shim_supported_dialects".to_owned(),
        serde_json::to_value(&plugin.compatibility_shim_supported_dialects).unwrap_or(Value::Null),
    );
    object.insert(
        "compatibility_shim_supported_bridges".to_owned(),
        serde_json::to_value(&plugin.compatibility_shim_supported_bridges).unwrap_or(Value::Null),
    );
    object.insert(
        "compatibility_shim_supported_adapter_families".to_owned(),
        serde_json::to_value(&plugin.compatibility_shim_supported_adapter_families)
            .unwrap_or(Value::Null),
    );
    object.insert(
        "compatibility_shim_supported_source_languages".to_owned(),
        serde_json::to_value(&plugin.compatibility_shim_supported_source_languages)
            .unwrap_or(Value::Null),
    );
    object.insert(
        "compatibility_shim_mismatch_reasons".to_owned(),
        serde_json::to_value(&plugin.compatibility_shim_mismatch_reasons).unwrap_or(Value::Null),
    );
    object.insert(
        "plugin_id".to_owned(),
        Value::String(plugin.plugin_id.clone()),
    );
    object.insert(
        "provider_id".to_owned(),
        Value::String(plugin.provider_id.clone()),
    );
    object.insert(
        "connector_name".to_owned(),
        Value::String(plugin.connector_name.clone()),
    );
    object.insert(
        "source_path".to_owned(),
        Value::String(plugin.source_path.clone()),
    );
    object.insert(
        "source_kind".to_owned(),
        Value::String(plugin.source_kind.clone()),
    );
    object.insert(
        "package_root".to_owned(),
        Value::String(plugin.package_root.clone()),
    );
    object.insert(
        "package_manifest_path".to_owned(),
        serde_json::to_value(&plugin.package_manifest_path).unwrap_or(Value::Null),
    );
    object.insert(
        "summary".to_owned(),
        serde_json::to_value(&plugin.summary).unwrap_or(Value::Null),
    );
    object.insert(
        "tags".to_owned(),
        serde_json::to_value(&plugin.tags).unwrap_or(Value::Null),
    );
    object.insert(
        "capabilities".to_owned(),
        serde_json::to_value(&plugin.capabilities).unwrap_or(Value::Null),
    );
    object.insert(
        "bridge_kind".to_owned(),
        Value::String(plugin.bridge_kind.clone()),
    );
    object.insert(
        "adapter_family".to_owned(),
        Value::String(plugin.adapter_family.clone()),
    );
    object.insert(
        "source_language".to_owned(),
        Value::String(plugin.source_language.clone()),
    );
    object.insert(
        "entrypoint_hint".to_owned(),
        Value::String(plugin.entrypoint_hint.clone()),
    );
    object.insert(
        "setup_mode".to_owned(),
        serde_json::to_value(&plugin.setup_mode).unwrap_or(Value::Null),
    );
    object.insert(
        "setup_surface".to_owned(),
        serde_json::to_value(&plugin.setup_surface).unwrap_or(Value::Null),
    );
    object.insert(
        "slot_claims".to_owned(),
        serde_json::to_value(&plugin.slot_claims).unwrap_or(Value::Null),
    );
    object.insert(
        "conflicting_slot_claims".to_owned(),
        serde_json::to_value(&plugin.conflicting_slot_claims).unwrap_or(Value::Null),
    );
    object.insert("status".to_owned(), Value::String(plugin.status.clone()));
    object.insert("reason".to_owned(), Value::String(plugin.reason.clone()));
    object.insert(
        "bootstrap_hint".to_owned(),
        serde_json::to_value(&plugin.bootstrap_hint).unwrap_or(Value::Null),
    );
    object.insert(
        "diagnostic_codes".to_owned(),
        serde_json::to_value(&plugin.diagnostic_codes).unwrap_or(Value::Null),
    );
    object.insert(
        "missing_required_env_vars".to_owned(),
        serde_json::to_value(&plugin.missing_required_env_vars).unwrap_or(Value::Null),
    );
    object.insert(
        "missing_required_config_keys".to_owned(),
        serde_json::to_value(&plugin.missing_required_config_keys).unwrap_or(Value::Null),
    );
    object.insert(
        "extension_contract".to_owned(),
        serde_json::to_value(&plugin.extension_contract).unwrap_or(Value::Null),
    );
    object.insert(
        "extension_family".to_owned(),
        serde_json::to_value(&plugin.extension_family).unwrap_or(Value::Null),
    );
    object.insert(
        "extension_trust_lane".to_owned(),
        serde_json::to_value(&plugin.extension_trust_lane).unwrap_or(Value::Null),
    );
    object.insert(
        "extension_facets".to_owned(),
        serde_json::to_value(&plugin.extension_facets).unwrap_or(Value::Null),
    );
    object.insert(
        "extension_methods".to_owned(),
        serde_json::to_value(&plugin.extension_methods).unwrap_or(Value::Null),
    );
    object.insert(
        "extension_events".to_owned(),
        serde_json::to_value(&plugin.extension_events).unwrap_or(Value::Null),
    );
    object.insert(
        "extension_host_actions".to_owned(),
        serde_json::to_value(&plugin.extension_host_actions).unwrap_or(Value::Null),
    );
    object.insert(
        "extension_metadata_issues".to_owned(),
        serde_json::to_value(&plugin.extension_metadata_issues).unwrap_or(Value::Null),
    );
    object.insert(
        "authoring_guidance".to_owned(),
        serde_json::to_value(&plugin.authoring_guidance).unwrap_or(Value::Null),
    );
    Value::Object(object)
}

fn shell_policy_default_str(
    mode: mvp::tools::shell_policy_ext::ShellPolicyDefault,
) -> &'static str {
    match mode {
        mvp::tools::shell_policy_ext::ShellPolicyDefault::Deny => "deny",
        mvp::tools::shell_policy_ext::ShellPolicyDefault::Allow => "allow",
    }
}

fn json_string_field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("-")
}
