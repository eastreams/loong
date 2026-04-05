use super::Commands;

#[test]
fn command_kind_for_logging_uses_stable_variant_names() {
    assert_eq!(Commands::Welcome.command_kind_for_logging(), "welcome");
    assert_eq!(Commands::AuditDemo.command_kind_for_logging(), "audit_demo");
    assert_eq!(
        Commands::RunTask {
            objective: "test".to_owned(),
            payload: "{}".to_owned(),
        }
        .command_kind_for_logging(),
        "run_task"
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
}
