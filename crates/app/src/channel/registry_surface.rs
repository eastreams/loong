use std::collections::BTreeMap;

use super::*;

pub(super) fn build_channel_surfaces(
    channel_catalog: &[ChannelCatalogEntry],
    channels: &[ChannelStatusSnapshot],
    plugin_bridge_discovery_by_id: &BTreeMap<&'static str, ChannelPluginBridgeDiscovery>,
) -> Vec<ChannelSurface> {
    channel_catalog
        .iter()
        .map(|catalog| {
            let configured_accounts = channels
                .iter()
                .filter(|snapshot| snapshot.id == catalog.id)
                .cloned()
                .collect::<Vec<_>>();
            let default_configured_account_id = configured_accounts
                .iter()
                .find(|snapshot| snapshot.is_default_account)
                .map(|snapshot| snapshot.configured_account_id.clone());
            let plugin_bridge_discovery = plugin_bridge_discovery_by_id.get(catalog.id).cloned();

            ChannelSurface {
                catalog: catalog.clone(),
                configured_accounts,
                default_configured_account_id,
                plugin_bridge_discovery,
            }
        })
        .collect()
}
