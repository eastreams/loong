use super::*;

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
