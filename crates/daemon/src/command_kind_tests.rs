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
            bind: Some("127.0.0.1:9998".to_owned()),
            path: None,
        }
        .command_kind_for_logging(),
        "line_serve"
    );
    assert_eq!(
        Commands::WebhookServe {
            config: None,
            account: None,
            bind: Some("127.0.0.1:9999".to_owned()),
            path: None,
        }
        .command_kind_for_logging(),
        "webhook_serve"
    );
    assert_eq!(
        Commands::WeixinSend {
            config: None,
            account: None,
            target: "weixin:default:contact:wxid_alice".to_owned(),
            target_kind: crate::mvp::channel::ChannelOutboundTargetKind::Conversation,
            text: "hello".to_owned(),
        }
        .command_kind_for_logging(),
        "weixin_send"
    );
    assert_eq!(
        Commands::WeixinServe {
            config: None,
            once: false,
            account: None,
        }
        .command_kind_for_logging(),
        "weixin_serve"
    );
    assert_eq!(
        Commands::QqbotSend {
            config: None,
            account: None,
            target: "qqbot:default:group:123".to_owned(),
            target_kind: crate::mvp::channel::ChannelOutboundTargetKind::Conversation,
            text: "hello".to_owned(),
        }
        .command_kind_for_logging(),
        "qqbot_send"
    );
    assert_eq!(
        Commands::QqbotServe {
            config: None,
            once: false,
            account: None,
        }
        .command_kind_for_logging(),
        "qqbot_serve"
    );
    assert_eq!(
        Commands::OnebotSend {
            config: None,
            account: None,
            target: "onebot:default:user:10001".to_owned(),
            target_kind: crate::mvp::channel::ChannelOutboundTargetKind::Conversation,
            text: "hello".to_owned(),
        }
        .command_kind_for_logging(),
        "onebot_send"
    );
    assert_eq!(
        Commands::OnebotServe {
            config: None,
            once: false,
            account: None,
        }
        .command_kind_for_logging(),
        "onebot_serve"
    );
}
