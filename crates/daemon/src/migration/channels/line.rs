use loongclaw_app as mvp;

use super::ChannelDoctorCheck;
use super::ensure_default_env_binding;
use super::{ChannelCheckLevel, ChannelPreflightCheck, ChannelPreview, build_channel_preview};
use crate::migration::ChannelCredentialState;
use crate::migration::{ChannelImportReadiness, ImportSurfaceLevel};

pub(super) const ID: &str = "line";

const FALLBACK_DESCRIPTOR: mvp::config::ChannelDescriptor = mvp::config::ChannelDescriptor {
    id: ID,
    label: "line",
    surface_label: "line channel",
    runtime_kind: mvp::config::ChannelRuntimeKind::Service,
    serve_subcommand: Some("line-serve"),
};

pub(super) fn collect_preview(
    config: &mvp::config::LoongClawConfig,
    readiness: &ChannelImportReadiness,
    source: &str,
) -> Option<ChannelPreview> {
    let credential_state = readiness.state(ID);
    let default_line = mvp::config::LineChannelConfig::default();
    let configured = config.line.enabled
        || config.line.account_id.is_some()
        || config.line.default_account.is_some()
        || !config.line.accounts.is_empty()
        || credential_state != ChannelCredentialState::Missing
        || config.line.channel_access_token_env != default_line.channel_access_token_env
        || config.line.channel_secret_env != default_line.channel_secret_env
        || config.line.api_base_url != default_line.api_base_url;
    if !configured {
        return None;
    }

    let level = if credential_state.is_ready() {
        ImportSurfaceLevel::Ready
    } else if config.line.enabled {
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
    merge_line_config(&mut target.line, &source.line)
}

pub(super) fn readiness_state(config: &mvp::config::LoongClawConfig) -> ChannelCredentialState {
    let send_ready = line_send_credentials_ready(config);
    let serve_ready = line_serve_credentials_ready(config);
    let has_any_credential = line_has_any_runtime_credential(config);

    if send_ready && serve_ready {
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
        target.line.enabled = true;
    }
}

pub(super) fn collect_preflight_checks(
    config: &mvp::config::LoongClawConfig,
) -> Vec<ChannelPreflightCheck> {
    let send_ready = line_send_credentials_ready(config);
    let serve_ready = line_serve_credentials_ready(config);
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
    let send_detail = line_send_detail(send_ready);
    let serve_detail = line_serve_detail(serve_ready);

    vec![
        ChannelPreflightCheck {
            name: descriptor().surface_label,
            level: send_level,
            detail: send_detail,
        },
        ChannelPreflightCheck {
            name: "line webhook service",
            level: serve_level,
            detail: serve_detail,
        },
    ]
}

pub(super) fn collect_doctor_checks(
    config: &mvp::config::LoongClawConfig,
) -> Vec<ChannelDoctorCheck> {
    let send_ready = line_send_credentials_ready(config);
    let serve_ready = line_serve_credentials_ready(config);
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
    let send_detail = line_send_detail(send_ready);
    let serve_detail = line_serve_detail(serve_ready);

    vec![
        ChannelDoctorCheck {
            name: descriptor().surface_label,
            level: send_level,
            detail: send_detail,
        },
        ChannelDoctorCheck {
            name: "line webhook service",
            level: serve_level,
            detail: serve_detail,
        },
    ]
}

pub(super) fn apply_default_env_bindings(config: &mut mvp::config::LoongClawConfig) -> Vec<String> {
    let mut fixes = Vec::new();
    let default = mvp::config::LineChannelConfig::default();

    ensure_default_env_binding(
        &mut config.line.channel_access_token_env,
        default.channel_access_token_env.as_deref(),
        "set line.channel_access_token_env",
        &mut fixes,
    );
    ensure_default_env_binding(
        &mut config.line.channel_secret_env,
        default.channel_secret_env.as_deref(),
        "set line.channel_secret_env",
        &mut fixes,
    );

    fixes
}

fn preview_detail(
    config: &mvp::config::LoongClawConfig,
    credential_state: ChannelCredentialState,
) -> String {
    match (config.line.enabled, credential_state) {
        (true, ChannelCredentialState::Ready) => {
            "enabled · push and webhook credentials resolved".to_owned()
        }
        (false, ChannelCredentialState::Ready) => {
            "push and webhook credentials resolved · can enable during onboarding".to_owned()
        }
        (true, ChannelCredentialState::Partial) => {
            "enabled · channel_access_token or channel_secret missing".to_owned()
        }
        (false, ChannelCredentialState::Partial) => {
            "configured · channel_access_token or channel_secret missing".to_owned()
        }
        (true, ChannelCredentialState::Missing) => {
            "enabled · channel_access_token or channel_secret missing".to_owned()
        }
        (false, ChannelCredentialState::Missing) => "configured but disabled".to_owned(),
    }
}

fn line_send_credentials_ready(config: &mvp::config::LoongClawConfig) -> bool {
    config.line.channel_access_token().is_some()
}

fn line_serve_credentials_ready(config: &mvp::config::LoongClawConfig) -> bool {
    let has_channel_access_token = config.line.channel_access_token().is_some();
    let has_channel_secret = config.line.channel_secret().is_some();

    has_channel_access_token && has_channel_secret
}

fn line_has_any_runtime_credential(config: &mvp::config::LoongClawConfig) -> bool {
    let has_channel_access_token = config.line.channel_access_token().is_some();
    let has_channel_secret = config.line.channel_secret().is_some();

    has_channel_access_token || has_channel_secret
}

fn line_send_detail(send_ready: bool) -> String {
    if send_ready {
        return "push message credential resolved".to_owned();
    }

    "enabled but channel_access_token is missing".to_owned()
}

fn line_serve_detail(serve_ready: bool) -> String {
    if serve_ready {
        return "webhook reply credentials resolved".to_owned();
    }

    "enabled but channel_access_token or channel_secret is missing".to_owned()
}

fn merge_line_config(
    target: &mut mvp::config::LineChannelConfig,
    source: &mvp::config::LineChannelConfig,
) -> bool {
    let default = mvp::config::LineChannelConfig::default();
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
    if target.channel_access_token.is_none() && source.channel_access_token.is_some() {
        target.channel_access_token = source.channel_access_token.clone();
        changed = true;
    }
    if let Some(source_channel_access_token_env) = source.channel_access_token_env.as_ref() {
        let target_uses_default_env = target.channel_access_token_env.is_none()
            || target.channel_access_token_env == default.channel_access_token_env;
        let target_matches_source =
            target.channel_access_token_env.as_ref() == Some(source_channel_access_token_env);
        if target_uses_default_env && !target_matches_source {
            target.channel_access_token_env = Some(source_channel_access_token_env.clone());
            changed = true;
        }
    }
    if target.channel_secret.is_none() && source.channel_secret.is_some() {
        target.channel_secret = source.channel_secret.clone();
        changed = true;
    }
    if let Some(source_channel_secret_env) = source.channel_secret_env.as_ref() {
        let target_uses_default_env = target.channel_secret_env.is_none()
            || target.channel_secret_env == default.channel_secret_env;
        let target_matches_source =
            target.channel_secret_env.as_ref() == Some(source_channel_secret_env);
        if target_uses_default_env && !target_matches_source {
            target.channel_secret_env = Some(source_channel_secret_env.clone());
            changed = true;
        }
    }
    let target_uses_default_api_base =
        target.api_base_url.is_none() || target.api_base_url == default.api_base_url;
    if target_uses_default_api_base && source.api_base_url != default.api_base_url {
        target.api_base_url = source.api_base_url.clone();
        changed = true;
    }
    for (account_key, source_account) in &source.accounts {
        if target.accounts.contains_key(account_key) {
            continue;
        }
        target.accounts.insert(account_key.clone(), source_account.clone());
        changed = true;
    }

    changed
}

fn descriptor() -> &'static mvp::config::ChannelDescriptor {
    mvp::config::channel_descriptor(ID).unwrap_or(&FALLBACK_DESCRIPTOR)
}
