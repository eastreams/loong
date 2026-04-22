use super::*;

fn temp_feishu_test_dir(prefix: &str, label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "{prefix}-{label}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ))
}

pub(crate) fn temp_feishu_cli_dir(label: &str) -> PathBuf {
    temp_feishu_test_dir("loong-feishu-cli", label)
}

pub(crate) fn temp_doctor_feishu_dir(label: &str) -> PathBuf {
    temp_feishu_test_dir("loong-doctor-feishu", label)
}

fn base_feishu_test_config(dir: &Path) -> mvp::config::LoongConfig {
    let mut config = mvp::config::LoongConfig::default();
    config.feishu.enabled = true;
    config.feishu.account_id = Some("feishu_main".to_owned());
    config.feishu.app_id = Some(loong_contracts::SecretRef::Inline("cli_a1b2c3".to_owned()));
    config.feishu.app_secret = Some(loong_contracts::SecretRef::Inline("app-secret".to_owned()));
    config.feishu_integration.sqlite_path = dir.join("feishu.sqlite3").display().to_string();
    config
}

pub(crate) fn sample_feishu_config(dir: &Path) -> mvp::config::LoongConfig {
    let mut config = base_feishu_test_config(dir);
    config.feishu_integration.capabilities_explicitly_configured = false;
    config
}

pub(crate) fn sample_feishu_config_with_capabilities(
    dir: &Path,
    capabilities: mvp::config::FeishuCapabilityConfig,
) -> mvp::config::LoongConfig {
    let mut config = base_feishu_test_config(dir);
    config.feishu_integration.capabilities = capabilities;
    config.feishu_integration.capabilities_explicitly_configured = true;
    config
}

pub(crate) fn sample_feishu_config_with_capabilities_and_default_scopes(
    dir: &Path,
    capabilities: mvp::config::FeishuCapabilityConfig,
    default_scopes: Vec<String>,
) -> mvp::config::LoongConfig {
    let mut config = sample_feishu_config_with_capabilities(dir, capabilities);
    config.feishu_integration.default_scopes = default_scopes;
    config
}

pub(crate) fn write_feishu_test_config(dir: &Path, config: &mvp::config::LoongConfig) -> PathBuf {
    fs::create_dir_all(dir).expect("create temp feishu config dir");
    let config_path = dir.join("loong.toml");
    mvp::config::write(config_path.to_str(), config, true).expect("write sample feishu config");
    config_path
}

pub(crate) fn write_sample_feishu_config(dir: &Path) -> PathBuf {
    let config = sample_feishu_config(dir);
    write_feishu_test_config(dir, &config)
}

pub(crate) fn write_sample_feishu_config_with_capabilities(
    dir: &Path,
    capabilities: Option<mvp::config::FeishuCapabilityConfig>,
) -> PathBuf {
    let config = match capabilities {
        Some(capabilities) => sample_feishu_config_with_capabilities(dir, capabilities),
        None => sample_feishu_config(dir),
    };
    write_feishu_test_config(dir, &config)
}

pub(crate) fn write_sample_feishu_config_with_capabilities_and_default_scopes(
    dir: &Path,
    capabilities: mvp::config::FeishuCapabilityConfig,
    default_scopes: Vec<String>,
) -> PathBuf {
    let config = sample_feishu_config_with_capabilities_and_default_scopes(
        dir,
        capabilities,
        default_scopes,
    );
    write_feishu_test_config(dir, &config)
}

pub(crate) fn sample_grant(
    account_id: &str,
    open_id: &str,
    access_token: &str,
    refresh_token: &str,
    now_s: i64,
) -> mvp::channel::feishu::api::FeishuGrant {
    mvp::channel::feishu::api::FeishuGrant {
        principal: mvp::channel::feishu::api::FeishuUserPrincipal {
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
        access_token: access_token.to_owned(),
        refresh_token: refresh_token.to_owned(),
        scopes: mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
            "offline_access",
            "docx:document:readonly",
            "im:message:readonly",
            "im:message.group_msg",
            "search:message",
            "calendar:calendar:readonly",
        ]),
        access_expires_at_s: now_s + 3600,
        refresh_expires_at_s: now_s + 86_400,
        refreshed_at_s: now_s,
    }
}

pub(crate) fn sample_default_grant(
    account_id: &str,
    now_s: i64,
) -> mvp::channel::feishu::api::FeishuGrant {
    sample_grant(account_id, "ou_123", "u-token", "r-token", now_s)
}

pub(crate) fn sample_grant_covering_default_coarse_capabilities(
    account_id: &str,
    open_id: &str,
    access_token: &str,
    refresh_token: &str,
    now_s: i64,
) -> mvp::channel::feishu::api::FeishuGrant {
    let mut grant = sample_grant(account_id, open_id, access_token, refresh_token, now_s);
    let config = mvp::config::FeishuIntegrationConfig {
        capabilities: mvp::config::FeishuCapabilityConfig {
            docs: true,
            messages: true,
            calendar: true,
            bitable: false,
        },
        capabilities_explicitly_configured: true,
        ..mvp::config::FeishuIntegrationConfig::default()
    };
    grant.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes(
        loong_daemon::feishu_support::scopes_for_configured_capabilities(
            &loong_daemon::feishu_support::configured_capabilities_from_config(&config),
        ),
    );
    grant
}

pub(crate) fn sample_default_grant_covering_default_coarse_capabilities(
    account_id: &str,
    now_s: i64,
) -> mvp::channel::feishu::api::FeishuGrant {
    sample_grant_covering_default_coarse_capabilities(
        account_id, "ou_123", "u-token", "r-token", now_s,
    )
}
