use super::*;

#[test]
fn normalize_channel_catalog_id_maps_runtime_and_stub_aliases() {
    assert_eq!(normalize_channel_catalog_id("lark"), Some("feishu"));
    assert_eq!(normalize_channel_catalog_id(" TELEGRAM "), Some("telegram"));
    assert_eq!(normalize_channel_catalog_id("discord-bot"), Some("discord"));
    assert_eq!(normalize_channel_catalog_id("slack"), Some("slack"));
    assert_eq!(normalize_channel_catalog_id("gchat"), Some("google-chat"));
    assert_eq!(
        normalize_channel_catalog_id("synochat"),
        Some("synology-chat")
    );
    assert_eq!(
        normalize_channel_catalog_id("bluebubbles"),
        Some("imessage")
    );
    assert_eq!(normalize_channel_catalog_id("urbit"), Some("tlon"));
    assert_eq!(normalize_channel_catalog_id("web-ui"), Some("webchat"));
    assert_eq!(normalize_channel_catalog_id("unknown"), None);
}

#[test]
fn resolve_channel_catalog_command_family_descriptor_includes_runtime_and_stub_channels() {
    let telegram = resolve_channel_catalog_command_family_descriptor("telegram")
        .expect("telegram catalog command family");
    let lark = resolve_channel_catalog_command_family_descriptor("lark")
        .expect("lark catalog command family");
    let slack = resolve_channel_catalog_command_family_descriptor("slack-bot")
        .expect("slack alias catalog command family");
    let google_chat = resolve_channel_catalog_command_family_descriptor("gchat")
        .expect("google chat alias catalog command family");
    let synology_chat = resolve_channel_catalog_command_family_descriptor("synochat")
        .expect("synology chat alias catalog command family");
    let irc = resolve_channel_catalog_command_family_descriptor("irc").expect("irc catalog family");
    let imessage = resolve_channel_catalog_command_family_descriptor("bluebubbles")
        .expect("imessage alias catalog command family");
    let tlon = resolve_channel_catalog_command_family_descriptor("urbit")
        .expect("tlon alias catalog command family");

    assert_eq!(telegram.channel_id, "telegram");
    assert_eq!(telegram.send.id, CHANNEL_OPERATION_SEND_ID);
    assert_eq!(telegram.send.command, "channels send telegram");
    assert_eq!(telegram.serve.id, CHANNEL_OPERATION_SERVE_ID);
    assert_eq!(telegram.serve.command, "channels serve telegram");
    assert_eq!(
        telegram.default_send_target_kind,
        ChannelCatalogTargetKind::Conversation
    );

    assert_eq!(lark.channel_id, "feishu");
    assert_eq!(lark.send.command, "feishu send");
    assert_eq!(lark.serve.command, "feishu serve");
    assert_eq!(
        lark.default_send_target_kind,
        ChannelCatalogTargetKind::ReceiveId
    );

    assert_eq!(slack.channel_id, "slack");
    assert_eq!(slack.send.command, "slack-send");
    assert_eq!(slack.serve.command, "slack-serve");
    assert_eq!(
        slack.default_send_target_kind,
        ChannelCatalogTargetKind::Conversation
    );

    assert_eq!(google_chat.channel_id, "google-chat");
    assert_eq!(google_chat.send.command, "channels send google-chat");
    assert_eq!(google_chat.serve.command, "google-chat-serve");
    assert_eq!(
        google_chat.default_send_target_kind,
        ChannelCatalogTargetKind::Endpoint
    );

    assert_eq!(synology_chat.channel_id, "synology-chat");
    assert_eq!(synology_chat.send.command, "channels send synology-chat");
    assert_eq!(synology_chat.serve.command, "synology-chat-serve");
    assert_eq!(
        synology_chat.default_send_target_kind,
        ChannelCatalogTargetKind::Address
    );

    assert_eq!(irc.channel_id, "irc");
    assert_eq!(irc.send.command, "channels send irc");
    assert_eq!(irc.serve.command, "irc-serve");
    assert_eq!(
        irc.default_send_target_kind,
        ChannelCatalogTargetKind::Conversation
    );

    assert_eq!(imessage.channel_id, "imessage");
    assert_eq!(imessage.send.command, "channels send imessage");
    assert_eq!(imessage.serve.command, "imessage-serve");
    assert_eq!(
        imessage.default_send_target_kind,
        ChannelCatalogTargetKind::Conversation
    );

    assert_eq!(tlon.channel_id, "tlon");
    assert_eq!(tlon.send.command, "channels send tlon");
    assert_eq!(tlon.serve.command, "tlon-serve");
    assert_eq!(
        tlon.default_send_target_kind,
        ChannelCatalogTargetKind::Conversation
    );
}

#[test]
fn channel_catalog_includes_openclaw_inspired_extended_surfaces() {
    let catalog = list_channel_catalog();
    let signal = catalog
        .iter()
        .find(|entry| entry.id == "signal")
        .expect("signal catalog entry");
    let twitch = catalog
        .iter()
        .find(|entry| entry.id == "twitch")
        .expect("twitch catalog entry");
    let teams = catalog
        .iter()
        .find(|entry| entry.id == "teams")
        .expect("teams catalog entry");
    let synology_chat = catalog
        .iter()
        .find(|entry| entry.id == "synology-chat")
        .expect("synology chat catalog entry");
    let imessage = catalog
        .iter()
        .find(|entry| entry.id == "imessage")
        .expect("imessage catalog entry");
    let tlon = catalog
        .iter()
        .find(|entry| entry.id == "tlon")
        .expect("tlon catalog entry");
    let webchat = catalog
        .iter()
        .find(|entry| entry.id == "webchat")
        .expect("webchat catalog entry");

    assert_eq!(
        signal.supported_target_kinds,
        vec![ChannelCatalogTargetKind::Address]
    );
    assert_eq!(signal.operations[0].command, "channels send signal");
    assert_eq!(signal.operations[1].command, "signal-serve");

    assert_eq!(
        twitch.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(twitch.selection_order, 135);
    assert_eq!(twitch.aliases, vec!["tmi"]);
    assert_eq!(twitch.transport, "twitch_chat_api");
    assert!(
        twitch.blurb.contains("Twitch Chat API"),
        "unexpected twitch blurb: {}",
        twitch.blurb
    );
    assert_eq!(
        twitch.supported_target_kinds,
        vec![ChannelCatalogTargetKind::Conversation]
    );
    assert_eq!(twitch.operations[0].command, "channels send twitch");
    assert_eq!(twitch.operations[1].command, "twitch-serve");
    assert_eq!(
        twitch.operations[0].availability,
        ChannelCatalogOperationAvailability::Implemented
    );
    assert_eq!(
        twitch.operations[1].availability,
        ChannelCatalogOperationAvailability::Stub
    );

    assert_eq!(
        teams.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(teams.selection_order, 140);
    assert_eq!(teams.aliases, vec!["msteams", "ms-teams"]);
    assert_eq!(teams.transport, "microsoft_teams_incoming_webhook");
    assert_eq!(
        teams.supported_target_kinds,
        vec![
            ChannelCatalogTargetKind::Endpoint,
            ChannelCatalogTargetKind::Conversation,
        ]
    );
    assert_eq!(teams.operations[0].command, "channels send teams");
    assert_eq!(teams.operations[1].command, "teams-serve");
    assert_eq!(
        teams.operations[0].availability,
        ChannelCatalogOperationAvailability::Implemented
    );
    assert_eq!(
        teams.operations[1].availability,
        ChannelCatalogOperationAvailability::Stub
    );

    assert_eq!(
        synology_chat.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(synology_chat.selection_order, 165);
    assert_eq!(synology_chat.aliases, vec!["synologychat", "synochat"]);
    assert_eq!(
        synology_chat.transport,
        "synology_chat_outgoing_incoming_webhooks"
    );
    assert_eq!(
        synology_chat.supported_target_kinds,
        vec![ChannelCatalogTargetKind::Address]
    );
    assert_eq!(
        synology_chat.operations[0].command,
        "channels send synology-chat"
    );
    assert_eq!(synology_chat.operations[1].command, "synology-chat-serve");
    assert_eq!(
        synology_chat.operations[0].availability,
        ChannelCatalogOperationAvailability::Implemented
    );
    assert_eq!(
        synology_chat.operations[1].availability,
        ChannelCatalogOperationAvailability::Stub
    );

    assert_eq!(
        imessage.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(imessage.aliases, vec!["bluebubbles", "blue-bubbles"]);
    assert_eq!(imessage.selection_order, 180);
    assert_eq!(imessage.transport, "imessage_bridge_api");
    assert!(imessage.blurb.contains("BlueBubbles"));
    assert_eq!(
        imessage.supported_target_kinds,
        vec![ChannelCatalogTargetKind::Conversation]
    );
    assert_eq!(imessage.operations[0].command, "channels send imessage");
    assert_eq!(imessage.operations[1].command, "imessage-serve");
    assert_eq!(
        imessage.operations[0].availability,
        ChannelCatalogOperationAvailability::Implemented
    );
    assert_eq!(
        imessage.operations[1].availability,
        ChannelCatalogOperationAvailability::Stub
    );

    assert_eq!(
        tlon.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(tlon.selection_order, 205);
    assert_eq!(tlon.aliases, vec!["urbit"]);
    assert_eq!(tlon.transport, "tlon_urbit_ship_api");
    assert_eq!(
        tlon.supported_target_kinds,
        vec![ChannelCatalogTargetKind::Conversation]
    );
    assert_eq!(tlon.operations[0].command, "channels send tlon");
    assert_eq!(tlon.operations[1].command, "tlon-serve");
    assert_eq!(
        tlon.operations[0].availability,
        ChannelCatalogOperationAvailability::Implemented
    );
    assert_eq!(
        tlon.operations[1].availability,
        ChannelCatalogOperationAvailability::Stub
    );
    assert_eq!(
        tlon.onboarding.strategy,
        ChannelOnboardingStrategy::ManualConfig
    );
    assert_eq!(tlon.onboarding.status_command, "loong doctor");
    assert_eq!(tlon.onboarding.repair_command, Some("loong doctor --fix"));

    assert_eq!(webchat.selection_order, 230);
    assert_eq!(webchat.aliases, vec!["browser-chat", "web-ui"]);
    assert_eq!(webchat.transport, "webchat_websocket");
    assert_eq!(
        webchat.supported_target_kinds,
        vec![ChannelCatalogTargetKind::Conversation]
    );
}

#[test]
fn channel_inventory_combines_runtime_and_catalog_surfaces() {
    let config = LoongConfig::default();
    let inventory = channel_inventory(&config);

    assert_eq!(
        inventory
            .channels
            .iter()
            .map(|snapshot| snapshot.id)
            .collect::<Vec<_>>(),
        vec![
            "telegram",
            "feishu",
            "matrix",
            "wecom",
            "qqbot",
            "weixin",
            "onebot",
            "discord",
            "slack",
            "line",
            "dingtalk",
            "whatsapp",
            "email",
            "webhook",
            "google-chat",
            "signal",
            "twitch",
            "teams",
            "mattermost",
            "nextcloud-talk",
            "synology-chat",
            "irc",
            "imessage",
            "nostr",
            "tlon",
        ]
    );
    assert_eq!(
        inventory
            .catalog_only_channels
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        vec!["zalo", "zalo-personal", "webchat"]
    );
    assert_eq!(
        inventory
            .channel_catalog
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        vec![
            "telegram",
            "feishu",
            "matrix",
            "wecom",
            "qqbot",
            "weixin",
            "onebot",
            "discord",
            "slack",
            "line",
            "dingtalk",
            "whatsapp",
            "email",
            "webhook",
            "google-chat",
            "signal",
            "twitch",
            "teams",
            "mattermost",
            "nextcloud-talk",
            "synology-chat",
            "irc",
            "imessage",
            "nostr",
            "tlon",
            "zalo",
            "zalo-personal",
            "webchat",
        ]
    );

    let nostr = inventory
        .channel_catalog
        .iter()
        .find(|entry| entry.id == "nostr")
        .expect("nostr catalog entry");
    assert_eq!(
        nostr.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(
        nostr.capabilities,
        vec![ChannelCapability::MultiAccount, ChannelCapability::Send]
    );
    assert_eq!(
        nostr
            .operations
            .iter()
            .map(|operation| operation.availability)
            .collect::<Vec<_>>(),
        vec![
            ChannelCatalogOperationAvailability::Implemented,
            ChannelCatalogOperationAvailability::Stub,
        ]
    );
}

#[test]
fn channel_inventory_exposes_grouped_channel_surfaces() {
    let mut env = crate::test_support::ScopedEnv::new();
    env.remove("TELEGRAM_BOT_TOKEN");

    let config = LoongConfig::default();
    let inventory = channel_inventory(&config);

    assert_eq!(
        inventory
            .channel_surfaces
            .iter()
            .map(|surface| surface.catalog.id)
            .collect::<Vec<_>>(),
        vec![
            "telegram",
            "feishu",
            "matrix",
            "wecom",
            "qqbot",
            "weixin",
            "onebot",
            "discord",
            "slack",
            "line",
            "dingtalk",
            "whatsapp",
            "email",
            "webhook",
            "google-chat",
            "signal",
            "twitch",
            "teams",
            "mattermost",
            "nextcloud-talk",
            "synology-chat",
            "irc",
            "imessage",
            "nostr",
            "tlon",
            "zalo",
            "zalo-personal",
            "webchat",
        ]
    );

    let telegram = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "telegram")
        .expect("telegram surface");
    assert_eq!(telegram.configured_accounts.len(), 1);
    assert_eq!(
        telegram.default_configured_account_id.as_deref(),
        Some("default")
    );
    assert_eq!(telegram.configured_accounts[0].id, "telegram");

    let discord = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "discord")
        .expect("discord surface");
    assert_eq!(
        discord.catalog.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(discord.configured_accounts.len(), 1);
    assert_eq!(
        discord.default_configured_account_id.as_deref(),
        Some("default")
    );
    assert_eq!(discord.configured_accounts[0].id, "discord");

    let line = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "line")
        .expect("line surface");
    assert_eq!(
        line.catalog.implementation_status,
        ChannelCatalogImplementationStatus::RuntimeBacked
    );
    assert_eq!(line.configured_accounts.len(), 1);
    assert_eq!(
        line.default_configured_account_id.as_deref(),
        Some("default")
    );
    assert_eq!(line.configured_accounts[0].id, "line");

    let wecom = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "wecom")
        .expect("wecom surface");
    assert_eq!(
        wecom.catalog.implementation_status,
        ChannelCatalogImplementationStatus::RuntimeBacked
    );
    assert_eq!(wecom.configured_accounts.len(), 1);
    assert_eq!(
        wecom.default_configured_account_id.as_deref(),
        Some("default")
    );
    assert_eq!(wecom.configured_accounts[0].id, "wecom");

    let mattermost = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "mattermost")
        .expect("mattermost surface");
    assert_eq!(
        mattermost.catalog.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(mattermost.configured_accounts.len(), 1);
    assert_eq!(
        mattermost.default_configured_account_id.as_deref(),
        Some("default")
    );
    assert_eq!(mattermost.configured_accounts[0].id, "mattermost");

    let teams = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "teams")
        .expect("teams surface");
    assert_eq!(
        teams.catalog.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(teams.configured_accounts.len(), 1);
    assert_eq!(
        teams.default_configured_account_id.as_deref(),
        Some("default")
    );
    assert_eq!(teams.configured_accounts[0].id, "teams");

    let synology_chat = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "synology-chat")
        .expect("synology chat surface");
    assert_eq!(
        synology_chat.catalog.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(synology_chat.configured_accounts.len(), 1);
    assert_eq!(
        synology_chat.default_configured_account_id.as_deref(),
        Some("default")
    );
    assert_eq!(synology_chat.configured_accounts[0].id, "synology-chat");

    let imessage = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "imessage")
        .expect("imessage surface");
    assert_eq!(
        imessage.catalog.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(imessage.configured_accounts.len(), 1);
    assert_eq!(
        imessage.default_configured_account_id.as_deref(),
        Some("default")
    );
    assert_eq!(imessage.configured_accounts[0].id, "imessage");

    let nostr = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "nostr")
        .expect("nostr surface");
    assert_eq!(
        nostr.catalog.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(nostr.configured_accounts.len(), 1);
    assert_eq!(
        nostr.default_configured_account_id.as_deref(),
        Some("default")
    );
    assert_eq!(nostr.configured_accounts[0].id, "nostr");

    let twitch = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "twitch")
        .expect("twitch surface");
    assert_eq!(
        twitch.catalog.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(twitch.configured_accounts.len(), 1);
    assert_eq!(
        twitch.default_configured_account_id.as_deref(),
        Some("default")
    );
    assert_eq!(twitch.configured_accounts[0].id, "twitch");

    let tlon = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "tlon")
        .expect("tlon surface");
    assert_eq!(
        tlon.catalog.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(tlon.configured_accounts.len(), 1);
    assert_eq!(
        tlon.default_configured_account_id.as_deref(),
        Some("default")
    );
    assert_eq!(tlon.configured_accounts[0].id, "tlon");

    let webchat = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "webchat")
        .expect("webchat surface");
    assert_eq!(
        webchat.catalog.implementation_status,
        ChannelCatalogImplementationStatus::Stub
    );
    assert_eq!(webchat.catalog.aliases, vec!["browser-chat", "web-ui"]);
    assert!(webchat.configured_accounts.is_empty());
}
