use super::*;

pub(super) async fn dashboard_summary(
    State(state): State<Arc<WebApiState>>,
) -> Result<Json<ApiEnvelope<DashboardSummaryPayload>>, WebApiError> {
    let snapshot = load_web_snapshot(state.as_ref())?;
    Ok(Json(ApiEnvelope {
        ok: true,
        data: DashboardSummaryPayload {
            runtime_status: "ready",
            active_provider: snapshot.config.active_provider_id().map(str::to_owned),
            active_model: snapshot.config.provider.model.clone(),
            memory_backend: "sqlite",
            session_count: snapshot.sessions.len(),
            web_install_mode: state.web_install_mode,
        },
    }))
}

pub(super) async fn dashboard_providers(
    State(state): State<Arc<WebApiState>>,
) -> Result<Json<ApiEnvelope<DashboardProvidersPayload>>, WebApiError> {
    let snapshot = load_web_snapshot(state.as_ref())?;
    Ok(Json(ApiEnvelope {
        ok: true,
        data: DashboardProvidersPayload {
            active_provider: snapshot.config.active_provider_id().map(str::to_owned),
            items: build_provider_items(&snapshot.config),
        },
    }))
}

pub(super) async fn dashboard_runtime(
    State(state): State<Arc<WebApiState>>,
) -> Result<Json<ApiEnvelope<DashboardRuntimePayload>>, WebApiError> {
    let snapshot = load_web_snapshot(state.as_ref())?;
    Ok(Json(ApiEnvelope {
        ok: true,
        data: DashboardRuntimePayload {
            status: "ready",
            source: "local_daemon",
            config_path: snapshot.resolved_path.display().to_string(),
            memory_backend: snapshot.config.memory.resolved_backend().as_str(),
            memory_mode: snapshot.config.memory.resolved_mode().as_str(),
            ingest_mode: snapshot.config.memory.ingest_mode.as_str(),
            web_install_mode: state.web_install_mode,
            active_provider: snapshot.config.active_provider_id().map(str::to_owned),
            active_model: snapshot.config.provider.model.clone(),
            acp_enabled: snapshot.config.acp.enabled,
            strict_memory: !snapshot.config.memory.effective_fail_open(),
        },
    }))
}

pub(super) async fn dashboard_connectivity(
    State(state): State<Arc<WebApiState>>,
) -> Result<Json<ApiEnvelope<DashboardConnectivityPayload>>, WebApiError> {
    let snapshot = load_web_snapshot(state.as_ref())?;
    let endpoint = snapshot.config.provider.endpoint();
    let parsed = reqwest::Url::parse(&endpoint).map_err(|error| {
        WebApiError::internal(format!(
            "parse provider endpoint for connectivity failed: {error}"
        ))
    })?;
    let host = parsed
        .host_str()
        .ok_or_else(|| WebApiError::internal("provider endpoint host was missing"))?
        .to_owned();
    let port = parsed.port_or_known_default().unwrap_or(443);
    let dns_addresses = resolve_provider_host_addresses(host.as_str(), port).await;
    let fake_ip_detected = dns_addresses
        .iter()
        .any(|address| is_fake_ip_address(address));
    let proxy_env_detected = has_proxy_environment();
    let (probe_status, probe_status_code) = probe_provider_endpoint(endpoint.as_str()).await;
    let degraded = fake_ip_detected || probe_status != "reachable";
    let recommendation = if fake_ip_detected {
        Some("direct_host_and_fake_ip_filter")
    } else if probe_status != "reachable" {
        Some("check_network_route")
    } else {
        None
    };

    Ok(Json(ApiEnvelope {
        ok: true,
        data: DashboardConnectivityPayload {
            status: if degraded { "degraded" } else { "healthy" },
            endpoint,
            host,
            dns_addresses,
            probe_status,
            probe_status_code,
            fake_ip_detected,
            proxy_env_detected,
            recommendation,
        },
    }))
}

pub(super) async fn dashboard_config(
    State(state): State<Arc<WebApiState>>,
) -> Result<Json<ApiEnvelope<DashboardConfigPayload>>, WebApiError> {
    let snapshot = load_web_snapshot(state.as_ref())?;
    let active_provider = build_provider_items(&snapshot.config)
        .into_iter()
        .find(|item| item.enabled);

    Ok(Json(ApiEnvelope {
        ok: true,
        data: DashboardConfigPayload {
            active_provider: snapshot.config.active_provider_id().map(str::to_owned),
            last_provider: snapshot.config.last_provider.clone(),
            model: snapshot.config.provider.model.clone(),
            endpoint: snapshot.config.provider.endpoint(),
            api_key_configured: active_provider
                .as_ref()
                .map(|item| item.api_key_configured)
                .unwrap_or(false),
            api_key_masked: active_provider.and_then(|item| item.api_key_masked),
            personality: prompt_personality_id(snapshot.config.cli.resolved_personality())
                .to_owned(),
            prompt_mode: if snapshot.config.cli.uses_native_prompt_pack() {
                "native_prompt_pack"
            } else {
                "inline_prompt"
            },
            prompt_addendum_configured: snapshot
                .config
                .cli
                .system_prompt_addendum
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty()),
            prompt_addendum: snapshot
                .config
                .cli
                .system_prompt_addendum
                .clone()
                .unwrap_or_default(),
            memory_profile: snapshot
                .config
                .memory
                .resolved_profile()
                .as_str()
                .to_owned(),
            memory_system: snapshot.config.memory.resolved_system().as_str(),
            sqlite_path: snapshot
                .config
                .memory
                .resolved_sqlite_path()
                .display()
                .to_string(),
            file_root: snapshot
                .config
                .tools
                .resolved_file_root()
                .display()
                .to_string(),
            sliding_window: snapshot.config.memory.sliding_window,
            summary_max_chars: snapshot.config.memory.summary_max_chars,
        },
    }))
}

pub(super) async fn dashboard_tools(
    State(state): State<Arc<WebApiState>>,
) -> Result<Json<ApiEnvelope<DashboardToolsPayload>>, WebApiError> {
    let snapshot = load_web_snapshot(state.as_ref())?;
    let tool_runtime = mvp::tools::runtime_config::ToolRuntimeConfig::from_loongclaw_config(
        &snapshot.config,
        None,
    );
    Ok(Json(ApiEnvelope {
        ok: true,
        data: DashboardToolsPayload {
            approval_mode: approval_mode_label(snapshot.config.tools.approval.mode).to_owned(),
            shell_default_mode: snapshot.config.tools.shell_default_mode.clone(),
            shell_allow_count: snapshot.config.tools.shell_allow.len(),
            shell_deny_count: snapshot.config.tools.shell_deny.len(),
            items: build_tool_items(&snapshot.config, &tool_runtime),
        },
    }))
}
