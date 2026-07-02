use std::collections::BTreeSet;
use std::env;
#[cfg(test)]
use std::io;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::thread;
use std::time::Duration;

use loong_app as mvp;
use loong_contracts::SecretRef;
use loong_spec::CliResult;

use crate::copilot_onboarding::finalize_github_copilot_onboard_credentials;
use crate::onboard::finalize::{
    build_onboarding_success_summary_with_memory, render_onboarding_success_summary_lines,
};
use crate::onboard::model_policy as onboarding_model_policy;
pub use crate::onboard::preflight::{
    OnboardCheck, OnboardCheckLevel, OnboardNonInteractiveWarningPolicy,
    collect_channel_preflight_checks, directory_preflight_check, provider_credential_check,
    render_current_setup_preflight_summary_screen_lines,
    render_detected_setup_preflight_summary_screen_lines, render_preflight_summary_screen_lines,
};
use crate::onboard::preflight::{
    config_validation_failure_message,
    is_explicitly_accepted_non_interactive_warning as preflight_accepts_non_interactive_warning,
    non_interactive_preflight_failure_message, render_preflight_summary_screen_lines_with_progress,
    run_preflight_checks,
};
pub use crate::onboard::types::OnboardingCredentialSummary;
#[cfg(test)]
use crate::onboard::web_search::{
    WebSearchProviderRecommendation, WebSearchProviderRecommendationSource,
    recommend_web_search_provider_from_available_credentials,
};
use crate::onboard::web_search::{
    current_web_search_provider, explicit_web_search_provider_override,
    resolve_effective_web_search_default_provider, resolve_web_search_provider_recommendation,
};
#[cfg(not(test))]
use crate::onboard::write_recovery::OnboardWriteRecovery;
use crate::onboard::write_recovery::{
    ConfigWritePlan, prepare_output_path_for_write, resolve_backup_path,
    rollback_onboard_write_failure,
};
#[cfg(test)]
use crate::onboard::write_recovery::{
    OnboardWriteRecovery, format_backup_timestamp_at, resolve_backup_path_at,
};
use crate::provider::credential_policy as provider_credential_policy;
use crate::query_search_surface::{
    configured_query_search_credential_env_name, configured_query_search_credential_source_value,
    configured_query_search_secret, preferred_query_search_credential_env_default,
    query_search_has_inline_credential, query_search_provider_display_name,
    summarize_query_search_credential,
};
use mvp::tui_surface::{
    TuiCalloutTone, TuiChoiceSpec, TuiHeaderStyle, TuiScreenSpec, TuiSectionSpec,
    render_onboard_screen_spec,
};
#[cfg(test)]
use std::fs;
#[cfg(test)]
use time::OffsetDateTime;

mod select;

pub use crate::onboard::import::{
    ImportCandidate, ImportSurface, ImportSurfaceLevel, OnboardEntryChoice, OnboardEntryOption,
    build_onboard_entry_options,
};
use crate::onboard::import::{
    StartingConfigSelection, default_onboard_entry_choice, default_starting_config_selection,
    import_candidate_from_migration, migration_candidate_for_onboard_display,
    migration_candidate_from_onboard, onboard_starting_point_label, prepare_import_starting_state,
    select_non_interactive_starting_config_from_state, sort_starting_point_candidates,
};

use self::select::*;
mod screen_spec;

use self::screen_spec::*;
mod starting_point;
mod starting_point_render;
use self::starting_point::*;
pub use self::starting_point::{
    collect_import_candidates_with_paths, detect_import_starting_config_with_channel_readiness,
    should_offer_current_setup_shortcut, should_offer_detected_setup_shortcut,
    validate_non_interactive_risk_gate,
};
mod preinstalled_skills;
use self::preinstalled_skills::*;
mod tail;
use self::tail::*;
pub use self::tail::{
    build_channel_onboarding_follow_up_lines, collect_import_surfaces,
    collect_import_surfaces_with_channel_readiness, memory_profile_id, parse_memory_profile,
    parse_prompt_personality, parse_provider_kind, preferred_api_key_env_default,
    prompt_personality_id, provider_default_api_key_env, provider_kind_display_name,
    provider_kind_id, should_skip_config_write, supported_memory_profile_list,
    supported_personality_list, supported_provider_list,
};
mod api_key_selection;
mod guided_config;
use self::guided_config::*;
mod entry_render;
mod flow;
mod flow_types;
mod guided_render;
mod model_selection;
mod prompt_path;
mod prompt_ui;
mod provider_selection;
mod render;
mod review;
mod runtime;
mod shortcut_write_render;
mod web_search_selection;
use self::api_key_selection::*;
pub use self::entry_render::render_onboard_entry_screen_lines;
use self::entry_render::{
    prompt_onboard_entry_choice, render_onboard_entry_interactive_screen_lines_with_style,
};
use self::flow::{
    OnboardSessionPreparation, build_onboard_review_context, complete_onboard_preflight,
    finalize_onboard_closeout, is_explicitly_accepted_non_interactive_warning,
    prepare_onboard_session,
};
use self::flow_types::*;
pub use self::guided_render::{
    render_api_key_env_selection_screen_lines,
    render_api_key_env_selection_screen_lines_with_default, render_model_selection_screen_lines,
    render_model_selection_screen_lines_with_default, render_provider_selection_screen_lines,
    render_system_prompt_selection_screen_lines,
    render_system_prompt_selection_screen_lines_with_default,
};
use self::guided_render::{
    render_api_key_env_selection_screen_lines_with_style,
    render_model_selection_screen_lines_with_style, render_provider_selection_header_lines,
    render_system_prompt_selection_screen_lines_with_style,
    render_web_search_credential_selection_screen_lines_with_style,
};
use self::model_selection::*;
pub use self::prompt_path::resolve_guided_prompt_path_label_for_test;
use self::prompt_path::*;
pub(crate) use self::prompt_ui::StdioOnboardUi;
#[cfg(test)]
use self::prompt_ui::{
    OnboardPromptLineReader, OnboardPromptRead, StdioOnboardLineMessage, StdioOnboardLineReader,
    is_explicit_onboard_cancel_input, onboard_line_channel_with_capacity,
    onboard_paste_drain_window, read_single_line_prompt_capture,
};
use self::prompt_ui::{
    ensure_onboard_input_not_cancelled, is_explicit_onboard_clear_input, print_lines, print_message,
};
use self::provider_selection::*;
pub use self::provider_selection::{
    build_provider_selection_plan_for_candidate, resolve_provider_config_from_selection,
    resolve_provider_config_from_selector,
};
pub use self::render::{append_escape_cancel_hint, render_default_choice_footer_line};
use self::render::{
    render_onboard_choice_screen, render_prompt_with_default_text, screen_subtitle,
    tui_header_style,
};
#[cfg(test)]
use self::render::{render_onboard_option_lines, render_onboard_option_prefix};
#[cfg(test)]
use self::review::provider_matches_for_review;
use self::review::{
    build_onboard_review_candidate_with_selected_context,
    render_onboard_review_lines_with_guidance_and_style,
};
pub use self::review::{
    render_current_setup_review_lines_with_guidance,
    render_detected_setup_review_lines_with_guidance, render_onboard_review_lines_with_guidance,
    summarize_prompt_addendum, summarize_prompt_mode, summarize_provider_credential,
};
pub use self::runtime::{
    OnboardCommandOptions, OnboardRuntimeContext, OnboardUi, SelectInteractionMode, SelectOption,
    run_onboard_cli, run_onboard_cli_with_ui,
};
#[cfg(test)]
use self::shortcut_write_render::render_onboard_shortcut_screen_lines_with_style;
pub use self::shortcut_write_render::{
    render_continue_current_setup_screen_lines, render_continue_detected_setup_screen_lines,
    render_current_setup_write_confirmation_screen_lines,
    render_detected_setup_write_confirmation_screen_lines,
    render_existing_config_write_screen_lines, render_onboarding_risk_screen_lines,
    render_write_confirmation_screen_lines,
};
use self::shortcut_write_render::{
    render_existing_config_write_header_lines_with_style,
    render_onboard_shortcut_header_lines_with_style,
};
#[cfg(test)]
use self::starting_point_render::render_starting_point_selection_header_lines_with_style;
pub use self::starting_point_render::{
    render_single_detected_setup_preview_screen_lines, render_starting_point_selection_screen_lines,
};
use self::web_search_selection::*;
pub use crate::onboard::finalize::{
    OnboardingAction, OnboardingActionKind, OnboardingChannelSurfaceSummary,
    OnboardingDomainOutcome, OnboardingSuccessSummary, build_onboarding_success_summary,
    render_onboarding_success_summary_with_width,
};
pub use crate::onboard::write_recovery::backup_existing_config;
const ONBOARD_CLEAR_INPUT_TOKEN: &str = ":clear";
const ONBOARD_CUSTOM_MODEL_OPTION_SLUG: &str = "__custom_model__";
const ONBOARD_ESCAPE_CANCEL_HINT: &str = "- press Esc then Enter to cancel onboarding";
const ONBOARD_SINGLE_LINE_INPUT_HINT: &str = "- single-line input only";
const ONBOARD_PASTE_DRAIN_WINDOW_ENV: &str = "LOONG_ONBOARD_PASTE_DRAIN_WINDOW_MS";
const DEFAULT_ONBOARD_PASTE_DRAIN_WINDOW: Duration = Duration::from_millis(75);
const ONBOARD_LINE_READER_BUFFER_SIZE: usize = 64;
const PREINSTALLED_SKILLS_PROMPT_LABEL: &str = "preinstalled skills";

#[cfg(test)]
fn provider_model_probe_failure_check(
    config: &mvp::config::LoongConfig,
    error: String,
) -> OnboardCheck {
    runtime::provider_model_probe_failure_check(config, error)
}

pub type ChannelImportReadiness = crate::migration::ChannelImportReadiness;

#[cfg(test)]
mod tests;
