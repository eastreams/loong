use super::*;

#[test]
fn doctor_reports_missing_feishu_grant_when_channel_is_enabled() {
    let temp_dir = temp_doctor_feishu_dir("missing-grant");
    let config = sample_feishu_config(&temp_dir);
    let mut fixes = Vec::new();

    let checks = loong_daemon::doctor_cli::check_feishu_integration(&config, false, &mut fixes);

    let grant_check = checks
        .iter()
        .find(|check| check.name.contains("feishu user grant"))
        .expect("grant check should exist");
    assert_eq!(
        grant_check.level,
        loong_daemon::doctor_cli::DoctorCheckLevel::Warn
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
    let now_s = loong_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    store
        .save_grant(&sample_default_grant_covering_default_coarse_capabilities(
            "feishu_main",
            now_s,
        ))
        .expect("seed feishu grant");
    let mut fixes = Vec::new();

    let checks = loong_daemon::doctor_cli::check_feishu_integration(&config, false, &mut fixes);

    let freshness_check = checks
        .iter()
        .find(|check| check.name.contains("feishu token freshness"))
        .expect("token freshness check should exist");
    assert_eq!(
        freshness_check.level,
        loong_daemon::doctor_cli::DoctorCheckLevel::Pass
    );

    let scope_check = checks
        .iter()
        .find(|check| check.name.contains("feishu scope coverage"))
        .expect("scope coverage check should exist");
    assert_eq!(
        scope_check.level,
        loong_daemon::doctor_cli::DoctorCheckLevel::Pass
    );
}

#[test]
fn doctor_warns_when_feishu_grant_lacks_doc_write_scope() {
    let temp_dir = temp_doctor_feishu_dir("doc-write-scope-missing");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let config = sample_feishu_config_with_capabilities(
        &temp_dir,
        mvp::config::FeishuCapabilityConfig::default(),
    );
    let now_s = loong_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let grant = sample_default_grant("feishu_main", now_s);
    store.save_grant(&grant).expect("seed feishu grant");
    let mut fixes = Vec::new();

    let checks = loong_daemon::doctor_cli::check_feishu_integration(&config, false, &mut fixes);

    let doc_write_check = checks
        .iter()
        .find(|check| check.name.contains("feishu doc write readiness"))
        .expect("doc write readiness check should exist");
    assert_eq!(
        doc_write_check.level,
        loong_daemon::doctor_cli::DoctorCheckLevel::Warn
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
    let config = sample_feishu_config_with_capabilities(
        &temp_dir,
        mvp::config::FeishuCapabilityConfig::default(),
    );
    let now_s = loong_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let mut grant = sample_default_grant("feishu_main", now_s);
    grant.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
        "offline_access",
        "docx:document:readonly",
        "docx:document",
        "im:message:readonly",
    ]);
    store.save_grant(&grant).expect("seed feishu grant");
    let mut fixes = Vec::new();

    let checks = loong_daemon::doctor_cli::check_feishu_integration(&config, false, &mut fixes);

    let doc_write_check = checks
        .iter()
        .find(|check| check.name.contains("feishu doc write readiness"))
        .expect("doc write readiness check should exist");
    assert_eq!(
        doc_write_check.level,
        loong_daemon::doctor_cli::DoctorCheckLevel::Pass
    );
    assert!(doc_write_check.detail.contains("doc_write_ready=true"));
    assert!(!doc_write_check.detail.contains("rerun `"));
}

#[test]
fn doctor_warns_when_feishu_grant_lacks_message_write_scope() {
    let temp_dir = temp_doctor_feishu_dir("write-scope-missing");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let config = sample_feishu_config_with_capabilities(
        &temp_dir,
        mvp::config::FeishuCapabilityConfig::default(),
    );
    let now_s = loong_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let mut grant = sample_default_grant("feishu_main", now_s);
    grant.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
        "offline_access",
        "docx:document:readonly",
        "im:message:readonly",
    ]);
    store.save_grant(&grant).expect("seed feishu grant");
    let mut fixes = Vec::new();

    let checks = loong_daemon::doctor_cli::check_feishu_integration(&config, false, &mut fixes);

    let write_check = checks
        .iter()
        .find(|check| check.name.contains("feishu message write readiness"))
        .expect("message write readiness check should exist");
    assert_eq!(
        write_check.level,
        loong_daemon::doctor_cli::DoctorCheckLevel::Warn
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
fn doctor_skips_write_warnings_when_capabilities_disable_docs_and_messages() {
    let temp_dir = temp_doctor_feishu_dir("calendar-only-write-status");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let config = sample_feishu_config_with_capabilities(
        &temp_dir,
        mvp::config::FeishuCapabilityConfig {
            docs: false,
            messages: false,
            calendar: true,
            bitable: false,
        },
    );
    let now_s = loong_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let mut grant = sample_default_grant("feishu_main", now_s);
    grant.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
        "offline_access",
        "calendar:calendar:readonly",
    ]);
    store.save_grant(&grant).expect("seed feishu grant");
    let mut fixes = Vec::new();

    let checks = loong_daemon::doctor_cli::check_feishu_integration(&config, false, &mut fixes);

    let doc_write_check = checks
        .iter()
        .find(|check| check.name.contains("feishu doc write readiness"))
        .expect("doc write readiness check should exist");
    assert_eq!(
        doc_write_check.level,
        loong_daemon::doctor_cli::DoctorCheckLevel::Pass
    );
    assert!(
        doc_write_check
            .detail
            .contains("not required by current config")
    );
    assert!(!doc_write_check.detail.contains("rerun `"));

    let write_check = checks
        .iter()
        .find(|check| check.name.contains("feishu message write readiness"))
        .expect("message write readiness check should exist");
    assert_eq!(
        write_check.level,
        loong_daemon::doctor_cli::DoctorCheckLevel::Pass
    );
    assert!(
        write_check
            .detail
            .contains("not required by current config")
    );
    assert!(!write_check.detail.contains("rerun `"));
}

#[test]
fn doctor_warns_when_config_requires_bitable_scope_but_grant_lacks_it() {
    let temp_dir = temp_doctor_feishu_dir("bitable-scope-missing");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let config = sample_feishu_config_with_capabilities(
        &temp_dir,
        mvp::config::FeishuCapabilityConfig {
            docs: true,
            messages: true,
            calendar: true,
            bitable: true,
        },
    );
    let now_s = loong_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    store
        .save_grant(&sample_default_grant_covering_default_coarse_capabilities(
            "feishu_main",
            now_s,
        ))
        .expect("seed feishu grant");
    let mut fixes = Vec::new();

    let checks = loong_daemon::doctor_cli::check_feishu_integration(&config, false, &mut fixes);

    let scope_check = checks
        .iter()
        .find(|check| check.name.contains("feishu scope coverage"))
        .expect("scope coverage check should exist");
    assert_eq!(
        scope_check.level,
        loong_daemon::doctor_cli::DoctorCheckLevel::Warn
    );
    assert!(scope_check.detail.contains("bitable:app"));
    assert!(scope_check.detail.contains("base:table:read"));
    assert!(scope_check.detail.contains("base:record:create"));
    assert!(scope_check.detail.contains("base:record:retrieve"));
    assert!(scope_check.detail.contains("base:record:write"));
    assert!(scope_check.detail.contains("drive:drive:readonly"));
    assert!(
        scope_check
            .detail
            .contains("loong feishu auth start --account feishu_main")
    );
}

#[test]
fn doctor_ignores_legacy_bitable_default_scope_when_capability_block_is_explicit() {
    let temp_dir = temp_doctor_feishu_dir("explicit-default-capabilities");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let config = sample_feishu_config_with_capabilities_and_default_scopes(
        &temp_dir,
        mvp::config::FeishuCapabilityConfig::default(),
        vec![
            "offline_access".to_owned(),
            "docx:document:readonly".to_owned(),
            "im:message:readonly".to_owned(),
            "im:message.group_msg".to_owned(),
            "search:message".to_owned(),
            "calendar:calendar:readonly".to_owned(),
            "bitable:app".to_owned(),
        ],
    );
    let now_s = loong_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    store
        .save_grant(&sample_default_grant_covering_default_coarse_capabilities(
            "feishu_main",
            now_s,
        ))
        .expect("seed feishu grant");
    let mut fixes = Vec::new();

    let checks = loong_daemon::doctor_cli::check_feishu_integration(&config, false, &mut fixes);

    let scope_check = checks
        .iter()
        .find(|check| check.name.contains("feishu scope coverage"))
        .expect("scope coverage check should exist");
    assert_eq!(
        scope_check.level,
        loong_daemon::doctor_cli::DoctorCheckLevel::Pass
    );
    assert!(!scope_check.detail.contains("bitable:app"));
}

#[test]
fn doctor_passes_when_feishu_grant_has_message_write_scope_without_rerun_hint() {
    let temp_dir = temp_doctor_feishu_dir("write-scope-ready");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let config = sample_feishu_config(&temp_dir);
    let now_s = loong_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let mut grant = sample_default_grant("feishu_main", now_s);
    grant.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
        "offline_access",
        "docx:document:readonly",
        "im:message:readonly",
        "im:message:send_as_bot",
    ]);
    store.save_grant(&grant).expect("seed feishu grant");
    let mut fixes = Vec::new();

    let checks = loong_daemon::doctor_cli::check_feishu_integration(&config, false, &mut fixes);

    let write_check = checks
        .iter()
        .find(|check| check.name.contains("feishu message write readiness"))
        .expect("message write readiness check should exist");
    assert_eq!(
        write_check.level,
        loong_daemon::doctor_cli::DoctorCheckLevel::Pass
    );
    assert!(write_check.detail.contains("write_ready=true"));
    assert!(!write_check.detail.contains("rerun `"));
}

#[test]
fn doctor_warns_when_multiple_feishu_grants_exist_without_selected_default() {
    let temp_dir = temp_doctor_feishu_dir("multi-grant-no-selection");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let config = sample_feishu_config(&temp_dir);
    let now_s = loong_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    store
        .save_grant(&sample_default_grant("feishu_main", now_s))
        .expect("seed first feishu grant");
    let mut second = sample_default_grant("feishu_main", now_s + 1);
    second.principal.open_id = "ou_456".to_owned();
    second.principal.name = Some("Bob".to_owned());
    store.save_grant(&second).expect("seed second feishu grant");
    let mut fixes = Vec::new();

    let checks = loong_daemon::doctor_cli::check_feishu_integration(&config, false, &mut fixes);

    let selection_check = checks
        .iter()
        .find(|check| check.name.contains("feishu selected grant"))
        .expect("selected grant check should exist");
    assert_eq!(
        selection_check.level,
        loong_daemon::doctor_cli::DoctorCheckLevel::Warn
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
    let now_s = loong_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    store
        .save_grant(&sample_default_grant("feishu_main", now_s))
        .expect("seed first feishu grant");
    let mut second = sample_default_grant("feishu_main", now_s + 1);
    second.principal.open_id = "ou_456".to_owned();
    second.principal.name = Some("Bob".to_owned());
    store.save_grant(&second).expect("seed second feishu grant");
    store
        .set_selected_grant("feishu_main", "ou_456", now_s + 2)
        .expect("persist selected grant");
    let mut fixes = Vec::new();

    let checks = loong_daemon::doctor_cli::check_feishu_integration(&config, false, &mut fixes);

    let selection_check = checks
        .iter()
        .find(|check| check.name.contains("feishu selected grant"))
        .expect("selected grant check should exist");
    assert_eq!(
        selection_check.level,
        loong_daemon::doctor_cli::DoctorCheckLevel::Pass
    );
    assert!(selection_check.detail.contains("selected_open_id=ou_456"));
}

#[test]
fn doctor_uses_effective_selected_grant_for_freshness_and_scope_checks() {
    let temp_dir = temp_doctor_feishu_dir("selected-grant-health");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let config = sample_feishu_config(&temp_dir);
    let now_s = loong_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));

    let mut selected = sample_default_grant("feishu_main", now_s);
    selected.principal.open_id = "ou_selected".to_owned();
    selected.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
        "offline_access",
        "im:message:readonly",
    ]);
    selected.access_expires_at_s = now_s - 60;
    selected.refresh_expires_at_s = now_s + 3600;
    selected.refreshed_at_s = now_s;
    store.save_grant(&selected).expect("seed selected grant");

    let mut latest = sample_default_grant("feishu_main", now_s + 100);
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
    let checks = loong_daemon::doctor_cli::check_feishu_integration(&config, false, &mut fixes);

    let freshness_check = checks
        .iter()
        .find(|check| check.name.contains("feishu token freshness"))
        .expect("token freshness check should exist");
    assert_eq!(
        freshness_check.level,
        loong_daemon::doctor_cli::DoctorCheckLevel::Warn
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
        loong_daemon::doctor_cli::DoctorCheckLevel::Warn
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
    let now_s = loong_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    store
        .save_grant(&sample_default_grant("feishu_main", now_s))
        .expect("seed grant");
    store
        .set_selected_grant("feishu_main", "ou_missing", now_s + 1)
        .expect("persist stale selected grant");
    let mut fixes = Vec::new();

    let checks = loong_daemon::doctor_cli::check_feishu_integration(&config, false, &mut fixes);

    let selection_check = checks
        .iter()
        .find(|check| check.name.contains("feishu selected grant"))
        .expect("selected grant check should exist");
    assert_eq!(
        selection_check.level,
        loong_daemon::doctor_cli::DoctorCheckLevel::Warn
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
    let now_s = loong_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    store
        .save_grant(&sample_default_grant("feishu_main", now_s))
        .expect("seed first grant");
    let mut second = sample_default_grant("feishu_main", now_s + 1);
    second.principal.open_id = "ou_456".to_owned();
    second.principal.name = Some("Bob".to_owned());
    store.save_grant(&second).expect("seed second grant");
    let mut fixes = Vec::new();

    let checks = loong_daemon::doctor_cli::check_feishu_integration(&config, false, &mut fixes);

    let freshness_check = checks
        .iter()
        .find(|check| check.name.contains("feishu token freshness"))
        .expect("token freshness check should exist");
    assert_eq!(
        freshness_check.level,
        loong_daemon::doctor_cli::DoctorCheckLevel::Warn
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
        loong_daemon::doctor_cli::DoctorCheckLevel::Warn
    );
    assert!(
        write_check
            .detail
            .contains("cannot determine active write readiness")
    );
}
