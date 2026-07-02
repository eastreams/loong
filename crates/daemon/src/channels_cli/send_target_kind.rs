use super::*;

pub fn default_twitch_send_target_kind() -> app::channel::ChannelOutboundTargetKind {
    default_channel_send_target_kind(TWITCH_SEND_CLI_SPEC)
}

pub fn parse_twitch_send_target_kind(
    raw: &str,
) -> Result<app::channel::ChannelOutboundTargetKind, String> {
    parse_channel_send_target_kind(TWITCH_SEND_CLI_SPEC, raw)
}
