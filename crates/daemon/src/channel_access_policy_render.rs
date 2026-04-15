use std::collections::BTreeMap;

use crate::mvp;

pub(crate) fn channel_access_policy_by_account(
    inventory: &mvp::channel::ChannelInventory,
) -> BTreeMap<(String, String), mvp::channel::ChannelConfiguredAccountAccessPolicy> {
    let mut policies = BTreeMap::new();
    for access_policy in &inventory.channel_access_policies {
        let key = (
            access_policy.channel_id.to_owned(),
            access_policy.configured_account_id.clone(),
        );
        policies.insert(key, access_policy.clone());
    }
    policies
}

pub(crate) fn render_channel_access_policy_line(
    access_policy: &mvp::channel::ChannelConfiguredAccountAccessPolicy,
) -> String {
    let conversations =
        render_channel_access_policy_values(access_policy.summary.allowed_conversations.as_slice());
    let senders =
        render_channel_access_policy_values(access_policy.summary.allowed_senders.as_slice());
    let conversation_mode =
        render_channel_access_restriction_mode(access_policy.summary.conversation_mode);
    let sender_mode = render_channel_access_restriction_mode(access_policy.summary.sender_mode);

    format!(
        "    policy conversation_key={} conversation_mode={} sender_key={} sender_mode={} mention_required={} conversations={} senders={}",
        access_policy.conversation_config_key,
        conversation_mode,
        access_policy.sender_config_key,
        sender_mode,
        access_policy.summary.mention_required,
        conversations,
        senders,
    )
}

fn render_channel_access_policy_values(values: &[String]) -> String {
    if values.is_empty() {
        return "-".to_owned();
    }
    values.join(",")
}

fn render_channel_access_restriction_mode(
    mode: mvp::channel::ChannelAccessRestrictionMode,
) -> &'static str {
    match mode {
        mvp::channel::ChannelAccessRestrictionMode::Open => "open",
        mvp::channel::ChannelAccessRestrictionMode::ExactAllowlist => "exact_allowlist",
        mvp::channel::ChannelAccessRestrictionMode::WildcardAllowlist => "wildcard_allowlist",
    }
}
