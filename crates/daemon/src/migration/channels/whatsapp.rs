use loongclaw_app as mvp;

use super::ChannelDoctorCheck;
use super::ensure_default_env_binding;
use super::{ChannelCheckLevel, ChannelPreflightCheck, ChannelPreview, build_channel_preview};
use crate::migration::ChannelCredentialState;
use crate::migration::{ChannelImportReadiness, ImportSurfaceLevel};

pub(super) const ID: &str = "whatsapp";

const FALLBACK_DESCRIPTOR: mvp::config::ChannelDescriptor = mvp::config::ChannelDescriptor {
    id: ID,
    label: "whatsapp",
    surface_label: "whatsapp channel",
    runtime_kind: mvp::config::ChannelRuntimeKind::Service,
    serve_subcommand: Some("whatsapp-serve"),
};

pub(super) fn collect_preview(
    config: &mvp::config::LoongClawConfig,
    readiness: &ChannelImportReadiness,
    source: &str,
) -> Option<ChannelPreview> {
    let credential_state = readiness.state(ID);
    let default_whatsapp = mvp::config::WhatsappChannelConfig::default();
    let configured = config.whatsapp.enabled
        || credential_state != ChannelCredentialState::Missing
        || config.whatsapp.access_token_env != default_whatsapp.access_token_env
        || config.whatsapp.phone_number_id_env != default_whatsapp.phone_number_id_env
        || config.whatsapp.verify_token_env != default_whatsapp.verify_token_env
        || config.whatsapp.app_secret_env != default_whatsapp.app_secret_env
        || config.whatsapp.api_base_url != default_whatsapp.api_base_url
        || config.whatsapp.webhook_bind != default_whatsapp.webhook_bind
        || config.whatsapp.webhook_path != default_whatsapp.webhook_path;
    if !configured {
        return None;
    }

    let level = if credential_state.is_ready() {
        ImportSurfaceLevel::Ready
    } else if config.whatsapp.enabled {
        ImportSurfaceLevel::Review
    } else {
        ImportSurfaceLevel::Blocked
    };
    let detail = preview_detail(config, credential_state);

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
    merge_whatsapp_config(&mut target.whatsapp, &source.whatsapp)
}

pub(super) fn readiness_state(config: &mvp::config::LoongClawConfig) -> ChannelCredentialState {
    let send_ready = whatsapp_send_credentials_ready(config);
    let webhook_ready = whatsapp_webhook_credentials_ready(config);
    let has_any_credential = whatsapp_has_any_runtime_credential(config);

    if send_ready && webhook_ready {
        return ChannelCredentialState::Ready;
    }
    if has_any_credential {
        return ChannelCredentialState::Partial;
    }

    ChannelCredentialState::Missing
}

pub(super) fn apply_import_readiness(
    target: &mut mvp::config::LoongClawConfig,
    state: ChannelCredentialState,
) {
    if state.is_ready() {
        target.whatsapp.enabled = true;
    }
}

pub(super) fn collect_preflight_checks(
    config: &mvp::config::LoongClawConfig,
) -> Vec<ChannelPreflightCheck> {
    let send_ready = whatsapp_send_credentials_ready(config);
    let webhook_ready = whatsapp_webhook_credentials_ready(config);
    let send_level = if send_ready {
        ChannelCheckLevel::Pass
    } else {
        ChannelCheckLevel::Warn
    };
    let webhook_level = if webhook_ready {
        ChannelCheckLevel::Pass
    } else {
        ChannelCheckLevel::Warn
    };
    let send_detail = whatsapp_send_detail(send_ready);
    let webhook_detail = whatsapp_webhook_detail(webhook_ready);

    vec![
        ChannelPreflightCheck {
            name: descriptor().surface_label,
            level: send_level,
            detail: send_detail,
        },
        ChannelPreflightCheck {
            name: "whatsapp webhook service",
            level: webhook_level,
            detail: webhook_detail,
        },
    ]
}

pub(super) fn collect_doctor_checks(
    config: &mvp::config::LoongClawConfig,
) -> Vec<ChannelDoctorCheck> {
    let send_ready = whatsapp_send_credentials_ready(config);
    let webhook_ready = whatsapp_webhook_credentials_ready(config);
    let send_level = if send_ready {
        ChannelCheckLevel::Pass
    } else {
        ChannelCheckLevel::Fail
    };
    let webhook_level = if webhook_ready {
        ChannelCheckLevel::Pass
    } else {
        ChannelCheckLevel::Fail
    };
    let send_detail = whatsapp_send_detail(send_ready);
    let webhook_detail = whatsapp_webhook_detail(webhook_ready);

    vec![
        ChannelDoctorCheck {
            name: descriptor().surface_label,
            level: send_level,
            detail: send_detail,
        },
        ChannelDoctorCheck {
            name: "whatsapp webhook service",
            level: webhook_level,
            detail: webhook_detail,
        },
    ]
}

pub(super) fn apply_default_env_bindings(config: &mut mvp::config::LoongClawConfig) -> Vec<String> {
    let mut fixes = Vec::new();
    let default = mvp::config::WhatsappChannelConfig::default();

    ensure_default_env_binding(
        &mut config.whatsapp.access_token_env,
        default.access_token_env.as_deref(),
        "set whatsapp.access_token_env",
        &mut fixes,
    );
    ensure_default_env_binding(
        &mut config.whatsapp.phone_number_id_env,
        default.phone_number_id_env.as_deref(),
        "set whatsapp.phone_number_id_env",
        &mut fixes,
    );
    ensure_default_env_binding(
        &mut config.whatsapp.verify_token_env,
        default.verify_token_env.as_deref(),
        "set whatsapp.verify_token_env",
        &mut fixes,
    );
    ensure_default_env_binding(
        &mut config.whatsapp.app_secret_env,
        default.app_secret_env.as_deref(),
        "set whatsapp.app_secret_env",
        &mut fixes,
    );

    fixes
}

fn preview_detail(
    config: &mvp::config::LoongClawConfig,
    credential_state: ChannelCredentialState,
) -> String {
    match (config.whatsapp.enabled, credential_state) {
        (true, ChannelCredentialState::Ready) => {
            "enabled · cloud api and webhook credentials resolved".to_owned()
        }
        (false, ChannelCredentialState::Ready) => {
            "cloud api and webhook credentials resolved · can enable during onboarding".to_owned()
        }
        (true, ChannelCredentialState::Partial) => {
            "enabled · access_token, phone_number_id, verify_token, or app_secret missing"
                .to_owned()
        }
        (false, ChannelCredentialState::Partial) => {
            "configured · access_token, phone_number_id, verify_token, or app_secret missing"
                .to_owned()
        }
        (true, ChannelCredentialState::Missing) => {
            "enabled · access_token, phone_number_id, verify_token, or app_secret missing"
                .to_owned()
        }
        (false, ChannelCredentialState::Missing) => "configured but disabled".to_owned(),
    }
}

fn whatsapp_send_credentials_ready(config: &mvp::config::LoongClawConfig) -> bool {
    let has_access_token = config.whatsapp.access_token().is_some();
    let has_phone_number_id = config.whatsapp.phone_number_id().is_some();

    has_access_token && has_phone_number_id
}

fn whatsapp_webhook_credentials_ready(config: &mvp::config::LoongClawConfig) -> bool {
    let has_verify_token = config.whatsapp.verify_token().is_some();
    let has_app_secret = config.whatsapp.app_secret().is_some();

    has_verify_token && has_app_secret
}

fn whatsapp_has_any_runtime_credential(config: &mvp::config::LoongClawConfig) -> bool {
    let has_access_token = config.whatsapp.access_token().is_some();
    let has_phone_number_id = config.whatsapp.phone_number_id().is_some();
    let has_verify_token = config.whatsapp.verify_token().is_some();
    let has_app_secret = config.whatsapp.app_secret().is_some();

    has_access_token || has_phone_number_id || has_verify_token || has_app_secret
}

fn whatsapp_send_detail(send_ready: bool) -> String {
    if send_ready {
        return "cloud api send credentials resolved".to_owned();
    }

    "enabled but access_token or phone_number_id is missing".to_owned()
}

fn whatsapp_webhook_detail(webhook_ready: bool) -> String {
    if webhook_ready {
        return "webhook verification credentials resolved".to_owned();
    }

    "enabled but verify_token or app_secret is missing".to_owned()
}

fn merge_whatsapp_config(
    target: &mut mvp::config::WhatsappChannelConfig,
    source: &mvp::config::WhatsappChannelConfig,
) -> bool {
    let default = mvp::config::WhatsappChannelConfig::default();
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
    if target.access_token.is_none() && source.access_token.is_some() {
        target.access_token = source.access_token.clone();
        changed = true;
    }
    if let Some(source_access_token_env) = source.access_token_env.as_ref() {
        let target_uses_default_env = target.access_token_env.is_none()
            || target.access_token_env == default.access_token_env;
        let target_matches_source =
            target.access_token_env.as_ref() == Some(source_access_token_env);
        if target_uses_default_env && !target_matches_source {
            target.access_token_env = Some(source_access_token_env.clone());
            changed = true;
        }
    }
    if target.phone_number_id.is_none() && source.phone_number_id.is_some() {
        target.phone_number_id = source.phone_number_id.clone();
        changed = true;
    }
    if let Some(source_phone_number_id_env) = source.phone_number_id_env.as_ref() {
        let target_uses_default_env = target.phone_number_id_env.is_none()
            || target.phone_number_id_env == default.phone_number_id_env;
        let target_matches_source =
            target.phone_number_id_env.as_ref() == Some(source_phone_number_id_env);
        if target_uses_default_env && !target_matches_source {
            target.phone_number_id_env = Some(source_phone_number_id_env.clone());
            changed = true;
        }
    }
    if target.verify_token.is_none() && source.verify_token.is_some() {
        target.verify_token = source.verify_token.clone();
        changed = true;
    }
    if let Some(source_verify_token_env) = source.verify_token_env.as_ref() {
        let target_uses_default_env = target.verify_token_env.is_none()
            || target.verify_token_env == default.verify_token_env;
        let target_matches_source =
            target.verify_token_env.as_ref() == Some(source_verify_token_env);
        if target_uses_default_env && !target_matches_source {
            target.verify_token_env = Some(source_verify_token_env.clone());
            changed = true;
        }
    }
    if target.app_secret.is_none() && source.app_secret.is_some() {
        target.app_secret = source.app_secret.clone();
        changed = true;
    }
    if let Some(source_app_secret_env) = source.app_secret_env.as_ref() {
        let target_uses_default_env =
            target.app_secret_env.is_none() || target.app_secret_env == default.app_secret_env;
        let target_matches_source = target.app_secret_env.as_ref() == Some(source_app_secret_env);
        if target_uses_default_env && !target_matches_source {
            target.app_secret_env = Some(source_app_secret_env.clone());
            changed = true;
        }
    }
    if target.api_base_url == default.api_base_url && source.api_base_url != default.api_base_url {
        target.api_base_url = source.api_base_url.clone();
        changed = true;
    }
    if target.webhook_bind.is_none() && source.webhook_bind.is_some() {
        target.webhook_bind = source.webhook_bind.clone();
        changed = true;
    }
    if target.webhook_path.is_none() && source.webhook_path.is_some() {
        target.webhook_path = source.webhook_path.clone();
        changed = true;
    }

    changed
}

fn descriptor() -> &'static mvp::config::ChannelDescriptor {
    mvp::config::channel_descriptor(ID).unwrap_or(&FALLBACK_DESCRIPTOR)
}
