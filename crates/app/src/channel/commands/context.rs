use std::path::PathBuf;

use crate::config::ChannelResolvedAccountRoute;
use crate::config::LoongConfig;
#[cfg(feature = "channel-feishu")]
use crate::config::ResolvedFeishuChannelConfig;
#[cfg(feature = "channel-line")]
use crate::config::ResolvedLineChannelConfig;
#[cfg(feature = "channel-matrix")]
use crate::config::ResolvedMatrixChannelConfig;
#[cfg(feature = "channel-qqbot")]
use crate::config::ResolvedQqbotChannelConfig;
#[cfg(feature = "channel-telegram")]
use crate::config::ResolvedTelegramChannelConfig;
#[cfg(feature = "channel-webhook")]
use crate::config::ResolvedWebhookChannelConfig;
#[cfg(feature = "channel-wecom")]
use crate::config::ResolvedWecomChannelConfig;

use super::super::http;

#[derive(Debug, Clone)]
pub(in crate::channel) struct ChannelCommandContext<R> {
    pub(in crate::channel) resolved_path: PathBuf,
    pub(in crate::channel) config: LoongConfig,
    pub(in crate::channel) resolved: R,
    pub(in crate::channel) route: ChannelResolvedAccountRoute,
}

impl<R> ChannelCommandContext<R> {
    pub(in crate::channel) fn emit_route_notice(&self, channel_id: &str) {
        if let Some(notice) = render_channel_route_notice(channel_id, &self.route) {
            #[allow(clippy::print_stderr)]
            {
                eprintln!("warning: {notice}");
            }
        }
    }

    pub(in crate::channel) fn outbound_http_policy(&self) -> http::ChannelOutboundHttpPolicy {
        http::outbound_http_policy_from_config(&self.config)
    }
}

pub(in crate::channel) trait ChannelResolvedRuntimeAccount {
    fn runtime_account_id(&self) -> &str;
    fn runtime_account_label(&self) -> &str;
}

#[cfg(feature = "channel-telegram")]
impl ChannelResolvedRuntimeAccount for ResolvedTelegramChannelConfig {
    fn runtime_account_id(&self) -> &str {
        self.account.id.as_str()
    }

    fn runtime_account_label(&self) -> &str {
        self.account.label.as_str()
    }
}

#[cfg(feature = "channel-feishu")]
impl ChannelResolvedRuntimeAccount for ResolvedFeishuChannelConfig {
    fn runtime_account_id(&self) -> &str {
        self.account.id.as_str()
    }

    fn runtime_account_label(&self) -> &str {
        self.account.label.as_str()
    }
}

#[cfg(feature = "channel-line")]
impl ChannelResolvedRuntimeAccount for ResolvedLineChannelConfig {
    fn runtime_account_id(&self) -> &str {
        self.account.id.as_str()
    }

    fn runtime_account_label(&self) -> &str {
        self.account.label.as_str()
    }
}

#[cfg(feature = "channel-matrix")]
impl ChannelResolvedRuntimeAccount for ResolvedMatrixChannelConfig {
    fn runtime_account_id(&self) -> &str {
        self.account.id.as_str()
    }

    fn runtime_account_label(&self) -> &str {
        self.account.label.as_str()
    }
}

#[cfg(feature = "channel-wecom")]
impl ChannelResolvedRuntimeAccount for ResolvedWecomChannelConfig {
    fn runtime_account_id(&self) -> &str {
        self.account.id.as_str()
    }

    fn runtime_account_label(&self) -> &str {
        self.account.label.as_str()
    }
}

#[cfg(feature = "channel-webhook")]
impl ChannelResolvedRuntimeAccount for ResolvedWebhookChannelConfig {
    fn runtime_account_id(&self) -> &str {
        self.account.id.as_str()
    }

    fn runtime_account_label(&self) -> &str {
        self.account.label.as_str()
    }
}

#[cfg(feature = "channel-qqbot")]
impl ChannelResolvedRuntimeAccount for ResolvedQqbotChannelConfig {
    fn runtime_account_id(&self) -> &str {
        self.account.id.as_str()
    }

    fn runtime_account_label(&self) -> &str {
        self.account.label.as_str()
    }
}

pub(in crate::channel) fn render_channel_route_notice(
    channel_id: &str,
    route: &ChannelResolvedAccountRoute,
) -> Option<String> {
    if !route.uses_implicit_fallback_default() {
        return None;
    }
    let config_key = channel_id.replace('-', "_");
    Some(format!(
        "{} omitted --account and routed to configured account `{}` via fallback default selection; set {}.default_account or pass --account to avoid routing surprises",
        channel_id, route.selected_configured_account_id, config_key
    ))
}
