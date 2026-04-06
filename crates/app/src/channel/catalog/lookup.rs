use std::collections::BTreeSet;

use super::{
    CHANNEL_OPERATION_SEND_ID, CHANNEL_OPERATION_SERVE_ID, ChannelCatalogCommandFamilyDescriptor,
    ChannelCatalogOperation, ChannelCommandFamilyDescriptor, ChannelDoctorOperationSpec,
    ChannelOnboardingDescriptor, ChannelOperationDescriptor, ChannelRuntimeCommandDescriptor,
};
use crate::channel::ChannelPlatform;
use crate::channel::registry::{
    ChannelCatalogEntry, ChannelStatusSnapshot, channel_catalog_entry_from_descriptor,
    find_channel_registry_descriptor, sorted_channel_registry_descriptors,
};

pub fn resolve_channel_onboarding_descriptor(raw: &str) -> Option<ChannelOnboardingDescriptor> {
    find_channel_registry_descriptor(raw).map(|descriptor| descriptor.onboarding)
}

pub fn list_channel_catalog() -> Vec<ChannelCatalogEntry> {
    sorted_channel_registry_descriptors()
        .into_iter()
        .map(channel_catalog_entry_from_descriptor)
        .collect()
}

pub(crate) fn resolve_channel_selection_order(raw: &str) -> Option<u16> {
    let descriptor = find_channel_registry_descriptor(raw)?;
    Some(descriptor.selection_order)
}

pub fn normalize_channel_catalog_id(raw: &str) -> Option<&'static str> {
    find_channel_registry_descriptor(raw).map(|descriptor| descriptor.id)
}

pub fn resolve_channel_catalog_entry(raw: &str) -> Option<ChannelCatalogEntry> {
    find_channel_registry_descriptor(raw).map(channel_catalog_entry_from_descriptor)
}

pub fn resolve_channel_catalog_operation(
    raw_channel_id: &str,
    operation_id: &str,
) -> Option<ChannelCatalogOperation> {
    resolve_channel_operation_descriptor(raw_channel_id, operation_id)
        .map(|descriptor| descriptor.operation)
}

pub fn resolve_channel_operation_descriptor(
    raw_channel_id: &str,
    operation_id: &str,
) -> Option<ChannelOperationDescriptor> {
    let descriptor = find_channel_registry_descriptor(raw_channel_id)?
        .operations
        .iter()
        .find(|descriptor| descriptor.operation.id == operation_id)?;
    Some(ChannelOperationDescriptor {
        operation: descriptor.operation,
        doctor: (!descriptor.doctor_checks.is_empty()).then_some(ChannelDoctorOperationSpec {
            checks: descriptor.doctor_checks,
        }),
    })
}

pub fn resolve_channel_doctor_operation_spec(
    raw_channel_id: &str,
    operation_id: &str,
) -> Option<ChannelDoctorOperationSpec> {
    resolve_channel_operation_descriptor(raw_channel_id, operation_id)
        .and_then(|descriptor| descriptor.doctor)
}

pub fn catalog_only_channel_entries(
    snapshots: &[ChannelStatusSnapshot],
) -> Vec<ChannelCatalogEntry> {
    let catalog = list_channel_catalog();
    catalog_only_channel_entries_from(&catalog, snapshots)
}

pub(crate) fn catalog_only_channel_entries_from(
    catalog: &[ChannelCatalogEntry],
    snapshots: &[ChannelStatusSnapshot],
) -> Vec<ChannelCatalogEntry> {
    let snapshot_ids = snapshots
        .iter()
        .map(|snapshot| snapshot.id)
        .collect::<BTreeSet<_>>();
    catalog
        .iter()
        .filter(|entry| !snapshot_ids.contains(entry.id))
        .cloned()
        .collect()
}

pub fn normalize_channel_platform(raw: &str) -> Option<ChannelPlatform> {
    find_channel_registry_descriptor(raw).and_then(|descriptor| {
        descriptor
            .runtime
            .map(|runtime| runtime.family.runtime.platform)
    })
}

pub fn resolve_channel_command_family_descriptor(
    raw: &str,
) -> Option<ChannelCommandFamilyDescriptor> {
    find_channel_registry_descriptor(raw)
        .and_then(|descriptor| descriptor.runtime.map(|runtime| runtime.family))
}

pub fn resolve_channel_catalog_command_family_descriptor(
    raw: &str,
) -> Option<ChannelCatalogCommandFamilyDescriptor> {
    let descriptor = find_channel_registry_descriptor(raw)?;
    let send = descriptor
        .operations
        .iter()
        .find(|descriptor| descriptor.operation.id == CHANNEL_OPERATION_SEND_ID)?
        .operation;
    let serve = descriptor
        .operations
        .iter()
        .find(|descriptor| descriptor.operation.id == CHANNEL_OPERATION_SERVE_ID)?
        .operation;
    Some(ChannelCatalogCommandFamilyDescriptor {
        channel_id: descriptor.id,
        default_send_target_kind: send.default_target_kind()?,
        send,
        serve,
    })
}

pub fn resolve_channel_runtime_command_descriptor(
    raw: &str,
) -> Option<ChannelRuntimeCommandDescriptor> {
    find_channel_registry_descriptor(raw)
        .and_then(|descriptor| descriptor.runtime.map(|runtime| runtime.family.runtime))
}
