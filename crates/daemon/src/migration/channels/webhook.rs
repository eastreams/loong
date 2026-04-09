use loongclaw_app as mvp;

use super::ChannelDoctorCheck;
use super::ensure_default_env_binding;
use super::{ChannelCheckLevel, ChannelPreflightCheck, ChannelPreview, build_channel_preview};
use crate::migration::ChannelCredentialState;
use crate::migration::{ChannelImportReadiness, ImportSurfaceLevel};

pub(super) const ID: &str = "webhook";

const FALLBACK_DESCRIPTOR: mvp::config::ChannelDescriptor = mvp::config::ChannelDescriptor {
    id: ID,
    label: "webhook",
    surface_label: "webhook channel",
    runtime_kind: mvp::config::ChannelRuntimeKind::Service,
    serve_subcommand: Some("webhook-serve"),
};

pub(super) fn collect_preview(
    config: &mvp::config::LoongClawConfig,
    readiness: &ChannelImportReadiness,
    source: &str,
) -> Option<ChannelPreview> {
    let credential_state = readiness.state(ID);
    let default_webhook = mvp::config::WebhookChannelConfig::default();
    let configured = config.webhook.enabled
        || credential_state != ChannelCredentialState::Missing
        || config.webhook.endpoint_url_env != default_webhook.endpoint_url_env
        || config.webhook.auth_token_env != default_webhook.auth_token_env
        || config.webhook.auth_header_name != default_webhook.auth_header_name
        || config.webhook.auth_token_prefix != default_webhook.auth_token_prefix
        || config.webhook.payload_format != default_webhook.payload_format
        || config.webhook.payload_text_field != default_webhook.payload_text_field
        || config.webhook.public_base_url != default_webhook.public_base_url
        || config.webhook.signing_secret_env != default_webhook.signing_secret_env;
    if !configured {
        return None;
    }

    let level = if credential_state.is_ready() {
        ImportSurfaceLevel::Ready
    } else if config.webhook.enabled {
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
    merge_webhook_config(&mut target.webhook, &source.webhook)
}

pub(super) fn readiness_state(config: &mvp::config::LoongClawConfig) -> ChannelCredentialState {
    let send_ready = webhook_send_credentials_ready(config);
    let serve_ready = webhook_serve_credentials_ready(config);
    let has_any_credential = webhook_has_any_runtime_credential(config);

    if serve_ready {
        return ChannelCredentialState::Ready;
    }
    if send_ready || has_any_credential {
        return ChannelCredentialState::Partial;
    }

    ChannelCredentialState::Missing
}

pub(super) fn apply_import_readiness(
    target: &mut mvp::config::LoongClawConfig,
    state: ChannelCredentialState,
) {
    if state.is_ready() {
        target.webhook.enabled = true;
    }
}

pub(super) fn collect_preflight_checks(
    config: &mvp::config::LoongClawConfig,
) -> Vec<ChannelPreflightCheck> {
    let send_ready = webhook_send_credentials_ready(config);
    let serve_ready = webhook_serve_credentials_ready(config);
    let send_level = if send_ready {
        ChannelCheckLevel::Pass
    } else {
        ChannelCheckLevel::Warn
    };
    let serve_level = if serve_ready {
        ChannelCheckLevel::Pass
    } else {
        ChannelCheckLevel::Warn
    };
    let send_detail = webhook_send_detail(send_ready);
    let serve_detail = webhook_serve_detail(serve_ready);

    vec![
        ChannelPreflightCheck {
            name: descriptor().surface_label,
            level: send_level,
            detail: send_detail,
        },
        ChannelPreflightCheck {
            name: "webhook signed service",
            level: serve_level,
            detail: serve_detail,
        },
    ]
}

pub(super) fn collect_doctor_checks(
    config: &mvp::config::LoongClawConfig,
) -> Vec<ChannelDoctorCheck> {
    let send_ready = webhook_send_credentials_ready(config);
    let serve_ready = webhook_serve_credentials_ready(config);
    let send_level = if send_ready {
        ChannelCheckLevel::Pass
    } else {
        ChannelCheckLevel::Fail
    };
    let serve_level = if serve_ready {
        ChannelCheckLevel::Pass
    } else {
        ChannelCheckLevel::Fail
    };
    let send_detail = webhook_send_detail(send_ready);
    let serve_detail = webhook_serve_detail(serve_ready);

    vec![
        ChannelDoctorCheck {
            name: descriptor().surface_label,
            level: send_level,
            detail: send_detail,
        },
        ChannelDoctorCheck {
            name: "webhook signed service",
            level: serve_level,
            detail: serve_detail,
        },
    ]
}

pub(super) fn apply_default_env_bindings(config: &mut mvp::config::LoongClawConfig) -> Vec<String> {
    let mut fixes = Vec::new();
    let default = mvp::config::WebhookChannelConfig::default();

    ensure_default_env_binding(
        &mut config.webhook.endpoint_url_env,
        default.endpoint_url_env.as_deref(),
        "set webhook.endpoint_url_env",
        &mut fixes,
    );
    ensure_default_env_binding(
        &mut config.webhook.auth_token_env,
        default.auth_token_env.as_deref(),
        "set webhook.auth_token_env",
        &mut fixes,
    );
    ensure_default_env_binding(
        &mut config.webhook.signing_secret_env,
        default.signing_secret_env.as_deref(),
        "set webhook.signing_secret_env",
        &mut fixes,
    );

    fixes
}

fn preview_detail(
    config: &mvp::config::LoongClawConfig,
    credential_state: ChannelCredentialState,
) -> String {
    match (config.webhook.enabled, credential_state) {
        (true, ChannelCredentialState::Ready) => {
            "enabled · endpoint delivery and signed serve credentials resolved".to_owned()
        }
        (false, ChannelCredentialState::Ready) => {
            "endpoint delivery and signed serve credentials resolved · can enable during onboarding"
                .to_owned()
        }
        (true, ChannelCredentialState::Partial) => {
            "enabled · endpoint_url or signing_secret missing".to_owned()
        }
        (false, ChannelCredentialState::Partial) => {
            "configured · endpoint_url or signing_secret missing".to_owned()
        }
        (true, ChannelCredentialState::Missing) => {
            "enabled · endpoint_url or signing_secret missing".to_owned()
        }
        (false, ChannelCredentialState::Missing) => "configured but disabled".to_owned(),
    }
}

fn webhook_send_credentials_ready(config: &mvp::config::LoongClawConfig) -> bool {
    config.webhook.endpoint_url().is_some()
}

fn webhook_serve_credentials_ready(config: &mvp::config::LoongClawConfig) -> bool {
    config.webhook.signing_secret().is_some()
}

fn webhook_has_any_runtime_credential(config: &mvp::config::LoongClawConfig) -> bool {
    let has_endpoint_url = config.webhook.endpoint_url().is_some();
    let has_auth_token = config.webhook.auth_token().is_some();
    let has_signing_secret = config.webhook.signing_secret().is_some();

    has_endpoint_url || has_auth_token || has_signing_secret
}

fn webhook_send_detail(send_ready: bool) -> String {
    if send_ready {
        return "endpoint delivery target resolved".to_owned();
    }

    "enabled but endpoint_url is missing".to_owned()
}

fn webhook_serve_detail(serve_ready: bool) -> String {
    if serve_ready {
        return "signed webhook serve credential resolved".to_owned();
    }

    "enabled but signing_secret is missing".to_owned()
}

fn merge_webhook_config(
    target: &mut mvp::config::WebhookChannelConfig,
    source: &mvp::config::WebhookChannelConfig,
) -> bool {
    let default = mvp::config::WebhookChannelConfig::default();
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
    if target.endpoint_url.is_none() && source.endpoint_url.is_some() {
        target.endpoint_url = source.endpoint_url.clone();
        changed = true;
    }
    if let Some(source_endpoint_url_env) = source.endpoint_url_env.as_ref() {
        let target_uses_default_env = target.endpoint_url_env.is_none()
            || target.endpoint_url_env == default.endpoint_url_env;
        let target_matches_source =
            target.endpoint_url_env.as_ref() == Some(source_endpoint_url_env);
        if target_uses_default_env && !target_matches_source {
            target.endpoint_url_env = Some(source_endpoint_url_env.clone());
            changed = true;
        }
    }
    if target.auth_token.is_none() && source.auth_token.is_some() {
        target.auth_token = source.auth_token.clone();
        changed = true;
    }
    if let Some(source_auth_token_env) = source.auth_token_env.as_ref() {
        let target_uses_default_env =
            target.auth_token_env.is_none() || target.auth_token_env == default.auth_token_env;
        let target_matches_source = target.auth_token_env.as_ref() == Some(source_auth_token_env);
        if target_uses_default_env && !target_matches_source {
            target.auth_token_env = Some(source_auth_token_env.clone());
            changed = true;
        }
    }
    if target.auth_header_name == default.auth_header_name
        && source.auth_header_name != default.auth_header_name
    {
        target.auth_header_name = source.auth_header_name.clone();
        changed = true;
    }
    if target.auth_token_prefix == default.auth_token_prefix
        && source.auth_token_prefix != default.auth_token_prefix
    {
        target.auth_token_prefix = source.auth_token_prefix.clone();
        changed = true;
    }
    if target.payload_format == default.payload_format
        && source.payload_format != default.payload_format
    {
        target.payload_format = source.payload_format;
        changed = true;
    }
    if target.payload_text_field == default.payload_text_field
        && source.payload_text_field != default.payload_text_field
    {
        target.payload_text_field = source.payload_text_field.clone();
        changed = true;
    }
    if target.public_base_url.is_none() && source.public_base_url.is_some() {
        target.public_base_url = source.public_base_url.clone();
        changed = true;
    }
    if target.signing_secret.is_none() && source.signing_secret.is_some() {
        target.signing_secret = source.signing_secret.clone();
        changed = true;
    }
    if let Some(source_signing_secret_env) = source.signing_secret_env.as_ref() {
        let target_uses_default_env = target.signing_secret_env.is_none()
            || target.signing_secret_env == default.signing_secret_env;
        let target_matches_source =
            target.signing_secret_env.as_ref() == Some(source_signing_secret_env);
        if target_uses_default_env && !target_matches_source {
            target.signing_secret_env = Some(source_signing_secret_env.clone());
            changed = true;
        }
    }

    changed
}

fn descriptor() -> &'static mvp::config::ChannelDescriptor {
    mvp::config::channel_descriptor(ID).unwrap_or(&FALLBACK_DESCRIPTOR)
}
