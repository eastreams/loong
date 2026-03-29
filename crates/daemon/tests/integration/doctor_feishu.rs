use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_doctor_feishu_dir(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "loongclaw-doctor-feishu-{label}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ))
}

fn sample_feishu_config(dir: &std::path::Path) -> mvp::config::LoongClawConfig {
    let mut config = mvp::config::LoongClawConfig::default();
    config.feishu.enabled = true;
    config.feishu.account_id = Some("feishu_main".to_owned());
    config.feishu.app_id = Some(loongclaw_contracts::SecretRef::Inline(
        "cli_a1b2c3".to_owned(),
    ));
    config.feishu.app_secret = Some(loongclaw_contracts::SecretRef::Inline(
        "app-secret".to_owned(),
    ));
    config.feishu_integration.sqlite_path = dir.join("feishu.sqlite3").display().to_string();
    config
}

fn sample_grant(account_id: &str, now_s: i64) -> mvp::channel::feishu::api::FeishuGrant {
    mvp::channel::feishu::api::FeishuGrant {
        principal: mvp::channel::feishu::api::FeishuUserPrincipal {
            account_id: account_id.to_owned(),
            open_id: "ou_123".to_owned(),
            union_id: Some("on_456".to_owned()),
            user_id: Some("u_789".to_owned()),
            name: Some("Alice".to_owned()),
            tenant_key: Some("tenant_x".to_owned()),
            avatar_url: None,
            email: Some("alice@example.com".to_owned()),
            enterprise_email: None,
        },
        access_token: "u-token".to_owned(),
        refresh_token: "r-token".to_owned(),
        scopes: mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
            "offline_access",
            "docx:document:readonly",
            "im:message:readonly",
            "im:message.group_msg:readonly",
            "search:message",
            "calendar:calendar:readonly",
        ]),
        access_expires_at_s: now_s + 3600,
        refresh_expires_at_s: now_s + 86400,
        refreshed_at_s: now_s,
    }
}

#[test]
fn doctor_reports_missing_feishu_grant_when_channel_is_enabled() {
    let temp_dir = temp_doctor_feishu_dir("missing-grant");
    let config = sample_feishu_config(&temp_dir);
    let mut fixes = Vec::new();

    let checks = loongclaw_daemon::doctor_cli::check_feishu_integration(&config, false, &mut fixes);

    let grant_check = checks
        .iter()
        .find(|check| check.name.contains("feishu user grant"))
        .expect("grant check should exist");
    assert_eq!(
        grant_check.level,
        loongclaw_daemon::doctor_cli::DoctorCheckLevel::Warn
    );
    assert!(grant_check.detail.contains("missing stored user grant"));
    assert!(
        grant_check
            .detail
            .contains("loong feishu auth start --account feishu_main")
    );
}

#[test]
fn doctor_reports_feishu_grant_freshness_when_valid_grant_exists() {
    let temp_dir = temp_doctor_feishu_dir("valid-grant");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let config = sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    store
        .save_grant(&sample_grant("feishu_main", now_s))
        .expect("seed feishu grant");
    let mut fixes = Vec::new();

    let checks = loongclaw_daemon::doctor_cli::check_feishu_integration(&config, false, &mut fixes);

    let freshness_check = checks
        .iter()
        .find(|check| check.name.contains("feishu token freshness"))
        .expect("token freshness check should exist");
    assert_eq!(
        freshness_check.level,
        loongclaw_daemon::doctor_cli::DoctorCheckLevel::Pass
    );

    let scope_check = checks
        .iter()
        .find(|check| check.name.contains("feishu scope coverage"))
        .expect("scope coverage check should exist");
    assert_eq!(
        scope_check.level,
        loongclaw_daemon::doctor_cli::DoctorCheckLevel::Pass
    );
}

#[test]
fn doctor_warns_when_feishu_grant_lacks_doc_write_scope() {
    let temp_dir = temp_doctor_feishu_dir("doc-write-scope-missing");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let config = sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let grant = sample_grant("feishu_main", now_s);
    store.save_grant(&grant).expect("seed feishu grant");
    let mut fixes = Vec::new();

    let checks = loongclaw_daemon::doctor_cli::check_feishu_integration(&config, false, &mut fixes);

    let doc_write_check = checks
        .iter()
        .find(|check| check.name.contains("feishu doc write readiness"))
        .expect("doc write readiness check should exist");
    assert_eq!(
        doc_write_check.level,
        loongclaw_daemon::doctor_cli::DoctorCheckLevel::Warn
    );
    assert!(doc_write_check.detail.contains("doc_write_ready=false"));
    assert!(doc_write_check.detail.contains("docx:document"));
    assert!(
        doc_write_check
            .detail
            .contains("loong feishu auth start --account feishu_main --capability doc-write")
    );
}

#[test]
fn doctor_passes_when_feishu_grant_has_doc_write_scope_without_rerun_hint() {
    let temp_dir = temp_doctor_feishu_dir("doc-write-scope-ready");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let config = sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let mut grant = sample_grant("feishu_main", now_s);
    grant.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
        "offline_access",
        "docx:document:readonly",
        "docx:document",
        "im:message:readonly",
    ]);
    store.save_grant(&grant).expect("seed feishu grant");
    let mut fixes = Vec::new();

    let checks = loongclaw_daemon::doctor_cli::check_feishu_integration(&config, false, &mut fixes);

    let doc_write_check = checks
        .iter()
        .find(|check| check.name.contains("feishu doc write readiness"))
        .expect("doc write readiness check should exist");
    assert_eq!(
        doc_write_check.level,
        loongclaw_daemon::doctor_cli::DoctorCheckLevel::Pass
    );
    assert!(doc_write_check.detail.contains("doc_write_ready=true"));
    assert!(!doc_write_check.detail.contains("rerun `"));
}

#[test]
fn doctor_warns_when_feishu_grant_lacks_message_write_scope() {
    let temp_dir = temp_doctor_feishu_dir("write-scope-missing");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let config = sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let mut grant = sample_grant("feishu_main", now_s);
    grant.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
        "offline_access",
        "docx:document:readonly",
        "im:message:readonly",
    ]);
    store.save_grant(&grant).expect("seed feishu grant");
    let mut fixes = Vec::new();

    let checks = loongclaw_daemon::doctor_cli::check_feishu_integration(&config, false, &mut fixes);

    let write_check = checks
        .iter()
        .find(|check| check.name.contains("feishu message write readiness"))
        .expect("message write readiness check should exist");
    assert_eq!(
        write_check.level,
        loongclaw_daemon::doctor_cli::DoctorCheckLevel::Warn
    );
    assert!(write_check.detail.contains("write_ready=false"));
    assert!(write_check.detail.contains("im:message:send_as_bot"));
    assert!(
        write_check
            .detail
            .contains("loong feishu auth start --account feishu_main --capability message-write")
    );
}

#[test]
fn doctor_passes_when_feishu_grant_has_message_write_scope_without_rerun_hint() {
    let temp_dir = temp_doctor_feishu_dir("write-scope-ready");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let config = sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let mut grant = sample_grant("feishu_main", now_s);
    grant.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
        "offline_access",
        "docx:document:readonly",
        "im:message:readonly",
        "im:message:send_as_bot",
    ]);
    store.save_grant(&grant).expect("seed feishu grant");
    let mut fixes = Vec::new();

    let checks = loongclaw_daemon::doctor_cli::check_feishu_integration(&config, false, &mut fixes);

    let write_check = checks
        .iter()
        .find(|check| check.name.contains("feishu message write readiness"))
        .expect("message write readiness check should exist");
    assert_eq!(
        write_check.level,
        loongclaw_daemon::doctor_cli::DoctorCheckLevel::Pass
    );
    assert!(write_check.detail.contains("write_ready=true"));
    assert!(!write_check.detail.contains("rerun `"));
}

#[test]
fn doctor_warns_when_multiple_feishu_grants_exist_without_selected_default() {
    let temp_dir = temp_doctor_feishu_dir("multi-grant-no-selection");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let config = sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    store
        .save_grant(&sample_grant("feishu_main", now_s))
        .expect("seed first feishu grant");
    let mut second = sample_grant("feishu_main", now_s + 1);
    second.principal.open_id = "ou_456".to_owned();
    second.principal.name = Some("Bob".to_owned());
    store.save_grant(&second).expect("seed second feishu grant");
    let mut fixes = Vec::new();

    let checks = loongclaw_daemon::doctor_cli::check_feishu_integration(&config, false, &mut fixes);

    let selection_check = checks
        .iter()
        .find(|check| check.name.contains("feishu selected grant"))
        .expect("selected grant check should exist");
    assert_eq!(
        selection_check.level,
        loongclaw_daemon::doctor_cli::DoctorCheckLevel::Warn
    );
    assert!(selection_check.detail.contains("multiple stored grants"));
    assert!(
        selection_check
            .detail
            .contains("loong feishu auth select --account feishu_main")
    );
}

#[test]
fn doctor_reports_selected_feishu_grant_when_default_exists() {
    let temp_dir = temp_doctor_feishu_dir("multi-grant-selected");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let config = sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    store
        .save_grant(&sample_grant("feishu_main", now_s))
        .expect("seed first feishu grant");
    let mut second = sample_grant("feishu_main", now_s + 1);
    second.principal.open_id = "ou_456".to_owned();
    second.principal.name = Some("Bob".to_owned());
    store.save_grant(&second).expect("seed second feishu grant");
    store
        .set_selected_grant("feishu_main", "ou_456", now_s + 2)
        .expect("persist selected grant");
    let mut fixes = Vec::new();

    let checks = loongclaw_daemon::doctor_cli::check_feishu_integration(&config, false, &mut fixes);

    let selection_check = checks
        .iter()
        .find(|check| check.name.contains("feishu selected grant"))
        .expect("selected grant check should exist");
    assert_eq!(
        selection_check.level,
        loongclaw_daemon::doctor_cli::DoctorCheckLevel::Pass
    );
    assert!(selection_check.detail.contains("selected_open_id=ou_456"));
}

#[test]
fn doctor_uses_effective_selected_grant_for_freshness_and_scope_checks() {
    let temp_dir = temp_doctor_feishu_dir("selected-grant-health");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let config = sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));

    let mut selected = sample_grant("feishu_main", now_s);
    selected.principal.open_id = "ou_selected".to_owned();
    selected.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
        "offline_access",
        "im:message:readonly",
    ]);
    selected.access_expires_at_s = now_s - 60;
    selected.refresh_expires_at_s = now_s + 3600;
    selected.refreshed_at_s = now_s;
    store.save_grant(&selected).expect("seed selected grant");

    let mut latest = sample_grant("feishu_main", now_s + 100);
    latest.principal.open_id = "ou_latest".to_owned();
    latest.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
        "offline_access",
        "docx:document:readonly",
        "im:message",
        "search:message",
        "calendar:calendar:readonly",
    ]);
    latest.access_expires_at_s = now_s + 7200;
    latest.refresh_expires_at_s = now_s + 86_400;
    latest.refreshed_at_s = now_s + 100;
    store.save_grant(&latest).expect("seed latest grant");
    store
        .set_selected_grant("feishu_main", "ou_selected", now_s + 200)
        .expect("persist selected grant");

    let mut fixes = Vec::new();
    let checks = loongclaw_daemon::doctor_cli::check_feishu_integration(&config, false, &mut fixes);

    let freshness_check = checks
        .iter()
        .find(|check| check.name.contains("feishu token freshness"))
        .expect("token freshness check should exist");
    assert_eq!(
        freshness_check.level,
        loongclaw_daemon::doctor_cli::DoctorCheckLevel::Warn
    );
    assert!(
        freshness_check
            .detail
            .contains("effective_open_id=ou_selected")
    );
    assert!(freshness_check.detail.contains("access_expired=true"));

    let scope_check = checks
        .iter()
        .find(|check| check.name.contains("feishu scope coverage"))
        .expect("scope coverage check should exist");
    assert_eq!(
        scope_check.level,
        loongclaw_daemon::doctor_cli::DoctorCheckLevel::Warn
    );
    assert!(scope_check.detail.contains("effective_open_id=ou_selected"));
    assert!(
        scope_check
            .detail
            .contains("missing_scopes=docx:document:readonly")
    );
}

#[test]
fn doctor_warns_when_selected_open_id_is_stale_but_single_grant_routes_implicitly() {
    let temp_dir = temp_doctor_feishu_dir("stale-selected-single-grant");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let config = sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    store
        .save_grant(&sample_grant("feishu_main", now_s))
        .expect("seed grant");
    store
        .set_selected_grant("feishu_main", "ou_missing", now_s + 1)
        .expect("persist stale selected grant");
    let mut fixes = Vec::new();

    let checks = loongclaw_daemon::doctor_cli::check_feishu_integration(&config, false, &mut fixes);

    let selection_check = checks
        .iter()
        .find(|check| check.name.contains("feishu selected grant"))
        .expect("selected grant check should exist");
    assert_eq!(
        selection_check.level,
        loongclaw_daemon::doctor_cli::DoctorCheckLevel::Warn
    );
    assert!(
        selection_check
            .detail
            .contains("stale selected_open_id=ou_missing was cleared")
    );
    assert!(selection_check.detail.contains("now routes implicitly"));
}

#[test]
fn doctor_warns_when_effective_grant_is_ambiguous_without_selected_default() {
    let temp_dir = temp_doctor_feishu_dir("ambiguous-effective-grant");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let config = sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    store
        .save_grant(&sample_grant("feishu_main", now_s))
        .expect("seed first grant");
    let mut second = sample_grant("feishu_main", now_s + 1);
    second.principal.open_id = "ou_456".to_owned();
    second.principal.name = Some("Bob".to_owned());
    store.save_grant(&second).expect("seed second grant");
    let mut fixes = Vec::new();

    let checks = loongclaw_daemon::doctor_cli::check_feishu_integration(&config, false, &mut fixes);

    let freshness_check = checks
        .iter()
        .find(|check| check.name.contains("feishu token freshness"))
        .expect("token freshness check should exist");
    assert_eq!(
        freshness_check.level,
        loongclaw_daemon::doctor_cli::DoctorCheckLevel::Warn
    );
    assert!(
        freshness_check
            .detail
            .contains("cannot determine effective token freshness")
    );
    assert!(
        freshness_check
            .detail
            .contains("loong feishu auth select --account feishu_main --open-id <open_id>")
    );

    let write_check = checks
        .iter()
        .find(|check| check.name.contains("feishu message write readiness"))
        .expect("message write readiness check should exist");
    assert_eq!(
        write_check.level,
        loongclaw_daemon::doctor_cli::DoctorCheckLevel::Warn
    );
    assert!(
        write_check
            .detail
            .contains("cannot determine active write readiness")
    );
}
