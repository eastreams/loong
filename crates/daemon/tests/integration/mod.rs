use super::*;
pub use clap::{CommandFactory, Parser};
use std::ffi::OsString;

const CLI_STACK_SIZE_BYTES: usize = 16 * 1024 * 1024;

fn with_cli_stack<T, F>(thread_name: &str, operation: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let thread_builder = std::thread::Builder::new();
    let thread_builder = thread_builder.name(thread_name.to_owned());
    let thread_builder = thread_builder.stack_size(CLI_STACK_SIZE_BYTES);
    let join_handle = thread_builder
        .spawn(operation)
        .expect("spawn CLI stack thread");
    match join_handle.join() {
        Ok(value) => value,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

fn try_parse_cli<const N: usize>(args: [&str; N]) -> Result<Cli, clap::Error> {
    let owned_args = args
        .into_iter()
        .map(OsString::from)
        .collect::<Vec<OsString>>();
    with_cli_stack("integration-cli-parse", move || {
        Cli::try_parse_from(owned_args)
    })
}

fn cli_command_name() -> String {
    with_cli_stack("integration-cli-command-name", || {
        let command = Cli::command();
        command.get_name().to_owned()
    })
}

fn render_cli_help<const N: usize>(subcommand_path: [&str; N]) -> String {
    let owned_path = subcommand_path
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<String>>();
    with_cli_stack("integration-cli-help", move || {
        let mut command = Cli::command();
        let mut current = &mut command;
        for subcommand in owned_path {
            current = current
                .find_subcommand_mut(subcommand.as_str())
                .unwrap_or_else(|| panic!("missing CLI subcommand `{subcommand}`"));
        }
        let mut rendered = Vec::new();
        current
            .write_long_help(&mut rendered)
            .expect("render CLI help");
        String::from_utf8(rendered).expect("help should be utf8")
    })
}

fn validation_diagnostic_with_severity(
    severity: &str,
    code: &str,
) -> mvp::config::ConfigValidationDiagnostic {
    mvp::config::ConfigValidationDiagnostic {
        severity: severity.to_owned(),
        code: code.to_owned(),
        problem_type: format!("urn:loongclaw:problem:{code}"),
        title_key: format!("{code}.title"),
        title: code.to_owned(),
        message_key: code.to_owned(),
        message_locale: "en".to_owned(),
        message_variables: BTreeMap::new(),
        field_path: "active_provider".to_owned(),
        inline_field_path: "providers".to_owned(),
        example_env_name: String::new(),
        suggested_env_name: None,
        message: code.to_owned(),
    }
}

mod acp;
mod architecture;
mod chat_cli;
mod cli_tests;
mod doctor_feishu;
mod feishu_cli;
mod import_cli;
mod memory_context_benchmark_cli;
mod migrate_cli;
mod migration;
mod multi_channel_serve_cli;
mod onboard_cli;
mod programmatic;
mod runtime_capability_cli;
mod runtime_experiment_cli;
mod runtime_restore_cli;
mod runtime_snapshot_cli;
mod skills_cli;
mod spec_runtime;
mod spec_runtime_bridge;

#[test]
fn cli_uses_loongclaw_program_name() {
    assert_eq!(cli_command_name(), "loongclaw");
}

#[test]
fn cli_import_help_explains_explicit_power_user_flow() {
    let help = render_cli_help(["import"]);

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
fn cli_migrate_help_explains_explicit_migration_flow() {
    let help = render_cli_help(["migrate"]);

    assert!(
        help.contains("Power-user migration flow"),
        "migrate help should explain when to use the explicit migration command: {help}"
    );
    assert!(
        help.contains("--mode <MODE>"),
        "migrate help should surface the required mode flag: {help}"
    );
    assert!(
        help.contains("discover"),
        "migrate help should list supported migration modes: {help}"
    );
    assert!(
        help.contains("loongclaw onboard"),
        "migrate help should direct guided users back to onboard: {help}"
    );
}

#[test]
fn cli_onboard_help_mentions_detected_reusable_settings() {
    let help = render_cli_help(["onboard"]);

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
fn cli_ask_help_mentions_one_shot_assistant_usage() {
    let help = render_cli_help(["ask"]);

    assert!(
        help.contains("one-shot"),
        "ask help should describe the non-interactive one-shot flow: {help}"
    );
    assert!(
        help.contains("--message <MESSAGE>"),
        "ask help should require an inline message input: {help}"
    );
    assert!(
        help.contains("loongclaw chat"),
        "ask help should point users to chat for the interactive path: {help}"
    );
}

#[test]
fn cli_runtime_restore_help_mentions_dry_run_default() {
    let help = render_cli_help(["runtime-restore"]);

    assert!(
        help.contains("Dry-run by default"),
        "runtime-restore help should explain the default dry-run behavior: {help}"
    );
    assert!(
        help.contains("--apply"),
        "runtime-restore help should explain how to perform mutations: {help}"
    );
}

#[test]
fn ask_cli_accepts_message_session_and_acp_flags() {
    let cli = try_parse_cli([
        "loongclaw",
        "ask",
        "--message",
        "Summarize this repository",
        "--session",
        "telegram:42",
        "--acp",
        "--acp-event-stream",
        "--acp-bootstrap-mcp-server",
        "filesystem",
        "--acp-cwd",
        "/workspace/project",
    ])
    .expect("ask CLI should parse one-shot flags");

    match cli.command {
        Some(Commands::Ask {
            message,
            session,
            acp,
            acp_event_stream,
            acp_bootstrap_mcp_server,
            acp_cwd,
            ..
        }) => {
            assert_eq!(message, "Summarize this repository");
            assert_eq!(session.as_deref(), Some("telegram:42"));
            assert!(acp);
            assert!(acp_event_stream);
            assert_eq!(acp_bootstrap_mcp_server, vec!["filesystem".to_owned()]);
            assert_eq!(acp_cwd.as_deref(), Some("/workspace/project"));
        }
        other => panic!("unexpected command parse result: {other:?}"),
    }
}

#[test]
fn ask_cli_requires_message_flag() {
    let error = try_parse_cli(["loongclaw", "ask"]).expect_err("ask without --message should fail");
    let rendered = error.to_string();

    assert!(
        rendered.contains("--message <MESSAGE>"),
        "parse failure should mention the required message flag: {rendered}"
    );
}

#[test]
fn audit_cli_recent_parses_global_flags_after_subcommand() {
    let cli = try_parse_cli([
        "loongclaw",
        "audit",
        "recent",
        "--config",
        "/tmp/loongclaw.toml",
        "--limit",
        "25",
        "--json",
    ])
    .expect("audit recent CLI should parse");

    match cli.command {
        Some(Commands::Audit {
            config,
            json,
            command,
        }) => {
            assert_eq!(config.as_deref(), Some("/tmp/loongclaw.toml"));
            assert!(json);
            match command {
                loongclaw_daemon::audit_cli::AuditCommands::Recent { limit } => {
                    assert_eq!(limit, 25);
                }
                other => panic!("unexpected audit subcommand parsed: {other:?}"),
            }
        }
        other => panic!("unexpected command parse result: {other:?}"),
    }
}

#[test]
fn audit_cli_summary_parses_limit_without_json() {
    let cli = try_parse_cli(["loongclaw", "audit", "summary", "--limit", "10"])
        .expect("audit summary CLI should parse");

    match cli.command {
        Some(Commands::Audit {
            config,
            json,
            command,
        }) => {
            assert_eq!(config, None);
            assert!(!json);
            match command {
                loongclaw_daemon::audit_cli::AuditCommands::Summary { limit } => {
                    assert_eq!(limit, 10);
                }
                other => panic!("unexpected audit subcommand parsed: {other:?}"),
            }
        }
        other => panic!("unexpected command parse result: {other:?}"),
    }
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
fn validation_summary_treats_warning_only_diagnostics_as_valid() {
    let summary = summarize_validation_diagnostics(&[validation_diagnostic_with_severity(
        "warn",
        "config.provider_selection.implicit_active",
    )]);

    assert!(summary.valid);
    assert_eq!(summary.error_count, 0);
    assert_eq!(summary.warning_count, 1);
}

#[test]
fn validation_summary_counts_error_and_warning_diagnostics_separately() {
    let summary = summarize_validation_diagnostics(&[
        validation_diagnostic_with_severity("error", "config.env_pointer.dollar_prefix"),
        validation_diagnostic_with_severity("warn", "config.provider_selection.implicit_active"),
    ]);

    assert!(!summary.valid);
    assert_eq!(summary.error_count, 1);
    assert_eq!(summary.warning_count, 1);
}

#[test]
fn render_channel_surfaces_text_reports_aliases_and_operation_health() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.telegram.enabled = true;
    config.telegram.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
        "123456:telegram-token".to_owned(),
    ));
    config.telegram.allowed_chat_ids = vec![1001];
    config.feishu.enabled = true;
    config.feishu.app_id = Some(loongclaw_contracts::SecretRef::Inline(
        "cli_a1b2c3".to_owned(),
    ));
    config.feishu.app_secret = Some(loongclaw_contracts::SecretRef::Inline(
        "app-secret".to_owned(),
    ));
    config.wecom.enabled = true;
    config.wecom.bot_id = Some(loongclaw_contracts::SecretRef::Inline(
        "bot_test".to_owned(),
    ));
    config.wecom.secret = Some(loongclaw_contracts::SecretRef::Inline(
        "secret_test".to_owned(),
    ));
    config.wecom.allowed_conversation_ids = vec!["group_demo".to_owned()];

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
        "op serve ({}) misconfigured: allowed_chat_ids is empty; verification_token is missing; encrypt_key is missing target_kinds=message_reply requirements=enabled,app_id,app_secret,mode,allowed_chat_ids,verification_token,encrypt_key",
        channel_serve_command("feishu")
    )));
    assert!(rendered.contains("WeCom [wecom]"));
    assert!(rendered.contains("account=wecom:bot_test"));
    assert!(rendered.contains(&format!(
        "op send ({}) ready: ready target_kinds=conversation requirements=enabled,bot_id,secret,websocket_url",
        channel_send_command("wecom")
    )));
    assert!(rendered.contains(&format!(
        "op serve ({}) ready: ready target_kinds=conversation requirements=enabled,bot_id,secret,allowed_conversation_ids,websocket_url,ping_interval_s",
        channel_serve_command("wecom")
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
        "Discord [discord] implementation_status=config_backed selection_order=40 selection_label=\"community server bot\" capabilities=multi_account,send aliases=discord-bot transport=discord_http_api target_kinds=conversation configured_accounts=1 default_configured_account=default"
    ));
    assert!(rendered.contains(
        "blurb: Shipped Discord outbound message surface with config-backed direct sends; inbound gateway/runtime support remains planned."
    ));
    assert!(rendered.contains(&format!(
        "op send ({}) disabled: disabled by discord account configuration target_kinds=conversation requirements=enabled,bot_token",
        channel_send_command("discord")
    )));
    assert!(rendered.contains(&format!(
        "op serve ({}) unsupported: discord serve runtime is not implemented yet target_kinds=conversation requirements=enabled,bot_token,application_id,allowed_guild_ids",
        channel_serve_command("discord")
    )));
    assert!(rendered.contains(
        "Slack [slack] implementation_status=config_backed selection_order=50 selection_label=\"workspace event bot\" capabilities=multi_account,send aliases=slack-bot transport=slack_web_api target_kinds=conversation configured_accounts=1 default_configured_account=default"
    ));
    assert!(rendered.contains(&format!(
        "op send ({}) disabled: disabled by slack account configuration target_kinds=conversation requirements=enabled,bot_token",
        channel_send_command("slack")
    )));
    assert!(rendered.contains(&format!(
        "op serve ({}) unsupported: slack serve runtime is not implemented yet target_kinds=conversation requirements=enabled,bot_token,app_token,signing_secret,allowed_channel_ids",
        channel_serve_command("slack")
    )));
    assert!(rendered.contains(
        "WhatsApp [whatsapp] implementation_status=config_backed selection_order=90 selection_label=\"business messaging app\" capabilities=multi_account,send aliases=wa,whatsapp-cloud transport=whatsapp_cloud_api target_kinds=address configured_accounts=1 default_configured_account=default"
    ));
    assert!(rendered.contains(&format!(
        "op send ({}) disabled: disabled by whatsapp account configuration target_kinds=address requirements=enabled,access_token,phone_number_id",
        channel_send_command("whatsapp")
    )));
    assert!(rendered.contains(&format!(
        "op serve ({}) unsupported: whatsapp serve runtime is not implemented yet target_kinds=address requirements=enabled,access_token,phone_number_id,verify_token,app_secret",
        channel_serve_command("whatsapp")
    )));
    assert!(rendered.contains(
        "LINE [line] implementation_status=config_backed selection_order=60 selection_label=\"consumer messaging bot\" capabilities=multi_account,send aliases=line-bot transport=line_messaging_api target_kinds=address configured_accounts=1 default_configured_account=default"
    ));
    assert!(rendered.contains(
        "DingTalk [dingtalk] implementation_status=config_backed selection_order=80 selection_label=\"group webhook bot\" capabilities=multi_account,send aliases=ding,ding-bot transport=dingtalk_custom_robot_webhook target_kinds=endpoint configured_accounts=1 default_configured_account=default"
    ));
    assert!(rendered.contains(
        "Google Chat [google-chat] implementation_status=config_backed selection_order=120 selection_label=\"workspace space webhook\" capabilities=multi_account,send aliases=gchat,googlechat transport=google_chat_incoming_webhook target_kinds=endpoint configured_accounts=1 default_configured_account=default"
    ));
    assert!(rendered.contains(
        "op send (dingtalk-send) disabled: disabled by dingtalk account configuration target_kinds=endpoint requirements=enabled,webhook_url"
    ));
    assert!(rendered.contains(
        "op send (google-chat-send) disabled: disabled by google_chat account configuration target_kinds=endpoint requirements=enabled,webhook_url"
    ));
    assert!(rendered.contains(
        "op serve (google-chat-serve) unsupported: google chat incoming webhook surface is outbound-only target_kinds=endpoint requirements=enabled,webhook_url"
    ));
    assert!(rendered.contains(
        "Signal [signal] implementation_status=config_backed selection_order=130 selection_label=\"private messenger bridge\" capabilities=multi_account,send aliases=signal-cli transport=signal_cli_rest_api target_kinds=address configured_accounts=1 default_configured_account=default"
    ));
    assert!(rendered.contains(
        "op send (signal-send) disabled: disabled by signal account configuration target_kinds=address requirements=enabled,service_url,account"
    ));
    assert!(rendered.contains(
        "op serve (signal-serve) unsupported: signal serve runtime is not implemented yet target_kinds=address requirements=enabled,service_url,account"
    ));
    assert!(rendered.contains(
        "Microsoft Teams [teams] implementation_status=config_backed selection_order=140 selection_label=\"workspace webhook bot\" capabilities=multi_account,send aliases=msteams,ms-teams transport=microsoft_teams_incoming_webhook target_kinds=endpoint,conversation configured_accounts=1 default_configured_account=default"
    ));
    assert!(rendered.contains(
        "op send (teams-send) disabled: disabled by teams account configuration target_kinds=endpoint requirements=enabled,webhook_url"
    ));
    assert!(rendered.contains(
        "op serve (teams-serve) unsupported: microsoft teams incoming webhook surface is outbound-only today target_kinds=conversation requirements=enabled,app_id,app_password,tenant_id,allowed_conversation_ids"
    ));
    assert!(rendered.contains(
        "Nextcloud Talk [nextcloud-talk] implementation_status=config_backed selection_order=160 selection_label=\"self-hosted room bot\" capabilities=multi_account,send aliases=nextcloud,nextcloudtalk transport=nextcloud_talk_bot_api target_kinds=conversation configured_accounts=1 default_configured_account=default"
    ));
    assert!(rendered.contains(
        "op send (nextcloud-talk-send) disabled: disabled by nextcloud_talk account configuration target_kinds=conversation requirements=enabled,server_url,shared_secret"
    ));
    assert!(rendered.contains(
        "op serve (nextcloud-talk-serve) unsupported: nextcloud talk bot callback serve is not implemented yet target_kinds=conversation requirements=enabled,server_url,shared_secret"
    ));
    assert!(rendered.contains(
        "Synology Chat [synology-chat] implementation_status=config_backed selection_order=165 selection_label=\"nas webhook bot\" capabilities=multi_account,send aliases=synologychat,synochat transport=synology_chat_outgoing_incoming_webhooks target_kinds=address configured_accounts=1 default_configured_account=default"
    ));
    assert!(rendered.contains(
        "op send (synology-chat-send) disabled: disabled by synology_chat account configuration target_kinds=address requirements=enabled,incoming_url"
    ));
    assert!(rendered.contains(
        "op serve (synology-chat-serve) unsupported: synology chat outgoing webhook serve is not implemented yet target_kinds=address requirements=enabled,token,incoming_url,allowed_user_ids"
    ));
    assert!(rendered.contains(
        "iMessage [imessage] implementation_status=config_backed selection_order=180 selection_label=\"apple message bridge\" capabilities=multi_account,send aliases=bluebubbles,blue-bubbles transport=imessage_bridge_api target_kinds=conversation configured_accounts=1 default_configured_account=default"
    ));
    assert!(rendered.contains(
        "op send (imessage-send) disabled: disabled by imessage account configuration target_kinds=conversation requirements=enabled,bridge_url,bridge_token"
    ));
    assert!(rendered.contains(
        "op serve (imessage-serve) unsupported: imessage bridge sync runtime is not implemented yet target_kinds=conversation requirements=enabled,bridge_url,bridge_token,allowed_chat_ids"
    ));
    assert!(rendered.contains(
        "Webhook [webhook] implementation_status=config_backed selection_order=110 selection_label=\"generic http integration\" capabilities=multi_account,send aliases=http-webhook transport=generic_webhook target_kinds=endpoint configured_accounts=1 default_configured_account=default"
    ));
    assert!(rendered.contains(
        "WebChat [webchat] implementation_status=stub selection_order=230 selection_label=\"embedded web inbox\""
    ));
    assert!(rendered.contains(
        "op send (webhook-send) disabled: disabled by webhook account configuration target_kinds=endpoint requirements=enabled,endpoint_url"
    ));
    assert!(rendered.contains(
        "op serve (webhook-serve) unsupported: generic webhook serve runtime is not implemented yet target_kinds=endpoint requirements=enabled,public_base_url,signing_secret"
    ));
    assert!(rendered.contains(
        "onboarding strategy=manual_config status_command=\"loongclaw doctor\" repair_command=\"loongclaw doctor --fix\""
    ));
    assert!(rendered.contains(
        "setup_hint=\"configure discord bot credentials in loongclaw.toml under discord or discord.accounts.<account>; outbound direct send is shipped, while gateway-based serve support remains planned\""
    ));
}

#[test]
fn render_channel_surfaces_text_reports_managed_plugin_bridge_discovery() {
    let config = mvp::config::LoongClawConfig::default();
    let inventory = mvp::channel::channel_inventory(&config);
    let rendered = render_channel_surfaces_text("/tmp/loongclaw.toml", &inventory);

    assert!(
        rendered.contains("Weixin [weixin]"),
        "rendered channel surfaces should include the weixin surface: {rendered}"
    );
    assert!(
        rendered.contains(
            "managed_plugin_bridge_discovery status=not_configured managed_install_root=- scan_issue=- compatible=0 compatible_plugin_ids=- ambiguity_status=- incomplete=0 incompatible=0"
        ),
        "rendered channel surfaces should include managed discovery summaries: {rendered}"
    );
}

#[test]
fn render_channel_surfaces_text_reports_managed_plugin_bridge_ambiguity_and_setup_guidance() {
    let config = mvp::config::LoongClawConfig::default();
    let mut inventory = mvp::channel::channel_inventory(&config);
    let weixin_surface = inventory
        .channel_surfaces
        .iter_mut()
        .find(|surface| surface.catalog.id == "weixin")
        .expect("weixin surface");
    let discovery = weixin_surface
        .plugin_bridge_discovery
        .as_mut()
        .expect("weixin managed discovery");

    discovery.status = mvp::channel::ChannelPluginBridgeDiscoveryStatus::MatchesFound;
    discovery.ambiguity_status =
        Some(mvp::channel::ChannelPluginBridgeDiscoveryAmbiguityStatus::MultipleCompatiblePlugins);
    discovery.compatible_plugins = 2;
    discovery.compatible_plugin_ids =
        vec!["weixin-bridge-a".to_owned(), "weixin-bridge-b".to_owned()];
    discovery.incomplete_plugins = 1;
    discovery.incompatible_plugins = 0;
    discovery.plugins = vec![mvp::channel::ChannelDiscoveredPluginBridge {
        plugin_id: "weixin-bridge-a".to_owned(),
        source_path: "/tmp/weixin-bridge-a/loongclaw.plugin.json".to_owned(),
        package_root: "/tmp/weixin-bridge-a".to_owned(),
        package_manifest_path: Some("/tmp/weixin-bridge-a/loongclaw.plugin.json".to_owned()),
        bridge_kind: "managed_connector".to_owned(),
        adapter_family: "channel-bridge".to_owned(),
        transport_family: Some("wechat_clawbot_ilink_bridge".to_owned()),
        target_contract: Some("weixin_reply_loop".to_owned()),
        account_scope: Some("shared".to_owned()),
        status: mvp::channel::ChannelDiscoveredPluginBridgeStatus::CompatibleIncompleteContract,
        issues: vec!["example issue".to_owned()],
        missing_fields: vec!["metadata.transport_family".to_owned()],
        required_env_vars: vec!["WEIXIN_BRIDGE_URL".to_owned()],
        recommended_env_vars: vec!["WEIXIN_BRIDGE_ACCESS_TOKEN".to_owned()],
        required_config_keys: vec!["weixin.bridge_url".to_owned()],
        default_env_var: Some("WEIXIN_BRIDGE_URL".to_owned()),
        setup_docs_urls: vec!["https://example.test/docs/weixin-bridge".to_owned()],
        setup_remediation: Some(
            "Run the ClawBot setup flow before enabling this bridge.\nThen verify only one managed bridge remains.".to_owned(),
        ),
    }];

    let rendered = render_channel_surfaces_text("/tmp/loongclaw.toml", &inventory);

    assert!(
        rendered.contains("ambiguity_status=multiple_compatible_plugins"),
        "rendered channel surfaces should expose managed bridge ambiguity status: {rendered}"
    );
    assert!(
        rendered.contains("compatible_plugin_ids=weixin-bridge-a,weixin-bridge-b"),
        "rendered channel surfaces should expose managed bridge compatible plugin ids: {rendered}"
    );
    assert!(
        rendered.contains("required_env_vars=WEIXIN_BRIDGE_URL"),
        "rendered channel surfaces should expose managed bridge setup env requirements: {rendered}"
    );
    assert!(
        rendered.contains("setup_docs_urls=https://example.test/docs/weixin-bridge"),
        "rendered channel surfaces should expose managed bridge setup docs links: {rendered}"
    );
    assert!(
        rendered.contains(
            "setup_remediation=\"Run the ClawBot setup flow before enabling this bridge.\\nThen verify only one managed bridge remains.\""
        ),
        "rendered channel surfaces should expose managed bridge setup remediation text: {rendered}"
    );
}

#[test]
fn memory_system_metadata_json_includes_stage_families_summary_and_source() {
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
    assert_eq!(
        payload["supported_pre_assembly_stage_families"],
        json!(["derive", "retrieve", "rank"])
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
    assert_eq!(
        payload["selected"]["supported_pre_assembly_stage_families"],
        json!(["derive", "retrieve", "rank"])
    );
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
    assert!(rendered.contains(
        "selected=builtin source=default api_version=1 capabilities=canonical_store,deterministic_summary,profile_note_projection,prompt_hydration pre_assembly_stages=derive,retrieve,rank"
    ));
    assert!(rendered.contains("policy=backend:sqlite profile:window_plus_summary mode:window_plus_summary ingest_mode:async_background fail_open:false strict_mode_requested:true strict_mode_active:false effective_fail_open:true"));
    assert!(rendered.contains(
        "- builtin api_version=1 capabilities=canonical_store,deterministic_summary,profile_note_projection,prompt_hydration pre_assembly_stages=derive,retrieve,rank"
    ));
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
                    "mode",
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
                        == Some("manual_config")
                    && surface
                        .get("catalog")
                        .and_then(|catalog| catalog.get("onboarding"))
                        .and_then(|onboarding| onboarding.get("status_command"))
                        .and_then(serde_json::Value::as_str)
                        == Some("loongclaw doctor")
                    && surface
                        .get("catalog")
                        .and_then(|catalog| catalog.get("onboarding"))
                        .and_then(|onboarding| onboarding.get("repair_command"))
                        .and_then(serde_json::Value::as_str)
                        == Some("loongclaw doctor --fix")
            })
    );
}

#[test]
fn build_channels_cli_json_payload_includes_plugin_bridge_contracts() {
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
                entry.get("id").and_then(serde_json::Value::as_str) == Some("weixin")
                    && entry
                        .get("plugin_bridge_contract")
                        .and_then(|contract| contract.get("manifest_channel_id"))
                        .and_then(serde_json::Value::as_str)
                        == Some("weixin")
                    && entry
                        .get("plugin_bridge_contract")
                        .and_then(|contract| contract.get("required_setup_surface"))
                        .and_then(serde_json::Value::as_str)
                        == Some("channel")
                    && entry
                        .get("plugin_bridge_contract")
                        .and_then(|contract| contract.get("required_metadata_keys"))
                        .and_then(serde_json::Value::as_array)
                        .map(|keys| {
                            keys.iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["transport_family", "target_contract"])
                    && entry
                        .get("plugin_bridge_contract")
                        .and_then(|contract| contract.get("runtime_owner"))
                        .and_then(serde_json::Value::as_str)
                        == Some("external_plugin")
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
                    == Some("qqbot")
                    && surface
                        .get("catalog")
                        .and_then(|catalog| catalog.get("plugin_bridge_contract"))
                        .and_then(|contract| contract.get("supported_operations"))
                        .and_then(serde_json::Value::as_array)
                        .map(|operations| {
                            operations
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["send", "serve"])
            })
    );
}

#[test]
fn build_channels_cli_json_payload_includes_managed_plugin_bridge_discovery() {
    let config = mvp::config::LoongClawConfig::default();
    let inventory = mvp::channel::channel_inventory(&config);
    let payload = build_channels_cli_json_payload("/tmp/loongclaw.toml", &inventory);
    let encoded = serde_json::to_value(&payload).expect("serialize payload");

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
                    == Some("weixin")
                    && surface
                        .get("plugin_bridge_discovery")
                        .and_then(|discovery| discovery.get("status"))
                        .and_then(serde_json::Value::as_str)
                        == Some("not_configured")
                    && surface
                        .get("plugin_bridge_discovery")
                        .and_then(|discovery| discovery.get("compatible_plugins"))
                        .and_then(serde_json::Value::as_u64)
                        == Some(0)
            })
    );
}

#[test]
fn build_channels_cli_json_payload_includes_managed_plugin_bridge_guidance_fields() {
    let config = mvp::config::LoongClawConfig::default();
    let mut inventory = mvp::channel::channel_inventory(&config);
    let weixin_surface = inventory
        .channel_surfaces
        .iter_mut()
        .find(|surface| surface.catalog.id == "weixin")
        .expect("weixin surface");
    let discovery = weixin_surface
        .plugin_bridge_discovery
        .as_mut()
        .expect("weixin managed discovery");

    discovery.status = mvp::channel::ChannelPluginBridgeDiscoveryStatus::MatchesFound;
    discovery.ambiguity_status =
        Some(mvp::channel::ChannelPluginBridgeDiscoveryAmbiguityStatus::MultipleCompatiblePlugins);
    discovery.compatible_plugins = 2;
    discovery.compatible_plugin_ids =
        vec!["weixin-bridge-a".to_owned(), "weixin-bridge-b".to_owned()];
    discovery.plugins = vec![mvp::channel::ChannelDiscoveredPluginBridge {
        plugin_id: "weixin-bridge-a".to_owned(),
        source_path: "/tmp/weixin-bridge-a/loongclaw.plugin.json".to_owned(),
        package_root: "/tmp/weixin-bridge-a".to_owned(),
        package_manifest_path: Some("/tmp/weixin-bridge-a/loongclaw.plugin.json".to_owned()),
        bridge_kind: "managed_connector".to_owned(),
        adapter_family: "channel-bridge".to_owned(),
        transport_family: Some("wechat_clawbot_ilink_bridge".to_owned()),
        target_contract: Some("weixin_reply_loop".to_owned()),
        account_scope: Some("shared".to_owned()),
        status: mvp::channel::ChannelDiscoveredPluginBridgeStatus::CompatibleReady,
        issues: Vec::new(),
        missing_fields: Vec::new(),
        required_env_vars: vec!["WEIXIN_BRIDGE_URL".to_owned()],
        recommended_env_vars: vec!["WEIXIN_BRIDGE_ACCESS_TOKEN".to_owned()],
        required_config_keys: vec!["weixin.bridge_url".to_owned()],
        default_env_var: Some("WEIXIN_BRIDGE_URL".to_owned()),
        setup_docs_urls: vec!["https://example.test/docs/weixin-bridge".to_owned()],
        setup_remediation: Some(
            "Run the ClawBot setup flow before enabling this bridge.".to_owned(),
        ),
    }];

    let payload = build_channels_cli_json_payload("/tmp/loongclaw.toml", &inventory);
    let encoded = serde_json::to_value(&payload).expect("serialize payload");
    let surfaces = encoded["channel_surfaces"]
        .as_array()
        .expect("channel surfaces array");
    let weixin = surfaces
        .iter()
        .find(|surface| {
            surface
                .get("catalog")
                .and_then(|catalog| catalog.get("id"))
                .and_then(serde_json::Value::as_str)
                == Some("weixin")
        })
        .expect("weixin surface entry");

    assert_eq!(
        weixin["plugin_bridge_discovery"]["ambiguity_status"]
            .as_str()
            .expect("ambiguity_status should be string"),
        "multiple_compatible_plugins"
    );
    assert_eq!(
        weixin["plugin_bridge_discovery"]["compatible_plugin_ids"]
            .as_array()
            .expect("compatible_plugin_ids should be array")
            .len(),
        2
    );
    assert_eq!(
        weixin["plugin_bridge_discovery"]["plugins"][0]["setup_docs_urls"][0]
            .as_str()
            .expect("setup docs url should be string"),
        "https://example.test/docs/weixin-bridge"
    );
    assert_eq!(
        weixin["plugin_bridge_discovery"]["plugins"][0]["setup_remediation"]
            .as_str()
            .expect("setup remediation should be string"),
        "Run the ClawBot setup flow before enabling this bridge."
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
        Some(inventory.channel_catalog.len())
    );
    assert_eq!(
        encoded
            .get("catalog_only_channels")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(inventory.catalog_only_channels.len())
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
                entry.get("id").and_then(serde_json::Value::as_str) == Some("matrix")
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
                entry.get("id").and_then(serde_json::Value::as_str) == Some("wecom")
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
                        == Some("config_backed")
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
                        == Some(vec!["implemented", "stub"])
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
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(40)
                    && entry
                        .get("selection_label")
                        .and_then(serde_json::Value::as_str)
                        == Some("community server bot")
                    && entry
                        .get("blurb")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|value| value.contains("config-backed direct sends"))
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("slack")
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("config_backed")
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
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(50)
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("whatsapp")
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("config_backed")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["address"])
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(90)
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("webhook")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["endpoint"])
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(110)
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("google-chat")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["endpoint"])
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(120)
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("teams")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["endpoint", "conversation"])
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(140)
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("config_backed")
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("nextcloud-talk")
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
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(160)
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("config_backed")
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("imessage")
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
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(180)
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("config_backed")
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("synology-chat")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["address"])
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(165)
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("config_backed")
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("signal")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["address"])
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(130)
            })
    );
}

#[test]
fn build_channels_cli_json_payload_includes_grouped_channel_surfaces() {
    let _env = super::MigrationEnvironmentGuard::set(&[("TELEGRAM_BOT_TOKEN", None)]);

    let config = mvp::config::LoongClawConfig::default();
    let inventory = mvp::channel::channel_inventory(&config);
    let payload = build_channels_cli_json_payload("/tmp/loongclaw.toml", &inventory);
    let encoded = serde_json::to_value(&payload).expect("serialize payload");

    assert_eq!(
        encoded
            .get("channel_surfaces")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(inventory.channel_surfaces.len())
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
            == Some("slack")
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
                == Some(channel_capability_ids("slack"))
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
                == Some(channel_supported_target_kinds("slack"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("selection_order"))
                .and_then(serde_json::Value::as_u64)
                == Some(50)
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
            == Some("whatsapp")
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
                == Some(channel_capability_ids("whatsapp"))
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
                == Some(channel_supported_target_kinds("whatsapp"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("selection_order"))
                .and_then(serde_json::Value::as_u64)
                == Some(90)
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
            == Some("signal")
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
                == Some(channel_capability_ids("signal"))
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
                == Some(channel_supported_target_kinds("signal"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("selection_order"))
                .and_then(serde_json::Value::as_u64)
                == Some(130)
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
            == Some("wecom")
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
                == Some(channel_capability_ids("wecom"))
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
                == Some(channel_supported_target_kinds("wecom"))
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
            == Some("matrix")
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
                == Some(channel_capability_ids("matrix"))
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
                == Some(channel_supported_target_kinds("matrix"))
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
                .get("catalog")
                .and_then(|catalog| catalog.get("selection_order"))
                .and_then(serde_json::Value::as_u64)
                == Some(40)
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
            == Some("webhook")
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
                == Some(channel_supported_target_kinds("webhook"))
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
            == Some("webchat")
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("selection_order"))
                .and_then(serde_json::Value::as_u64)
                == Some(230)
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
                == Some(channel_supported_target_kinds("webchat"))
            && surface
                .get("configured_accounts")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                == Some(0)
    }));
}
