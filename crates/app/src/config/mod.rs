mod audit;
mod channels;
mod conversation;
mod feishu_integration;
mod irc;
mod memory;
mod outbound_http;
mod provider;
mod runtime;
mod shared;
mod tools;

#[allow(unused_imports)]
pub use crate::channel::{ChannelDescriptor, ChannelOperationalModel, ChannelRuntimeKind};
#[allow(unused_imports)]
pub use crate::channel::{
    catalog_only_channel_descriptors, channel_descriptor, gateway_ingress_channel_descriptors,
    gateway_supervised_channel_descriptors, outbound_only_channel_descriptors,
    plugin_backed_channel_descriptors, runtime_backed_channel_descriptors,
    service_channel_descriptors, standalone_runtime_channel_descriptors,
};
pub use crate::mcp::{McpConfig, McpServerConfig, McpServerTransportConfig};
#[allow(unused_imports)]
pub use audit::{AuditConfig, AuditMode};
#[allow(unused_imports)]
pub use channels::bridge::{
    OnebotAccountConfig, OnebotChannelConfig, ResolvedOnebotChannelConfig,
    ResolvedWeixinChannelConfig, ResolvedWhatsappPersonalChannelConfig, WeixinAccountConfig,
    WeixinChannelConfig, WhatsappPersonalAccountConfig, WhatsappPersonalChannelConfig,
};
#[allow(unused_imports)]
pub use channels::{
    ChannelAccountIdentity, ChannelAccountIdentitySource, ChannelAcpConfig,
    ChannelDefaultAccountSelection, ChannelDefaultAccountSelectionSource,
    ChannelResolvedAccountRoute, CliChannelConfig, DingtalkAccountConfig, DingtalkChannelConfig,
    DiscordAccountConfig, DiscordChannelConfig, EmailAccountConfig, EmailChannelConfig,
    FeishuAccountConfig, FeishuChannelConfig, FeishuChannelServeMode, FeishuDomain,
    GoogleChatAccountConfig, GoogleChatChannelConfig, ImessageAccountConfig, ImessageChannelConfig,
    IrcAccountConfig, IrcChannelConfig, LineAccountConfig, LineChannelConfig, MatrixAccountConfig,
    MatrixChannelConfig, MattermostAccountConfig, MattermostChannelConfig,
    NextcloudTalkAccountConfig, NextcloudTalkChannelConfig, NostrAccountConfig, NostrChannelConfig,
    QqbotAccountConfig, QqbotChannelConfig, ResolvedDingtalkChannelConfig,
    ResolvedDiscordChannelConfig, ResolvedEmailChannelConfig, ResolvedFeishuChannelConfig,
    ResolvedGoogleChatChannelConfig, ResolvedImessageChannelConfig, ResolvedIrcChannelConfig,
    ResolvedLineChannelConfig, ResolvedMatrixChannelConfig, ResolvedMattermostChannelConfig,
    ResolvedNextcloudTalkChannelConfig, ResolvedNostrChannelConfig, ResolvedQqbotChannelConfig,
    ResolvedSignalChannelConfig, ResolvedSlackChannelConfig, ResolvedSynologyChatChannelConfig,
    ResolvedTeamsChannelConfig, ResolvedTelegramChannelConfig, ResolvedTlonChannelConfig,
    ResolvedTwitchChannelConfig, ResolvedWebhookChannelConfig, ResolvedWecomChannelConfig,
    ResolvedWhatsappChannelConfig, SignalAccountConfig, SignalChannelConfig, SlackAccountConfig,
    SlackChannelConfig, SynologyChatAccountConfig, SynologyChatChannelConfig, TeamsAccountConfig,
    TeamsChannelConfig, TelegramAccountConfig, TelegramChannelConfig, TelegramStreamingMode,
    TlonAccountConfig, TlonChannelConfig, TwitchAccountConfig, TwitchChannelConfig,
    WebhookAccountConfig, WebhookChannelConfig, WebhookPayloadFormat, WecomAccountConfig,
    WecomChannelConfig, WhatsappAccountConfig, WhatsappChannelConfig, normalize_channel_account_id,
};
#[allow(unused_imports)]
pub(crate) use channels::{
    DINGTALK_SECRET_ENV, DINGTALK_WEBHOOK_URL_ENV, DISCORD_BOT_TOKEN_ENV, EMAIL_IMAP_PASSWORD_ENV,
    EMAIL_IMAP_USERNAME_ENV, EMAIL_SMTP_PASSWORD_ENV, EMAIL_SMTP_USERNAME_ENV, EmailSmtpEndpoint,
    FEISHU_APP_ID_ENV, FEISHU_APP_SECRET_ENV, FEISHU_ENCRYPT_KEY_ENV,
    FEISHU_VERIFICATION_TOKEN_ENV, GOOGLE_CHAT_WEBHOOK_URL_ENV, IMESSAGE_BRIDGE_TOKEN_ENV,
    IMESSAGE_BRIDGE_URL_ENV, LINE_CHANNEL_ACCESS_TOKEN_ENV, LINE_CHANNEL_SECRET_ENV,
    MATRIX_ACCESS_TOKEN_ENV, MATTERMOST_BOT_TOKEN_ENV, MATTERMOST_SERVER_URL_ENV,
    NEXTCLOUD_TALK_SERVER_URL_ENV, NEXTCLOUD_TALK_SHARED_SECRET_ENV, NOSTR_PRIVATE_KEY_ENV,
    NOSTR_RELAY_URLS_ENV, ONEBOT_ACCESS_TOKEN_ENV, ONEBOT_WEBSOCKET_URL_ENV, QQBOT_APP_ID_ENV,
    QQBOT_CLIENT_SECRET_ENV, SIGNAL_ACCOUNT_ENV, SIGNAL_SERVICE_URL_ENV, SLACK_BOT_TOKEN_ENV,
    SYNOLOGY_CHAT_INCOMING_URL_ENV, SYNOLOGY_CHAT_TOKEN_ENV, TEAMS_APP_ID_ENV,
    TEAMS_APP_PASSWORD_ENV, TEAMS_TENANT_ID_ENV, TEAMS_WEBHOOK_URL_ENV, TELEGRAM_BOT_TOKEN_ENV,
    TLON_CODE_ENV, TLON_SHIP_ENV, TLON_URL_ENV, TWITCH_ACCESS_TOKEN_ENV, WEBHOOK_AUTH_TOKEN_ENV,
    WEBHOOK_ENDPOINT_URL_ENV, WEBHOOK_SIGNING_SECRET_ENV, WECOM_BOT_ID_ENV, WECOM_SECRET_ENV,
    WEIXIN_BRIDGE_ACCESS_TOKEN_ENV, WEIXIN_BRIDGE_URL_ENV, WHATSAPP_ACCESS_TOKEN_ENV,
    WHATSAPP_APP_SECRET_ENV, WHATSAPP_PERSONAL_AUTH_DIR_ENV, WHATSAPP_PERSONAL_BRIDGE_URL_ENV,
    WHATSAPP_PHONE_NUMBER_ID_ENV, WHATSAPP_VERIFY_TOKEN_ENV, parse_email_smtp_endpoint,
    parse_nostr_private_key_hex, parse_nostr_public_key_hex,
};
#[allow(unused_imports)]
pub use conversation::{ConversationConfig, ConversationTurnLoopConfig};
pub use feishu_integration::{FeishuCapabilityConfig, FeishuIntegrationConfig};
pub(crate) use irc::{
    IRC_NICKNAME_ENV, IRC_SERVER_ENV, IrcServerEndpoint, IrcServerTransport,
    parse_irc_server_endpoint,
};
#[allow(unused_imports)]
pub use memory::{
    InitiativeLevel, MemoryBackendKind, MemoryConfig, MemoryIngestMode, MemoryMode, MemoryProfile,
    MemorySystemKind, PersonalizationConfig, PersonalizationPromptState, ResponseDensity,
};
#[allow(unused_imports)]
pub use outbound_http::OutboundHttpConfig;
pub(crate) use provider::{
    ANTHROPIC_DEFAULT_HEADERS, GITHUB_COPILOT_DEFAULT_HEADERS, GITHUB_COPILOT_USER_AGENT,
};
#[allow(unused_imports)]
pub use provider::{
    ModelCatalogProbeRecovery, PROVIDER_DESCRIPTOR_SCHEMA_VERSION, ProviderAuthScheme,
    ProviderConfig, ProviderDescriptorAuth, ProviderDescriptorDocument, ProviderDescriptorFeature,
    ProviderDescriptorHeader, ProviderDescriptorRegionEndpoint, ProviderDescriptorRegionVariant,
    ProviderDescriptorSchema, ProviderFeatureFamily, ProviderKind, ProviderProfileConfig,
    ProviderProfileHealthModeConfig, ProviderProfileStateBackendKind, ProviderProtocolFamily,
    ProviderReasoningExtraBodyModeConfig, ProviderToolSchemaModeConfig, ProviderTransportFallback,
    ProviderTransportPolicy, ProviderTransportReadiness, ProviderTransportReadinessLevel,
    ProviderWireApi, ReasoningEffort, parse_provider_kind_id,
};
#[cfg(test)]
pub(crate) use runtime::inject_test_config_write_failure;
#[allow(unused_imports)]
pub use runtime::{
    AcpBackendProfilesConfig, AcpConfig, AcpConversationRoutingMode, AcpDispatchConfig,
    AcpDispatchThreadRoutingMode, AcpxBackendConfig, AcpxMcpServerConfig,
    ConfigValidationDiagnostic, ControlPlaneConfig, GatewayConfig, LoongConfig,
    PROVIDER_SELECTOR_COMPACT_NOTE, PROVIDER_SELECTOR_HUMAN_SUMMARY, PROVIDER_SELECTOR_NOTE,
    PROVIDER_SELECTOR_PLACEHOLDER, PROVIDER_SELECTOR_TARGET_SUMMARY, ProviderSelectorProfileRef,
    ProviderSelectorResolution, accepted_provider_selectors, default_config_path,
    default_loong_home, describe_provider_selector_target, load, normalize_validation_locale,
    preferred_provider_selector, provider_selector_catalog, provider_selector_recommendation_hint,
    render, resolve_provider_selector, supported_validation_locales, validate_file,
    validate_file_with_locale, write, write_template,
};
pub(crate) use runtime::{normalize_dispatch_account_id, normalize_dispatch_channel_id};
pub(crate) use shared::ConfigValidationIssue;
#[allow(unused_imports)]
pub use shared::{
    CLI_COMMAND_NAME, HOME_DIR_NAME, LEGACY_CLI_COMMAND_NAME, LEGACY_HOME_DIR_NAME,
    PRODUCT_DISPLAY_NAME, active_cli_command_name, detect_invoked_cli_command_name,
    detect_invoked_cli_command_name_from_arg0, detect_legacy_home, expand_path,
    set_active_cli_command_name,
};
pub(crate) use shared::{
    pop_default_loong_home_env_override_for_tests, push_default_loong_home_env_override_for_tests,
};
#[cfg(test)]
pub(crate) use shared::{
    pop_default_loong_home_override_for_tests, push_default_loong_home_override_for_tests,
};
#[allow(unused_imports)]
pub use tools::{
    AUTONOMY_PROFILE_VALID_VALUES, AutonomyProfile, BrowserCompanionToolConfig, BrowserToolConfig,
    DEFAULT_BROWSER_COMPANION_TIMEOUT_SECONDS, DEFAULT_BROWSER_MAX_LINKS,
    DEFAULT_BROWSER_MAX_SESSIONS, DEFAULT_BROWSER_MAX_TEXT_CHARS,
    DEFAULT_EXTERNAL_SKILLS_BLOCKED_DOMAIN_RULES, DEFAULT_RUNTIME_SELF_MAX_SOURCE_CHARS,
    DEFAULT_RUNTIME_SELF_MAX_TOTAL_CHARS, DEFAULT_SHELL_ALLOW, DEFAULT_WEB_FETCH_MAX_BYTES,
    DEFAULT_WEB_FETCH_MAX_REDIRECTS, DEFAULT_WEB_FETCH_TIMEOUT_SECONDS,
    DEFAULT_WEB_SEARCH_MAX_RESULTS, DEFAULT_WEB_SEARCH_PROVIDER,
    DEFAULT_WEB_SEARCH_TIMEOUT_SECONDS, DelegateToolConfig, ExternalSkillsConfig,
    GovernedToolApprovalConfig, GovernedToolApprovalMode, MAX_BROWSER_MAX_LINKS,
    MAX_BROWSER_MAX_SESSIONS, MAX_BROWSER_MAX_TEXT_CHARS, MAX_RUNTIME_SELF_MAX_SOURCE_CHARS,
    MAX_RUNTIME_SELF_MAX_TOTAL_CHARS, MAX_WEB_FETCH_MAX_BYTES, RuntimePluginsConfig,
    RuntimeSelfToolConfig, SessionVisibility, ToolConfig, ToolConsentConfig, ToolConsentMode,
    ToolFileRootResolution, WEB_SEARCH_BRAVE_API_KEY_ENV, WEB_SEARCH_EXA_API_KEY_ENV,
    WEB_SEARCH_FIRECRAWL_API_KEY_ENV, WEB_SEARCH_JINA_API_KEY_ENV, WEB_SEARCH_JINA_AUTH_TOKEN_ENV,
    WEB_SEARCH_PERPLEXITY_API_KEY_ENV, WEB_SEARCH_PROVIDER_BRAVE, WEB_SEARCH_PROVIDER_DUCKDUCKGO,
    WEB_SEARCH_PROVIDER_EXA, WEB_SEARCH_PROVIDER_FIRECRAWL, WEB_SEARCH_PROVIDER_JINA,
    WEB_SEARCH_PROVIDER_PERPLEXITY, WEB_SEARCH_PROVIDER_TAVILY, WEB_SEARCH_PROVIDER_VALID_VALUES,
    WEB_SEARCH_TAVILY_API_KEY_ENV, WebSearchProviderDescriptor, WebSearchToolConfig, WebToolConfig,
    normalize_web_search_provider, parse_autonomy_profile, web_search_provider_api_key_env_names,
    web_search_provider_default_api_key_env, web_search_provider_descriptor,
    web_search_provider_descriptors,
};
pub(crate) use tools::{MIN_RUNTIME_SELF_MAX_SOURCE_CHARS, MIN_RUNTIME_SELF_MAX_TOTAL_CHARS};
#[cfg(feature = "tool-websearch")]
pub(crate) use tools::{
    WEB_SEARCH_PROVIDER_SCHEMA_VALUES, web_search_provider_parameter_description,
};

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use loong_contracts::SecretRef;

    use super::*;
    use crate::test_support::{ScopedEnv, ScopedLoongHome};
    use std::collections::BTreeSet;

    fn clear_config_test_secret_envs(env: &mut ScopedEnv) {
        for key in [
            "LOONG_TEST_API_KEY_REF",
            "LOONG_TEST_MISSING_API_KEY",
            "LOONG_TEST_LEGACY_FALLBACK",
            "LOONG_TEST_TYPED_SECRET_REF",
            "LOONG_TEST_TELEGRAM_SECRET_REF",
        ] {
            env.remove(key);
        }
    }

    fn expected_service_channel_ids() -> Vec<&'static str> {
        vec![
            "telegram", "feishu", "matrix", "wecom", "qqbot", "line", "whatsapp", "webhook",
        ]
    }

    fn expected_gateway_supervised_channel_ids() -> Vec<&'static str> {
        vec!["telegram", "feishu", "matrix", "wecom", "qqbot", "whatsapp"]
    }

    fn expected_gateway_ingress_channel_ids() -> Vec<&'static str> {
        vec!["feishu", "line", "whatsapp", "webhook"]
    }

    fn expected_standalone_runtime_channel_ids() -> Vec<&'static str> {
        vec!["line", "webhook"]
    }

    fn expected_plugin_backed_channel_ids() -> Vec<&'static str> {
        vec!["weixin", "onebot", "whatsapp-personal"]
    }

    fn expected_outbound_channel_ids() -> Vec<&'static str> {
        vec![
            "discord",
            "slack",
            "dingtalk",
            "email",
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
    }

    #[test]
    fn endpoint_resolution_for_openai_compatible_is_stable() {
        let config = ProviderConfig {
            base_url: "https://api.openai.com/".to_owned(),
            chat_completions_path: "/v1/chat/completions".to_owned(),
            ..ProviderConfig::default()
        };
        assert_eq!(
            config.endpoint(),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn channel_descriptor_lookup_reports_shared_metadata() {
        let cli = channel_descriptor("cli").expect("cli descriptor");
        assert_eq!(cli.id, "cli");
        assert_eq!(cli.surface_label, "cli channel");
        assert_eq!(cli.runtime_kind, ChannelRuntimeKind::Interactive);
        assert_eq!(cli.serve_subcommand, None);

        let telegram = channel_descriptor("telegram").expect("telegram descriptor");
        assert_eq!(telegram.id, "telegram");
        assert_eq!(telegram.surface_label, "telegram channel");
        assert_eq!(telegram.runtime_kind, ChannelRuntimeKind::RuntimeBacked);
        assert_eq!(
            telegram.operational_model,
            ChannelOperationalModel::GatewaySupervised
        );
        assert_eq!(telegram.serve_subcommand, Some("channels serve telegram"));

        let feishu = channel_descriptor("feishu").expect("feishu descriptor");
        assert_eq!(feishu.id, "feishu");
        assert_eq!(feishu.surface_label, "feishu channel");
        assert_eq!(feishu.runtime_kind, ChannelRuntimeKind::RuntimeBacked);
        assert_eq!(
            feishu.operational_model,
            ChannelOperationalModel::GatewaySupervised
        );
        assert_eq!(feishu.serve_subcommand, Some("feishu serve"));

        let wecom = channel_descriptor("wecom").expect("wecom descriptor");
        assert_eq!(wecom.id, "wecom");
        assert_eq!(wecom.surface_label, "wecom channel");
        assert_eq!(wecom.runtime_kind, ChannelRuntimeKind::RuntimeBacked);
        assert_eq!(
            wecom.operational_model,
            ChannelOperationalModel::GatewaySupervised
        );
        assert_eq!(wecom.serve_subcommand, Some("channels serve wecom"));

        let weixin = channel_descriptor("wechat").expect("weixin descriptor");
        assert_eq!(weixin.id, "weixin");
        assert_eq!(weixin.surface_label, "weixin channel");
        assert_eq!(weixin.runtime_kind, ChannelRuntimeKind::PluginBacked);
        assert_eq!(
            weixin.operational_model,
            ChannelOperationalModel::PluginBacked
        );
        assert_eq!(weixin.serve_subcommand, Some("channels serve weixin"));

        let qqbot = channel_descriptor("qq").expect("qqbot descriptor");
        assert_eq!(qqbot.id, "qqbot");
        assert_eq!(qqbot.surface_label, "qq bot channel");
        assert_eq!(qqbot.runtime_kind, ChannelRuntimeKind::RuntimeBacked);
        assert_eq!(
            qqbot.operational_model,
            ChannelOperationalModel::GatewaySupervised
        );
        assert_eq!(qqbot.serve_subcommand, Some("qqbot-serve"));

        let onebot = channel_descriptor("onebot-v11").expect("onebot descriptor");
        assert_eq!(onebot.id, "onebot");
        assert_eq!(onebot.surface_label, "onebot channel");
        assert_eq!(onebot.runtime_kind, ChannelRuntimeKind::PluginBacked);
        assert_eq!(
            onebot.operational_model,
            ChannelOperationalModel::PluginBacked
        );
        assert_eq!(onebot.serve_subcommand, Some("channels serve onebot"));

        let discord = channel_descriptor("discord").expect("discord descriptor");
        assert_eq!(discord.id, "discord");
        assert_eq!(discord.surface_label, "discord channel");
        assert_eq!(discord.runtime_kind, ChannelRuntimeKind::OutboundOnly);
        assert_eq!(
            discord.operational_model,
            ChannelOperationalModel::OutboundOnly
        );
        assert_eq!(discord.serve_subcommand, None);

        let slack = channel_descriptor("slack").expect("slack descriptor");
        assert_eq!(slack.id, "slack");
        assert_eq!(slack.surface_label, "slack channel");
        assert_eq!(slack.runtime_kind, ChannelRuntimeKind::OutboundOnly);
        assert_eq!(
            slack.operational_model,
            ChannelOperationalModel::OutboundOnly
        );
        assert_eq!(slack.serve_subcommand, None);

        let line = channel_descriptor("line").expect("line descriptor");
        assert_eq!(line.id, "line");
        assert_eq!(line.surface_label, "line channel");
        assert_eq!(line.runtime_kind, ChannelRuntimeKind::RuntimeBacked);
        assert_eq!(
            line.operational_model,
            ChannelOperationalModel::StandaloneRuntime
        );
        assert_eq!(line.serve_subcommand, Some("channels serve line"));

        let dingtalk = channel_descriptor("dingtalk").expect("dingtalk descriptor");
        assert_eq!(dingtalk.id, "dingtalk");
        assert_eq!(dingtalk.surface_label, "dingtalk channel");
        assert_eq!(dingtalk.runtime_kind, ChannelRuntimeKind::OutboundOnly);
        assert_eq!(dingtalk.serve_subcommand, None);

        let whatsapp = channel_descriptor("whatsapp").expect("whatsapp descriptor");
        assert_eq!(whatsapp.id, "whatsapp");
        assert_eq!(whatsapp.surface_label, "whatsapp channel");
        assert_eq!(whatsapp.runtime_kind, ChannelRuntimeKind::RuntimeBacked);
        assert_eq!(
            whatsapp.operational_model,
            ChannelOperationalModel::GatewaySupervised
        );
        assert_eq!(whatsapp.serve_subcommand, Some("channels serve whatsapp"));

        let email = channel_descriptor("email").expect("email descriptor");
        assert_eq!(email.id, "email");
        assert_eq!(email.surface_label, "email channel");
        assert_eq!(email.runtime_kind, ChannelRuntimeKind::OutboundOnly);
        assert_eq!(email.serve_subcommand, None);

        let webhook = channel_descriptor("webhook").expect("webhook descriptor");
        assert_eq!(webhook.id, "webhook");
        assert_eq!(webhook.surface_label, "webhook channel");
        assert_eq!(webhook.runtime_kind, ChannelRuntimeKind::RuntimeBacked);
        assert_eq!(
            webhook.operational_model,
            ChannelOperationalModel::StandaloneRuntime
        );
        assert_eq!(webhook.serve_subcommand, Some("channels serve webhook"));

        let google_chat = channel_descriptor("google-chat").expect("google chat descriptor");
        assert_eq!(google_chat.id, "google-chat");
        assert_eq!(google_chat.surface_label, "google chat channel");
        assert_eq!(google_chat.runtime_kind, ChannelRuntimeKind::OutboundOnly);
        assert_eq!(google_chat.serve_subcommand, None);

        let signal = channel_descriptor("signal").expect("signal descriptor");
        assert_eq!(signal.id, "signal");
        assert_eq!(signal.surface_label, "signal channel");
        assert_eq!(signal.runtime_kind, ChannelRuntimeKind::OutboundOnly);
        assert_eq!(signal.serve_subcommand, None);

        let irc = channel_descriptor("irc").expect("irc descriptor");
        assert_eq!(irc.id, "irc");
        assert_eq!(irc.surface_label, "irc channel");
        assert_eq!(irc.runtime_kind, ChannelRuntimeKind::OutboundOnly);
        assert_eq!(irc.serve_subcommand, None);

        let twitch = channel_descriptor("twitch").expect("twitch descriptor");
        assert_eq!(twitch.id, "twitch");
        assert_eq!(twitch.surface_label, "twitch channel");
        assert_eq!(twitch.runtime_kind, ChannelRuntimeKind::OutboundOnly);
        assert_eq!(twitch.serve_subcommand, None);

        let teams = channel_descriptor("teams").expect("teams descriptor");
        assert_eq!(teams.id, "teams");
        assert_eq!(teams.surface_label, "teams channel");
        assert_eq!(teams.runtime_kind, ChannelRuntimeKind::OutboundOnly);
        assert_eq!(teams.serve_subcommand, None);

        let mattermost = channel_descriptor("mattermost").expect("mattermost descriptor");
        assert_eq!(mattermost.id, "mattermost");
        assert_eq!(mattermost.surface_label, "mattermost channel");
        assert_eq!(mattermost.runtime_kind, ChannelRuntimeKind::OutboundOnly);
        assert_eq!(mattermost.serve_subcommand, None);

        let nextcloud_talk =
            channel_descriptor("nextcloud-talk").expect("nextcloud talk descriptor");
        assert_eq!(nextcloud_talk.id, "nextcloud-talk");
        assert_eq!(nextcloud_talk.surface_label, "nextcloud talk channel");
        assert_eq!(
            nextcloud_talk.runtime_kind,
            ChannelRuntimeKind::OutboundOnly
        );
        assert_eq!(nextcloud_talk.serve_subcommand, None);

        let synology_chat = channel_descriptor("synology-chat").expect("synology chat descriptor");
        assert_eq!(synology_chat.id, "synology-chat");
        assert_eq!(synology_chat.surface_label, "synology chat channel");
        assert_eq!(synology_chat.runtime_kind, ChannelRuntimeKind::OutboundOnly);
        assert_eq!(synology_chat.serve_subcommand, None);

        let imessage = channel_descriptor("imessage").expect("imessage descriptor");
        assert_eq!(imessage.id, "imessage");
        assert_eq!(imessage.surface_label, "imessage channel");
        assert_eq!(imessage.runtime_kind, ChannelRuntimeKind::OutboundOnly);
        assert_eq!(imessage.serve_subcommand, None);

        assert!(channel_descriptor("unknown").is_none());
    }

    #[test]
    fn enabled_channel_views_follow_shared_catalog_order() {
        let default_config = LoongConfig::default();
        assert_eq!(default_config.enabled_channel_ids(), vec!["cli"]);
        assert!(default_config.enabled_service_channel_ids().is_empty());
        let mut config = LoongConfig::default();
        let expected_service_ids = expected_service_channel_ids();
        let expected_plugin_backed_ids = expected_plugin_backed_channel_ids();
        let expected_outbound_ids = expected_outbound_channel_ids();
        let expected_enabled_service_ids = expected_service_ids
            .iter()
            .map(|channel_id| (*channel_id).to_owned())
            .collect::<Vec<_>>();
        let mut expected_enabled_channel_ids = vec!["cli".to_owned()];
        expected_enabled_channel_ids.extend(expected_enabled_service_ids.iter().cloned());
        expected_enabled_channel_ids.extend(
            expected_plugin_backed_ids
                .iter()
                .map(|channel_id| (*channel_id).to_owned()),
        );
        expected_enabled_channel_ids.extend(
            expected_outbound_ids
                .iter()
                .map(|channel_id| (*channel_id).to_owned()),
        );

        config.telegram.enabled = true;
        config.feishu.enabled = true;
        config.matrix.enabled = true;
        config.wecom.enabled = true;
        config.weixin.enabled = true;
        config.qqbot.enabled = true;
        config.onebot.enabled = true;
        config.whatsapp_personal.enabled = true;
        config.discord.enabled = true;
        config.slack.enabled = true;
        config.line.enabled = true;
        config.dingtalk.enabled = true;
        config.whatsapp.enabled = true;
        config.email.enabled = true;
        config.webhook.enabled = true;
        config.google_chat.enabled = true;
        config.signal.enabled = true;
        config.twitch.enabled = true;
        config.teams.enabled = true;
        config.mattermost.enabled = true;
        config.nextcloud_talk.enabled = true;
        config.synology_chat.enabled = true;
        config.irc.enabled = true;
        config.imessage.enabled = true;
        config.nostr.enabled = true;
        config.tlon.enabled = true;

        assert_eq!(config.enabled_channel_ids(), expected_enabled_channel_ids);
        assert_eq!(
            config.enabled_service_channel_ids(),
            expected_enabled_service_ids
        );

        let service_ids = service_channel_descriptors()
            .into_iter()
            .map(|descriptor| descriptor.id)
            .collect::<Vec<_>>();
        assert_eq!(service_ids, expected_service_ids);
    }

    #[test]
    fn channel_descriptor_lookup_reports_matrix_metadata() {
        let matrix = channel_descriptor("matrix").expect("matrix descriptor");
        assert_eq!(matrix.id, "matrix");
        assert_eq!(matrix.surface_label, "matrix channel");
        assert_eq!(matrix.runtime_kind, ChannelRuntimeKind::RuntimeBacked);
        assert_eq!(matrix.serve_subcommand, Some("channels serve matrix"));
    }

    #[test]
    fn service_channel_descriptors_follow_registry_selection_order() {
        let service_ids = service_channel_descriptors()
            .into_iter()
            .map(|descriptor| descriptor.id)
            .collect::<Vec<_>>();
        assert_eq!(service_ids, expected_service_channel_ids());
    }

    #[test]
    fn channel_descriptor_helper_groups_follow_runtime_taxonomy() {
        let runtime_backed_ids = runtime_backed_channel_descriptors()
            .into_iter()
            .map(|descriptor| descriptor.id)
            .collect::<Vec<_>>();
        let gateway_supervised_ids = gateway_supervised_channel_descriptors()
            .into_iter()
            .map(|descriptor| descriptor.id)
            .collect::<Vec<_>>();
        let gateway_ingress_ids = gateway_ingress_channel_descriptors()
            .into_iter()
            .map(|descriptor| descriptor.id)
            .collect::<Vec<_>>();
        let standalone_runtime_ids = standalone_runtime_channel_descriptors()
            .into_iter()
            .map(|descriptor| descriptor.id)
            .collect::<Vec<_>>();
        let plugin_backed_ids = plugin_backed_channel_descriptors()
            .into_iter()
            .map(|descriptor| descriptor.id)
            .collect::<Vec<_>>();
        let outbound_only_ids = outbound_only_channel_descriptors()
            .into_iter()
            .map(|descriptor| descriptor.id)
            .collect::<Vec<_>>();
        let catalog_only_ids = catalog_only_channel_descriptors()
            .into_iter()
            .map(|descriptor| descriptor.id)
            .collect::<Vec<_>>();

        assert_eq!(runtime_backed_ids, expected_service_channel_ids());
        assert_eq!(
            gateway_supervised_ids,
            expected_gateway_supervised_channel_ids()
        );
        assert_eq!(gateway_ingress_ids, expected_gateway_ingress_channel_ids());
        assert_eq!(
            standalone_runtime_ids,
            expected_standalone_runtime_channel_ids()
        );
        assert_eq!(plugin_backed_ids, expected_plugin_backed_channel_ids());
        assert_eq!(outbound_only_ids, expected_outbound_channel_ids());
        assert_eq!(catalog_only_ids, vec!["zalo", "zalo-personal", "webchat"]);
    }

    #[test]
    fn enabled_channel_runtime_groups_follow_runtime_taxonomy() {
        let mut config = LoongConfig::default();
        config.telegram.enabled = true;
        config.weixin.enabled = true;
        config.discord.enabled = true;

        assert_eq!(
            config.enabled_runtime_backed_channel_ids(),
            vec!["telegram".to_owned()]
        );
        assert_eq!(
            config.enabled_service_channel_ids(),
            vec!["telegram".to_owned()]
        );
        assert_eq!(
            config.enabled_plugin_backed_channel_ids(),
            vec!["weixin".to_owned()]
        );
        assert_eq!(
            config.enabled_outbound_only_channel_ids(),
            vec!["discord".to_owned()]
        );
        assert!(config.enabled_catalog_only_channel_ids().is_empty());
    }

    #[test]
    fn endpoint_resolution_for_volcengine_prefers_explicit_endpoint() {
        let mut config = ProviderConfig {
            kind: ProviderKind::Volcengine,
            ..ProviderConfig::default()
        };
        config.set_endpoint(Some(
            "https://example.volcengine.com/chat/completions".to_owned(),
        ));
        assert_eq!(
            config.endpoint(),
            "https://example.volcengine.com/chat/completions"
        );
    }

    #[test]
    fn provider_kinds_are_sorted_alphabetically() {
        let kinds = ProviderKind::all_sorted();
        let mut ids = Vec::new();
        for kind in kinds {
            ids.push(kind.as_str());
        }
        assert_eq!(
            ids,
            vec![
                "anthropic",
                "bailian_coding",
                "bedrock",
                "byteplus",
                "byteplus_coding",
                "cerebras",
                "cloudflare_ai_gateway",
                "cohere",
                "custom",
                "deepseek",
                "fireworks",
                "gemini",
                "github-copilot",
                "groq",
                "kimi",
                "kimi_coding",
                "llamacpp",
                "lm_studio",
                "mistral",
                "minimax",
                "novita",
                "nvidia",
                "ollama",
                "openai",
                "opencode_go",
                "opencode_zen",
                "openrouter",
                "perplexity",
                "qianfan",
                "qwen",
                "sambanova",
                "sglang",
                "siliconflow",
                "stepfun",
                "step_plan",
                "together",
                "venice",
                "vercel_ai_gateway",
                "vllm",
                "volcengine",
                "volcengine_coding",
                "xai",
                "xiaomi",
                "zai",
                "zhipu"
            ]
        );
        let unique = ids.iter().collect::<BTreeSet<_>>();
        assert_eq!(unique.len(), ids.len());
    }

    #[test]
    fn endpoint_resolution_for_supported_provider_profiles_is_stable() {
        let cases = vec![
            (
                ProviderKind::Anthropic,
                "https://api.anthropic.com/v1/messages",
            ),
            (
                ProviderKind::Kimi,
                "https://api.moonshot.cn/v1/chat/completions",
            ),
            (
                ProviderKind::KimiCoding,
                "https://api.kimi.com/coding/v1/chat/completions",
            ),
            (
                ProviderKind::Minimax,
                "https://api.minimaxi.com/v1/chat/completions",
            ),
            (
                ProviderKind::Ollama,
                "http://127.0.0.1:11434/v1/chat/completions",
            ),
            (
                ProviderKind::Openai,
                "https://api.openai.com/v1/chat/completions",
            ),
            (
                ProviderKind::OpencodeGo,
                "https://opencode.ai/zen/go/v1/chat/completions",
            ),
            (
                ProviderKind::OpencodeZen,
                "https://opencode.ai/zen/v1/chat/completions",
            ),
            (
                ProviderKind::Openrouter,
                "https://openrouter.ai/api/v1/chat/completions",
            ),
            (
                ProviderKind::Volcengine,
                "https://ark.cn-beijing.volces.com/api/v3/chat/completions",
            ),
            (ProviderKind::Xai, "https://api.x.ai/v1/chat/completions"),
            (
                ProviderKind::Xiaomi,
                "https://api.xiaomimimo.com/v1/chat/completions",
            ),
            (
                ProviderKind::Zai,
                "https://api.z.ai/api/paas/v4/chat/completions",
            ),
            (
                ProviderKind::Zhipu,
                "https://open.bigmodel.cn/api/paas/v4/chat/completions",
            ),
        ];
        for (kind, expected) in cases {
            let config = ProviderConfig {
                kind,
                ..ProviderConfig::default()
            };
            assert_eq!(config.endpoint(), expected, "kind={kind:?}");
        }
    }

    #[test]
    fn provider_kind_default_api_key_env_mapping_is_stable() {
        let cases = vec![
            (ProviderKind::Kimi, Some("MOONSHOT_API_KEY")),
            (ProviderKind::Minimax, Some("MINIMAX_API_KEY")),
            (ProviderKind::Openai, Some("OPENAI_API_KEY")),
            (ProviderKind::OpencodeGo, Some("OPENCODE_API_KEY")),
            (ProviderKind::OpencodeZen, Some("OPENCODE_API_KEY")),
            (ProviderKind::Xiaomi, Some("XIAOMI_API_KEY")),
        ];
        for (kind, expected) in cases {
            let config = ProviderConfig {
                kind,
                ..ProviderConfig::default()
            };
            assert_eq!(config.default_api_key_env().as_deref(), expected);
        }
    }

    #[test]
    fn provider_default_config_does_not_prepopulate_legacy_api_key_env_pointer() {
        let config = ProviderConfig::default();
        assert_eq!(config.api_key_env, None);
    }

    #[test]
    fn switching_provider_kind_uses_profile_defaults() {
        let config = ProviderConfig {
            kind: ProviderKind::Openrouter,
            ..ProviderConfig::default()
        };
        assert_eq!(
            config.endpoint(),
            "https://openrouter.ai/api/v1/chat/completions"
        );
        assert_eq!(
            config.default_api_key_env().as_deref(),
            Some("OPENROUTER_API_KEY")
        );
    }

    #[test]
    fn provider_display_names_remain_stable_for_tool_contracts() {
        assert_eq!(ProviderKind::Stepfun.display_name(), "StepFun");
        assert_eq!(ProviderKind::Zhipu.display_name(), "Zhipu");
    }

    #[test]
    fn kimi_coding_uses_native_profile_defaults() {
        let config = ProviderConfig {
            kind: ProviderKind::KimiCoding,
            ..ProviderConfig::default()
        };
        assert_eq!(
            config.endpoint(),
            "https://api.kimi.com/coding/v1/chat/completions"
        );
        assert_eq!(
            config.models_endpoint(),
            "https://api.kimi.com/coding/v1/models"
        );
        assert_eq!(
            config.default_api_key_env().as_deref(),
            Some("KIMI_CODING_API_KEY")
        );
        assert_eq!(config.resolved_model(), None);
        assert!(config.model_selection_requires_fetch());
    }

    #[test]
    fn switching_provider_kind_keeps_profile_defaults_with_partial_template_edits() {
        let with_empty_path = ProviderConfig {
            kind: ProviderKind::Volcengine,
            chat_completions_path: String::new(),
            ..ProviderConfig::default()
        };
        assert_eq!(
            with_empty_path.endpoint(),
            "https://ark.cn-beijing.volces.com/api/v3/chat/completions"
        );

        let with_empty_base = ProviderConfig {
            kind: ProviderKind::Volcengine,
            base_url: String::new(),
            ..ProviderConfig::default()
        };
        assert_eq!(
            with_empty_base.endpoint(),
            "https://ark.cn-beijing.volces.com/api/v3/chat/completions"
        );
    }

    #[test]
    fn model_catalog_cache_ttl_default_and_clamp_are_stable() {
        let default_cfg = ProviderConfig::default();
        assert_eq!(default_cfg.resolved_model_catalog_cache_ttl_ms(), 30_000);
        assert_eq!(
            default_cfg.resolved_model_catalog_stale_if_error_ms(),
            120_000
        );
        assert_eq!(default_cfg.resolved_model_catalog_cache_max_entries(), 32);
        assert_eq!(default_cfg.resolved_model_candidate_cooldown_ms(), 300_000);
        assert_eq!(
            default_cfg.resolved_model_candidate_cooldown_max_ms(),
            3_600_000
        );
        assert_eq!(
            default_cfg.resolved_model_candidate_cooldown_max_entries(),
            64
        );
        assert_eq!(default_cfg.resolved_profile_cooldown_ms(), 60_000);
        assert_eq!(default_cfg.resolved_profile_cooldown_max_ms(), 3_600_000);
        assert_eq!(
            default_cfg.resolved_profile_auth_reject_disable_ms(),
            21_600_000
        );
        assert_eq!(default_cfg.resolved_profile_state_max_entries(), 256);
        assert_eq!(
            default_cfg.resolved_profile_state_backend(),
            ProviderProfileStateBackendKind::File
        );
        assert_eq!(
            default_cfg.resolved_profile_health_mode_config(),
            ProviderProfileHealthModeConfig::ProviderDefault
        );
        assert_eq!(
            default_cfg.resolved_tool_schema_mode_config(),
            ProviderToolSchemaModeConfig::ProviderDefault
        );
        assert_eq!(
            default_cfg.resolved_reasoning_extra_body_mode_config(),
            ProviderReasoningExtraBodyModeConfig::ProviderDefault
        );
        assert!(
            default_cfg
                .resolved_tool_schema_disabled_model_hints()
                .is_empty()
        );
        assert!(
            default_cfg
                .resolved_tool_schema_strict_model_hints()
                .is_empty()
        );
        assert!(
            default_cfg
                .resolved_reasoning_extra_body_kimi_model_hints()
                .is_empty()
        );
        assert!(
            default_cfg
                .resolved_reasoning_extra_body_omit_model_hints()
                .is_empty()
        );
        assert_eq!(default_cfg.resolved_profile_state_sqlite_path(), None);

        let disabled = ProviderConfig {
            model_catalog_cache_ttl_ms: 0,
            ..ProviderConfig::default()
        };
        assert_eq!(disabled.resolved_model_catalog_cache_ttl_ms(), 0);

        let no_stale_fallback = ProviderConfig {
            model_catalog_stale_if_error_ms: 0,
            ..ProviderConfig::default()
        };
        assert_eq!(
            no_stale_fallback.resolved_model_catalog_stale_if_error_ms(),
            0
        );

        let min_entries = ProviderConfig {
            model_catalog_cache_max_entries: 0,
            ..ProviderConfig::default()
        };
        assert_eq!(min_entries.resolved_model_catalog_cache_max_entries(), 1);

        let clamped = ProviderConfig {
            model_catalog_cache_ttl_ms: 999_999,
            ..ProviderConfig::default()
        };
        assert_eq!(clamped.resolved_model_catalog_cache_ttl_ms(), 300_000);

        let stale_clamped = ProviderConfig {
            model_catalog_stale_if_error_ms: 9_999_999,
            ..ProviderConfig::default()
        };
        assert_eq!(
            stale_clamped.resolved_model_catalog_stale_if_error_ms(),
            600_000
        );

        let max_entries_clamped = ProviderConfig {
            model_catalog_cache_max_entries: 9_999,
            ..ProviderConfig::default()
        };
        assert_eq!(
            max_entries_clamped.resolved_model_catalog_cache_max_entries(),
            256
        );

        let disabled_cooldown = ProviderConfig {
            model_candidate_cooldown_ms: 0,
            ..ProviderConfig::default()
        };
        assert_eq!(disabled_cooldown.resolved_model_candidate_cooldown_ms(), 0);

        let cooldown_clamped = ProviderConfig {
            model_candidate_cooldown_ms: 9_999_999,
            ..ProviderConfig::default()
        };
        assert_eq!(
            cooldown_clamped.resolved_model_candidate_cooldown_ms(),
            3_600_000
        );

        let cooldown_max_clamped = ProviderConfig {
            model_candidate_cooldown_max_ms: 999_999_999,
            ..ProviderConfig::default()
        };
        assert_eq!(
            cooldown_max_clamped.resolved_model_candidate_cooldown_max_ms(),
            86_400_000
        );

        let cooldown_max_uses_base_floor = ProviderConfig {
            model_candidate_cooldown_ms: 120_000,
            model_candidate_cooldown_max_ms: 30_000,
            ..ProviderConfig::default()
        };
        assert_eq!(
            cooldown_max_uses_base_floor.resolved_model_candidate_cooldown_max_ms(),
            120_000
        );

        let cooldown_entries_min = ProviderConfig {
            model_candidate_cooldown_max_entries: 0,
            ..ProviderConfig::default()
        };
        assert_eq!(
            cooldown_entries_min.resolved_model_candidate_cooldown_max_entries(),
            1
        );

        let cooldown_entries_clamped = ProviderConfig {
            model_candidate_cooldown_max_entries: 99_999,
            ..ProviderConfig::default()
        };
        assert_eq!(
            cooldown_entries_clamped.resolved_model_candidate_cooldown_max_entries(),
            512
        );

        let profile_cooldown_disabled = ProviderConfig {
            profile_cooldown_ms: 0,
            ..ProviderConfig::default()
        };
        assert_eq!(profile_cooldown_disabled.resolved_profile_cooldown_ms(), 0);

        let profile_cooldown_clamped = ProviderConfig {
            profile_cooldown_ms: 9_999_999,
            ..ProviderConfig::default()
        };
        assert_eq!(
            profile_cooldown_clamped.resolved_profile_cooldown_ms(),
            3_600_000
        );

        let profile_cooldown_max_clamped = ProviderConfig {
            profile_cooldown_max_ms: 999_999_999,
            ..ProviderConfig::default()
        };
        assert_eq!(
            profile_cooldown_max_clamped.resolved_profile_cooldown_max_ms(),
            86_400_000
        );

        let profile_cooldown_max_uses_base_floor = ProviderConfig {
            profile_cooldown_ms: 120_000,
            profile_cooldown_max_ms: 30_000,
            ..ProviderConfig::default()
        };
        assert_eq!(
            profile_cooldown_max_uses_base_floor.resolved_profile_cooldown_max_ms(),
            120_000
        );

        let profile_auth_disable_min = ProviderConfig {
            profile_auth_reject_disable_ms: 10,
            ..ProviderConfig::default()
        };
        assert_eq!(
            profile_auth_disable_min.resolved_profile_auth_reject_disable_ms(),
            60_000
        );

        let profile_auth_disable_max = ProviderConfig {
            profile_auth_reject_disable_ms: 999_999_999,
            ..ProviderConfig::default()
        };
        assert_eq!(
            profile_auth_disable_max.resolved_profile_auth_reject_disable_ms(),
            604_800_000
        );

        let profile_entries_min = ProviderConfig {
            profile_state_max_entries: 0,
            ..ProviderConfig::default()
        };
        assert_eq!(profile_entries_min.resolved_profile_state_max_entries(), 1);

        let profile_entries_max = ProviderConfig {
            profile_state_max_entries: 99_999,
            ..ProviderConfig::default()
        };
        assert_eq!(
            profile_entries_max.resolved_profile_state_max_entries(),
            1024
        );

        let profile_sqlite_memory = ProviderConfig {
            profile_state_sqlite_path: Some("memory".to_owned()),
            ..ProviderConfig::default()
        };
        assert_eq!(
            profile_sqlite_memory.resolved_profile_state_sqlite_path(),
            Some(std::path::PathBuf::from(":memory:"))
        );

        let profile_sqlite_explicit_memory = ProviderConfig {
            profile_state_sqlite_path: Some(":memory:".to_owned()),
            ..ProviderConfig::default()
        };
        assert_eq!(
            profile_sqlite_explicit_memory.resolved_profile_state_sqlite_path(),
            Some(std::path::PathBuf::from(":memory:"))
        );

        let _home = ScopedLoongHome::new("loong-provider-profile-home");
        let profile_sqlite_default = ProviderConfig::default();
        let expected_default = default_loong_home().join("provider-profile-state.sqlite3");
        assert_eq!(
            profile_sqlite_default.resolved_profile_state_sqlite_path_with_default(),
            expected_default
        );

        let profile_health_enforce = ProviderConfig {
            profile_health_mode: ProviderProfileHealthModeConfig::Enforce,
            ..ProviderConfig::default()
        };
        assert_eq!(
            profile_health_enforce.resolved_profile_health_mode_config(),
            ProviderProfileHealthModeConfig::Enforce
        );

        let profile_health_observe_only = ProviderConfig {
            profile_health_mode: ProviderProfileHealthModeConfig::ObserveOnly,
            ..ProviderConfig::default()
        };
        assert_eq!(
            profile_health_observe_only.resolved_profile_health_mode_config(),
            ProviderProfileHealthModeConfig::ObserveOnly
        );
    }

    #[test]
    fn openai_codex_oauth_can_override_api_key_auth() {
        let config = ProviderConfig {
            kind: ProviderKind::Openai,
            oauth_access_token: Some(SecretRef::Inline("oauth-token".to_owned())),
            api_key: Some(SecretRef::Inline("api-key-should-not-win".to_owned())),
            ..ProviderConfig::default()
        };
        assert_eq!(
            config.default_oauth_access_token_env().as_deref(),
            Some("OPENAI_CODEX_OAUTH_TOKEN")
        );
        assert_eq!(
            config.authorization_header(),
            Some("Bearer oauth-token".to_owned())
        );
    }

    #[test]
    fn volcengine_coding_plan_has_no_default_oauth_env_but_accepts_explicit_oauth_token() {
        let config = ProviderConfig {
            kind: ProviderKind::VolcengineCoding,
            oauth_access_token: Some(SecretRef::Inline("vc-oauth-token".to_owned())),
            api_key: Some(SecretRef::Inline("api-key-should-not-win".to_owned())),
            ..ProviderConfig::default()
        };
        assert_eq!(config.default_oauth_access_token_env().as_deref(), None);
        assert_eq!(
            config.authorization_header(),
            Some("Bearer vc-oauth-token".to_owned())
        );
    }

    #[test]
    fn provider_api_key_supports_common_explicit_env_reference_formats() {
        // Use a dedicated env var instead of PATH — Windows PATH contains `;`
        // which `split_secret_candidates` treats as a candidate separator,
        // causing `api_key()` to return only the first segment.
        let mut env = ScopedEnv::new();
        clear_config_test_secret_envs(&mut env);
        let env_key = "LOONG_TEST_API_KEY_REF";
        let env_val = "test-secret-value-for-env-ref";
        env.set(env_key, env_val);

        let cases = vec![
            format!("${{{env_key}}}"),
            format!("${env_key}"),
            format!("env:{env_key}"),
            format!("%{env_key}%"),
        ];

        for raw_api_key in &cases {
            let config = ProviderConfig {
                kind: ProviderKind::Ollama,
                api_key: Some(SecretRef::Inline(raw_api_key.clone())),
                api_key_env: None,
                ..ProviderConfig::default()
            };
            assert_eq!(
                config.api_key().as_deref(),
                Some(env_val),
                "api_key={raw_api_key}"
            );
            assert_eq!(
                config.authorization_header().as_deref(),
                Some(format!("Bearer {env_val}").as_str()),
                "authorization_header should resolve env ref for {raw_api_key}"
            );
        }
    }

    #[test]
    fn provider_api_key_missing_explicit_env_reference_is_not_treated_as_literal() {
        let mut env = ScopedEnv::new();
        clear_config_test_secret_envs(&mut env);

        let config = ProviderConfig {
            kind: ProviderKind::Ollama,
            api_key: Some(SecretRef::Inline(
                "${LOONG_TEST_MISSING_API_KEY}".to_owned(),
            )),
            api_key_env: None,
            ..ProviderConfig::default()
        };

        assert_eq!(config.api_key(), None);
        assert_eq!(config.authorization_header(), None);
    }

    #[test]
    fn provider_api_key_missing_explicit_env_reference_does_not_fall_back_to_legacy_env() {
        let mut env = ScopedEnv::new();
        clear_config_test_secret_envs(&mut env);

        let config = ProviderConfig {
            kind: ProviderKind::Openai,
            api_key: Some(SecretRef::Inline(
                "${LOONG_TEST_MISSING_API_KEY}".to_owned(),
            )),
            api_key_env: Some("PATH".to_owned()),
            ..ProviderConfig::default()
        };

        assert_eq!(config.api_key(), None);
        assert_eq!(config.authorization_header(), None);
    }

    #[test]
    fn provider_api_key_env_legacy_fallback_still_works() {
        // Use a dedicated env var instead of PATH — Windows PATH contains `;`
        // which `split_secret_candidates` treats as a candidate separator.
        let mut env = ScopedEnv::new();
        clear_config_test_secret_envs(&mut env);
        let env_key = "LOONG_TEST_LEGACY_FALLBACK";
        let env_val = "test-secret-value-for-legacy";
        env.set(env_key, env_val);

        let config = ProviderConfig {
            kind: ProviderKind::Ollama,
            api_key: None,
            api_key_env: Some(env_key.to_owned()),
            ..ProviderConfig::default()
        };

        assert_eq!(config.api_key().as_deref(), Some(env_val));
    }

    #[test]
    fn provider_api_key_supports_typed_env_secret_ref() {
        let mut env = ScopedEnv::new();
        clear_config_test_secret_envs(&mut env);
        let env_key = "LOONG_TEST_TYPED_SECRET_REF";
        let env_val = "typed-secret-value";
        env.set(env_key, env_val);

        let config = ProviderConfig {
            kind: ProviderKind::Ollama,
            api_key: Some(SecretRef::Env {
                env: env_key.to_owned(),
            }),
            api_key_env: None,
            ..ProviderConfig::default()
        };

        assert_eq!(config.api_key().as_deref(), Some(env_val));
        assert_eq!(
            config.authorization_header().as_deref(),
            Some("Bearer typed-secret-value")
        );
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn provider_toml_parses_secret_ref_file_table() {
        let raw = r#"
[provider]
api_key = { file = "/run/secrets/openai" }
"#;
        let parsed = toml::from_str::<LoongConfig>(raw).expect("secret table should parse");

        assert_eq!(
            parsed.provider.api_key,
            Some(SecretRef::File {
                file: PathBuf::from("/run/secrets/openai"),
            })
        );
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn provider_kind_keeps_legacy_volcengine_custom_alias() {
        let raw = r#"
[provider]
kind = "volcengine_custom"
model = "model-example"
"#;
        let parsed =
            toml::from_str::<LoongConfig>(raw).expect("parse legacy kind alias should pass");
        assert_eq!(parsed.provider.kind, ProviderKind::Volcengine);
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn provider_kind_keeps_legacy_compatible_aliases() {
        let raw = r#"
[provider]
kind = "xai_compatible"
model = "model-example"
"#;
        let parsed =
            toml::from_str::<LoongConfig>(raw).expect("parse compatible alias should pass");
        assert_eq!(parsed.provider.kind, ProviderKind::Xai);
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn provider_kind_keeps_kimi_coding_compatible_alias() {
        let raw = r#"
[provider]
kind = "kimi_coding_compatible"
model = "model-example"
"#;
        let parsed =
            toml::from_str::<LoongConfig>(raw).expect("parse kimi coding alias should pass");
        assert_eq!(parsed.provider.kind, ProviderKind::KimiCoding);
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn kimi_coding_partial_config_uses_internal_defaults() {
        let raw = r#"
[provider]
kind = "kimi_coding"
"#;
        let parsed = toml::from_str::<LoongConfig>(raw).expect("parse minimal kimi coding config");
        assert_eq!(parsed.provider.kind, ProviderKind::KimiCoding);
        assert_eq!(
            parsed.provider.endpoint(),
            "https://api.kimi.com/coding/v1/chat/completions"
        );
        assert_eq!(
            parsed.provider.models_endpoint(),
            "https://api.kimi.com/coding/v1/models"
        );
        assert_eq!(parsed.provider.resolved_model(), None);
        assert_eq!(
            parsed.provider.default_api_key_env().as_deref(),
            Some("KIMI_CODING_API_KEY")
        );
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn bailian_coding_partial_config_uses_internal_defaults() {
        let raw = r#"
[provider]
kind = "bailian_coding"
"#;
        let parsed =
            toml::from_str::<LoongConfig>(raw).expect("parse minimal bailian coding config");
        assert_eq!(parsed.provider.kind, ProviderKind::BailianCoding);
        assert_eq!(
            parsed.provider.endpoint(),
            "https://coding.dashscope.aliyuncs.com/v1/chat/completions"
        );
        assert_eq!(
            parsed.provider.models_endpoint(),
            "https://coding.dashscope.aliyuncs.com/v1/models"
        );
        assert_eq!(
            parsed.provider.default_api_key_env().as_deref(),
            Some("BAILIAN_API_KEY")
        );
        assert_eq!(parsed.provider.kind.default_user_agent(), Some("openclaw"));
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn provider_kind_keeps_new_provider_aliases() {
        let cases = vec![
            ("aws-bedrock", ProviderKind::Bedrock),
            ("bailian_coding_compatible", ProviderKind::BailianCoding),
            ("byteplus_compatible", ProviderKind::Byteplus),
            ("byteplus_coding_compatible", ProviderKind::ByteplusCoding),
            ("openai_custom", ProviderKind::Custom),
            (
                "volcengine_coding_compatible",
                ProviderKind::VolcengineCoding,
            ),
        ];

        for (kind, expected) in cases {
            let raw = format!(
                r#"
[provider]
kind = "{kind}"
model = "model-example"
"#
            );
            let parsed =
                toml::from_str::<LoongConfig>(&raw).expect("parse provider alias should succeed");
            assert_eq!(parsed.provider.kind, expected, "kind={kind}");
        }
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn byteplus_coding_partial_config_uses_internal_defaults() {
        let raw = r#"
[provider]
kind = "byteplus_coding"
"#;
        let parsed =
            toml::from_str::<LoongConfig>(raw).expect("parse minimal byteplus coding config");
        assert_eq!(parsed.provider.kind, ProviderKind::ByteplusCoding);
        assert_eq!(
            parsed.provider.endpoint(),
            "https://ark.ap-southeast.bytepluses.com/api/coding/v3/chat/completions"
        );
        assert_eq!(
            parsed.provider.models_endpoint(),
            "https://ark.ap-southeast.bytepluses.com/api/coding/v3/models"
        );
        assert_eq!(
            parsed.provider.default_api_key_env().as_deref(),
            Some("BYTEPLUS_API_KEY")
        );
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn volcengine_coding_partial_config_uses_internal_defaults() {
        let raw = r#"
[provider]
kind = "volcengine_coding"
"#;
        let parsed =
            toml::from_str::<LoongConfig>(raw).expect("parse minimal volcengine coding config");
        assert_eq!(parsed.provider.kind, ProviderKind::VolcengineCoding);
        assert_eq!(
            parsed.provider.endpoint(),
            "https://ark.cn-beijing.volces.com/api/coding/v3/chat/completions"
        );
        assert_eq!(
            parsed.provider.models_endpoint(),
            "https://ark.cn-beijing.volces.com/api/coding/v3/models"
        );
        assert_eq!(
            parsed.provider.default_oauth_access_token_env().as_deref(),
            None
        );
    }

    #[test]
    fn custom_provider_requires_concrete_base_url_configuration() {
        let config = ProviderConfig {
            kind: ProviderKind::Custom,
            ..ProviderConfig::default()
        };

        assert!(config.has_unresolved_custom_base_url());
        let hint = config
            .configuration_hint()
            .expect("custom provider should require a concrete base url");
        assert!(hint.contains("custom"));
        assert!(hint.contains("<openai-compatible-host>"));
    }

    #[test]
    fn volcengine_coding_warns_when_pointed_at_generic_modelark_path() {
        let config = ProviderConfig {
            kind: ProviderKind::VolcengineCoding,
            base_url: "https://ark.cn-beijing.volces.com/api/v3".to_owned(),
            ..ProviderConfig::default()
        };

        let hint = config
            .configuration_hint()
            .expect("volcengine_coding should require the dedicated Coding Plan path");
        assert!(hint.contains("volcengine_coding"));
        assert!(hint.contains("/api/coding/v3"));
    }

    #[test]
    fn opencode_zen_warns_when_given_go_model_prefix() {
        let config = ProviderConfig {
            kind: ProviderKind::OpencodeZen,
            model: "opencode-go/glm-5.1".to_owned(),
            ..ProviderConfig::default()
        };

        let hint = config
            .configuration_hint()
            .expect("opencode_zen should warn about Go-prefixed models");
        assert!(hint.contains("opencode_go"));
    }

    #[test]
    fn opencode_go_warns_when_pointed_at_zen_base_url() {
        let config = ProviderConfig {
            kind: ProviderKind::OpencodeGo,
            base_url: "https://opencode.ai/zen/v1".to_owned(),
            ..ProviderConfig::default()
        };

        let hint = config
            .configuration_hint()
            .expect("opencode_go should warn about the wrong base url root");
        assert!(hint.contains("opencode_zen"));
    }

    #[test]
    fn bedrock_uses_region_template_endpoints() {
        let config = ProviderConfig {
            kind: ProviderKind::Bedrock,
            ..ProviderConfig::default()
        };

        assert_eq!(
            config.endpoint(),
            "https://bedrock-runtime.<region>.amazonaws.com/model/{modelId}/converse"
        );
        assert_eq!(
            config.models_endpoint(),
            "https://bedrock.<region>.amazonaws.com/foundation-models"
        );
        assert_eq!(
            config.default_api_key_env().as_deref(),
            Some("AWS_BEARER_TOKEN_BEDROCK")
        );
    }

    #[test]
    fn minimax_region_endpoint_note_points_to_global_alternative() {
        let config = ProviderConfig {
            kind: ProviderKind::Minimax,
            ..ProviderConfig::default()
        };

        let note = config
            .region_endpoint_note()
            .expect("minimax should surface region endpoint guidance");
        assert!(note.contains("CN default"));
        assert!(note.contains("https://api.minimaxi.com"));
        assert!(note.contains("https://api.minimax.io"));
    }

    #[test]
    fn kimi_region_endpoint_note_respects_explicit_global_override() {
        let config = ProviderConfig {
            kind: ProviderKind::Kimi,
            base_url: "https://api.moonshot.ai".to_owned(),
            ..ProviderConfig::default()
        };

        let note = config
            .region_endpoint_note()
            .expect("kimi should surface region endpoint guidance");
        assert!(note.contains("using Global"));
        assert!(note.contains("https://api.moonshot.ai"));
        assert!(note.contains("https://api.moonshot.cn"));
    }

    #[test]
    fn zhipu_region_endpoint_failure_hint_points_to_global_zai_endpoint() {
        let config = ProviderConfig {
            kind: ProviderKind::Zhipu,
            ..ProviderConfig::default()
        };

        let hint = config
            .region_endpoint_failure_hint()
            .expect("zhipu should surface a region retry hint");
        assert!(hint.contains("provider.base_url"));
        assert!(hint.contains("https://open.bigmodel.cn"));
        assert!(hint.contains("https://api.z.ai"));
    }

    #[test]
    fn zhipu_region_endpoint_info_uses_normalized_family_label_and_ordered_variants() {
        let region_info = ProviderKind::Zhipu
            .region_endpoint_info()
            .expect("zhipu should expose onboarding region info");

        assert_eq!(region_info.family_label, "Z.ai");
        assert_eq!(region_info.variants.len(), 2);
        assert_eq!(region_info.variants[0].label, "CN");
        assert_eq!(region_info.variants[0].base_url, "https://open.bigmodel.cn");
        assert_eq!(region_info.variants[1].label, "Global");
        assert_eq!(region_info.variants[1].base_url, "https://api.z.ai");
    }

    #[test]
    fn minimax_region_endpoint_hint_respects_explicit_endpoint_override() {
        let mut config = ProviderConfig {
            kind: ProviderKind::Minimax,
            ..ProviderConfig::default()
        };
        config.set_endpoint(Some(
            "https://api.minimax.io/v1/chat/completions".to_owned(),
        ));

        let note = config
            .region_endpoint_note()
            .expect("minimax should surface explicit endpoint override guidance");
        assert!(note.contains("provider.endpoint"));
        assert!(note.contains("https://api.minimax.io/v1/chat/completions"));

        let hint = config
            .region_endpoint_failure_hint()
            .expect("minimax should surface explicit endpoint override failure guidance");
        assert!(hint.contains("provider.endpoint"));
        assert!(hint.contains("Changing `provider.base_url` alone will not affect"));
    }

    #[test]
    fn zai_region_endpoint_hint_respects_explicit_models_endpoint_override() {
        let mut config = ProviderConfig {
            kind: ProviderKind::Zai,
            ..ProviderConfig::default()
        };
        config.set_models_endpoint(Some("https://open.bigmodel.cn/v1/models".to_owned()));

        let note = config
            .region_endpoint_note()
            .expect("zai should surface explicit models endpoint override guidance");
        assert!(note.contains("provider.models_endpoint"));
        assert!(note.contains("https://open.bigmodel.cn/v1/models"));

        let hint = config
            .region_endpoint_failure_hint()
            .expect("zai should surface explicit models endpoint override failure guidance");
        assert!(hint.contains("provider.models_endpoint"));
        assert!(hint.contains("Changing `provider.base_url` alone will not affect"));
    }

    #[test]
    fn minimax_region_endpoint_note_for_custom_explicit_endpoint_labels_official_hosts_correctly() {
        let mut config = ProviderConfig {
            kind: ProviderKind::Minimax,
            ..ProviderConfig::default()
        };
        config.set_endpoint(Some(
            "https://proxy.example.test/v1/chat/completions".to_owned(),
        ));

        let note = config
            .region_endpoint_note()
            .expect("minimax should surface explicit endpoint override guidance");

        assert!(note.contains("provider.endpoint"));
        assert!(note.contains("https://proxy.example.test/v1/chat/completions"));
        assert!(note.contains("official CN endpoint `https://api.minimaxi.com`"));
        assert!(note.contains("official Global endpoint `https://api.minimax.io`"));
    }

    #[test]
    fn models_endpoint_resolution_for_supported_provider_profiles_includes_new_first_class_providers()
     {
        let cases = vec![
            (
                ProviderKind::Bedrock,
                "https://bedrock.<region>.amazonaws.com/foundation-models",
            ),
            (
                ProviderKind::Byteplus,
                "https://ark.ap-southeast.bytepluses.com/api/v3/models",
            ),
            (
                ProviderKind::ByteplusCoding,
                "https://ark.ap-southeast.bytepluses.com/api/coding/v3/models",
            ),
            (
                ProviderKind::VolcengineCoding,
                "https://ark.cn-beijing.volces.com/api/coding/v3/models",
            ),
            (
                ProviderKind::OpencodeGo,
                "https://opencode.ai/zen/go/v1/models",
            ),
            (
                ProviderKind::OpencodeZen,
                "https://opencode.ai/zen/v1/models",
            ),
        ];

        for (kind, expected) in cases {
            let config = ProviderConfig {
                kind,
                ..ProviderConfig::default()
            };
            assert_eq!(config.models_endpoint(), expected, "kind={kind:?}");
        }
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn provider_profile_health_mode_parses_from_toml() {
        let raw = r#"
[provider]
kind = "openrouter"
profile_health_mode = "enforce"
"#;
        let parsed =
            toml::from_str::<LoongConfig>(raw).expect("parse profile health mode should pass");
        assert_eq!(parsed.provider.kind, ProviderKind::Openrouter);
        assert_eq!(
            parsed.provider.resolved_profile_health_mode_config(),
            ProviderProfileHealthModeConfig::Enforce
        );
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn provider_profile_health_mode_supports_all_config_values() {
        let observe_only_raw = r#"
[provider]
kind = "openai"
profile_health_mode = "observe_only"
"#;
        let observe_only = toml::from_str::<LoongConfig>(observe_only_raw)
            .expect("parse observe_only profile health mode should pass");
        assert_eq!(
            observe_only.provider.resolved_profile_health_mode_config(),
            ProviderProfileHealthModeConfig::ObserveOnly
        );

        let provider_default_raw = r#"
[provider]
kind = "openrouter"
profile_health_mode = "provider_default"
"#;
        let provider_default = toml::from_str::<LoongConfig>(provider_default_raw)
            .expect("parse provider_default profile health mode should pass");
        assert_eq!(
            provider_default
                .provider
                .resolved_profile_health_mode_config(),
            ProviderProfileHealthModeConfig::ProviderDefault
        );
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn provider_tool_schema_mode_parses_from_toml() {
        let raw = r#"
[provider]
kind = "openai"
tool_schema_mode = "enabled_strict"
"#;
        let parsed =
            toml::from_str::<LoongConfig>(raw).expect("parse tool schema mode should pass");
        assert_eq!(parsed.provider.kind, ProviderKind::Openai);
        assert_eq!(
            parsed.provider.resolved_tool_schema_mode_config(),
            ProviderToolSchemaModeConfig::EnabledStrict
        );
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn provider_tool_schema_mode_supports_all_config_values() {
        let disabled_raw = r#"
[provider]
kind = "openai"
tool_schema_mode = "disabled"
"#;
        let disabled = toml::from_str::<LoongConfig>(disabled_raw)
            .expect("parse disabled tool schema mode should pass");
        assert_eq!(
            disabled.provider.resolved_tool_schema_mode_config(),
            ProviderToolSchemaModeConfig::Disabled
        );

        let downgraded_raw = r#"
[provider]
kind = "openai"
tool_schema_mode = "enabled_with_downgrade"
"#;
        let downgraded = toml::from_str::<LoongConfig>(downgraded_raw)
            .expect("parse enabled_with_downgrade tool schema mode should pass");
        assert_eq!(
            downgraded.provider.resolved_tool_schema_mode_config(),
            ProviderToolSchemaModeConfig::EnabledWithDowngrade
        );

        let provider_default_raw = r#"
[provider]
kind = "openai"
tool_schema_mode = "provider_default"
"#;
        let provider_default = toml::from_str::<LoongConfig>(provider_default_raw)
            .expect("parse provider_default tool schema mode should pass");
        assert_eq!(
            provider_default.provider.resolved_tool_schema_mode_config(),
            ProviderToolSchemaModeConfig::ProviderDefault
        );
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn provider_reasoning_extra_body_mode_supports_all_config_values() {
        let kimi_thinking_raw = r#"
[provider]
kind = "openai"
reasoning_extra_body_mode = "kimi_thinking"
"#;
        let kimi_thinking = toml::from_str::<LoongConfig>(kimi_thinking_raw)
            .expect("parse kimi_thinking reasoning mode should pass");
        assert_eq!(
            kimi_thinking
                .provider
                .resolved_reasoning_extra_body_mode_config(),
            ProviderReasoningExtraBodyModeConfig::KimiThinking
        );

        let omit_raw = r#"
[provider]
kind = "openai"
reasoning_extra_body_mode = "omit"
"#;
        let omit =
            toml::from_str::<LoongConfig>(omit_raw).expect("parse omit reasoning mode should pass");
        assert_eq!(
            omit.provider.resolved_reasoning_extra_body_mode_config(),
            ProviderReasoningExtraBodyModeConfig::Omit
        );

        let provider_default_raw = r#"
[provider]
kind = "openai"
reasoning_extra_body_mode = "provider_default"
"#;
        let provider_default = toml::from_str::<LoongConfig>(provider_default_raw)
            .expect("parse provider_default reasoning mode should pass");
        assert_eq!(
            provider_default
                .provider
                .resolved_reasoning_extra_body_mode_config(),
            ProviderReasoningExtraBodyModeConfig::ProviderDefault
        );
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn provider_capability_model_hints_parse_from_toml() {
        let raw = r#"
[provider]
kind = "openai"
tool_schema_disabled_model_hints = ["no-tools", "legacy-plain-text"]
tool_schema_strict_model_hints = ["strict-schema"]
reasoning_extra_body_kimi_model_hints = ["enable-thinking"]
reasoning_extra_body_omit_model_hints = ["disable-thinking"]
"#;
        let parsed = toml::from_str::<LoongConfig>(raw)
            .expect("parse provider capability model hints should pass");

        assert_eq!(
            parsed.provider.resolved_tool_schema_disabled_model_hints(),
            vec!["no-tools", "legacy-plain-text"]
        );
        assert_eq!(
            parsed.provider.resolved_tool_schema_strict_model_hints(),
            vec!["strict-schema"]
        );
        assert_eq!(
            parsed
                .provider
                .resolved_reasoning_extra_body_kimi_model_hints(),
            vec!["enable-thinking"]
        );
        assert_eq!(
            parsed
                .provider
                .resolved_reasoning_extra_body_omit_model_hints(),
            vec!["disable-thinking"]
        );
    }

    #[test]
    fn provider_capability_model_hints_normalize_empty_entries() {
        let provider = ProviderConfig {
            tool_schema_disabled_model_hints: vec![
                "".to_owned(),
                "  no-tools  ".to_owned(),
                "   ".to_owned(),
            ],
            tool_schema_strict_model_hints: vec![" strict-schema ".to_owned()],
            reasoning_extra_body_kimi_model_hints: vec![" enable-thinking ".to_owned()],
            reasoning_extra_body_omit_model_hints: vec![" disable-thinking ".to_owned()],
            ..ProviderConfig::default()
        };

        assert_eq!(
            provider.resolved_tool_schema_disabled_model_hints(),
            vec!["no-tools"]
        );
        assert_eq!(
            provider.resolved_tool_schema_strict_model_hints(),
            vec!["strict-schema"]
        );
        assert_eq!(
            provider.resolved_reasoning_extra_body_kimi_model_hints(),
            vec!["enable-thinking"]
        );
        assert_eq!(
            provider.resolved_reasoning_extra_body_omit_model_hints(),
            vec!["disable-thinking"]
        );
    }

    #[test]
    #[cfg(feature = "channel-telegram")]
    fn telegram_token_prefers_inline_secret() {
        let config = TelegramChannelConfig {
            bot_token: Some(loong_contracts::SecretRef::Inline(
                "inline-token".to_owned(),
            )),
            bot_token_env: Some("SHOULD_NOT_BE_READ".to_owned()),
            ..TelegramChannelConfig::default()
        };
        assert_eq!(config.bot_token().as_deref(), Some("inline-token"));
    }

    #[test]
    #[cfg(feature = "channel-telegram")]
    fn telegram_bot_token_supports_typed_env_secret_ref() {
        let mut env = ScopedEnv::new();
        clear_config_test_secret_envs(&mut env);
        let env_key = "LOONG_TEST_TELEGRAM_SECRET_REF";
        let env_val = "123456789:telegram-secret";
        env.set(env_key, env_val);

        let config = TelegramChannelConfig {
            bot_token: Some(SecretRef::Env {
                env: env_key.to_owned(),
            }),
            bot_token_env: None,
            ..TelegramChannelConfig::default()
        };

        assert_eq!(config.bot_token().as_deref(), Some(env_val));
    }

    #[test]
    #[cfg(all(feature = "channel-telegram", feature = "config-toml"))]
    fn telegram_toml_parses_secret_ref_file_table() {
        let raw = r#"
[telegram]
bot_token = { file = "/run/secrets/telegram" }
"#;
        let parsed = toml::from_str::<LoongConfig>(raw).expect("secret table should parse");

        assert_eq!(
            parsed.telegram.bot_token,
            Some(SecretRef::File {
                file: PathBuf::from("/run/secrets/telegram"),
            })
        );
    }

    #[test]
    fn provider_api_key_candidates_support_delimited_key_pool() {
        let config = ProviderConfig {
            kind: ProviderKind::Ollama,
            api_key: Some(SecretRef::Inline(
                "key-a, key-b;key-c\nkey-d\r\nkey-e".to_owned(),
            )),
            api_key_env: None,
            ..ProviderConfig::default()
        };

        assert_eq!(
            config.api_key_candidates(),
            vec![
                "key-a".to_owned(),
                "key-b".to_owned(),
                "key-c".to_owned(),
                "key-d".to_owned(),
                "key-e".to_owned()
            ]
        );
        assert_eq!(config.api_key(), Some("key-a".to_owned()));
    }

    #[test]
    fn config_validation_rejects_secret_literal_in_provider_api_key_env() {
        let mut config = LoongConfig::default();
        config.provider.api_key_env = Some("sk-live-direct-secret-value".to_owned());
        config.provider.api_key = None;

        let error = config
            .validate()
            .expect_err("secret literal in provider.api_key_env should be rejected");
        assert!(error.contains("config.env_pointer.secret_literal"));
        assert!(error.contains("provider.api_key_env"));
        assert!(error.contains("provider.api_key"));
    }

    #[test]
    fn config_validation_message_does_not_echo_secret_literal() {
        let secret = "sk-live-direct-secret-value";
        let mut config = LoongConfig::default();
        config.provider.api_key_env = Some(secret.to_owned());
        config.provider.api_key = None;

        let error = config
            .validate()
            .expect_err("secret literal in provider.api_key_env should be rejected");
        assert!(
            !error.contains(secret),
            "validation error should not leak secret"
        );
    }

    #[test]
    fn config_validation_uses_provider_specific_example_env_name() {
        let mut config = LoongConfig::default();
        config.provider.kind = ProviderKind::Minimax;
        config.provider.api_key_env = Some("sk-minimax-inline-secret".to_owned());

        let error = config
            .validate()
            .expect_err("secret literal in minimax env pointer should be rejected");
        assert!(error.contains("MINIMAX_API_KEY"));
    }

    #[test]
    fn config_validation_rejects_secret_literal_in_telegram_bot_token_env() {
        let mut config = LoongConfig::default();
        config.telegram.bot_token_env = Some("123456789:telegram-secret-token-literal".to_owned());
        config.telegram.bot_token = None;

        let error = config
            .validate()
            .expect_err("secret literal in telegram.bot_token_env should be rejected");
        assert!(error.contains("config.env_pointer.secret_literal"));
        assert!(error.contains("telegram.bot_token_env"));
        assert!(error.contains("telegram.bot_token"));
    }

    #[test]
    fn config_validation_rejects_duplicate_normalized_telegram_account_ids() {
        let config: LoongConfig = serde_json::from_value(serde_json::json!({
            "telegram": {
                "accounts": {
                    "Work Bot": {
                        "bot_token_env": "WORK_TELEGRAM_TOKEN"
                    },
                    "work-bot": {
                        "bot_token_env": "WORK_TELEGRAM_TOKEN_DUP"
                    }
                }
            }
        }))
        .expect("deserialize telegram duplicate-account config");

        let error = config
            .validate()
            .expect_err("duplicate normalized telegram account ids should fail");
        assert!(error.contains("config.channel_account.duplicate_id"));
        assert!(error.contains("telegram.accounts"));
        assert!(error.contains("work-bot"));
        assert!(error.contains("Work Bot"));
    }

    #[test]
    fn config_validation_rejects_duplicate_normalized_feishu_account_ids() {
        let config: LoongConfig = serde_json::from_value(serde_json::json!({
            "feishu": {
                "accounts": {
                    "Lark Prod": {
                        "app_id_env": "LARK_APP_ID",
                        "app_secret_env": "LARK_APP_SECRET"
                    },
                    "lark-prod": {
                        "app_id_env": "LARK_APP_ID_DUP",
                        "app_secret_env": "LARK_APP_SECRET_DUP"
                    }
                }
            }
        }))
        .expect("deserialize feishu duplicate-account config");

        let error = config
            .validate()
            .expect_err("duplicate normalized feishu account ids should fail");
        assert!(error.contains("config.channel_account.duplicate_id"));
        assert!(error.contains("feishu.accounts"));
        assert!(error.contains("lark-prod"));
        assert!(error.contains("Lark Prod"));
    }

    #[test]
    fn config_validation_rejects_unknown_telegram_default_account() {
        let config: LoongConfig = serde_json::from_value(serde_json::json!({
            "telegram": {
                "default_account": "missing",
                "accounts": {
                    "alpha": {
                        "bot_token_env": "ALPHA_TELEGRAM_TOKEN"
                    },
                    "beta": {
                        "bot_token_env": "BETA_TELEGRAM_TOKEN"
                    }
                }
            }
        }))
        .expect("deserialize telegram unknown-default config");

        let error = config
            .validate()
            .expect_err("unknown telegram default account should fail");
        assert!(error.contains("config.channel_account.unknown_default"));
        assert!(error.contains("telegram.default_account"));
        assert!(error.contains("missing"));
        assert!(error.contains("alpha"));
        assert!(error.contains("beta"));
    }

    #[test]
    fn config_validation_accepts_shell_style_env_names() {
        let mut config = LoongConfig::default();
        config.provider.api_key_env = Some("KIMI_CODING_API_KEY".to_owned());
        config.telegram.bot_token_env = Some("TELEGRAM_BOT_TOKEN".to_owned());

        config
            .validate()
            .expect("valid shell-style env names should pass");
    }

    #[test]
    fn config_validation_accepts_non_shell_env_names_for_compatibility() {
        let mut config = LoongConfig::default();
        config.provider.api_key_env = Some("OPENAI-API-KEY".to_owned());

        config
            .validate()
            .expect("non-shell env names stay compatible as env pointers");
    }

    #[test]
    fn config_validation_accepts_long_compatible_env_names() {
        let mut config = LoongConfig::default();
        config.provider.api_key_env = Some("VERY-LONG-ENV-NAME-WITH-DASHES-AND-DOTS.v2".to_owned());

        config
            .validate()
            .expect("long compatible env names should not be mistaken for secret literals");
    }

    #[test]
    fn config_validation_rejects_zero_memory_sliding_window() {
        let mut config = LoongConfig::default();
        config.memory.sliding_window = 0;

        let error = config
            .validate()
            .expect_err("zero memory.sliding_window should be rejected");
        assert!(error.contains("memory.sliding_window"));
        assert!(error.contains("between 1 and 128"));
    }

    #[test]
    fn config_validation_rejects_memory_sliding_window_above_adapter_cap() {
        let mut config = LoongConfig::default();
        config.memory.sliding_window = 129;

        let error = config
            .validate()
            .expect_err("memory.sliding_window above adapter cap should be rejected");
        assert!(error.contains("memory.sliding_window"));
        assert!(error.contains("between 1 and 128"));
        assert!(error.contains("129"));
    }

    #[test]
    fn config_validation_rejects_assignment_style_env_pointer() {
        let mut config = LoongConfig::default();
        config.provider.api_key_env = Some("OPENAI_API_KEY=sk-1234567890".to_owned());

        let error = config
            .validate()
            .expect_err("assignment-style value should be rejected");
        assert!(error.contains("provider.api_key_env"));
        assert!(error.contains("KEY=VALUE"));
    }

    #[test]
    fn config_validation_rejects_export_assignment_style_env_pointer() {
        let mut config = LoongConfig::default();
        config.provider.api_key_env = Some("export OPENAI_API_KEY=sk-1234567890".to_owned());

        let error = config
            .validate()
            .expect_err("export assignment-style value should be rejected");
        assert!(error.contains("provider.api_key_env"));
        assert!(error.contains("OPENAI_API_KEY"));
    }

    #[test]
    fn config_validation_rejects_set_assignment_style_env_pointer() {
        let mut config = LoongConfig::default();
        config.provider.api_key_env = Some("set OPENAI_API_KEY=sk-1234567890".to_owned());

        let error = config
            .validate()
            .expect_err("set assignment-style value should be rejected");
        assert!(error.contains("provider.api_key_env"));
        assert!(error.contains("OPENAI_API_KEY"));
    }

    #[test]
    fn config_validation_rejects_dollar_prefixed_env_pointer() {
        let mut config = LoongConfig::default();
        config.provider.api_key_env = Some("$OPENAI_API_KEY".to_owned());

        let error = config
            .validate()
            .expect_err("dollar-prefixed env pointer should be rejected");
        assert!(error.contains("provider.api_key_env"));
        assert!(error.contains("without `$`"));
    }

    #[test]
    fn config_validation_rejects_braced_dollar_prefixed_env_pointer() {
        let mut config = LoongConfig::default();
        config.provider.api_key_env = Some("${OPENAI_API_KEY}".to_owned());

        let error = config
            .validate()
            .expect_err("braced dollar-prefixed env pointer should be rejected");
        assert!(error.contains("provider.api_key_env"));
        assert!(error.contains("without `$`"));
        assert!(error.contains("OPENAI_API_KEY"));
    }

    #[test]
    fn config_validation_rejects_percent_wrapped_env_pointer() {
        let mut config = LoongConfig::default();
        config.provider.api_key_env = Some("%OPENAI_API_KEY%".to_owned());

        let error = config
            .validate()
            .expect_err("percent-wrapped env pointer should be rejected");
        assert!(error.contains("provider.api_key_env"));
        assert!(error.contains("%VAR%"));
        assert!(error.contains("OPENAI_API_KEY"));
    }

    #[test]
    fn config_validation_rejects_bare_dollar_env_pointer() {
        let mut config = LoongConfig::default();
        config.provider.api_key_env = Some("$".to_owned());

        let error = config
            .validate()
            .expect_err("bare dollar env pointer should be rejected");
        assert!(error.contains("provider.api_key_env"));
        assert!(error.contains("without `$`"));
        assert!(error.contains("OPENAI_API_KEY"));
    }

    #[test]
    fn config_validation_rejects_invalid_env_pointer_name() {
        let mut config = LoongConfig::default();
        config.provider.api_key_env = Some("OPENAI API KEY".to_owned());

        let error = config
            .validate()
            .expect_err("whitespace in env pointer should be rejected");
        assert!(error.contains("config.env_pointer.invalid_name"));
        assert!(error.contains("provider.api_key_env"));
    }

    #[test]
    fn config_validation_rejects_bearer_prefixed_secret_in_env_pointer() {
        let mut config = LoongConfig::default();
        config.provider.api_key_env = Some("Bearer sk-live-token-value".to_owned());

        let error = config
            .validate()
            .expect_err("bearer-prefixed secret should be rejected");
        assert!(error.contains("provider.api_key_env"));
        assert!(error.contains("secret literal"));
    }

    #[test]
    fn config_validation_rejects_uuid_shaped_secret_in_env_pointer() {
        let mut config = LoongConfig::default();
        config.provider.api_key_env = Some("9f479837-0a12-4b56-89ab-cdef01234567".to_owned());

        let error = config
            .validate()
            .expect_err("uuid-shaped provider secrets should be rejected in env pointer fields");
        assert!(error.contains("provider.api_key_env"));
        assert!(error.contains("secret literal"));
    }

    #[test]
    fn config_validation_rejects_invalid_typed_secret_ref_env_names() {
        let config: LoongConfig = serde_json::from_value(serde_json::json!({
            "provider": {
                "api_key": {
                    "env": "$OPENAI_API_KEY"
                }
            },
            "telegram": {
                "bot_token": {
                    "env": "123456789:AAEZZ_exampleTokenValue"
                },
                "accounts": {
                    "Work Bot": {
                        "bot_token": {
                            "env": "export WORK_TELEGRAM_TOKEN=demo"
                        }
                    }
                }
            },
            "feishu": {
                "app_secret": {
                    "env": "FEISHU APP SECRET"
                }
            },
            "matrix": {
                "access_token": {
                    "env": "%MATRIX_ACCESS_TOKEN%"
                }
            }
        }))
        .expect("deserialize config with invalid typed env refs");

        let error = config
            .validate()
            .expect_err("invalid typed env refs should be rejected");

        assert!(error.contains("provider.api_key.env"));
        assert!(error.contains("telegram.bot_token.env"));
        assert!(error.contains("telegram.accounts.work-bot.bot_token.env"));
        assert!(error.contains("feishu.app_secret.env"));
        assert!(error.contains("matrix.access_token.env"));
    }

    #[test]
    fn config_validation_rejects_telegram_like_token_in_env_pointer() {
        let mut config = LoongConfig::default();
        config.telegram.bot_token_env = Some("123456789:AAEZZ_exampleTokenValue".to_owned());

        let error = config
            .validate()
            .expect_err("telegram-like token should be rejected");
        assert!(error.contains("telegram.bot_token_env"));
        assert!(error.contains("secret literal"));
    }

    #[test]
    fn config_validation_reports_multiple_env_pointer_issues_in_one_pass() {
        let mut config = LoongConfig::default();
        config.provider.api_key_env = Some("OPENAI_API_KEY=sk-inline".to_owned());
        config.telegram.bot_token_env = Some("123456789:telegram-inline-secret-literal".to_owned());

        let error = config
            .validate()
            .expect_err("multiple config issues should be aggregated");
        assert!(error.contains("provider.api_key_env"));
        assert!(error.contains("telegram.bot_token_env"));
    }

    #[test]
    fn feishu_defaults_are_stable() {
        let config = FeishuChannelConfig::default();
        assert_eq!(config.domain, FeishuDomain::Feishu);
        assert_eq!(config.base_url, None);
        assert_eq!(config.resolved_base_url(), "https://open.feishu.cn");
        assert_eq!(config.receive_id_type, "chat_id");
        assert_eq!(config.webhook_bind, "127.0.0.1:8080");
        assert_eq!(config.webhook_path, "/feishu/events");
        assert_eq!(
            config.encrypt_key_env.as_deref(),
            Some("FEISHU_ENCRYPT_KEY")
        );
        assert!(config.ignore_bot_messages);
    }

    #[test]
    fn feishu_lark_domain_uses_lark_base_url_when_base_url_not_set() {
        let config = FeishuChannelConfig {
            domain: FeishuDomain::Lark,
            base_url: None,
            ..FeishuChannelConfig::default()
        };

        assert_eq!(config.resolved_base_url(), "https://open.larksuite.com");
    }

    #[test]
    fn feishu_explicit_base_url_overrides_domain_default() {
        let config = FeishuChannelConfig {
            domain: FeishuDomain::Lark,
            base_url: Some("https://example.internal".to_owned()),
            ..FeishuChannelConfig::default()
        };

        assert_eq!(config.resolved_base_url(), "https://example.internal");
    }

    #[test]
    fn provider_retry_defaults_are_stable() {
        let config = ProviderConfig::default();
        assert_eq!(config.request_timeout_ms, 30_000);
        assert_eq!(config.retry_max_attempts, 3);
        assert_eq!(config.retry_initial_backoff_ms, 300);
        assert_eq!(config.retry_max_backoff_ms, 3_000);
    }

    #[test]
    fn provider_default_model_uses_auto_discovery() {
        let config = ProviderConfig::default();
        assert_eq!(config.model, "auto");
        assert!(config.model_selection_requires_fetch());
    }

    #[test]
    fn turn_loop_policy_defaults_are_stable() {
        let config = LoongConfig::default();
        assert_eq!(
            config
                .conversation
                .turn_loop
                .max_followup_tool_payload_chars,
            8_000
        );
        assert_eq!(
            config
                .conversation
                .turn_loop
                .max_followup_tool_payload_chars_total,
            20_000
        );
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn turn_loop_policy_can_be_overridden_from_toml() {
        let raw = r#"
[conversation.turn_loop]
max_followup_tool_payload_chars = 1200
max_followup_tool_payload_chars_total = 3200
"#;
        let parsed =
            toml::from_str::<LoongConfig>(raw).expect("parse turn-loop config should pass");
        assert_eq!(
            parsed
                .conversation
                .turn_loop
                .max_followup_tool_payload_chars,
            1200
        );
        assert_eq!(
            parsed
                .conversation
                .turn_loop
                .max_followup_tool_payload_chars_total,
            3200
        );
    }

    #[test]
    fn models_endpoint_resolution_for_supported_provider_profiles_is_stable() {
        let cases = vec![
            (
                ProviderKind::Anthropic,
                "https://api.anthropic.com/v1/models",
            ),
            (ProviderKind::Kimi, "https://api.moonshot.cn/v1/models"),
            (
                ProviderKind::KimiCoding,
                "https://api.kimi.com/coding/v1/models",
            ),
            (ProviderKind::Minimax, "https://api.minimaxi.com/v1/models"),
            (ProviderKind::Ollama, "http://127.0.0.1:11434/v1/models"),
            (ProviderKind::Openai, "https://api.openai.com/v1/models"),
            (
                ProviderKind::OpencodeGo,
                "https://opencode.ai/zen/go/v1/models",
            ),
            (
                ProviderKind::OpencodeZen,
                "https://opencode.ai/zen/v1/models",
            ),
            (
                ProviderKind::Openrouter,
                "https://openrouter.ai/api/v1/models",
            ),
            (
                ProviderKind::Volcengine,
                "https://ark.cn-beijing.volces.com/api/v3/models",
            ),
            (ProviderKind::Xai, "https://api.x.ai/v1/language-models"),
            (ProviderKind::Xiaomi, "https://api.xiaomimimo.com/v1/models"),
            (ProviderKind::Zai, "https://api.z.ai/api/paas/v4/models"),
            (
                ProviderKind::Zhipu,
                "https://open.bigmodel.cn/api/paas/v4/models",
            ),
        ];
        for (kind, expected) in cases {
            let config = ProviderConfig {
                kind,
                ..ProviderConfig::default()
            };
            assert_eq!(config.models_endpoint(), expected, "kind={kind:?}");
        }
    }

    #[test]
    fn kimi_coding_header_lookup_is_case_insensitive() {
        let config = ProviderConfig {
            kind: ProviderKind::KimiCoding,
            headers: [("User-Agent".to_owned(), "KimiCLI/custom".to_owned())]
                .into_iter()
                .collect(),
            ..ProviderConfig::default()
        };
        assert_eq!(config.header_value("user-agent"), Some("KimiCLI/custom"));
        assert_eq!(config.header_value("USER-AGENT"), Some("KimiCLI/custom"));
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn conversation_context_engine_field_parses_and_normalizes() {
        let raw = r#"
[conversation]
context_engine = " Legacy "
"#;
        let parsed = toml::from_str::<LoongConfig>(raw).expect("parse conversation context_engine");
        assert_eq!(
            parsed.conversation.context_engine_id().as_deref(),
            Some("legacy")
        );
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn conversation_turn_middlewares_field_parses_and_normalizes() {
        let raw = r#"
[conversation]
turn_middlewares = [" Alpha ", "beta", "", "alpha"]
"#;
        let parsed =
            toml::from_str::<LoongConfig>(raw).expect("parse conversation turn_middlewares");
        assert_eq!(
            parsed.conversation.turn_middleware_ids(),
            vec!["alpha".to_owned(), "beta".to_owned()]
        );
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn memory_system_field_parses_and_normalizes() {
        let raw = r#"
[memory]
system = " Builtin "
"#;
        let parsed = toml::from_str::<LoongConfig>(raw).expect("parse memory.system");
        assert_eq!(parsed.memory.resolved_system().as_str(), "builtin");
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn memory_system_field_parses_recall_first_variant() {
        let raw = r#"
[memory]
system = " recall_first "
"#;
        let parsed = toml::from_str::<LoongConfig>(raw).expect("parse recall_first memory.system");
        assert_eq!(parsed.memory.resolved_system().as_str(), "recall_first");
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn memory_system_id_field_parses_and_normalizes() {
        let raw = r#"
[memory]
system_id = " LuCid "
"#;
        let parsed = toml::from_str::<LoongConfig>(raw).expect("parse memory.system_id");
        assert_eq!(parsed.memory.system_id.as_deref(), Some("lucid"));
        assert_eq!(parsed.memory.resolved_system_id(), "lucid");
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn memory_system_field_rejects_unimplemented_future_variant() {
        let raw = r#"
[memory]
system = " LuCid "
"#;
        let error = toml::from_str::<LoongConfig>(raw).expect_err("lucid should stay unsupported");
        assert!(
            error
                .to_string()
                .contains("available: builtin, workspace_recall, recall_first"),
            "error should expose the supported memory-system surface: {error}"
        );
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn conversation_compaction_fields_parse_and_gate_compact_hook() {
        let raw = r#"
[conversation]
compact_enabled = true
compact_min_messages = 6
compact_trigger_estimated_tokens = 120
compact_preserve_recent_turns = 4
compact_preserve_recent_estimated_tokens = 96
compact_fail_open = false
"#;
        let parsed =
            toml::from_str::<LoongConfig>(raw).expect("parse conversation compaction config");
        assert!(parsed.conversation.compact_enabled);
        assert_eq!(parsed.conversation.compact_min_messages(), Some(6));
        assert_eq!(
            parsed.conversation.compact_trigger_estimated_tokens(),
            Some(120)
        );
        assert_eq!(parsed.conversation.compact_preserve_recent_turns(), 4);
        assert_eq!(
            parsed
                .conversation
                .compact_preserve_recent_estimated_tokens(),
            Some(96)
        );
        assert!(!parsed.conversation.compaction_fail_open());
        assert!(!parsed.conversation.should_compact(5));
        assert!(parsed.conversation.should_compact(6));
        assert!(
            !parsed
                .conversation
                .should_compact_with_estimate(0, Some(119))
        );
        assert!(
            parsed
                .conversation
                .should_compact_with_estimate(0, Some(120))
        );
    }

    #[test]
    fn conversation_compaction_defaults_require_explicit_thresholds() {
        let config = ConversationConfig::default();
        assert!(config.turn_middleware_ids().is_empty());
        assert!(config.compact_enabled);
        assert!(config.compaction_fail_open());
        assert_eq!(config.compact_preserve_recent_turns(), 6);
        assert_eq!(config.compact_trigger_estimated_tokens(), None);
        assert_eq!(config.compact_preserve_recent_estimated_tokens(), None);
        assert!(!config.should_compact(0));
        assert!(!config.should_compact_with_estimate(0, None));
        assert!(!config.should_compact_with_estimate(100, Some(10_000)));
    }

    #[test]
    fn conversation_compaction_enabled_without_thresholds_does_not_trigger() {
        let config = ConversationConfig {
            compact_enabled: true,
            compact_min_messages: None,
            compact_trigger_estimated_tokens: None,
            compact_fail_open: true,
            context_engine: None,
            ..ConversationConfig::default()
        };

        assert!(!config.should_compact(1));
        assert!(!config.should_compact_with_estimate(100, Some(10_000)));
    }

    #[test]
    fn conversation_compaction_token_gate_without_message_threshold() {
        let config = ConversationConfig {
            compact_enabled: true,
            compact_min_messages: None,
            compact_trigger_estimated_tokens: Some(50),
            compact_fail_open: true,
            context_engine: None,
            ..ConversationConfig::default()
        };
        assert!(!config.should_compact_with_estimate(0, Some(49)));
        assert!(config.should_compact_with_estimate(0, Some(50)));
        assert!(!config.should_compact_with_estimate(100, None));
        assert_eq!(config.compact_preserve_recent_estimated_tokens(), Some(50));
    }

    #[test]
    fn conversation_compaction_recent_token_budget_falls_back_to_trigger_threshold() {
        let mut config = ConversationConfig {
            compact_trigger_estimated_tokens: Some(120),
            ..ConversationConfig::default()
        };

        assert_eq!(config.compact_preserve_recent_estimated_tokens(), Some(120));

        config.compact_preserve_recent_estimated_tokens = Some(64);
        assert_eq!(config.compact_preserve_recent_estimated_tokens(), Some(64));
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn acp_fields_parse_and_normalize() {
        let raw = r#"
[acp]
enabled = true
backend = " ACPX "
default_agent = " Claude "
allowed_agents = ["Codex", "claude", " gemini "]
max_concurrent_sessions = 12
session_idle_ttl_ms = 45000
startup_timeout_ms = 12000
turn_timeout_ms = 99000
queue_owner_ttl_ms = 7000
bindings_enabled = true
emit_runtime_events = true
allow_mcp_server_injection = true

[acp.dispatch]
bootstrap_mcp_servers = [" Filesystem ", "search", "filesystem"]
working_directory = " /workspace/dispatch "

[acp.backends.acpx]
command = " /usr/local/bin/acpx "
expected_version = " 0.1.16 "
cwd = " /workspace/project "
permission_mode = " approve-reads "
non_interactive_permissions = " fail "
strict_windows_cmd_wrapper = true
timeout_seconds = 45.5
queue_owner_ttl_seconds = 0.25

[acp.backends.acpx.mcp_servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/workspace/project"]

[acp.backends.acpx.mcp_servers.filesystem.env]
MCP_LOG = "warn"
"#;
        let parsed = toml::from_str::<LoongConfig>(raw).expect("parse ACP config");
        assert!(parsed.acp.enabled);
        assert_eq!(parsed.acp.backend_id().as_deref(), Some("acpx"));
        assert_eq!(parsed.acp.resolved_default_agent().as_deref(), Ok("claude"));
        assert_eq!(
            parsed.acp.allowed_agent_ids(),
            Ok(vec![
                "codex".to_owned(),
                "claude".to_owned(),
                "gemini".to_owned()
            ])
        );
        assert_eq!(parsed.acp.max_concurrent_sessions(), 12);
        assert_eq!(parsed.acp.session_idle_ttl_ms(), 45_000);
        assert_eq!(parsed.acp.startup_timeout_ms(), 12_000);
        assert_eq!(parsed.acp.turn_timeout_ms(), 99_000);
        assert_eq!(parsed.acp.queue_owner_ttl_ms(), 7_000);
        assert!(parsed.acp.bindings_enabled);
        assert!(parsed.acp.emit_runtime_events);
        assert!(parsed.acp.allow_mcp_server_injection);
        assert_eq!(
            parsed.acp.dispatch.bootstrap_mcp_server_names(),
            Ok(vec!["filesystem".to_owned(), "search".to_owned()])
        );
        assert_eq!(
            parsed.acp.dispatch.resolved_working_directory(),
            Some(std::path::PathBuf::from("/workspace/dispatch"))
        );
        let acpx = parsed
            .acp
            .acpx_profile()
            .expect("acpx profile should parse from backend-local config");
        assert_eq!(acpx.command().as_deref(), Some("/usr/local/bin/acpx"));
        assert_eq!(acpx.expected_version().as_deref(), Some("0.1.16"));
        assert_eq!(acpx.cwd().as_deref(), Some("/workspace/project"));
        assert_eq!(acpx.permission_mode().as_deref(), Some("approve-reads"));
        assert_eq!(acpx.non_interactive_permissions().as_deref(), Some("fail"));
        assert_eq!(acpx.strict_windows_cmd_wrapper, Some(true));
        assert_eq!(acpx.timeout_seconds, Some(45.5));
        assert_eq!(acpx.queue_owner_ttl_seconds, Some(0.25));
        let mcp = acpx
            .mcp_servers
            .get("filesystem")
            .expect("filesystem MCP server should parse");
        assert_eq!(mcp.command, "npx");
        assert_eq!(
            mcp.args,
            vec![
                "-y".to_owned(),
                "@modelcontextprotocol/server-filesystem".to_owned(),
                "/workspace/project".to_owned()
            ]
        );
        assert_eq!(mcp.env.get("MCP_LOG").map(String::as_str), Some("warn"));
    }

    #[test]
    fn acp_dispatch_bootstrap_mcp_server_names_with_additions_merge_and_dedupe() {
        let dispatch = AcpDispatchConfig {
            bootstrap_mcp_servers: vec![" Filesystem ".to_owned()],
            ..AcpDispatchConfig::default()
        };

        let resolved = dispatch
            .bootstrap_mcp_server_names_with_additions(&[
                " search ".to_owned(),
                "filesystem".to_owned(),
                "SEARCH".to_owned(),
            ])
            .expect("merged bootstrap MCP server names should normalize");

        assert_eq!(resolved, vec!["filesystem".to_owned(), "search".to_owned()]);
    }

    #[test]
    fn acp_defaults_are_control_plane_safe() {
        let config = AcpConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.backend_id(), None);
        assert_eq!(config.resolved_default_agent().as_deref(), Ok("codex"));
        assert_eq!(config.allowed_agent_ids(), Ok(vec!["codex".to_owned()]));
        assert_eq!(config.max_concurrent_sessions(), 8);
        assert_eq!(config.session_idle_ttl_ms(), 900_000);
        assert_eq!(config.startup_timeout_ms(), 15_000);
        assert_eq!(config.turn_timeout_ms(), 120_000);
        assert_eq!(config.queue_owner_ttl_ms(), 30_000);
        assert!(!config.bindings_enabled);
        assert!(!config.emit_runtime_events);
        assert!(config.allow_mcp_server_injection);
        assert!(config.acpx_profile().is_none());
    }

    #[test]
    fn control_plane_defaults_are_loopback_safe() {
        let config = ControlPlaneConfig::default();
        assert!(!config.allow_remote);
        assert_eq!(config.resolved_shared_token(), Ok(None));
    }

    #[test]
    fn acp_allowed_agents_must_include_default_agent() {
        let config = AcpConfig {
            default_agent: Some("claude".to_owned()),
            allowed_agents: vec!["codex".to_owned()],
            ..AcpConfig::default()
        };

        let error = config
            .allowed_agent_ids()
            .expect_err("default ACP agent must be included in allowlist");
        assert!(error.contains("default agent"));
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn acp_dispatch_fields_parse_and_preserve_backward_compatible_defaults() {
        let raw = r#"
[acp]
enabled = true

[acp.dispatch]
enabled = false
conversation_routing = "agent_prefixed_only"
allowed_channels = [" Telegram ", "feishu"]
allowed_account_ids = [" Work Bot ", "ops-bot"]
thread_routing = "thread_only"
"#;
        let parsed = toml::from_str::<LoongConfig>(raw).expect("parse ACP dispatch config");
        assert!(parsed.acp.enabled);
        assert!(!parsed.acp.dispatch.enabled);
        assert_eq!(
            parsed.acp.dispatch.conversation_routing,
            AcpConversationRoutingMode::AgentPrefixedOnly
        );
        assert_eq!(
            parsed.acp.dispatch.allowed_channel_ids(),
            Ok(vec!["telegram".to_owned(), "feishu".to_owned()])
        );
        assert_eq!(
            parsed.acp.dispatch.allowed_account_ids(),
            Ok(vec!["work-bot".to_owned(), "ops-bot".to_owned()])
        );
        assert_eq!(
            parsed.acp.dispatch.thread_routing,
            AcpDispatchThreadRoutingMode::ThreadOnly
        );
    }

    #[test]
    fn acp_dispatch_defaults_keep_normal_sessions_off_without_explicit_acp_route() {
        let config = AcpConfig::default();
        assert!(config.dispatch.enabled);
        assert_eq!(
            config.dispatch.conversation_routing,
            AcpConversationRoutingMode::AgentPrefixedOnly
        );
        assert_eq!(config.dispatch.allowed_channel_ids(), Ok(Vec::new()));
        assert_eq!(config.dispatch.allowed_account_ids(), Ok(Vec::new()));
        assert_eq!(
            config.dispatch.thread_routing,
            AcpDispatchThreadRoutingMode::All
        );
        assert_eq!(config.dispatch.bootstrap_mcp_server_names(), Ok(Vec::new()));
        assert_eq!(config.dispatch.resolved_working_directory(), None);
    }

    #[test]
    fn acp_dispatch_normalizes_bootstrap_mcp_server_names() {
        let config = AcpConfig {
            dispatch: AcpDispatchConfig {
                bootstrap_mcp_servers: vec![
                    " Filesystem ".to_owned(),
                    "search".to_owned(),
                    "filesystem".to_owned(),
                ],
                ..AcpDispatchConfig::default()
            },
            ..AcpConfig::default()
        };

        assert_eq!(
            config.dispatch.bootstrap_mcp_server_names(),
            Ok(vec!["filesystem".to_owned(), "search".to_owned()])
        );
    }

    #[test]
    fn acp_dispatch_normalizes_working_directory() {
        let config = AcpConfig {
            dispatch: AcpDispatchConfig {
                working_directory: Some(" /workspace/dispatch ".to_owned()),
                ..AcpDispatchConfig::default()
            },
            ..AcpConfig::default()
        };

        assert_eq!(
            config.dispatch.resolved_working_directory(),
            Some(std::path::PathBuf::from("/workspace/dispatch"))
        );
    }

    #[test]
    fn acp_dispatch_rejects_invalid_allowed_channel_ids() {
        let config = AcpConfig {
            dispatch: AcpDispatchConfig {
                allowed_channels: vec!["***".to_owned()],
                ..AcpDispatchConfig::default()
            },
            ..AcpConfig::default()
        };

        let error = config
            .dispatch
            .allowed_channel_ids()
            .expect_err("invalid ACP dispatch channel ids must be rejected");
        assert!(error.contains("allowed channel"));
    }

    #[test]
    fn acp_dispatch_normalizes_allowed_account_ids() {
        let config = AcpConfig {
            dispatch: AcpDispatchConfig {
                allowed_account_ids: vec![
                    " Work Bot ".to_owned(),
                    "ops-bot".to_owned(),
                    "OPS BOT".to_owned(),
                ],
                ..AcpDispatchConfig::default()
            },
            ..AcpConfig::default()
        };

        assert_eq!(
            config.dispatch.allowed_account_ids(),
            Ok(vec!["work-bot".to_owned(), "ops-bot".to_owned()])
        );
    }

    #[test]
    fn acp_dispatch_rejects_invalid_allowed_account_ids() {
        let config = AcpConfig {
            dispatch: AcpDispatchConfig {
                allowed_account_ids: vec!["***".to_owned()],
                ..AcpDispatchConfig::default()
            },
            ..AcpConfig::default()
        };

        let error = config
            .dispatch
            .allowed_account_ids()
            .expect_err("invalid ACP dispatch account ids must be rejected");
        assert!(error.contains("allowed account"));
    }
}
