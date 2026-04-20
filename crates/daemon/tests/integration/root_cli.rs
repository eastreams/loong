use super::*;

#[test]
fn cli_uses_loong_program_name() {
    assert_eq!(cli_command_name(), "loong");
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
        help.contains("loong onboard"),
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
fn cli_migrate_help_explains_explicit_config_import_flow() {
    let help = render_cli_help(["migrate"]);

    assert!(
        help.contains("Power-user config import flow"),
        "migrate help should explain when to use the explicit config import command: {help}"
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
        help.contains("loong onboard"),
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
        help.contains("loong chat"),
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
        "loong",
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
fn ask_cli_accepts_latest_session_selector() {
    let cli = try_parse_cli([
        "loong",
        "ask",
        "--message",
        "Summarize this repository",
        "--session",
        "latest",
    ])
    .expect("ask CLI should accept the latest session selector");

    match cli.command {
        Some(Commands::Ask {
            message, session, ..
        }) => {
            assert_eq!(message, "Summarize this repository");
            assert_eq!(session.as_deref(), Some("latest"));
        }
        other => panic!("unexpected command parse result: {other:?}"),
    }
}

#[test]
fn init_spec_cli_accepts_plugin_trust_guard_preset() {
    let cli = try_parse_cli([
        "loong",
        "init-spec",
        "--output",
        "/tmp/plugin-trust-guard.json",
        "--preset",
        "plugin-trust-guard",
    ])
    .expect("init-spec CLI should parse plugin trust guard preset");

    match cli.command {
        Some(Commands::InitSpec { output, preset }) => {
            assert_eq!(output, "/tmp/plugin-trust-guard.json");
            assert_eq!(preset, InitSpecPreset::PluginTrustGuard);
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn run_spec_cli_accepts_render_summary_flag() {
    let cli = try_parse_cli([
        "loong",
        "run-spec",
        "--spec",
        "/tmp/tool-search-trusted.json",
        "--render-summary",
    ])
    .expect("run-spec CLI should parse render summary flag");

    match cli.command {
        Some(Commands::RunSpec {
            spec,
            print_audit,
            render_summary,
            ..
        }) => {
            assert_eq!(spec, "/tmp/tool-search-trusted.json");
            assert!(!print_audit);
            assert!(render_summary);
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn ask_cli_requires_message_flag() {
    let error = try_parse_cli(["loong", "ask"]).expect_err("ask without --message should fail");
    let rendered = error.to_string();

    assert!(
        rendered.contains("--message <MESSAGE>"),
        "parse failure should mention the required message flag: {rendered}"
    );
}

#[test]
fn audit_cli_recent_parses_global_flags_after_subcommand() {
    let cli = try_parse_cli([
        "loong",
        "audit",
        "recent",
        "--config",
        "/tmp/loong.toml",
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
            assert_eq!(config.as_deref(), Some("/tmp/loong.toml"));
            assert!(json);
            match command {
                loong_daemon::audit_cli::AuditCommands::Recent {
                    limit,
                    since_epoch_s,
                    until_epoch_s,
                    pack_id,
                    agent_id,
                    event_id,
                    token_id,
                    kind,
                    triage_label,
                    query_contains,
                    trust_tier,
                } => {
                    assert_eq!(limit, 25);
                    assert_eq!(since_epoch_s, None);
                    assert_eq!(until_epoch_s, None);
                    assert_eq!(pack_id, None);
                    assert_eq!(agent_id, None);
                    assert_eq!(event_id, None);
                    assert_eq!(token_id, None);
                    assert_eq!(kind, None);
                    assert_eq!(triage_label, None);
                    assert_eq!(query_contains, None);
                    assert_eq!(trust_tier, None);
                }
                other => panic!("unexpected audit subcommand parsed: {other:?}"),
            }
        }
        other => panic!("unexpected command parse result: {other:?}"),
    }
}

#[test]
fn audit_cli_summary_parses_limit_without_json() {
    let cli = try_parse_cli(["loong", "audit", "summary", "--limit", "10"])
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
                loong_daemon::audit_cli::AuditCommands::Summary {
                    limit,
                    since_epoch_s,
                    until_epoch_s,
                    pack_id,
                    agent_id,
                    event_id,
                    token_id,
                    kind,
                    triage_label,
                    group_by,
                } => {
                    assert_eq!(limit, 10);
                    assert_eq!(since_epoch_s, None);
                    assert_eq!(until_epoch_s, None);
                    assert_eq!(pack_id, None);
                    assert_eq!(agent_id, None);
                    assert_eq!(event_id, None);
                    assert_eq!(token_id, None);
                    assert_eq!(kind, None);
                    assert_eq!(triage_label, None);
                    assert_eq!(group_by, None);
                }
                other => panic!("unexpected audit subcommand parsed: {other:?}"),
            }
        }
        other => panic!("unexpected command parse result: {other:?}"),
    }
}

#[test]
fn audit_cli_recent_parses_kind_and_triage_filters() {
    let cli = try_parse_cli([
        "loong",
        "audit",
        "recent",
        "--limit",
        "15",
        "--kind",
        "tool-search-evaluated",
        "--triage-label",
        "tool-search-trust-conflict",
    ])
    .expect("audit recent CLI should parse kind and triage filters");

    match cli.command {
        Some(Commands::Audit { command, .. }) => match command {
            loong_daemon::audit_cli::AuditCommands::Recent {
                limit,
                since_epoch_s,
                until_epoch_s,
                pack_id,
                agent_id,
                event_id,
                token_id,
                kind,
                triage_label,
                query_contains,
                trust_tier,
            } => {
                assert_eq!(limit, 15);
                assert_eq!(since_epoch_s, None);
                assert_eq!(until_epoch_s, None);
                assert_eq!(pack_id, None);
                assert_eq!(agent_id, None);
                assert_eq!(event_id, None);
                assert_eq!(token_id, None);
                assert_eq!(kind.as_deref(), Some("ToolSearchEvaluated"));
                assert_eq!(triage_label.as_deref(), Some("tool_search_trust_conflict"));
                assert_eq!(query_contains, None);
                assert_eq!(trust_tier, None);
            }
            other => panic!("unexpected audit subcommand parsed: {other:?}"),
        },
        other => panic!("unexpected command parse result: {other:?}"),
    }
}

#[test]
fn audit_cli_recent_parses_tool_search_filters() {
    let cli = try_parse_cli([
        "loong",
        "audit",
        "recent",
        "--query-contains",
        "trust:official",
        "--trust-tier",
        "verified_community",
        "--kind",
        "tool-search-evaluated",
    ])
    .expect("audit recent CLI should parse tool search filters");

    match cli.command {
        Some(Commands::Audit { command, .. }) => match command {
            loong_daemon::audit_cli::AuditCommands::Recent {
                kind,
                query_contains,
                trust_tier,
                ..
            } => {
                assert_eq!(kind.as_deref(), Some("ToolSearchEvaluated"));
                assert_eq!(query_contains.as_deref(), Some("trust:official"));
                assert_eq!(trust_tier.as_deref(), Some("verified-community"));
            }
            other => panic!("unexpected audit subcommand parsed: {other:?}"),
        },
        other => panic!("unexpected command parse result: {other:?}"),
    }
}

#[test]
fn audit_cli_summary_parses_kind_filter_in_canonical_form() {
    let cli = try_parse_cli(["loong", "audit", "summary", "--kind", "ToolSearchEvaluated"])
        .expect("audit summary CLI should parse canonical event kind filter");

    match cli.command {
        Some(Commands::Audit { command, .. }) => match command {
            loong_daemon::audit_cli::AuditCommands::Summary {
                limit,
                since_epoch_s,
                until_epoch_s,
                pack_id,
                agent_id,
                event_id,
                token_id,
                kind,
                triage_label,
                group_by,
            } => {
                assert_eq!(limit, 200);
                assert_eq!(since_epoch_s, None);
                assert_eq!(until_epoch_s, None);
                assert_eq!(pack_id, None);
                assert_eq!(agent_id, None);
                assert_eq!(event_id, None);
                assert_eq!(token_id, None);
                assert_eq!(kind.as_deref(), Some("ToolSearchEvaluated"));
                assert_eq!(triage_label, None);
                assert_eq!(group_by, None);
            }
            other => panic!("unexpected audit subcommand parsed: {other:?}"),
        },
        other => panic!("unexpected command parse result: {other:?}"),
    }
}

#[test]
fn audit_cli_summary_parses_group_by_alias() {
    let cli = try_parse_cli(["loong", "audit", "summary", "--group-by", "token-id"])
        .expect("audit summary CLI should parse group-by alias");

    match cli.command {
        Some(Commands::Audit { command, .. }) => match command {
            loong_daemon::audit_cli::AuditCommands::Summary { group_by, .. } => {
                assert_eq!(group_by.as_deref(), Some("token"));
            }
            other => panic!("unexpected audit subcommand parsed: {other:?}"),
        },
        other => panic!("unexpected command parse result: {other:?}"),
    }
}

#[test]
fn audit_cli_discovery_parses_trust_filters_and_aliases() {
    let cli = try_parse_cli([
        "loong",
        "audit",
        "discovery",
        "--limit",
        "30",
        "--triage-label",
        "conflict",
        "--query-contains",
        "trust:official",
        "--trust-tier",
        "verified_community",
    ])
    .expect("audit discovery CLI should parse trust filters and aliases");

    match cli.command {
        Some(Commands::Audit { command, .. }) => match command {
            loong_daemon::audit_cli::AuditCommands::Discovery {
                limit,
                since_epoch_s,
                until_epoch_s,
                pack_id,
                agent_id,
                event_id,
                token_id,
                triage_label,
                query_contains,
                trust_tier,
                group_by,
            } => {
                assert_eq!(limit, 30);
                assert_eq!(since_epoch_s, None);
                assert_eq!(until_epoch_s, None);
                assert_eq!(pack_id, None);
                assert_eq!(agent_id, None);
                assert_eq!(event_id, None);
                assert_eq!(token_id, None);
                assert_eq!(triage_label.as_deref(), Some("tool_search_trust_conflict"));
                assert_eq!(query_contains.as_deref(), Some("trust:official"));
                assert_eq!(trust_tier.as_deref(), Some("verified-community"));
                assert_eq!(group_by, None);
            }
            other => panic!("unexpected audit subcommand parsed: {other:?}"),
        },
        other => panic!("unexpected command parse result: {other:?}"),
    }
}

#[test]
fn audit_cli_discovery_parses_group_by_alias() {
    let cli = try_parse_cli(["loong", "audit", "discovery", "--group-by", "agent-id"])
        .expect("audit discovery CLI should parse group-by alias");

    match cli.command {
        Some(Commands::Audit { command, .. }) => match command {
            loong_daemon::audit_cli::AuditCommands::Discovery { group_by, .. } => {
                assert_eq!(group_by.as_deref(), Some("agent"));
            }
            other => panic!("unexpected audit subcommand parsed: {other:?}"),
        },
        other => panic!("unexpected command parse result: {other:?}"),
    }
}

#[test]
fn audit_cli_recent_parses_time_window_filters() {
    let cli = try_parse_cli([
        "loong",
        "audit",
        "recent",
        "--since-epoch-s",
        "1700010000",
        "--until-epoch-s",
        "1700010900",
        "--limit",
        "5",
    ])
    .expect("audit recent CLI should parse time window filters");

    match cli.command {
        Some(Commands::Audit { command, .. }) => match command {
            loong_daemon::audit_cli::AuditCommands::Recent {
                limit,
                since_epoch_s,
                until_epoch_s,
                pack_id,
                agent_id,
                event_id,
                token_id,
                kind,
                triage_label,
                query_contains,
                trust_tier,
            } => {
                assert_eq!(limit, 5);
                assert_eq!(since_epoch_s, Some(1_700_010_000));
                assert_eq!(until_epoch_s, Some(1_700_010_900));
                assert_eq!(pack_id, None);
                assert_eq!(agent_id, None);
                assert_eq!(event_id, None);
                assert_eq!(token_id, None);
                assert_eq!(kind, None);
                assert_eq!(triage_label, None);
                assert_eq!(query_contains, None);
                assert_eq!(trust_tier, None);
            }
            other => panic!("unexpected audit subcommand parsed: {other:?}"),
        },
        other => panic!("unexpected command parse result: {other:?}"),
    }
}

#[test]
fn audit_cli_discovery_parses_pack_and_agent_filters() {
    let cli = try_parse_cli([
        "loong",
        "audit",
        "discovery",
        "--pack-id",
        "sales-intel",
        "--agent-id",
        "agent-search",
        "--limit",
        "7",
    ])
    .expect("audit discovery CLI should parse pack and agent filters");

    match cli.command {
        Some(Commands::Audit { command, .. }) => match command {
            loong_daemon::audit_cli::AuditCommands::Discovery {
                limit,
                since_epoch_s,
                until_epoch_s,
                pack_id,
                agent_id,
                event_id,
                token_id,
                triage_label,
                query_contains,
                trust_tier,
                group_by,
            } => {
                assert_eq!(limit, 7);
                assert_eq!(since_epoch_s, None);
                assert_eq!(until_epoch_s, None);
                assert_eq!(pack_id.as_deref(), Some("sales-intel"));
                assert_eq!(agent_id.as_deref(), Some("agent-search"));
                assert_eq!(event_id, None);
                assert_eq!(token_id, None);
                assert_eq!(triage_label, None);
                assert_eq!(query_contains, None);
                assert_eq!(trust_tier, None);
                assert_eq!(group_by, None);
            }
            other => panic!("unexpected audit subcommand parsed: {other:?}"),
        },
        other => panic!("unexpected command parse result: {other:?}"),
    }
}

#[test]
fn audit_cli_recent_parses_event_and_token_filters() {
    let cli = try_parse_cli([
        "loong",
        "audit",
        "recent",
        "--event-id",
        "evt-123",
        "--token-id",
        "token-abc",
    ])
    .expect("audit recent CLI should parse event and token filters");

    match cli.command {
        Some(Commands::Audit { command, .. }) => match command {
            loong_daemon::audit_cli::AuditCommands::Recent {
                limit,
                since_epoch_s,
                until_epoch_s,
                pack_id,
                agent_id,
                event_id,
                token_id,
                kind,
                triage_label,
                query_contains,
                trust_tier,
            } => {
                assert_eq!(limit, 50);
                assert_eq!(since_epoch_s, None);
                assert_eq!(until_epoch_s, None);
                assert_eq!(pack_id, None);
                assert_eq!(agent_id, None);
                assert_eq!(event_id.as_deref(), Some("evt-123"));
                assert_eq!(token_id.as_deref(), Some("token-abc"));
                assert_eq!(kind, None);
                assert_eq!(triage_label, None);
                assert_eq!(query_contains, None);
                assert_eq!(trust_tier, None);
            }
            other => panic!("unexpected audit subcommand parsed: {other:?}"),
        },
        other => panic!("unexpected command parse result: {other:?}"),
    }
}

#[test]
fn audit_cli_token_trail_parses_required_token_and_identity_filters() {
    let cli = try_parse_cli([
        "loong",
        "audit",
        "token-trail",
        "--token-id",
        "token-abc",
        "--limit",
        "12",
        "--since-epoch-s",
        "1700010000",
        "--until-epoch-s",
        "1700010900",
        "--pack-id",
        "sales-intel",
        "--agent-id",
        "agent-a",
    ])
    .expect("audit token-trail CLI should parse token and identity filters");

    match cli.command {
        Some(Commands::Audit { command, .. }) => match command {
            loong_daemon::audit_cli::AuditCommands::TokenTrail {
                token_id,
                limit,
                since_epoch_s,
                until_epoch_s,
                pack_id,
                agent_id,
            } => {
                assert_eq!(token_id, "token-abc");
                assert_eq!(limit, 12);
                assert_eq!(since_epoch_s, Some(1_700_010_000));
                assert_eq!(until_epoch_s, Some(1_700_010_900));
                assert_eq!(pack_id.as_deref(), Some("sales-intel"));
                assert_eq!(agent_id.as_deref(), Some("agent-a"));
            }
            other => panic!("unexpected audit subcommand parsed: {other:?}"),
        },
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
