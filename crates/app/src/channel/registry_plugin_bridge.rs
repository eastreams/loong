use super::*;

pub(super) fn plugin_bridge_contract_from_descriptor(
    descriptor: &ChannelRegistryDescriptor,
) -> Option<ChannelPluginBridgeContract> {
    let is_plugin_backed =
        descriptor.implementation_status == ChannelCatalogImplementationStatus::PluginBacked;
    let is_plugin_bridge =
        descriptor.onboarding.strategy == ChannelOnboardingStrategy::PluginBridge;

    if !is_plugin_backed {
        return None;
    }

    if !is_plugin_bridge {
        return None;
    }

    let supported_operations = descriptor
        .operations
        .iter()
        .map(|operation| operation.operation.id)
        .collect();
    let recommended_metadata_keys = PLUGIN_BRIDGE_RECOMMENDED_METADATA_KEYS.to_vec();

    Some(ChannelPluginBridgeContract {
        manifest_channel_id: descriptor.id,
        required_setup_surface: PLUGIN_BRIDGE_REQUIRED_SETUP_SURFACE,
        runtime_owner: PLUGIN_BRIDGE_RUNTIME_OWNER,
        supported_operations,
        recommended_metadata_keys,
    })
}

pub fn validate_plugin_channel_bridge_manifest(
    manifest: &loongclaw_kernel::PluginManifest,
) -> Option<ChannelPluginBridgeManifestValidation> {
    let raw_channel_id = manifest.channel_id.as_deref();
    let declared_channel_id = normalized_manifest_channel_id(raw_channel_id)?;
    let registry_descriptor = find_channel_registry_descriptor(&declared_channel_id);

    let Some(registry_descriptor) = registry_descriptor else {
        return Some(ChannelPluginBridgeManifestValidation {
            channel_id: declared_channel_id,
            status: ChannelPluginBridgeManifestStatus::UnknownChannel,
            issues: vec!["channel registry entry is unknown".to_owned()],
            recommended_metadata_keys: Vec::new(),
        });
    };

    let resolved_channel_id = registry_descriptor.id.to_owned();
    let plugin_bridge_contract = plugin_bridge_contract_from_descriptor(registry_descriptor);

    let Some(plugin_bridge_contract) = plugin_bridge_contract else {
        return Some(ChannelPluginBridgeManifestValidation {
            channel_id: resolved_channel_id,
            status: ChannelPluginBridgeManifestStatus::UnsupportedChannelSurface,
            issues: vec!["channel does not accept external plugin bridge ownership".to_owned()],
            recommended_metadata_keys: Vec::new(),
        });
    };

    let setup_surface = normalized_manifest_setup_surface(manifest);

    let Some(setup_surface) = setup_surface else {
        return Some(ChannelPluginBridgeManifestValidation {
            channel_id: resolved_channel_id,
            status: ChannelPluginBridgeManifestStatus::MissingSetupSurface,
            issues: vec!["plugin bridge manifest must declare setup.surface".to_owned()],
            recommended_metadata_keys: plugin_bridge_contract.recommended_metadata_keys,
        });
    };

    let required_setup_surface = plugin_bridge_contract.required_setup_surface.to_owned();
    let setup_surface_matches = setup_surface == required_setup_surface;

    if !setup_surface_matches {
        let issue = format!(
            "plugin bridge manifest declares setup.surface={setup_surface}, expected {required_setup_surface}"
        );

        return Some(ChannelPluginBridgeManifestValidation {
            channel_id: resolved_channel_id,
            status: ChannelPluginBridgeManifestStatus::UnsupportedChannelSurface,
            issues: vec![issue],
            recommended_metadata_keys: plugin_bridge_contract.recommended_metadata_keys,
        });
    }

    Some(ChannelPluginBridgeManifestValidation {
        channel_id: resolved_channel_id,
        status: ChannelPluginBridgeManifestStatus::Compatible,
        issues: Vec::new(),
        recommended_metadata_keys: plugin_bridge_contract.recommended_metadata_keys,
    })
}

fn normalized_manifest_channel_id(raw: Option<&str>) -> Option<String> {
    let raw = raw?;
    let trimmed = raw.trim();

    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_ascii_lowercase())
}

fn normalized_manifest_setup_surface(
    manifest: &loongclaw_kernel::PluginManifest,
) -> Option<String> {
    let setup = manifest.setup.as_ref()?;
    let surface = setup.surface.as_deref()?;
    let trimmed = surface.trim();

    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_ascii_lowercase())
}
