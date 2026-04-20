use serde::Serialize;

use crate::{CliResult, mvp};

#[derive(Debug, Clone, Serialize)]
pub struct ChannelResolveOutput {
    pub config: String,
    pub input: String,
    pub resolution: ChannelResolveReadModel,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChannelCatalogResolutionDetails {
    pub canonical_channel_id: String,
    pub catalog: mvp::channel::ChannelCatalogEntry,
    pub surface: Option<mvp::channel::ChannelSurface>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChannelSessionResolutionDetails {
    pub route_session_id: String,
    pub target: mvp::channel::ResolvedKnownChannelSessionTarget,
    pub surface: Option<mvp::channel::ChannelSurface>,
    pub matched_configured_account_id: Option<String>,
    pub matched_account: Option<mvp::channel::ChannelStatusSnapshot>,
    pub send_operation: Option<mvp::channel::ChannelOperationStatus>,
    pub serve_operation: Option<mvp::channel::ChannelOperationStatus>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChannelResolveReadModel {
    Catalog(Box<ChannelCatalogResolutionDetails>),
    Session(Box<ChannelSessionResolutionDetails>),
}

pub fn build_channel_resolution(
    config_path: &str,
    config: &mvp::config::LoongConfig,
    inventory: &mvp::channel::ChannelInventory,
    input: &str,
) -> CliResult<ChannelResolveOutput> {
    let trimmed_input = input.trim();
    if trimmed_input.is_empty() {
        return Err("channels --resolve requires a non-empty query".to_owned());
    }

    if let Ok(target) = mvp::channel::resolve_known_channel_session_target(config, trimmed_input) {
        let surface = inventory
            .channel_surfaces
            .iter()
            .find(|surface| surface.catalog.id == target.channel_id)
            .cloned();
        let matched_configured_account_id =
            matched_configured_account_id_for_target(config, &target)?;
        let matched_account = surface.as_ref().and_then(|surface| {
            matched_configured_account_id
                .as_deref()
                .and_then(|configured_account_id| {
                    surface
                        .configured_accounts
                        .iter()
                        .find(|snapshot| snapshot.configured_account_id == configured_account_id)
                        .cloned()
                })
        });
        let send_operation = matched_account
            .as_ref()
            .and_then(|account| account.operation(mvp::channel::CHANNEL_OPERATION_SEND_ID))
            .cloned();
        let serve_operation = matched_account
            .as_ref()
            .and_then(|account| account.operation(mvp::channel::CHANNEL_OPERATION_SERVE_ID))
            .cloned();

        return Ok(ChannelResolveOutput {
            config: config_path.to_owned(),
            input: trimmed_input.to_owned(),
            resolution: ChannelResolveReadModel::Session(Box::new(
                ChannelSessionResolutionDetails {
                    route_session_id: trimmed_input.to_owned(),
                    target,
                    surface,
                    matched_configured_account_id,
                    matched_account,
                    send_operation,
                    serve_operation,
                },
            )),
        });
    }

    let catalog = mvp::channel::resolve_channel_catalog_entry(trimmed_input)
        .ok_or_else(|| format!("unknown channel or route-session `{trimmed_input}`"))?;
    let canonical_channel_id = catalog.id.to_owned();
    let surface = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == catalog.id)
        .cloned();

    Ok(ChannelResolveOutput {
        config: config_path.to_owned(),
        input: trimmed_input.to_owned(),
        resolution: ChannelResolveReadModel::Catalog(Box::new(ChannelCatalogResolutionDetails {
            canonical_channel_id,
            catalog,
            surface,
        })),
    })
}

pub fn render_channel_resolution_text(resolution: &ChannelResolveOutput) -> String {
    let mut lines = vec![
        format!("config={}", resolution.config),
        format!("input={}", resolution.input),
    ];

    match &resolution.resolution {
        ChannelResolveReadModel::Catalog(details) => {
            let canonical_channel_id = details.canonical_channel_id.as_str();
            let catalog = &details.catalog;
            let surface = details.surface.as_ref();
            lines.push("resolve_kind=catalog".to_owned());
            lines.push(format!("channel_id={canonical_channel_id}"));
            lines.push(format!("label={}", catalog.label));
            lines.push(format!(
                "implementation_status={}",
                catalog.implementation_status.as_str()
            ));
            lines.push(format!("transport={}", catalog.transport));
            lines.push(format!("selection_order={}", catalog.selection_order));
            lines.push(format!("selection_label={}", catalog.selection_label));
            lines.push(format!(
                "supported_target_kinds={}",
                crate::render_channel_target_kind_ids(catalog.supported_target_kinds.as_slice())
            ));
            if let Some(surface) = surface {
                lines.push(format!(
                    "configured_accounts={} default_configured_account={}",
                    surface.configured_accounts.len(),
                    surface
                        .default_configured_account_id
                        .as_deref()
                        .unwrap_or("-")
                ));
            }
            for operation in &catalog.operations {
                lines.push(format!(
                    "operation id={} command={} availability={} tracks_runtime={} default_target_kind={} supported_target_kinds={}",
                    operation.id,
                    operation.command,
                    operation.availability.as_str(),
                    operation.tracks_runtime,
                    operation
                        .default_target_kind()
                        .map(mvp::channel::ChannelCatalogTargetKind::as_str)
                        .unwrap_or("-"),
                    crate::render_channel_target_kind_ids(operation.supported_target_kinds),
                ));
            }
        }
        ChannelResolveReadModel::Session(details) => {
            let route_session_id = details.route_session_id.as_str();
            let target = &details.target;
            let surface = details.surface.as_ref();
            let matched_configured_account_id = details.matched_configured_account_id.as_ref();
            let matched_account = details.matched_account.as_ref();
            let send_operation = details.send_operation.as_ref();
            let serve_operation = details.serve_operation.as_ref();
            lines.push("resolve_kind=session".to_owned());
            lines.push(format!("route_session_id={route_session_id}"));
            lines.push(format!("channel_id={}", target.channel_id));
            lines.push(format!("session_shape={}", target.session_shape));
            lines.push(format!("target_kind={}", target.target_kind.as_str()));
            lines.push(format!("target_id={}", target.target_id));
            lines.push(format!(
                "account_id={}",
                target.account_id.as_deref().unwrap_or("-")
            ));
            lines.push(format!(
                "matched_configured_account={}",
                matched_configured_account_id
                    .map(String::as_str)
                    .unwrap_or("-")
            ));
            lines.push(format!(
                "conversation_id={}",
                target.conversation_id.as_deref().unwrap_or("-")
            ));
            lines.push(format!(
                "participant_id={}",
                target.participant_id.as_deref().unwrap_or("-")
            ));
            lines.push(format!(
                "thread_id={}",
                target.thread_id.as_deref().unwrap_or("-")
            ));
            lines.push(format!(
                "reply_message_id={}",
                target.reply_message_id.as_deref().unwrap_or("-")
            ));
            lines.push(format!(
                "chat_type={}",
                target
                    .chat_type
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned())
            ));
            lines.push(format!(
                "raw_scope={}",
                if target.raw_scope.is_empty() {
                    "-".to_owned()
                } else {
                    target.raw_scope.join(":")
                }
            ));
            if let Some(surface) = surface {
                lines.push(format!("label={}", surface.catalog.label));
                lines.push(format!(
                    "implementation_status={}",
                    surface.catalog.implementation_status.as_str()
                ));
            }
            if let Some(matched_account) = matched_account {
                lines.push(format!(
                    "matched_account_enabled={} api_base_url={}",
                    matched_account.enabled,
                    matched_account.api_base_url.as_deref().unwrap_or("-")
                ));
            }
            if let Some(send_operation) = send_operation {
                lines.push(format!(
                    "send_health={} send_command={} send_detail={}",
                    send_operation.health.as_str(),
                    send_operation.command,
                    send_operation.detail,
                ));
            }
            if let Some(serve_operation) = serve_operation {
                lines.push(format!(
                    "serve_health={} serve_command={} serve_detail={}",
                    serve_operation.health.as_str(),
                    serve_operation.command,
                    serve_operation.detail,
                ));
            }
        }
    }

    lines.join("\n")
}

fn matched_configured_account_id_for_target(
    config: &mvp::config::LoongConfig,
    target: &mvp::channel::ResolvedKnownChannelSessionTarget,
) -> CliResult<Option<String>> {
    match target.channel_id.as_str() {
        "telegram" => config
            .telegram
            .resolve_account_for_session_account_id(target.account_id.as_deref())
            .map(|resolved| Some(resolved.configured_account_id)),
        "feishu" => config
            .feishu
            .resolve_account_for_session_account_id(target.account_id.as_deref())
            .map(|resolved| Some(resolved.configured_account_id)),
        "matrix" => config
            .matrix
            .resolve_account_for_session_account_id(target.account_id.as_deref())
            .map(|resolved| Some(resolved.configured_account_id)),
        "wecom" => config
            .wecom
            .resolve_account_for_session_account_id(target.account_id.as_deref())
            .map(|resolved| Some(resolved.configured_account_id)),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_resolution_prefers_known_session_targets_over_catalog_aliases() {
        let config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
            "telegram": {
                "enabled": true,
                "bot_token": "123456:test-token",
                "allowed_chat_ids": [123]
            }
        }))
        .expect("deserialize telegram config");
        let inventory = mvp::channel::channel_inventory(&config);

        let resolution =
            build_channel_resolution("/tmp/loong.toml", &config, &inventory, "telegram:123")
                .expect("resolve session");

        match resolution.resolution {
            ChannelResolveReadModel::Session(details) => {
                let target = &details.target;
                assert_eq!(target.channel_id, "telegram");
                assert_eq!(target.target_id, "123");
            }
            other => panic!("expected session resolution, got {other:?}"),
        }
    }

    #[test]
    fn channel_resolution_resolves_catalog_aliases() {
        let config = mvp::config::LoongConfig::default();
        let inventory = mvp::channel::channel_inventory(&config);

        let resolution = build_channel_resolution("/tmp/loong.toml", &config, &inventory, "lark")
            .expect("resolve catalog");

        match resolution.resolution {
            ChannelResolveReadModel::Catalog(details) => {
                let canonical_channel_id = details.canonical_channel_id.as_str();
                let catalog = &details.catalog;
                assert_eq!(canonical_channel_id, "feishu");
                assert_eq!(catalog.id, "feishu");
            }
            other => panic!("expected catalog resolution, got {other:?}"),
        }
    }

    #[test]
    fn channel_resolution_text_renders_known_session_summary() {
        let config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
            "telegram": {
                "enabled": true,
                "accounts": {
                    "ops": {
                        "account_id": "Ops-Bot",
                        "bot_token": "123456:test-token",
                        "allowed_chat_ids": [123]
                    }
                }
            }
        }))
        .expect("deserialize telegram config");
        let inventory = mvp::channel::channel_inventory(&config);
        let resolution = build_channel_resolution(
            "/tmp/loong.toml",
            &config,
            &inventory,
            "telegram:Ops-Bot:123",
        )
        .expect("resolve known session");

        let rendered = render_channel_resolution_text(&resolution);

        assert!(rendered.contains("resolve_kind=session"));
        assert!(rendered.contains("channel_id=telegram"));
        assert!(rendered.contains("session_shape=telegram_chat"));
        assert!(rendered.contains("matched_configured_account=ops"));
        assert!(rendered.contains("send_command=telegram-send"));
    }
}
