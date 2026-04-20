pub mod channels;
pub mod discovery;
pub mod planner;
pub mod provider_selection;
pub mod provider_transport;
pub mod render;
pub mod types;

#[allow(unused_imports)]
pub use discovery::collect_import_candidates_with_paths;
#[allow(unused_imports)]
pub use discovery::collect_import_candidates_with_paths_and_readiness;
#[allow(unused_imports)]
pub use discovery::collect_import_surfaces;
#[allow(unused_imports)]
pub use discovery::{
    build_import_candidate, classify_current_setup, collect_import_surfaces_with_channel_readiness,
    detect_import_starting_config_with_channel_readiness, detect_workspace_guidance,
    resolve_channel_import_readiness_from_config,
};
#[allow(unused_imports)]
pub use planner::{compose_recommended_import_candidate, prepend_recommended_import_candidate};
#[allow(unused_imports)]
pub use provider_selection::resolve_choice_by_selector;
#[allow(unused_imports)]
pub use provider_selection::{
    ImportedChoiceSelectorResolution, ImportedProviderChoice, ProviderSelectionPlan,
    accepted_selectors_for_choice, build_provider_selection_plan_for_candidate, describe_choice,
    describe_matching_choices, format_ambiguous_selector_error, format_unknown_selector_error,
    guidance_lines, preferred_selector_for_choice, recommendation_hint,
    recommendation_hint_for_profile_ids, resolve_choice_by_selector_resolution,
    resolve_provider_config_from_selection, selector_catalog, unresolved_choice_note_segments,
};
#[allow(unused_imports)]
pub use types::{
    ChannelCandidate, ChannelCredentialState, ChannelImportReadiness, CurrentSetupState,
    DomainPreview, ImportCandidate, ImportSourceKind, ImportSurface, ImportSurfaceLevel,
    PreviewStatus, SetupDomainKind, WorkspaceGuidanceCandidate, WorkspaceGuidanceKind,
};
