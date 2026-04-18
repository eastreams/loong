use loong_app as mvp;

use super::ChannelDoctorCheck;
use super::ensure_default_env_binding;
use super::{ChannelCheckLevel, ChannelPreflightCheck, ChannelPreview, build_channel_preview};
use crate::migration::ChannelCredentialState;
use crate::migration::{ChannelImportReadiness, ImportSurfaceLevel};

pub(super) const ID: &str = "matrix";

const FALLBACK_DESCRIPTOR: mvp::config::ChannelDescriptor = mvp::config::ChannelDescriptor {
    id: ID,
    label: "matrix",
    surface_label: "matrix channel",
    runtime_kind: mvp::config::ChannelRuntimeKind::RuntimeBacked,
    serve_subcommand: Some("matrix-serve"),
};

#[derive(Debug, Clone)]
struct EffectiveMatrixConfig {
    enabled: bool,
    user_id: Option<String>,
    access_token: Option<String>,
    access_token_env: Option<String>,
    base_url: Option<String>,
    sync_timeout_s: u64,
    allowed_room_ids: Vec<String>,
    ignore_self_messages: bool,
}

impl EffectiveMatrixConfig {
    fn access_token(&self) -> Option<String> {
        crate::doctor_cli::resolve_secret_value(
            self.access_token.as_deref(),
            self.access_token_env.as_deref(),
        )
    }

    fn has_base_url(&self) -> bool {
        self.base_url
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
    }
}

pub(super) fn collect_preview(
    config: &mvp::config::LoongConfig,
    readiness: &ChannelImportReadiness,
    source: &str,
) -> Option<ChannelPreview> {
    let effective = effective_matrix_config(config);
    let access_token_resolved = readiness.is_ready(ID) || effective.access_token().is_some();
    let default_matrix = mvp::config::MatrixChannelConfig::default();
    let configured = effective.enabled
        || access_token_resolved
        || effective.access_token_env != default_matrix.access_token_env
        || effective.base_url != default_matrix.base_url
        || effective.sync_timeout_s != default_matrix.sync_timeout_s
        || !effective.allowed_room_ids.is_empty()
        || effective.ignore_self_messages != default_matrix.ignore_self_messages
        || effective.user_id != default_matrix.user_id;
    if !configured {
        return None;
    }

    let level = if access_token_resolved {
        ImportSurfaceLevel::Ready
    } else if effective.enabled {
        ImportSurfaceLevel::Review
    } else {
        ImportSurfaceLevel::Blocked
    };
    let detail = if effective.enabled && access_token_resolved {
        format!(
            "enabled, access token resolved, {} allowed room id(s)",
            effective.allowed_room_ids.len()
        )
    } else if access_token_resolved {
        "access token resolved, can enable during onboarding".to_owned()
    } else if effective.enabled {
        "enabled, access token missing".to_owned()
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
    merge_matrix_config(&mut target.matrix, &source.matrix)
}

pub(super) fn readiness_state(config: &mvp::config::LoongConfig) -> ChannelCredentialState {
    if effective_matrix_config(config).access_token().is_some() {
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
        target.matrix.enabled = true;
    }
}

pub(super) fn collect_preflight_checks(
    config: &mvp::config::LoongConfig,
) -> Vec<ChannelPreflightCheck> {
    let state = readiness_state(config);
    let effective = effective_matrix_config(config);
    let base_url_ready = effective.has_base_url();

    vec![
        ChannelPreflightCheck {
            name: descriptor().surface_label,
            level: if state.is_ready() {
                ChannelCheckLevel::Pass
            } else {
                ChannelCheckLevel::Warn
            },
            detail: if state.is_ready() {
                "access token resolved".to_owned()
            } else {
                "enabled but access token is missing (matrix.access_token or env)".to_owned()
            },
        },
        ChannelPreflightCheck {
            name: "matrix room sync",
            level: if base_url_ready {
                ChannelCheckLevel::Pass
            } else {
                ChannelCheckLevel::Warn
            },
            detail: if base_url_ready {
                "homeserver base url is configured".to_owned()
            } else {
                "matrix.base_url is missing".to_owned()
            },
        },
    ]
}

pub(super) fn collect_doctor_checks(config: &mvp::config::LoongConfig) -> Vec<ChannelDoctorCheck> {
    let state = readiness_state(config);
    let effective = effective_matrix_config(config);
    let base_url_ready = effective.has_base_url();

    vec![
        ChannelDoctorCheck {
            name: descriptor().surface_label,
            level: if state.is_ready() {
                ChannelCheckLevel::Pass
            } else {
                ChannelCheckLevel::Fail
            },
            detail: if state.is_ready() {
                "access token resolved".to_owned()
            } else {
                "enabled but access token is missing (matrix.access_token or env)".to_owned()
            },
        },
        ChannelDoctorCheck {
            name: "matrix room sync",
            level: if base_url_ready {
                ChannelCheckLevel::Pass
            } else {
                ChannelCheckLevel::Fail
            },
            detail: if base_url_ready {
                "homeserver base url is configured".to_owned()
            } else {
                "matrix.base_url is missing".to_owned()
            },
        },
    ]
}

pub(super) fn apply_default_env_bindings(config: &mut mvp::config::LoongConfig) -> Vec<String> {
    let mut fixes = Vec::new();
    let default = mvp::config::MatrixChannelConfig::default();
    ensure_default_env_binding(
        &mut config.matrix.access_token_env,
        default.access_token_env.as_deref(),
        "set matrix.access_token_env",
        &mut fixes,
    );
    fixes
}

fn merge_matrix_config(
    target: &mut mvp::config::MatrixChannelConfig,
    source: &mvp::config::MatrixChannelConfig,
) -> bool {
    let default = mvp::config::MatrixChannelConfig::default();
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
    if target.user_id.is_none() && source.user_id.is_some() {
        target.user_id = source.user_id.clone();
        changed = true;
    }
    if target.access_token.is_none() && source.access_token.is_some() {
        target.access_token = source.access_token.clone();
        changed = true;
    }
    if let Some(source_access_token_env) = source.access_token_env.as_ref() {
        let target_uses_default_env = target.access_token_env.is_none()
            || target.access_token_env == default.access_token_env;
        if target_uses_default_env
            && target.access_token_env.as_ref() != Some(source_access_token_env)
        {
            target.access_token_env = Some(source_access_token_env.clone());
            changed = true;
        }
    }
    if target.base_url == default.base_url && source.base_url != default.base_url {
        target.base_url = source.base_url.clone();
        changed = true;
    }
    if target.sync_timeout_s == default.sync_timeout_s
        && source.sync_timeout_s != default.sync_timeout_s
    {
        target.sync_timeout_s = source.sync_timeout_s;
        changed = true;
    }
    for room_id in &source.allowed_room_ids {
        changed |= merge_unique_string(&mut target.allowed_room_ids, room_id);
    }
    if target.ignore_self_messages == default.ignore_self_messages
        && source.ignore_self_messages != default.ignore_self_messages
    {
        target.ignore_self_messages = source.ignore_self_messages;
        changed = true;
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
                changed |= merge_matrix_account_config(entry.get_mut(), source_account);
            }
        }
    }

    changed
}

fn effective_matrix_config(config: &mvp::config::LoongConfig) -> EffectiveMatrixConfig {
    if let Ok(resolved) = config.matrix.resolve_account(None) {
        let access_token = resolved.access_token();
        return EffectiveMatrixConfig {
            enabled: resolved.enabled,
            user_id: resolved.user_id,
            access_token,
            access_token_env: resolved.access_token_env,
            base_url: resolved.base_url,
            sync_timeout_s: resolved.sync_timeout_s,
            allowed_room_ids: resolved.allowed_room_ids,
            ignore_self_messages: resolved.ignore_self_messages,
        };
    }

    let access_token = config.matrix.access_token();
    EffectiveMatrixConfig {
        enabled: config.matrix.enabled,
        user_id: config.matrix.user_id.clone(),
        access_token,
        access_token_env: config.matrix.access_token_env.clone(),
        base_url: config.matrix.base_url.clone(),
        sync_timeout_s: config.matrix.sync_timeout_s,
        allowed_room_ids: config.matrix.allowed_room_ids.clone(),
        ignore_self_messages: config.matrix.ignore_self_messages,
    }
}

fn merge_matrix_account_config(
    target: &mut mvp::config::MatrixAccountConfig,
    source: &mvp::config::MatrixAccountConfig,
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
    if target.user_id.is_none() && source.user_id.is_some() {
        target.user_id = source.user_id.clone();
        changed = true;
    }
    if target.access_token.is_none() && source.access_token.is_some() {
        target.access_token = source.access_token.clone();
        changed = true;
    }
    if target.access_token_env.is_none() && source.access_token_env.is_some() {
        target.access_token_env = source.access_token_env.clone();
        changed = true;
    }
    if target.base_url.is_none() && source.base_url.is_some() {
        target.base_url = source.base_url.clone();
        changed = true;
    }
    if target.sync_timeout_s.is_none() && source.sync_timeout_s.is_some() {
        target.sync_timeout_s = source.sync_timeout_s;
        changed = true;
    }
    match (&mut target.allowed_room_ids, &source.allowed_room_ids) {
        (None, Some(room_ids)) => {
            target.allowed_room_ids = Some(room_ids.clone());
            changed = true;
        }
        (Some(target_room_ids), Some(source_room_ids)) => {
            for room_id in source_room_ids {
                changed |= merge_unique_string(target_room_ids, room_id);
            }
        }
        _ => {}
    }
    if target.ignore_self_messages.is_none() && source.ignore_self_messages.is_some() {
        target.ignore_self_messages = source.ignore_self_messages;
        changed = true;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn collect_doctor_checks_uses_default_account_settings() {
        let mut config = mvp::config::LoongConfig::default();
        config.matrix.enabled = true;
        config.matrix.default_account = Some("ops".to_owned());
        config.matrix.accounts.insert(
            "ops".to_owned(),
            mvp::config::MatrixAccountConfig {
                access_token: Some(loong_contracts::SecretRef::Inline(
                    "matrix-token".to_owned(),
                )),
                base_url: Some("https://matrix.example.org".to_owned()),
                allowed_room_ids: Some(vec!["!ops:example.org".to_owned()]),
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
    fn merge_matrix_config_copies_custom_defaults_and_account_overrides() {
        let mut target = mvp::config::MatrixChannelConfig::default();
        let source = mvp::config::MatrixChannelConfig {
            enabled: true,
            default_account: Some("ops".to_owned()),
            access_token_env: Some("CUSTOM_TOP_LEVEL_MATRIX_TOKEN".to_owned()),
            accounts: BTreeMap::from([(
                "ops".to_owned(),
                mvp::config::MatrixAccountConfig {
                    user_id: Some("@ops-bot:example.org".to_owned()),
                    access_token_env: Some("CUSTOM_MATRIX_TOKEN".to_owned()),
                    base_url: Some("https://matrix.example.org".to_owned()),
                    allowed_room_ids: Some(vec!["!ops:example.org".to_owned()]),
                    ignore_self_messages: Some(false),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };

        let changed = merge_matrix_config(&mut target, &source);

        assert!(changed);
        assert!(target.enabled);
        assert_eq!(target.default_account.as_deref(), Some("ops"));
        assert_eq!(
            target.access_token_env.as_deref(),
            Some("CUSTOM_TOP_LEVEL_MATRIX_TOKEN")
        );
        let account = target.accounts.get("ops").expect("merged matrix account");
        assert_eq!(account.user_id.as_deref(), Some("@ops-bot:example.org"));
        assert_eq!(
            account.access_token_env.as_deref(),
            Some("CUSTOM_MATRIX_TOKEN")
        );
        assert_eq!(
            account.base_url.as_deref(),
            Some("https://matrix.example.org")
        );
        assert_eq!(
            account.allowed_room_ids.as_ref(),
            Some(&vec!["!ops:example.org".to_owned()])
        );
        assert_eq!(account.ignore_self_messages, Some(false));
    }
}
