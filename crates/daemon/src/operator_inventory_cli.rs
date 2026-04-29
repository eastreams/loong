use super::*;

pub const CHANNELS_CLI_JSON_SCHEMA_VERSION: u32 = 2;
pub const CHANNELS_CLI_JSON_LEGACY_VIEWS: &[&str] = &["channels", "catalog_only_channels"];

pub fn run_channels_cli(
    config_path: Option<&str>,
    resolve: Option<&str>,
    as_json: bool,
) -> CliResult<()> {
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let inventory = mvp::channel::channel_inventory(&config);
    let resolved_path_display = resolved_path.display().to_string();

    if let Some(resolve) = resolve {
        let resolution = channel_resolution::build_channel_resolution(
            resolved_path_display.as_str(),
            &config,
            &inventory,
            resolve,
        )?;
        if as_json {
            let pretty = serde_json::to_string_pretty(&resolution)
                .map_err(|error| format!("serialize channel resolution output failed: {error}"))?;
            println!("{pretty}");
            return Ok(());
        }
        println!(
            "{}",
            channel_resolution::render_channel_resolution_text(&resolution)
        );
        return Ok(());
    }

    if as_json {
        let payload = build_channels_cli_json_payload(&resolved_path_display, &inventory);
        let pretty = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("serialize channel status output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    println!(
        "{}",
        render_channel_surfaces_shell_text(&resolved_path_display, &inventory)
    );
    Ok(())
}

pub fn build_channels_cli_json_payload(
    config_path: &str,
    inventory: &mvp::channel::ChannelInventory,
) -> ChannelsCliJsonPayload {
    gateway::read_models::build_channel_inventory_read_model(config_path, inventory)
}

pub fn render_channel_surfaces_text(
    config_path: &str,
    inventory: &mvp::channel::ChannelInventory,
) -> String {
    let lines = build_channel_surfaces_body_lines(config_path, inventory);
    let mut rendered = mvp::presentation::render_compact_brand_header(
        mvp::presentation::detect_render_width()
            .max(96)
            .saturating_sub(2),
        &mvp::presentation::BuildVersionInfo::current(),
        Some("operator channels"),
    )
    .into_iter()
    .map(|line| line.text)
    .collect::<Vec<_>>();
    rendered.push(String::new());
    rendered.push("channels".to_owned());
    rendered.push(String::new());
    rendered.extend(lines);
    rendered.join("\n")
}

pub fn render_channel_surfaces_shell_text(
    config_path: &str,
    inventory: &mvp::channel::ChannelInventory,
) -> String {
    render_operator_shell_surface(
        "channels",
        "operator channels",
        Vec::new(),
        build_channel_surfaces_body_lines(config_path, inventory),
        Vec::new(),
    )
}

fn build_channel_surfaces_body_lines(
    config_path: &str,
    inventory: &mvp::channel::ChannelInventory,
) -> Vec<String> {
    let mut lines = vec![format!("config={config_path}")];
    lines.push(render_channel_surface_summary_line(
        &inventory.channel_surfaces,
    ));
    let channel_access_policies = channel_access_policy_by_account(inventory);

    let grouped_surfaces = [
        (
            "runtime-backed channels:",
            mvp::channel::ChannelCatalogImplementationStatus::RuntimeBacked,
        ),
        (
            "config-backed channels:",
            mvp::channel::ChannelCatalogImplementationStatus::ConfigBacked,
        ),
        (
            "plugin-backed channels:",
            mvp::channel::ChannelCatalogImplementationStatus::PluginBacked,
        ),
        (
            "catalog-only channels:",
            mvp::channel::ChannelCatalogImplementationStatus::Stub,
        ),
    ];

    for (section_title, implementation_status) in grouped_surfaces {
        let grouped = inventory
            .channel_surfaces
            .iter()
            .filter(|surface| surface.catalog.implementation_status == implementation_status)
            .collect::<Vec<_>>();
        if grouped.is_empty() {
            continue;
        }

        lines.push(section_title.to_owned());
        for surface in grouped {
            push_channel_surface_block(&mut lines, surface, &channel_access_policies);
        }
    }
    lines
}

fn render_channel_surface_summary_line(surfaces: &[mvp::channel::ChannelSurface]) -> String {
    let runtime_backed = surfaces
        .iter()
        .filter(|surface| {
            surface.catalog.implementation_status
                == mvp::channel::ChannelCatalogImplementationStatus::RuntimeBacked
        })
        .count();
    let config_backed = surfaces
        .iter()
        .filter(|surface| {
            surface.catalog.implementation_status
                == mvp::channel::ChannelCatalogImplementationStatus::ConfigBacked
        })
        .count();
    let plugin_backed = surfaces
        .iter()
        .filter(|surface| {
            surface.catalog.implementation_status
                == mvp::channel::ChannelCatalogImplementationStatus::PluginBacked
        })
        .count();
    let catalog_only = surfaces
        .iter()
        .filter(|surface| {
            surface.catalog.implementation_status
                == mvp::channel::ChannelCatalogImplementationStatus::Stub
        })
        .count();

    format!(
        "summary total_surfaces={} runtime_backed={} config_backed={} plugin_backed={} catalog_only={}",
        surfaces.len(),
        runtime_backed,
        config_backed,
        plugin_backed,
        catalog_only
    )
}

fn push_channel_surface_block(
    lines: &mut Vec<String>,
    surface: &mvp::channel::ChannelSurface,
    channel_access_policies: &std::collections::BTreeMap<
        (String, String),
        mvp::channel::ChannelConfiguredAccountAccessPolicy,
    >,
) {
    push_channel_surface_header(lines, surface);
    lines.push(render_channel_onboarding_line(&surface.catalog.onboarding));
    push_channel_surface_plugin_bridge_contract(lines, surface);
    push_channel_surface_managed_plugin_bridge_discovery(lines, surface);

    if surface.catalog.implementation_status
        == mvp::channel::ChannelCatalogImplementationStatus::Stub
    {
        for operation in &surface.catalog.operations {
            lines.push(format!(
                "  catalog op {} ({}) availability={} tracks_runtime={} target_kinds={} requirements={}",
                operation.id,
                operation.command,
                operation.availability.as_str(),
                operation.tracks_runtime,
                render_channel_target_kind_ids(operation.supported_target_kinds),
                render_channel_operation_requirement_ids(operation.requirements)
            ));
        }
        return;
    }

    for snapshot in &surface.configured_accounts {
        let api_base_url = snapshot.api_base_url.as_deref().unwrap_or("-");
        lines.push(format!(
            "  account configured_account={} configured_account_label={} default_account={} default_source={} compiled={} enabled={} api_base_url={}",
            snapshot.configured_account_id,
            snapshot.configured_account_label,
            snapshot.is_default_account,
            snapshot.default_account_source.as_str(),
            snapshot.compiled,
            snapshot.enabled,
            api_base_url
        ));
        for note in &snapshot.notes {
            lines.push(format!("    note: {note}"));
        }
        let access_policy_key = (
            surface.catalog.id.to_owned(),
            snapshot.configured_account_id.clone(),
        );
        if let Some(access_policy) = channel_access_policies.get(&access_policy_key) {
            lines.push(render_channel_access_policy_line(access_policy));
        }
        for operation in &snapshot.operations {
            let catalog_operation = surface.catalog.operation(operation.id);
            let requirement_ids = catalog_operation
                .map(|catalog_operation| {
                    render_channel_operation_requirement_ids(catalog_operation.requirements)
                })
                .unwrap_or_else(|| "-".to_owned());
            lines.push(format!(
                "    op {} ({}) {}: {} target_kinds={} requirements={}",
                operation.id,
                operation.command,
                operation.health.as_str(),
                operation.detail,
                render_channel_target_kind_ids(
                    catalog_operation
                        .map(|catalog_operation| catalog_operation.supported_target_kinds)
                        .unwrap_or(&[])
                ),
                requirement_ids,
            ));
            if let Some(runtime) = &operation.runtime {
                lines.push(format!(
                    "      runtime account={} account_id={} running={} stale={} busy={} active_runs={} instance_count={} running_instances={} stale_instances={} last_run_activity_at={} last_heartbeat_at={} pid={}",
                    runtime.account_label.as_deref().unwrap_or("-"),
                    runtime.account_id.as_deref().unwrap_or("-"),
                    runtime.running,
                    runtime.stale,
                    runtime.busy,
                    runtime.active_runs,
                    runtime.instance_count,
                    runtime.running_instances,
                    runtime.stale_instances,
                    runtime
                        .last_run_activity_at
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_owned()),
                    runtime
                        .last_heartbeat_at
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_owned()),
                    runtime
                        .pid
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_owned())
                ));
            }
            for issue in &operation.issues {
                lines.push(format!("      issue: {issue}"));
            }
        }
    }
}

pub fn render_channel_onboarding_line(
    onboarding: &mvp::channel::ChannelOnboardingDescriptor,
) -> String {
    format!(
        "  onboarding strategy={} status_command=\"{}\" repair_command={} setup_hint=\"{}\"",
        onboarding.strategy.as_str(),
        onboarding.status_command,
        onboarding
            .repair_command
            .map(|command| format!("\"{command}\""))
            .unwrap_or_else(|| "-".to_owned()),
        onboarding.setup_hint
    )
}

pub fn render_channel_operation_requirement_ids(
    requirements: &[mvp::channel::ChannelCatalogOperationRequirement],
) -> String {
    if requirements.is_empty() {
        return "-".to_owned();
    }
    requirements
        .iter()
        .map(|requirement| requirement.id)
        .collect::<Vec<_>>()
        .join(",")
}

pub fn render_channel_target_kind_ids(
    target_kinds: &[mvp::channel::ChannelCatalogTargetKind],
) -> String {
    if target_kinds.is_empty() {
        return "-".to_owned();
    }
    target_kinds
        .iter()
        .map(|kind| kind.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

pub fn push_channel_surface_header(
    lines: &mut Vec<String>,
    surface: &mvp::channel::ChannelSurface,
) {
    let aliases = if surface.catalog.aliases.is_empty() {
        "-".to_owned()
    } else {
        surface.catalog.aliases.join(",")
    };
    let capabilities = if surface.catalog.capabilities.is_empty() {
        "-".to_owned()
    } else {
        surface
            .catalog
            .capabilities
            .iter()
            .map(|capability| capability.as_str())
            .collect::<Vec<_>>()
            .join(",")
    };
    let target_kinds = render_channel_target_kind_ids(&surface.catalog.supported_target_kinds);
    lines.push(format!(
        "{} [{}] implementation_status={} selection_order={} selection_label=\"{}\" capabilities={} aliases={} transport={} target_kinds={} configured_accounts={} default_configured_account={}",
        surface.catalog.label,
        surface.catalog.id,
        surface.catalog.implementation_status.as_str(),
        surface.catalog.selection_order,
        surface.catalog.selection_label,
        capabilities,
        aliases,
        surface.catalog.transport,
        target_kinds,
        surface.configured_accounts.len(),
        surface
            .default_configured_account_id
            .as_deref()
            .unwrap_or("-")
    ));
    lines.push(format!("  blurb: {}", surface.catalog.blurb));
}

pub fn run_list_context_engines_cli(config_path: Option<&str>, as_json: bool) -> CliResult<()> {
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let snapshot = mvp::conversation::collect_context_engine_runtime_snapshot(&config)?;

    if as_json {
        let payload = json!({
            "config": resolved_path.display().to_string(),
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
                "preserve_recent_turns": snapshot.compaction.preserve_recent_turns,
                "preserve_recent_estimated_tokens": snapshot.compaction.preserve_recent_estimated_tokens,
                "fail_open": snapshot.compaction.fail_open,
                "hygiene": {
                    "strategy": snapshot.compaction.hygiene_strategy(),
                    "diagnostics_surface": snapshot.compaction.diagnostics_surface(),
                },
            },
        });
        let pretty = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("serialize context-engine output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    println!("config={}", resolved_path.display());
    println!(
        "selected={} source={} api_version={} capabilities={}",
        snapshot.selected_metadata.id,
        snapshot.selected.source.as_str(),
        snapshot.selected_metadata.api_version,
        format_capability_names(&snapshot.selected_metadata.capability_names())
    );
    println!(
        "compaction=enabled:{} min_messages:{} trigger_estimated_tokens:{} preserve_recent_turns:{} preserve_recent_estimated_tokens:{} fail_open:{} hygiene_strategy={} diagnostics_surface={}",
        snapshot.compaction.enabled,
        snapshot
            .compaction
            .min_messages
            .map_or_else(|| "(none)".to_owned(), |value| value.to_string()),
        snapshot
            .compaction
            .trigger_estimated_tokens
            .map_or_else(|| "(none)".to_owned(), |value| value.to_string()),
        snapshot.compaction.preserve_recent_turns,
        snapshot
            .compaction
            .preserve_recent_estimated_tokens
            .map_or_else(|| "(none)".to_owned(), |value| value.to_string()),
        snapshot.compaction.fail_open,
        snapshot.compaction.hygiene_strategy(),
        snapshot.compaction.diagnostics_surface(),
    );
    println!("available:");
    for metadata in snapshot.available {
        println!(
            "- {} api_version={} capabilities={}",
            metadata.id,
            metadata.api_version,
            format_capability_names(&metadata.capability_names())
        );
    }
    Ok(())
}

pub fn run_list_memory_systems_cli(config_path: Option<&str>, as_json: bool) -> CliResult<()> {
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let snapshot = mvp::memory::collect_memory_system_runtime_snapshot(&config)?;

    if as_json {
        let payload =
            build_memory_systems_cli_json_payload(&resolved_path.display().to_string(), &snapshot);
        let pretty = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("serialize memory-system output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    println!(
        "{}",
        render_memory_system_snapshot_text(&resolved_path.display().to_string(), &snapshot)
    );
    Ok(())
}

pub fn run_safe_lane_summary_cli(
    config_path: Option<&str>,
    session: Option<&str>,
    limit: usize,
    as_json: bool,
) -> CliResult<()> {
    if limit == 0 {
        return Err("safe-lane-summary limit must be >= 1".to_owned());
    }

    let (_, config) = mvp::config::load(config_path)?;
    let session_id = session
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("default")
        .to_owned();

    #[cfg(feature = "memory-sqlite")]
    {
        let mem_config =
            mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
        let turns = mvp::memory::window_direct(&session_id, limit, &mem_config)
            .map_err(|error| format!("load safe-lane summary failed: {error}"))?;
        let summary = mvp::conversation::summarize_safe_lane_events(
            turns
                .iter()
                .filter_map(|turn| (turn.role == "assistant").then_some(turn.content.as_str())),
        );
        if as_json {
            let payload = json!({
                "session": session_id,
                "limit": limit,
                "summary": summary,
            });
            let pretty = serde_json::to_string_pretty(&payload)
                .map_err(|error| format!("serialize safe-lane summary failed: {error}"))?;
            println!("{pretty}");
            return Ok(());
        }

        let final_status = match summary.final_status {
            Some(mvp::conversation::SafeLaneFinalStatus::Succeeded) => "succeeded",
            Some(mvp::conversation::SafeLaneFinalStatus::Failed) => "failed",
            None => "unknown",
        };
        println!("safe_lane_summary session={} limit={}", session_id, limit);
        println!(
            "events lane_selected={} round_started={} round_completed_succeeded={} round_completed_failed={} verify_failed={} verify_policy_adjusted={} replan_triggered={} final_status={} governor_engaged={} governor_force_no_replan={}",
            summary.lane_selected_events,
            summary.round_started_events,
            summary.round_completed_succeeded_events,
            summary.round_completed_failed_events,
            summary.verify_failed_events,
            summary.verify_policy_adjusted_events,
            summary.replan_triggered_events,
            summary.final_status_events,
            summary.session_governor_engaged_events,
            summary.session_governor_force_no_replan_events
        );
        println!(
            "terminal status={} failure_code={} route_decision={} route_reason={}",
            final_status,
            summary.final_failure_code.as_deref().unwrap_or("-"),
            summary.final_route_decision.as_deref().unwrap_or("-"),
            summary.final_route_reason.as_deref().unwrap_or("-")
        );
        let route_reasons_rollup = if summary.route_reason_counts.is_empty() {
            "-".to_owned()
        } else {
            summary
                .route_reason_counts
                .iter()
                .map(|(key, value)| format!("{key}:{value}"))
                .collect::<Vec<_>>()
                .join(",")
        };
        println!(
            "governor trigger_failed_threshold={} trigger_backpressure_threshold={} trigger_trend_threshold={} trigger_recovery_threshold={}",
            summary.session_governor_failed_threshold_triggered_events,
            summary.session_governor_backpressure_threshold_triggered_events,
            summary.session_governor_trend_threshold_triggered_events,
            summary.session_governor_recovery_threshold_triggered_events
        );
        println!(
            "governor_latest snapshots={} trend_samples={} trend_min_samples={} trend_failure_ewma={} trend_backpressure_ewma={} recovery_success_streak={} recovery_streak_threshold={}",
            summary.session_governor_metrics_snapshots_seen,
            summary
                .session_governor_latest_trend_samples
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            summary
                .session_governor_latest_trend_min_samples
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            format_milli_ratio(summary.session_governor_latest_trend_failure_ewma_milli),
            format_milli_ratio(summary.session_governor_latest_trend_backpressure_ewma_milli),
            summary
                .session_governor_latest_recovery_success_streak
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            summary
                .session_governor_latest_recovery_success_streak_threshold
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned())
        );
        println!("rollup route_reasons={route_reasons_rollup}");
        Ok(())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (config, session_id, as_json);
        Err("safe-lane-summary requires memory-sqlite feature".to_owned())
    }
}

#[cfg(feature = "memory-sqlite")]
pub fn format_capability_names(names: &[&str]) -> String {
    if names.is_empty() {
        return "(none)".to_owned();
    }
    names.join(",")
}

pub fn format_milli_ratio(value: Option<u32>) -> String {
    value
        .map(|raw| format!("{:.3}", (raw as f64) / 1000.0))
        .unwrap_or_else(|| "-".to_owned())
}
