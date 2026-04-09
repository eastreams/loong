#![allow(unsafe_code)]
#![allow(
    clippy::disallowed_methods,
    clippy::multiple_unsafe_ops_per_block,
    clippy::undocumented_unsafe_blocks
)]

use super::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static MIGRATION_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
fn unique_temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    let counter = MIGRATION_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir();
    let canonical_temp_dir = dunce::canonicalize(&temp_dir).unwrap_or(temp_dir);
    canonical_temp_dir.join(format!(
        "loongclaw-migration-{label}-{}-{nanos}-{counter}",
        std::process::id(),
    ))
}

fn normalized_path_text(value: &str) -> String {
    value.replace('\\', "/")
}

#[test]
fn source_presentation_canonical_labels_are_stable() {
    assert_eq!(
        loongclaw_daemon::source_presentation::recommended_plan_source_label(),
        "recommended import plan"
    );
    assert_eq!(
        loongclaw_daemon::source_presentation::environment_source_label(),
        "your current environment"
    );
    assert_eq!(
        loongclaw_daemon::source_presentation::workspace_source_label(),
        "workspace"
    );
    assert_eq!(
        loongclaw_daemon::source_presentation::current_onboarding_draft_source_label(),
        "current onboarding draft"
    );
    assert_eq!(
        loongclaw_daemon::source_presentation::suggested_starting_point_label(),
        "suggested starting point"
    );
}

#[test]
fn source_presentation_rollup_and_onboarding_labels_follow_canonical_rules() {
    assert_eq!(
        loongclaw_daemon::source_presentation::onboarding_source_label(
            Some(loongclaw_daemon::migration::ImportSourceKind::RecommendedPlan),
            loongclaw_daemon::source_presentation::recommended_plan_source_label(),
        ),
        "suggested starting point"
    );
    assert_eq!(
        loongclaw_daemon::source_presentation::rollup_source_label(
            loongclaw_daemon::source_presentation::recommended_plan_source_label()
        ),
        None
    );
    assert_eq!(
        loongclaw_daemon::source_presentation::rollup_source_label(
            loongclaw_daemon::source_presentation::workspace_source_label()
        ),
        Some("workspace guidance".to_owned())
    );
    assert_eq!(
        loongclaw_daemon::source_presentation::rollup_source_label(
            loongclaw_daemon::source_presentation::environment_source_label()
        ),
        Some("your current environment".to_owned())
    );
    assert_eq!(
        loongclaw_daemon::source_presentation::source_path(
            Some(loongclaw_daemon::migration::ImportSourceKind::CodexConfig),
            "Codex config at ~/.codex/config.toml"
        ),
        Some(PathBuf::from("~/.codex/config.toml"))
    );
    assert_eq!(
        loongclaw_daemon::source_presentation::source_path(
            Some(loongclaw_daemon::migration::ImportSourceKind::ExistingLoongClawConfig),
            "existing config at ~/.config/loongclaw/config.toml"
        ),
        Some(PathBuf::from("~/.config/loongclaw/config.toml"))
    );
    assert_eq!(
        loongclaw_daemon::source_presentation::source_path(
            Some(loongclaw_daemon::migration::ImportSourceKind::Environment),
            loongclaw_daemon::source_presentation::environment_source_label()
        ),
        None
    );
}

#[test]
fn migration_preview_descriptors_and_starting_point_reasons_are_stable() {
    assert_eq!(
        loongclaw_daemon::migration::types::PreviewStatus::NeedsReview.label(),
        "Needs review"
    );
    assert_eq!(
        loongclaw_daemon::migration::types::PreviewDecision::AdjustedInSession.label(),
        "adjusted in this setup"
    );
    assert_eq!(
        loongclaw_daemon::migration::types::PreviewDecision::AdjustedInSession.outcome_label(),
        "adjusted now"
    );
    assert!(
        loongclaw_daemon::migration::types::PreviewDecision::ReviewConflict.outcome_rank()
            < loongclaw_daemon::migration::types::PreviewDecision::UseDetected.outcome_rank()
    );
    assert_eq!(
        loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig
            .direct_starting_point_reason(),
        Some("reuse Codex config as your starting point")
    );
    assert_eq!(
        loongclaw_daemon::migration::types::ImportSourceKind::ExplicitPath
            .direct_starting_point_reason(),
        Some("reuse the selected config file as your starting point")
    );
    assert_eq!(
        loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan
            .direct_starting_point_reason(),
        None
    );
    assert_eq!(
        loongclaw_daemon::migration::types::SetupDomainKind::Provider.starting_point_reason(
            loongclaw_daemon::migration::types::PreviewDecision::KeepCurrent
        ),
        Some("keep current provider")
    );
    assert_eq!(
        loongclaw_daemon::migration::types::SetupDomainKind::Channels
            .starting_point_reason(loongclaw_daemon::migration::types::PreviewDecision::Supplement),
        Some("add detected channels")
    );
    assert_eq!(
        loongclaw_daemon::migration::types::SetupDomainKind::WorkspaceGuidance
            .starting_point_reason(
                loongclaw_daemon::migration::types::PreviewDecision::UseDetected
            ),
        Some("reuse workspace guidance")
    );
    assert_eq!(
        loongclaw_daemon::migration::types::SetupDomainKind::Provider
            .starting_point_reason(loongclaw_daemon::migration::types::PreviewDecision::Supplement),
        None
    );
}

#[test]
fn migration_collect_import_candidates_maps_unknown_codex_provider_with_openai_compatible_wire_api()
{
    let root = unique_temp_dir("codex-compatible-provider");
    std::fs::create_dir_all(&root).expect("create temp root");
    let output_path = root.join("config.toml");
    let codex_path = root.join("codex.toml");
    std::fs::write(
        &codex_path,
        r#"
model_provider = "sub2api"
model = "openai/gpt-5.1-codex"

[model_providers.sub2api]
base_url = "https://codex.example.com/v1"
wire_api = "responses"
"#,
    )
    .expect("write codex config");

    let candidates =
        loongclaw_daemon::migration::collect_import_candidates_with_paths_and_readiness(
            &output_path,
            Some(&codex_path),
            None,
            loongclaw_daemon::migration::ChannelImportReadiness::default(),
        )
        .expect("collect import candidates");

    let codex_candidate = candidates
        .iter()
        .find(|candidate| {
            candidate.source_kind == loongclaw_daemon::migration::ImportSourceKind::CodexConfig
        })
        .expect("codex candidate");
    assert_eq!(
        codex_candidate.config.provider.kind,
        mvp::config::ProviderKind::Openai,
        "unknown Codex provider should fall back only when the provider section clearly looks OpenAI-compatible"
    );
    assert_eq!(
        codex_candidate.config.provider.base_url,
        "https://codex.example.com/v1"
    );
    assert_eq!(
        codex_candidate.config.provider.model,
        "openai/gpt-5.1-codex"
    );
}

#[test]
fn migration_collect_import_candidates_skip_unknown_codex_provider_without_compatibility_signals() {
    let root = unique_temp_dir("codex-unknown-provider");
    std::fs::create_dir_all(&root).expect("create temp root");
    let output_path = root.join("config.toml");
    let codex_path = root.join("codex.toml");
    std::fs::write(
        &codex_path,
        r#"
model_provider = "mystery_proxy"
model = "mystery/model"

[model_providers.mystery_proxy]
base_url = "https://mystery.example.com/v1"
"#,
    )
    .expect("write codex config");

    let candidates =
        loongclaw_daemon::migration::collect_import_candidates_with_paths_and_readiness(
            &output_path,
            Some(&codex_path),
            None,
            loongclaw_daemon::migration::ChannelImportReadiness::default(),
        )
        .expect("collect import candidates");

    assert!(
        candidates.iter().all(|candidate| candidate.source_kind
            != loongclaw_daemon::migration::ImportSourceKind::CodexConfig),
        "unknown Codex providers without compatibility evidence should be skipped instead of silently imported as OpenAI"
    );
}

#[test]
fn migration_collect_import_candidates_preserves_provider_default_auth_env_for_recognized_codex_provider()
 {
    let root = unique_temp_dir("codex-recognized-provider-env");
    std::fs::create_dir_all(&root).expect("create temp root");
    let output_path = root.join("config.toml");
    let codex_path = root.join("codex.toml");
    std::fs::write(
        &codex_path,
        r#"
model_provider = "deepseek"
model = "deepseek-chat"
"#,
    )
    .expect("write codex config");

    let candidates =
        loongclaw_daemon::migration::collect_import_candidates_with_paths_and_readiness(
            &output_path,
            Some(&codex_path),
            None,
            loongclaw_daemon::migration::ChannelImportReadiness::default(),
        )
        .expect("collect import candidates");

    let codex_candidate = candidates
        .iter()
        .find(|candidate| {
            candidate.source_kind == loongclaw_daemon::migration::ImportSourceKind::CodexConfig
        })
        .expect("codex candidate");
    assert_eq!(
        codex_candidate.config.provider.kind,
        mvp::config::ProviderKind::Deepseek
    );
    assert_eq!(
        codex_candidate.config.provider.api_key,
        Some(loongclaw_contracts::SecretRef::Env {
            env: "DEEPSEEK_API_KEY".to_owned(),
        }),
        "recognized Codex providers should start from their provider baseline so imports keep the correct canonical credential binding even without an explicit provider section"
    );
    assert_eq!(
        codex_candidate.config.provider.oauth_access_token_env, None,
        "Codex imports should not infer an unrelated OAuth credential path unless the source config explicitly describes one"
    );
}

#[test]
fn migration_channel_registry_includes_matrix_when_enabled() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.matrix.enabled = true;
    config.matrix.access_token = Some(loongclaw_contracts::SecretRef::Inline(
        "matrix-token".to_owned(),
    ));
    config.matrix.base_url = Some("https://matrix.example.org".to_owned());

    let checks = loongclaw_daemon::migration::channels::collect_channel_doctor_checks(&config);
    let names = checks.iter().map(|check| check.name).collect::<Vec<_>>();

    assert_eq!(names, vec!["matrix channel", "matrix room sync"]);
}

#[test]
fn migration_channel_registry_includes_wecom_when_enabled() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.wecom.enabled = true;
    config.wecom.bot_id = Some(loongclaw_contracts::SecretRef::Inline(
        "wecom-bot".to_owned(),
    ));
    config.wecom.secret = Some(loongclaw_contracts::SecretRef::Inline(
        "wecom-secret".to_owned(),
    ));
    config.wecom.allowed_conversation_ids = vec!["group_demo".to_owned()];

    let checks = loongclaw_daemon::migration::channels::collect_channel_doctor_checks(&config);
    let names = checks.iter().map(|check| check.name).collect::<Vec<_>>();

    assert_eq!(names, vec!["wecom channel", "wecom aibot long connection"]);
}

#[test]
fn migration_channel_registry_includes_discord_when_enabled() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.discord.enabled = true;
    config.discord.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
        "discord-token".to_owned(),
    ));

    let checks = loongclaw_daemon::migration::channels::collect_channel_doctor_checks(&config);
    let names = checks.iter().map(|check| check.name).collect::<Vec<_>>();

    assert_eq!(names, vec!["discord channel"]);
    assert!(
        checks.iter().any(|check| {
            check.name == "discord channel"
                && check.level == loongclaw_daemon::migration::channels::ChannelCheckLevel::Pass
                && check.detail.contains("outbound-only")
                && check.detail.contains("direct send ready")
        }),
        "doctor checks should treat configured outbound-only channels as first-class surfaces: {checks:#?}"
    );
}

#[test]
fn migration_channel_registry_includes_weixin_bridge_when_enabled() {
    let install_root = unique_temp_dir("managed-bridge-doctor-ready");
    let manifest = super::managed_bridge_manifest(
        "weixin",
        Some("channel"),
        super::compatible_managed_bridge_metadata(
            "wechat_clawbot_ilink_bridge",
            "weixin_reply_loop",
        ),
    );
    let mut config: mvp::config::LoongClawConfig = serde_json::from_value(json!({
        "weixin": {
            "enabled": true,
            "bridge_url": "https://bridge.example.test/weixin",
            "bridge_access_token": "weixin-token",
            "allowed_contact_ids": ["wxid_alice"]
        }
    }))
    .expect("deserialize weixin config");

    std::fs::create_dir_all(&install_root).expect("create managed bridge install root");
    super::write_managed_bridge_manifest(
        install_root.as_path(),
        "weixin-managed-bridge",
        &manifest,
    );
    config.external_skills.install_root = Some(install_root.display().to_string());

    let checks = loongclaw_daemon::migration::channels::collect_channel_doctor_checks(&config);
    let names = checks.iter().map(|check| check.name).collect::<Vec<_>>();

    assert_eq!(names, vec!["weixin channel"]);
    assert!(
        checks.iter().any(|check| {
            check.name == "weixin channel"
                && check.level == loongclaw_daemon::migration::channels::ChannelCheckLevel::Pass
                && check.detail.contains("plugin-backed")
                && check.detail.contains("weixin-managed-bridge")
        }),
        "doctor checks should treat ready plugin-backed channels as first-class surfaces: {checks:#?}"
    );
}

#[test]
fn migration_channel_env_binding_applies_matrix_and_wecom_defaults() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.matrix.access_token_env = None;
    config.wecom.bot_id_env = None;
    config.wecom.secret_env = None;

    let fixes =
        loongclaw_daemon::migration::channels::apply_default_channel_env_bindings(&mut config);

    assert_eq!(
        config.matrix.access_token_env.as_deref(),
        Some("MATRIX_ACCESS_TOKEN")
    );
    assert_eq!(config.wecom.bot_id_env.as_deref(), Some("WECOM_BOT_ID"));
    assert_eq!(config.wecom.secret_env.as_deref(), Some("WECOM_SECRET"));
    assert!(
        fixes
            .iter()
            .any(|fix| fix == "set matrix.access_token_env=MATRIX_ACCESS_TOKEN")
    );
    assert!(
        fixes
            .iter()
            .any(|fix| fix == "set wecom.bot_id_env=WECOM_BOT_ID")
    );
    assert!(
        fixes
            .iter()
            .any(|fix| fix == "set wecom.secret_env=WECOM_SECRET")
    );
}

#[test]
fn migration_collect_import_candidates_preserves_api_key_flow_for_openai_codex_provider() {
    let root = unique_temp_dir("codex-openai-api-key-flow");
    std::fs::create_dir_all(&root).expect("create temp root");
    let output_path = root.join("config.toml");
    let codex_path = root.join("codex.toml");
    std::fs::write(
        &codex_path,
        r#"
model_provider = "openai"
model = "openai/gpt-5.1-codex"
"#,
    )
    .expect("write codex config");

    let candidates =
        loongclaw_daemon::migration::collect_import_candidates_with_paths_and_readiness(
            &output_path,
            Some(&codex_path),
            None,
            loongclaw_daemon::migration::ChannelImportReadiness::default(),
        )
        .expect("collect import candidates");

    let codex_candidate = candidates
        .iter()
        .find(|candidate| {
            candidate.source_kind == loongclaw_daemon::migration::ImportSourceKind::CodexConfig
        })
        .expect("codex candidate");
    assert_eq!(
        codex_candidate.config.provider.kind,
        mvp::config::ProviderKind::Openai
    );
    assert_eq!(
        codex_candidate.config.provider.api_key,
        Some(loongclaw_contracts::SecretRef::Env {
            env: "OPENAI_API_KEY".to_owned(),
        })
    );
    assert_eq!(codex_candidate.config.provider.api_key_env, None);
    assert_eq!(
        codex_candidate.config.provider.oauth_access_token_env, None,
        "Codex imports should keep the portable API-key path by default instead of auto-enabling machine-local OAuth envs"
    );
}

#[test]
fn migration_collect_import_candidates_detects_base_and_agent_scoped_codex_configs() {
    let root = unique_temp_dir("codex-detected-paths");
    let home = root.join("home");
    let output_path = root.join("config.toml");
    let base_codex_path = home.join(".codex/config.toml");
    let agent_codex_path = home
        .join(".codex/agents")
        .join(mvp::config::CLI_COMMAND_NAME)
        .join("config.toml");

    std::fs::create_dir_all(base_codex_path.parent().expect("base codex parent"))
        .expect("create base codex parent");
    std::fs::create_dir_all(agent_codex_path.parent().expect("agent codex parent"))
        .expect("create agent codex parent");
    std::fs::write(
        &base_codex_path,
        r#"
model_provider = "openai"
model = "openai/gpt-5.1-codex"
"#,
    )
    .expect("write base codex config");
    std::fs::write(
        &agent_codex_path,
        r#"
model_provider = "deepseek"
model = "deepseek-chat"
"#,
    )
    .expect("write agent codex config");

    let _env_guard =
        MigrationEnvironmentGuard::set(&[("HOME", Some(home.to_string_lossy().as_ref()))]);

    let candidates =
        loongclaw_daemon::migration::discovery::collect_import_candidates_with_path_list_and_readiness(
            &output_path,
            &loongclaw_daemon::migration::discovery::default_detected_codex_config_paths(),
            None,
            loongclaw_daemon::migration::ChannelImportReadiness::default(),
        )
        .expect("collect import candidates");

    let codex_sources = candidates
        .iter()
        .filter(|candidate| {
            candidate.source_kind == loongclaw_daemon::migration::ImportSourceKind::CodexConfig
        })
        .map(|candidate| candidate.source.as_str())
        .collect::<Vec<_>>();
    let normalized_codex_sources = codex_sources
        .iter()
        .map(|source| normalized_path_text(source))
        .collect::<Vec<_>>();

    assert!(
        normalized_codex_sources
            .iter()
            .any(|source| source.ends_with("/.codex/config.toml")),
        "detected candidates should include the base Codex config path: {codex_sources:#?}"
    );
    assert!(
        normalized_codex_sources
            .iter()
            .any(|source| source.ends_with(&format!(
                "/.codex/agents/{}/config.toml",
                mvp::config::CLI_COMMAND_NAME
            ))),
        "detected candidates should include the agent-scoped Codex config path: {codex_sources:#?}"
    );
}

#[test]
fn migration_domain_previews_preserve_source_attribution() {
    let workspace_root = unique_temp_dir("guidance");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    std::fs::write(workspace_root.join("AGENTS.md"), "# repo guidance\n").expect("write AGENTS");

    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "provider-secret".to_owned(),
    ));
    config.provider.model = "openai/gpt-5.1-codex".to_owned();
    config.telegram.enabled = true;
    config.telegram.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
        "123456:test-token".to_owned(),
    ));
    config.cli.system_prompt = "Use the repo rules.".to_owned();

    let guidance =
        loongclaw_daemon::migration::discovery::detect_workspace_guidance(&workspace_root);
    let candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
        "Codex config at ~/.codex/config.toml".to_owned(),
        config,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        guidance,
    )
    .expect("candidate should be generated");

    assert!(
        candidate.domains.iter().any(|domain| {
            domain.kind == loongclaw_daemon::migration::types::SetupDomainKind::Provider
                && domain.source == "Codex config at ~/.codex/config.toml"
        }),
        "provider preview should keep source attribution: {candidate:#?}"
    );
    assert!(
        candidate.domains.iter().any(|domain| {
            domain.kind == loongclaw_daemon::migration::types::SetupDomainKind::WorkspaceGuidance
                && domain.summary.contains("AGENTS.md")
        }),
        "workspace guidance preview should be present when repo guidance exists: {candidate:#?}"
    );
}

#[test]
fn migration_channel_domain_summary_includes_channel_maturity_labels() {
    let config: mvp::config::LoongClawConfig = serde_json::from_value(json!({
        "telegram": {
            "enabled": true,
            "bot_token": "123456:test-token",
            "allowed_chat_ids": [42]
        },
        "weixin": {
            "enabled": true,
            "bridge_url": "https://bridge.example.test/weixin",
            "bridge_access_token": "weixin-token",
            "allowed_contact_ids": ["wxid_alice"]
        },
        "discord": {
            "enabled": true,
            "bot_token": "discord-token"
        }
    }))
    .expect("deserialize mixed channel config");

    let candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
        "Codex config at ~/.codex/config.toml".to_owned(),
        config,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("candidate should be generated");

    let channels_domain = candidate
        .domains
        .iter()
        .find(|domain| domain.kind == loongclaw_daemon::migration::types::SetupDomainKind::Channels)
        .expect("channels domain");

    assert!(
        channels_domain
            .summary
            .contains("Telegram Ready (runtime-backed)"),
        "channels domain summary should keep runtime-backed channel maturity visible: {channels_domain:#?}"
    );
    assert!(
        channels_domain
            .summary
            .contains("Weixin Needs review (plugin-backed)"),
        "channels domain summary should keep plugin-backed channel maturity visible: {channels_domain:#?}"
    );
    assert!(
        channels_domain
            .summary
            .contains("Discord Ready (outbound-only)"),
        "channels domain summary should keep outbound-only channel maturity visible: {channels_domain:#?}"
    );
}

#[test]
fn migration_render_preview_channel_rows_include_channel_maturity_labels() {
    let candidate = loongclaw_daemon::migration::types::ImportCandidate {
        source_kind: loongclaw_daemon::migration::types::ImportSourceKind::Environment,
        source: "your current environment".to_owned(),
        config: mvp::config::LoongClawConfig::default(),
        surfaces: Vec::new(),
        domains: vec![loongclaw_daemon::migration::types::DomainPreview {
            kind: loongclaw_daemon::migration::types::SetupDomainKind::Channels,
            status: loongclaw_daemon::migration::types::PreviewStatus::NeedsReview,
            decision: Some(loongclaw_daemon::migration::types::PreviewDecision::UseDetected),
            source: "your current environment".to_owned(),
            summary: "Telegram Ready (runtime-backed) · Discord Ready (outbound-only)".to_owned(),
        }],
        channel_candidates: vec![
            loongclaw_daemon::migration::types::ChannelCandidate {
                id: "telegram",
                label: "telegram",
                status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
                source: "your current environment".to_owned(),
                summary: "runtime-backed · token resolved".to_owned(),
            },
            loongclaw_daemon::migration::types::ChannelCandidate {
                id: "discord",
                label: "discord",
                status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
                source: "your current environment".to_owned(),
                summary: "outbound-only · direct send ready on 1 account(s)".to_owned(),
            },
        ],
        workspace_guidance: Vec::new(),
    };

    let lines = loongclaw_daemon::migration::render::render_candidate_preview_lines(&candidate, 96);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("telegram [Ready · runtime-backed]")),
        "rendered preview should show runtime-backed channel maturity inline: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("discord [Ready · outbound-only]")),
        "rendered preview should show outbound-only channel maturity inline: {lines:#?}"
    );
}

#[test]
fn migration_classify_current_setup_distinguishes_basic_states() {
    let home = unique_temp_dir("classify-home");
    std::fs::create_dir_all(&home).expect("create classify home");
    let _env_guard = MigrationEnvironmentGuard::set(&[
        ("HOME", Some(home.to_string_lossy().as_ref())),
        ("OPENAI_API_KEY", None),
        ("OPENAI_CODEX_OAUTH_TOKEN", None),
        ("OPENAI_OAUTH_ACCESS_TOKEN", None),
        ("TELEGRAM_BOT_TOKEN", None),
    ]);

    let missing = unique_temp_dir("missing").join("config.toml");
    assert_eq!(
        loongclaw_daemon::migration::discovery::classify_current_setup(&missing),
        loongclaw_daemon::migration::types::CurrentSetupState::Absent
    );

    let healthy_path = unique_temp_dir("healthy").join("config.toml");
    let mut healthy = mvp::config::LoongClawConfig::default();
    healthy.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "provider-secret".to_owned(),
    ));
    healthy.provider.model = "openai/gpt-5.1-codex".to_owned();
    mvp::config::write(
        Some(healthy_path.to_string_lossy().as_ref()),
        &healthy,
        true,
    )
    .expect("write healthy config");
    assert_eq!(
        loongclaw_daemon::migration::discovery::classify_current_setup(&healthy_path),
        loongclaw_daemon::migration::types::CurrentSetupState::Healthy
    );

    let repairable_path = unique_temp_dir("repairable").join("config.toml");
    let mut repairable = mvp::config::LoongClawConfig::default();
    repairable.telegram.enabled = true;
    mvp::config::write(
        Some(repairable_path.to_string_lossy().as_ref()),
        &repairable,
        true,
    )
    .expect("write repairable config");
    assert_eq!(
        loongclaw_daemon::migration::discovery::classify_current_setup(&repairable_path),
        loongclaw_daemon::migration::types::CurrentSetupState::Repairable
    );

    let legacy_path = unique_temp_dir("legacy").join("config.toml");
    let mut legacy = mvp::config::LoongClawConfig::default();
    legacy.provider.kind = mvp::config::ProviderKind::Ollama;
    let profile = legacy.provider.kind.profile();
    legacy.provider.base_url = profile.base_url.to_owned();
    legacy.provider.chat_completions_path = profile.chat_completions_path.to_owned();
    legacy.provider.api_key_env = None;
    legacy.provider.oauth_access_token_env = None;
    mvp::config::write(Some(legacy_path.to_string_lossy().as_ref()), &legacy, true)
        .expect("write legacy config");
    assert_eq!(
        loongclaw_daemon::migration::discovery::classify_current_setup(&legacy_path),
        loongclaw_daemon::migration::types::CurrentSetupState::LegacyOrIncomplete
    );
}

#[test]
fn migration_classify_current_setup_treats_provider_tuning_changes_as_repairable() {
    let path = unique_temp_dir("provider-tuning").join("config.toml");
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Ollama;
    let profile = config.provider.kind.profile();
    config.provider.base_url = profile.base_url.to_owned();
    config.provider.chat_completions_path = profile.chat_completions_path.to_owned();
    config.provider.api_key_env = None;
    config.provider.oauth_access_token_env = None;
    config.provider.temperature = 0.75;
    mvp::config::write(Some(path.to_string_lossy().as_ref()), &config, true)
        .expect("write config with provider tuning change");

    assert_eq!(
        loongclaw_daemon::migration::discovery::classify_current_setup(&path),
        loongclaw_daemon::migration::types::CurrentSetupState::Repairable,
        "provider transport/tuning changes should not be collapsed into the legacy selection-only bucket"
    );
}

#[test]
fn migration_classify_current_setup_treats_prompt_and_memory_metadata_as_repairable() {
    let path = unique_temp_dir("prompt-memory-metadata").join("config.toml");
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Ollama;
    let profile = config.provider.kind.profile();
    config.provider.base_url = profile.base_url.to_owned();
    config.provider.chat_completions_path = profile.chat_completions_path.to_owned();
    config.provider.api_key_env = None;
    config.provider.oauth_access_token_env = None;
    config.cli.personality = Some(mvp::prompt::PromptPersonality::Hermit);
    config.cli.refresh_native_system_prompt();
    config.memory.profile = mvp::config::MemoryProfile::ProfilePlusWindow;
    mvp::config::write(Some(path.to_string_lossy().as_ref()), &config, true)
        .expect("write config with prompt and memory metadata");

    assert_eq!(
        crate::migration::discovery::classify_current_setup(&path),
        crate::migration::types::CurrentSetupState::Repairable,
        "prompt-pack or memory-profile metadata changes should not be collapsed into the legacy selection-only bucket"
    );
}

#[test]
fn migration_classify_current_setup_ignores_home_drift_for_default_memory_path() {
    let home_a = unique_temp_dir("classify-home-drift-a");
    let home_b = unique_temp_dir("classify-home-drift-b");
    std::fs::create_dir_all(&home_a).expect("create first classify home");
    std::fs::create_dir_all(&home_b).expect("create second classify home");

    let path = unique_temp_dir("classify-home-drift").join("config.toml");
    {
        let _guard = MigrationEnvironmentGuard::set(&[
            ("HOME", Some(home_a.to_string_lossy().as_ref())),
            ("OPENAI_API_KEY", None),
            ("OPENAI_CODEX_OAUTH_TOKEN", None),
            ("OPENAI_OAUTH_ACCESS_TOKEN", None),
            ("TELEGRAM_BOT_TOKEN", None),
        ]);

        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Ollama;
        let profile = config.provider.kind.profile();
        config.provider.base_url = profile.base_url.to_owned();
        config.provider.chat_completions_path = profile.chat_completions_path.to_owned();
        config.provider.api_key_env = None;
        config.provider.oauth_access_token_env = None;
        mvp::config::write(Some(path.to_string_lossy().as_ref()), &config, true)
            .expect("write legacy-style config under first home");
    }

    let _guard = MigrationEnvironmentGuard::set(&[
        ("HOME", Some(home_b.to_string_lossy().as_ref())),
        ("OPENAI_API_KEY", None),
        ("OPENAI_CODEX_OAUTH_TOKEN", None),
        ("OPENAI_OAUTH_ACCESS_TOKEN", None),
        ("TELEGRAM_BOT_TOKEN", None),
    ]);

    assert_eq!(
        crate::migration::discovery::classify_current_setup(&path),
        crate::migration::types::CurrentSetupState::LegacyOrIncomplete,
        "default memory sqlite path drift from HOME changes should not force a legacy selection-only config into the repairable bucket"
    );
}

#[test]
fn migration_build_import_candidate_detects_provider_env_pointer_only_changes() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.api_key_env = Some("LOONGCLAW_CUSTOM_OPENAI_KEY".to_owned());

    let candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::ExistingLoongClawConfig,
        "existing config at ~/.config/loongclaw/config.toml".to_owned(),
        config,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("provider env pointer change should produce an import candidate");

    assert!(
        candidate
            .domains
            .iter()
            .any(|domain| domain.kind
                == loongclaw_daemon::migration::types::SetupDomainKind::Provider),
        "provider env-pointer-only changes should still surface as a provider import domain: {candidate:#?}"
    );
}

#[test]
fn migration_recommended_plan_supplements_cli_prompt_metadata_and_memory_profile() {
    let mut current_config = mvp::config::LoongClawConfig::default();
    current_config.provider.api_key_env = Some("OPENAI_API_KEY".to_owned());
    let current = crate::migration::discovery::build_import_candidate(
        crate::migration::types::ImportSourceKind::ExistingLoongClawConfig,
        "existing config at ~/.config/loongclaw/config.toml".to_owned(),
        current_config,
        crate::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("current config candidate");

    let mut detected_config = mvp::config::LoongClawConfig::default();
    detected_config.cli.personality = Some(mvp::prompt::PromptPersonality::Hermit);
    detected_config.cli.system_prompt_addendum = Some("Keep answers direct.".to_owned());
    detected_config.cli.refresh_native_system_prompt();
    detected_config.memory.profile = mvp::config::MemoryProfile::ProfilePlusWindow;

    let detected = crate::migration::discovery::build_import_candidate(
        crate::migration::types::ImportSourceKind::Environment,
        "your current environment".to_owned(),
        detected_config,
        crate::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("detected prompt and memory metadata should build as a candidate");

    let recommended =
        crate::migration::planner::compose_recommended_import_candidate(&[current, detected])
            .expect("recommended import plan");

    assert_eq!(
        recommended.config.cli.personality,
        Some(mvp::prompt::PromptPersonality::Hermit)
    );
    assert_eq!(
        recommended.config.cli.system_prompt_addendum.as_deref(),
        Some("Keep answers direct.")
    );
    assert!(
        recommended.config.cli.uses_native_prompt_pack(),
        "supplemented CLI config should stay on the native prompt-pack path"
    );
    assert_eq!(
        recommended.config.memory.profile,
        mvp::config::MemoryProfile::ProfilePlusWindow
    );
}

#[test]
fn channel_registry_lists_registered_channel_ids() {
    assert_eq!(
        loongclaw_daemon::migration::channels::registered_channel_ids(),
        vec![
            "telegram", "feishu", "matrix", "wecom", "line", "whatsapp", "webhook"
        ]
    );
}

#[test]
fn channel_registry_registered_ids_follow_shared_service_channel_catalog_order() {
    let service_ids = mvp::config::service_channel_descriptors()
        .into_iter()
        .map(|descriptor| descriptor.id)
        .collect::<Vec<_>>();
    let registry_ids = loongclaw_daemon::migration::channels::registered_channel_ids();
    let ordered_subset = service_ids
        .into_iter()
        .filter(|channel_id| registry_ids.contains(channel_id))
        .collect::<Vec<_>>();

    assert_eq!(registry_ids, ordered_subset);
}

#[test]
fn channel_import_readiness_tracks_channel_states_by_id() {
    let readiness = loongclaw_daemon::migration::ChannelImportReadiness::default()
        .with_state(
            "telegram",
            loongclaw_daemon::migration::ChannelCredentialState::Ready,
        )
        .with_state(
            "feishu",
            loongclaw_daemon::migration::ChannelCredentialState::Partial,
        );

    assert_eq!(
        readiness.state("telegram"),
        loongclaw_daemon::migration::ChannelCredentialState::Ready
    );
    assert_eq!(
        readiness.state("feishu"),
        loongclaw_daemon::migration::ChannelCredentialState::Partial
    );
    assert_eq!(
        readiness.state("unknown"),
        loongclaw_daemon::migration::ChannelCredentialState::Missing
    );
}

#[test]
fn channel_registry_collects_ready_channel_candidates() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.telegram.enabled = true;
    config.telegram.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
        "123456:test-token".to_owned(),
    ));
    config.feishu.enabled = true;
    config.feishu.app_id = Some(loongclaw_contracts::SecretRef::Inline(
        "cli_a1b2c3".to_owned(),
    ));
    config.feishu.app_secret = Some(loongclaw_contracts::SecretRef::Inline(
        "feishu-secret".to_owned(),
    ));
    config.line.enabled = true;
    config.line.channel_access_token = Some(loongclaw_contracts::SecretRef::Inline(
        "line-access-token".to_owned(),
    ));
    config.line.channel_secret = Some(loongclaw_contracts::SecretRef::Inline(
        "line-channel-secret".to_owned(),
    ));
    config.wecom.enabled = true;
    config.wecom.bot_id = Some(loongclaw_contracts::SecretRef::Inline(
        "wecom-bot".to_owned(),
    ));
    config.wecom.secret = Some(loongclaw_contracts::SecretRef::Inline(
        "wecom-secret".to_owned(),
    ));
    config.wecom.allowed_conversation_ids = vec!["group_demo".to_owned()];
    config.whatsapp.enabled = true;
    config.whatsapp.access_token = Some(loongclaw_contracts::SecretRef::Inline(
        "whatsapp-access-token".to_owned(),
    ));
    config.whatsapp.phone_number_id = Some("123456789".to_owned());
    config.whatsapp.verify_token = Some(loongclaw_contracts::SecretRef::Inline(
        "whatsapp-verify-token".to_owned(),
    ));
    config.whatsapp.app_secret = Some(loongclaw_contracts::SecretRef::Inline(
        "whatsapp-app-secret".to_owned(),
    ));
    config.webhook.enabled = true;
    config.webhook.endpoint_url = Some(loongclaw_contracts::SecretRef::Inline(
        "https://hooks.example.test/inbound".to_owned(),
    ));
    config.webhook.signing_secret = Some(loongclaw_contracts::SecretRef::Inline(
        "webhook-signing-secret".to_owned(),
    ));

    let previews = loongclaw_daemon::migration::channels::collect_channel_previews(
        &config,
        &loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config(
            &config,
        ),
        "test source",
    );
    let ids = previews
        .iter()
        .map(|preview| preview.candidate.id)
        .collect::<Vec<_>>();

    assert_eq!(
        ids,
        vec!["telegram", "feishu", "wecom", "line", "whatsapp", "webhook"]
    );
    assert!(
        previews.iter().all(|preview| {
            preview.candidate.status == loongclaw_daemon::migration::types::PreviewStatus::Ready
        }),
        "all registered channels should be ready when credentials resolve: {previews:#?}"
    );
}

#[test]
fn channel_registry_collects_managed_bridge_preview_when_enabled_bridge_needs_review() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.weixin.enabled = true;
    config.weixin.bridge_url = Some("https://bridge.example.test".to_owned());
    config.weixin.bridge_access_token = Some(loongclaw_contracts::SecretRef::Inline(
        "bridge-token".to_owned(),
    ));
    config.weixin.allowed_contact_ids = vec!["wx-contact".to_owned()];

    let readiness =
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config(
            &config,
        );
    let previews = loongclaw_daemon::migration::channels::collect_channel_previews(
        &config,
        &readiness,
        "test source",
    );
    let weixin_preview = previews
        .iter()
        .find(|preview| preview.candidate.id == "weixin")
        .expect("weixin preview");

    assert_eq!(
        weixin_preview.candidate.status,
        loongclaw_daemon::migration::types::PreviewStatus::NeedsReview
    );
    assert_eq!(
        weixin_preview.surface.level,
        loongclaw_daemon::migration::types::ImportSurfaceLevel::Review
    );
    assert!(
        weixin_preview
            .candidate
            .summary
            .contains("external_skills.install_root is not configured"),
        "enabled managed bridge channels should surface discovery guidance during migration preview: {weixin_preview:#?}"
    );
}

#[test]
fn channel_registry_collects_ready_managed_bridge_preview_when_compatible_plugin_exists() {
    let install_root = unique_temp_dir("managed-bridge-preview-ready");
    let mut config = mvp::config::LoongClawConfig::default();
    let manifest = managed_bridge_manifest(
        "weixin",
        Some("channel"),
        compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
    );

    std::fs::create_dir_all(&install_root).expect("create managed bridge install root");

    write_managed_bridge_manifest(install_root.as_path(), "weixin-ready-bridge", &manifest);

    config.weixin.enabled = true;
    config.weixin.bridge_url = Some("https://bridge.example.test".to_owned());
    config.weixin.bridge_access_token = Some(loongclaw_contracts::SecretRef::Inline(
        "bridge-token".to_owned(),
    ));
    config.weixin.allowed_contact_ids = vec!["wx-contact".to_owned()];
    config.external_skills.install_root = Some(install_root.display().to_string());

    let readiness =
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config(
            &config,
        );
    let previews = loongclaw_daemon::migration::channels::collect_channel_previews(
        &config,
        &readiness,
        "test source",
    );
    let weixin_preview = previews
        .iter()
        .find(|preview| preview.candidate.id == "weixin")
        .expect("weixin preview");

    assert_eq!(
        weixin_preview.candidate.status,
        loongclaw_daemon::migration::types::PreviewStatus::Ready
    );
    assert_eq!(
        weixin_preview.surface.level,
        loongclaw_daemon::migration::types::ImportSurfaceLevel::Ready
    );
    assert!(
        weixin_preview
            .candidate
            .summary
            .contains("weixin-managed-bridge"),
        "ready managed bridge previews should surface the compatible plugin identity: {weixin_preview:#?}"
    );
}

#[test]
fn channel_registry_keeps_managed_bridge_preview_in_review_when_bridge_contract_is_misconfigured() {
    let install_root = unique_temp_dir("managed-bridge-preview-contract-misconfigured");
    let manifest = managed_bridge_manifest(
        "weixin",
        Some("channel"),
        compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
    );
    let mut config: mvp::config::LoongClawConfig = serde_json::from_value(json!({
        "weixin": {
            "enabled": true,
            "bridge_access_token": "bridge-token",
            "allowed_contact_ids": ["wx-contact"]
        }
    }))
    .expect("deserialize weixin config");

    std::fs::create_dir_all(&install_root).expect("create managed bridge install root");
    write_managed_bridge_manifest(install_root.as_path(), "weixin-ready-bridge", &manifest);
    config.external_skills.install_root = Some(install_root.display().to_string());

    let readiness =
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config(
            &config,
        );
    let previews = loongclaw_daemon::migration::channels::collect_channel_previews(
        &config,
        &readiness,
        "test source",
    );
    let weixin_preview = previews
        .iter()
        .find(|preview| preview.candidate.id == "weixin")
        .expect("weixin preview");

    assert_eq!(
        weixin_preview.candidate.status,
        loongclaw_daemon::migration::types::PreviewStatus::NeedsReview
    );
    assert_eq!(
        weixin_preview.surface.level,
        loongclaw_daemon::migration::types::ImportSurfaceLevel::Review
    );
    assert!(
        weixin_preview
            .candidate
            .summary
            .contains("bridge_url is missing"),
        "managed bridge preview should keep channel contract misconfiguration visible even when discovery selects a compatible plugin: {weixin_preview:#?}"
    );
}

#[test]
fn channel_registry_summarizes_mixed_multi_account_managed_bridge_preview_detail() {
    let install_root = unique_temp_dir("managed-bridge-preview-multi-account-detail");
    let mut config = super::mixed_account_weixin_plugin_bridge_config();

    super::install_ready_weixin_managed_bridge(install_root.as_path());
    config.external_skills.install_root = Some(install_root.display().to_string());

    let readiness =
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config(
            &config,
        );
    let previews = loongclaw_daemon::migration::channels::collect_channel_previews(
        &config,
        &readiness,
        "test source",
    );
    let weixin_preview = previews
        .iter()
        .find(|preview| preview.candidate.id == "weixin")
        .expect("weixin preview");
    let summary = weixin_preview.candidate.summary.as_str();

    assert_eq!(
        weixin_preview.candidate.status,
        loongclaw_daemon::migration::types::PreviewStatus::NeedsReview
    );
    assert_eq!(
        weixin_preview.surface.level,
        loongclaw_daemon::migration::types::ImportSurfaceLevel::Review
    );
    assert!(
        summary.contains("weixin-managed-bridge"),
        "multi-account managed bridge preview should keep the selected plugin identity visible: {weixin_preview:#?}"
    );
    assert!(
        summary.contains("configured_account=ops"),
        "multi-account managed bridge preview should mention the ready default account: {weixin_preview:#?}"
    );
    assert!(
        summary.contains("(default): ready"),
        "multi-account managed bridge preview should mark the default account as ready when it passes contract checks: {weixin_preview:#?}"
    );
    assert!(
        summary.contains("configured_account=backup"),
        "multi-account managed bridge preview should mention blocked non-default accounts: {weixin_preview:#?}"
    );
    assert!(
        summary.contains("bridge_url is missing"),
        "multi-account managed bridge preview should keep the blocking contract detail visible: {weixin_preview:#?}"
    );
    assert!(
        summary.contains(super::MIXED_ACCOUNT_WEIXIN_PLUGIN_BRIDGE_SUMMARY),
        "preview detail should keep the shared mixed-account summary inside the discovery-ready prefix: {weixin_preview:#?}"
    );
}

#[test]
fn channel_preview_order_follows_shared_service_channel_catalog_order() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.telegram.enabled = true;
    config.telegram.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
        "123456:test-token".to_owned(),
    ));
    config.feishu.enabled = true;
    config.feishu.app_id = Some(loongclaw_contracts::SecretRef::Inline(
        "cli_a1b2c3".to_owned(),
    ));
    config.feishu.app_secret = Some(loongclaw_contracts::SecretRef::Inline(
        "feishu-secret".to_owned(),
    ));
    config.wecom.enabled = true;
    config.wecom.bot_id = Some(loongclaw_contracts::SecretRef::Inline(
        "wecom-bot".to_owned(),
    ));
    config.wecom.secret = Some(loongclaw_contracts::SecretRef::Inline(
        "wecom-secret".to_owned(),
    ));
    config.wecom.allowed_conversation_ids = vec!["group_demo".to_owned()];

    let previews = loongclaw_daemon::migration::channels::collect_channel_previews(
        &config,
        &loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config(
            &config,
        ),
        "test source",
    );
    let preview_ids = previews
        .iter()
        .map(|preview| preview.candidate.id)
        .collect::<Vec<_>>();
    let ordered_subset = mvp::config::service_channel_descriptors()
        .into_iter()
        .map(|descriptor| descriptor.id)
        .filter(|channel_id| preview_ids.contains(channel_id))
        .collect::<Vec<_>>();

    assert_eq!(preview_ids, ordered_subset);
}

#[test]
fn migration_classify_current_setup_treats_enabled_managed_bridge_without_ready_plugin_as_repairable()
 {
    let path = unique_temp_dir("managed-bridge-repairable").join("config.toml");
    let mut config = mvp::config::LoongClawConfig::default();

    config.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "provider-secret".to_owned(),
    ));
    config.provider.model = "openai/gpt-5.1-codex".to_owned();
    config.weixin.enabled = true;
    config.weixin.bridge_url = Some("https://bridge.example.test".to_owned());
    config.weixin.bridge_access_token = Some(loongclaw_contracts::SecretRef::Inline(
        "bridge-token".to_owned(),
    ));
    config.weixin.allowed_contact_ids = vec!["wx-contact".to_owned()];

    mvp::config::write(Some(path.to_string_lossy().as_ref()), &config, true)
        .expect("write managed bridge config");

    assert_eq!(
        loongclaw_daemon::migration::discovery::classify_current_setup(&path),
        loongclaw_daemon::migration::types::CurrentSetupState::Repairable,
        "enabled managed bridge channels without a discovered compatible plugin should keep the current setup in the repairable bucket"
    );
}

#[test]
fn migration_classify_current_setup_treats_blocked_outbound_only_channel_as_repairable() {
    let path = unique_temp_dir("outbound-only-repairable").join("config.toml");
    let mut config = mvp::config::LoongClawConfig::default();

    config.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "provider-secret".to_owned(),
    ));
    config.provider.model = "openai/gpt-5.1-codex".to_owned();
    config.discord.enabled = true;
    config.discord.bot_token = None;
    config.discord.bot_token_env = None;

    mvp::config::write(Some(path.to_string_lossy().as_ref()), &config, true)
        .expect("write outbound-only repairable config");

    assert_eq!(
        loongclaw_daemon::migration::discovery::classify_current_setup(&path),
        loongclaw_daemon::migration::types::CurrentSetupState::Repairable,
        "blocked outbound-only channels should keep the current setup in the repairable bucket instead of producing a false healthy result"
    );
}

#[test]
fn resolve_channel_import_readiness_reports_partial_channel_credentials() {
    let _env = MigrationEnvironmentGuard::set(&[("TELEGRAM_BOT_TOKEN", None)]);

    let mut config = mvp::config::LoongClawConfig::default();
    config.feishu.app_id = Some(loongclaw_contracts::SecretRef::Inline(
        "cli_a1b2c3".to_owned(),
    ));

    let readiness =
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config(
            &config,
        );

    assert_eq!(
        readiness.state("telegram"),
        loongclaw_daemon::migration::ChannelCredentialState::Missing
    );
    assert_eq!(
        readiness.state("feishu"),
        loongclaw_daemon::migration::ChannelCredentialState::Partial
    );
}

#[test]
fn channel_registry_lists_enabled_channel_ids() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.telegram.enabled = true;
    config.feishu.enabled = true;
    config.wecom.enabled = true;

    assert_eq!(
        loongclaw_daemon::migration::channels::registered_enabled_channel_ids(&config),
        vec!["telegram", "feishu", "wecom"]
    );
}

#[test]
fn channel_registry_enabled_ids_follow_app_service_channel_catalog() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.telegram.enabled = true;
    config.feishu.enabled = true;
    config.wecom.enabled = true;

    assert_eq!(
        config.enabled_service_channel_ids(),
        vec![
            "telegram".to_owned(),
            "feishu".to_owned(),
            "wecom".to_owned()
        ]
    );
    assert_eq!(
        loongclaw_daemon::migration::channels::registered_enabled_channel_ids(&config),
        vec!["telegram", "feishu", "wecom"]
    );
}

#[test]
fn channel_registry_collects_preflight_checks_for_enabled_channels() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.telegram.enabled = true;
    config.telegram.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
        "123456:test-token".to_owned(),
    ));
    config.feishu.enabled = true;
    config.feishu.app_id = Some(loongclaw_contracts::SecretRef::Inline(
        "cli_a1b2c3".to_owned(),
    ));
    config.feishu.app_secret = Some(loongclaw_contracts::SecretRef::Inline(
        "feishu-secret".to_owned(),
    ));
    config.feishu.verification_token = Some(loongclaw_contracts::SecretRef::Inline(
        "verify-token".to_owned(),
    ));
    config.wecom.enabled = true;
    config.wecom.bot_id = Some(loongclaw_contracts::SecretRef::Inline(
        "wecom-bot".to_owned(),
    ));
    config.wecom.secret = Some(loongclaw_contracts::SecretRef::Inline(
        "wecom-secret".to_owned(),
    ));
    config.wecom.allowed_conversation_ids = vec!["group_demo".to_owned()];

    let checks = loongclaw_daemon::migration::channels::collect_channel_preflight_checks(&config);

    assert!(
        checks.iter().any(|check| {
            check.name == "telegram channel"
                && check.level == loongclaw_daemon::migration::channels::ChannelCheckLevel::Pass
                && check.detail.contains("bot token resolved")
        }),
        "registry preflight should include telegram readiness: {checks:#?}"
    );
    assert!(
        checks.iter().any(|check| {
            check.name == "feishu channel"
                && check.level == loongclaw_daemon::migration::channels::ChannelCheckLevel::Pass
                && check.detail.contains("app credentials resolved")
        }),
        "registry preflight should include feishu app-credential readiness: {checks:#?}"
    );
    assert!(
        checks.iter().any(|check| {
            check.name == "feishu inbound transport"
                && check.level == loongclaw_daemon::migration::channels::ChannelCheckLevel::Pass
        }),
        "registry preflight should include feishu inbound transport readiness: {checks:#?}"
    );
    assert!(
        checks.iter().any(|check| {
            check.name == "wecom channel"
                && check.level == loongclaw_daemon::migration::channels::ChannelCheckLevel::Pass
                && check.detail.contains("bot credentials resolved")
        }),
        "registry preflight should include wecom credential readiness: {checks:#?}"
    );
    assert!(
        checks.iter().any(|check| {
            check.name == "wecom aibot long connection"
                && check.level == loongclaw_daemon::migration::channels::ChannelCheckLevel::Pass
        }),
        "registry preflight should include wecom long-connection readiness: {checks:#?}"
    );
}

#[test]
fn channel_registry_warns_when_managed_bridge_install_root_is_missing_for_enabled_plugin_backed_channel()
 {
    let config: mvp::config::LoongClawConfig = serde_json::from_value(json!({
        "weixin": {
            "enabled": true,
            "bridge_url": "https://bridge.example.test/weixin",
            "bridge_access_token": "weixin-token",
            "allowed_contact_ids": ["wxid_alice"]
        }
    }))
    .expect("deserialize weixin config");

    let checks = loongclaw_daemon::migration::channels::collect_channel_preflight_checks(&config);

    assert!(
        checks.iter().any(|check| {
            check.name == "weixin channel"
                && check.level == loongclaw_daemon::migration::channels::ChannelCheckLevel::Warn
                && check.detail.contains("external_skills.install_root")
        }),
        "registry preflight should surface managed bridge discovery guidance when install_root is missing: {checks:#?}"
    );
}

#[test]
fn channel_registry_passes_when_single_compatible_managed_bridge_is_available_for_enabled_plugin_backed_channel()
 {
    let install_root = unique_temp_dir("managed-bridge-preflight-ready");
    let manifest = super::managed_bridge_manifest(
        "weixin",
        Some("channel"),
        super::compatible_managed_bridge_metadata(
            "wechat_clawbot_ilink_bridge",
            "weixin_reply_loop",
        ),
    );
    let mut config: mvp::config::LoongClawConfig = serde_json::from_value(json!({
        "weixin": {
            "enabled": true,
            "bridge_url": "https://bridge.example.test/weixin",
            "bridge_access_token": "weixin-token",
            "allowed_contact_ids": ["wxid_alice"]
        }
    }))
    .expect("deserialize weixin config");

    std::fs::create_dir_all(&install_root).expect("create managed bridge install root");
    super::write_managed_bridge_manifest(
        install_root.as_path(),
        "weixin-managed-bridge",
        &manifest,
    );
    config.external_skills.install_root = Some(install_root.display().to_string());

    let checks = loongclaw_daemon::migration::channels::collect_channel_preflight_checks(&config);

    assert!(
        checks.iter().any(|check| {
            check.name == "weixin channel"
                && check.level == loongclaw_daemon::migration::channels::ChannelCheckLevel::Pass
                && check.detail.contains("weixin-managed-bridge")
        }),
        "registry preflight should pass when exactly one compatible managed bridge is ready: {checks:#?}"
    );
}

#[test]
fn channel_registry_warns_when_managed_bridge_setup_is_incomplete_for_enabled_plugin_backed_channel()
 {
    let install_root = unique_temp_dir("managed-bridge-preflight-incomplete");
    let mut metadata = super::compatible_managed_bridge_metadata(
        "qq_official_bot_gateway_or_plugin_bridge",
        "qqbot_reply_loop",
    );
    let removed_transport_family = metadata.remove("transport_family");
    let setup = super::managed_bridge_setup_with_guidance(
        "channel",
        vec!["QQBOT_BRIDGE_URL"],
        vec!["qqbot.bridge_url"],
        vec!["https://example.test/docs/qqbot-bridge"],
        Some("Run the QQ bridge setup flow before enabling this bridge."),
    );
    let manifest = super::managed_bridge_manifest_with_setup("qqbot", metadata, Some(setup));
    let mut config: mvp::config::LoongClawConfig = serde_json::from_value(json!({
        "qqbot": {
            "enabled": true,
            "app_id": "10001",
            "client_secret": "qqbot-secret",
            "allowed_peer_ids": ["openid-alice"]
        }
    }))
    .expect("deserialize qqbot config");

    assert_eq!(
        removed_transport_family.as_deref(),
        Some("qq_official_bot_gateway_or_plugin_bridge")
    );

    std::fs::create_dir_all(&install_root).expect("create managed bridge install root");
    super::write_managed_bridge_manifest(install_root.as_path(), "qqbot-bridge-guided", &manifest);
    config.external_skills.install_root = Some(install_root.display().to_string());

    let checks = loongclaw_daemon::migration::channels::collect_channel_preflight_checks(&config);

    assert!(
        checks.iter().any(|check| {
            check.name == "qq bot channel"
                && check.level == loongclaw_daemon::migration::channels::ChannelCheckLevel::Warn
                && check.detail.contains("QQBOT_BRIDGE_URL")
                && check.detail.contains("qqbot.bridge_url")
        }),
        "registry preflight should preserve managed bridge setup guidance when discovery finds only incomplete plugins: {checks:#?}"
    );
}

#[test]
fn channel_registry_warns_when_managed_bridge_discovery_is_ambiguous_for_enabled_plugin_backed_channel()
 {
    let install_root = unique_temp_dir("managed-bridge-preflight-ambiguous");
    let first_manifest = super::managed_bridge_manifest_with_plugin_id(
        "weixin-bridge-a",
        "weixin",
        super::compatible_managed_bridge_metadata(
            "wechat_clawbot_ilink_bridge",
            "weixin_reply_loop",
        ),
        Some(super::managed_bridge_setup_with_guidance(
            "channel",
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        )),
    );
    let second_manifest = super::managed_bridge_manifest_with_plugin_id(
        "weixin-bridge-b",
        "weixin",
        super::compatible_managed_bridge_metadata(
            "wechat_clawbot_ilink_bridge",
            "weixin_reply_loop",
        ),
        Some(super::managed_bridge_setup_with_guidance(
            "channel",
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        )),
    );
    let mut config: mvp::config::LoongClawConfig = serde_json::from_value(json!({
        "weixin": {
            "enabled": true,
            "bridge_url": "https://bridge.example.test/weixin",
            "bridge_access_token": "weixin-token",
            "allowed_contact_ids": ["wxid_alice"]
        }
    }))
    .expect("deserialize weixin config");

    std::fs::create_dir_all(&install_root).expect("create managed bridge install root");
    super::write_managed_bridge_manifest(
        install_root.as_path(),
        "weixin-bridge-a",
        &first_manifest,
    );
    super::write_managed_bridge_manifest(
        install_root.as_path(),
        "weixin-bridge-b",
        &second_manifest,
    );
    config.external_skills.install_root = Some(install_root.display().to_string());

    let checks = loongclaw_daemon::migration::channels::collect_channel_preflight_checks(&config);

    assert!(
        checks.iter().any(|check| {
            check.name == "weixin channel"
                && check.level == loongclaw_daemon::migration::channels::ChannelCheckLevel::Warn
                && check.detail.contains("weixin-bridge-a")
                && check.detail.contains("weixin-bridge-b")
        }),
        "registry preflight should warn when multiple compatible managed bridges are discovered: {checks:#?}"
    );
}

#[test]
fn channel_registry_passes_when_managed_bridge_plugin_id_selects_a_compatible_plugin() {
    let install_root = unique_temp_dir("managed-bridge-preflight-selected");
    let first_manifest = super::managed_bridge_manifest_with_plugin_id(
        "weixin-bridge-a",
        "weixin",
        super::compatible_managed_bridge_metadata(
            "wechat_clawbot_ilink_bridge",
            "weixin_reply_loop",
        ),
        Some(super::managed_bridge_setup_with_guidance(
            "channel",
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        )),
    );
    let second_manifest = super::managed_bridge_manifest_with_plugin_id(
        "weixin-bridge-b",
        "weixin",
        super::compatible_managed_bridge_metadata(
            "wechat_clawbot_ilink_bridge",
            "weixin_reply_loop",
        ),
        Some(super::managed_bridge_setup_with_guidance(
            "channel",
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        )),
    );
    let mut config: mvp::config::LoongClawConfig = serde_json::from_value(json!({
        "weixin": {
            "enabled": true,
            "managed_bridge_plugin_id": "weixin-bridge-b",
            "bridge_url": "https://bridge.example.test/weixin",
            "bridge_access_token": "weixin-token",
            "allowed_contact_ids": ["wxid_alice"]
        }
    }))
    .expect("deserialize weixin config");

    std::fs::create_dir_all(&install_root).expect("create managed bridge install root");
    super::write_managed_bridge_manifest(
        install_root.as_path(),
        "weixin-bridge-a",
        &first_manifest,
    );
    super::write_managed_bridge_manifest(
        install_root.as_path(),
        "weixin-bridge-b",
        &second_manifest,
    );
    config.external_skills.install_root = Some(install_root.display().to_string());

    let checks = loongclaw_daemon::migration::channels::collect_channel_preflight_checks(&config);

    assert!(
        checks.iter().any(|check| {
            check.name == "weixin channel"
                && check.level == loongclaw_daemon::migration::channels::ChannelCheckLevel::Pass
                && check.detail.contains("selected plugin weixin-bridge-b")
        }),
        "registry preflight should pass when managed_bridge_plugin_id resolves the compatible bridge: {checks:#?}"
    );
}

#[test]
fn channel_registry_warns_when_managed_bridge_plugin_id_does_not_match_any_plugin() {
    let install_root = unique_temp_dir("managed-bridge-preflight-selection-missing");
    let first_manifest = super::managed_bridge_manifest_with_plugin_id(
        "weixin-bridge-a",
        "weixin",
        super::compatible_managed_bridge_metadata(
            "wechat_clawbot_ilink_bridge",
            "weixin_reply_loop",
        ),
        Some(super::managed_bridge_setup_with_guidance(
            "channel",
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        )),
    );
    let second_manifest = super::managed_bridge_manifest_with_plugin_id(
        "weixin-bridge-b",
        "weixin",
        super::compatible_managed_bridge_metadata(
            "wechat_clawbot_ilink_bridge",
            "weixin_reply_loop",
        ),
        Some(super::managed_bridge_setup_with_guidance(
            "channel",
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        )),
    );
    let mut config: mvp::config::LoongClawConfig = serde_json::from_value(json!({
        "weixin": {
            "enabled": true,
            "managed_bridge_plugin_id": "missing-bridge",
            "bridge_url": "https://bridge.example.test/weixin",
            "bridge_access_token": "weixin-token",
            "allowed_contact_ids": ["wxid_alice"]
        }
    }))
    .expect("deserialize weixin config");

    std::fs::create_dir_all(&install_root).expect("create managed bridge install root");
    super::write_managed_bridge_manifest(
        install_root.as_path(),
        "weixin-bridge-a",
        &first_manifest,
    );
    super::write_managed_bridge_manifest(
        install_root.as_path(),
        "weixin-bridge-b",
        &second_manifest,
    );
    config.external_skills.install_root = Some(install_root.display().to_string());

    let checks = loongclaw_daemon::migration::channels::collect_channel_preflight_checks(&config);

    assert!(
        checks.iter().any(|check| {
            check.name == "weixin channel"
                && check.level == loongclaw_daemon::migration::channels::ChannelCheckLevel::Warn
                && check
                    .detail
                    .contains("managed_bridge_plugin_id=missing-bridge")
                && check.detail.contains("weixin-bridge-a")
                && check.detail.contains("weixin-bridge-b")
        }),
        "registry preflight should explain when managed_bridge_plugin_id does not match any discovered plugin: {checks:#?}"
    );
}

#[test]
fn channel_registry_warns_when_managed_bridge_plugin_id_matches_duplicate_packages() {
    let install_root = unique_temp_dir("managed-bridge-preflight-selection-duplicated");
    let first_manifest = super::managed_bridge_manifest_with_plugin_id(
        "weixin-bridge-shared",
        "weixin",
        super::compatible_managed_bridge_metadata(
            "wechat_clawbot_ilink_bridge",
            "weixin_reply_loop",
        ),
        Some(super::managed_bridge_setup_with_guidance(
            "channel",
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        )),
    );
    let second_manifest = super::managed_bridge_manifest_with_plugin_id(
        "weixin-bridge-shared",
        "weixin",
        super::compatible_managed_bridge_metadata(
            "wechat_clawbot_ilink_bridge",
            "weixin_reply_loop",
        ),
        Some(super::managed_bridge_setup_with_guidance(
            "channel",
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        )),
    );
    let mut config: mvp::config::LoongClawConfig = serde_json::from_value(json!({
        "weixin": {
            "enabled": true,
            "managed_bridge_plugin_id": "weixin-bridge-shared",
            "bridge_url": "https://bridge.example.test/weixin",
            "bridge_access_token": "weixin-token",
            "allowed_contact_ids": ["wxid_alice"]
        }
    }))
    .expect("deserialize weixin config");

    std::fs::create_dir_all(&install_root).expect("create managed bridge install root");
    super::write_managed_bridge_manifest(
        install_root.as_path(),
        "weixin-bridge-a",
        &first_manifest,
    );
    super::write_managed_bridge_manifest(
        install_root.as_path(),
        "weixin-bridge-b",
        &second_manifest,
    );
    config.external_skills.install_root = Some(install_root.display().to_string());

    let checks = loongclaw_daemon::migration::channels::collect_channel_preflight_checks(&config);

    assert!(
        checks.iter().any(|check| {
            check.name == "weixin channel"
                && check.level == loongclaw_daemon::migration::channels::ChannelCheckLevel::Warn
                && check
                    .detail
                    .contains("managed_bridge_plugin_id=weixin-bridge-shared")
                && check.detail.contains("weixin-bridge-shared")
        }),
        "registry preflight should reject configured managed bridge ids that match multiple packages: {checks:#?}"
    );
}

#[test]
fn channel_registry_ignores_unconfigured_plugin_backed_channels_even_when_managed_bridge_is_installed()
 {
    let install_root = unique_temp_dir("managed-bridge-preflight-unconfigured");
    let manifest = super::managed_bridge_manifest(
        "weixin",
        Some("channel"),
        super::compatible_managed_bridge_metadata(
            "wechat_clawbot_ilink_bridge",
            "weixin_reply_loop",
        ),
    );
    let mut config = mvp::config::LoongClawConfig::default();

    std::fs::create_dir_all(&install_root).expect("create managed bridge install root");
    super::write_managed_bridge_manifest(
        install_root.as_path(),
        "weixin-managed-bridge",
        &manifest,
    );
    config.external_skills.install_root = Some(install_root.display().to_string());

    let checks = loongclaw_daemon::migration::channels::collect_channel_preflight_checks(&config);

    assert!(
        checks.iter().all(|check| check.name != "weixin channel"),
        "registry preflight should not surface unmanaged plugin-backed channels that still match the default placeholder snapshot: {checks:#?}"
    );
}

#[test]
fn channel_registry_collects_serve_actions_for_enabled_channels() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.telegram.enabled = true;
    config.feishu.enabled = true;
    config.wecom.enabled = true;

    let actions = loongclaw_daemon::migration::channels::collect_channel_next_actions(
        &config,
        "/tmp/loongclaw-config.toml",
    );

    assert_eq!(actions.len(), 3);
    assert_eq!(actions[0].label, "Telegram");
    assert_eq!(
        actions[0].command,
        "loong telegram-serve --config '/tmp/loongclaw-config.toml'"
    );
    assert_eq!(actions[1].label, "Feishu/Lark");
    assert_eq!(
        actions[1].command,
        "loong feishu-serve --config '/tmp/loongclaw-config.toml'"
    );
    assert_eq!(actions[2].label, "WeCom");
    assert_eq!(
        actions[2].command,
        "loong wecom-serve --config '/tmp/loongclaw-config.toml'"
    );
}

#[test]
fn channel_registry_apply_selected_channels_merges_wecom_config() {
    let mut target = mvp::config::LoongClawConfig::default();
    target.wecom.secret_env = Some("TARGET_WECOM_SECRET".to_owned());

    let mut source = mvp::config::LoongClawConfig::default();
    source.wecom.enabled = true;
    source.wecom.default_account = Some("ops".to_owned());
    source.wecom.bot_id_env = Some("SOURCE_WECOM_BOT_ID".to_owned());
    source.wecom.secret_env = Some("SOURCE_WECOM_SECRET".to_owned());
    source.wecom.allowed_conversation_ids = vec!["group_alpha".to_owned()];
    source.wecom.accounts.insert(
        "ops".to_owned(),
        mvp::config::WecomAccountConfig {
            websocket_url: Some("wss://wecom.example.test".to_owned()),
            allowed_conversation_ids: Some(vec!["group_ops".to_owned()]),
            ..Default::default()
        },
    );

    let changed = loongclaw_daemon::migration::channels::apply_selected_channels(
        &mut target,
        &source,
        &["wecom"],
    );

    assert!(changed);
    assert!(target.wecom.enabled);
    assert_eq!(target.wecom.default_account.as_deref(), Some("ops"));
    assert_eq!(
        target.wecom.bot_id_env.as_deref(),
        Some("SOURCE_WECOM_BOT_ID")
    );
    assert_eq!(
        target.wecom.secret_env.as_deref(),
        Some("TARGET_WECOM_SECRET")
    );
    assert_eq!(target.wecom.allowed_conversation_ids, vec!["group_alpha"]);
    assert_eq!(
        target
            .wecom
            .accounts
            .get("ops")
            .and_then(|account| account.websocket_url.as_deref()),
        Some("wss://wecom.example.test")
    );
}

#[test]
fn resolve_channel_import_readiness_reports_partial_wecom_channel_credentials() {
    let _env = MigrationEnvironmentGuard::set(&[("WECOM_BOT_ID", None), ("WECOM_SECRET", None)]);

    let mut config = mvp::config::LoongClawConfig::default();
    config.wecom.bot_id = Some(loongclaw_contracts::SecretRef::Inline(
        "wecom-bot".to_owned(),
    ));

    let readiness =
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config(
            &config,
        );

    assert_eq!(
        readiness.state("wecom"),
        loongclaw_daemon::migration::ChannelCredentialState::Partial
    );
}

#[test]
fn channel_registry_collects_catalog_action_when_no_service_channels_are_enabled() {
    let config = mvp::config::LoongClawConfig::default();

    let actions = loongclaw_daemon::migration::channels::collect_channel_next_actions(
        &config,
        "/tmp/loongclaw-config.toml",
    );

    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].id, "channel_catalog");
    assert_eq!(actions[0].label, "channels");
    assert_eq!(
        actions[0].command,
        "loong channels --config '/tmp/loongclaw-config.toml'"
    );
}

#[test]
fn channel_registry_collects_contextual_inspection_action_for_enabled_discord_surface() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.discord.enabled = true;
    config.discord.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
        "discord-token".to_owned(),
    ));

    let actions = loongclaw_daemon::migration::channels::collect_channel_next_actions(
        &config,
        "/tmp/loongclaw-config.toml",
    );

    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].id, "configured_channels");
    assert_eq!(actions[0].label, "inspect Discord");
    assert_eq!(
        actions[0].command,
        "loong channels --config '/tmp/loongclaw-config.toml'"
    );
}

#[test]
fn channel_registry_collects_contextual_review_action_for_blocked_discord_surface() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.discord.enabled = true;
    config.discord.bot_token = None;
    config.discord.bot_token_env = None;

    let actions = loongclaw_daemon::migration::channels::collect_channel_next_actions(
        &config,
        "/tmp/loongclaw-config.toml",
    );

    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].id, "configured_channels");
    assert_eq!(actions[0].label, "review Discord setup");
    assert_eq!(
        actions[0].command,
        "loong channels --config '/tmp/loongclaw-config.toml'"
    );
}

#[test]
fn channel_registry_collects_outbound_inspection_action_for_multiple_enabled_outbound_surfaces() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.discord.enabled = true;
    config.discord.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
        "discord-token".to_owned(),
    ));
    config.slack.enabled = true;
    config.slack.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
        "xoxb-test-token".to_owned(),
    ));

    let actions = loongclaw_daemon::migration::channels::collect_channel_next_actions(
        &config,
        "/tmp/loongclaw-config.toml",
    );

    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].id, "configured_channels");
    assert_eq!(actions[0].label, "inspect configured outbound channels");
    assert_eq!(
        actions[0].command,
        "loong channels --config '/tmp/loongclaw-config.toml'"
    );
}

#[test]
fn channel_registry_keeps_non_runtime_inspection_action_when_runtime_channel_is_enabled() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.telegram.enabled = true;
    config.discord.enabled = true;
    config.discord.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
        "discord-token".to_owned(),
    ));

    let actions = loongclaw_daemon::migration::channels::collect_channel_next_actions(
        &config,
        "/tmp/loongclaw-config.toml",
    );

    assert_eq!(actions.len(), 2);
    assert_eq!(actions[0].label, "Telegram");
    assert_eq!(
        actions[0].command,
        "loong telegram-serve --config '/tmp/loongclaw-config.toml'"
    );
    assert_eq!(actions[1].id, "configured_channels");
    assert_eq!(actions[1].label, "inspect Discord");
    assert_eq!(
        actions[1].command,
        "loong channels --config '/tmp/loongclaw-config.toml'"
    );
}

#[test]
fn channel_registry_collects_outbound_review_action_when_multiple_outbound_surfaces_need_attention()
{
    let mut config = mvp::config::LoongClawConfig::default();
    config.discord.enabled = true;
    config.discord.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
        "discord-token".to_owned(),
    ));
    config.slack.enabled = true;
    config.slack.bot_token = None;
    config.slack.bot_token_env = None;

    let actions = loongclaw_daemon::migration::channels::collect_channel_next_actions(
        &config,
        "/tmp/loongclaw-config.toml",
    );

    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].id, "configured_channels");
    assert_eq!(actions[0].label, "review configured outbound channels");
    assert_eq!(
        actions[0].command,
        "loong channels --config '/tmp/loongclaw-config.toml'"
    );
}

#[test]
fn channel_registry_collects_contextual_inspection_action_for_enabled_weixin_bridge_surface() {
    let config: mvp::config::LoongClawConfig = serde_json::from_value(json!({
        "weixin": {
            "enabled": true,
            "bridge_url": "https://bridge.example.test/weixin",
            "bridge_access_token": "weixin-token",
            "allowed_contact_ids": ["wxid_alice"]
        }
    }))
    .expect("deserialize weixin config");

    let actions = loongclaw_daemon::migration::channels::collect_channel_next_actions(
        &config,
        "/tmp/loongclaw-config.toml",
    );

    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].id, "configured_channels");
    assert_eq!(actions[0].label, "review Weixin bridge");
    assert_eq!(
        actions[0].command,
        "loong channels --config '/tmp/loongclaw-config.toml'"
    );
}

#[test]
fn migration_render_preview_compacts_for_narrow_width() {
    let candidate = loongclaw_daemon::migration::types::ImportCandidate {
        source_kind: loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
        source: "Codex config at ~/.codex/config.toml".to_owned(),
        config: mvp::config::LoongClawConfig::default(),
        surfaces: Vec::new(),
        domains: vec![
            loongclaw_daemon::migration::types::DomainPreview {
                kind: loongclaw_daemon::migration::types::SetupDomainKind::Provider,
                status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
                decision: Some(loongclaw_daemon::migration::types::PreviewDecision::UseDetected),
                source: "Codex config at ~/.codex/config.toml".to_owned(),
                summary: "openai · openai/gpt-5.1-codex · credentials resolved".to_owned(),
            },
            loongclaw_daemon::migration::types::DomainPreview {
                kind: loongclaw_daemon::migration::types::SetupDomainKind::WorkspaceGuidance,
                status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
                decision: Some(loongclaw_daemon::migration::types::PreviewDecision::UseDetected),
                source: "workspace".to_owned(),
                summary: "AGENTS.md".to_owned(),
            },
        ],
        channel_candidates: vec![loongclaw_daemon::migration::types::ChannelCandidate {
            id: "telegram",
            label: "telegram",
            status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
            source: "Codex config at ~/.codex/config.toml".to_owned(),
            summary: "token resolved · can enable during onboarding".to_owned(),
        }],
        workspace_guidance: vec![
            loongclaw_daemon::migration::types::WorkspaceGuidanceCandidate {
                kind: loongclaw_daemon::migration::types::WorkspaceGuidanceKind::Agents,
                path: "/tmp/project/AGENTS.md".to_owned(),
            },
        ],
    };

    let lines = loongclaw_daemon::migration::render::render_candidate_preview_lines(&candidate, 54);

    assert!(
        lines.iter().any(|line| line.contains("provider [Ready]")),
        "narrow preview should keep a compact domain header: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("source: workspace")),
        "narrow preview should preserve source attribution on its own line: {lines:#?}"
    );
}

#[test]
fn migration_render_preview_wraps_long_domain_and_channel_details_for_narrow_width() {
    let candidate = loongclaw_daemon::migration::types::ImportCandidate {
        source_kind: loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
        source: "Codex config at ~/.codex/agents/loongclaw/config.toml".to_owned(),
        config: mvp::config::LoongClawConfig::default(),
        surfaces: Vec::new(),
        domains: vec![loongclaw_daemon::migration::types::DomainPreview {
            kind: loongclaw_daemon::migration::types::SetupDomainKind::Provider,
            status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
            decision: Some(loongclaw_daemon::migration::types::PreviewDecision::UseDetected),
            source: "Codex config at ~/.codex/agents/loongclaw/config.toml".to_owned(),
            summary: "openai · openai/gpt-5.1-codex · credentials resolved from environment"
                .to_owned(),
        }],
        channel_candidates: vec![loongclaw_daemon::migration::types::ChannelCandidate {
            id: "telegram",
            label: "telegram",
            status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
            source: "Codex config at ~/.codex/agents/loongclaw/config.toml".to_owned(),
            summary: "token resolved · can enable during onboarding".to_owned(),
        }],
        workspace_guidance: Vec::new(),
    };

    let lines = loongclaw_daemon::migration::render::render_candidate_preview_lines(&candidate, 48);

    assert!(
        lines.iter().any(|line| line == "source: Codex config at"),
        "narrow preview should keep the source label on the first wrapped line: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "  ~/.codex/agents/loongclaw/config.toml"),
        "narrow preview should continue long source paths on an indented line: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "  summary: openai · openai/gpt-5.1-codex ·"),
        "narrow preview should wrap long domain summaries without dropping the action context: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "  credentials resolved from environment"),
        "narrow preview should continue wrapped domain summaries on a readable continuation line: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "- telegram [Ready · runtime-backed]"),
        "narrow preview should keep channel rows compact before detail lines: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "  summary: token resolved · can enable during"),
        "narrow preview should wrap long channel summaries instead of cramming them into the header row: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "  onboarding"),
        "narrow preview should continue wrapped channel summaries on an indented line: {lines:#?}"
    );
}

#[test]
fn migration_render_preview_includes_channel_details_and_guidance() {
    let candidate = loongclaw_daemon::migration::types::ImportCandidate {
        source_kind: loongclaw_daemon::migration::types::ImportSourceKind::Environment,
        source: "your current environment".to_owned(),
        config: mvp::config::LoongClawConfig::default(),
        surfaces: Vec::new(),
        domains: vec![
            loongclaw_daemon::migration::types::DomainPreview {
                kind: loongclaw_daemon::migration::types::SetupDomainKind::Channels,
                status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
                decision: Some(loongclaw_daemon::migration::types::PreviewDecision::UseDetected),
                source: "your current environment".to_owned(),
                summary: "telegram Ready · feishu Ready".to_owned(),
            },
            loongclaw_daemon::migration::types::DomainPreview {
                kind: loongclaw_daemon::migration::types::SetupDomainKind::WorkspaceGuidance,
                status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
                decision: Some(loongclaw_daemon::migration::types::PreviewDecision::UseDetected),
                source: "workspace".to_owned(),
                summary: "AGENTS.md, CLAUDE.md".to_owned(),
            },
        ],
        channel_candidates: vec![
            loongclaw_daemon::migration::types::ChannelCandidate {
                id: "telegram",
                label: "telegram",
                status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
                source: "your current environment".to_owned(),
                summary: "token resolved · can enable during onboarding".to_owned(),
            },
            loongclaw_daemon::migration::types::ChannelCandidate {
                id: "feishu",
                label: "feishu",
                status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
                source: "your current environment".to_owned(),
                summary: "app credentials resolved · can enable during onboarding".to_owned(),
            },
        ],
        workspace_guidance: vec![
            loongclaw_daemon::migration::types::WorkspaceGuidanceCandidate {
                kind: loongclaw_daemon::migration::types::WorkspaceGuidanceKind::Agents,
                path: "/tmp/project/AGENTS.md".to_owned(),
            },
            loongclaw_daemon::migration::types::WorkspaceGuidanceCandidate {
                kind: loongclaw_daemon::migration::types::WorkspaceGuidanceKind::Claude,
                path: "/tmp/project/CLAUDE.md".to_owned(),
            },
        ],
    };

    let lines =
        loongclaw_daemon::migration::render::render_candidate_preview_lines(&candidate, 100);

    assert!(
        lines.iter().any(|line| line.contains("channels:")),
        "wide preview should include a channel detail section: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("your current environment")),
        "channel detail lines should preserve source attribution: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("workspace guidance")),
        "typed preview should expose workspace guidance as a first-class domain: {lines:#?}"
    );
}

#[test]
fn migration_render_preview_falls_back_to_stacked_rows_when_wide_table_would_overflow() {
    let candidate = loongclaw_daemon::migration::types::ImportCandidate {
        source_kind: loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
        source: "Codex config at ~/.codex/agents/loongclaw/config.toml".to_owned(),
        config: mvp::config::LoongClawConfig::default(),
        surfaces: Vec::new(),
        domains: vec![
            loongclaw_daemon::migration::types::DomainPreview {
                kind: loongclaw_daemon::migration::types::SetupDomainKind::Provider,
                status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
                decision: Some(loongclaw_daemon::migration::types::PreviewDecision::UseDetected),
                source: "Codex config at ~/.codex/agents/loongclaw/config.toml".to_owned(),
                summary: "openai · openai/gpt-5.1-codex · credentials resolved from environment"
                    .to_owned(),
            },
            loongclaw_daemon::migration::types::DomainPreview {
                kind: loongclaw_daemon::migration::types::SetupDomainKind::WorkspaceGuidance,
                status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
                decision: Some(loongclaw_daemon::migration::types::PreviewDecision::UseDetected),
                source: "workspace".to_owned(),
                summary: "AGENTS.md, CLAUDE.md, and custom workspace instructions".to_owned(),
            },
        ],
        channel_candidates: vec![loongclaw_daemon::migration::types::ChannelCandidate {
            id: "telegram",
            label: "telegram",
            status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
            source: "Codex config at ~/.codex/agents/loongclaw/config.toml".to_owned(),
            summary: "token resolved · can enable during onboarding".to_owned(),
        }],
        workspace_guidance: Vec::new(),
    };

    let lines = loongclaw_daemon::migration::render::render_candidate_preview_lines(&candidate, 80);

    assert!(
        lines.iter().all(|line| line != "domains:"),
        "medium-width preview should avoid switching into a wide table when rows would overflow: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "- provider [Ready]"),
        "medium-width preview should fall back to stacked domain rows when the wide row would be too long: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line
                == "  summary: openai · openai/gpt-5.1-codex · credentials resolved from"),
        "stacked fallback should keep long summaries readable instead of forcing one overlong table row: {lines:#?}"
    );
}

#[test]
fn migration_render_preview_falls_back_to_stacked_channel_rows_when_wide_line_would_overflow() {
    let candidate = loongclaw_daemon::migration::types::ImportCandidate {
        source_kind: loongclaw_daemon::migration::types::ImportSourceKind::Environment,
        source: "your current environment".to_owned(),
        config: mvp::config::LoongClawConfig::default(),
        surfaces: Vec::new(),
        domains: vec![loongclaw_daemon::migration::types::DomainPreview {
            kind: loongclaw_daemon::migration::types::SetupDomainKind::Channels,
            status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
            decision: Some(loongclaw_daemon::migration::types::PreviewDecision::UseDetected),
            source: "your current environment".to_owned(),
            summary: "telegram Ready".to_owned(),
        }],
        channel_candidates: vec![loongclaw_daemon::migration::types::ChannelCandidate {
            id: "telegram",
            label: "telegram",
            status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
            source: "Codex config at ~/.codex/agents/loongclaw/config.toml".to_owned(),
            summary:
                "token resolved and channel defaults can be imported during onboarding without losing existing allowlist or timeout settings"
                    .to_owned(),
        }],
        workspace_guidance: Vec::new(),
    };

    let lines = loongclaw_daemon::migration::render::render_candidate_preview_lines(&candidate, 80);

    assert!(
        lines.iter().any(|line| line == "channels:"),
        "channel fallback should keep the channel section heading visible: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "- telegram [Ready · runtime-backed]"),
        "medium-width preview should fall back to stacked channel rows when the wide row would overflow: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "  source: Codex config at ~/.codex/agents/loongclaw/config.toml"),
        "stacked channel fallback should keep source attribution on its own readable line: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line
            == "  summary: token resolved and channel defaults can be imported during onboarding"),
        "stacked channel fallback should wrap long summary text instead of reverting to a wide single-line row: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "  without losing existing allowlist or timeout settings"),
        "stacked channel fallback should continue wrapped summary text on a readable continuation line: {lines:#?}"
    );
}

#[test]
fn migration_render_preview_surfaces_domain_actions() {
    let candidate = loongclaw_daemon::migration::types::ImportCandidate {
        source_kind: loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan,
        source: "recommended import plan".to_owned(),
        config: mvp::config::LoongClawConfig::default(),
        surfaces: Vec::new(),
        domains: vec![
            loongclaw_daemon::migration::types::DomainPreview {
                kind: loongclaw_daemon::migration::types::SetupDomainKind::Provider,
                status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
                decision: Some(loongclaw_daemon::migration::types::PreviewDecision::KeepCurrent),
                source: "existing config at ~/.config/loongclaw/config.toml".to_owned(),
                summary: "openai · openai/gpt-5.1-codex · credentials resolved".to_owned(),
            },
            loongclaw_daemon::migration::types::DomainPreview {
                kind: loongclaw_daemon::migration::types::SetupDomainKind::Channels,
                status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
                decision: Some(loongclaw_daemon::migration::types::PreviewDecision::Supplement),
                source: "multiple sources".to_owned(),
                summary: "telegram Ready".to_owned(),
            },
        ],
        channel_candidates: Vec::new(),
        workspace_guidance: Vec::new(),
    };

    let lines =
        loongclaw_daemon::migration::render::render_candidate_preview_lines(&candidate, 100);

    assert!(
        lines.iter().any(|line| line.contains("keep current value")),
        "preview should explicitly show when a domain keeps the current value: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("supplement with detected values")),
        "preview should explicitly show when a domain is being supplemented from other detected sources: {lines:#?}"
    );
}

#[test]
fn migration_render_preview_summarizes_multi_source_inputs() {
    let candidate = loongclaw_daemon::migration::types::ImportCandidate {
        source_kind: loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan,
        source: "recommended import plan".to_owned(),
        config: mvp::config::LoongClawConfig::default(),
        surfaces: Vec::new(),
        domains: vec![
            loongclaw_daemon::migration::types::DomainPreview {
                kind: loongclaw_daemon::migration::types::SetupDomainKind::Provider,
                status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
                decision: Some(loongclaw_daemon::migration::types::PreviewDecision::KeepCurrent),
                source: "existing config at ~/.config/loongclaw/config.toml".to_owned(),
                summary: "openai · openai/gpt-5.1-codex · credentials resolved".to_owned(),
            },
            loongclaw_daemon::migration::types::DomainPreview {
                kind: loongclaw_daemon::migration::types::SetupDomainKind::WorkspaceGuidance,
                status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
                decision: Some(loongclaw_daemon::migration::types::PreviewDecision::UseDetected),
                source: "workspace".to_owned(),
                summary: "AGENTS.md".to_owned(),
            },
        ],
        channel_candidates: vec![loongclaw_daemon::migration::types::ChannelCandidate {
            id: "telegram",
            label: "telegram",
            status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
            source: "your current environment".to_owned(),
            summary: "token resolved · can enable during onboarding".to_owned(),
        }],
        workspace_guidance: vec![
            loongclaw_daemon::migration::types::WorkspaceGuidanceCandidate {
                kind: loongclaw_daemon::migration::types::WorkspaceGuidanceKind::Agents,
                path: "/tmp/project/AGENTS.md".to_owned(),
            },
        ],
    };

    let joined =
        loongclaw_daemon::migration::render::render_candidate_preview_lines(&candidate, 80)
            .join("\n");

    assert!(
        joined.contains("derived from:"),
        "preview should add a dedicated multi-source attribution line for composed candidates: {joined}"
    );
    assert!(
        joined.contains("existing config at ~/.config/loongclaw/config.toml"),
        "preview should keep the current-config contribution visible in the source rollup: {joined}"
    );
    assert!(
        joined.contains("your current environment"),
        "preview should keep environment-derived contributions visible in the source rollup: {joined}"
    );
    assert!(
        joined.contains("workspace guidance"),
        "preview should call out workspace guidance as one of the composed sources: {joined}"
    );
}

#[test]
fn migration_compose_recommended_candidate_supplements_channels_without_overwriting_ready_provider()
{
    let _env = MigrationEnvironmentGuard::set(&[("TELEGRAM_BOT_TOKEN", None)]);

    let mut existing = mvp::config::LoongClawConfig::default();
    existing.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "openai-secret".to_owned(),
    ));
    existing.provider.model = "openai/gpt-5.1-codex".to_owned();
    existing.feishu.enabled = true;
    existing.feishu.app_id = Some(loongclaw_contracts::SecretRef::Inline(
        "cli_a1b2c3".to_owned(),
    ));
    existing.feishu.app_secret = Some(loongclaw_contracts::SecretRef::Inline(
        "feishu-secret".to_owned(),
    ));
    let existing_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::ExistingLoongClawConfig,
        "existing config at ~/.config/loongclaw/config.toml".to_owned(),
        existing,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("existing candidate");

    let mut codex = mvp::config::LoongClawConfig::default();
    codex.provider.kind = mvp::config::ProviderKind::Deepseek;
    let profile = codex.provider.kind.profile();
    codex.provider.base_url = profile.base_url.to_owned();
    codex.provider.chat_completions_path = profile.chat_completions_path.to_owned();
    codex.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "deepseek-secret".to_owned(),
    ));
    codex.provider.api_key_env = Some("DEEPSEEK_API_KEY".to_owned());
    let codex_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
        "Codex config at ~/.codex/config.toml".to_owned(),
        codex,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("codex candidate");

    let mut env = mvp::config::LoongClawConfig::default();
    env.telegram.enabled = true;
    env.telegram.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
        "123456:test-token".to_owned(),
    ));
    let env_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::Environment,
        "your current environment".to_owned(),
        env,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("env candidate");

    let composed = loongclaw_daemon::migration::planner::compose_recommended_import_candidate(&[
        existing_candidate,
        codex_candidate,
        env_candidate,
    ])
    .expect("recommended candidate");

    assert_eq!(
        composed.source_kind,
        loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan
    );
    assert_eq!(
        composed.config.provider.kind,
        mvp::config::ProviderKind::Openai,
        "ready current provider should win over conflicting alternative providers"
    );
    assert!(
        composed.config.telegram.enabled,
        "telegram should be supplemented from a secondary source"
    );
    assert!(
        composed.config.feishu.enabled,
        "existing ready channels should remain in the recommended plan"
    );
    assert!(
        composed
            .channel_candidates
            .iter()
            .any(|channel| channel.source == "your current environment"),
        "channel attribution should preserve the source used for supplementation: {composed:#?}"
    );
    let provider_domain = composed
        .domains
        .iter()
        .find(|domain| domain.kind == loongclaw_daemon::migration::types::SetupDomainKind::Provider)
        .expect("provider domain");
    assert_eq!(
        provider_domain.decision,
        Some(loongclaw_daemon::migration::types::PreviewDecision::KeepCurrent),
        "composed provider preview should explicitly say that the current provider is being kept"
    );
    let channels_domain = composed
        .domains
        .iter()
        .find(|domain| domain.kind == loongclaw_daemon::migration::types::SetupDomainKind::Channels)
        .expect("channels domain");
    assert_eq!(
        channels_domain.decision,
        Some(loongclaw_daemon::migration::types::PreviewDecision::Supplement),
        "composed channels preview should explicitly say that channels were supplemented from detected sources"
    );
}

#[test]
fn migration_compose_recommended_candidate_keeps_ready_plugin_backed_bridge_channels() {
    let install_root = unique_temp_dir("managed-bridge-compose-ready");
    let manifest = managed_bridge_manifest(
        "weixin",
        Some("channel"),
        compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
    );
    let mut existing = mvp::config::LoongClawConfig::default();
    let mut codex = mvp::config::LoongClawConfig::default();

    existing.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "openai-secret".to_owned(),
    ));
    existing.provider.model = "openai/gpt-5.1-codex".to_owned();

    std::fs::create_dir_all(&install_root).expect("create managed bridge install root");
    write_managed_bridge_manifest(install_root.as_path(), "weixin-ready-bridge", &manifest);

    codex.provider.kind = mvp::config::ProviderKind::Deepseek;
    codex.provider.base_url = codex.provider.kind.profile().base_url.to_owned();
    codex.provider.chat_completions_path = codex
        .provider
        .kind
        .profile()
        .chat_completions_path
        .to_owned();
    codex.weixin.enabled = true;
    codex.weixin.bridge_url = Some("https://bridge.example.test/weixin".to_owned());
    codex.weixin.bridge_access_token = Some(loongclaw_contracts::SecretRef::Inline(
        "weixin-token".to_owned(),
    ));
    codex.weixin.allowed_contact_ids = vec!["wxid_alice".to_owned()];
    codex.external_skills.install_root = Some(install_root.display().to_string());

    let existing_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::ExistingLoongClawConfig,
        "existing config at ~/.config/loongclaw/config.toml".to_owned(),
        existing,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("existing candidate");
    let codex_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
        "Codex config at ~/.codex/config.toml".to_owned(),
        codex,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("codex candidate");

    let composed = loongclaw_daemon::migration::planner::compose_recommended_import_candidate(&[
        existing_candidate,
        codex_candidate,
    ])
    .expect("recommended candidate");

    assert!(
        composed.config.weixin.enabled,
        "recommended import should keep plugin-backed bridge enablement when the detected source is selected"
    );
    assert_eq!(
        composed.config.weixin.bridge_url.as_deref(),
        Some("https://bridge.example.test/weixin")
    );
    assert!(
        composed
            .channel_candidates
            .iter()
            .any(|channel| channel.id == "weixin"
                && channel.status == loongclaw_daemon::migration::types::PreviewStatus::Ready),
        "recommended import should preserve the ready plugin-backed bridge candidate: {composed:#?}"
    );
    assert!(
        composed
            .channel_candidates
            .iter()
            .any(|channel| channel.id == "weixin"
                && channel.source == "Codex config at ~/.codex/config.toml"),
        "recommended import should preserve source attribution for plugin-backed bridge channels: {composed:#?}"
    );
}

#[test]
fn channel_registry_apply_selected_channels_copies_plugin_bridge_install_root() {
    let install_root = unique_temp_dir("managed-bridge-apply-install-root");
    let mut target = mvp::config::LoongClawConfig::default();
    let source: mvp::config::LoongClawConfig = serde_json::from_value(json!({
        "weixin": {
            "enabled": true,
            "bridge_url": "https://bridge.example.test/weixin",
            "bridge_access_token": "weixin-token",
            "allowed_contact_ids": ["wxid_alice"]
        },
        "external_skills": {
            "install_root": install_root.display().to_string()
        }
    }))
    .expect("deserialize source config");

    let changed = loongclaw_daemon::migration::channels::apply_selected_channels(
        &mut target,
        &source,
        &["weixin"],
    );

    assert!(
        changed,
        "plugin-backed channel selection should mutate the target config"
    );
    assert_eq!(
        target.external_skills.install_root.as_deref(),
        Some(install_root.to_string_lossy().as_ref()),
        "plugin-backed channel selection should carry the managed bridge install root alongside the channel config"
    );
}

#[test]
fn channel_registry_apply_selected_channels_skips_conflicting_plugin_bridge_install_root() {
    let target_install_root = unique_temp_dir("managed-bridge-target-install-root");
    let source_install_root = unique_temp_dir("managed-bridge-source-install-root");
    let mut target: mvp::config::LoongClawConfig = serde_json::from_value(json!({
        "external_skills": {
            "install_root": target_install_root.display().to_string()
        }
    }))
    .expect("deserialize target config");
    let source: mvp::config::LoongClawConfig = serde_json::from_value(json!({
        "weixin": {
            "enabled": true,
            "bridge_url": "https://bridge.example.test/weixin",
            "bridge_access_token": "weixin-token",
            "allowed_contact_ids": ["wxid_alice"]
        },
        "external_skills": {
            "install_root": source_install_root.display().to_string()
        }
    }))
    .expect("deserialize source config");

    let changed = loongclaw_daemon::migration::channels::apply_selected_channels(
        &mut target,
        &source,
        &["weixin"],
    );

    assert!(
        !changed,
        "plugin-backed channel selection should not silently merge a channel that needs a different managed bridge install root"
    );
    assert!(
        !target.weixin.enabled,
        "conflicting managed bridge install roots should leave the target channel unchanged"
    );
    assert_eq!(
        target.external_skills.install_root.as_deref(),
        Some(target_install_root.to_string_lossy().as_ref()),
        "conflicting managed bridge install roots should preserve the existing managed bridge root"
    );
}

#[test]
fn migration_compose_recommended_candidate_marks_plugin_bridge_install_root_conflicts_for_review() {
    let current_install_root = unique_temp_dir("managed-bridge-current-install-root");
    let detected_install_root = unique_temp_dir("managed-bridge-detected-install-root");
    let current_manifest = managed_bridge_manifest(
        "qqbot",
        Some("channel"),
        compatible_managed_bridge_metadata(
            "qq_official_bot_gateway_or_plugin_bridge",
            "qqbot_reply_loop",
        ),
    );
    let detected_manifest = managed_bridge_manifest(
        "weixin",
        Some("channel"),
        compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
    );
    let mut codex = mvp::config::LoongClawConfig::default();
    let existing: mvp::config::LoongClawConfig = serde_json::from_value(json!({
        "provider": {
            "api_key": "openai-secret",
            "model": "openai/gpt-5.1-codex"
        },
        "qqbot": {
            "enabled": true,
            "app_id": "10001",
            "client_secret": "qqbot-secret",
            "allowed_peer_ids": ["openid-alice"]
        },
        "external_skills": {
            "install_root": current_install_root.display().to_string()
        }
    }))
    .expect("deserialize existing config");

    std::fs::create_dir_all(&current_install_root).expect("create current install root");
    std::fs::create_dir_all(&detected_install_root).expect("create detected install root");
    write_managed_bridge_manifest(
        current_install_root.as_path(),
        "qqbot-current-bridge",
        &current_manifest,
    );
    write_managed_bridge_manifest(
        detected_install_root.as_path(),
        "weixin-detected-bridge",
        &detected_manifest,
    );

    codex.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "openai-secret".to_owned(),
    ));
    codex.provider.model = "openai/gpt-5.1-codex".to_owned();
    codex.weixin.enabled = true;
    codex.weixin.bridge_url = Some("https://bridge.example.test/weixin".to_owned());
    codex.weixin.bridge_access_token = Some(loongclaw_contracts::SecretRef::Inline(
        "weixin-token".to_owned(),
    ));
    codex.weixin.allowed_contact_ids = vec!["wxid_alice".to_owned()];
    codex.external_skills.install_root = Some(detected_install_root.display().to_string());

    let existing_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::ExistingLoongClawConfig,
        "existing config at ~/.config/loongclaw/config.toml".to_owned(),
        existing,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("existing candidate");
    let codex_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
        "Codex config at ~/.codex/config.toml".to_owned(),
        codex,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("codex candidate");

    let composed = loongclaw_daemon::migration::planner::compose_recommended_import_candidate(&[
        existing_candidate,
        codex_candidate,
    ])
    .expect("recommended candidate");
    let weixin_candidate = composed
        .channel_candidates
        .iter()
        .find(|channel| channel.id == "weixin")
        .expect("weixin candidate");

    assert!(
        composed.config.qqbot.enabled,
        "recommended import should preserve the currently configured plugin-backed channel"
    );
    assert!(
        !composed.config.weixin.enabled,
        "recommended import should not silently merge a plugin-backed channel that depends on a conflicting managed bridge install root"
    );
    assert_eq!(
        composed.config.external_skills.install_root.as_deref(),
        Some(current_install_root.to_string_lossy().as_ref()),
        "recommended import should preserve the active managed bridge install root when another source conflicts"
    );
    assert_eq!(
        weixin_candidate.status,
        loongclaw_daemon::migration::types::PreviewStatus::NeedsReview,
        "conflicting managed bridge install roots should downgrade the affected channel to review"
    );
    assert!(
        weixin_candidate
            .summary
            .contains("managed bridge install_root conflict"),
        "recommended import should explain why the plugin-backed channel was not merged automatically: {weixin_candidate:#?}"
    );
}

#[test]
fn migration_compose_recommended_candidate_preserves_selected_channel_conflict_over_later_supplement()
 {
    let current_install_root = unique_temp_dir("managed-bridge-selected-conflict-current");
    let conflicting_install_root = unique_temp_dir("managed-bridge-selected-conflict-detected");
    let current_manifest = managed_bridge_manifest(
        "qqbot",
        Some("channel"),
        compatible_managed_bridge_metadata(
            "qq_official_bot_gateway_or_plugin_bridge",
            "qqbot_reply_loop",
        ),
    );
    let mut current: mvp::config::LoongClawConfig = serde_json::from_value(json!({
        "qqbot": {
            "enabled": true,
            "app_id": "10001",
            "client_secret": "qqbot-secret",
            "allowed_peer_ids": ["openid-alice"]
        },
        "external_skills": {
            "install_root": current_install_root.display().to_string()
        }
    }))
    .expect("deserialize current config");
    let mut conflicting = mvp::config::LoongClawConfig::default();
    let mut supplemental = mvp::config::LoongClawConfig::default();

    std::fs::create_dir_all(&current_install_root).expect("create current install root");
    std::fs::create_dir_all(&conflicting_install_root).expect("create conflicting install root");
    write_managed_bridge_manifest(
        current_install_root.as_path(),
        "qqbot-current-bridge",
        &current_manifest,
    );

    current.provider.model = "openai/gpt-5.1-codex".to_owned();

    conflicting.provider.model = "openai/gpt-5.1-codex".to_owned();
    conflicting.weixin.enabled = true;
    conflicting.weixin.bridge_url = Some("https://bridge.example.test/weixin".to_owned());
    conflicting.weixin.bridge_access_token = Some(loongclaw_contracts::SecretRef::Inline(
        "weixin-token".to_owned(),
    ));
    conflicting.weixin.allowed_contact_ids = vec!["wxid_alice".to_owned()];
    conflicting.external_skills.install_root = Some(conflicting_install_root.display().to_string());

    supplemental.provider.model = "openai/gpt-5.1-codex".to_owned();
    supplemental.weixin.enabled = true;
    supplemental.weixin.bridge_url = Some("https://bridge.example.test/weixin".to_owned());
    supplemental.weixin.bridge_access_token = Some(loongclaw_contracts::SecretRef::Inline(
        "weixin-token".to_owned(),
    ));
    supplemental.weixin.allowed_contact_ids = vec!["wxid_alice".to_owned()];

    let current_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::ExistingLoongClawConfig,
        "existing config at ~/.config/loongclaw/config.toml".to_owned(),
        current,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("current candidate");
    let conflicting_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
        "Codex config at ~/.codex/config.toml".to_owned(),
        conflicting,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("conflicting candidate");
    let supplemental_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::Environment,
        "your current environment".to_owned(),
        supplemental,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("supplemental candidate");

    let composed = loongclaw_daemon::migration::planner::compose_recommended_import_candidate(&[
        current_candidate,
        conflicting_candidate,
        supplemental_candidate,
    ])
    .expect("recommended candidate");
    let weixin_candidate = composed
        .channel_candidates
        .iter()
        .find(|channel| channel.id == "weixin")
        .expect("weixin candidate");

    assert_eq!(
        weixin_candidate.status,
        loongclaw_daemon::migration::types::PreviewStatus::NeedsReview,
        "a conflicting selected channel should stay in review even if a later source could supplement the config"
    );
    assert!(
        weixin_candidate
            .summary
            .contains("managed bridge install_root conflict"),
        "selected channel conflicts must remain visible in the composed preview: {weixin_candidate:#?}"
    );
    assert!(
        !composed.config.weixin.enabled,
        "the composed config should not silently enable the conflicting plugin-backed channel"
    );
}

#[test]
fn migration_compose_recommended_candidate_upgrades_incomplete_provider_from_compatible_source() {
    let mut existing = mvp::config::LoongClawConfig::default();
    existing.provider.kind = mvp::config::ProviderKind::KimiCoding;
    let existing_profile = existing.provider.kind.profile();
    existing.provider.base_url = existing_profile.base_url.to_owned();
    existing.provider.chat_completions_path = existing_profile.chat_completions_path.to_owned();
    existing.provider.model = "kimi-for-coding".to_owned();
    existing.provider.api_key_env = Some(format!(
        "LOONGCLAW_TEST_UNSET_KIMI_CODING_KEY_{}",
        std::process::id()
    ));
    let existing_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::ExistingLoongClawConfig,
        "existing config at ~/.config/loongclaw/config.toml".to_owned(),
        existing,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("existing candidate");

    let mut codex = mvp::config::LoongClawConfig::default();
    codex.provider.kind = mvp::config::ProviderKind::KimiCoding;
    let codex_profile = codex.provider.kind.profile();
    codex.provider.base_url = codex_profile.base_url.to_owned();
    codex.provider.chat_completions_path = codex_profile.chat_completions_path.to_owned();
    codex.provider.model = "kimi-for-coding".to_owned();
    codex.provider.api_key_env = Some("KIMI_CODING_API_KEY".to_owned());
    codex.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "kimi-coding-secret".to_owned(),
    ));
    let codex_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
        "Codex config at ~/.codex/config.toml".to_owned(),
        codex,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("codex candidate");

    let composed = loongclaw_daemon::migration::planner::compose_recommended_import_candidate(&[
        existing_candidate,
        codex_candidate,
    ])
    .expect("recommended candidate");

    let provider_domain = composed
        .domains
        .iter()
        .find(|domain| domain.kind == loongclaw_daemon::migration::types::SetupDomainKind::Provider)
        .expect("provider domain");
    assert_eq!(
        provider_domain.source, "Codex config at ~/.codex/config.toml",
        "compatible ready provider should upgrade an incomplete current provider"
    );
    assert_eq!(
        composed.config.provider.api_key,
        Some(loongclaw_contracts::SecretRef::Inline(
            "kimi-coding-secret".to_owned(),
        ))
    );
    assert_eq!(composed.config.provider.api_key_env, None);
}

#[test]
fn migration_compose_recommended_candidate_supplements_channel_fields_across_sources() {
    let mut existing = mvp::config::LoongClawConfig::default();
    existing.telegram.enabled = true;
    existing.telegram.allowed_chat_ids = vec![42];
    existing.telegram.polling_timeout_s = 90;
    let existing_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::ExistingLoongClawConfig,
        "existing config at ~/.config/loongclaw/config.toml".to_owned(),
        existing,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("existing candidate");

    let mut env = mvp::config::LoongClawConfig::default();
    env.telegram.enabled = true;
    env.telegram.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
        "123456:test-token".to_owned(),
    ));
    let env_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::Environment,
        "your current environment".to_owned(),
        env,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("env candidate");

    let composed = loongclaw_daemon::migration::planner::compose_recommended_import_candidate(&[
        existing_candidate,
        env_candidate,
    ])
    .expect("recommended candidate");

    assert_eq!(
        composed.config.telegram.allowed_chat_ids,
        vec![42],
        "recommended plan should preserve existing telegram allowlist while filling credentials"
    );
    assert_eq!(
        composed.config.telegram.polling_timeout_s, 90,
        "recommended plan should preserve non-default channel settings from the existing config"
    );
    assert_eq!(
        composed.config.telegram.bot_token(),
        Some("123456:test-token".to_owned()),
        "recommended plan should still fill missing telegram credentials from another source"
    );
}

#[test]
fn migration_compose_recommended_candidate_preserves_current_custom_provider_endpoint() {
    let _env = MigrationEnvironmentGuard::set(&[
        ("TELEGRAM_BOT_TOKEN", None),
        ("OPENROUTER_API_KEY", None),
    ]);

    let mut existing = mvp::config::LoongClawConfig::default();
    existing.provider.kind = mvp::config::ProviderKind::Openrouter;
    existing.provider.model = "openrouter/openai/gpt-5.1".to_owned();
    existing.provider.base_url = "https://proxy.example.com/v1".to_owned();
    existing.provider.chat_completions_path = "/chat/completions".to_owned();
    existing.provider.api_key_env = Some(format!(
        "LOONGCLAW_TEST_UNSET_OPENROUTER_KEY_{}",
        std::process::id()
    ));
    let existing_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::ExistingLoongClawConfig,
        "existing config at ~/.config/loongclaw/config.toml".to_owned(),
        existing,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("existing candidate");

    let mut codex = mvp::config::LoongClawConfig::default();
    codex.provider.kind = mvp::config::ProviderKind::Openrouter;
    let profile = codex.provider.kind.profile();
    codex.provider.base_url = profile.base_url.to_owned();
    codex.provider.chat_completions_path = profile.chat_completions_path.to_owned();
    codex.provider.model = "openrouter/openai/gpt-5.1".to_owned();
    codex.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "openrouter-secret".to_owned(),
    ));
    codex.provider.api_key_env = Some("OPENROUTER_API_KEY".to_owned());
    let codex_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
        "Codex config at ~/.codex/config.toml".to_owned(),
        codex,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("codex candidate");

    let composed = loongclaw_daemon::migration::planner::compose_recommended_import_candidate(&[
        existing_candidate,
        codex_candidate,
    ])
    .expect("recommended candidate");

    assert_eq!(
        composed.config.provider.base_url, "https://proxy.example.com/v1",
        "recommended plan should preserve the current custom provider base URL when only credentials are supplemented"
    );
    assert_eq!(
        composed.config.provider.chat_completions_path, "/chat/completions",
        "recommended plan should preserve the current compatible endpoint path when only credentials are supplemented"
    );
    assert_eq!(
        composed.config.provider.api_key,
        Some(loongclaw_contracts::SecretRef::Inline(
            "openrouter-secret".to_owned(),
        )),
        "recommended plan should still upgrade missing credentials from the compatible source into the canonical api_key field"
    );
    assert_eq!(composed.config.provider.api_key_env, None);
}

#[test]
fn migration_compose_recommended_candidate_supplements_provider_wire_api() {
    let mut existing = mvp::config::LoongClawConfig::default();
    existing.provider.kind = mvp::config::ProviderKind::Openrouter;
    existing.provider.model = "openrouter/openai/gpt-5.1".to_owned();
    existing.provider.api_key_env = Some(format!(
        "LOONGCLAW_TEST_UNSET_OPENROUTER_WIRE_API_{}",
        std::process::id()
    ));
    let existing_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::ExistingLoongClawConfig,
        "existing config at ~/.config/loongclaw/config.toml".to_owned(),
        existing,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("existing candidate");

    let mut env = mvp::config::LoongClawConfig::default();
    env.provider.kind = mvp::config::ProviderKind::Openrouter;
    let profile = env.provider.kind.profile();
    env.provider.base_url = profile.base_url.to_owned();
    env.provider.chat_completions_path = profile.chat_completions_path.to_owned();
    env.provider.model = "openrouter/openai/gpt-5.1".to_owned();
    env.provider.wire_api = mvp::config::ProviderWireApi::Responses;
    env.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "openrouter-secret".to_owned(),
    ));
    env.provider.api_key_env = Some("OPENROUTER_API_KEY".to_owned());
    let env_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::Environment,
        "your current environment".to_owned(),
        env,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("env candidate");

    let composed = loongclaw_daemon::migration::planner::compose_recommended_import_candidate(&[
        existing_candidate,
        env_candidate,
    ])
    .expect("recommended candidate");

    assert_eq!(
        composed.config.provider.wire_api,
        mvp::config::ProviderWireApi::Responses,
        "recommended plan should supplement the imported provider wire_api so compatible Responses setups remain runnable after merge"
    );
    assert_eq!(
        composed.config.provider.transport_readiness().summary,
        "responses compatibility mode with chat fallback",
        "recommended plan should preserve the supplemented transport explanation for the merged provider"
    );
}

#[test]
fn migration_compose_recommended_candidate_supplements_provider_transport_tuning() {
    let mut existing = mvp::config::LoongClawConfig::default();
    existing.provider.kind = mvp::config::ProviderKind::Openrouter;
    existing.provider.model = "openrouter/openai/gpt-5.1".to_owned();
    existing.provider.api_key_env = Some(format!(
        "LOONGCLAW_TEST_UNSET_OPENROUTER_KEY_{}",
        std::process::id()
    ));
    let existing_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::ExistingLoongClawConfig,
        "existing config at ~/.config/loongclaw/config.toml".to_owned(),
        existing,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("existing candidate");

    let mut env = mvp::config::LoongClawConfig::default();
    env.provider.kind = mvp::config::ProviderKind::Openrouter;
    env.provider.model = "openrouter/openai/gpt-5.1".to_owned();
    env.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "openrouter-secret".to_owned(),
    ));
    env.provider.api_key_env = Some("OPENROUTER_API_KEY".to_owned());
    env.provider.temperature = 0.55;
    env.provider.request_timeout_ms = 45_000;
    env.provider.retry_max_attempts = 5;
    env.provider.retry_initial_backoff_ms = 450;
    env.provider.retry_max_backoff_ms = 4_500;
    let env_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::Environment,
        "your current environment".to_owned(),
        env,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("env candidate");

    let composed = loongclaw_daemon::migration::planner::compose_recommended_import_candidate(&[
        existing_candidate,
        env_candidate,
    ])
    .expect("recommended candidate");

    assert_eq!(composed.config.provider.temperature, 0.55);
    assert_eq!(composed.config.provider.request_timeout_ms, 45_000);
    assert_eq!(composed.config.provider.retry_max_attempts, 5);
    assert_eq!(composed.config.provider.retry_initial_backoff_ms, 450);
    assert_eq!(composed.config.provider.retry_max_backoff_ms, 4_500);
}

#[test]
fn migration_compose_recommended_candidate_avoids_provider_auto_pick_on_cross_source_conflict() {
    let _env = MigrationEnvironmentGuard::set(&[("TELEGRAM_BOT_TOKEN", None)]);

    let mut codex = mvp::config::LoongClawConfig::default();
    codex.provider.model = "openai/gpt-5.1-codex".to_owned();
    codex.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "openai-secret".to_owned(),
    ));
    let codex_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
        "Codex config at ~/.codex/config.toml".to_owned(),
        codex,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("codex candidate");

    let mut env = mvp::config::LoongClawConfig::default();
    env.provider.kind = mvp::config::ProviderKind::Deepseek;
    let profile = env.provider.kind.profile();
    env.provider.base_url = profile.base_url.to_owned();
    env.provider.chat_completions_path = profile.chat_completions_path.to_owned();
    env.provider.model = "deepseek-chat".to_owned();
    env.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "deepseek-secret".to_owned(),
    ));
    let env_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::Environment,
        "your current environment".to_owned(),
        env,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("env candidate");

    let composed = loongclaw_daemon::migration::planner::compose_recommended_import_candidate(&[
        codex_candidate,
        env_candidate,
    ]);
    assert!(
        composed.is_none(),
        "recommended plan should be omitted when the only available import signal is a cross-source provider conflict"
    );
}

#[test]
fn migration_compose_recommended_candidate_ignores_home_drift_for_default_memory_path() {
    let home_a = unique_temp_dir("home-drift-a");
    let home_b = unique_temp_dir("home-drift-b");
    std::fs::create_dir_all(&home_a).expect("create first home");
    std::fs::create_dir_all(&home_b).expect("create second home");

    let (codex, env) = {
        let _guard = MigrationEnvironmentGuard::set(&[
            ("HOME", Some(home_a.to_string_lossy().as_ref())),
            ("OPENAI_API_KEY", None),
            ("OPENAI_CODEX_OAUTH_TOKEN", None),
            ("OPENAI_OAUTH_ACCESS_TOKEN", None),
            ("TELEGRAM_BOT_TOKEN", None),
        ]);

        let mut codex = mvp::config::LoongClawConfig::default();
        codex.provider.model = "openai/gpt-5.1-codex".to_owned();
        codex.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
            "openai-secret".to_owned(),
        ));

        let mut env = mvp::config::LoongClawConfig::default();
        env.provider.kind = mvp::config::ProviderKind::Deepseek;
        let profile = env.provider.kind.profile();
        env.provider.base_url = profile.base_url.to_owned();
        env.provider.chat_completions_path = profile.chat_completions_path.to_owned();
        env.provider.model = "deepseek-chat".to_owned();
        env.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
            "deepseek-secret".to_owned(),
        ));
        (codex, env)
    };

    let _guard = MigrationEnvironmentGuard::set(&[
        ("HOME", Some(home_b.to_string_lossy().as_ref())),
        ("OPENAI_API_KEY", None),
        ("OPENAI_CODEX_OAUTH_TOKEN", None),
        ("OPENAI_OAUTH_ACCESS_TOKEN", None),
        ("TELEGRAM_BOT_TOKEN", None),
    ]);

    let codex_candidate = crate::migration::discovery::build_import_candidate(
        crate::migration::types::ImportSourceKind::CodexConfig,
        "Codex config at ~/.codex/config.toml".to_owned(),
        codex,
        crate::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("codex candidate");
    let env_candidate = crate::migration::discovery::build_import_candidate(
        crate::migration::types::ImportSourceKind::Environment,
        "your current environment".to_owned(),
        env,
        crate::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("env candidate");

    let composed = crate::migration::planner::compose_recommended_import_candidate(&[
        codex_candidate,
        env_candidate,
    ]);
    assert!(
        composed.is_none(),
        "default memory sqlite paths should not create a fake recommended plan after HOME changes"
    );
}

#[test]
fn migration_compose_recommended_candidate_keeps_non_provider_domains_when_cross_source_provider_conflict_requires_manual_choice()
 {
    let mut codex = mvp::config::LoongClawConfig::default();
    codex.provider.model = "openai/gpt-5.1-codex".to_owned();
    codex.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "openai-secret".to_owned(),
    ));
    codex.telegram.enabled = true;
    codex.telegram.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
        "123456:test-token".to_owned(),
    ));
    let codex_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
        "Codex config at ~/.codex/config.toml".to_owned(),
        codex,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("codex candidate");

    let mut env = mvp::config::LoongClawConfig::default();
    env.provider.kind = mvp::config::ProviderKind::Deepseek;
    let profile = env.provider.kind.profile();
    env.provider.base_url = profile.base_url.to_owned();
    env.provider.chat_completions_path = profile.chat_completions_path.to_owned();
    env.provider.model = "deepseek-chat".to_owned();
    env.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "deepseek-secret".to_owned(),
    ));
    let env_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::Environment,
        "your current environment".to_owned(),
        env,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("env candidate");

    let composed = loongclaw_daemon::migration::planner::compose_recommended_import_candidate(&[
        codex_candidate,
        env_candidate,
    ])
    .expect("recommended candidate should still exist for non-provider domains");

    assert!(
        composed
            .domains
            .iter()
            .all(|domain| domain.kind
                != loongclaw_daemon::migration::types::SetupDomainKind::Provider),
        "provider should stay unresolved when multiple ready providers conflict without a safe anchor: {composed:#?}"
    );
    assert!(
        composed
            .domains
            .iter()
            .any(|domain| domain.kind
                == loongclaw_daemon::migration::types::SetupDomainKind::Channels),
        "non-provider domains should still be preserved in the recommended plan: {composed:#?}"
    );
    assert!(
        composed.config.telegram.enabled,
        "recommended plan should still carry reusable channel configuration while provider choice is deferred"
    );
}

#[test]
fn migration_compose_recommended_candidate_supplements_cli_memory_and_tools_across_sources() {
    let mut existing = mvp::config::LoongClawConfig::default();
    existing.cli.system_prompt = "Use repo guidance".to_owned();
    existing.tools.file_root = Some("~/workspace/current".to_owned());
    existing.memory.sqlite_path = "~/.loongclaw/current.sqlite".to_owned();
    let existing_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::ExistingLoongClawConfig,
        "existing config at ~/.config/loongclaw/config.toml".to_owned(),
        existing,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("existing candidate");

    let mut codex = mvp::config::LoongClawConfig::default();
    codex.cli.exit_commands.push("/bye".to_owned());
    codex.tools.shell_allow.push("git".to_owned());
    codex.memory.sliding_window = 24;
    let codex_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
        "Codex config at ~/.codex/config.toml".to_owned(),
        codex,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("codex candidate");

    let composed = loongclaw_daemon::migration::planner::compose_recommended_import_candidate(&[
        existing_candidate,
        codex_candidate,
    ])
    .expect("recommended candidate");

    assert_eq!(
        composed.config.cli.system_prompt, "Use repo guidance",
        "recommended plan should preserve existing custom CLI prompt"
    );
    assert!(
        composed
            .config
            .cli
            .exit_commands
            .iter()
            .any(|value| value == "/bye"),
        "recommended plan should supplement additional CLI exit commands"
    );
    assert_eq!(
        composed.config.tools.file_root.as_deref(),
        Some("~/workspace/current"),
        "recommended plan should preserve the existing tools root"
    );
    assert!(
        composed
            .config
            .tools
            .shell_allow
            .iter()
            .any(|value| value == "git"),
        "recommended plan should supplement additional tool permissions"
    );
    assert_eq!(
        composed.config.memory.sqlite_path, "~/.loongclaw/current.sqlite",
        "recommended plan should preserve the existing memory database path"
    );
    assert_eq!(
        composed.config.memory.sliding_window, 24,
        "recommended plan should supplement a non-default memory window"
    );
}

#[test]
fn migration_compose_recommended_candidate_keeps_incomplete_current_provider_when_alternative_conflicts()
 {
    let mut existing = mvp::config::LoongClawConfig::default();
    existing.provider.kind = mvp::config::ProviderKind::KimiCoding;
    let existing_profile = existing.provider.kind.profile();
    existing.provider.base_url = existing_profile.base_url.to_owned();
    existing.provider.chat_completions_path = existing_profile.chat_completions_path.to_owned();
    existing.provider.model = "kimi-for-coding".to_owned();
    existing.provider.api_key_env = Some(format!(
        "LOONGCLAW_TEST_UNSET_KIMI_CODING_CONFLICT_{}",
        std::process::id()
    ));
    let existing_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::ExistingLoongClawConfig,
        "existing config at ~/.config/loongclaw/config.toml".to_owned(),
        existing,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("existing candidate");

    let mut codex = mvp::config::LoongClawConfig::default();
    codex.provider.kind = mvp::config::ProviderKind::Deepseek;
    let codex_profile = codex.provider.kind.profile();
    codex.provider.base_url = codex_profile.base_url.to_owned();
    codex.provider.chat_completions_path = codex_profile.chat_completions_path.to_owned();
    codex.provider.model = "deepseek-chat".to_owned();
    codex.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "deepseek-secret".to_owned(),
    ));
    codex.provider.api_key_env = Some("DEEPSEEK_API_KEY".to_owned());
    let codex_candidate = loongclaw_daemon::migration::discovery::build_import_candidate(
        loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
        "Codex config at ~/.codex/config.toml".to_owned(),
        codex,
        loongclaw_daemon::migration::discovery::resolve_channel_import_readiness_from_config,
        Vec::new(),
    )
    .expect("codex candidate");

    let composed = loongclaw_daemon::migration::planner::compose_recommended_import_candidate(&[
        existing_candidate,
        codex_candidate,
    ])
    .expect("recommended candidate");

    let provider_domain = composed
        .domains
        .iter()
        .find(|domain| domain.kind == loongclaw_daemon::migration::types::SetupDomainKind::Provider)
        .expect("provider domain");
    assert_eq!(
        composed.config.provider.kind,
        mvp::config::ProviderKind::KimiCoding,
        "recommended plan should not auto-switch to a conflicting provider"
    );
    assert_eq!(
        provider_domain.source, "existing config at ~/.config/loongclaw/config.toml",
        "current provider should remain the source of truth when alternatives conflict"
    );
    assert_eq!(
        provider_domain.status,
        loongclaw_daemon::migration::types::PreviewStatus::NeedsReview
    );
    assert!(
        provider_domain
            .summary
            .contains("also detected from Codex config at ~/.codex/config.toml"),
        "preview should still surface the conflicting ready alternative: {provider_domain:#?}"
    );
}
