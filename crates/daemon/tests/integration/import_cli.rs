#![allow(unsafe_code)]
#![allow(
    clippy::disallowed_methods,
    clippy::multiple_unsafe_ops_per_block,
    clippy::undocumented_unsafe_blocks
)]

use super::*;
use serde_json::json;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::MutexGuard;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static IMPORT_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
fn unique_temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    let counter = IMPORT_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "loongclaw-import-{label}-{}-{nanos}-{counter}",
        std::process::id(),
    ))
}

struct ImportEnvironmentGuard {
    _lock: MutexGuard<'static, ()>,
    saved: Vec<(String, Option<OsString>)>,
}

impl ImportEnvironmentGuard {
    fn set(pairs: &[(&str, Option<&str>)]) -> Self {
        let lock = super::lock_daemon_test_environment();
        let mut saved = Vec::new();
        for (key, value) in pairs {
            saved.push(((*key).to_owned(), std::env::var_os(key)));
            match value {
                Some(value) => unsafe {
                    std::env::set_var(key, value);
                },
                None => unsafe {
                    std::env::remove_var(key);
                },
            }
        }
        Self { _lock: lock, saved }
    }
}

impl Drop for ImportEnvironmentGuard {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..).rev() {
            match value {
                Some(value) => unsafe {
                    std::env::set_var(&key, value);
                },
                None => unsafe {
                    std::env::remove_var(&key);
                },
            }
        }
    }
}

fn provider_choice_preview_env_guard() -> ImportEnvironmentGuard {
    ImportEnvironmentGuard::set(&[
        ("OPENAI_API_KEY", Some("test-openai-key")),
        ("DEEPSEEK_API_KEY", Some("test-deepseek-key")),
    ])
}

fn sample_import_candidate() -> loongclaw_daemon::migration::types::ImportCandidate {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Openrouter;
    config.provider.model = "openrouter/openai/gpt-5.1".to_owned();
    config.provider.api_key_env = Some("OPENROUTER_API_KEY".to_owned());
    config.cli.system_prompt = "Imported CLI prompt".to_owned();
    config.telegram.enabled = true;
    config.telegram.bot_token_env = Some("TELEGRAM_BOT_TOKEN".to_owned());
    config.tools.file_root = Some("~/workspace/demo".to_owned());
    config.memory.sqlite_path = "~/.loongclaw/demo.sqlite".to_owned();

    loongclaw_daemon::migration::types::ImportCandidate {
        source_kind: loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
        source: "Codex config at ~/.codex/config.toml".to_owned(),
        config,
        surfaces: Vec::new(),
        domains: vec![
            loongclaw_daemon::migration::types::DomainPreview {
                kind: loongclaw_daemon::migration::types::SetupDomainKind::Provider,
                status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
                decision: Some(loongclaw_daemon::migration::types::PreviewDecision::UseDetected),
                source: "Codex config at ~/.codex/config.toml".to_owned(),
                summary: "openrouter · openrouter/openai/gpt-5.1".to_owned(),
            },
            loongclaw_daemon::migration::types::DomainPreview {
                kind: loongclaw_daemon::migration::types::SetupDomainKind::Channels,
                status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
                decision: Some(loongclaw_daemon::migration::types::PreviewDecision::UseDetected),
                source: "Codex config at ~/.codex/config.toml".to_owned(),
                summary: "telegram Ready".to_owned(),
            },
            loongclaw_daemon::migration::types::DomainPreview {
                kind: loongclaw_daemon::migration::types::SetupDomainKind::Cli,
                status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
                decision: Some(loongclaw_daemon::migration::types::PreviewDecision::UseDetected),
                source: "Codex config at ~/.codex/config.toml".to_owned(),
                summary: "custom CLI behavior detected".to_owned(),
            },
            loongclaw_daemon::migration::types::DomainPreview {
                kind: loongclaw_daemon::migration::types::SetupDomainKind::Tools,
                status: loongclaw_daemon::migration::types::PreviewStatus::NeedsReview,
                decision: Some(loongclaw_daemon::migration::types::PreviewDecision::UseDetected),
                source: "Codex config at ~/.codex/config.toml".to_owned(),
                summary: "workspace root ~/workspace/demo".to_owned(),
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
    }
}

fn import_candidate_with_provider(
    source_kind: loongclaw_daemon::migration::types::ImportSourceKind,
    source: &str,
    kind: mvp::config::ProviderKind,
    model: &str,
    credential_env: &str,
) -> loongclaw_daemon::migration::types::ImportCandidate {
    let mut candidate = sample_import_candidate();
    let profile = kind.profile();
    candidate.source_kind = source_kind;
    candidate.source = source.to_owned();
    candidate.config.provider.kind = kind;
    candidate.config.provider.base_url = profile.base_url.to_owned();
    candidate.config.provider.chat_completions_path = profile.chat_completions_path.to_owned();
    candidate.config.provider.model = model.to_owned();
    candidate.config.provider.api_key_env = Some(credential_env.to_owned());
    candidate.domains.retain(|domain| {
        domain.kind != loongclaw_daemon::migration::types::SetupDomainKind::Provider
    });
    candidate.domains.insert(
        0,
        loongclaw_daemon::migration::types::DomainPreview {
            kind: loongclaw_daemon::migration::types::SetupDomainKind::Provider,
            status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
            decision: Some(loongclaw_daemon::migration::types::PreviewDecision::UseDetected),
            source: source.to_owned(),
            summary: loongclaw_daemon::provider_presentation::provider_identity_summary(
                &candidate.config.provider,
            ),
        },
    );
    candidate
}

#[test]
fn import_cli_parse_source_selector_accepts_known_values() {
    assert_eq!(
        loongclaw_daemon::import_cli::parse_import_source_selector("recommended"),
        Some(loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan)
    );
    assert_eq!(
        loongclaw_daemon::import_cli::parse_import_source_selector("composed"),
        Some(loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan)
    );
    assert_eq!(
        loongclaw_daemon::import_cli::parse_import_source_selector("codex"),
        Some(loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig)
    );
    assert_eq!(
        loongclaw_daemon::import_cli::parse_import_source_selector("existing"),
        Some(loongclaw_daemon::migration::types::ImportSourceKind::ExistingLoongClawConfig)
    );
    assert_eq!(
        loongclaw_daemon::import_cli::parse_import_source_selector("env"),
        Some(loongclaw_daemon::migration::types::ImportSourceKind::Environment)
    );
    assert_eq!(
        loongclaw_daemon::import_cli::parse_import_source_selector("unknown"),
        None
    );
}

#[test]
fn import_cli_parse_source_selector_rejects_non_importable_values() {
    assert_eq!(
        loongclaw_daemon::import_cli::parse_import_source_selector("current"),
        None
    );
    assert_eq!(
        loongclaw_daemon::import_cli::parse_import_source_selector("path"),
        None
    );
}

#[test]
fn import_cli_parse_domain_selector_accepts_known_values() {
    assert_eq!(
        loongclaw_daemon::import_cli::parse_import_domain_selector("provider"),
        Some(loongclaw_daemon::migration::types::SetupDomainKind::Provider)
    );
    assert_eq!(
        loongclaw_daemon::import_cli::parse_import_domain_selector("workspace_guidance"),
        Some(loongclaw_daemon::migration::types::SetupDomainKind::WorkspaceGuidance)
    );
    assert_eq!(
        loongclaw_daemon::import_cli::parse_import_domain_selector("workspace-guidance"),
        Some(loongclaw_daemon::migration::types::SetupDomainKind::WorkspaceGuidance)
    );
    assert_eq!(
        loongclaw_daemon::import_cli::parse_import_domain_selector("unknown"),
        None
    );
}

#[test]
fn import_cli_supported_selector_lists_stay_canonical() {
    assert_eq!(
        loongclaw_daemon::migration::types::ImportSourceKind::supported_import_cli_selector_list(),
        "recommended, existing, codex, env"
    );
    assert_eq!(
        loongclaw_daemon::migration::types::SetupDomainKind::supported_selector_list(),
        "provider, channels, cli, memory, tools, workspace_guidance"
    );
}

#[test]
fn import_cli_resolve_selected_domains_respects_include_and_exclude() {
    let candidate = sample_import_candidate();

    let selected = loongclaw_daemon::import_cli::resolve_selected_domains(
        &candidate,
        &[
            loongclaw_daemon::migration::types::SetupDomainKind::Provider,
            loongclaw_daemon::migration::types::SetupDomainKind::Channels,
            loongclaw_daemon::migration::types::SetupDomainKind::Tools,
        ],
        &[loongclaw_daemon::migration::types::SetupDomainKind::Tools],
    );

    assert_eq!(
        selected,
        vec![
            loongclaw_daemon::migration::types::SetupDomainKind::Provider,
            loongclaw_daemon::migration::types::SetupDomainKind::Channels,
        ]
    );
}

#[test]
fn import_cli_surface_matches_future_channel_surfaces_without_hardcoded_names() {
    let selected_channels = std::collections::BTreeSet::from([
        loongclaw_daemon::migration::types::SetupDomainKind::Channels,
    ]);
    let selected_cli = std::collections::BTreeSet::from([
        loongclaw_daemon::migration::types::SetupDomainKind::Cli,
    ]);
    let future_channel_surface = loongclaw_daemon::migration::types::ImportSurface {
        name: "slack channel",
        domain: loongclaw_daemon::migration::types::SetupDomainKind::Channels,
        level: loongclaw_daemon::migration::types::ImportSurfaceLevel::Ready,
        detail: "token resolved".to_owned(),
    };
    let cli_surface = loongclaw_daemon::migration::types::ImportSurface {
        name: "cli channel",
        domain: loongclaw_daemon::migration::types::SetupDomainKind::Cli,
        level: loongclaw_daemon::migration::types::ImportSurfaceLevel::Ready,
        detail: "custom CLI behavior detected".to_owned(),
    };

    assert!(
        loongclaw_daemon::import_cli::surface_matches_selected_domains(
            &future_channel_surface,
            &selected_channels
        ),
        "channel domain matching should use typed metadata for future channel surfaces"
    );
    assert!(
        loongclaw_daemon::import_cli::surface_matches_selected_domains(&cli_surface, &selected_cli),
        "cli channel surface should remain mapped to the cli domain"
    );
    assert!(
        !loongclaw_daemon::import_cli::surface_matches_selected_domains(
            &cli_surface,
            &selected_channels
        ),
        "cli channel surface should not be folded into the generic channels domain"
    );
}

#[test]
fn import_cli_apply_selected_domains_preserves_unselected_existing_values() {
    let candidate = sample_import_candidate();
    let mut base = mvp::config::LoongClawConfig::default();
    base.provider.kind = mvp::config::ProviderKind::Anthropic;
    base.provider.model = "claude-sonnet-4-5".to_owned();
    base.provider.api_key_env = Some("ANTHROPIC_API_KEY".to_owned());
    base.feishu.enabled = true;
    base.feishu.app_id_env = Some("FEISHU_APP_ID".to_owned());
    base.feishu.app_secret_env = Some("FEISHU_APP_SECRET".to_owned());
    base.tools.file_root = Some("~/workspace/current".to_owned());

    let selected = vec![
        loongclaw_daemon::migration::types::SetupDomainKind::Channels,
        loongclaw_daemon::migration::types::SetupDomainKind::Cli,
    ];
    let applied = loongclaw_daemon::import_cli::apply_selected_domains_to_config(
        &base, &candidate, &selected,
    );

    assert_eq!(
        applied.provider.kind,
        mvp::config::ProviderKind::Anthropic,
        "provider should remain unchanged when provider is not selected"
    );
    assert_eq!(
        applied.provider.model, "claude-sonnet-4-5",
        "provider model should remain unchanged when provider is not selected"
    );
    assert_eq!(
        applied.cli.system_prompt, "Imported CLI prompt",
        "selected CLI domain should be imported"
    );
    assert!(
        applied.telegram.enabled,
        "selected channels should import telegram from the candidate"
    );
    assert!(
        applied.feishu.enabled,
        "channels merge should preserve existing feishu settings when the candidate does not include feishu"
    );
    assert_eq!(
        applied.tools.file_root.as_deref(),
        Some("~/workspace/current"),
        "tools should remain unchanged when tools are not selected"
    );
}

#[test]
fn import_cli_apply_selected_channels_supplements_existing_channel_fields() {
    let candidate = sample_import_candidate();
    let mut base = mvp::config::LoongClawConfig::default();
    base.telegram.enabled = true;
    base.telegram.allowed_chat_ids = vec![42];
    base.telegram.polling_timeout_s = 90;

    let applied = loongclaw_daemon::import_cli::apply_selected_domains_to_config(
        &base,
        &candidate,
        &[loongclaw_daemon::migration::types::SetupDomainKind::Channels],
    );

    assert_eq!(
        applied.telegram.allowed_chat_ids,
        vec![42],
        "channel import should preserve existing telegram allowlist while filling credentials"
    );
    assert_eq!(
        applied.telegram.polling_timeout_s, 90,
        "channel import should preserve non-default channel tuning while filling credentials"
    );
    assert_eq!(
        applied.telegram.bot_token_env.as_deref(),
        Some("TELEGRAM_BOT_TOKEN"),
        "channel import should still fill the reusable credential pointer from the candidate"
    );
}

#[test]
fn import_cli_render_preview_lists_domain_status_and_source() {
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
                summary: "OpenAI · openai/gpt-5.1-codex".to_owned(),
            },
            loongclaw_daemon::migration::types::DomainPreview {
                kind: loongclaw_daemon::migration::types::SetupDomainKind::WorkspaceGuidance,
                status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
                decision: Some(loongclaw_daemon::migration::types::PreviewDecision::UseDetected),
                source: "workspace".to_owned(),
                summary: "AGENTS.md".to_owned(),
            },
        ],
        channel_candidates: Vec::new(),
        workspace_guidance: vec![
            loongclaw_daemon::migration::types::WorkspaceGuidanceCandidate {
                kind: loongclaw_daemon::migration::types::WorkspaceGuidanceKind::Agents,
                path: "/tmp/project/AGENTS.md".to_owned(),
            },
        ],
    };

    let lines = loongclaw_daemon::import_cli::render_import_preview_lines_for_width(&candidate, 80);

    assert!(
        lines.iter().any(|line| line.contains("provider")),
        "preview should list provider domain: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Codex config at ~/.codex/config.toml")),
        "preview should preserve source attribution: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("workspace guidance")),
        "preview should list workspace guidance: {lines:#?}"
    );
}

#[test]
fn import_cli_render_preview_includes_brand_header_and_title() {
    let lines = loongclaw_daemon::import_cli::render_import_preview_lines_for_width(
        &sample_import_candidate(),
        80,
    );

    assert!(
        lines[0].starts_with("██╗"),
        "import preview should start with the shared LOONGCLAW brand block: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.starts_with('v')),
        "import preview should include a version line under the banner: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "import preview"),
        "import preview should include a stable title line: {lines:#?}"
    );
}

#[test]
fn import_cli_render_preview_labels_candidate_position_when_multiple_candidates() {
    let recommended = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan,
        "recommended import plan",
        mvp::config::ProviderKind::Openai,
        "openai/gpt-5.1-codex",
        "OPENAI_API_KEY",
    );
    let env = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::Environment,
        "your current environment",
        mvp::config::ProviderKind::Deepseek,
        "deepseek-chat",
        "DEEPSEEK_API_KEY",
    );
    let all_candidates = vec![recommended.clone(), env.clone()];

    let first = loongclaw_daemon::import_cli::render_import_preview_lines_for_candidates(
        &recommended,
        &all_candidates,
        80,
    );
    let second = loongclaw_daemon::import_cli::render_import_preview_lines_for_candidates(
        &env,
        &all_candidates,
        80,
    );

    assert!(
        first.iter().any(|line| line == "candidate 1 of 2"),
        "multi-candidate preview should label the first candidate so users can compare options reliably: {first:#?}"
    );
    assert!(
        second.iter().any(|line| line == "candidate 2 of 2"),
        "multi-candidate preview should label the second candidate so users can compare options reliably: {second:#?}"
    );
}

#[test]
fn import_cli_apply_summary_wraps_long_path_and_domains_for_narrow_width() {
    let candidate = sample_import_candidate();
    let lines = loongclaw_daemon::import_cli::render_import_apply_summary_lines_for_width(
        std::path::Path::new("/tmp/shared workspace/loongclaw config.toml"),
        &candidate,
        &[
            loongclaw_daemon::migration::types::SetupDomainKind::Provider,
            loongclaw_daemon::migration::types::SetupDomainKind::Channels,
            loongclaw_daemon::migration::types::SetupDomainKind::WorkspaceGuidance,
        ],
        &candidate.config,
        true,
        46,
    );

    assert!(
        lines[0].starts_with("LOONGCLAW  v"),
        "apply summary should use the compact LOONGCLAW header: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "import applied"),
        "apply summary should keep a focused title: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "- write mode: supplemented existing config"),
        "apply summary should explain that the import supplemented an existing config when one was present: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "- config: /tmp/shared workspace/loongclaw"),
        "apply summary should keep the config label visible before wrapping long paths: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "  config.toml"),
        "apply summary should continue wrapped config paths on an indented line: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "- domains: provider, channels"),
        "apply summary should wrap long domain lists instead of overflowing them: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "  workspace guidance"),
        "apply summary should continue wrapped domain lists on readable continuation lines: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "next step: loongclaw ask --config '/tmp/shared"),
        "apply summary should keep the ask-next-step label visible before wrapping long command paths: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "  workspace/loongclaw config.toml' --message"),
        "apply summary should continue wrapped ask commands on an indented line: {lines:#?}"
    );
}

#[test]
fn import_cli_apply_summary_distinguishes_new_config_creation() {
    let candidate = sample_import_candidate();
    let lines = loongclaw_daemon::import_cli::render_import_apply_summary_lines_for_width(
        std::path::Path::new("/tmp/loongclaw-config.toml"),
        &candidate,
        &[loongclaw_daemon::migration::types::SetupDomainKind::Channels],
        &candidate.config,
        false,
        80,
    );

    assert!(
        lines
            .iter()
            .any(|line| line == "- write mode: created new config"),
        "apply summary should distinguish a brand-new config from a supplemental import: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| line != "- write mode: supplemented existing config"),
        "new-config apply summary should not reuse the supplement wording: {lines:#?}"
    );
}

#[test]
fn import_cli_apply_summary_includes_registry_channel_actions() {
    let candidate = sample_import_candidate();
    let lines = loongclaw_daemon::import_cli::render_import_apply_summary_lines_for_width(
        std::path::Path::new("/tmp/loongclaw-config.toml"),
        &candidate,
        &[loongclaw_daemon::migration::types::SetupDomainKind::Channels],
        &candidate.config,
        false,
        120,
    );

    assert!(
        lines.iter().any(|line| {
            line == "also available: chat · loongclaw chat --config '/tmp/loongclaw-config.toml'"
        }),
        "apply summary should surface interactive chat immediately after the primary ask step: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| {
            line
                == "also available: telegram · loongclaw telegram-serve --config '/tmp/loongclaw-config.toml'"
        }),
        "apply summary should continue surfacing registry-driven channel handoff commands after ask/chat: {lines:#?}"
    );
}

#[test]
fn import_cli_apply_summary_shell_quotes_config_paths_with_single_quotes() {
    let candidate = sample_import_candidate();
    let lines = loongclaw_daemon::import_cli::render_import_apply_summary_lines_for_width(
        std::path::Path::new("/tmp/loongclaw's config.toml"),
        &candidate,
        &[loongclaw_daemon::migration::types::SetupDomainKind::Channels],
        &candidate.config,
        false,
        160,
    );
    let rendered = lines.join(" ");

    assert!(
        rendered.contains(
            "next step: loongclaw ask --config '/tmp/loongclaw'\"'\"'s config.toml' --message"
        ),
        "apply summary should shell-quote single quotes in the primary ask command: {lines:#?}"
    );
    assert!(
        rendered.contains(
            "also available: chat · loongclaw chat --config '/tmp/loongclaw'\"'\"'s config.toml'"
        ),
        "apply summary should shell-quote single quotes in the secondary chat command: {lines:#?}"
    );
}

#[test]
fn import_cli_apply_summary_uses_channel_handoff_when_cli_is_disabled() {
    let mut candidate = sample_import_candidate();
    candidate.config.cli.enabled = false;
    let lines = loongclaw_daemon::import_cli::render_import_apply_summary_lines_for_width(
        std::path::Path::new("/tmp/loongclaw-config.toml"),
        &candidate,
        &[loongclaw_daemon::migration::types::SetupDomainKind::Channels],
        &candidate.config,
        false,
        120,
    );

    assert!(
        lines.iter().any(|line| {
            line == "next step: loongclaw telegram-serve --config '/tmp/loongclaw-config.toml'"
        }),
        "apply summary should not hand users to CLI chat when the imported config has cli disabled: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.starts_with("next step: loongclaw ask --config")),
        "ask should not remain the primary handoff when cli is disabled: {lines:#?}"
    );
}

#[test]
fn import_cli_render_preview_marks_provider_choice_required_for_unresolved_recommended_plan() {
    let _env_guard = provider_choice_preview_env_guard();
    let mut recommended = sample_import_candidate();
    recommended.source_kind = loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan;
    recommended.source = "recommended import plan".to_owned();
    recommended.domains.retain(|domain| {
        domain.kind != loongclaw_daemon::migration::types::SetupDomainKind::Provider
    });

    let openai = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
        "Codex config at ~/.codex/config.toml",
        mvp::config::ProviderKind::Openai,
        "openai/gpt-5.1-codex",
        "OPENAI_API_KEY",
    );
    let deepseek = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::Environment,
        "your current environment",
        mvp::config::ProviderKind::Deepseek,
        "deepseek-chat",
        "DEEPSEEK_API_KEY",
    );

    let lines = loongclaw_daemon::import_cli::render_import_preview_lines_for_candidates(
        &recommended,
        &[recommended.clone(), openai, deepseek],
        80,
    );

    assert!(
        lines
            .iter()
            .any(|line| line.contains("provider choice required")),
        "recommended preview should make unresolved provider choice explicit: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "- OpenAI [openai]"),
        "preview should list imported provider choices with human-readable labels while preserving the stable id: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "- DeepSeek [deepseek]"),
        "preview should list all conflicting imported providers with human-readable labels while preserving the stable id: {lines:#?}"
    );
}

#[test]
fn import_cli_render_preview_explains_provider_conflict_apply_behavior() {
    let _env_guard = provider_choice_preview_env_guard();
    let mut recommended = sample_import_candidate();
    recommended.source_kind = loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan;
    recommended.source = "recommended import plan".to_owned();
    recommended.domains.retain(|domain| {
        domain.kind != loongclaw_daemon::migration::types::SetupDomainKind::Provider
    });

    let lines = loongclaw_daemon::import_cli::render_import_preview_lines_for_candidates(
        &recommended,
        &[
            recommended.clone(),
            import_candidate_with_provider(
                loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
                "Codex config at ~/.codex/config.toml",
                mvp::config::ProviderKind::Openai,
                "openai/gpt-5.1-codex",
                "OPENAI_API_KEY",
            ),
            import_candidate_with_provider(
                loongclaw_daemon::migration::types::ImportSourceKind::Environment,
                "your current environment",
                mvp::config::ProviderKind::Deepseek,
                "deepseek-chat",
                "DEEPSEEK_API_KEY",
            ),
        ],
        80,
    );

    assert!(
        lines
            .iter()
            .any(|line| line.contains("other detected settings stay merged")),
        "preview should explain that non-provider domains still compose while the active provider remains unresolved: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains(&format!(
            "--provider {}",
            loongclaw_daemon::migration::provider_selection::PROVIDER_SELECTOR_PLACEHOLDER
        ))),
        "preview should direct power users to the explicit provider flag: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("selectors: openai, openai/gpt-5.1-codex, gpt-5.1-codex")),
        "preview should surface the exact selectors users can type for each provider choice: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("try one of: openai, deepseek")),
        "preview should surface quick selector picks when provider choice is still unresolved: {lines:#?}"
    );
}

#[test]
fn import_cli_render_preview_wraps_provider_choices_for_narrow_width() {
    let _env_guard = provider_choice_preview_env_guard();
    let mut recommended = sample_import_candidate();
    recommended.source_kind = loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan;
    recommended.source = "recommended import plan".to_owned();
    recommended.domains.retain(|domain| {
        domain.kind != loongclaw_daemon::migration::types::SetupDomainKind::Provider
    });

    let lines = loongclaw_daemon::import_cli::render_import_preview_lines_for_candidates(
        &recommended,
        &[
            recommended.clone(),
            import_candidate_with_provider(
                loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
                "Codex config at ~/.codex/agents/loongclaw/config.toml",
                mvp::config::ProviderKind::Openai,
                "openai/gpt-5.1-codex",
                "OPENAI_API_KEY",
            ),
            import_candidate_with_provider(
                loongclaw_daemon::migration::types::ImportSourceKind::Environment,
                "your current environment",
                mvp::config::ProviderKind::Deepseek,
                "deepseek-chat",
                "DEEPSEEK_API_KEY",
            ),
        ],
        52,
    );

    assert!(
        lines.iter().any(|line| line == "provider choice required:"),
        "narrow import preview should keep the provider-choice heading explicit: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "- OpenAI [openai]"),
        "narrow import preview should keep each provider option on its own compact header row with a readable label plus the stable id: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "  source: Codex config at"),
        "narrow import preview should wrap long provider source labels instead of overflowing them: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "  ~/.codex/agents/loongclaw/config.toml"),
        "narrow import preview should continue long provider source paths on an indented line: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "  summary: OpenAI · openai/gpt-5.1-codex ·"),
        "narrow import preview should wrap long provider summaries in a readable way using the provider display name: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "  credentials resolved"),
        "narrow import preview should keep wrapped provider-summary continuations readable: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "  selector: openai"),
        "narrow import preview should collapse selector aliases into one preferred selector per provider row: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("selectors: openai, openai/gpt-5.1-codex")),
        "narrow import preview should avoid repeating the full selector catalog inside each provider row: {lines:#?}"
    );
}

#[test]
fn import_cli_render_preview_falls_back_to_stacked_provider_rows_when_medium_width_overflows() {
    let _env_guard = provider_choice_preview_env_guard();
    let mut recommended = sample_import_candidate();
    recommended.source_kind = loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan;
    recommended.source = "recommended import plan".to_owned();
    recommended.domains.retain(|domain| {
        domain.kind != loongclaw_daemon::migration::types::SetupDomainKind::Provider
    });

    let lines = loongclaw_daemon::import_cli::render_import_preview_lines_for_candidates(
        &recommended,
        &[
            recommended.clone(),
            import_candidate_with_provider(
                loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
                "Codex config at ~/.codex/agents/loongclaw/config.toml",
                mvp::config::ProviderKind::Openai,
                "openai/gpt-5.1-codex",
                "OPENAI_API_KEY",
            ),
            import_candidate_with_provider(
                loongclaw_daemon::migration::types::ImportSourceKind::Environment,
                "your current environment",
                mvp::config::ProviderKind::Deepseek,
                "deepseek-chat",
                "DEEPSEEK_API_KEY",
            ),
        ],
        80,
    );

    assert!(
        lines.iter().any(|line| line == "provider choice required:"),
        "medium-width preview should still show the provider-choice section: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "- OpenAI [openai]"),
        "medium-width preview should fall back to stacked provider rows when the wide row would overflow, while keeping a readable provider label: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "  summary: OpenAI · openai/gpt-5.1-codex · credentials resolved"),
        "stacked provider fallback should keep provider details as readable stacked lines with the display name instead of a single overlong row: {lines:#?}"
    );
}

#[test]
fn import_cli_render_preview_surfaces_responses_transport_for_provider_candidate() {
    let mut candidate = sample_import_candidate();
    candidate.config.provider.kind = mvp::config::ProviderKind::Deepseek;
    candidate.config.provider.model = "deepseek-chat".to_owned();
    candidate.config.provider.wire_api = mvp::config::ProviderWireApi::Responses;
    candidate.domains[0].summary =
        loongclaw_daemon::provider_presentation::provider_identity_summary(
            &candidate.config.provider,
        );

    let lines = loongclaw_daemon::import_cli::render_import_preview_lines_for_width(&candidate, 80);

    assert!(
        lines.iter().any(|line| {
            line == "provider transport: responses compatibility mode with chat fallback"
        }),
        "import preview should surface the provider transport before apply time: {lines:#?}"
    );
}

#[test]
fn import_cli_render_preview_keeps_provider_choice_transport_visible_on_wide_width() {
    let mut recommended = sample_import_candidate();
    recommended.source_kind = loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan;
    recommended.source = "recommended import plan".to_owned();
    recommended.domains.retain(|domain| {
        domain.kind != loongclaw_daemon::migration::types::SetupDomainKind::Provider
    });

    let mut deepseek = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::Environment,
        "your current environment",
        mvp::config::ProviderKind::Deepseek,
        "deepseek-chat",
        "DEEPSEEK_API_KEY",
    );
    deepseek.config.provider.wire_api = mvp::config::ProviderWireApi::Responses;
    deepseek.domains[0].summary =
        loongclaw_daemon::provider_presentation::provider_identity_summary(
            &deepseek.config.provider,
        );

    let lines = loongclaw_daemon::import_cli::render_import_preview_lines_for_candidates(
        &recommended,
        &[
            recommended.clone(),
            import_candidate_with_provider(
                loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
                "Codex config at ~/.codex/config.toml",
                mvp::config::ProviderKind::Openai,
                "openai/gpt-5.1-codex",
                "OPENAI_API_KEY",
            ),
            deepseek,
        ],
        120,
    );

    assert!(
        lines.iter().any(|line| line == "- DeepSeek [deepseek]"),
        "wide import preview should keep provider choices readable when one choice needs transport detail, with a readable label plus the stable id: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| { line == "  transport: responses compatibility mode with chat fallback" }),
        "wide import preview should keep per-choice Responses transport visible instead of hiding it in the wide-row layout: {lines:#?}"
    );
}

#[test]
fn import_cli_json_preview_redacts_config_secrets() {
    let mut candidate = sample_import_candidate();
    candidate.config.provider.api_key = Some("super-secret-provider-key".to_owned());
    candidate.config.telegram.bot_token = Some("123456:telegram-secret".to_owned());

    let payload = loongclaw_daemon::import_cli::render_import_preview_json(&[candidate])
        .expect("json preview should render");

    assert!(
        payload.contains("Codex config at ~/.codex/config.toml"),
        "preview json should keep source attribution: {payload}"
    );
    assert!(
        payload.contains("\"domains\""),
        "preview json should expose domain previews: {payload}"
    );
    assert!(
        !payload.contains("super-secret-provider-key"),
        "preview json must not leak inline provider secrets: {payload}"
    );
    assert!(
        !payload.contains("123456:telegram-secret"),
        "preview json must not leak inline channel secrets: {payload}"
    );
}

#[test]
fn import_cli_json_preview_includes_provider_selection_requirements() {
    let mut recommended = sample_import_candidate();
    recommended.source_kind = loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan;
    recommended.source = "recommended import plan".to_owned();
    recommended.domains.retain(|domain| {
        domain.kind != loongclaw_daemon::migration::types::SetupDomainKind::Provider
    });

    let payload = loongclaw_daemon::import_cli::render_import_preview_json(&[
        recommended.clone(),
        import_candidate_with_provider(
            loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
            "Codex config at ~/.codex/config.toml",
            mvp::config::ProviderKind::Openai,
            "openai/gpt-5.1-codex",
            "OPENAI_API_KEY",
        ),
        import_candidate_with_provider(
            loongclaw_daemon::migration::types::ImportSourceKind::Environment,
            "your current environment",
            mvp::config::ProviderKind::Deepseek,
            "deepseek-chat",
            "DEEPSEEK_API_KEY",
        ),
    ])
    .expect("json preview should render");

    assert!(
        payload.contains("\"provider_selection\""),
        "preview json should expose provider choice metadata for scriptable flows: {payload}"
    );
    assert!(
        payload.contains("\"required\": true"),
        "preview json should mark unresolved provider selection as required: {payload}"
    );
    assert!(
        payload.contains("\"kind\": \"openai\"") && payload.contains("\"kind\": \"deepseek\""),
        "preview json should list the available provider choices: {payload}"
    );
}

#[test]
fn import_cli_json_preview_includes_provider_choice_transport() {
    let mut recommended = sample_import_candidate();
    recommended.source_kind = loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan;
    recommended.source = "recommended import plan".to_owned();
    recommended.domains.retain(|domain| {
        domain.kind != loongclaw_daemon::migration::types::SetupDomainKind::Provider
    });

    let mut deepseek = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::Environment,
        "your current environment",
        mvp::config::ProviderKind::Deepseek,
        "deepseek-chat",
        "DEEPSEEK_API_KEY",
    );
    deepseek.config.provider.wire_api = mvp::config::ProviderWireApi::Responses;
    deepseek.domains[0].summary =
        loongclaw_daemon::provider_presentation::provider_identity_summary(
            &deepseek.config.provider,
        );

    let payload = loongclaw_daemon::import_cli::render_import_preview_json(&[
        recommended.clone(),
        import_candidate_with_provider(
            loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
            "Codex config at ~/.codex/config.toml",
            mvp::config::ProviderKind::Openai,
            "openai/gpt-5.1-codex",
            "OPENAI_API_KEY",
        ),
        deepseek,
    ])
    .expect("json preview should render");

    assert!(
        payload.contains("\"transport\": \"responses compatibility mode with chat fallback\""),
        "provider choice transport should be exposed in json preview for power-user and scripted import flows: {payload}"
    );
}

#[test]
fn import_cli_json_preview_includes_source_path_for_path_level_selection() {
    let payload =
        loongclaw_daemon::import_cli::render_import_preview_json(&[sample_import_candidate()])
            .expect("json preview should render");

    assert!(
        payload.contains("\"source_path\": \"~/.codex/config.toml\""),
        "preview json should expose the extracted source path for path-level disambiguation in scripted flows: {payload}"
    );
}

#[test]
fn import_cli_apply_prefers_recommended_plan_when_multiple_candidates_exist() {
    let recommended = loongclaw_daemon::migration::types::ImportCandidate {
        source_kind: loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan,
        source: "recommended import plan".to_owned(),
        ..sample_import_candidate()
    };
    let candidates = vec![
        sample_import_candidate(),
        loongclaw_daemon::migration::types::ImportCandidate {
            source_kind: loongclaw_daemon::migration::types::ImportSourceKind::Environment,
            source: "your current environment".to_owned(),
            ..sample_import_candidate()
        },
        recommended,
    ];

    let selected = loongclaw_daemon::import_cli::select_apply_candidate_index(&candidates)
        .expect("recommended plan should be selected automatically");

    assert_eq!(
        candidates[selected].source_kind,
        loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan
    );
}

#[test]
fn import_cli_apply_requires_explicit_source_without_recommended_plan() {
    let candidates = vec![
        sample_import_candidate(),
        loongclaw_daemon::migration::types::ImportCandidate {
            source_kind: loongclaw_daemon::migration::types::ImportSourceKind::Environment,
            source: "your current environment".to_owned(),
            ..sample_import_candidate()
        },
    ];

    let error = loongclaw_daemon::import_cli::select_apply_candidate_index(&candidates)
        .expect_err("multiple raw candidates should still require --from");

    assert!(
        error.contains("--from recommended|existing|codex|env"),
        "error should direct the operator to the supported selectors: {error}"
    );
}

#[test]
fn import_cli_apply_reports_ambiguous_sources_when_from_filter_still_matches_multiple_candidates() {
    let mut first = sample_import_candidate();
    first.source = "Codex config at ~/.codex/config.toml".to_owned();
    let mut second = sample_import_candidate();
    second.source = "Codex config at ~/.codex/agents/loongclaw/config.toml".to_owned();

    let error = loongclaw_daemon::import_cli::select_apply_candidate_index(&[first, second])
        .expect_err("multiple candidates from the same source kind should remain ambiguous");

    assert!(
        error.contains("multiple codex candidates"),
        "error should explain that the selected source kind still resolved to multiple detected configs: {error}"
    );
    assert!(
        error.contains("~/.codex/config.toml"),
        "error should include the first matching source path: {error}"
    );
    assert!(
        error.contains("~/.codex/agents/loongclaw/config.toml"),
        "error should include the second matching source path: {error}"
    );
}

#[test]
fn import_cli_apply_accepts_single_selected_source_without_recommended_plan() {
    let candidates = vec![sample_import_candidate()];

    let selected = loongclaw_daemon::import_cli::select_apply_candidate_index(&candidates)
        .expect("a single selected source should be directly applicable");

    assert_eq!(selected, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn import_cli_detected_multi_path_codex_sources_require_path_level_disambiguation() {
    let temp_root = unique_temp_dir("codex-multi-path-ambiguous");
    let home = temp_root.join("home");
    std::fs::create_dir_all(home.join(".codex")).expect("create fake home codex dir");
    std::fs::create_dir_all(home.join(".codex/agents/loongclaw"))
        .expect("create fake agent-scoped codex dir");
    let output_path = temp_root.join("loongclaw-config.toml");

    std::fs::write(
        home.join(".codex/config.toml"),
        r#"
model_provider = "openai"
model = "openai/gpt-5.1-codex"
"#,
    )
    .expect("write base codex config");
    std::fs::write(
        home.join(".codex/agents/loongclaw/config.toml"),
        r#"
model_provider = "deepseek"
model = "deepseek-chat"
"#,
    )
    .expect("write agent codex config");

    let _env_guard =
        ImportEnvironmentGuard::set(&[("HOME", Some(home.to_string_lossy().as_ref()))]);

    let error = loongclaw_daemon::import_cli::run_import_cli(
        loongclaw_daemon::import_cli::ImportCommandOptions {
            output: Some(output_path.display().to_string()),
            force: false,
            preview: false,
            apply: true,
            json: false,
            from: Some("codex".to_owned()),
            source_path: None,
            provider: None,
            include: Vec::new(),
            exclude: Vec::new(),
        },
    )
    .await
    .expect_err("multiple detected codex configs should require path-level disambiguation");

    assert!(
        error.contains("multiple codex candidates"),
        "runtime import should explain that the Codex source filter still matched multiple configs: {error}"
    );
    assert!(
        error.contains(".codex/config.toml"),
        "runtime import should surface the base Codex config path: {error}"
    );
    assert!(
        error.contains(".codex/agents/loongclaw/config.toml"),
        "runtime import should surface the agent-scoped Codex config path: {error}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn import_cli_source_path_selects_specific_detected_codex_config() {
    let temp_root = unique_temp_dir("codex-source-path-select");
    let home = temp_root.join("home");
    std::fs::create_dir_all(home.join(".codex")).expect("create fake home codex dir");
    std::fs::create_dir_all(home.join(".codex/agents/loongclaw"))
        .expect("create fake agent-scoped codex dir");
    let output_path = temp_root.join("loongclaw-config.toml");

    std::fs::write(
        home.join(".codex/config.toml"),
        r#"
model_provider = "openai"
model = "openai/gpt-5.1-codex"
"#,
    )
    .expect("write base codex config");
    std::fs::write(
        home.join(".codex/agents/loongclaw/config.toml"),
        r#"
model_provider = "deepseek"
model = "deepseek-chat"
"#,
    )
    .expect("write agent codex config");

    let _env_guard = ImportEnvironmentGuard::set(&[
        ("HOME", Some(home.to_string_lossy().as_ref())),
        ("DEEPSEEK_API_KEY", Some("deepseek-test-key")),
    ]);

    loongclaw_daemon::import_cli::run_import_cli(
        loongclaw_daemon::import_cli::ImportCommandOptions {
            output: Some(output_path.display().to_string()),
            force: false,
            preview: false,
            apply: true,
            json: false,
            from: Some("codex".to_owned()),
            source_path: Some("~/.codex/agents/loongclaw/config.toml".to_owned()),
            provider: None,
            include: Vec::new(),
            exclude: Vec::new(),
        },
    )
    .await
    .expect("source-path-selected codex import should apply cleanly");

    let (_, imported) = mvp::config::load(Some(output_path.to_string_lossy().as_ref()))
        .expect("load imported config");
    assert_eq!(
        imported.provider.kind,
        mvp::config::ProviderKind::Deepseek,
        "source-path selection should apply the matching detected Codex config instead of the base one"
    );
    assert_eq!(imported.provider.model, "deepseek-chat");
    assert_eq!(
        imported.provider.api_key.as_deref(),
        Some("${DEEPSEEK_API_KEY}"),
        "source-path-selected provider should retain its provider-specific credential binding in canonical inline-env form"
    );
    assert_eq!(
        imported.provider.authorization_header().as_deref(),
        Some("Bearer deepseek-test-key"),
        "source-path-selected provider should still resolve the imported credential binding at runtime"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn import_cli_applies_codex_source_and_imported_config_is_usable() {
    let temp_root = unique_temp_dir("codex-e2e");
    let home = temp_root.join("home");
    std::fs::create_dir_all(home.join(".codex")).expect("create fake home codex dir");
    let output_path = temp_root.join("loongclaw-config.toml");

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local provider test listener");
    let addr = listener.local_addr().expect("local addr");
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().expect("accept local provider request");
            let mut request_buf = [0_u8; 8192];
            let len = stream.read(&mut request_buf).expect("read request");
            let request = String::from_utf8_lossy(&request_buf[..len]).to_string();
            requests.push(request.clone());

            let (status_line, body) = if request.starts_with("GET /v1/models ") {
                (
                    "HTTP/1.1 200 OK",
                    r#"{"data":[{"id":"openai/gpt-5.1-codex"}]}"#.to_owned(),
                )
            } else if request.starts_with("POST /v1/responses ") {
                (
                    "HTTP/1.1 200 OK",
                    r#"{"output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"pong"}]}]}"#.to_owned(),
                )
            } else {
                (
                    "HTTP/1.1 404 Not Found",
                    r#"{"error":{"message":"unexpected request"}}"#.to_owned(),
                )
            };

            let response = format!(
                "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
        }
        requests
    });

    std::fs::write(
        home.join(".codex/config.toml"),
        format!(
            r#"
model_provider = "sub2api"
model = "openai/gpt-5.1-codex"

[model_providers.sub2api]
base_url = "http://{addr}"
wire_api = "responses"
requires_openai_auth = true
"#
        ),
    )
    .expect("write fake codex config");

    let _env_guard = ImportEnvironmentGuard::set(&[
        ("HOME", Some(home.to_string_lossy().as_ref())),
        ("OPENAI_API_KEY", Some("test-openai-key")),
        (
            "OPENAI_CODEX_OAUTH_TOKEN",
            Some("oauth-token-should-not-be-forwarded"),
        ),
    ]);

    loongclaw_daemon::import_cli::run_import_cli(
        loongclaw_daemon::import_cli::ImportCommandOptions {
            output: Some(output_path.display().to_string()),
            force: false,
            preview: false,
            apply: true,
            json: false,
            from: Some("codex".to_owned()),
            source_path: None,
            provider: None,
            include: Vec::new(),
            exclude: Vec::new(),
        },
    )
    .await
    .expect("codex import should apply cleanly");

    let (_, imported) = mvp::config::load(Some(output_path.to_string_lossy().as_ref()))
        .expect("load imported config");
    assert_eq!(imported.provider.kind, mvp::config::ProviderKind::Openai);
    assert_eq!(imported.provider.model, "openai/gpt-5.1-codex");
    assert_eq!(imported.provider.base_url, format!("http://{addr}"));
    assert_eq!(
        imported.provider.wire_api,
        mvp::config::ProviderWireApi::Responses
    );
    assert_eq!(
        imported.provider.api_key.as_deref(),
        Some("${OPENAI_API_KEY}"),
        "codex import should persist OpenAI auth in canonical inline-env form"
    );
    assert!(
        imported.provider.api_key_env.is_none(),
        "codex import should not keep the legacy api_key_env pointer once the canonical inline-env reference is written"
    );

    let models = mvp::provider::fetch_available_models(&imported)
        .await
        .expect("imported config should fetch models from the codex-derived endpoint");
    assert_eq!(models, vec!["openai/gpt-5.1-codex".to_owned()]);

    let completion = mvp::provider::request_completion(
        &imported,
        &[json!({
            "role": "user",
            "content": "ping"
        })],
        mvp::provider::ProviderRuntimeBinding::direct(),
    )
    .await
    .expect("imported config should support a provider completion request");
    assert_eq!(completion, "pong");

    let requests = server.join().expect("join local provider server");
    assert!(
        requests.iter().any(|request| {
            let normalized = request.to_ascii_lowercase();
            request.starts_with("GET /v1/models ")
                && normalized.contains("authorization: bearer test-openai-key")
        }),
        "model probe should use the imported auth binding against the derived models endpoint: {requests:#?}"
    );
    assert!(
        requests.iter().any(|request| {
            let normalized = request.to_ascii_lowercase();
            request.starts_with("POST /v1/responses ")
                && normalized.contains("authorization: bearer test-openai-key")
                && request.contains("\"input\"")
                && !request.contains("\"max_output_tokens\"")
        }),
        "completion request should use the imported auth binding against the responses endpoint: {requests:#?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn import_cli_applies_codex_source_with_custom_chat_completions_path() {
    let temp_root = unique_temp_dir("codex-custom-chat-path");
    let home = temp_root.join("home");
    std::fs::create_dir_all(home.join(".codex")).expect("create fake home codex dir");
    let output_path = temp_root.join("loongclaw-config.toml");

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local provider test listener");
    let addr = listener.local_addr().expect("local addr");
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().expect("accept local provider request");
            let mut request_buf = [0_u8; 8192];
            let len = stream.read(&mut request_buf).expect("read request");
            let request = String::from_utf8_lossy(&request_buf[..len]).to_string();
            requests.push(request.clone());

            let (status_line, body) = if request.starts_with("GET /proxy/models ") {
                (
                    "HTTP/1.1 200 OK",
                    r#"{"data":[{"id":"openai/gpt-5.1-codex"}]}"#.to_owned(),
                )
            } else if request.starts_with("POST /proxy/chat/completions ") {
                (
                    "HTTP/1.1 200 OK",
                    r#"{"choices":[{"message":{"role":"assistant","content":"pong"}}]}"#.to_owned(),
                )
            } else {
                (
                    "HTTP/1.1 404 Not Found",
                    r#"{"error":{"message":"unexpected request"}}"#.to_owned(),
                )
            };

            let response = format!(
                "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
        }
        requests
    });

    std::fs::write(
        home.join(".codex/config.toml"),
        format!(
            r#"
model_provider = "proxy"
model = "openai/gpt-5.1-codex"

[model_providers.proxy]
base_url = "http://{addr}"
chat_completions_path = "/proxy/chat/completions"
wire_api = "chat_completions"
requires_openai_auth = true
"#
        ),
    )
    .expect("write fake codex config");

    let _env_guard = ImportEnvironmentGuard::set(&[
        ("HOME", Some(home.to_string_lossy().as_ref())),
        ("OPENAI_API_KEY", Some("test-openai-key")),
    ]);

    loongclaw_daemon::import_cli::run_import_cli(
        loongclaw_daemon::import_cli::ImportCommandOptions {
            output: Some(output_path.display().to_string()),
            force: false,
            preview: false,
            apply: true,
            json: false,
            from: Some("codex".to_owned()),
            source_path: None,
            provider: None,
            include: Vec::new(),
            exclude: Vec::new(),
        },
    )
    .await
    .expect("codex import should apply cleanly");

    let (_, imported) = mvp::config::load(Some(output_path.to_string_lossy().as_ref()))
        .expect("load imported config");
    assert_eq!(imported.provider.kind, mvp::config::ProviderKind::Openai);
    assert_eq!(imported.provider.model, "openai/gpt-5.1-codex");
    assert_eq!(imported.provider.base_url, format!("http://{addr}"));
    assert_eq!(
        imported.provider.chat_completions_path,
        "/proxy/chat/completions"
    );
    assert_eq!(
        imported.provider.wire_api,
        mvp::config::ProviderWireApi::ChatCompletions
    );

    let models = mvp::provider::fetch_available_models(&imported)
        .await
        .expect("imported config should fetch models from the custom chat-path-derived endpoint");
    assert_eq!(models, vec!["openai/gpt-5.1-codex".to_owned()]);

    let completion = mvp::provider::request_completion(
        &imported,
        &[json!({
            "role": "user",
            "content": "ping"
        })],
        mvp::provider::ProviderRuntimeBinding::direct(),
    )
    .await
    .expect("imported config should send chat completions to the custom endpoint");
    assert_eq!(completion, "pong");

    let requests = server.join().expect("join provider server");
    assert!(
        requests
            .iter()
            .any(|request| request.starts_with("GET /proxy/models ")),
        "model discovery should use the imported custom chat-path-derived models endpoint: {requests:#?}"
    );
    assert!(
        requests
            .iter()
            .any(|request| request.starts_with("POST /proxy/chat/completions ")),
        "completion request should use the imported custom chat completions path: {requests:#?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn import_cli_applies_codex_source_and_imported_turn_falls_back_from_responses() {
    let temp_root = unique_temp_dir("codex-turn-fallback");
    let home = temp_root.join("home");
    std::fs::create_dir_all(home.join(".codex")).expect("create fake home codex dir");
    let output_path = temp_root.join("loongclaw-config.toml");

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local provider test listener");
    let addr = listener.local_addr().expect("local addr");
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().expect("accept local provider request");
            let mut request_buf = [0_u8; 8192];
            let len = stream.read(&mut request_buf).expect("read request");
            let request = String::from_utf8_lossy(&request_buf[..len]).to_string();
            requests.push(request.clone());

            let (status_line, body) = if request.starts_with("GET /v1/models ") {
                (
                    "HTTP/1.1 200 OK",
                    r#"{"data":[{"id":"openai/gpt-5.1-codex"}]}"#.to_owned(),
                )
            } else if request.starts_with("POST /v1/responses ") {
                (
                    "HTTP/1.1 422 Unprocessable Entity",
                    r#"{"error":{"code":"invalid_request_error","param":"input","message":"Missing required parameter: `messages`. This provider expects /v1/chat/completions instead of Responses input."}}"#.to_owned(),
                )
            } else if request.starts_with("POST /v1/chat/completions ") {
                (
                    "HTTP/1.1 200 OK",
                    r#"{"choices":[{"message":{"role":"assistant","content":"fallback turn ok"}}]}"#
                        .to_owned(),
                )
            } else {
                (
                    "HTTP/1.1 404 Not Found",
                    r#"{"error":{"message":"unexpected request"}}"#.to_owned(),
                )
            };

            let response = format!(
                "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
        }
        requests
    });

    std::fs::write(
        home.join(".codex/config.toml"),
        format!(
            r#"
model_provider = "sub2api"
model = "openai/gpt-5.1-codex"

[model_providers.sub2api]
base_url = "http://{addr}"
wire_api = "responses"
requires_openai_auth = true
"#
        ),
    )
    .expect("write fake codex config");

    let _env_guard = ImportEnvironmentGuard::set(&[
        ("HOME", Some(home.to_string_lossy().as_ref())),
        ("OPENAI_API_KEY", Some("test-openai-key")),
    ]);

    loongclaw_daemon::import_cli::run_import_cli(
        loongclaw_daemon::import_cli::ImportCommandOptions {
            output: Some(output_path.display().to_string()),
            force: false,
            preview: false,
            apply: true,
            json: false,
            from: Some("codex".to_owned()),
            source_path: None,
            provider: None,
            include: Vec::new(),
            exclude: Vec::new(),
        },
    )
    .await
    .expect("codex import should apply cleanly");

    let (_, imported) = mvp::config::load(Some(output_path.to_string_lossy().as_ref()))
        .expect("load imported config");
    let turn = mvp::provider::request_turn(
        &imported,
        "import-codex-session",
        "import-codex-turn",
        &[json!({
            "role": "user",
            "content": "ping"
        })],
        mvp::provider::ProviderRuntimeBinding::direct(),
    )
    .await
    .expect("imported config should fallback from Responses to chat-completions for turn requests");
    assert_eq!(turn.assistant_text, "fallback turn ok");

    let requests = server.join().expect("join local provider server");
    assert!(
        requests.iter().any(|request| {
            request.starts_with("POST /v1/responses ")
                && request.contains("\"input\"")
                && !request.contains("\"messages\"")
        }),
        "turn path should first attempt the imported Responses endpoint: {requests:#?}"
    );
    assert!(
        requests.iter().any(|request| {
            request.starts_with("POST /v1/chat/completions ") && request.contains("\"messages\"")
        }),
        "turn path should retry the imported provider through chat-completions fallback: {requests:#?}"
    );
}

#[test]
fn import_cli_provider_selection_requires_explicit_choice_for_unresolved_recommended_plan() {
    let mut recommended = sample_import_candidate();
    recommended.source_kind = loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan;
    recommended.source = "recommended import plan".to_owned();
    recommended.domains.retain(|domain| {
        domain.kind != loongclaw_daemon::migration::types::SetupDomainKind::Provider
    });

    let error = loongclaw_daemon::import_cli::resolve_import_provider_selection(
        &mvp::config::ProviderConfig::default(),
        &[
            recommended.clone(),
            import_candidate_with_provider(
                loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
                "Codex config at ~/.codex/config.toml",
                mvp::config::ProviderKind::Openai,
                "openai/gpt-5.1-codex",
                "OPENAI_API_KEY",
            ),
            import_candidate_with_provider(
                loongclaw_daemon::migration::types::ImportSourceKind::Environment,
                "your current environment",
                mvp::config::ProviderKind::Deepseek,
                "deepseek-chat",
                "DEEPSEEK_API_KEY",
            ),
        ],
        &recommended,
        None,
    )
    .expect_err("unresolved recommended plan should require an explicit provider choice");

    assert!(
        error.contains("--provider"),
        "scriptable import flow should direct experts to the explicit provider flag: {error}"
    );
}

#[test]
fn import_cli_provider_selection_accepts_manual_choice_for_unresolved_recommended_plan() {
    let mut recommended = sample_import_candidate();
    recommended.source_kind = loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan;
    recommended.source = "recommended import plan".to_owned();
    recommended.domains.retain(|domain| {
        domain.kind != loongclaw_daemon::migration::types::SetupDomainKind::Provider
    });

    let provider = loongclaw_daemon::import_cli::resolve_import_provider_selection(
        &mvp::config::ProviderConfig::default(),
        &[
            recommended.clone(),
            import_candidate_with_provider(
                loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
                "Codex config at ~/.codex/config.toml",
                mvp::config::ProviderKind::Openai,
                "openai/gpt-5.1-codex",
                "OPENAI_API_KEY",
            ),
            import_candidate_with_provider(
                loongclaw_daemon::migration::types::ImportSourceKind::Environment,
                "your current environment",
                mvp::config::ProviderKind::Deepseek,
                "deepseek-chat",
                "DEEPSEEK_API_KEY",
            ),
        ],
        &recommended,
        Some("deepseek"),
    )
    .expect("explicit provider choice should be accepted");

    assert_eq!(provider.kind, mvp::config::ProviderKind::Deepseek);
    assert_eq!(provider.model, "deepseek-chat");
    assert_eq!(provider.api_key_env.as_deref(), Some("DEEPSEEK_API_KEY"));
}

#[test]
fn provider_selection_resolve_choice_by_kind_prefers_default_profile_id() {
    let plan = loongclaw_daemon::migration::ProviderSelectionPlan {
        imported_choices: vec![
            loongclaw_daemon::migration::ImportedProviderChoice {
                profile_id: "openai-main".to_owned(),
                kind: mvp::config::ProviderKind::Openai,
                source: "Codex config at ~/.codex/config.toml".to_owned(),
                summary: "OpenAI · gpt-5 · credentials resolved".to_owned(),
                config: mvp::config::ProviderConfig {
                    kind: mvp::config::ProviderKind::Openai,
                    model: "gpt-5".to_owned(),
                    ..mvp::config::ProviderConfig::default()
                },
            },
            loongclaw_daemon::migration::ImportedProviderChoice {
                profile_id: "openai-reasoning".to_owned(),
                kind: mvp::config::ProviderKind::Openai,
                source: "your current environment".to_owned(),
                summary: "OpenAI · o4-mini · credentials resolved".to_owned(),
                config: mvp::config::ProviderConfig {
                    kind: mvp::config::ProviderKind::Openai,
                    model: "o4-mini".to_owned(),
                    ..mvp::config::ProviderConfig::default()
                },
            },
        ],
        default_kind: Some(mvp::config::ProviderKind::Openai),
        default_profile_id: Some("openai-reasoning".to_owned()),
        requires_explicit_choice: false,
    };

    let choice = loongclaw_daemon::migration::resolve_choice_by_selector(&plan, "openai")
        .expect("kind selector should prefer the plan default profile when multiple same-kind profiles exist");
    assert_eq!(choice.profile_id, "openai-reasoning");
}

#[test]
fn provider_selection_recommendation_hint_prefers_short_human_selectors() {
    let plan = loongclaw_daemon::migration::ProviderSelectionPlan {
        imported_choices: vec![
            loongclaw_daemon::migration::ImportedProviderChoice {
                profile_id: "openai-reasoning".to_owned(),
                kind: mvp::config::ProviderKind::Openai,
                source: "your current environment".to_owned(),
                summary: "OpenAI · o4-mini · credentials resolved".to_owned(),
                config: mvp::config::ProviderConfig {
                    kind: mvp::config::ProviderKind::Openai,
                    model: "o4-mini".to_owned(),
                    ..mvp::config::ProviderConfig::default()
                },
            },
            loongclaw_daemon::migration::ImportedProviderChoice {
                profile_id: "openai-main".to_owned(),
                kind: mvp::config::ProviderKind::Openai,
                source: "Codex config at ~/.codex/config.toml".to_owned(),
                summary: "OpenAI · gpt-5 · credentials resolved".to_owned(),
                config: mvp::config::ProviderConfig {
                    kind: mvp::config::ProviderKind::Openai,
                    model: "gpt-5".to_owned(),
                    ..mvp::config::ProviderConfig::default()
                },
            },
            loongclaw_daemon::migration::ImportedProviderChoice {
                profile_id: "deepseek-main".to_owned(),
                kind: mvp::config::ProviderKind::Deepseek,
                source: "another source".to_owned(),
                summary: "DeepSeek · deepseek-chat · credentials resolved".to_owned(),
                config: mvp::config::ProviderConfig {
                    kind: mvp::config::ProviderKind::Deepseek,
                    model: "deepseek-chat".to_owned(),
                    ..mvp::config::ProviderConfig::default()
                },
            },
        ],
        default_kind: Some(mvp::config::ProviderKind::Openai),
        default_profile_id: Some("openai-reasoning".to_owned()),
        requires_explicit_choice: false,
    };

    assert_eq!(
        loongclaw_daemon::migration::recommendation_hint(&plan),
        Some("try one of: openai, gpt-5, deepseek".to_owned())
    );
    assert_eq!(
        loongclaw_daemon::migration::preferred_selector_for_choice(&plan, "openai-main"),
        Some("gpt-5".to_owned())
    );
}

#[test]
fn provider_selection_resolve_choice_by_model_accepts_unique_model_name() {
    let plan = loongclaw_daemon::migration::ProviderSelectionPlan {
        imported_choices: vec![
            loongclaw_daemon::migration::ImportedProviderChoice {
                profile_id: "openai-main".to_owned(),
                kind: mvp::config::ProviderKind::Openai,
                source: "Codex config at ~/.codex/config.toml".to_owned(),
                summary: "OpenAI · gpt-5 · credentials resolved".to_owned(),
                config: mvp::config::ProviderConfig {
                    kind: mvp::config::ProviderKind::Openai,
                    model: "gpt-5".to_owned(),
                    ..mvp::config::ProviderConfig::default()
                },
            },
            loongclaw_daemon::migration::ImportedProviderChoice {
                profile_id: "deepseek-main".to_owned(),
                kind: mvp::config::ProviderKind::Deepseek,
                source: "your current environment".to_owned(),
                summary: "DeepSeek · deepseek-chat · credentials resolved".to_owned(),
                config: mvp::config::ProviderConfig {
                    kind: mvp::config::ProviderKind::Deepseek,
                    model: "deepseek-chat".to_owned(),
                    ..mvp::config::ProviderConfig::default()
                },
            },
        ],
        default_kind: Some(mvp::config::ProviderKind::Openai),
        default_profile_id: Some("openai-main".to_owned()),
        requires_explicit_choice: false,
    };

    let choice = loongclaw_daemon::migration::resolve_choice_by_selector(&plan, "deepseek-chat")
        .expect("model selector should resolve to the unique matching imported profile");
    assert_eq!(choice.profile_id, "deepseek-main");
}

#[test]
fn provider_selection_resolve_choice_by_model_suffix_accepts_unique_suffix() {
    let plan = loongclaw_daemon::migration::ProviderSelectionPlan {
        imported_choices: vec![
            loongclaw_daemon::migration::ImportedProviderChoice {
                profile_id: "openrouter-main".to_owned(),
                kind: mvp::config::ProviderKind::Openrouter,
                source: "Codex config at ~/.codex/config.toml".to_owned(),
                summary: "OpenRouter · openai/gpt-5.1-codex · credentials resolved".to_owned(),
                config: mvp::config::ProviderConfig {
                    kind: mvp::config::ProviderKind::Openrouter,
                    model: "openai/gpt-5.1-codex".to_owned(),
                    ..mvp::config::ProviderConfig::default()
                },
            },
            loongclaw_daemon::migration::ImportedProviderChoice {
                profile_id: "deepseek-main".to_owned(),
                kind: mvp::config::ProviderKind::Deepseek,
                source: "your current environment".to_owned(),
                summary: "DeepSeek · deepseek-chat · credentials resolved".to_owned(),
                config: mvp::config::ProviderConfig {
                    kind: mvp::config::ProviderKind::Deepseek,
                    model: "deepseek-chat".to_owned(),
                    ..mvp::config::ProviderConfig::default()
                },
            },
        ],
        default_kind: Some(mvp::config::ProviderKind::Openrouter),
        default_profile_id: Some("openrouter-main".to_owned()),
        requires_explicit_choice: false,
    };

    let choice = loongclaw_daemon::migration::resolve_choice_by_selector(&plan, "gpt-5.1-codex")
        .expect("model suffix selector should resolve to the unique matching imported profile");
    assert_eq!(choice.profile_id, "openrouter-main");
}

#[test]
fn import_cli_provider_selection_reports_ambiguous_model_selector() {
    let mut recommended = sample_import_candidate();
    recommended.source_kind = loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan;
    recommended.source = "recommended import plan".to_owned();
    recommended.domains.retain(|domain| {
        domain.kind != loongclaw_daemon::migration::types::SetupDomainKind::Provider
    });

    let error = loongclaw_daemon::import_cli::resolve_import_provider_selection(
        &mvp::config::ProviderConfig::default(),
        &[
            recommended.clone(),
            import_candidate_with_provider(
                loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
                "Codex config at ~/.codex/config.toml",
                mvp::config::ProviderKind::Openai,
                "gpt-5",
                "OPENAI_API_KEY",
            ),
            import_candidate_with_provider(
                loongclaw_daemon::migration::types::ImportSourceKind::Environment,
                "your current environment",
                mvp::config::ProviderKind::Openrouter,
                "gpt-5",
                "OPENROUTER_API_KEY",
            ),
        ],
        &recommended,
        Some("gpt-5"),
    )
    .expect_err("duplicate model selectors should require clarification instead of guessing");

    assert!(error.contains("ambiguous"));
    assert!(error.contains("openai"));
    assert!(error.contains("openrouter"));
    assert!(error.contains("model=gpt-5"));
    assert!(error.contains("selectors=openai"));
    assert!(error.contains("selectors=openrouter"));
}

#[test]
fn import_cli_provider_selection_unknown_selector_lists_accepted_selectors() {
    let mut recommended = sample_import_candidate();
    recommended.source_kind = loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan;
    recommended.source = "recommended import plan".to_owned();
    recommended.domains.retain(|domain| {
        domain.kind != loongclaw_daemon::migration::types::SetupDomainKind::Provider
    });

    let error = loongclaw_daemon::import_cli::resolve_import_provider_selection(
        &mvp::config::ProviderConfig::default(),
        &[
            recommended.clone(),
            import_candidate_with_provider(
                loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
                "Codex config at ~/.codex/config.toml",
                mvp::config::ProviderKind::Openai,
                "openai/gpt-5.1-codex",
                "OPENAI_API_KEY",
            ),
            import_candidate_with_provider(
                loongclaw_daemon::migration::types::ImportSourceKind::Environment,
                "your current environment",
                mvp::config::ProviderKind::Deepseek,
                "deepseek-chat",
                "DEEPSEEK_API_KEY",
            ),
        ],
        &recommended,
        Some("missing-provider"),
    )
    .expect_err("unknown selector should surface accepted selector help");

    assert!(error.contains("accepted selectors"));
    assert!(error.contains("try one of:"));
    assert!(error.contains("openai"));
    assert!(error.contains("openai/gpt-5.1-codex"));
    assert!(error.contains("gpt-5.1-codex"));
    assert!(error.contains("deepseek"));
    assert!(error.contains("deepseek-chat"));
}

#[tokio::test(flavor = "current_thread")]
async fn import_cli_apply_recommended_import_retains_multiple_same_kind_provider_profiles() {
    let temp_root = unique_temp_dir("same-kind-provider-profiles");
    std::fs::create_dir_all(&temp_root).expect("create temp dir");
    let output_path = temp_root.join("config.toml");

    let mut recommended = sample_import_candidate();
    recommended.source_kind = loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan;
    recommended.source = "recommended import plan".to_owned();
    recommended.domains.retain(|domain| {
        domain.kind != loongclaw_daemon::migration::types::SetupDomainKind::Provider
    });

    let codex = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
        "Codex config at ~/.codex/config.toml",
        mvp::config::ProviderKind::Openai,
        "gpt-5",
        "OPENAI_MAIN_API_KEY",
    );
    let env = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::Environment,
        "your current environment",
        mvp::config::ProviderKind::Openai,
        "o4-mini",
        "OPENAI_REASONING_API_KEY",
    );

    loongclaw_daemon::import_cli::apply_import_candidate(
        &output_path,
        true,
        &[recommended.clone(), codex, env],
        &recommended,
        Some("openai-o4-mini"),
    )
    .expect("recommended import should retain multiple same-kind provider profiles");

    let (_, imported) = mvp::config::load(Some(output_path.to_string_lossy().as_ref()))
        .expect("load imported config");

    assert_eq!(
        imported.providers.len(),
        2,
        "recommended import should keep both same-kind provider profiles instead of collapsing them into one saved profile"
    );
    assert!(
        imported.providers.contains_key("openai-gpt-5"),
        "the primary imported profile should keep a stable model-derived id: {:#?}",
        imported.providers.keys().collect::<Vec<_>>()
    );
    assert!(
        imported.providers.contains_key("openai-o4-mini"),
        "the alternate imported profile should keep a stable model-derived id: {:#?}",
        imported.providers.keys().collect::<Vec<_>>()
    );
    assert_eq!(imported.active_provider_id(), Some("openai-o4-mini"));
    assert_eq!(imported.provider.model, "o4-mini");
    assert!(
        imported
            .providers
            .get("openai-o4-mini")
            .expect("saved active profile")
            .default_for_kind,
        "the selected active same-kind profile should become the default_for_kind for future selector-based switching"
    );
    assert!(
        !imported
            .providers
            .get("openai-gpt-5")
            .expect("saved non-active profile")
            .default_for_kind,
        "non-active same-kind profiles should not remain the default after explicit selection"
    );
}

#[test]
fn import_cli_preview_json_reports_provider_profiles_and_active_provider() {
    let payload =
        loongclaw_daemon::import_cli::render_import_preview_json(&[sample_import_candidate()])
            .expect("preview json should serialize");

    assert!(
        payload.contains("\"provider_profiles\""),
        "preview json should expose retained provider profiles: {payload}"
    );
    assert!(
        payload.contains("\"active_provider\""),
        "preview json should expose the active provider candidate: {payload}"
    );
    assert!(
        payload.contains("\"accepted_selectors\""),
        "preview json should expose accepted provider selectors for automation and TUI rendering: {payload}"
    );
    assert!(
        payload.contains("\"gpt-5.1\""),
        "preview json should expose the unique model suffix alias when available: {payload}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn import_cli_apply_supplements_existing_provider_profiles_without_replacing_active_provider()
{
    let temp_root = unique_temp_dir("provider-profile-supplement");
    std::fs::create_dir_all(&temp_root).expect("create temp dir");
    let output_path = temp_root.join("config.toml");

    let mut base = mvp::config::LoongClawConfig::default();
    base.provider.kind = mvp::config::ProviderKind::Openai;
    base.provider.model = "gpt-5".to_owned();
    base.provider.api_key_env = Some("OPENAI_API_KEY".to_owned());
    mvp::config::write(Some(output_path.to_string_lossy().as_ref()), &base, true)
        .expect("write base config");

    let candidate = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::Environment,
        "your current environment",
        mvp::config::ProviderKind::Deepseek,
        "deepseek-chat",
        "DEEPSEEK_API_KEY",
    );

    loongclaw_daemon::import_cli::apply_import_candidate(
        &output_path,
        true,
        std::slice::from_ref(&candidate),
        &candidate,
        None,
    )
    .expect("provider supplement import should apply");

    let (_, imported) = mvp::config::load(Some(output_path.to_string_lossy().as_ref()))
        .expect("load imported config");
    assert_eq!(imported.active_provider_id(), Some("openai"));
    assert_eq!(imported.provider.kind, mvp::config::ProviderKind::Openai);
    assert!(imported.providers.contains_key("openai"));
    assert!(imported.providers.contains_key("deepseek"));
}

#[test]
fn import_cli_apply_summary_surfaces_transport_summary() {
    let mut candidate = sample_import_candidate();
    candidate.config.provider.kind = mvp::config::ProviderKind::Deepseek;
    candidate.config.provider.model = "deepseek-chat".to_owned();
    candidate.config.provider.wire_api = mvp::config::ProviderWireApi::Responses;

    let lines = loongclaw_daemon::import_cli::render_import_apply_summary_lines_for_width(
        std::path::Path::new("/tmp/loongclaw-config.toml"),
        &candidate,
        &[loongclaw_daemon::migration::types::SetupDomainKind::Provider],
        &candidate.config,
        false,
        90,
    );

    assert!(
        lines
            .iter()
            .any(|line| { line == "- transport: responses compatibility mode with chat fallback" }),
        "import apply summary should make the resolved provider transport explicit: {lines:#?}"
    );
}

#[test]
fn import_cli_apply_summary_reports_active_provider_and_saved_profiles() {
    let mut candidate = sample_import_candidate();
    candidate.config.provider.kind = mvp::config::ProviderKind::Deepseek;
    candidate.config.provider.model = "deepseek-chat".to_owned();

    let mut resolved = mvp::config::LoongClawConfig::default();
    resolved.provider.kind = mvp::config::ProviderKind::Openai;
    resolved.provider.model = "gpt-5".to_owned();
    resolved.active_provider = Some("openai".to_owned());
    resolved.providers.insert(
        "openai".to_owned(),
        mvp::config::ProviderProfileConfig::from_provider(resolved.provider.clone()),
    );
    resolved.providers.insert(
        "deepseek".to_owned(),
        mvp::config::ProviderProfileConfig::from_provider(mvp::config::ProviderConfig {
            kind: mvp::config::ProviderKind::Deepseek,
            model: "deepseek-chat".to_owned(),
            api_key_env: Some("DEEPSEEK_API_KEY".to_owned()),
            ..mvp::config::ProviderConfig::default()
        }),
    );

    let lines = loongclaw_daemon::import_cli::render_import_apply_summary_lines_for_width(
        std::path::Path::new("/tmp/loongclaw-config.toml"),
        &candidate,
        &[loongclaw_daemon::migration::types::SetupDomainKind::Provider],
        &resolved,
        true,
        90,
    );

    assert!(
        lines
            .iter()
            .any(|line| line.contains("- active provider: OpenAI")),
        "import apply summary should tell the user which provider remains active after supplementing: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- saved provider profiles: openai, deepseek")),
        "import apply summary should show retained provider profiles after supplementing: {lines:#?}"
    );
}
