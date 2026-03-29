use loongclaw_app as mvp;

use super::ChannelDoctorCheck;
use super::ensure_default_env_binding;
use super::{ChannelCheckLevel, ChannelPreflightCheck, ChannelPreview, build_channel_preview};
use crate::migration::ChannelCredentialState;
use crate::migration::{ChannelImportReadiness, ImportSurfaceLevel};

pub(super) const ID: &str = "feishu";

const FALLBACK_DESCRIPTOR: mvp::config::ChannelDescriptor = mvp::config::ChannelDescriptor {
    id: ID,
    label: "feishu",
    surface_label: "feishu channel",
    runtime_kind: mvp::config::ChannelRuntimeKind::Service,
    serve_subcommand: Some("feishu-serve"),
};

pub(super) fn collect_preview(
    config: &mvp::config::LoongClawConfig,
    readiness: &ChannelImportReadiness,
    source: &str,
) -> Option<ChannelPreview> {
    let credential_state = readiness.state(ID);
    let app_credentials_resolved = credential_state.is_ready();
    let default_feishu = mvp::config::FeishuChannelConfig::default();
    let configured = config.feishu.enabled
        || app_credentials_resolved
        || config.feishu.app_id_env != default_feishu.app_id_env
        || config.feishu.app_secret_env != default_feishu.app_secret_env
        || config.feishu.base_url != default_feishu.base_url
        || config.feishu.mode != default_feishu.mode
        || config.feishu.receive_id_type != default_feishu.receive_id_type
        || config.feishu.webhook_bind != default_feishu.webhook_bind
        || config.feishu.webhook_path != default_feishu.webhook_path
        || config.feishu.verification_token_env != default_feishu.verification_token_env
        || config.feishu.encrypt_key_env != default_feishu.encrypt_key_env
        || !config.feishu.allowed_chat_ids.is_empty()
        || !config.feishu.ignore_bot_messages;
    if !configured {
        return None;
    }

    let level = if app_credentials_resolved {
        ImportSurfaceLevel::Ready
    } else if config.feishu.enabled {
        ImportSurfaceLevel::Review
    } else {
        ImportSurfaceLevel::Blocked
    };
    let detail = if config.feishu.enabled && app_credentials_resolved {
        format!(
            "enabled · app credentials resolved · {} allowed chat id(s)",
            config.feishu.allowed_chat_ids.len()
        )
    } else if app_credentials_resolved {
        "app credentials resolved · can enable during onboarding".to_owned()
    } else if config.feishu.enabled {
        "enabled · app_id or app_secret missing".to_owned()
    } else {
        "configured but disabled".to_owned()
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
    target: &mut mvp::config::LoongClawConfig,
    source: &mvp::config::LoongClawConfig,
) -> bool {
    merge_feishu_config(&mut target.feishu, &source.feishu)
}

pub(super) fn readiness_state(config: &mvp::config::LoongClawConfig) -> ChannelCredentialState {
    let app_id_resolved = config.feishu.app_id().is_some();
    let app_secret_resolved = config.feishu.app_secret().is_some();
    match (app_id_resolved, app_secret_resolved) {
        (true, true) => ChannelCredentialState::Ready,
        (true, false) | (false, true) => ChannelCredentialState::Partial,
        (false, false) => ChannelCredentialState::Missing,
    }
}

pub(super) fn apply_import_readiness(
    target: &mut mvp::config::LoongClawConfig,
    state: ChannelCredentialState,
) {
    if state.is_ready() {
        target.feishu.enabled = true;
    }
}

pub(super) fn collect_preflight_checks(
    config: &mvp::config::LoongClawConfig,
) -> Vec<ChannelPreflightCheck> {
    let credential_state = readiness_state(config);
    let (transport_level, transport_detail) = inbound_transport_check(config);

    vec![
        ChannelPreflightCheck {
            name: descriptor().surface_label,
            level: if credential_state.is_ready() {
                ChannelCheckLevel::Pass
            } else {
                ChannelCheckLevel::Warn
            },
            detail: if credential_state.is_ready() {
                "app credentials resolved".to_owned()
            } else {
                "enabled but app_id or app_secret is missing".to_owned()
            },
        },
        ChannelPreflightCheck {
            name: "feishu inbound transport",
            level: transport_level,
            detail: transport_detail,
        },
    ]
}

pub(super) fn collect_doctor_checks(
    config: &mvp::config::LoongClawConfig,
) -> Vec<ChannelDoctorCheck> {
    let credential_state = readiness_state(config);
    let (transport_level, transport_detail) = inbound_transport_check(config);

    vec![
        ChannelDoctorCheck {
            name: descriptor().surface_label,
            level: if credential_state.is_ready() {
                ChannelCheckLevel::Pass
            } else {
                ChannelCheckLevel::Fail
            },
            detail: if credential_state.is_ready() {
                "app credentials resolved".to_owned()
            } else {
                "enabled but app_id or app_secret is missing".to_owned()
            },
        },
        ChannelDoctorCheck {
            name: "feishu inbound transport",
            level: transport_level,
            detail: transport_detail,
        },
    ]
}

pub(super) fn apply_default_env_bindings(config: &mut mvp::config::LoongClawConfig) -> Vec<String> {
    let mut fixes = Vec::new();
    let default = mvp::config::FeishuChannelConfig::default();
    ensure_default_env_binding(
        &mut config.feishu.app_id_env,
        default.app_id_env.as_deref(),
        "set feishu.app_id_env",
        &mut fixes,
    );
    ensure_default_env_binding(
        &mut config.feishu.app_secret_env,
        default.app_secret_env.as_deref(),
        "set feishu.app_secret_env",
        &mut fixes,
    );
    if config
        .feishu
        .mode
        .unwrap_or(mvp::config::FeishuChannelServeMode::Websocket)
        != mvp::config::FeishuChannelServeMode::Websocket
    {
        ensure_default_env_binding(
            &mut config.feishu.verification_token_env,
            default.verification_token_env.as_deref(),
            "set feishu.verification_token_env",
            &mut fixes,
        );
        ensure_default_env_binding(
            &mut config.feishu.encrypt_key_env,
            default.encrypt_key_env.as_deref(),
            "set feishu.encrypt_key_env",
            &mut fixes,
        );
    }
    fixes
}

fn merge_feishu_config(
    target: &mut mvp::config::FeishuChannelConfig,
    source: &mvp::config::FeishuChannelConfig,
) -> bool {
    let default = mvp::config::FeishuChannelConfig::default();
    let mut changed = false;

    if !target.enabled && source.enabled {
        target.enabled = true;
        changed = true;
    }
    if target.app_id.is_none() && source.app_id.is_some() {
        target.app_id = source.app_id.clone();
        changed = true;
    }
    if target.app_secret.is_none() && source.app_secret.is_some() {
        target.app_secret = source.app_secret.clone();
        changed = true;
    }
    if target.app_id_env.is_none() && source.app_id_env.is_some() {
        target.app_id_env = source.app_id_env.clone();
        changed = true;
    }
    if target.app_secret_env.is_none() && source.app_secret_env.is_some() {
        target.app_secret_env = source.app_secret_env.clone();
        changed = true;
    }
    if target.base_url == default.base_url && source.base_url != default.base_url {
        target.base_url = source.base_url.clone();
        changed = true;
    }
    if target.mode.is_none() {
        let next_mode = source.mode.or(if target.enabled {
            Some(mvp::config::FeishuChannelServeMode::Websocket)
        } else {
            None
        });
        if next_mode.is_some() {
            target.mode = next_mode;
            changed = true;
        }
    }
    if target.receive_id_type == default.receive_id_type
        && source.receive_id_type != default.receive_id_type
    {
        target.receive_id_type = source.receive_id_type.clone();
        changed = true;
    }
    if target.webhook_bind == default.webhook_bind && source.webhook_bind != default.webhook_bind {
        target.webhook_bind = source.webhook_bind.clone();
        changed = true;
    }
    if target.webhook_path == default.webhook_path && source.webhook_path != default.webhook_path {
        target.webhook_path = source.webhook_path.clone();
        changed = true;
    }
    if target.verification_token.is_none() && source.verification_token.is_some() {
        target.verification_token = source.verification_token.clone();
        changed = true;
    }
    if target.verification_token_env.is_none() && source.verification_token_env.is_some() {
        target.verification_token_env = source.verification_token_env.clone();
        changed = true;
    }
    if target.encrypt_key.is_none() && source.encrypt_key.is_some() {
        target.encrypt_key = source.encrypt_key.clone();
        changed = true;
    }
    if target.encrypt_key_env.is_none() && source.encrypt_key_env.is_some() {
        target.encrypt_key_env = source.encrypt_key_env.clone();
        changed = true;
    }
    for chat_id in &source.allowed_chat_ids {
        if !target.allowed_chat_ids.contains(chat_id) {
            target.allowed_chat_ids.push(chat_id.clone());
            changed = true;
        }
    }
    if target.ignore_bot_messages == default.ignore_bot_messages
        && source.ignore_bot_messages != default.ignore_bot_messages
    {
        target.ignore_bot_messages = source.ignore_bot_messages;
        changed = true;
    }

    changed
}

fn descriptor() -> &'static mvp::config::ChannelDescriptor {
    mvp::config::channel_descriptor(ID).unwrap_or(&FALLBACK_DESCRIPTOR)
}

fn inbound_transport_check(config: &mvp::config::LoongClawConfig) -> (ChannelCheckLevel, String) {
    if config
        .feishu
        .mode
        .unwrap_or(mvp::config::FeishuChannelServeMode::Websocket)
        == mvp::config::FeishuChannelServeMode::Websocket
    {
        return (
            ChannelCheckLevel::Pass,
            "websocket mode configured; webhook secrets are not required".to_owned(),
        );
    }

    let verification_token = config.feishu.verification_token();
    let encrypt_key = config.feishu.encrypt_key();
    if verification_token.is_some() || encrypt_key.is_some() {
        (
            ChannelCheckLevel::Pass,
            "webhook verification token or encrypt key is configured".to_owned(),
        )
    } else {
        (
            ChannelCheckLevel::Warn,
            "webhook mode is configured but verification_token and encrypt_key are both missing"
                .to_owned(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_config(raw: &str) -> mvp::config::LoongClawConfig {
        toml::from_str(raw).expect("deserialize loongclaw config")
    }

    #[test]
    fn merge_preserves_explicit_top_level_webhook_mode() {
        let mut target = parse_config(
            r#"
            [feishu]
            mode = "webhook"
            "#,
        );
        let source = parse_config(
            r#"
            [feishu]
            mode = "websocket"
            "#,
        );

        assert!(
            !apply(&mut target, &source),
            "an explicit top-level webhook selection should leave the merged config unchanged"
        );
        assert_eq!(
            target
                .feishu
                .resolve_account(None)
                .expect("resolve merged feishu config")
                .mode,
            mvp::config::FeishuChannelServeMode::Webhook,
            "an explicit top-level webhook selection must not be replaced by a later websocket source"
        );
    }

    #[test]
    fn websocket_mode_skips_default_webhook_secret_bindings() {
        let mut config = parse_config(
            r#"
            [feishu]
            mode = "websocket"
            "#,
        );
        config.feishu.app_id_env = None;
        config.feishu.app_secret_env = None;
        config.feishu.verification_token_env = None;
        config.feishu.encrypt_key_env = None;

        let fixes = apply_default_env_bindings(&mut config);

        assert!(
            fixes
                .iter()
                .any(|fix| fix.starts_with("set feishu.app_id_env=")),
            "websocket mode still needs default app_id env guidance"
        );
        assert!(
            fixes
                .iter()
                .any(|fix| fix.starts_with("set feishu.app_secret_env=")),
            "websocket mode still needs default app_secret env guidance"
        );
        assert!(
            fixes
                .iter()
                .all(|fix| !fix.starts_with("set feishu.verification_token_env=")),
            "websocket mode must not auto-fill webhook verification secrets"
        );
        assert!(
            fixes
                .iter()
                .all(|fix| !fix.starts_with("set feishu.encrypt_key_env=")),
            "websocket mode must not auto-fill webhook encrypt secrets"
        );
        assert!(config.feishu.verification_token_env.is_none());
        assert!(config.feishu.encrypt_key_env.is_none());
    }

    #[test]
    fn absent_mode_env_fix_skips_webhook_secrets() {
        let mut config = parse_config(
            r#"
            [feishu]
            enabled = true
            "#,
        );
        // Deserialized config with [feishu] present but no mode key gives mode = None.
        assert!(
            config.feishu.mode.is_none(),
            "deserialized config with absent mode field must be None"
        );
        config.feishu.app_id_env = None;
        config.feishu.app_secret_env = None;
        config.feishu.verification_token_env = None;
        config.feishu.encrypt_key_env = None;

        let fixes = apply_default_env_bindings(&mut config);

        assert!(
            fixes
                .iter()
                .all(|fix| !fix.starts_with("set feishu.verification_token_env=")),
            "absent mode (defaulting to websocket) must not auto-fill webhook verification secrets"
        );
        assert!(
            fixes
                .iter()
                .all(|fix| !fix.starts_with("set feishu.encrypt_key_env=")),
            "absent mode (defaulting to websocket) must not auto-fill webhook encrypt secrets"
        );
        assert!(config.feishu.verification_token_env.is_none());
        assert!(config.feishu.encrypt_key_env.is_none());
    }

    #[test]
    fn merge_defaults_to_websocket_when_enabling_feishu_without_explicit_mode() {
        let mut target = parse_config(
            r#"
            [feishu]
            enabled = false
            "#,
        );
        let source = parse_config(
            r#"
            [feishu]
            enabled = true
            "#,
        );

        assert!(
            apply(&mut target, &source),
            "enabling feishu without explicit mode should still produce a persisted mode"
        );
        assert_eq!(
            target.feishu.mode,
            Some(mvp::config::FeishuChannelServeMode::Websocket),
            "merge should persist websocket mode for enabled feishu imports when mode is omitted"
        );
    }

    #[test]
    fn merge_defaults_to_websocket_when_target_already_enabled() {
        let mut target = parse_config(
            r#"
            [feishu]
            enabled = true
            "#,
        );
        let source = parse_config(
            r#"
            [feishu]
            enabled = false
            "#,
        );

        assert!(
            apply(&mut target, &source),
            "enabled feishu target should persist websocket mode even if incoming source omits mode"
        );
        assert_eq!(
            target.feishu.mode,
            Some(mvp::config::FeishuChannelServeMode::Websocket),
            "merge should persist websocket mode whenever merged feishu remains enabled and mode is omitted"
        );
    }
}
