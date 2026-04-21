use super::*;

#[test]
fn list_mcp_servers_cli_parses_json_flag() {
    let cli = try_parse_cli(["loong", "runtime", "mcp", "list", "--json"])
        .expect("`runtime mcp list --json` should parse");

    match cli.command {
        Some(Commands::Runtime {
            command:
                loong_daemon::runtime_cli::RuntimeCommands::Mcp {
                    command:
                        loong_daemon::runtime_cli::RuntimeMcpCommands::List(
                            loong_daemon::runtime_cli::RuntimeReadArgs { config, json },
                        ),
                },
        }) => {
            assert_eq!(config, None);
            assert!(json);
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn show_mcp_server_cli_parses_name_and_json_flag() {
    let cli = try_parse_cli([
        "loong", "runtime", "mcp", "show", "--name", "docs", "--json",
    ])
    .expect("`runtime mcp show --name docs --json` should parse");

    match cli.command {
        Some(Commands::Runtime {
            command:
                loong_daemon::runtime_cli::RuntimeCommands::Mcp {
                    command:
                        loong_daemon::runtime_cli::RuntimeMcpCommands::Show(
                            loong_daemon::runtime_cli::RuntimeShowMcpServerArgs {
                                config,
                                name,
                                json,
                            },
                        ),
                },
        }) => {
            assert_eq!(config, None);
            assert_eq!(name, "docs");
            assert!(json);
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn build_mcp_servers_cli_json_payload_includes_server_status_and_missing_selection() {
    let snapshot = mvp::mcp::McpRuntimeSnapshot {
        servers: vec![mvp::mcp::McpRuntimeServerSnapshot {
            name: "docs".to_owned(),
            enabled: true,
            required: false,
            selected_for_acp_bootstrap: true,
            origins: vec![
                mvp::mcp::McpServerOrigin {
                    kind: mvp::mcp::McpServerOriginKind::Config,
                    source_id: None,
                },
                mvp::mcp::McpServerOrigin {
                    kind: mvp::mcp::McpServerOriginKind::AcpBootstrapSelection,
                    source_id: None,
                },
            ],
            status: mvp::mcp::McpServerStatus {
                kind: mvp::mcp::McpServerStatusKind::Pending,
                auth: mvp::mcp::McpAuthStatus::Unknown,
                last_error: None,
            },
            transport: mvp::mcp::McpTransportSnapshot::Stdio {
                command: "uvx".to_owned(),
                args: vec!["context7-mcp".to_owned()],
                cwd: Some("/workspace/repo".to_owned()),
                env_var_names: vec!["API_TOKEN".to_owned()],
            },
            enabled_tools: vec!["search".to_owned()],
            disabled_tools: vec!["write".to_owned()],
            startup_timeout_ms: Some(15_000),
            tool_timeout_ms: Some(120_000),
        }],
        missing_selected_servers: vec!["missing".to_owned()],
    };

    let payload = build_mcp_servers_cli_json_payload("/tmp/loong.toml", &snapshot);

    assert_eq!(payload["config"], "/tmp/loong.toml");
    assert_eq!(payload["server_count"], 1);
    assert_eq!(payload["missing_selected_servers"][0], "missing");
    assert_eq!(payload["servers"][0]["name"], "docs");
    assert_eq!(payload["servers"][0]["selected_for_acp_bootstrap"], true);
    assert_eq!(payload["servers"][0]["status"]["kind"], "pending");
    assert_eq!(payload["servers"][0]["transport"]["transport"], "stdio");
    assert_eq!(payload["servers"][0]["transport"]["command"], "uvx");
    assert_eq!(payload["servers"][0]["enabled_tools"][0], "search");
    assert_eq!(payload["servers"][0]["disabled_tools"][0], "write");
    assert_eq!(payload["servers"][0]["origins"][0]["kind"], "config");
    assert_eq!(
        payload["servers"][0]["origins"][1]["kind"],
        "acp_bootstrap_selection"
    );
}

#[test]
fn build_mcp_server_detail_cli_json_payload_wraps_single_server() {
    let server = mvp::mcp::McpRuntimeServerSnapshot {
        name: "docs".to_owned(),
        enabled: true,
        required: false,
        selected_for_acp_bootstrap: true,
        origins: vec![mvp::mcp::McpServerOrigin {
            kind: mvp::mcp::McpServerOriginKind::Config,
            source_id: None,
        }],
        status: mvp::mcp::McpServerStatus {
            kind: mvp::mcp::McpServerStatusKind::Pending,
            auth: mvp::mcp::McpAuthStatus::Unknown,
            last_error: None,
        },
        transport: mvp::mcp::McpTransportSnapshot::Stdio {
            command: "uvx".to_owned(),
            args: vec!["context7-mcp".to_owned()],
            cwd: Some("/workspace/repo".to_owned()),
            env_var_names: vec!["API_TOKEN".to_owned()],
        },
        enabled_tools: vec!["search".to_owned()],
        disabled_tools: vec!["write".to_owned()],
        startup_timeout_ms: Some(15_000),
        tool_timeout_ms: Some(120_000),
    };

    let payload = build_mcp_server_detail_cli_json_payload("/tmp/loong.toml", &server);

    assert_eq!(payload["config"], "/tmp/loong.toml");
    assert_eq!(payload["server"]["name"], "docs");
    assert_eq!(payload["server"]["status"]["kind"], "pending");
    assert_eq!(payload["server"]["transport"]["transport"], "stdio");
    assert_eq!(payload["server"]["enabled_tools"][0], "search");
}
