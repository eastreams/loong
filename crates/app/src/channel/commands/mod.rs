pub(in crate::channel) mod context;
mod send;
mod serve;

pub(super) use context::ChannelCommandContext;
pub(super) use send::{ChannelSendCommandSpec, run_channel_send_command};
#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-qqbot",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
    feature = "channel-webhook"
))]
pub(super) use serve::{ChannelServeCommandSpec, run_channel_serve_command_with_stop};
