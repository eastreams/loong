use clap::{Args, Subcommand};

use crate::{
    ChannelSendCliArgs, ChannelSendCliSpec, ChannelServeCliArgs, ChannelServeCliSpec, CliResult,
    DINGTALK_SEND_CLI_SPEC, DISCORD_SEND_CLI_SPEC, EMAIL_SEND_CLI_SPEC, FEISHU_SEND_CLI_SPEC,
    FEISHU_SERVE_CLI_SPEC, GOOGLE_CHAT_SEND_CLI_SPEC, IMESSAGE_SEND_CLI_SPEC, IRC_SEND_CLI_SPEC,
    LINE_SEND_CLI_SPEC, LINE_SERVE_CLI_SPEC, MATRIX_SEND_CLI_SPEC, MATRIX_SERVE_CLI_SPEC,
    MATTERMOST_SEND_CLI_SPEC, NEXTCLOUD_TALK_SEND_CLI_SPEC, NOSTR_SEND_CLI_SPEC,
    ONEBOT_SEND_CLI_SPEC, ONEBOT_SERVE_CLI_SPEC, QQBOT_SEND_CLI_SPEC, QQBOT_SERVE_CLI_SPEC,
    SIGNAL_SEND_CLI_SPEC, SLACK_SEND_CLI_SPEC, SYNOLOGY_CHAT_SEND_CLI_SPEC, TEAMS_SEND_CLI_SPEC,
    TELEGRAM_SEND_CLI_SPEC, TELEGRAM_SERVE_CLI_SPEC, TLON_SEND_CLI_SPEC, TWITCH_SEND_CLI_SPEC,
    WEBHOOK_SEND_CLI_SPEC, WEBHOOK_SERVE_CLI_SPEC, WECOM_SEND_CLI_SPEC, WECOM_SERVE_CLI_SPEC,
    WEIXIN_SEND_CLI_SPEC, WEIXIN_SERVE_CLI_SPEC, WHATSAPP_PERSONAL_SEND_CLI_SPEC,
    WHATSAPP_PERSONAL_SERVE_CLI_SPEC, WHATSAPP_SEND_CLI_SPEC, WHATSAPP_SERVE_CLI_SPEC,
    default_channel_send_target_kind, parse_channel_send_target_kind, run_channel_send_cli,
    run_channel_serve_cli, run_channels_cli,
};

pub use loong_app as mvp;

#[derive(Subcommand, Debug)]
pub enum ChannelsCommands {
    /// List compiled channel surfaces, aliases, and readiness status
    List(ChannelsListArgs),
    /// Resolve one channel id or alias through the catalog and runtime inventory
    Resolve(ChannelsResolveArgs),
    /// Send one proactive message through the canonical channel surface
    Send(ChannelsSendArgs),
    /// Run or control one channel serve loop through the canonical channel surface
    Serve(ChannelsServeArgs),
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct ChannelsListArgs {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct ChannelsResolveArgs {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
    #[arg(value_name = "CHANNEL")]
    pub channel: String,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct ChannelsSendArgs {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub account: Option<String>,
    #[arg(value_name = "CHANNEL", conflicts_with = "channel_name")]
    pub channel: Option<String>,
    #[arg(long = "channel", value_name = "CHANNEL", conflicts_with = "channel")]
    pub channel_name: Option<String>,
    #[arg(long)]
    pub target: String,
    #[arg(long = "target-kind")]
    pub target_kind: Option<String>,
    #[arg(long)]
    pub text: String,
    #[arg(long, default_value_t = false)]
    pub card: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct ChannelsServeArgs {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub account: Option<String>,
    #[arg(value_name = "CHANNEL", conflicts_with = "channel_name")]
    pub channel: Option<String>,
    #[arg(long = "channel", value_name = "CHANNEL", conflicts_with = "channel")]
    pub channel_name: Option<String>,
    #[arg(long, default_value_t = false)]
    pub once: bool,
    #[arg(long, default_value_t = false, conflicts_with = "once")]
    pub stop: bool,
    #[arg(long, default_value_t = false, conflicts_with_all = ["once", "stop"])]
    pub stop_duplicates: bool,
    #[arg(long)]
    pub bind: Option<String>,
    #[arg(long)]
    pub path: Option<String>,
}

pub async fn run_grouped_channels_cli(
    legacy_config: Option<String>,
    legacy_resolve: Option<String>,
    legacy_json: bool,
    command: Option<ChannelsCommands>,
) -> CliResult<()> {
    match command {
        None => run_channels_cli(
            legacy_config.as_deref(),
            legacy_resolve.as_deref(),
            legacy_json,
        ),
        Some(command) => {
            if legacy_config.is_some() || legacy_resolve.is_some() || legacy_json {
                return Err(
                    "legacy `loong channels` flags cannot be combined with grouped subcommands; \
                     use `loong channels list ...`, `loong channels resolve ...`, `loong channels send ...`, or `loong channels serve ...`"
                        .to_owned(),
                );
            }
            run_channels_command(command).await
        }
    }
}

pub async fn run_channels_command(command: ChannelsCommands) -> CliResult<()> {
    match command {
        ChannelsCommands::List(args) => run_channels_cli(args.config.as_deref(), None, args.json),
        ChannelsCommands::Resolve(args) => run_channels_cli(
            args.config.as_deref(),
            Some(args.channel.as_str()),
            args.json,
        ),
        ChannelsCommands::Send(args) => run_grouped_channel_send(args).await,
        ChannelsCommands::Serve(args) => run_grouped_channel_serve(args).await,
    }
}

async fn run_grouped_channel_send(args: ChannelsSendArgs) -> CliResult<()> {
    let raw_channel =
        resolve_grouped_channel_id(args.channel.as_deref(), args.channel_name.as_deref())?;
    let spec = resolve_channel_send_cli_spec(raw_channel)
        .ok_or_else(|| render_grouped_channel_operation_error(raw_channel, "send", true))?;
    let target_kind = match args.target_kind.as_deref() {
        Some(raw) => parse_channel_send_target_kind(spec, raw)?,
        None => default_channel_send_target_kind(spec),
    };

    run_channel_send_cli(
        spec,
        ChannelSendCliArgs {
            config_path: args.config.as_deref(),
            account: args.account.as_deref(),
            target: Some(args.target.as_str()),
            target_kind,
            text: args.text.as_str(),
            as_card: args.card,
        },
    )
    .await
}

async fn run_grouped_channel_serve(args: ChannelsServeArgs) -> CliResult<()> {
    let raw_channel =
        resolve_grouped_channel_id(args.channel.as_deref(), args.channel_name.as_deref())?;
    let spec = resolve_channel_serve_cli_spec(raw_channel)
        .ok_or_else(|| render_grouped_channel_operation_error(raw_channel, "serve", false))?;

    run_channel_serve_cli(
        spec,
        ChannelServeCliArgs {
            config_path: args.config.as_deref(),
            account: args.account.as_deref(),
            once: args.once,
            stop_requested: args.stop,
            stop_duplicates_requested: args.stop_duplicates,
            bind_override: args.bind.as_deref(),
            path_override: args.path.as_deref(),
        },
    )
    .await
}

fn resolve_grouped_channel_id<'a>(
    positional: Option<&'a str>,
    flagged: Option<&'a str>,
) -> CliResult<&'a str> {
    positional
        .or(flagged)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "canonical grouped channel commands require a channel id; pass `CHANNEL` positionally or `--channel <CHANNEL>`".to_owned())
}

fn render_grouped_channel_operation_error(
    raw_channel: &str,
    operation: &str,
    is_send: bool,
) -> String {
    let Some(normalized) = mvp::channel::normalize_channel_catalog_id(raw_channel) else {
        return format!(
            "unknown channel `{raw_channel}`; run `{} channels` to inspect the available channel catalog",
            crate::CLI_COMMAND_NAME
        );
    };

    let Some(family) = mvp::channel::resolve_channel_catalog_command_family_descriptor(normalized)
    else {
        return format!(
            "channel `{normalized}` does not expose a canonical `{operation}` operation in the catalog"
        );
    };

    let legacy_command = if is_send {
        family.send.command
    } else {
        family.serve.command
    };

    format!(
        "channel `{normalized}` does not support canonical `{} channels {operation}` routing yet; use the dedicated namespace or legacy `{}` command instead",
        crate::CLI_COMMAND_NAME,
        legacy_command
    )
}

fn resolve_channel_send_cli_spec(raw_channel: &str) -> Option<ChannelSendCliSpec> {
    let normalized = mvp::channel::normalize_channel_catalog_id(raw_channel)?;
    Some(match normalized {
        "telegram" => TELEGRAM_SEND_CLI_SPEC,
        "feishu" => FEISHU_SEND_CLI_SPEC,
        "matrix" => MATRIX_SEND_CLI_SPEC,
        "wecom" => WECOM_SEND_CLI_SPEC,
        "weixin" => WEIXIN_SEND_CLI_SPEC,
        "qqbot" => QQBOT_SEND_CLI_SPEC,
        "onebot" => ONEBOT_SEND_CLI_SPEC,
        "whatsapp-personal" => WHATSAPP_PERSONAL_SEND_CLI_SPEC,
        "discord" => DISCORD_SEND_CLI_SPEC,
        "dingtalk" => DINGTALK_SEND_CLI_SPEC,
        "slack" => SLACK_SEND_CLI_SPEC,
        "line" => LINE_SEND_CLI_SPEC,
        "whatsapp" => WHATSAPP_SEND_CLI_SPEC,
        "email" => EMAIL_SEND_CLI_SPEC,
        "webhook" => WEBHOOK_SEND_CLI_SPEC,
        "google_chat" => GOOGLE_CHAT_SEND_CLI_SPEC,
        "teams" => TEAMS_SEND_CLI_SPEC,
        "tlon" => TLON_SEND_CLI_SPEC,
        "signal" => SIGNAL_SEND_CLI_SPEC,
        "twitch" => TWITCH_SEND_CLI_SPEC,
        "mattermost" => MATTERMOST_SEND_CLI_SPEC,
        "nextcloud_talk" => NEXTCLOUD_TALK_SEND_CLI_SPEC,
        "synology_chat" => SYNOLOGY_CHAT_SEND_CLI_SPEC,
        "irc" => IRC_SEND_CLI_SPEC,
        "imessage" => IMESSAGE_SEND_CLI_SPEC,
        "nostr" => NOSTR_SEND_CLI_SPEC,
        _ => return None,
    })
}

fn resolve_channel_serve_cli_spec(raw_channel: &str) -> Option<ChannelServeCliSpec> {
    let normalized = mvp::channel::normalize_channel_catalog_id(raw_channel)?;
    Some(match normalized {
        "telegram" => TELEGRAM_SERVE_CLI_SPEC,
        "feishu" => FEISHU_SERVE_CLI_SPEC,
        "matrix" => MATRIX_SERVE_CLI_SPEC,
        "wecom" => WECOM_SERVE_CLI_SPEC,
        "weixin" => WEIXIN_SERVE_CLI_SPEC,
        "qqbot" => QQBOT_SERVE_CLI_SPEC,
        "onebot" => ONEBOT_SERVE_CLI_SPEC,
        "whatsapp-personal" => WHATSAPP_PERSONAL_SERVE_CLI_SPEC,
        "line" => LINE_SERVE_CLI_SPEC,
        "whatsapp" => WHATSAPP_SERVE_CLI_SPEC,
        "webhook" => WEBHOOK_SERVE_CLI_SPEC,
        _ => return None,
    })
}
