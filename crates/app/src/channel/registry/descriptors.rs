use super::bridge::{
    ONEBOT_CHANNEL_REGISTRY_DESCRIPTOR, WEIXIN_CHANNEL_REGISTRY_DESCRIPTOR,
    WHATSAPP_PERSONAL_CHANNEL_REGISTRY_DESCRIPTOR,
};
use super::nostr_impl::{NOSTR_ONBOARDING_DESCRIPTOR, NOSTR_OPERATIONS, build_nostr_snapshots};
use super::planned::{
    WEBCHAT_CHANNEL_REGISTRY_DESCRIPTOR, ZALO_CHANNEL_REGISTRY_DESCRIPTOR,
    ZALO_PERSONAL_CHANNEL_REGISTRY_DESCRIPTOR,
};
use super::tlon::TLON_CHANNEL_REGISTRY_DESCRIPTOR;
use super::twitch::{TWITCH_ONBOARDING_DESCRIPTOR, TWITCH_OPERATIONS, build_twitch_snapshots};
use super::*;

pub(super) const TELEGRAM_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor =
    ChannelRegistryDescriptor {
        id: "telegram",
        runtime: Some(ChannelRuntimeDescriptor {
            family: TELEGRAM_COMMAND_FAMILY_DESCRIPTOR,
        }),
        snapshot_builder: Some(build_telegram_snapshots),
        selection_order: 10,
        selection_label: "personal and group chat bot",
        blurb: "Shipped Telegram Bot API surface with direct send and reply-loop runtime support.",
        implementation_status: ChannelCatalogImplementationStatus::RuntimeBacked,
        capabilities: TELEGRAM_CAPABILITIES,
        label: "Telegram",
        aliases: &[],
        transport: "telegram_bot_api_polling",
        onboarding: TELEGRAM_ONBOARDING_DESCRIPTOR,
        operations: TELEGRAM_OPERATIONS,
    };

pub(super) const FEISHU_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor =
    ChannelRegistryDescriptor {
        id: "feishu",
        runtime: Some(ChannelRuntimeDescriptor {
            family: FEISHU_COMMAND_FAMILY_DESCRIPTOR,
        }),
        snapshot_builder: Some(build_feishu_snapshots),
        selection_order: 20,
        selection_label: "enterprise chat app",
        blurb: "Shipped Feishu/Lark app surface with webhook or websocket ingress and account-aware runtime state.",
        implementation_status: ChannelCatalogImplementationStatus::RuntimeBacked,
        capabilities: FEISHU_CAPABILITIES,
        label: "Feishu/Lark",
        aliases: &["lark"],
        transport: "feishu_openapi_webhook_or_websocket",
        onboarding: FEISHU_ONBOARDING_DESCRIPTOR,
        operations: FEISHU_OPERATIONS,
    };

pub(super) const MATRIX_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor =
    ChannelRegistryDescriptor {
        id: "matrix",
        runtime: Some(ChannelRuntimeDescriptor {
            family: MATRIX_COMMAND_FAMILY_DESCRIPTOR,
        }),
        snapshot_builder: Some(build_matrix_snapshots),
        selection_order: 30,
        selection_label: "federated room sync bot",
        blurb: "Shipped Matrix surface with direct send and sync-based reply-loop support.",
        implementation_status: ChannelCatalogImplementationStatus::RuntimeBacked,
        capabilities: MATRIX_CAPABILITIES,
        label: "Matrix",
        aliases: &[],
        transport: "matrix_client_server_sync",
        onboarding: MATRIX_ONBOARDING_DESCRIPTOR,
        operations: MATRIX_OPERATIONS,
    };

pub(super) const WECOM_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor =
    ChannelRegistryDescriptor {
        id: "wecom",
        runtime: Some(ChannelRuntimeDescriptor {
            family: WECOM_COMMAND_FAMILY_DESCRIPTOR,
        }),
        snapshot_builder: Some(build_wecom_snapshots),
        selection_order: 35,
        selection_label: "enterprise aibot",
        blurb: "Shipped WeCom AIBot long-connection surface with proactive send and account-aware runtime state.",
        implementation_status: ChannelCatalogImplementationStatus::RuntimeBacked,
        capabilities: WECOM_CAPABILITIES,
        label: "WeCom",
        aliases: &["wechat-work", "qywx"],
        transport: "wecom_aibot_long_connection",
        onboarding: WECOM_ONBOARDING_DESCRIPTOR,
        operations: WECOM_OPERATIONS,
    };

pub(super) const DISCORD_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor =
    ChannelRegistryDescriptor {
        id: "discord",
        runtime: None,
        snapshot_builder: Some(build_discord_snapshots),
        selection_order: 40,
        selection_label: "community server bot",
        blurb: "Shipped Discord outbound message surface with config-backed direct sends; inbound gateway/runtime support remains planned.",
        implementation_status: ChannelCatalogImplementationStatus::ConfigBacked,
        capabilities: CONFIG_BACKED_SEND_CHANNEL_CAPABILITIES,
        label: "Discord",
        aliases: &["discord-bot"],
        transport: "discord_http_api",
        onboarding: DISCORD_ONBOARDING_DESCRIPTOR,
        operations: DISCORD_OPERATIONS,
    };

pub(super) const SLACK_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor =
    ChannelRegistryDescriptor {
        id: "slack",
        runtime: None,
        snapshot_builder: Some(build_slack_snapshots),
        selection_order: 50,
        selection_label: "workspace event bot",
        blurb: "Shipped Slack outbound message surface with config-backed direct sends; inbound Events API or Socket Mode support remains planned.",
        implementation_status: ChannelCatalogImplementationStatus::ConfigBacked,
        capabilities: CONFIG_BACKED_SEND_CHANNEL_CAPABILITIES,
        label: "Slack",
        aliases: &["slack-bot"],
        transport: "slack_web_api",
        onboarding: SLACK_ONBOARDING_DESCRIPTOR,
        operations: SLACK_OPERATIONS,
    };

pub(super) const LINE_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor =
    ChannelRegistryDescriptor {
        id: "line",
        runtime: Some(ChannelRuntimeDescriptor {
            family: LINE_COMMAND_FAMILY_DESCRIPTOR,
        }),
        snapshot_builder: Some(build_line_snapshots),
        selection_order: 60,
        selection_label: "consumer messaging bot",
        blurb: "Shipped LINE Messaging API surface with push sends and signed webhook reply-loop runtime support.",
        implementation_status: ChannelCatalogImplementationStatus::RuntimeBacked,
        capabilities: LINE_CAPABILITIES,
        label: "LINE",
        aliases: &["line-bot"],
        transport: "line_messaging_api",
        onboarding: LINE_ONBOARDING_DESCRIPTOR,
        operations: LINE_OPERATIONS,
    };

pub(super) const WHATSAPP_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor =
    ChannelRegistryDescriptor {
        id: "whatsapp",
        runtime: Some(ChannelRuntimeDescriptor {
            family: WHATSAPP_COMMAND_FAMILY_DESCRIPTOR,
        }),
        snapshot_builder: Some(build_whatsapp_snapshots),
        selection_order: 90,
        selection_label: "business messaging app",
        blurb: "Shipped WhatsApp Cloud API surface with business send and webhook serve runtime support.",
        implementation_status: ChannelCatalogImplementationStatus::RuntimeBacked,
        capabilities: WHATSAPP_CAPABILITIES,
        label: "WhatsApp",
        aliases: &["wa", "whatsapp-cloud"],
        transport: "whatsapp_cloud_api",
        onboarding: WHATSAPP_ONBOARDING_DESCRIPTOR,
        operations: WHATSAPP_OPERATIONS,
    };

pub(super) const SIGNAL_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor =
    ChannelRegistryDescriptor {
        id: "signal",
        runtime: None,
        snapshot_builder: Some(build_signal_snapshots),
        selection_order: 130,
        selection_label: "private messenger bridge",
        blurb: "Shipped Signal bridge outbound surface with config-backed direct sends; inbound listener support remains planned.",
        implementation_status: ChannelCatalogImplementationStatus::ConfigBacked,
        capabilities: CONFIG_BACKED_SEND_CHANNEL_CAPABILITIES,
        label: "Signal",
        aliases: &["signal-cli"],
        transport: "signal_cli_rest_api",
        onboarding: SIGNAL_ONBOARDING_DESCRIPTOR,
        operations: SIGNAL_OPERATIONS,
    };

pub(super) const TWITCH_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor =
    ChannelRegistryDescriptor {
        id: "twitch",
        runtime: None,
        snapshot_builder: Some(build_twitch_snapshots),
        selection_order: 135,
        selection_label: "livestream chat bot",
        blurb: "Shipped Twitch outbound surface with config-backed chat sends via the Twitch Chat API; inbound EventSub or chat-listener support remains planned.",
        implementation_status: ChannelCatalogImplementationStatus::ConfigBacked,
        capabilities: CONFIG_BACKED_SEND_CHANNEL_CAPABILITIES,
        label: "Twitch",
        aliases: &["tmi"],
        transport: "twitch_chat_api",
        onboarding: TWITCH_ONBOARDING_DESCRIPTOR,
        operations: TWITCH_OPERATIONS,
    };

pub(super) const MATTERMOST_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor =
    ChannelRegistryDescriptor {
        id: "mattermost",
        runtime: None,
        snapshot_builder: Some(build_mattermost_snapshots),
        selection_order: 150,
        selection_label: "self-hosted workspace bot",
        blurb: "Shipped Mattermost outbound surface with config-backed post sends; inbound websocket serve support remains planned.",
        implementation_status: ChannelCatalogImplementationStatus::ConfigBacked,
        capabilities: CONFIG_BACKED_SEND_CHANNEL_CAPABILITIES,
        label: "Mattermost",
        aliases: &["mm"],
        transport: "mattermost_rest_api",
        onboarding: MATTERMOST_ONBOARDING_DESCRIPTOR,
        operations: MATTERMOST_OPERATIONS,
    };

const DINGTALK_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor = ChannelRegistryDescriptor {
    id: "dingtalk",
    runtime: None,
    snapshot_builder: Some(build_dingtalk_snapshots),
    selection_order: 80,
    selection_label: "group webhook bot",
    blurb: "Shipped DingTalk custom robot outbound surface with config-backed webhook sends; inbound callback serve support remains planned.",
    implementation_status: ChannelCatalogImplementationStatus::ConfigBacked,
    capabilities: CONFIG_BACKED_SEND_CHANNEL_CAPABILITIES,
    label: "DingTalk",
    aliases: &["ding", "ding-bot"],
    transport: "dingtalk_custom_robot_webhook",
    onboarding: DINGTALK_ONBOARDING_DESCRIPTOR,
    operations: DINGTALK_OPERATIONS,
};

const EMAIL_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor = ChannelRegistryDescriptor {
    id: "email",
    runtime: None,
    snapshot_builder: Some(build_email_snapshots),
    selection_order: 100,
    selection_label: "mailbox agent",
    blurb: "Shipped email SMTP outbound surface with config-backed plain-text sends; IMAP-backed reply-loop serve support remains planned.",
    implementation_status: ChannelCatalogImplementationStatus::ConfigBacked,
    capabilities: CONFIG_BACKED_SEND_CHANNEL_CAPABILITIES,
    label: "Email",
    aliases: &["smtp", "imap"],
    transport: "smtp_imap",
    onboarding: EMAIL_ONBOARDING_DESCRIPTOR,
    operations: EMAIL_OPERATIONS,
};

const WEBHOOK_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor = ChannelRegistryDescriptor {
    id: "webhook",
    runtime: Some(ChannelRuntimeDescriptor {
        family: WEBHOOK_COMMAND_FAMILY_DESCRIPTOR,
    }),
    snapshot_builder: Some(build_webhook_snapshots),
    selection_order: 110,
    selection_label: "generic http integration",
    blurb: "Shipped generic webhook surface with outbound POST delivery and signed inbound webhook serve support.",
    implementation_status: ChannelCatalogImplementationStatus::RuntimeBacked,
    capabilities: WEBHOOK_CAPABILITIES,
    label: "Webhook",
    aliases: &["http-webhook"],
    transport: "generic_webhook",
    onboarding: WEBHOOK_ONBOARDING_DESCRIPTOR,
    operations: WEBHOOK_OPERATIONS,
};

const GOOGLE_CHAT_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor =
    ChannelRegistryDescriptor {
        id: "google-chat",
        runtime: None,
        snapshot_builder: Some(build_google_chat_snapshots),
        selection_order: 120,
        selection_label: "workspace space webhook",
        blurb: "Shipped Google Chat outbound surface with config-backed incoming-webhook sends; interactive event serve support remains planned.",
        implementation_status: ChannelCatalogImplementationStatus::ConfigBacked,
        capabilities: CONFIG_BACKED_SEND_CHANNEL_CAPABILITIES,
        label: "Google Chat",
        aliases: &["gchat", "googlechat"],
        transport: "google_chat_incoming_webhook",
        onboarding: GOOGLE_CHAT_ONBOARDING_DESCRIPTOR,
        operations: GOOGLE_CHAT_OPERATIONS,
    };

const TEAMS_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor = ChannelRegistryDescriptor {
    id: "teams",
    runtime: None,
    snapshot_builder: Some(build_teams_snapshots),
    selection_order: 140,
    selection_label: "workspace webhook bot",
    blurb: "Shipped Microsoft Teams outbound surface with config-backed incoming-webhook sends; bot-framework serve support remains planned.",
    implementation_status: ChannelCatalogImplementationStatus::ConfigBacked,
    capabilities: CONFIG_BACKED_SEND_CHANNEL_CAPABILITIES,
    label: "Microsoft Teams",
    aliases: &["msteams", "ms-teams"],
    transport: "microsoft_teams_incoming_webhook",
    onboarding: TEAMS_ONBOARDING_DESCRIPTOR,
    operations: TEAMS_OPERATIONS,
};

const NEXTCLOUD_TALK_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor =
    ChannelRegistryDescriptor {
        id: "nextcloud-talk",
        runtime: None,
        snapshot_builder: Some(build_nextcloud_talk_snapshots),
        selection_order: 160,
        selection_label: "self-hosted room bot",
        blurb: "Shipped Nextcloud Talk bot outbound surface with config-backed room sends; inbound callback serve support remains planned.",
        implementation_status: ChannelCatalogImplementationStatus::ConfigBacked,
        capabilities: CONFIG_BACKED_SEND_CHANNEL_CAPABILITIES,
        label: "Nextcloud Talk",
        aliases: &["nextcloud", "nextcloudtalk"],
        transport: "nextcloud_talk_bot_api",
        onboarding: NEXTCLOUD_TALK_ONBOARDING_DESCRIPTOR,
        operations: NEXTCLOUD_TALK_OPERATIONS,
    };

const SYNOLOGY_CHAT_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor =
    ChannelRegistryDescriptor {
        id: "synology-chat",
        runtime: None,
        snapshot_builder: Some(build_synology_chat_snapshots),
        selection_order: 165,
        selection_label: "nas webhook bot",
        blurb: "Shipped Synology Chat outbound surface with config-backed incoming-webhook sends; inbound outgoing-webhook serve support remains planned.",
        implementation_status: ChannelCatalogImplementationStatus::ConfigBacked,
        capabilities: CONFIG_BACKED_SEND_CHANNEL_CAPABILITIES,
        label: "Synology Chat",
        aliases: &["synologychat", "synochat"],
        transport: "synology_chat_outgoing_incoming_webhooks",
        onboarding: SYNOLOGY_CHAT_ONBOARDING_DESCRIPTOR,
        operations: SYNOLOGY_CHAT_OPERATIONS,
    };

const IRC_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor = ChannelRegistryDescriptor {
    id: "irc",
    runtime: None,
    snapshot_builder: Some(build_irc_snapshots),
    selection_order: 170,
    selection_label: "relay and channel bot",
    blurb: "Shipped IRC outbound surface with config-backed sends for channels or direct nick targets; relay-loop serve support remains planned.",
    implementation_status: ChannelCatalogImplementationStatus::ConfigBacked,
    capabilities: CONFIG_BACKED_SEND_CHANNEL_CAPABILITIES,
    label: "IRC",
    aliases: &[],
    transport: "irc_socket",
    onboarding: IRC_ONBOARDING_DESCRIPTOR,
    operations: IRC_OPERATIONS,
};

const IMESSAGE_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor = ChannelRegistryDescriptor {
    id: "imessage",
    runtime: None,
    snapshot_builder: Some(build_imessage_snapshots),
    selection_order: 180,
    selection_label: "apple message bridge",
    blurb: "Shipped BlueBubbles-backed iMessage outbound surface with config-backed chat sends; inbound bridge sync support remains planned.",
    implementation_status: ChannelCatalogImplementationStatus::ConfigBacked,
    capabilities: CONFIG_BACKED_SEND_CHANNEL_CAPABILITIES,
    label: "iMessage",
    aliases: &["bluebubbles", "blue-bubbles"],
    transport: "imessage_bridge_api",
    onboarding: IMESSAGE_ONBOARDING_DESCRIPTOR,
    operations: IMESSAGE_OPERATIONS,
};

const NOSTR_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor = ChannelRegistryDescriptor {
    id: "nostr",
    runtime: None,
    snapshot_builder: Some(build_nostr_snapshots),
    selection_order: 190,
    selection_label: "relay-signed social bot",
    blurb: "Shipped Nostr outbound surface for signed relay publication; inbound subscriptions and relay runtime support remain planned.",
    implementation_status: ChannelCatalogImplementationStatus::ConfigBacked,
    capabilities: CONFIG_BACKED_SEND_CHANNEL_CAPABILITIES,
    label: "Nostr",
    aliases: &[],
    transport: "nostr_relays",
    onboarding: NOSTR_ONBOARDING_DESCRIPTOR,
    operations: NOSTR_OPERATIONS,
};

pub(crate) const QQBOT_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor =
    ChannelRegistryDescriptor {
        id: "qqbot",
        runtime: Some(ChannelRuntimeDescriptor {
            family: QQBOT_COMMAND_FAMILY_DESCRIPTOR,
        }),
        snapshot_builder: Some(build_qqbot_snapshots),
        selection_order: 36,
        selection_label: "qq gateway bot",
        blurb: "Shipped QQBot official gateway surface with WebSocket ingress and C2C message reply-loop runtime support.",
        implementation_status: ChannelCatalogImplementationStatus::RuntimeBacked,
        capabilities: QQBOT_CAPABILITIES,
        label: "QQ Bot",
        aliases: &["qq", "qq-bot", "tencent-qq"],
        transport: "qq_official_bot_gateway",
        onboarding: QQBOT_ONBOARDING_DESCRIPTOR,
        operations: QQBOT_OPERATIONS,
    };

pub(super) const CHANNEL_REGISTRY: &[ChannelRegistryDescriptor] = &[
    TELEGRAM_CHANNEL_REGISTRY_DESCRIPTOR,
    FEISHU_CHANNEL_REGISTRY_DESCRIPTOR,
    MATRIX_CHANNEL_REGISTRY_DESCRIPTOR,
    WECOM_CHANNEL_REGISTRY_DESCRIPTOR,
    WEIXIN_CHANNEL_REGISTRY_DESCRIPTOR,
    QQBOT_CHANNEL_REGISTRY_DESCRIPTOR,
    ONEBOT_CHANNEL_REGISTRY_DESCRIPTOR,
    WHATSAPP_PERSONAL_CHANNEL_REGISTRY_DESCRIPTOR,
    DISCORD_CHANNEL_REGISTRY_DESCRIPTOR,
    SLACK_CHANNEL_REGISTRY_DESCRIPTOR,
    LINE_CHANNEL_REGISTRY_DESCRIPTOR,
    DINGTALK_CHANNEL_REGISTRY_DESCRIPTOR,
    WHATSAPP_CHANNEL_REGISTRY_DESCRIPTOR,
    EMAIL_CHANNEL_REGISTRY_DESCRIPTOR,
    WEBHOOK_CHANNEL_REGISTRY_DESCRIPTOR,
    GOOGLE_CHAT_CHANNEL_REGISTRY_DESCRIPTOR,
    SIGNAL_CHANNEL_REGISTRY_DESCRIPTOR,
    TWITCH_CHANNEL_REGISTRY_DESCRIPTOR,
    TEAMS_CHANNEL_REGISTRY_DESCRIPTOR,
    MATTERMOST_CHANNEL_REGISTRY_DESCRIPTOR,
    NEXTCLOUD_TALK_CHANNEL_REGISTRY_DESCRIPTOR,
    SYNOLOGY_CHAT_CHANNEL_REGISTRY_DESCRIPTOR,
    IRC_CHANNEL_REGISTRY_DESCRIPTOR,
    IMESSAGE_CHANNEL_REGISTRY_DESCRIPTOR,
    NOSTR_CHANNEL_REGISTRY_DESCRIPTOR,
    TLON_CHANNEL_REGISTRY_DESCRIPTOR,
    ZALO_CHANNEL_REGISTRY_DESCRIPTOR,
    ZALO_PERSONAL_CHANNEL_REGISTRY_DESCRIPTOR,
    WEBCHAT_CHANNEL_REGISTRY_DESCRIPTOR,
];
