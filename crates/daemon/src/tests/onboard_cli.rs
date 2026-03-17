use super::*;
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

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

#[tokio::test]
async fn non_interactive_onboard_persists_generic_provider_api_key_reference() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("loongclaw-onboard-api-key-{unique}"));
    fs::create_dir_all(&temp_dir).expect("create temp directory");
    let config_path = temp_dir.join("config.toml");
    let path_string = config_path.display().to_string();

    crate::onboard_cli::run_onboard_cli(crate::onboard_cli::OnboardCommandOptions {
        output: Some(path_string.clone()),
        force: true,
        non_interactive: true,
        accept_risk: true,
        provider: Some("openai".to_owned()),
        model: Some("auto".to_owned()),
        api_key: Some("PATH".to_owned()),
        personality: None,
        memory_profile: None,
        system_prompt: None,
        skip_model_probe: true,
    })
    .await
    .expect("non-interactive onboarding should succeed with populated env");

    let (_, config) = mvp::config::load(Some(&path_string)).expect("load written config");
    assert_eq!(config.provider.api_key.as_deref(), Some("${PATH}"));
    assert_eq!(config.provider.api_key_env, None);

    fs::remove_file(&config_path).ok();
    fs::remove_dir_all(&temp_dir).ok();
}

#[tokio::test]
async fn non_interactive_onboard_preserves_inline_api_key_literals() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("loongclaw-onboard-api-key-inline-{unique}"));
    fs::create_dir_all(&temp_dir).expect("create temp directory");
    let config_path = temp_dir.join("config.toml");
    let path_string = config_path.display().to_string();
    let literal_api_key = "sk-proj-demo-token";

    crate::onboard_cli::run_onboard_cli(crate::onboard_cli::OnboardCommandOptions {
        output: Some(path_string.clone()),
        force: true,
        non_interactive: true,
        accept_risk: true,
        provider: Some("openai".to_owned()),
        model: Some("auto".to_owned()),
        api_key: Some(literal_api_key.to_owned()),
        personality: None,
        memory_profile: None,
        system_prompt: None,
        skip_model_probe: true,
    })
    .await
    .expect("non-interactive onboarding should accept inline api key literals");

    let (_, config) = mvp::config::load(Some(&path_string)).expect("load written config");
    assert_eq!(config.provider.api_key.as_deref(), Some(literal_api_key));
    assert_eq!(config.provider.api_key_env, None);

    fs::remove_file(&config_path).ok();
    fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn strip_default_brackets_removes_matched_wrapper() {
    assert_eq!(
        crate::onboard_cli::strip_default_brackets("[calm_engineering]"),
        "calm_engineering"
    );
    assert_eq!(
        crate::onboard_cli::strip_default_brackets("[window_only]"),
        "window_only"
    );
    assert_eq!(
        crate::onboard_cli::strip_default_brackets("calm_engineering"),
        "calm_engineering"
    );
    assert_eq!(
        crate::onboard_cli::strip_default_brackets("[partial"),
        "[partial"
    );
    assert_eq!(
        crate::onboard_cli::strip_default_brackets("partial]"),
        "partial]"
    );
    assert_eq!(crate::onboard_cli::strip_default_brackets("[]"), "");
    assert_eq!(crate::onboard_cli::strip_default_brackets(""), "");
}
