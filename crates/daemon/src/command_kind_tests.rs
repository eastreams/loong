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
        Commands::ListMcpServers {
            config: None,
            json: false,
        }
        .command_kind_for_logging(),
        "list_mcp_servers"
    );
    assert_eq!(
        Commands::ShowMcpServer {
            config: None,
            name: "test".to_owned(),
            json: false,
        }
        .command_kind_for_logging(),
        "show_mcp_server"
    );
    assert_eq!(
        Commands::WhatsappServe {
            config: None,
            account: None,
            bind: None,
            path: None,
        }
        .command_kind_for_logging(),
        "whatsapp_serve"
    );
    assert_eq!(
        Commands::LineServe {
            config: None,
            account: None,
            bind: "127.0.0.1:9998".to_owned(),
            path: None,
        }
        .command_kind_for_logging(),
        "line_serve"
    );
    assert_eq!(
        Commands::WebhookServe {
            config: None,
            account: None,
            bind: "127.0.0.1:9999".to_owned(),
            path: None,
        }
        .command_kind_for_logging(),
        "webhook_serve"
    );
}
