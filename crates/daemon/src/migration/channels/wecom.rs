use loong_app as mvp;

use super::ChannelDoctorCheck;
use super::ensure_default_env_binding;
use super::{ChannelCheckLevel, ChannelPreflightCheck, ChannelPreview, build_channel_preview};
use crate::migration::ChannelCredentialState;
use crate::migration::{ChannelImportReadiness, ImportSurfaceLevel};

pub(super) const ID: &str = "wecom";

const FALLBACK_DESCRIPTOR: mvp::config::ChannelDescriptor = mvp::config::ChannelDescriptor {
    id: ID,
    label: "wecom",
    surface_label: "wecom channel",
    runtime_kind: mvp::config::ChannelRuntimeKind::RuntimeBacked,
    operational_model: mvp::config::ChannelOperationalModel::GatewaySupervised,
    serve_subcommand: Some("channels serve wecom"),
};

#[derive(Debug, Clone)]
struct EffectiveWecomConfig {
    enabled: bool,
    bot_id: Option<String>,
    secret: Option<String>,
    bot_id_env: Option<String>,
    secret_env: Option<String>,
    websocket_url: String,
    allowed_conversation_ids: Vec<String>,
}

pub(super) fn collect_preview(
    config: &mvp::config::LoongConfig,
    readiness: &ChannelImportReadiness,
    source: &str,
) -> Option<ChannelPreview> {
    let credential_state = readiness.state(ID);
    let effective = effective_wecom_config(config);
    let default_wecom = mvp::config::WecomChannelConfig::default();
    let default_websocket_url = default_wecom.resolved_websocket_url();
    let configured = effective.enabled
        || credential_state != ChannelCredentialState::Missing
        || effective.bot_id_env != default_wecom.bot_id_env
        || effective.secret_env != default_wecom.secret_env
        || effective.websocket_url != default_websocket_url
        || !effective.allowed_conversation_ids.is_empty();
    if !configured {
        return None;
    }

    let level = if credential_state.is_ready() {
        ImportSurfaceLevel::Ready
    } else if effective.enabled {
        ImportSurfaceLevel::Review
    } else {
        ImportSurfaceLevel::Blocked
    };
    let detail = match (effective.enabled, credential_state) {
        (true, ChannelCredentialState::Ready) => {
            let conversation_count = effective.allowed_conversation_ids.len();
            format!(
                "enabled · credentials resolved · {conversation_count} allowed conversation id(s)"
            )
        }
        (false, ChannelCredentialState::Ready) => {
            "credentials resolved · can enable during onboarding".to_owned()
        }
        (true, ChannelCredentialState::Partial) => "enabled · bot_id or secret missing".to_owned(),
        (false, ChannelCredentialState::Partial) => {
            "configured · bot_id or secret missing".to_owned()
        }
        (true, ChannelCredentialState::Missing) => "enabled · bot_id or secret missing".to_owned(),
        (false, ChannelCredentialState::Missing) => "configured but disabled".to_owned(),
    };

    Some(build_channel_preview(
        ID,
        descriptor().label,
        descriptor().surface_label,
        source.to_owned(),
        level,
        detail,
    ))
}

pub(super) fn apply(
    target: &mut mvp::config::LoongConfig,
    source: &mvp::config::LoongConfig,
) -> bool {
    merge_wecom_config(&mut target.wecom, &source.wecom)
}

pub(super) fn readiness_state(config: &mvp::config::LoongConfig) -> ChannelCredentialState {
    let effective = effective_wecom_config(config);
    let has_bot_id = effective.bot_id.is_some();
    let has_secret = effective.secret.is_some();

    match (has_bot_id, has_secret) {
        (true, true) => ChannelCredentialState::Ready,
        (true, false) | (false, true) => ChannelCredentialState::Partial,
        (false, false) => ChannelCredentialState::Missing,
    }
}

pub(super) fn apply_import_readiness(
    target: &mut mvp::config::LoongConfig,
    state: ChannelCredentialState,
) {
    if state.is_ready() {
        target.wecom.enabled = true;
    }
}

pub(super) fn collect_preflight_checks(
    config: &mvp::config::LoongConfig,
) -> Vec<ChannelPreflightCheck> {
    let credential_state = readiness_state(config);
    let (transport_level, transport_detail) = long_connection_check(config, false);

    vec![
        ChannelPreflightCheck {
            name: descriptor().surface_label,
            level: if credential_state.is_ready() {
                ChannelCheckLevel::Pass
            } else {
                ChannelCheckLevel::Warn
            },
            detail: if credential_state.is_ready() {
                "bot credentials resolved".to_owned()
            } else {
                "enabled but bot_id or secret is missing".to_owned()
            },
        },
        ChannelPreflightCheck {
            name: "wecom aibot long connection",
            level: transport_level,
            detail: transport_detail,
        },
    ]
}

pub(super) fn collect_doctor_checks(config: &mvp::config::LoongConfig) -> Vec<ChannelDoctorCheck> {
    let credential_state = readiness_state(config);
    let (transport_level, transport_detail) = long_connection_check(config, true);

    vec![
        ChannelDoctorCheck {
            name: descriptor().surface_label,
            level: if credential_state.is_ready() {
                ChannelCheckLevel::Pass
            } else {
                ChannelCheckLevel::Fail
            },
            detail: if credential_state.is_ready() {
                "bot credentials resolved".to_owned()
            } else {
                "enabled but bot_id or secret is missing".to_owned()
            },
        },
        ChannelDoctorCheck {
            name: "wecom aibot long connection",
            level: transport_level,
            detail: transport_detail,
        },
    ]
}

pub(super) fn apply_default_env_bindings(config: &mut mvp::config::LoongConfig) -> Vec<String> {
    let mut fixes = Vec::new();
    let default = mvp::config::WecomChannelConfig::default();

    ensure_default_env_binding(
        &mut config.wecom.bot_id_env,
        default.bot_id_env.as_deref(),
        "set wecom.bot_id_env",
        &mut fixes,
    );
    ensure_default_env_binding(
        &mut config.wecom.secret_env,
        default.secret_env.as_deref(),
        "set wecom.secret_env",
        &mut fixes,
    );

    fixes
}

fn merge_wecom_config(
    target: &mut mvp::config::WecomChannelConfig,
    source: &mvp::config::WecomChannelConfig,
) -> bool {
    let default = mvp::config::WecomChannelConfig::default();
    let mut changed = false;

    if !target.enabled && source.enabled {
        target.enabled = true;
        changed = true;
    }
    if target.account_id.is_none() && source.account_id.is_some() {
        target.account_id = source.account_id.clone();
        changed = true;
    }
    if target.default_account.is_none() && source.default_account.is_some() {
        target.default_account = source.default_account.clone();
        changed = true;
    }
    if target.bot_id.is_none() && source.bot_id.is_some() {
        target.bot_id = source.bot_id.clone();
        changed = true;
    }
    if target.secret.is_none() && source.secret.is_some() {
        target.secret = source.secret.clone();
        changed = true;
    }
    if let Some(source_bot_id_env) = source.bot_id_env.as_ref() {
        let target_uses_default_env =
            target.bot_id_env.is_none() || target.bot_id_env == default.bot_id_env;
        let target_matches_source = target.bot_id_env.as_ref() == Some(source_bot_id_env);
        if target_uses_default_env && !target_matches_source {
            target.bot_id_env = Some(source_bot_id_env.clone());
            changed = true;
        }
    }
    if let Some(source_secret_env) = source.secret_env.as_ref() {
        let target_uses_default_env =
            target.secret_env.is_none() || target.secret_env == default.secret_env;
        let target_matches_source = target.secret_env.as_ref() == Some(source_secret_env);
        if target_uses_default_env && !target_matches_source {
            target.secret_env = Some(source_secret_env.clone());
            changed = true;
        }
    }
    let default_websocket_url = default.resolved_websocket_url();
    let target_websocket_url = target.resolved_websocket_url();
    let source_websocket_url = source.resolved_websocket_url();
    let target_uses_default_websocket = target_websocket_url == default_websocket_url;
    let source_uses_default_websocket = source_websocket_url == default_websocket_url;
    if target_uses_default_websocket && !source_uses_default_websocket {
        target.websocket_url = source.websocket_url.clone();
        changed = true;
    }
    if target.ping_interval_s == default.ping_interval_s
        && source.ping_interval_s != default.ping_interval_s
    {
        target.ping_interval_s = source.ping_interval_s;
        changed = true;
    }
    if target.reconnect_interval_s == default.reconnect_interval_s
        && source.reconnect_interval_s != default.reconnect_interval_s
    {
        target.reconnect_interval_s = source.reconnect_interval_s;
        changed = true;
    }
    for conversation_id in &source.allowed_conversation_ids {
        changed |= merge_unique_string(&mut target.allowed_conversation_ids, conversation_id);
    }
    if target.acp == mvp::config::ChannelAcpConfig::default()
        && source.acp != mvp::config::ChannelAcpConfig::default()
    {
        target.acp = source.acp.clone();
        changed = true;
    }
    for (account_id, source_account) in &source.accounts {
        match target.accounts.entry(account_id.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(source_account.clone());
                changed = true;
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                changed |= merge_wecom_account_config(entry.get_mut(), source_account);
            }
        }
    }

    changed
}

fn effective_wecom_config(config: &mvp::config::LoongConfig) -> EffectiveWecomConfig {
    if let Ok(resolved) = config.wecom.resolve_account(None) {
        let bot_id = resolved.bot_id();
        let secret = resolved.secret();
        let websocket_url = resolved.resolved_websocket_url();

        return EffectiveWecomConfig {
            enabled: resolved.enabled,
            bot_id,
            secret,
            bot_id_env: resolved.bot_id_env,
            secret_env: resolved.secret_env,
            websocket_url,
            allowed_conversation_ids: resolved.allowed_conversation_ids,
        };
    }

    let bot_id = config.wecom.bot_id();
    let secret = config.wecom.secret();
    let websocket_url = config.wecom.resolved_websocket_url();

    EffectiveWecomConfig {
        enabled: config.wecom.enabled,
        bot_id,
        secret,
        bot_id_env: config.wecom.bot_id_env.clone(),
        secret_env: config.wecom.secret_env.clone(),
        websocket_url,
        allowed_conversation_ids: config.wecom.allowed_conversation_ids.clone(),
    }
}

fn merge_wecom_account_config(
    target: &mut mvp::config::WecomAccountConfig,
    source: &mvp::config::WecomAccountConfig,
) -> bool {
    let mut changed = false;

    if target.enabled.is_none() && source.enabled.is_some() {
        target.enabled = source.enabled;
        changed = true;
    }
    if target.account_id.is_none() && source.account_id.is_some() {
        target.account_id = source.account_id.clone();
        changed = true;
    }
    if target.bot_id.is_none() && source.bot_id.is_some() {
        target.bot_id = source.bot_id.clone();
        changed = true;
    }
    if target.secret.is_none() && source.secret.is_some() {
        target.secret = source.secret.clone();
        changed = true;
    }
    if target.bot_id_env.is_none() && source.bot_id_env.is_some() {
        target.bot_id_env = source.bot_id_env.clone();
        changed = true;
    }
    if target.secret_env.is_none() && source.secret_env.is_some() {
        target.secret_env = source.secret_env.clone();
        changed = true;
    }
    if target.websocket_url.is_none() && source.websocket_url.is_some() {
        target.websocket_url = source.websocket_url.clone();
        changed = true;
    }
    if target.ping_interval_s.is_none() && source.ping_interval_s.is_some() {
        target.ping_interval_s = source.ping_interval_s;
        changed = true;
    }
    if target.reconnect_interval_s.is_none() && source.reconnect_interval_s.is_some() {
        target.reconnect_interval_s = source.reconnect_interval_s;
        changed = true;
    }
    match (
        &mut target.allowed_conversation_ids,
        &source.allowed_conversation_ids,
    ) {
        (None, Some(source_conversation_ids)) => {
            target.allowed_conversation_ids = Some(source_conversation_ids.clone());
            changed = true;
        }
        (Some(target_conversation_ids), Some(source_conversation_ids)) => {
            for conversation_id in source_conversation_ids {
                changed |= merge_unique_string(target_conversation_ids, conversation_id);
            }
        }
        _ => {}
    }
    if target.acp.is_none() && source.acp.is_some() {
        target.acp = source.acp.clone();
        changed = true;
    }

    changed
}

fn merge_unique_string(target: &mut Vec<String>, value: &str) -> bool {
    if target.iter().any(|existing| existing == value) {
        return false;
    }

    target.push(value.to_owned());
    true
}

fn descriptor() -> &'static mvp::config::ChannelDescriptor {
    mvp::config::channel_descriptor(ID).unwrap_or(&FALLBACK_DESCRIPTOR)
}

fn long_connection_check(
    config: &mvp::config::LoongConfig,
    fail_on_error: bool,
) -> (ChannelCheckLevel, String) {
    let effective = effective_wecom_config(config);
    let mut issues = Vec::new();

    let websocket_url_issue = validate_wecom_websocket_url(effective.websocket_url.as_str());
    if let Some(issue) = websocket_url_issue {
        issues.push(issue);
    }

    let has_allowlist = effective
        .allowed_conversation_ids
        .iter()
        .any(|value| !value.trim().is_empty());
    if !has_allowlist {
        issues.push("allowed_conversation_ids is empty".to_owned());
    }

    if issues.is_empty() {
        return (
            ChannelCheckLevel::Pass,
            "websocket transport and conversation allowlist are configured".to_owned(),
        );
    }

    let detail = issues.join("; ");
    let level = if fail_on_error {
        ChannelCheckLevel::Fail
    } else {
        ChannelCheckLevel::Warn
    };

    (level, detail)
}

fn validate_wecom_websocket_url(websocket_url: &str) -> Option<String> {
    let parse_result = reqwest::Url::parse(websocket_url);
    let parsed_url = match parse_result {
        Ok(value) => value,
        Err(error) => return Some(format!("websocket_url is invalid: {error}")),
    };

    let scheme = parsed_url.scheme();
    let uses_websocket_scheme = scheme == "ws" || scheme == "wss";
    if uses_websocket_scheme {
        return None;
    }

    Some("websocket_url must use ws or wss".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_doctor_checks_uses_default_account_settings() {
        let mut config = mvp::config::LoongConfig::default();
        config.wecom.enabled = true;
        config.wecom.default_account = Some("ops".to_owned());
        config.wecom.accounts.insert(
            "ops".to_owned(),
            mvp::config::WecomAccountConfig {
                bot_id: Some(loong_contracts::SecretRef::Inline("wecom-bot".to_owned())),
                secret: Some(loong_contracts::SecretRef::Inline(
                    "wecom-secret".to_owned(),
                )),
                allowed_conversation_ids: Some(vec!["group_ops".to_owned()]),
                ..Default::default()
            },
        );

        let checks = collect_doctor_checks(&config);

        assert_eq!(checks.len(), 2);
        assert!(
            checks
                .iter()
                .all(|check| check.level == ChannelCheckLevel::Pass)
        );
    }

    #[test]
    fn merge_wecom_config_copies_custom_defaults_and_account_overrides() {
        let mut target = mvp::config::WecomChannelConfig::default();
        let source_acp = mvp::config::ChannelAcpConfig {
            bootstrap_mcp_servers: vec!["filesystem".to_owned()],
            working_directory: Some("/tmp/wecom".to_owned()),
        };
        let source_account = mvp::config::WecomAccountConfig {
            bot_id_env: Some("OPS_WECOM_BOT_ID".to_owned()),
            secret_env: Some("OPS_WECOM_SECRET".to_owned()),
            websocket_url: Some("wss://ops.example.test".to_owned()),
            allowed_conversation_ids: Some(vec!["group_ops".to_owned()]),
            ..Default::default()
        };
        let mut source_accounts = std::collections::BTreeMap::new();
        source_accounts.insert("ops".to_owned(), source_account);
        let source = mvp::config::WecomChannelConfig {
            enabled: true,
            default_account: Some("ops".to_owned()),
            bot_id_env: Some("CUSTOM_TOP_LEVEL_WECOM_BOT_ID".to_owned()),
            secret_env: Some("CUSTOM_TOP_LEVEL_WECOM_SECRET".to_owned()),
            websocket_url: Some("wss://wecom.example.test".to_owned()),
            ping_interval_s: 45,
            reconnect_interval_s: 12,
            allowed_conversation_ids: vec!["group_ops".to_owned()],
            acp: source_acp,
            accounts: source_accounts,
            ..Default::default()
        };

        let changed = merge_wecom_config(&mut target, &source);

        assert!(changed);
        assert!(target.enabled);
        assert_eq!(target.default_account.as_deref(), Some("ops"));
        assert_eq!(
            target.bot_id_env.as_deref(),
            Some("CUSTOM_TOP_LEVEL_WECOM_BOT_ID")
        );
        assert_eq!(
            target.secret_env.as_deref(),
            Some("CUSTOM_TOP_LEVEL_WECOM_SECRET")
        );
        assert_eq!(
            target.websocket_url.as_deref(),
            Some("wss://wecom.example.test")
        );
        assert_eq!(target.ping_interval_s, 45);
        assert_eq!(target.reconnect_interval_s, 12);
        assert_eq!(target.allowed_conversation_ids, vec!["group_ops"]);
        assert_eq!(target.acp.working_directory.as_deref(), Some("/tmp/wecom"));
        assert!(target.accounts.contains_key("ops"));
    }

    #[test]
    fn merge_wecom_config_treats_explicit_default_websocket_url_as_default() {
        let default_websocket_url =
            mvp::config::WecomChannelConfig::default().resolved_websocket_url();
        let mut target = mvp::config::WecomChannelConfig {
            websocket_url: Some(default_websocket_url),
            ..Default::default()
        };
        let source = mvp::config::WecomChannelConfig {
            websocket_url: Some("wss://wecom.example.test".to_owned()),
            ..Default::default()
        };

        let changed = merge_wecom_config(&mut target, &source);

        assert!(changed);
        assert_eq!(
            target.websocket_url.as_deref(),
            Some("wss://wecom.example.test")
        );
    }

    #[test]
    fn merge_wecom_config_ignores_blank_websocket_url_override() {
        let mut target = mvp::config::WecomChannelConfig::default();
        let source = mvp::config::WecomChannelConfig {
            websocket_url: Some("   ".to_owned()),
            ..Default::default()
        };

        let changed = merge_wecom_config(&mut target, &source);

        assert!(!changed);
        assert_eq!(target.websocket_url, None);
    }
}
