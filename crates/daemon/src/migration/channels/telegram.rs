use loong_app as mvp;

use super::ChannelDoctorCheck;
use super::ensure_default_env_binding;
use super::{ChannelCheckLevel, ChannelPreflightCheck, ChannelPreview, build_channel_preview};
use crate::migration::ChannelCredentialState;
use crate::migration::{ChannelImportReadiness, ImportSurfaceLevel};

pub(super) const ID: &str = "telegram";

const FALLBACK_DESCRIPTOR: mvp::config::ChannelDescriptor = mvp::config::ChannelDescriptor {
    id: ID,
    label: "telegram",
    surface_label: "telegram channel",
    runtime_kind: mvp::config::ChannelRuntimeKind::RuntimeBacked,
    operational_model: mvp::config::ChannelOperationalModel::GatewaySupervised,
    serve_subcommand: Some("telegram-serve"),
};

pub(super) fn collect_preview(
    config: &mvp::config::LoongConfig,
    readiness: &ChannelImportReadiness,
    source: &str,
) -> Option<ChannelPreview> {
    let token_resolved = readiness.is_ready(ID);
    let default_telegram = mvp::config::TelegramChannelConfig::default();
    let configured = config.telegram.enabled
        || token_resolved
        || config.telegram.bot_token_env != default_telegram.bot_token_env
        || config.telegram.base_url != default_telegram.base_url
        || config.telegram.polling_timeout_s != default_telegram.polling_timeout_s
        || !config.telegram.allowed_chat_ids.is_empty();
    if !configured {
        return None;
    }

    let level = if token_resolved {
        ImportSurfaceLevel::Ready
    } else if config.telegram.enabled {
        ImportSurfaceLevel::Review
    } else {
        ImportSurfaceLevel::Blocked
    };
    let detail = if config.telegram.enabled && token_resolved {
        format!(
            "enabled · token resolved · {} allowed chat id(s)",
            config.telegram.allowed_chat_ids.len()
        )
    } else if token_resolved {
        "token resolved · can enable during onboarding".to_owned()
    } else if config.telegram.enabled {
        "enabled · token missing".to_owned()
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
    target: &mut mvp::config::LoongConfig,
    source: &mvp::config::LoongConfig,
) -> bool {
    merge_telegram_config(&mut target.telegram, &source.telegram)
}

pub(super) fn readiness_state(config: &mvp::config::LoongConfig) -> ChannelCredentialState {
    if config.telegram.bot_token().is_some() {
        ChannelCredentialState::Ready
    } else {
        ChannelCredentialState::Missing
    }
}

pub(super) fn apply_import_readiness(
    target: &mut mvp::config::LoongConfig,
    state: ChannelCredentialState,
) {
    if state.is_ready() {
        target.telegram.enabled = true;
    }
}

pub(super) fn collect_preflight_checks(
    config: &mvp::config::LoongConfig,
) -> Vec<ChannelPreflightCheck> {
    let state = readiness_state(config);
    vec![ChannelPreflightCheck {
        name: descriptor().surface_label,
        level: if state.is_ready() {
            ChannelCheckLevel::Pass
        } else {
            ChannelCheckLevel::Warn
        },
        detail: if state.is_ready() {
            "bot token resolved".to_owned()
        } else {
            "enabled but bot token is missing (telegram.bot_token or env)".to_owned()
        },
    }]
}

pub(super) fn collect_doctor_checks(config: &mvp::config::LoongConfig) -> Vec<ChannelDoctorCheck> {
    let state = readiness_state(config);
    vec![ChannelDoctorCheck {
        name: descriptor().surface_label,
        level: if state.is_ready() {
            ChannelCheckLevel::Pass
        } else {
            ChannelCheckLevel::Fail
        },
        detail: if state.is_ready() {
            "bot token resolved".to_owned()
        } else {
            "enabled but bot token is missing (telegram.bot_token or env)".to_owned()
        },
    }]
}

pub(super) fn apply_default_env_bindings(config: &mut mvp::config::LoongConfig) -> Vec<String> {
    let mut fixes = Vec::new();
    let default = mvp::config::TelegramChannelConfig::default();
    ensure_default_env_binding(
        &mut config.telegram.bot_token_env,
        default.bot_token_env.as_deref(),
        "set telegram.bot_token_env",
        &mut fixes,
    );
    fixes
}

fn merge_telegram_config(
    target: &mut mvp::config::TelegramChannelConfig,
    source: &mvp::config::TelegramChannelConfig,
) -> bool {
    let default = mvp::config::TelegramChannelConfig::default();
    let mut changed = false;

    if !target.enabled && source.enabled {
        target.enabled = true;
        changed = true;
    }
    if target.bot_token.is_none() && source.bot_token.is_some() {
        target.bot_token = source.bot_token.clone();
        changed = true;
    }
    if target.bot_token_env.is_none() && source.bot_token_env.is_some() {
        target.bot_token_env = source.bot_token_env.clone();
        changed = true;
    }
    if target.base_url == default.base_url && source.base_url != default.base_url {
        target.base_url = source.base_url.clone();
        changed = true;
    }
    if target.polling_timeout_s == default.polling_timeout_s
        && source.polling_timeout_s != default.polling_timeout_s
    {
        target.polling_timeout_s = source.polling_timeout_s;
        changed = true;
    }
    for chat_id in &source.allowed_chat_ids {
        if !target.allowed_chat_ids.contains(chat_id) {
            target.allowed_chat_ids.push(*chat_id);
            changed = true;
        }
    }

    changed
}

fn descriptor() -> &'static mvp::config::ChannelDescriptor {
    mvp::config::channel_descriptor(ID).unwrap_or(&FALLBACK_DESCRIPTOR)
}
