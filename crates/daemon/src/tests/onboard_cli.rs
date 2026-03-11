use super::*;

#[test]
fn parse_provider_kind_accepts_primary_and_legacy_aliases() {
    assert_eq!(
        crate::onboard_cli::parse_provider_kind("openai"),
        Some(mvp::config::ProviderKind::Openai)
    );
    assert_eq!(
        crate::onboard_cli::parse_provider_kind("openrouter_compatible"),
        Some(mvp::config::ProviderKind::Openrouter)
    );
    assert_eq!(
        crate::onboard_cli::parse_provider_kind("volcengine_custom"),
        Some(mvp::config::ProviderKind::Volcengine)
    );
    assert_eq!(
        crate::onboard_cli::parse_provider_kind("kimi_coding"),
        Some(mvp::config::ProviderKind::KimiCoding)
    );
    assert_eq!(
        crate::onboard_cli::parse_provider_kind("kimi_coding_compatible"),
        Some(mvp::config::ProviderKind::KimiCoding)
    );
    assert_eq!(crate::onboard_cli::parse_provider_kind("unsupported"), None);
}

#[test]
fn provider_default_env_mapping_is_stable() {
    assert_eq!(
        crate::onboard_cli::provider_default_api_key_env(mvp::config::ProviderKind::Openai),
        "OPENAI_API_KEY"
    );
    assert_eq!(
        crate::onboard_cli::provider_default_api_key_env(mvp::config::ProviderKind::Anthropic),
        "ANTHROPIC_API_KEY"
    );
    assert_eq!(
        crate::onboard_cli::provider_default_api_key_env(mvp::config::ProviderKind::Openrouter),
        "OPENROUTER_API_KEY"
    );
    assert_eq!(
        crate::onboard_cli::provider_default_api_key_env(mvp::config::ProviderKind::KimiCoding),
        "KIMI_CODING_API_KEY"
    );
}

#[test]
fn provider_kind_id_mapping_includes_kimi_coding() {
    assert_eq!(
        crate::onboard_cli::provider_kind_id(mvp::config::ProviderKind::KimiCoding),
        "kimi_coding"
    );
}

#[test]
fn parse_prompt_personality_accepts_supported_ids() {
    assert_eq!(
        crate::onboard_cli::parse_prompt_personality("calm_engineering"),
        Some(mvp::prompt::PromptPersonality::CalmEngineering)
    );
    assert_eq!(
        crate::onboard_cli::parse_prompt_personality("friendly_collab"),
        Some(mvp::prompt::PromptPersonality::FriendlyCollab)
    );
    assert_eq!(
        crate::onboard_cli::parse_prompt_personality("autonomous_executor"),
        Some(mvp::prompt::PromptPersonality::AutonomousExecutor)
    );
    assert_eq!(
        crate::onboard_cli::parse_prompt_personality("unknown"),
        None
    );
}

#[test]
fn parse_memory_profile_accepts_supported_ids() {
    assert_eq!(
        crate::onboard_cli::parse_memory_profile("window_only"),
        Some(mvp::config::MemoryProfile::WindowOnly)
    );
    assert_eq!(
        crate::onboard_cli::parse_memory_profile("window_plus_summary"),
        Some(mvp::config::MemoryProfile::WindowPlusSummary)
    );
    assert_eq!(
        crate::onboard_cli::parse_memory_profile("profile_plus_window"),
        Some(mvp::config::MemoryProfile::ProfilePlusWindow)
    );
    assert_eq!(crate::onboard_cli::parse_memory_profile("unknown"), None);
}

#[test]
fn non_interactive_requires_explicit_risk_acknowledgement() {
    let denied = crate::onboard_cli::validate_non_interactive_risk_gate(true, false)
        .expect_err("risk gate should reject non-interactive without acknowledgement");
    assert!(denied.contains("--accept-risk"));

    crate::onboard_cli::validate_non_interactive_risk_gate(true, true)
        .expect("risk gate should pass after acknowledgement");
    crate::onboard_cli::validate_non_interactive_risk_gate(false, false)
        .expect("interactive mode should not require explicit flag");
}

#[test]
fn onboard_import_strategy_defaults_to_recommended_single_source() {
    let summary = mvp::migration::DiscoveryPlanSummary {
        plans: vec![
            mvp::migration::PlannedImportSource {
                source: mvp::migration::LegacyClawSource::OpenClaw,
                source_id: "openclaw".to_owned(),
                input_path: std::path::PathBuf::from("/tmp/openclaw"),
                confidence_score: 42,
                prompt_addendum_present: true,
                profile_note_present: true,
                warning_count: 0,
            },
            mvp::migration::PlannedImportSource {
                source: mvp::migration::LegacyClawSource::Nanobot,
                source_id: "nanobot".to_owned(),
                input_path: std::path::PathBuf::from("/tmp/nanobot"),
                confidence_score: 18,
                prompt_addendum_present: false,
                profile_note_present: true,
                warning_count: 0,
            },
        ],
    };

    let recommendation = crate::onboard_cli::resolve_onboard_import_strategy(&summary, false)
        .expect("strategy should resolve");
    assert_eq!(
        recommendation.mode,
        crate::onboard_cli::OnboardImportMode::RecommendedSingleSource {
            source_id: "openclaw".to_owned()
        }
    );
}

#[test]
fn onboard_import_summary_shows_safe_merge_as_secondary_option() {
    let summary = mvp::migration::DiscoveryPlanSummary {
        plans: vec![
            mvp::migration::PlannedImportSource {
                source: mvp::migration::LegacyClawSource::OpenClaw,
                source_id: "openclaw".to_owned(),
                input_path: std::path::PathBuf::from("/tmp/openclaw"),
                confidence_score: 42,
                prompt_addendum_present: true,
                profile_note_present: true,
                warning_count: 0,
            },
            mvp::migration::PlannedImportSource {
                source: mvp::migration::LegacyClawSource::Nanobot,
                source_id: "nanobot".to_owned(),
                input_path: std::path::PathBuf::from("/tmp/nanobot"),
                confidence_score: 18,
                prompt_addendum_present: false,
                profile_note_present: true,
                warning_count: 1,
            },
        ],
    };
    let recommendation = mvp::migration::PrimarySourceRecommendation {
        source: mvp::migration::LegacyClawSource::OpenClaw,
        source_id: "openclaw".to_owned(),
        input_path: std::path::PathBuf::from("/tmp/openclaw"),
        reasons: vec!["contains imported prompt overlay".to_owned()],
    };

    let summary_text =
        crate::onboard_cli::build_onboard_import_summary(&summary, Some(&recommendation));
    assert!(summary_text.contains("Recommended import source: openclaw"));
    assert!(summary_text.contains("safe profile merge"));
}

#[test]
fn non_interactive_onboard_blocks_multi_source_merge_without_explicit_opt_in() {
    let strategy = crate::onboard_cli::OnboardImportStrategy {
        mode: crate::onboard_cli::OnboardImportMode::SafeProfileMerge,
        recommended_source_id: Some("openclaw".to_owned()),
    };

    let err = crate::onboard_cli::validate_non_interactive_import_strategy(&strategy, false)
        .expect_err("should block");
    assert!(err.contains("multi-source"));
}

#[test]
fn non_interactive_onboard_allows_selected_single_source_strategy() {
    let strategy = crate::onboard_cli::OnboardImportStrategy {
        mode: crate::onboard_cli::OnboardImportMode::SelectedSingleSource {
            source_id: "openclaw".to_owned(),
        },
        recommended_source_id: Some("openclaw".to_owned()),
    };

    crate::onboard_cli::validate_non_interactive_import_strategy(&strategy, false)
        .expect("single-source strategy should pass");
}
