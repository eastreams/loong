pub(crate) mod channels;
pub(crate) mod discovery;
pub(crate) mod planner;
pub(crate) mod provider_selection;
pub(crate) mod provider_transport;
pub(crate) mod render;
pub(crate) mod types;

#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use discovery::collect_import_candidates_with_paths;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use discovery::collect_import_surfaces;
#[allow(unused_imports)]
pub(crate) use discovery::{
    build_import_candidate, classify_current_setup,
    collect_import_candidates_with_paths_and_readiness,
    collect_import_surfaces_with_channel_readiness,
    detect_import_starting_config_with_channel_readiness, detect_workspace_guidance,
    resolve_channel_import_readiness_from_config,
};
#[allow(unused_imports)]
pub(crate) use planner::{
    compose_recommended_import_candidate, prepend_recommended_import_candidate,
};
#[allow(unused_imports)]
pub(crate) use provider_selection::{
    ImportedProviderChoice, ProviderSelectionPlan, build_provider_selection_plan_for_candidate,
    resolve_provider_config_from_selection,
};
#[allow(unused_imports)]
pub(crate) use types::{
    ChannelCandidate, ChannelCredentialState, ChannelImportReadiness, CurrentSetupState,
    DomainPreview, ImportCandidate, ImportSourceKind, ImportSurface, ImportSurfaceLevel,
    PreviewStatus, SetupDomainKind, WorkspaceGuidanceCandidate, WorkspaceGuidanceKind,
};
