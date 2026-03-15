use std::time::{SystemTime, UNIX_EPOCH};

use crate::CliResult;

use super::{FeishuClient, FeishuGrant, FeishuTokenStore, parse_token_exchange_response};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeishuGrantInventory {
    pub grants: Vec<FeishuGrant>,
    pub selected_open_id: Option<String>,
    pub stale_selected_open_id: Option<String>,
    pub effective_open_id: Option<String>,
}

impl FeishuGrantInventory {
    pub fn effective_grant(&self) -> Option<&FeishuGrant> {
        let effective_open_id = self.effective_open_id.as_deref()?;
        self.grants
            .iter()
            .find(|grant| grant.principal.open_id == effective_open_id)
    }

    pub fn selection_required(&self) -> bool {
        self.effective_open_id.is_none() && self.grants.len() > 1
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeishuGrantResolution {
    pub inventory: FeishuGrantInventory,
    pub requested_open_id: Option<String>,
    pub grant: Option<FeishuGrant>,
}

impl FeishuGrantResolution {
    pub fn selected_grant(&self) -> Option<&FeishuGrant> {
        self.grant.as_ref()
    }

    pub fn into_selected_grant(self) -> Option<FeishuGrant> {
        self.grant
    }

    pub fn missing_explicit_open_id(&self) -> Option<&str> {
        (self.grant.is_none())
            .then_some(self.requested_open_id.as_deref())
            .flatten()
    }

    pub fn selection_required(&self) -> bool {
        self.requested_open_id.is_none()
            && self.grant.is_none()
            && self.inventory.selection_required()
    }

    pub fn effective_open_id(&self) -> Option<&str> {
        if self.requested_open_id.is_some() {
            return self
                .grant
                .as_ref()
                .map(|grant| grant.principal.open_id.as_str());
        }
        self.grant
            .as_ref()
            .map(|grant| grant.principal.open_id.as_str())
            .or(self.inventory.effective_open_id.as_deref())
    }

    pub fn available_open_ids(&self) -> Vec<&str> {
        self.inventory
            .grants
            .iter()
            .map(|grant| grant.principal.open_id.as_str())
            .collect()
    }
}

pub fn unix_ts_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

pub fn resolve_requested_feishu_account(
    channel: &crate::config::FeishuChannelConfig,
    requested_account_id: Option<&str>,
    ambiguous_runtime_account_hint: &str,
) -> CliResult<crate::config::ResolvedFeishuChannelConfig> {
    let requested_account_id = requested_account_id
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(requested_account_id) = requested_account_id else {
        return channel.resolve_account(None);
    };

    match channel.resolve_account(Some(requested_account_id)) {
        Ok(resolved) => Ok(resolved),
        Err(original_error) => {
            let mut matched = channel
                .configured_account_ids()
                .into_iter()
                .filter_map(|configured_account_id| {
                    channel
                        .resolve_account(Some(configured_account_id.as_str()))
                        .ok()
                })
                .filter(|resolved| resolved.account.id == requested_account_id)
                .collect::<Vec<_>>();

            match matched.len() {
                1 => Ok(matched.remove(0)),
                0 => Err(original_error),
                _ => {
                    let configured_account_ids = matched
                        .iter()
                        .map(|resolved| resolved.configured_account_id.as_str())
                        .collect::<Vec<_>>();
                    let configured_accounts = matched
                        .iter()
                        .map(describe_configured_account_choice)
                        .collect::<Vec<_>>()
                        .join(", ");
                    let disambiguation_hint = format!(
                        "Use configured_account_id {} to disambiguate",
                        format_backticked_choice_list(&configured_account_ids)
                    );
                    let hint = ambiguous_runtime_account_hint.trim();
                    if hint.is_empty() {
                        Err(format!(
                            "requested Feishu runtime account `{requested_account_id}` is ambiguous across configured accounts: {configured_accounts}. {disambiguation_hint}"
                        ))
                    } else {
                        Err(format!(
                            "requested Feishu runtime account `{requested_account_id}` is ambiguous across configured accounts: {configured_accounts}. {disambiguation_hint}. {hint}"
                        ))
                    }
                }
            }
        }
    }
}

fn describe_configured_account_choice(
    resolved: &crate::config::ResolvedFeishuChannelConfig,
) -> String {
    let configured_account_id = resolved.configured_account_id.trim();
    let configured_account_label = resolved.configured_account_label.trim();

    if configured_account_label.is_empty() || configured_account_label == configured_account_id {
        return format!("`{configured_account_id}`");
    }

    format!("`{configured_account_id}` (label `{configured_account_label}`)")
}

fn format_backticked_choice_list(values: &[&str]) -> String {
    let quoted = values
        .iter()
        .map(|value| format!("`{value}`"))
        .collect::<Vec<_>>();

    match quoted.as_slice() {
        [] => "`default`".to_owned(),
        [single] => single.clone(),
        [first, second] => format!("{first} or {second}"),
        _ => {
            let mut prefix = quoted;
            let last = prefix.pop().unwrap_or_else(|| "`default`".to_owned());
            format!("{}, or {}", prefix.join(", "), last)
        }
    }
}

pub fn resolve_selected_grant(
    store: &FeishuTokenStore,
    account_id: &str,
    open_id: Option<&str>,
) -> CliResult<Option<FeishuGrant>> {
    let resolution = resolve_grant_selection(store, account_id, open_id)?;
    if resolution.selection_required() {
        let open_ids = resolution.available_open_ids().join(", ");
        Err(format!(
            "multiple stored Feishu grants exist for account `{account_id}` ({open_ids}); run `loongclaw feishu auth list` or pass `--open-id`"
        ))
    } else if resolution.missing_explicit_open_id().is_some() {
        Err(describe_grant_selection_error(account_id, &resolution)
            .unwrap_or_else(|| format!("no stored Feishu grant for account `{account_id}`")))
    } else {
        Ok(resolution.into_selected_grant())
    }
}

pub fn resolve_grant_selection(
    store: &FeishuTokenStore,
    account_id: &str,
    open_id: Option<&str>,
) -> CliResult<FeishuGrantResolution> {
    let inventory = inspect_grants_for_account(store, account_id)?;
    let requested_open_id = open_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let grant = if let Some(requested_open_id) = requested_open_id.as_deref() {
        inventory
            .grants
            .iter()
            .find(|grant| grant.principal.open_id == requested_open_id)
            .cloned()
    } else {
        inventory.effective_grant().cloned()
    };

    Ok(FeishuGrantResolution {
        inventory,
        requested_open_id,
        grant,
    })
}

pub fn describe_grant_selection_error(
    account_id: &str,
    resolution: &FeishuGrantResolution,
) -> Option<String> {
    describe_grant_selection_error_for_display(account_id, account_id, resolution)
}

pub fn describe_grant_selection_error_for_display(
    _storage_account_id: &str,
    display_account_id: &str,
    resolution: &FeishuGrantResolution,
) -> Option<String> {
    if let Some(requested_open_id) = resolution.missing_explicit_open_id() {
        if resolution.inventory.grants.is_empty() {
            return Some(format!(
                "no stored Feishu grant for account `{display_account_id}` and open_id `{requested_open_id}`; run `loongclaw feishu auth start --account {display_account_id}` first"
            ));
        }
        let available_open_ids = resolution.available_open_ids().join(", ");
        return Some(format!(
            "no stored Feishu grant for account `{display_account_id}` and open_id `{requested_open_id}`; available open_ids: {available_open_ids}; run `loongclaw feishu auth select --account {display_account_id} --open-id <open_id>` or `loongclaw feishu auth list --account {display_account_id}`"
        ));
    }

    if resolution.selection_required() {
        let open_ids = resolution.available_open_ids().join(", ");
        let stale_selected_hint = resolution
            .inventory
            .stale_selected_open_id
            .as_deref()
            .map(|open_id| format!("stale selected open_id `{open_id}` was cleared; "))
            .unwrap_or_default();
        return Some(format!(
            "{stale_selected_hint}multiple stored Feishu grants exist for account `{display_account_id}` ({open_ids}); run `loongclaw feishu auth select --account {display_account_id} --open-id <open_id>` or pass `open_id` explicitly"
        ));
    }

    if resolution.grant.is_none() {
        return Some(format!(
            "no stored Feishu grant for account `{display_account_id}`; run `loongclaw feishu auth start --account {display_account_id}` first"
        ));
    }

    None
}

pub fn effective_selected_open_id<'a>(
    inventory: &'a FeishuGrantInventory,
    explicit_open_id: Option<&'a str>,
) -> Option<&'a str> {
    if let Some(explicit_open_id) = explicit_open_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return inventory
            .grants
            .iter()
            .find(|grant| grant.principal.open_id == explicit_open_id)
            .map(|grant| grant.principal.open_id.as_str());
    }
    inventory.effective_open_id.as_deref()
}

pub fn inspect_grants_for_account(
    store: &FeishuTokenStore,
    account_id: &str,
) -> CliResult<FeishuGrantInventory> {
    let grants = store.list_grants_for_account(account_id)?;
    let mut selected_open_id = store.load_selected_grant(account_id)?;
    let mut stale_selected_open_id = None;

    if let Some(stored_selected_open_id) = selected_open_id.as_deref() {
        let exists = grants
            .iter()
            .any(|grant| grant.principal.open_id == stored_selected_open_id);
        if !exists {
            stale_selected_open_id = Some(stored_selected_open_id.to_owned());
            let _ = store.clear_selected_grant(account_id);
            selected_open_id = None;
        }
    }

    let effective_open_id = if let Some(selected_open_id) = selected_open_id.as_deref() {
        Some(selected_open_id.to_owned())
    } else if grants.len() == 1 {
        grants.first().map(|grant| grant.principal.open_id.clone())
    } else {
        None
    };

    Ok(FeishuGrantInventory {
        grants,
        selected_open_id,
        stale_selected_open_id,
        effective_open_id,
    })
}

pub async fn ensure_fresh_user_grant(
    client: &FeishuClient,
    store: &FeishuTokenStore,
    grant: &FeishuGrant,
) -> CliResult<FeishuGrant> {
    let now_s = unix_ts_now();
    if !grant.is_access_token_expired(now_s) {
        return Ok(grant.clone());
    }
    if grant.is_refresh_token_expired(now_s) {
        return Err(format!(
            "stored Feishu refresh token expired for `{}`; rerun the Feishu OAuth flow",
            grant.principal.storage_key()
        ));
    }

    let payload = client
        .refresh_user_access_token(&grant.refresh_token)
        .await?;
    let refreshed = parse_token_exchange_response(&payload, now_s, grant.principal.clone())?;
    store.save_grant(&refreshed)?;
    Ok(refreshed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn temp_dir(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "loongclaw-feishu-runtime-{label}-{}",
            unix_ts_now()
        ))
    }

    fn sample_grant(account_id: &str, open_id: &str, now_s: i64) -> FeishuGrant {
        FeishuGrant {
            principal: super::super::FeishuUserPrincipal {
                account_id: account_id.to_owned(),
                open_id: open_id.to_owned(),
                union_id: Some("on_456".to_owned()),
                user_id: Some("u_789".to_owned()),
                name: Some("Alice".to_owned()),
                tenant_key: Some("tenant_x".to_owned()),
                avatar_url: None,
                email: Some("alice@example.com".to_owned()),
                enterprise_email: None,
            },
            access_token: format!("u-token-{open_id}"),
            refresh_token: format!("r-token-{open_id}"),
            scopes: super::super::FeishuGrantScopeSet::from_scopes([
                "offline_access",
                "docx:document:readonly",
            ]),
            access_expires_at_s: now_s + 3600,
            refresh_expires_at_s: now_s + 86_400,
            refreshed_at_s: now_s,
        }
    }

    #[test]
    fn inspect_grants_for_account_clears_stale_selected_open_id_and_uses_single_grant() {
        let temp_dir = temp_dir("stale-selected-single");
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");
        let store = FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
        let now_s = unix_ts_now();
        store
            .save_grant(&sample_grant("feishu_main", "ou_123", now_s))
            .expect("save grant");
        store
            .set_selected_grant("feishu_main", "ou_missing", now_s + 1)
            .expect("persist stale selected grant");

        let inventory =
            inspect_grants_for_account(&store, "feishu_main").expect("inspect account grants");

        assert_eq!(inventory.selected_open_id, None);
        assert_eq!(
            inventory.stale_selected_open_id.as_deref(),
            Some("ou_missing")
        );
        assert_eq!(inventory.effective_open_id.as_deref(), Some("ou_123"));
        assert!(!inventory.selection_required());
        assert_eq!(
            store
                .load_selected_grant("feishu_main")
                .expect("load selected grant after cleanup"),
            None
        );
    }

    #[test]
    fn inspect_grants_for_account_marks_selection_required_after_stale_selected_open_id() {
        let temp_dir = temp_dir("stale-selected-multi");
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");
        let store = FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
        let now_s = unix_ts_now();
        store
            .save_grant(&sample_grant("feishu_main", "ou_123", now_s))
            .expect("save first grant");
        store
            .save_grant(&sample_grant("feishu_main", "ou_456", now_s + 1))
            .expect("save second grant");
        store
            .set_selected_grant("feishu_main", "ou_missing", now_s + 2)
            .expect("persist stale selected grant");

        let inventory =
            inspect_grants_for_account(&store, "feishu_main").expect("inspect account grants");

        assert_eq!(inventory.selected_open_id, None);
        assert_eq!(
            inventory.stale_selected_open_id.as_deref(),
            Some("ou_missing")
        );
        assert_eq!(inventory.effective_open_id, None);
        assert!(inventory.selection_required());
    }

    #[test]
    fn resolve_selected_grant_suggests_auth_list_when_multiple_grants_exist() {
        let temp_dir = temp_dir("multi-grant-hints");
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");
        let store = FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
        let now_s = unix_ts_now();
        store
            .save_grant(&sample_grant("feishu_main", "ou_123", now_s))
            .expect("save first grant");
        store
            .save_grant(&sample_grant("feishu_main", "ou_456", now_s + 1))
            .expect("save second grant");

        let error = resolve_selected_grant(&store, "feishu_main", None)
            .expect_err("multiple grants should require explicit selection");

        assert!(error.contains("loongclaw feishu auth list"));
        assert!(error.contains("--open-id"));
        assert!(error.contains("ou_123"));
        assert!(error.contains("ou_456"));
    }

    #[test]
    fn resolve_selected_grant_reports_missing_explicit_open_id() {
        let temp_dir = temp_dir("missing-explicit-open-id");
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");
        let store = FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
        let now_s = unix_ts_now();
        store
            .save_grant(&sample_grant("feishu_main", "ou_123", now_s))
            .expect("save grant");

        let error = resolve_selected_grant(&store, "feishu_main", Some("ou_missing"))
            .expect_err("unknown explicit open_id should fail");

        assert!(error.contains("open_id `ou_missing`"));
        assert!(error.contains("ou_123"));
        assert!(error.contains("auth select --account feishu_main"));
    }

    #[test]
    fn resolve_requested_feishu_account_accepts_unique_runtime_account_match() {
        let channel = crate::config::FeishuChannelConfig {
            enabled: true,
            accounts: BTreeMap::from([(
                "work".to_owned(),
                crate::config::FeishuAccountConfig {
                    account_id: Some("feishu_shared".to_owned()),
                    app_id: Some("cli_work".to_owned()),
                    app_secret: Some("app-secret-work".to_owned()),
                    ..crate::config::FeishuAccountConfig::default()
                },
            )]),
            ..crate::config::FeishuChannelConfig::default()
        };

        let resolved = resolve_requested_feishu_account(
            &channel,
            Some("feishu_shared"),
            "rerun with `--account <configured_account_id>`",
        )
        .expect("resolve unique runtime-account alias");

        assert_eq!(resolved.configured_account_id, "work");
        assert_eq!(resolved.account.id, "feishu_shared");
    }

    #[test]
    fn resolve_requested_feishu_account_reports_usable_configured_ids_for_ambiguous_runtime_match()
    {
        let channel = crate::config::FeishuChannelConfig {
            enabled: true,
            accounts: BTreeMap::from([
                (
                    "Work Bot".to_owned(),
                    crate::config::FeishuAccountConfig {
                        account_id: Some("feishu_shared".to_owned()),
                        app_id: Some("cli_work".to_owned()),
                        app_secret: Some("app-secret-work".to_owned()),
                        ..crate::config::FeishuAccountConfig::default()
                    },
                ),
                (
                    "Alerts Bot".to_owned(),
                    crate::config::FeishuAccountConfig {
                        account_id: Some("feishu_shared".to_owned()),
                        app_id: Some("cli_alerts".to_owned()),
                        app_secret: Some("app-secret-alerts".to_owned()),
                        ..crate::config::FeishuAccountConfig::default()
                    },
                ),
            ]),
            ..crate::config::FeishuChannelConfig::default()
        };

        let error = resolve_requested_feishu_account(
            &channel,
            Some("feishu_shared"),
            "rerun with `--account <configured_account_id>`",
        )
        .expect_err("ambiguous runtime-account alias should fail");

        assert!(error.contains("requested Feishu runtime account `feishu_shared` is ambiguous"));
        assert!(error.contains("work-bot"));
        assert!(error.contains("alerts-bot"));
        assert!(error.contains("Work Bot"));
        assert!(error.contains("Alerts Bot"));
        assert!(
            error.contains("Use configured_account_id `alerts-bot` or `work-bot` to disambiguate")
        );
        assert!(error.contains("--account <configured_account_id>"));
    }
}
