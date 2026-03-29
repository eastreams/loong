use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use clap::ValueEnum;
use rand::RngExt;
use serde::Serialize;
use sha2::{Digest, Sha256};

use loongclaw_app as mvp;
use loongclaw_spec::CliResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FeishuAuthCapability {
    ReadOnly,
    DocWrite,
    MessageWrite,
    All,
}

impl FeishuAuthCapability {
    pub fn as_cli_value(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::DocWrite => "doc-write",
            Self::MessageWrite => "message-write",
            Self::All => "all",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FeishuGrantRecommendations {
    pub auth_start_command: Option<String>,
    pub select_command: Option<String>,
    pub missing_required_scopes: Vec<String>,
    pub missing_doc_write_scope: bool,
    pub missing_message_write_scope: bool,
    pub requested_open_id_missing: bool,
    pub refresh_token_expired: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FeishuAccountRecommendations {
    pub auth_start_command: Option<String>,
    pub select_command: Option<String>,
    pub selection_required: bool,
    pub stale_selected_open_id: Option<String>,
}

pub struct FeishuDaemonContext {
    pub config_path: PathBuf,
    pub config: mvp::config::LoongClawConfig,
    pub resolved: mvp::config::ResolvedFeishuChannelConfig,
    pub store: mvp::feishu::FeishuTokenStore,
}

impl FeishuDaemonContext {
    pub fn build_client(&self) -> CliResult<mvp::feishu::FeishuClient> {
        mvp::feishu::FeishuClient::from_configs(&self.resolved, &self.config.feishu_integration)
    }

    pub fn account_id(&self) -> &str {
        self.resolved.account.id.as_str()
    }

    pub fn default_scopes(&self) -> Vec<String> {
        self.config.feishu_integration.trimmed_default_scopes()
    }
}

pub fn load_feishu_daemon_context(
    config_path: Option<&str>,
    account: Option<&str>,
) -> CliResult<FeishuDaemonContext> {
    let (config_path, config) = mvp::config::load(config_path)?;
    let resolved = mvp::feishu::resolve_requested_feishu_account(
        &config.feishu,
        account,
        "rerun with `--account <configured_account_id>` using one of those configured accounts",
    )?;
    let store =
        mvp::feishu::FeishuTokenStore::new(config.feishu_integration.resolved_sqlite_path());
    Ok(FeishuDaemonContext {
        config_path,
        config,
        resolved,
        store,
    })
}

pub fn unix_ts_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

pub fn generate_oauth_state() -> String {
    random_urlsafe_token(24)
}

pub fn build_pkce_pair() -> (String, String) {
    let verifier = random_urlsafe_token(32);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

pub fn resolve_scopes(
    default_scopes: &[String],
    override_scopes: &[String],
    capabilities: &[FeishuAuthCapability],
    include_message_write: bool,
) -> Vec<String> {
    let mut scopes = if override_scopes.is_empty() {
        default_scopes
            .iter()
            .filter_map(|raw| normalize_scope(raw))
            .collect::<Vec<_>>()
    } else {
        let mut scopes = Vec::new();
        for raw in override_scopes {
            push_scope_if_missing(&mut scopes, raw);
        }
        scopes
    };

    for capability in normalized_auth_start_capabilities(capabilities, include_message_write) {
        match capability {
            FeishuAuthCapability::ReadOnly => {
                for scope in default_scopes {
                    push_scope_if_missing(&mut scopes, scope);
                }
            }
            FeishuAuthCapability::DocWrite => {
                for scope in mvp::feishu::FEISHU_DOC_WRITE_ACCEPTED_SCOPES {
                    push_scope_if_missing(&mut scopes, scope);
                }
            }
            FeishuAuthCapability::MessageWrite => {
                for scope in mvp::feishu::FEISHU_MESSAGE_WRITE_RECOMMENDED_SCOPES {
                    push_scope_if_missing(&mut scopes, scope);
                }
            }
            FeishuAuthCapability::All => {
                for scope in default_scopes {
                    push_scope_if_missing(&mut scopes, scope);
                }
                for scope in mvp::feishu::FEISHU_DOC_WRITE_ACCEPTED_SCOPES {
                    push_scope_if_missing(&mut scopes, scope);
                }
                for scope in mvp::feishu::FEISHU_MESSAGE_WRITE_RECOMMENDED_SCOPES {
                    push_scope_if_missing(&mut scopes, scope);
                }
            }
        }
    }

    scopes
}

pub fn normalized_auth_start_capabilities(
    capabilities: &[FeishuAuthCapability],
    include_message_write: bool,
) -> Vec<FeishuAuthCapability> {
    let mut normalized = Vec::new();
    for capability in capabilities {
        if !normalized.iter().any(|existing| existing == capability) {
            normalized.push(*capability);
        }
    }
    if include_message_write && !normalized.contains(&FeishuAuthCapability::MessageWrite) {
        normalized.push(FeishuAuthCapability::MessageWrite);
    }
    normalized
}

pub fn feishu_auth_start_command_hint(
    configured_account_id: &str,
    include_message_write: bool,
    include_doc_write: bool,
) -> String {
    let mut parts = vec!["loongclaw feishu auth start".to_owned()];
    let configured_account_id = configured_account_id.trim();
    if !configured_account_id.is_empty() {
        parts.push(format!("--account {configured_account_id}"));
    }
    if include_doc_write {
        parts.push("--capability doc-write".to_owned());
    }
    if include_message_write {
        parts.push("--capability message-write".to_owned());
    }
    parts.join(" ")
}

pub fn feishu_auth_select_command_hint(configured_account_id: &str) -> String {
    let mut parts = vec!["loongclaw feishu auth select".to_owned()];
    let configured_account_id = configured_account_id.trim();
    if !configured_account_id.is_empty() {
        parts.push(format!("--account {configured_account_id}"));
    }
    parts.push("--open-id <open_id>".to_owned());
    parts.join(" ")
}

pub fn recommended_auth_start_command_for_grant(
    configured_account_id: &str,
    grant: Option<&mvp::feishu::FeishuGrant>,
    now_s: i64,
    required_scopes: &[String],
) -> Option<String> {
    let status = mvp::feishu::auth::summarize_grant_status(grant, now_s, required_scopes);
    let doc_write_status = mvp::feishu::summarize_doc_write_scope_status(grant);
    let write_status = mvp::feishu::summarize_message_write_scope_status(grant);
    let needs_auth_start = !status.has_grant
        || status.refresh_token_expired
        || !status.missing_scopes.is_empty()
        || grant.is_some() && !doc_write_status.ready
        || grant.is_some() && !write_status.ready;
    if !needs_auth_start {
        return None;
    }

    Some(feishu_auth_start_command_hint(
        configured_account_id,
        grant.is_some() && !doc_write_status.ready,
        grant.is_some() && !write_status.ready,
    ))
}

pub fn build_grant_recommendations(
    configured_account_id: &str,
    grant: Option<&mvp::feishu::FeishuGrant>,
    now_s: i64,
    required_scopes: &[String],
) -> FeishuGrantRecommendations {
    let status = mvp::feishu::auth::summarize_grant_status(grant, now_s, required_scopes);
    let doc_write_status = mvp::feishu::summarize_doc_write_scope_status(grant);
    let write_status = mvp::feishu::summarize_message_write_scope_status(grant);

    FeishuGrantRecommendations {
        auth_start_command: recommended_auth_start_command_for_grant(
            configured_account_id,
            grant,
            now_s,
            required_scopes,
        ),
        select_command: None,
        missing_required_scopes: status.missing_scopes,
        missing_doc_write_scope: grant.is_some() && !doc_write_status.ready,
        missing_message_write_scope: grant.is_some() && !write_status.ready,
        requested_open_id_missing: false,
        refresh_token_expired: status.refresh_token_expired,
    }
}

pub fn build_account_recommendations(
    configured_account_id: &str,
    inventory: &mvp::feishu::FeishuGrantInventory,
) -> FeishuAccountRecommendations {
    FeishuAccountRecommendations {
        auth_start_command: inventory
            .grants
            .is_empty()
            .then(|| feishu_auth_start_command_hint(configured_account_id, false, false)),
        select_command: inventory
            .selection_required()
            .then(|| feishu_auth_select_command_hint(configured_account_id)),
        selection_required: inventory.selection_required(),
        stale_selected_open_id: inventory.stale_selected_open_id.clone(),
    }
}

pub fn resolve_selected_grant(
    store: &mvp::feishu::FeishuTokenStore,
    account_id: &str,
    open_id: Option<&str>,
) -> CliResult<Option<mvp::feishu::FeishuGrant>> {
    let resolution = mvp::feishu::resolve_grant_selection(store, account_id, open_id)?;
    if let Some(grant) = resolution.selected_grant().cloned() {
        return Ok(Some(grant));
    }

    if resolution.selection_required() {
        let open_ids = resolution.available_open_ids().join(", ");
        return Err(format!(
            "multiple stored Feishu grants exist for account `{account_id}` ({open_ids}); run `loongclaw feishu auth list` or pass `--open-id`"
        ));
    }

    if resolution.missing_explicit_open_id().is_some() {
        return Err(
            mvp::feishu::describe_grant_selection_error(account_id, &resolution)
                .unwrap_or_else(|| format!("no stored Feishu grant for account `{account_id}`")),
        );
    }

    Ok(None)
}

fn random_urlsafe_token(bytes_len: usize) -> String {
    let mut bytes = vec![0_u8; bytes_len.max(16)];
    rand::rng().fill(bytes.as_mut_slice());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn normalize_scope(raw: &str) -> Option<String> {
    let scope = raw.trim();
    (!scope.is_empty()).then(|| scope.to_owned())
}

fn push_scope_if_missing(scopes: &mut Vec<String>, raw: &str) {
    let Some(scope) = normalize_scope(raw) else {
        return;
    };
    if scopes.iter().any(|existing| existing == &scope) {
        return;
    }
    scopes.push(scope);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_grant(account_id: &str, open_id: &str, now_s: i64) -> mvp::feishu::FeishuGrant {
        mvp::feishu::FeishuGrant {
            principal: mvp::feishu::FeishuUserPrincipal {
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
            scopes: mvp::feishu::FeishuGrantScopeSet::from_scopes([
                "offline_access",
                "docx:document:readonly",
            ]),
            access_expires_at_s: now_s + 3600,
            refresh_expires_at_s: now_s + 86_400,
            refreshed_at_s: now_s,
        }
    }

    #[test]
    fn disabled_feishu_channel_still_allows_integration_context_loading() {
        let temp_dir =
            std::env::temp_dir().join(format!("loongclaw-feishu-support-{}", unix_ts_now()));
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");
        let config_path = temp_dir.join("loongclaw.toml");
        let mut config = mvp::config::LoongClawConfig::default();
        config.feishu.enabled = false;
        config.feishu.account_id = Some("feishu_main".to_owned());
        config.feishu.app_id = Some(loongclaw_contracts::SecretRef::Inline(
            "cli_a1b2c3".to_owned(),
        ));
        config.feishu.app_secret = Some(loongclaw_contracts::SecretRef::Inline(
            "app-secret".to_owned(),
        ));
        config.feishu_integration.sqlite_path =
            temp_dir.join("feishu.sqlite3").display().to_string();
        mvp::config::write(config_path.to_str(), &config, true).expect("write config");

        let context = load_feishu_daemon_context(config_path.to_str(), Some("feishu_main"))
            .expect("load feishu daemon context");

        assert_eq!(context.account_id(), "feishu_main");
    }

    #[test]
    fn resolve_selected_grant_suggests_auth_list_when_multiple_grants_exist() {
        let temp_dir =
            std::env::temp_dir().join(format!("loongclaw-feishu-support-multi-{}", unix_ts_now()));
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");
        let store = mvp::feishu::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
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
    fn resolve_selected_grant_prefers_persisted_selected_open_id() {
        let temp_dir = std::env::temp_dir().join(format!(
            "loongclaw-feishu-support-selected-{}",
            unix_ts_now()
        ));
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");
        let store = mvp::feishu::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
        let now_s = unix_ts_now();
        store
            .save_grant(&sample_grant("feishu_main", "ou_123", now_s))
            .expect("save first grant");
        store
            .save_grant(&sample_grant("feishu_main", "ou_456", now_s + 1))
            .expect("save second grant");
        store
            .set_selected_grant("feishu_main", "ou_123", now_s + 2)
            .expect("persist selected grant");

        let selected = resolve_selected_grant(&store, "feishu_main", None)
            .expect("resolve selected grant")
            .expect("selected grant should exist");

        assert_eq!(selected.principal.open_id, "ou_123");
    }

    #[test]
    fn resolve_selected_grant_clears_stale_selected_open_id_and_returns_single_grant() {
        let temp_dir = std::env::temp_dir().join(format!(
            "loongclaw-feishu-support-stale-selected-single-{}",
            unix_ts_now()
        ));
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");
        let store = mvp::feishu::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
        let now_s = unix_ts_now();
        store
            .save_grant(&sample_grant("feishu_main", "ou_123", now_s))
            .expect("save grant");
        store
            .set_selected_grant("feishu_main", "ou_missing", now_s + 1)
            .expect("persist stale selected grant");

        let selected = resolve_selected_grant(&store, "feishu_main", None)
            .expect("resolve selected grant")
            .expect("single grant should be returned");

        assert_eq!(selected.principal.open_id, "ou_123");
        assert_eq!(
            store
                .load_selected_grant("feishu_main")
                .expect("load selected grant after cleanup"),
            None
        );
    }

    #[test]
    fn resolve_selected_grant_clears_stale_selected_open_id_before_multi_grant_error() {
        let temp_dir = std::env::temp_dir().join(format!(
            "loongclaw-feishu-support-stale-selected-multi-{}",
            unix_ts_now()
        ));
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");
        let store = mvp::feishu::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
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

        let error = resolve_selected_grant(&store, "feishu_main", None)
            .expect_err("multiple grants should still require explicit selection");

        assert!(error.contains("loongclaw feishu auth list"));
        assert_eq!(
            store
                .load_selected_grant("feishu_main")
                .expect("load selected grant after cleanup"),
            None
        );
    }

    #[test]
    fn resolve_selected_grant_reports_missing_explicit_open_id() {
        let temp_dir = std::env::temp_dir().join(format!(
            "loongclaw-feishu-support-missing-open-id-{}",
            unix_ts_now()
        ));
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");
        let store = mvp::feishu::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
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
    fn resolve_scopes_can_append_recommended_message_write_scopes() {
        let scopes = resolve_scopes(
            &[
                "offline_access".to_owned(),
                "docx:document:readonly".to_owned(),
            ],
            &[],
            &[],
            true,
        );

        assert!(scopes.iter().any(|scope| scope == "offline_access"));
        assert!(scopes.iter().any(|scope| scope == "im:message"));
        assert!(scopes.iter().any(|scope| scope == "im:message:send_as_bot"));
        assert_eq!(
            scopes
                .iter()
                .filter(|scope| scope.as_str() == "im:message")
                .count(),
            1
        );
    }

    #[test]
    fn resolve_scopes_can_append_doc_write_scope_capability() {
        let scopes = resolve_scopes(
            &[
                "offline_access".to_owned(),
                "docx:document:readonly".to_owned(),
            ],
            &[],
            &[FeishuAuthCapability::DocWrite],
            false,
        );

        assert!(scopes.iter().any(|scope| scope == "offline_access"));
        assert!(scopes.iter().any(|scope| scope == "docx:document:readonly"));
        assert!(scopes.iter().any(|scope| scope == "docx:document"));
    }

    #[test]
    fn resolve_scopes_can_expand_all_capability_bundle_over_custom_scopes() {
        let scopes = resolve_scopes(
            &[
                "offline_access".to_owned(),
                "docx:document:readonly".to_owned(),
                "im:message:readonly".to_owned(),
                "search:message".to_owned(),
                "calendar:calendar:readonly".to_owned(),
            ],
            &["offline_access".to_owned()],
            &[FeishuAuthCapability::All],
            false,
        );

        assert!(scopes.iter().any(|scope| scope == "offline_access"));
        assert!(scopes.iter().any(|scope| scope == "docx:document:readonly"));
        assert!(scopes.iter().any(|scope| scope == "docx:document"));
        assert!(scopes.iter().any(|scope| scope == "im:message:readonly"));
        assert!(scopes.iter().any(|scope| scope == "search:message"));
        assert!(
            scopes
                .iter()
                .any(|scope| scope == "calendar:calendar:readonly")
        );
        assert!(scopes.iter().any(|scope| scope == "im:message"));
        assert!(scopes.iter().any(|scope| scope == "im:message:send_as_bot"));
        assert_eq!(
            scopes
                .iter()
                .filter(|scope| scope.as_str() == "offline_access")
                .count(),
            1
        );
    }

    #[test]
    fn recommended_auth_start_command_for_missing_grant_omits_message_write_capability() {
        let command =
            recommended_auth_start_command_for_grant("feishu_main", None, unix_ts_now(), &[]);

        assert_eq!(
            command.as_deref(),
            Some("loongclaw feishu auth start --account feishu_main")
        );
    }

    #[test]
    fn build_grant_recommendations_marks_missing_write_scope_for_existing_grant() {
        let now_s = unix_ts_now();
        let grant = sample_grant("feishu_main", "ou_123", now_s);

        let recommendations = build_grant_recommendations("feishu_main", Some(&grant), now_s, &[]);

        assert!(recommendations.missing_doc_write_scope);
        assert!(recommendations.missing_message_write_scope);
        assert_eq!(
            recommendations.auth_start_command.as_deref(),
            Some(
                "loongclaw feishu auth start --account feishu_main --capability doc-write --capability message-write"
            )
        );
    }

    #[test]
    fn oauth_helpers_emit_urlsafe_unpadded_tokens() {
        let state = generate_oauth_state();
        let (verifier, challenge) = build_pkce_pair();

        for value in [&state, &verifier, &challenge] {
            assert!(!value.is_empty());
            assert!(!value.contains('='));
            assert!(
                value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            );
        }
    }
}
