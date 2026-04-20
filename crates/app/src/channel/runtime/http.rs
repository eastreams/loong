use super::super::core::http::ChannelOutboundHttpPolicy;
use crate::config::LoongConfig;

pub fn outbound_http_policy_from_config(config: &LoongConfig) -> ChannelOutboundHttpPolicy {
    ChannelOutboundHttpPolicy {
        allow_private_hosts: config.outbound_http.allow_private_hosts,
    }
}
