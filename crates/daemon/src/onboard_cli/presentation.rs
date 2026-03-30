#![allow(dead_code)]
use crate::migration::{CurrentSetupState, ImportSourceKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewFlowKind {
    Guided,
    QuickCurrentSetup,
    QuickDetectedSetup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReviewFlowCopy {
    pub progress_line: &'static str,
    pub header_subtitle: &'static str,
}

pub const fn review_flow_copy(kind: ReviewFlowKind) -> ReviewFlowCopy {
    match kind {
        ReviewFlowKind::Guided => ReviewFlowCopy {
            progress_line: "step 7 of 8 · review and write",
            header_subtitle: "review setup",
        },
        ReviewFlowKind::QuickCurrentSetup => ReviewFlowCopy {
            progress_line: "quick review · current setup",
            header_subtitle: "review current setup",
        },
        ReviewFlowKind::QuickDetectedSetup => ReviewFlowCopy {
            progress_line: "quick review · detected starting point",
            header_subtitle: "review detected starting point",
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutKind {
    CurrentSetup,
    DetectedSetup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortcutCopy {
    pub review_flow_kind: ReviewFlowKind,
    pub subtitle: &'static str,
    pub title: &'static str,
    pub summary_line: &'static str,
    pub primary_label: &'static str,
    pub default_choice_description: &'static str,
}

pub const fn shortcut_copy(kind: ShortcutKind) -> ShortcutCopy {
    match kind {
        ShortcutKind::CurrentSetup => ShortcutCopy {
            review_flow_kind: ReviewFlowKind::QuickCurrentSetup,
            subtitle: "keep the current setup or fine-tune it",
            title: "continue current setup",
            summary_line: "you can keep moving with this setup through a quick review, or adjust a few settings first",
            primary_label: "Keep current setup",
            default_choice_description: "keep current setup",
        },
        ShortcutKind::DetectedSetup => ShortcutCopy {
            review_flow_kind: ReviewFlowKind::QuickDetectedSetup,
            subtitle: "use the detected starting point or fine-tune it",
            title: "continue with detected starting point",
            summary_line: "you can keep moving with this detected starting point through a quick review, or adjust a few settings first",
            primary_label: "Use detected starting point",
            default_choice_description: "the detected starting point",
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryChoiceKind {
    CurrentSetup,
    DetectedSetup,
    StartFresh,
}

pub const fn current_setup_option_label() -> &'static str {
    "Continue current setup"
}

pub const fn detected_setup_option_label() -> &'static str {
    "Use detected starting point"
}

pub const fn start_fresh_option_label() -> &'static str {
    "Start fresh"
}

pub const fn detected_settings_section_heading() -> &'static str {
    "Detected settings"
}

pub const fn entry_choice_section_heading() -> &'static str {
    "Choose how to start"
}

pub const fn adjust_settings_label() -> &'static str {
    "Adjust settings"
}

pub const fn start_fresh_option_detail() -> &'static str {
    "Configure provider, channels, and local behavior from scratch."
}

pub const fn current_setup_state_label(state: CurrentSetupState) -> &'static str {
    match state {
        CurrentSetupState::Absent => "absent",
        CurrentSetupState::Healthy => "healthy",
        CurrentSetupState::Repairable => "repairable",
        CurrentSetupState::LegacyOrIncomplete => "legacy or incomplete",
    }
}

pub const fn current_setup_option_detail(state: CurrentSetupState) -> &'static str {
    match state {
        CurrentSetupState::Healthy => "Current config looks healthy and ready to keep using.",
        CurrentSetupState::Repairable => {
            "Current config exists, but a few settings should be reviewed."
        }
        CurrentSetupState::LegacyOrIncomplete => {
            "Current config exists, but it looks incomplete for the current alpha flow."
        }
        CurrentSetupState::Absent => "No current config was found.",
    }
}

pub fn import_option_detail(
    has_current_setup: bool,
    recommended_plan_available: bool,
    detected_source_count: usize,
) -> String {
    let reusable_source_phrase = reusable_source_phrase(detected_source_count);
    let suggested_starting_point = crate::source_presentation::suggested_starting_point_label();

    if has_current_setup {
        if recommended_plan_available {
            return format!(
                "A {suggested_starting_point} can supplement the current config with {reusable_source_phrase}."
            );
        }
        return format!(
            "{reusable_source_phrase} can supplement the current config without replacing it."
        );
    }

    if recommended_plan_available {
        return format!(
            "A {suggested_starting_point} is ready, built from {reusable_source_phrase}."
        );
    }

    if detected_source_count == 1 {
        "1 reusable source was detected for provider, channels, or guidance.".to_owned()
    } else {
        format!("{reusable_source_phrase} were detected for provider, channels, or guidance.")
    }
}

pub const fn detected_coverage_prefix(recommended_plan_available: bool) -> &'static str {
    if recommended_plan_available {
        "- suggested starting point covers: "
    } else {
        "- detected coverage: "
    }
}

pub const fn suggested_starting_point_ready_line() -> &'static str {
    "- suggested starting point: ready"
}

pub const fn entry_default_choice_description(choice: EntryChoiceKind) -> &'static str {
    match choice {
        EntryChoiceKind::CurrentSetup => "continue current setup",
        EntryChoiceKind::DetectedSetup => "the detected starting point",
        EntryChoiceKind::StartFresh => "start fresh",
    }
}

pub const fn shortcut_continue_detail() -> &'static str {
    "skip detailed edits and continue to quick review"
}

pub const fn shortcut_adjust_detail() -> &'static str {
    "review provider, model, credentials, prompt behavior, memory, and web search"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RiskScreenCopy {
    pub subtitle: &'static str,
    pub title: &'static str,
    pub continue_label: &'static str,
    pub continue_detail: &'static str,
    pub cancel_label: &'static str,
    pub cancel_detail: &'static str,
    pub default_choice_description: &'static str,
    pub confirm_prompt: &'static str,
}

pub const fn risk_screen_copy() -> RiskScreenCopy {
    RiskScreenCopy {
        subtitle: "security check before setup",
        title: "security check",
        continue_label: "Continue onboarding",
        continue_detail: "review provider, channels, and local behavior now",
        cancel_label: "Cancel",
        cancel_detail: "stop before changing or writing any config",
        default_choice_description: "cancel",
        confirm_prompt: "Continue",
    }
}

pub const fn preflight_header_title() -> &'static str {
    "verify before write"
}

pub const fn preflight_section_title() -> &'static str {
    "preflight checks"
}

pub const fn preflight_attention_summary_line() -> &'static str {
    "- some checks need attention before write"
}

pub const fn preflight_green_summary_line() -> &'static str {
    "- all checks are green for this draft"
}

pub const fn preflight_probe_rerun_hint() -> &'static str {
    "- rerun with --skip-model-probe if your provider blocks model listing during setup"
}

pub const fn preflight_explicit_model_rerun_hint() -> &'static str {
    "- rerun onboarding to choose a reviewed model, or set provider.model / preferred_models explicitly"
}

pub const fn preflight_explicit_model_only_rerun_hint() -> &'static str {
    "- set provider.model / preferred_models explicitly before retrying"
}

pub const fn preflight_continue_label() -> &'static str {
    "Continue anyway"
}

pub const fn preflight_continue_detail() -> &'static str {
    "accept the remaining warnings and continue with this draft"
}

pub const fn preflight_cancel_label() -> &'static str {
    "Cancel"
}

pub const fn preflight_cancel_detail() -> &'static str {
    "stop here and return without writing any config"
}

pub const fn preflight_default_choice_description() -> &'static str {
    "cancel"
}

pub const fn preflight_confirm_prompt() -> &'static str {
    "Continue anyway"
}

pub const fn workspace_step_title() -> &'static str {
    "workspace defaults"
}

pub const fn workspace_step_subtitle() -> &'static str {
    "review local storage paths"
}

pub const fn workspace_step_summary_line() -> &'static str {
    "these are the local paths LoongClaw will use for config, sqlite memory, and tool file access"
}

pub const fn protocol_step_title() -> &'static str {
    "protocol defaults"
}

pub const fn protocol_step_subtitle() -> &'static str {
    "review ACP and bootstrap MCP state"
}

pub const fn protocol_step_summary_line() -> &'static str {
    "this step reflects the ACP backend and any bootstrap MCP servers already configured for this draft"
}

pub const fn current_value_label() -> &'static str {
    "current value"
}

pub const fn detected_value_label() -> &'static str {
    "detected value"
}

pub const fn user_override_label() -> &'static str {
    "user override"
}

pub const fn write_confirmation_title() -> &'static str {
    "ready to write config"
}

pub const fn write_confirmation_status_line(warnings_kept: bool) -> &'static str {
    if warnings_kept {
        "- warnings were kept by choice"
    } else {
        "- preflight is green for this draft"
    }
}

pub const fn write_confirmation_label() -> &'static str {
    "Write config"
}

pub const fn write_confirmation_detail() -> &'static str {
    "persist this onboarding draft to the target path"
}

pub const fn write_confirmation_cancel_label() -> &'static str {
    "Cancel"
}

pub const fn write_confirmation_cancel_detail() -> &'static str {
    "return without writing any config"
}

pub const fn write_confirmation_default_choice_description() -> &'static str {
    "write config"
}

pub const fn write_confirmation_prompt() -> &'static str {
    "Write config"
}

pub const fn start_fresh_starting_point_fit_line() -> &'static str {
    "good fit: start clean with full control"
}

pub const fn start_fresh_starting_point_detail_line() -> &'static str {
    "configure provider, channels, and local behavior from scratch"
}

pub const fn starting_point_footer_description(first_kind: ImportSourceKind) -> &'static str {
    match first_kind {
        ImportSourceKind::RecommendedPlan => "the suggested starting point",
        ImportSourceKind::CurrentSetup
        | ImportSourceKind::ExistingLoongClawConfig
        | ImportSourceKind::CodexConfig
        | ImportSourceKind::Environment
        | ImportSourceKind::ExplicitPath => "the first starting point",
    }
}

pub const fn starting_point_selection_subtitle() -> &'static str {
    "choose the starting point for this setup"
}

pub const fn starting_point_selection_title() -> &'static str {
    "choose detected starting point"
}

pub const fn starting_point_selection_hint() -> &'static str {
    "detected settings can still supplement the chosen starting point when they do not conflict"
}

pub const fn single_detected_starting_point_preview_subtitle() -> &'static str {
    "review the detected starting point"
}

pub const fn single_detected_starting_point_preview_title() -> &'static str {
    "review detected starting point"
}

pub const fn single_detected_starting_point_preview_footer() -> &'static str {
    "continuing with the only detected starting point"
}

fn reusable_source_phrase(count: usize) -> String {
    if count == 1 {
        "1 reusable source".to_owned()
    } else {
        format!("{count} reusable sources")
    }
}
