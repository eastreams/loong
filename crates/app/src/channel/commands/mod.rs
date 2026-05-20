pub(in crate::channel) mod accounts;
pub(in crate::channel) mod context;
mod send;
mod serve;
pub(crate) mod session_send;

pub(in crate::channel) use context::ChannelCommandContext;
pub(in crate::channel) use send::{ChannelSendCommandSpec, run_channel_send_command};
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
pub(in crate::channel) use serve::{ChannelServeCommandSpec, run_channel_serve_command_with_stop};
