use std::{collections::BTreeSet, path::Path, sync::Arc};

use axum::Router;

use crate::{
    CliResult, KernelContext,
    config::LoongConfig,
    context::{DEFAULT_TOKEN_TTL_S, bootstrap_kernel_context_with_config},
};

use super::{
    CHANNEL_OPERATION_SERVE_ID, ChannelPlatform, ChannelServeRuntimeSpec,
    ensure_channel_operation_runtime_slot_available_in_dir,
    runtime::state::ChannelOperationRuntimeTracker,
};

#[cfg(feature = "channel-feishu")]
use super::{
    FEISHU_RUNTIME_COMMAND_DESCRIPTOR, dispatch::validate_feishu_security_config,
    feishu::build_gateway_feishu_ingress_router,
};

#[cfg(feature = "channel-line")]
use super::{
    LINE_RUNTIME_COMMAND_DESCRIPTOR,
    line::{build_gateway_line_ingress_router, gateway_line_ingress_path},
};

#[cfg(feature = "channel-webhook")]
use super::{
    WEBHOOK_RUNTIME_COMMAND_DESCRIPTOR,
    webhook::{build_gateway_webhook_ingress_router, gateway_webhook_ingress_path},
};

#[cfg(feature = "channel-whatsapp")]
use super::{WHATSAPP_RUNTIME_COMMAND_DESCRIPTOR, whatsapp::build_gateway_whatsapp_ingress_router};

pub struct GatewayIngressMount {
    router: Router,
    runtime_trackers: Vec<Arc<ChannelOperationRuntimeTracker>>,
}

impl GatewayIngressMount {
    pub fn into_parts(self) -> (Router, Vec<Arc<ChannelOperationRuntimeTracker>>) {
        (self.router, self.runtime_trackers)
    }
}

pub async fn build_gateway_ingress(
    resolved_path: &Path,
    config: &LoongConfig,
) -> CliResult<GatewayIngressMount> {
    let mut router = Router::new();
    let mut runtime_trackers = Vec::new();
    let mut registered_paths = BTreeSet::new();

    let build_result = async {
        mount_feishu_gateway_ingress(
            &mut router,
            &mut runtime_trackers,
            &mut registered_paths,
            resolved_path,
            config,
        )
        .await?;
        mount_line_gateway_ingress(
            &mut router,
            &mut runtime_trackers,
            &mut registered_paths,
            resolved_path,
            config,
        )
        .await?;
        mount_whatsapp_gateway_ingress(
            &mut router,
            &mut runtime_trackers,
            &mut registered_paths,
            resolved_path,
            config,
        )
        .await?;
        mount_webhook_gateway_ingress(
            &mut router,
            &mut runtime_trackers,
            &mut registered_paths,
            resolved_path,
            config,
        )
        .await?;
        Ok::<(), String>(())
    }
    .await;

    if let Err(error) = build_result {
        let shutdown_error = shutdown_gateway_ingress_runtimes(runtime_trackers)
            .await
            .err();
        let final_error = match shutdown_error {
            Some(shutdown_error) => format!("{error}; {shutdown_error}"),
            None => error,
        };
        return Err(final_error);
    }

    Ok(GatewayIngressMount {
        router,
        runtime_trackers,
    })
}

pub fn gateway_owned_runtime_channel_ids(
    config: &LoongConfig,
) -> CliResult<BTreeSet<&'static str>> {
    let mut channel_ids = BTreeSet::new();
    for descriptor in super::gateway_ingress_channel_descriptors() {
        if super::is_gateway_ingress_channel_enabled(descriptor.id, config, None)? {
            channel_ids.insert(descriptor.id);
        }
    }
    Ok(channel_ids)
}

#[cfg(feature = "channel-feishu")]
async fn mount_feishu_gateway_ingress(
    router: &mut Router,
    runtime_trackers: &mut Vec<Arc<ChannelOperationRuntimeTracker>>,
    registered_paths: &mut BTreeSet<String>,
    resolved_path: &Path,
    config: &LoongConfig,
) -> CliResult<()> {
    if !super::is_gateway_ingress_channel_enabled("feishu", config, None)? {
        return Ok(());
    }

    for configured_account_id in
        configured_account_ids_or_default(config.feishu.configured_account_ids())
    {
        let resolved = config
            .feishu
            .resolve_account(configured_account_id.as_deref())?;
        validate_feishu_security_config(&resolved)?;
        let path = normalize_http_path(resolved.webhook_path.as_str());
        register_gateway_ingress_path(
            registered_paths,
            "feishu",
            configured_account_id.as_deref(),
            path.as_str(),
        )?;
        let runtime = start_gateway_ingress_runtime(
            FEISHU_RUNTIME_COMMAND_DESCRIPTOR.platform,
            resolved.account.id.as_str(),
            resolved.account.label.as_str(),
        )
        .await?;
        let channel_router = build_gateway_feishu_ingress_router(
            config,
            &resolved,
            resolved_path,
            bootstrap_channel_kernel_context(
                FEISHU_RUNTIME_COMMAND_DESCRIPTOR.serve_bootstrap_agent_id,
                config,
            )?,
            runtime.clone(),
        )
        .await?;
        *router = std::mem::take(router).merge(channel_router);
        runtime_trackers.push(runtime);
    }

    Ok(())
}

#[cfg(not(feature = "channel-feishu"))]
async fn mount_feishu_gateway_ingress(
    _router: &mut Router,
    _runtime_trackers: &mut Vec<Arc<ChannelOperationRuntimeTracker>>,
    _registered_paths: &mut BTreeSet<String>,
    _resolved_path: &Path,
    _config: &LoongConfig,
) -> CliResult<()> {
    Ok(())
}

#[cfg(feature = "channel-whatsapp")]
async fn mount_whatsapp_gateway_ingress(
    router: &mut Router,
    runtime_trackers: &mut Vec<Arc<ChannelOperationRuntimeTracker>>,
    registered_paths: &mut BTreeSet<String>,
    resolved_path: &Path,
    config: &LoongConfig,
) -> CliResult<()> {
    if !super::is_gateway_ingress_channel_enabled("whatsapp", config, None)? {
        return Ok(());
    }

    for configured_account_id in
        configured_account_ids_or_default(config.whatsapp.configured_account_ids())
    {
        let resolved = config
            .whatsapp
            .resolve_account(configured_account_id.as_deref())?;
        let path = normalize_http_path(resolved.resolved_webhook_path().as_str());
        register_gateway_ingress_path(
            registered_paths,
            "whatsapp",
            configured_account_id.as_deref(),
            path.as_str(),
        )?;
        let runtime = start_gateway_ingress_runtime(
            WHATSAPP_RUNTIME_COMMAND_DESCRIPTOR.platform,
            resolved.account.id.as_str(),
            resolved.account.label.as_str(),
        )
        .await?;
        let channel_router = build_gateway_whatsapp_ingress_router(
            config,
            &resolved,
            resolved_path,
            bootstrap_channel_kernel_context(
                WHATSAPP_RUNTIME_COMMAND_DESCRIPTOR.serve_bootstrap_agent_id,
                config,
            )?,
            runtime.clone(),
        )?;
        *router = std::mem::take(router).merge(channel_router);
        runtime_trackers.push(runtime);
    }

    Ok(())
}

#[cfg(not(feature = "channel-whatsapp"))]
async fn mount_whatsapp_gateway_ingress(
    _router: &mut Router,
    _runtime_trackers: &mut Vec<Arc<ChannelOperationRuntimeTracker>>,
    _registered_paths: &mut BTreeSet<String>,
    _resolved_path: &Path,
    _config: &LoongConfig,
) -> CliResult<()> {
    Ok(())
}

#[cfg(feature = "channel-line")]
async fn mount_line_gateway_ingress(
    router: &mut Router,
    runtime_trackers: &mut Vec<Arc<ChannelOperationRuntimeTracker>>,
    registered_paths: &mut BTreeSet<String>,
    resolved_path: &Path,
    config: &LoongConfig,
) -> CliResult<()> {
    if !super::is_gateway_ingress_channel_enabled("line", config, None)? {
        return Ok(());
    }

    for configured_account_id in
        configured_account_ids_or_default(config.line.configured_account_ids())
    {
        let resolved = config
            .line
            .resolve_account(configured_account_id.as_deref())?;
        let path = gateway_line_ingress_path(&resolved);
        register_gateway_ingress_path(
            registered_paths,
            "line",
            configured_account_id.as_deref(),
            path.as_str(),
        )?;
        let runtime = start_gateway_ingress_runtime(
            LINE_RUNTIME_COMMAND_DESCRIPTOR.platform,
            resolved.account.id.as_str(),
            resolved.account.label.as_str(),
        )
        .await?;
        let channel_router = build_gateway_line_ingress_router(
            config,
            &resolved,
            resolved_path,
            bootstrap_channel_kernel_context(
                LINE_RUNTIME_COMMAND_DESCRIPTOR.serve_bootstrap_agent_id,
                config,
            )?,
            runtime.clone(),
        )?;
        *router = std::mem::take(router).merge(channel_router);
        runtime_trackers.push(runtime);
    }

    Ok(())
}

#[cfg(not(feature = "channel-line"))]
async fn mount_line_gateway_ingress(
    _router: &mut Router,
    _runtime_trackers: &mut Vec<Arc<ChannelOperationRuntimeTracker>>,
    _registered_paths: &mut BTreeSet<String>,
    _resolved_path: &Path,
    _config: &LoongConfig,
) -> CliResult<()> {
    Ok(())
}

#[cfg(feature = "channel-webhook")]
async fn mount_webhook_gateway_ingress(
    router: &mut Router,
    runtime_trackers: &mut Vec<Arc<ChannelOperationRuntimeTracker>>,
    registered_paths: &mut BTreeSet<String>,
    resolved_path: &Path,
    config: &LoongConfig,
) -> CliResult<()> {
    if !super::is_gateway_ingress_channel_enabled("webhook", config, None)? {
        return Ok(());
    }

    for configured_account_id in
        configured_account_ids_or_default(config.webhook.configured_account_ids())
    {
        let resolved = config
            .webhook
            .resolve_account(configured_account_id.as_deref())?;
        let path = gateway_webhook_ingress_path(&resolved)?;
        register_gateway_ingress_path(
            registered_paths,
            "webhook",
            configured_account_id.as_deref(),
            path.as_str(),
        )?;
        let runtime = start_gateway_ingress_runtime(
            WEBHOOK_RUNTIME_COMMAND_DESCRIPTOR.platform,
            resolved.account.id.as_str(),
            resolved.account.label.as_str(),
        )
        .await?;
        let channel_router = build_gateway_webhook_ingress_router(
            config,
            &resolved,
            resolved_path,
            bootstrap_channel_kernel_context(
                WEBHOOK_RUNTIME_COMMAND_DESCRIPTOR.serve_bootstrap_agent_id,
                config,
            )?,
            runtime.clone(),
        )?;
        *router = std::mem::take(router).merge(channel_router);
        runtime_trackers.push(runtime);
    }

    Ok(())
}

#[cfg(not(feature = "channel-webhook"))]
async fn mount_webhook_gateway_ingress(
    _router: &mut Router,
    _runtime_trackers: &mut Vec<Arc<ChannelOperationRuntimeTracker>>,
    _registered_paths: &mut BTreeSet<String>,
    _resolved_path: &Path,
    _config: &LoongConfig,
) -> CliResult<()> {
    Ok(())
}

fn configured_account_ids_or_default(configured_ids: Vec<String>) -> Vec<Option<String>> {
    if configured_ids.is_empty() {
        vec![None]
    } else {
        configured_ids.into_iter().map(Some).collect()
    }
}

fn normalize_http_path(raw_path: &str) -> String {
    let trimmed = raw_path.trim();
    if trimmed.starts_with('/') {
        trimmed.to_owned()
    } else {
        format!("/{trimmed}")
    }
}

fn register_gateway_ingress_path(
    registered_paths: &mut BTreeSet<String>,
    channel_id: &str,
    configured_account_id: Option<&str>,
    path: &str,
) -> CliResult<()> {
    let owned_path = path.to_owned();
    if registered_paths.insert(owned_path.clone()) {
        return Ok(());
    }
    let account = configured_account_id.unwrap_or("default");
    Err(format!(
        "gateway ingress path collision for {channel_id} account `{account}` at `{owned_path}`"
    ))
}

async fn start_gateway_ingress_runtime(
    platform: ChannelPlatform,
    account_id: &str,
    account_label: &str,
) -> CliResult<Arc<ChannelOperationRuntimeTracker>> {
    ensure_channel_operation_runtime_slot_available_in_dir(
        crate::channel::runtime::state::default_channel_runtime_state_dir().as_path(),
        ChannelServeRuntimeSpec {
            platform,
            operation_id: CHANNEL_OPERATION_SERVE_ID,
            account_id,
            account_label,
        },
    )?;
    let runtime = ChannelOperationRuntimeTracker::start(
        platform,
        CHANNEL_OPERATION_SERVE_ID,
        account_id,
        account_label,
    )
    .await?;
    Ok(Arc::new(runtime))
}

fn bootstrap_channel_kernel_context(
    bootstrap_agent_id: &str,
    config: &LoongConfig,
) -> CliResult<KernelContext> {
    bootstrap_kernel_context_with_config(bootstrap_agent_id, DEFAULT_TOKEN_TTL_S, config)
}

pub async fn shutdown_gateway_ingress_runtimes(
    runtime_trackers: Vec<Arc<ChannelOperationRuntimeTracker>>,
) -> CliResult<()> {
    let mut errors = Vec::new();
    for runtime in runtime_trackers {
        if let Err(error) = runtime.shutdown().await {
            errors.push(error);
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "channel-feishu")]
    use crate::config::FeishuChannelServeMode;

    #[cfg(feature = "channel-feishu")]
    fn configure_feishu_gateway_ingress(
        config: &mut LoongConfig,
        expected: &mut BTreeSet<&'static str>,
    ) {
        config.feishu.enabled = true;
        config.feishu.mode = Some(FeishuChannelServeMode::Webhook);
        expected.insert("feishu");
    }

    #[cfg(not(feature = "channel-feishu"))]
    fn configure_feishu_gateway_ingress(
        _config: &mut LoongConfig,
        _expected: &mut BTreeSet<&'static str>,
    ) {
    }

    #[cfg(feature = "channel-whatsapp")]
    fn configure_whatsapp_gateway_ingress(
        config: &mut LoongConfig,
        expected: &mut BTreeSet<&'static str>,
    ) {
        config.whatsapp.enabled = true;
        expected.insert("whatsapp");
    }

    #[cfg(not(feature = "channel-whatsapp"))]
    fn configure_whatsapp_gateway_ingress(
        _config: &mut LoongConfig,
        _expected: &mut BTreeSet<&'static str>,
    ) {
    }

    #[cfg(feature = "channel-line")]
    fn configure_line_gateway_ingress(
        config: &mut LoongConfig,
        expected: &mut BTreeSet<&'static str>,
    ) {
        config.line.enabled = true;
        expected.insert("line");
    }

    #[cfg(not(feature = "channel-line"))]
    fn configure_line_gateway_ingress(
        _config: &mut LoongConfig,
        _expected: &mut BTreeSet<&'static str>,
    ) {
    }

    #[cfg(feature = "channel-webhook")]
    fn configure_webhook_gateway_ingress(
        config: &mut LoongConfig,
        expected: &mut BTreeSet<&'static str>,
    ) {
        config.webhook.enabled = true;
        expected.insert("webhook");
    }

    #[cfg(not(feature = "channel-webhook"))]
    fn configure_webhook_gateway_ingress(
        _config: &mut LoongConfig,
        _expected: &mut BTreeSet<&'static str>,
    ) {
    }

    #[test]
    fn gateway_owned_runtime_channel_ids_follow_http_gateway_policy() {
        let mut config = LoongConfig::default();
        let mut expected = BTreeSet::new();
        configure_feishu_gateway_ingress(&mut config, &mut expected);
        configure_whatsapp_gateway_ingress(&mut config, &mut expected);
        configure_line_gateway_ingress(&mut config, &mut expected);
        configure_webhook_gateway_ingress(&mut config, &mut expected);

        let owned = gateway_owned_runtime_channel_ids(&config)
            .expect("resolve gateway owned runtime channel ids");

        assert_eq!(owned, expected);
    }

    #[test]
    fn register_gateway_ingress_path_rejects_duplicates() {
        let mut registered = BTreeSet::new();
        register_gateway_ingress_path(&mut registered, "feishu", Some("work"), "/ingress")
            .expect("first registration");
        let error =
            register_gateway_ingress_path(&mut registered, "whatsapp", Some("ops"), "/ingress")
                .expect_err("duplicate path should fail");
        assert!(error.contains("path collision"));
    }

    #[test]
    #[cfg(feature = "channel-line")]
    fn gateway_line_ingress_path_uses_account_scoped_route() {
        let mut config = LoongConfig::default();
        config.line.enabled = true;
        config.line.accounts.insert(
            "Marketing".to_owned(),
            crate::config::LineAccountConfig {
                enabled: Some(true),
                channel_access_token: Some(loong_contracts::SecretRef::Inline("token".to_owned())),
                channel_secret: Some(loong_contracts::SecretRef::Inline("secret".to_owned())),
                ..Default::default()
            },
        );

        let resolved = config
            .line
            .resolve_account(Some("Marketing"))
            .expect("resolve line account");

        assert_eq!(
            gateway_line_ingress_path(&resolved),
            "/ingress/line/marketing"
        );
    }

    #[test]
    #[cfg(feature = "channel-webhook")]
    fn gateway_webhook_ingress_path_prefers_public_path_and_falls_back_to_account_route() {
        let mut config = LoongConfig::default();
        config.webhook.enabled = true;
        config.webhook.accounts.insert(
            "Ops".to_owned(),
            crate::config::WebhookAccountConfig {
                enabled: Some(true),
                public_base_url: Some("https://example.test/hooks/ops".to_owned()),
                signing_secret: Some(loong_contracts::SecretRef::Inline("secret".to_owned())),
                ..Default::default()
            },
        );
        config.webhook.accounts.insert(
            "Fallback".to_owned(),
            crate::config::WebhookAccountConfig {
                enabled: Some(true),
                signing_secret: Some(loong_contracts::SecretRef::Inline("secret".to_owned())),
                ..Default::default()
            },
        );

        let ops = config
            .webhook
            .resolve_account(Some("Ops"))
            .expect("resolve ops webhook account");
        let fallback = config
            .webhook
            .resolve_account(Some("Fallback"))
            .expect("resolve fallback webhook account");

        assert_eq!(
            gateway_webhook_ingress_path(&ops).expect("ops path"),
            "/hooks/ops"
        );
        assert_eq!(
            gateway_webhook_ingress_path(&fallback).expect("fallback path"),
            "/ingress/webhook/fallback"
        );
    }
}
