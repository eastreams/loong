use super::*;
use clap::CommandFactory;
use std::sync::{Mutex, MutexGuard};

static DAEMON_TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

fn lock_daemon_test_environment() -> MutexGuard<'static, ()> {
    DAEMON_TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

fn catalog_entry(raw: &str) -> mvp::channel::ChannelCatalogEntry {
    mvp::channel::resolve_channel_catalog_entry(raw).expect("channel catalog entry")
}

fn catalog_command_family(raw: &str) -> mvp::channel::ChannelCatalogCommandFamilyDescriptor {
    mvp::channel::resolve_channel_catalog_command_family_descriptor(raw)
        .expect("channel catalog command family")
}

fn channel_send_command(raw: &str) -> &'static str {
    catalog_command_family(raw).send.command
}

fn channel_serve_command(raw: &str) -> &'static str {
    catalog_command_family(raw).serve.command
}

fn channel_capability_ids(raw: &str) -> Vec<&'static str> {
    catalog_entry(raw)
        .capabilities
        .into_iter()
        .map(|capability| capability.as_str())
        .collect()
}

fn channel_supported_target_kinds(raw: &str) -> Vec<&'static str> {
    catalog_entry(raw)
        .supported_target_kinds
        .into_iter()
        .map(|kind| kind.as_str())
        .collect()
}

fn approval_test_operation(tool_name: &str, payload: Value) -> OperationSpec {
    OperationSpec::ToolCore {
        tool_name: tool_name.to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool]),
        payload,
        core: None,
    }
}

fn write_temp_risk_profile(path: &Path, body: &str) {
    fs::create_dir_all(
        path.parent()
            .expect("temp risk profile path should have parent directory"),
    )
    .expect("create temp risk profile directory");
    fs::write(path, body).expect("write temp risk profile");
}

fn sign_security_scan_profile_for_test(profile: &SecurityScanProfile) -> (String, String) {
    use ed25519_dalek::{Signer, SigningKey};

    let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
    let signature = signing_key.sign(&security_scan_profile_message(profile));
    let public_key_base64 = BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes());
    let signature_base64 = BASE64_STANDARD.encode(signature.to_bytes());
    (public_key_base64, signature_base64)
}

mod acp;
mod architecture;
mod doctor_feishu;
mod feishu_cli;
mod import_claw_cli;
mod import_cli;
mod migration;
mod onboard_cli;
mod programmatic;
mod spec_runtime;
mod spec_runtime_bridge;

#[test]
fn cli_uses_loongclaw_program_name() {
    let command = Cli::command();
    assert_eq!(command.get_name(), "loongclaw");
}

#[test]
fn cli_import_help_explains_explicit_power_user_flow() {
    let mut command = Cli::command();
    let import = command
        .find_subcommand_mut("import")
        .expect("import subcommand should exist");
    let mut help = Vec::new();
    import
        .write_long_help(&mut help)
        .expect("render import help");
    let help = String::from_utf8(help).expect("help should be utf8");

    assert!(
        help.contains("Power-user import flow"),
        "import help should explain when to use the explicit import command: {help}"
    );
    assert!(
        help.contains("--source-path"),
        "import help should surface the path-level disambiguation flag: {help}"
    );
    assert!(
        help.contains("loongclaw onboard"),
        "import help should direct guided users back to onboard: {help}"
    );
    assert!(
        help.contains(&format!(
            "--provider <{}>",
            mvp::config::PROVIDER_SELECTOR_PLACEHOLDER
        )),
        "import help should expose the shared provider selector placeholder: {help}"
    );
    assert!(
        help.contains(mvp::config::PROVIDER_SELECTOR_HUMAN_SUMMARY),
        "import help should reuse the shared provider selector summary: {help}"
    );
}

#[test]
fn cli_onboard_help_mentions_detected_reusable_settings() {
    let mut command = Cli::command();
    let onboard = command
        .find_subcommand_mut("onboard")
        .expect("onboard subcommand should exist");
    let mut help = Vec::new();
    onboard
        .write_long_help(&mut help)
        .expect("render onboard help");
    let help = String::from_utf8(help).expect("help should be utf8");

    assert!(
        help.contains("detect"),
        "onboard help should mention that it detects reusable settings: {help}"
    );
    assert!(
        help.contains("provider, channels, or workspace guidance"),
        "onboard help should explain the kinds of detected settings it can reuse: {help}"
    );
    assert!(
        help.contains(&format!(
            "--provider <{}>",
            mvp::config::PROVIDER_SELECTOR_PLACEHOLDER
        )),
        "onboard help should expose the shared provider selector placeholder: {help}"
    );
    assert!(
        help.contains(mvp::config::PROVIDER_SELECTOR_HUMAN_SUMMARY),
        "onboard help should reuse the shared provider selector summary: {help}"
    );
}

#[test]
fn resolve_validate_output_defaults_to_text() {
    let resolved = resolve_validate_output(false, None).expect("resolve default output");
    assert_eq!(resolved, ValidateConfigOutput::Text);
}

#[test]
fn resolve_validate_output_uses_json_flag_legacy_alias() {
    let resolved = resolve_validate_output(true, None).expect("resolve json output");
    assert_eq!(resolved, ValidateConfigOutput::Json);
}

#[test]
fn resolve_validate_output_accepts_explicit_problem_json() {
    let resolved = resolve_validate_output(false, Some(ValidateConfigOutput::ProblemJson))
        .expect("resolve problem-json output");
    assert_eq!(resolved, ValidateConfigOutput::ProblemJson);
}

#[test]
fn resolve_validate_output_rejects_conflicting_json_and_output_flags() {
    let error = resolve_validate_output(true, Some(ValidateConfigOutput::Json))
        .expect_err("conflicting flags should fail");
    assert!(error.contains("conflicts"));
}

#[test]
fn render_channel_surfaces_text_reports_aliases_and_operation_health() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.telegram.enabled = true;
    config.telegram.bot_token = Some("123456:telegram-token".to_owned());
    config.telegram.allowed_chat_ids = vec![1001];
    config.feishu.enabled = true;
    config.feishu.app_id = Some("cli_a1b2c3".to_owned());
    config.feishu.app_secret = Some("app-secret".to_owned());

    let inventory = mvp::channel::channel_inventory(&config);
    let rendered = render_channel_surfaces_text("/tmp/loongclaw.toml", &inventory);

    assert!(rendered.contains("config=/tmp/loongclaw.toml"));
    assert!(rendered.contains("Telegram [telegram]"));
    assert!(
        rendered.contains("capabilities=runtime_backed,multi_account,send,serve,runtime_tracking")
    );
    assert!(rendered.contains(&format!(
        "op send ({}) ready: ready target_kinds=conversation requirements=enabled,bot_token",
        channel_send_command("telegram")
    )));
    assert!(rendered.contains("Feishu/Lark [feishu]"));
    assert!(rendered.contains("implementation_status=runtime_backed"));
    assert!(
        rendered.contains("capabilities=runtime_backed,multi_account,send,serve,runtime_tracking")
    );
    assert!(rendered.contains(
        "onboarding strategy=manual_config status_command=\"loongclaw doctor\" repair_command=\"loongclaw doctor --fix\""
    ));
    assert!(rendered.contains("setup_hint=\"configure telegram bot credentials"));
    assert!(rendered.contains("target_kinds=receive_id,message_reply"));
    assert!(rendered.contains("configured_accounts=1"));
    assert!(rendered.contains("aliases=lark"));
    assert!(rendered.contains("account=feishu:cli_a1b2c3"));
    assert!(rendered.contains(&format!(
        "op send ({}) ready: ready target_kinds=receive_id,message_reply requirements=enabled,app_id,app_secret",
        channel_send_command("feishu")
    )));
    assert!(rendered.contains(&format!(
        "op serve ({}) misconfigured: allowed_chat_ids is empty; verification_token is missing; encrypt_key is missing target_kinds=message_reply requirements=enabled,app_id,app_secret,allowed_chat_ids,verification_token,encrypt_key",
        channel_serve_command("feishu")
    )));
    assert!(rendered.contains("running=false"));
}

#[test]
fn render_channel_surfaces_text_reports_configured_accounts_for_multi_account_channels() {
    let config: mvp::config::LoongClawConfig = serde_json::from_value(serde_json::json!({
        "telegram": {
            "enabled": true,
            "default_account": "Work Bot",
            "allowed_chat_ids": [1001],
            "accounts": {
                "Work Bot": {
                    "account_id": "Ops-Bot",
                    "bot_token": "123456:token-work",
                    "allowed_chat_ids": [2002]
                },
                "Personal": {
                    "bot_token": "654321:token-personal",
                    "allowed_chat_ids": [3003]
                }
            }
        }
    }))
    .expect("deserialize multi-account config");

    let inventory = mvp::channel::channel_inventory(&config);
    let rendered = render_channel_surfaces_text("/tmp/loongclaw.toml", &inventory);

    assert!(rendered.contains("configured_accounts=2"));
    assert!(rendered.contains("default_configured_account=work-bot"));
    assert!(rendered.contains("configured_account=work-bot"));
    assert!(rendered.contains("configured_account=personal"));
}

#[test]
fn render_channel_surfaces_text_reports_default_account_marker() {
    let config: mvp::config::LoongClawConfig = serde_json::from_value(serde_json::json!({
        "telegram": {
            "enabled": true,
            "default_account": "Work Bot",
            "allowed_chat_ids": [1001],
            "accounts": {
                "Work Bot": {
                    "account_id": "Ops-Bot",
                    "bot_token": "123456:token-work",
                    "allowed_chat_ids": [2002]
                },
                "Personal": {
                    "bot_token": "654321:token-personal",
                    "allowed_chat_ids": [3003]
                }
            }
        }
    }))
    .expect("deserialize multi-account config");

    let inventory = mvp::channel::channel_inventory(&config);
    let rendered = render_channel_surfaces_text("/tmp/loongclaw.toml", &inventory);

    assert!(rendered.contains("configured_account=work-bot"));
    assert!(rendered.contains("default_account=true"));
    assert!(rendered.contains("default_source=explicit_default"));
}

#[test]
fn render_channel_surfaces_text_reports_catalog_only_channels() {
    let config = mvp::config::LoongClawConfig::default();
    let inventory = mvp::channel::channel_inventory(&config);
    let rendered = render_channel_surfaces_text("/tmp/loongclaw.toml", &inventory);

    assert!(rendered.contains("catalog-only channels:"));
    assert!(rendered.contains(
        "Discord [discord] implementation_status=stub capabilities=send,serve,runtime_tracking aliases=discord-bot transport=discord_gateway target_kinds=conversation"
    ));
    assert!(rendered.contains(&format!(
        "catalog op send ({}) availability=stub tracks_runtime=false target_kinds=conversation requirements=-",
        channel_send_command("discord")
    )));
    assert!(rendered.contains(&format!(
        "catalog op serve ({}) availability=stub tracks_runtime=true target_kinds=conversation requirements=-",
        channel_serve_command("discord")
    )));
    assert!(rendered.contains(
        "Slack [slack] implementation_status=stub capabilities=send,serve,runtime_tracking aliases=slack-bot transport=slack_events_api target_kinds=conversation"
    ));
    assert!(rendered.contains(&format!(
        "catalog op send ({}) availability=stub tracks_runtime=false target_kinds=conversation requirements=-",
        channel_send_command("slack")
    )));
    assert!(rendered.contains(&format!(
        "catalog op serve ({}) availability=stub tracks_runtime=true target_kinds=conversation requirements=-",
        channel_serve_command("slack")
    )));
    assert!(rendered.contains(
        "onboarding strategy=planned status_command=\"loongclaw channels --json\" repair_command=-"
    ));
    assert!(rendered.contains("setup_hint=\"stub surface only"));
}

#[test]
fn memory_system_metadata_json_includes_summary_and_source() {
    use mvp::memory::MemorySystem as _;

    let metadata = mvp::memory::BuiltinMemorySystem.metadata();
    let payload = memory_system_metadata_json(&metadata, Some("default"));

    assert_eq!(payload["id"], "builtin");
    assert_eq!(payload["api_version"], 1);
    assert_eq!(payload["source"], "default");
    assert!(
        payload["summary"]
            .as_str()
            .expect("summary should be a string")
            .contains("Built-in")
    );
    assert!(
        payload["capabilities"]
            .as_array()
            .expect("capabilities should be an array")
            .iter()
            .any(|entry| entry == "canonical_store")
    );
}

#[test]
fn build_memory_systems_cli_json_payload_includes_runtime_policy() {
    let config = mvp::config::LoongClawConfig {
        memory: mvp::config::MemoryConfig {
            profile: mvp::config::MemoryProfile::WindowPlusSummary,
            fail_open: false,
            ingest_mode: mvp::config::MemoryIngestMode::AsyncBackground,
            ..mvp::config::MemoryConfig::default()
        },
        ..mvp::config::LoongClawConfig::default()
    };
    let snapshot =
        mvp::memory::collect_memory_system_runtime_snapshot(&config).expect("runtime snapshot");

    let payload = build_memory_systems_cli_json_payload("/tmp/loongclaw.toml", &snapshot);

    assert_eq!(payload["config"], "/tmp/loongclaw.toml");
    assert_eq!(payload["selected"]["id"], "builtin");
    assert_eq!(payload["selected"]["source"], "default");
    assert_eq!(payload["policy"]["backend"], "sqlite");
    assert_eq!(payload["policy"]["profile"], "window_plus_summary");
    assert_eq!(payload["policy"]["mode"], "window_plus_summary");
    assert_eq!(payload["policy"]["ingest_mode"], "async_background");
    assert_eq!(payload["policy"]["fail_open"], false);
    assert_eq!(payload["policy"]["strict_mode_requested"], true);
    assert_eq!(payload["policy"]["strict_mode_active"], false);
    assert_eq!(payload["policy"]["effective_fail_open"], true);
}

#[test]
fn render_memory_system_snapshot_text_reports_fail_open_policy() {
    let config = mvp::config::LoongClawConfig {
        memory: mvp::config::MemoryConfig {
            profile: mvp::config::MemoryProfile::WindowPlusSummary,
            fail_open: false,
            ingest_mode: mvp::config::MemoryIngestMode::AsyncBackground,
            ..mvp::config::MemoryConfig::default()
        },
        ..mvp::config::LoongClawConfig::default()
    };
    let snapshot =
        mvp::memory::collect_memory_system_runtime_snapshot(&config).expect("runtime snapshot");

    let rendered = render_memory_system_snapshot_text("/tmp/loongclaw.toml", &snapshot);

    assert!(rendered.contains("config=/tmp/loongclaw.toml"));
    assert!(rendered.contains("selected=builtin source=default api_version=1"));
    assert!(rendered.contains("policy=backend:sqlite profile:window_plus_summary mode:window_plus_summary ingest_mode:async_background fail_open:false strict_mode_requested:true strict_mode_active:false effective_fail_open:true"));
}

#[test]
fn build_channels_cli_json_payload_includes_operation_requirement_metadata() {
    let config = mvp::config::LoongClawConfig::default();
    let inventory = mvp::channel::channel_inventory(&config);
    let payload = build_channels_cli_json_payload("/tmp/loongclaw.toml", &inventory);
    let encoded = serde_json::to_value(&payload).expect("serialize payload");
    let surfaces = encoded["channel_surfaces"]
        .as_array()
        .expect("channel surfaces array");

    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("telegram")
                    && entry
                        .get("operations")
                        .and_then(serde_json::Value::as_array)
                        .and_then(|operations| operations.first())
                        .and_then(|operation| operation.get("requirements"))
                        .and_then(serde_json::Value::as_array)
                        .map(|requirements| {
                            requirements
                                .iter()
                                .filter_map(|item| item.get("id"))
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["enabled", "bot_token"])
            })
    );

    assert!(surfaces.iter().any(|surface| {
        surface
            .get("catalog")
            .and_then(|catalog| catalog.get("id"))
            .and_then(serde_json::Value::as_str)
            == Some("feishu")
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("operations"))
                .and_then(serde_json::Value::as_array)
                .and_then(|operations| operations.get(1))
                .and_then(|operation| operation.get("requirements"))
                .and_then(serde_json::Value::as_array)
                .map(|requirements| {
                    requirements
                        .iter()
                        .filter_map(|item| item.get("id"))
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(vec![
                    "enabled",
                    "app_id",
                    "app_secret",
                    "allowed_chat_ids",
                    "verification_token",
                    "encrypt_key",
                ])
    }));
}

#[test]
fn build_channels_cli_json_payload_includes_onboarding_metadata() {
    let config = mvp::config::LoongClawConfig::default();
    let inventory = mvp::channel::channel_inventory(&config);
    let payload = build_channels_cli_json_payload("/tmp/loongclaw.toml", &inventory);
    let encoded = serde_json::to_value(&payload).expect("serialize payload");

    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("telegram")
                    && entry
                        .get("onboarding")
                        .and_then(|onboarding| onboarding.get("strategy"))
                        .and_then(serde_json::Value::as_str)
                        == Some("manual_config")
                    && entry
                        .get("onboarding")
                        .and_then(|onboarding| onboarding.get("status_command"))
                        .and_then(serde_json::Value::as_str)
                        == Some("loongclaw doctor")
                    && entry
                        .get("onboarding")
                        .and_then(|onboarding| onboarding.get("repair_command"))
                        .and_then(serde_json::Value::as_str)
                        == Some("loongclaw doctor --fix")
            })
    );

    assert!(
        encoded["channel_surfaces"]
            .as_array()
            .expect("channel surfaces array")
            .iter()
            .any(|surface| {
                surface
                    .get("catalog")
                    .and_then(|catalog| catalog.get("id"))
                    .and_then(serde_json::Value::as_str)
                    == Some("discord")
                    && surface
                        .get("catalog")
                        .and_then(|catalog| catalog.get("onboarding"))
                        .and_then(|onboarding| onboarding.get("strategy"))
                        .and_then(serde_json::Value::as_str)
                        == Some("planned")
                    && surface
                        .get("catalog")
                        .and_then(|catalog| catalog.get("onboarding"))
                        .and_then(|onboarding| onboarding.get("status_command"))
                        .and_then(serde_json::Value::as_str)
                        == Some("loongclaw channels --json")
                    && surface
                        .get("catalog")
                        .and_then(|catalog| catalog.get("onboarding"))
                        .and_then(|onboarding| onboarding.get("repair_command"))
                        .is_some_and(serde_json::Value::is_null)
            })
    );
}

#[test]
fn build_channels_cli_json_payload_includes_full_channel_catalog() {
    let config = mvp::config::LoongClawConfig::default();
    let inventory = mvp::channel::channel_inventory(&config);
    let payload = build_channels_cli_json_payload("/tmp/loongclaw.toml", &inventory);
    let encoded = serde_json::to_value(&payload).expect("serialize payload");

    assert_eq!(
        encoded.get("config").and_then(serde_json::Value::as_str),
        Some("/tmp/loongclaw.toml")
    );
    assert_eq!(
        encoded
            .get("schema")
            .and_then(|schema| schema.get("version"))
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert_eq!(
        encoded
            .get("schema")
            .and_then(|schema| schema.get("primary_channel_view"))
            .and_then(serde_json::Value::as_str),
        Some("channel_surfaces")
    );
    assert_eq!(
        encoded
            .get("schema")
            .and_then(|schema| schema.get("catalog_view"))
            .and_then(serde_json::Value::as_str),
        Some("channel_catalog")
    );
    assert_eq!(
        encoded
            .get("schema")
            .and_then(|schema| schema.get("legacy_channel_views"))
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
            }),
        Some(vec!["channels", "catalog_only_channels"])
    );
    assert_eq!(
        encoded
            .get("channel_catalog")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(4)
    );
    assert_eq!(
        encoded
            .get("catalog_only_channels")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(2)
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("telegram")
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("runtime_backed")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["conversation"])
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("discord")
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("stub")
                    && entry
                        .get("operations")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(|item| item.get("availability"))
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["stub", "stub"])
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["conversation"])
            })
    );
}

#[test]
fn build_channels_cli_json_payload_includes_grouped_channel_surfaces() {
    let config = mvp::config::LoongClawConfig::default();
    let inventory = mvp::channel::channel_inventory(&config);
    let payload = build_channels_cli_json_payload("/tmp/loongclaw.toml", &inventory);
    let encoded = serde_json::to_value(&payload).expect("serialize payload");

    assert_eq!(
        encoded
            .get("channel_surfaces")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(4)
    );

    let surfaces = encoded["channel_surfaces"]
        .as_array()
        .expect("channel surfaces array");

    assert!(surfaces.iter().any(|surface| {
        surface
            .get("catalog")
            .and_then(|catalog| catalog.get("id"))
            .and_then(serde_json::Value::as_str)
            == Some("telegram")
            && surface
                .get("default_configured_account_id")
                .and_then(serde_json::Value::as_str)
                == Some("default")
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("capabilities"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_capability_ids("telegram"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("supported_target_kinds"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_supported_target_kinds("telegram"))
            && surface
                .get("configured_accounts")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                == Some(1)
    }));

    assert!(surfaces.iter().any(|surface| {
        surface
            .get("catalog")
            .and_then(|catalog| catalog.get("id"))
            .and_then(serde_json::Value::as_str)
            == Some("discord")
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("capabilities"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_capability_ids("discord"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("supported_target_kinds"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_supported_target_kinds("discord"))
            && surface
                .get("configured_accounts")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                == Some(0)
    }));
}
