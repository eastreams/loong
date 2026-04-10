use crate::mvp;
use crate::{
    ChannelSendCliSpec, ChannelServeCliSpec, run_dingtalk_send_cli_impl, run_discord_send_cli_impl,
    run_email_send_cli_impl, run_feishu_send_cli_impl, run_feishu_serve_cli_impl,
    run_google_chat_send_cli_impl, run_imessage_send_cli_impl, run_irc_send_cli_impl,
    run_line_send_cli_impl, run_matrix_send_cli_impl, run_matrix_serve_cli_impl,
    run_mattermost_send_cli_impl, run_nextcloud_talk_send_cli_impl, run_nostr_send_cli_impl,
    run_onebot_send_cli_impl, run_onebot_serve_cli_impl, run_qqbot_send_cli_impl,
    run_qqbot_serve_cli_impl, run_signal_send_cli_impl, run_slack_send_cli_impl,
    run_synology_chat_send_cli_impl, run_teams_send_cli_impl, run_telegram_send_cli_impl,
    run_telegram_serve_cli_impl, run_twitch_send_cli_impl, run_webhook_send_cli_impl,
    run_wecom_send_cli_impl, run_wecom_serve_cli_impl, run_weixin_send_cli_impl,
    run_weixin_serve_cli_impl, run_whatsapp_send_cli_impl, run_whatsapp_serve_cli_impl,
};

pub const TELEGRAM_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::TELEGRAM_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_telegram_send_cli_impl,
};

pub const FEISHU_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::FEISHU_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_feishu_send_cli_impl,
};

pub const MATRIX_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::MATRIX_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_matrix_send_cli_impl,
};

pub const WECOM_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::WECOM_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_wecom_send_cli_impl,
};

pub const WEIXIN_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::WEIXIN_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_weixin_send_cli_impl,
};

pub const QQBOT_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::QQBOT_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_qqbot_send_cli_impl,
};

pub const ONEBOT_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::ONEBOT_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_onebot_send_cli_impl,
};

pub const DISCORD_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::DISCORD_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_discord_send_cli_impl,
};

pub const DINGTALK_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::DINGTALK_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_dingtalk_send_cli_impl,
};

pub const SLACK_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::SLACK_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_slack_send_cli_impl,
};

pub const LINE_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::LINE_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_line_send_cli_impl,
};

pub const WHATSAPP_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::WHATSAPP_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_whatsapp_send_cli_impl,
};

pub const EMAIL_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::EMAIL_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_email_send_cli_impl,
};

pub const WEBHOOK_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::WEBHOOK_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_webhook_send_cli_impl,
};

pub const GOOGLE_CHAT_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::GOOGLE_CHAT_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_google_chat_send_cli_impl,
};

pub const TEAMS_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::TEAMS_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_teams_send_cli_impl,
};

pub const SIGNAL_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::SIGNAL_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_signal_send_cli_impl,
};

pub const TWITCH_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::TWITCH_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_twitch_send_cli_impl,
};

pub const MATTERMOST_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::MATTERMOST_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_mattermost_send_cli_impl,
};

pub const NEXTCLOUD_TALK_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::NEXTCLOUD_TALK_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_nextcloud_talk_send_cli_impl,
};

pub const SYNOLOGY_CHAT_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::SYNOLOGY_CHAT_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_synology_chat_send_cli_impl,
};

pub const IRC_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::IRC_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_irc_send_cli_impl,
};

pub const IMESSAGE_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::IMESSAGE_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_imessage_send_cli_impl,
};

pub const NOSTR_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::NOSTR_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_nostr_send_cli_impl,
};

pub const TELEGRAM_SERVE_CLI_SPEC: ChannelServeCliSpec = ChannelServeCliSpec {
    family: mvp::channel::TELEGRAM_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_telegram_serve_cli_impl,
};

pub const FEISHU_SERVE_CLI_SPEC: ChannelServeCliSpec = ChannelServeCliSpec {
    family: mvp::channel::FEISHU_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_feishu_serve_cli_impl,
};

pub const MATRIX_SERVE_CLI_SPEC: ChannelServeCliSpec = ChannelServeCliSpec {
    family: mvp::channel::MATRIX_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_matrix_serve_cli_impl,
};

pub const WECOM_SERVE_CLI_SPEC: ChannelServeCliSpec = ChannelServeCliSpec {
    family: mvp::channel::WECOM_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_wecom_serve_cli_impl,
};

pub const WHATSAPP_SERVE_CLI_SPEC: ChannelServeCliSpec = ChannelServeCliSpec {
    family: mvp::channel::WHATSAPP_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_whatsapp_serve_cli_impl,
};

pub const WEIXIN_SERVE_CLI_SPEC: ChannelServeCliSpec = ChannelServeCliSpec {
    family: mvp::channel::WEIXIN_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_weixin_serve_cli_impl,
};

pub const QQBOT_SERVE_CLI_SPEC: ChannelServeCliSpec = ChannelServeCliSpec {
    family: mvp::channel::QQBOT_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_qqbot_serve_cli_impl,
};

pub const ONEBOT_SERVE_CLI_SPEC: ChannelServeCliSpec = ChannelServeCliSpec {
    family: mvp::channel::ONEBOT_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_onebot_serve_cli_impl,
};
