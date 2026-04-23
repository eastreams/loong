use super::Commands;

#[test]
fn command_kind_for_logging_uses_stable_variant_names() {
    assert_eq!(Commands::Welcome.command_kind_for_logging(), "welcome");
    assert_eq!(Commands::AuditDemo.command_kind_for_logging(), "audit_demo");
    assert_eq!(
        Commands::Turn {
            command: crate::TurnCommands::Run {
                config: None,
                session: None,
                message: "test".to_owned(),
                acp: false,
                acp_event_stream: false,
                acp_bootstrap_mcp_server: Vec::new(),
                acp_cwd: None,
            },
        }
        .command_kind_for_logging(),
        "turn_run"
    );
    assert_eq!(
        Commands::Channels {
            config: None,
            resolve: None,
            json: false,
            command: Some(crate::ChannelsCommands::List(
                crate::channels_cli::ChannelsListArgs {
                    config: None,
                    json: false,
                },
            )),
        }
        .command_kind_for_logging(),
        "channels"
    );
    assert_eq!(
        Commands::Runtime {
            command: crate::runtime_cli::RuntimeCommands::Snapshot(
                crate::runtime_cli::RuntimeSnapshotArgs {
                    config: None,
                    json: false,
                    output: None,
                    label: None,
                    experiment_id: None,
                    parent_snapshot_id: None,
                },
            ),
        }
        .command_kind_for_logging(),
        "runtime"
    );
    assert_eq!(
        Commands::Gateway {
            command: crate::gateway::service::GatewayCommand::Status { json: false },
        }
        .command_kind_for_logging(),
        "gateway"
    );
    assert_eq!(
        Commands::Feishu {
            command: crate::feishu_cli::FeishuCommand::Serve(crate::feishu_cli::FeishuServeArgs {
                common: crate::feishu_cli::FeishuCommonArgs {
                    config: None,
                    account: None,
                    json: false,
                },
                bind: None,
                path: None,
            }),
        }
        .command_kind_for_logging(),
        "feishu"
    );
}
