use super::*;
use std::path::{Path, PathBuf};
use std::sync::MutexGuard;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::onboard_finalize::{
    OnboardWriteRecovery, format_backup_timestamp_at, resolve_backup_path_at,
};
use crate::onboard_web_search::{
    WebSearchProviderRecommendation, WebSearchProviderRecommendationSource,
    explicit_web_search_provider_override,
    recommend_web_search_provider_from_available_credentials,
};
use crate::test_support::ScopedEnv;

#[test]
fn degraded_terminal_uses_plain_prompt_fallback() {
    let mode = resolve_onboard_interaction_mode_for_test(false, true, false);

    assert_eq!(
        mode,
        crate::onboard_state::OnboardInteractionMode::PlainInteractive,
        "interactive onboarding should fall back to plain prompts when the terminal is attended but rich prompt support is degraded"
    );
}

fn browser_companion_temp_dir(label: &str) -> PathBuf {
    static NEXT_TEMP_DIR_SEED: AtomicU64 = AtomicU64::new(1);
    let seed = NEXT_TEMP_DIR_SEED.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!(
        "loongclaw-browser-companion-onboard-{label}-{}-{seed}",
        std::process::id()
    ));
    std::fs::create_dir_all(&temp_dir).expect("create browser companion onboard temp dir");
    temp_dir
}

fn browser_companion_script_path(temp_dir: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        temp_dir.join("browser-companion.cmd")
    }
    #[cfg(not(windows))]
    {
        temp_dir.join("browser-companion")
    }
}

fn write_browser_companion_version_script(temp_dir: &Path, version: &str) -> PathBuf {
    let script_path = browser_companion_script_path(temp_dir);

    #[cfg(windows)]
    {
        let script_body = format!(
            "@echo off\r\nif \"%~1\"==\"--version\" (\r\n  echo loongclaw-browser-companion {version}\r\n  exit /b 0\r\n)\r\necho unexpected arguments 1>&2\r\nexit /b 1\r\n"
        );
        std::fs::write(&script_path, script_body).expect("write browser companion script");
    }

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        let script_body = format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo 'loongclaw-browser-companion {version}'\n  exit 0\nfi\necho 'unexpected arguments' >&2\nexit 1\n"
        );
        let mut file =
            std::fs::File::create(&script_path).expect("create browser companion script");
        file.write_all(script_body.as_bytes())
            .expect("write browser companion script");
        file.sync_all()
            .expect("sync browser companion script to disk");
        drop(file);

        let metadata = std::fs::metadata(&script_path).expect("script metadata");
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script_path, permissions)
            .expect("chmod browser companion script");
    }

    script_path
}

struct BrowserCompanionEnvGuard {
    _lock: MutexGuard<'static, ()>,
    saved_ready: Option<std::ffi::OsString>,
}

fn set_browser_companion_env_var(key: &str, value: &str) {
    // SAFETY: daemon tests serialize process env mutations behind
    // `lock_daemon_test_environment`, so no concurrent env readers/writers
    // observe racy updates while these tests run.
    #[allow(unsafe_code, clippy::disallowed_methods)]
    unsafe {
        std::env::set_var(key, value);
    }
}

fn remove_browser_companion_env_var(key: &str) {
    // SAFETY: daemon tests serialize process env mutations behind
    // `lock_daemon_test_environment`, so removing the variable here is
    // coordinated with all other env-mutating daemon tests.
    #[allow(unsafe_code, clippy::disallowed_methods)]
    unsafe {
        std::env::remove_var(key);
    }
}

impl BrowserCompanionEnvGuard {
    fn runtime_gate_closed() -> Self {
        Self::set_ready(None)
    }

    fn runtime_gate_open() -> Self {
        Self::set_ready(Some("true"))
    }

    fn set_ready(value: Option<&str>) -> Self {
        let lock = crate::test_support::lock_daemon_test_environment();
        let key = "LOONGCLAW_BROWSER_COMPANION_READY";
        let saved_ready = std::env::var_os(key);
        match value {
            Some(value) => set_browser_companion_env_var(key, value),
            None => remove_browser_companion_env_var(key),
        }
        Self {
            _lock: lock,
            saved_ready,
        }
    }
}

impl Drop for BrowserCompanionEnvGuard {
    fn drop(&mut self) {
        let key = "LOONGCLAW_BROWSER_COMPANION_READY";
        match self.saved_ready.take() {
            Some(value) => set_browser_companion_env_var(key, &value.to_string_lossy()),
            None => remove_browser_companion_env_var(key),
        }
    }
}

fn import_candidate_with_domain_status(
    source_kind: crate::migration::ImportSourceKind,
    source: &str,
    domains: impl IntoIterator<
        Item = (
            crate::migration::SetupDomainKind,
            crate::migration::PreviewStatus,
        ),
    >,
) -> ImportCandidate {
    ImportCandidate {
        source_kind,
        source: source.to_owned(),
        config: mvp::config::LoongClawConfig::default(),
        surfaces: Vec::new(),
        domains: domains
            .into_iter()
            .map(|(kind, status)| crate::migration::DomainPreview {
                kind,
                status,
                decision: Some(crate::migration::types::PreviewDecision::UseDetected),
                source: source.to_owned(),
                summary: format!("{} {}", kind.label(), status.label()),
            })
            .collect(),
        channel_candidates: Vec::new(),
        workspace_guidance: Vec::new(),
    }
}

fn recommended_import_entry_options() -> Vec<OnboardEntryOption> {
    vec![
        OnboardEntryOption {
            choice: OnboardEntryChoice::ImportDetectedSetup,
            label: "Use detected starting point",
            detail: "detected setup is recommended".to_owned(),
            recommended: true,
        },
        OnboardEntryOption {
            choice: OnboardEntryChoice::StartFresh,
            label: "Start fresh",
            detail: "configure from scratch".to_owned(),
            recommended: false,
        },
    ]
}

#[tokio::test(flavor = "current_thread")]
async fn run_preflight_checks_includes_provider_transport_review_for_responses_compatibility_mode()
{
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Deepseek;
    config.provider.model = "deepseek-chat".to_owned();
    config.provider.wire_api = mvp::config::ProviderWireApi::Responses;

    let checks = run_preflight_checks(&config, true).await;

    assert!(
        checks.iter().any(|check| {
            check.name == "provider transport"
                && check.level == OnboardCheckLevel::Warn
                && check
                    .detail
                    .contains("retry chat_completions automatically")
        }),
        "preflight should surface transport review before writing a Responses-compatible config: {checks:#?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn browser_companion_onboard_preflight_warns_when_enabled_without_command() {
    let _env_guard = BrowserCompanionEnvGuard::runtime_gate_closed();
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.api_key = Some(SecretRef::Inline("inline-openai-key".to_owned()));
    config.tools.browser_companion.enabled = true;

    let checks = run_preflight_checks(&config, true).await;

    assert!(
        checks.iter().any(|check| {
            check.name == "browser companion install"
                && check.level == OnboardCheckLevel::Warn
                && check.detail.contains("no command is configured")
        }),
        "onboard preflight should flag companion configs that cannot be executed yet: {checks:#?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn run_preflight_checks_fail_for_invalid_provider_credential_env_value() {
    let secret = "sk-live-direct-secret-value";
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Openai;
    config.provider.api_key_env = Some(secret.to_owned());
    config.provider.api_key = None;

    let checks = run_preflight_checks(&config, true).await;

    assert!(
        checks.iter().any(|check| {
            check.name == "config validation"
                && check.level == OnboardCheckLevel::Fail
                && check.detail.contains("provider.api_key_env")
                && !check.detail.contains(secret)
        }),
        "preflight should fail fast on invalid provider credential env values without echoing the secret: {checks:#?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn browser_companion_onboard_preflight_warns_when_runtime_gate_is_closed() {
    let _env_guard = BrowserCompanionEnvGuard::runtime_gate_closed();
    let temp_dir = browser_companion_temp_dir("runtime-gate");
    let script_path = write_browser_companion_version_script(&temp_dir, "1.5.0");

    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.api_key = Some(SecretRef::Inline("inline-openai-key".to_owned()));
    config.tools.browser_companion.enabled = true;
    config.tools.browser_companion.command = Some(script_path.display().to_string());
    config.tools.browser_companion.expected_version = Some("1.5.0".to_owned());

    let checks = run_preflight_checks(&config, true).await;

    assert!(
        checks.iter().any(|check| {
            check.name == "browser companion install"
                && check.level == OnboardCheckLevel::Warn
                && check.detail.contains("runtime gate is still closed")
        }),
        "onboard preflight should surface that a healthy install still is not runtime-ready: {checks:#?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn browser_companion_onboard_preflight_passes_when_runtime_gate_is_open() {
    let _env_guard = BrowserCompanionEnvGuard::runtime_gate_open();
    let temp_dir = browser_companion_temp_dir("runtime-ready");
    let script_path = write_browser_companion_version_script(&temp_dir, "1.5.0");

    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.api_key = Some(SecretRef::Inline("inline-openai-key".to_owned()));
    config.tools.browser_companion.enabled = true;
    config.tools.browser_companion.command = Some(script_path.display().to_string());
    config.tools.browser_companion.expected_version = Some("1.5.0".to_owned());

    let checks = run_preflight_checks(&config, true).await;

    assert!(
        checks.iter().any(|check| {
            check.name == "browser companion install"
                && check.level == OnboardCheckLevel::Pass
                && check.detail.contains("runtime is ready")
        }),
        "onboard preflight should mark the companion lane healthy when the runtime gate is open: {checks:#?}"
    );
}

#[test]
fn provider_model_probe_failure_warns_for_explicit_model() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.model = "openai/gpt-5.1-codex".to_owned();

    let check =
        provider_model_probe_failure_check(&config, "provider rejected the model list".to_owned());

    assert_eq!(check.name, "provider model probe");
    assert_eq!(check.level, OnboardCheckLevel::Warn);
    assert!(
        check.detail.contains("explicitly configured"),
        "explicit-model probe failures should explain that catalog discovery is advisory: {check:#?}"
    );
}

#[test]
fn provider_model_probe_transport_failure_prioritizes_route_guidance() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.model = "custom-explicit-model".to_owned();

    let check = provider_model_probe_failure_check(
        &config,
        "provider model-list request failed on attempt 3/3: operation timed out".to_owned(),
    );

    assert_eq!(check.name, "provider model probe");
    assert_eq!(check.level, OnboardCheckLevel::Fail);
    assert!(
        check
            .detail
            .contains(crate::provider_route_diagnostics::MODEL_CATALOG_TRANSPORT_FAILED_MARKER),
        "transport probe failures should use the route-focused marker during onboarding: {check:#?}"
    );
    assert!(
        !check.detail.contains("provider.model"),
        "transport probe failures should not suggest model-selection repair when the route is the real blocker: {check:#?}"
    );
    assert!(
        !check.detail.contains("below"),
        "transport probe failures should not promise a later probe section that may not exist in non-interactive output: {check:#?}"
    );
}

#[test]
fn provider_model_probe_failure_fails_for_auto_model() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.model = "auto".to_owned();

    let check =
        provider_model_probe_failure_check(&config, "provider rejected the model list".to_owned());

    assert_eq!(check.name, "provider model probe");
    assert_eq!(check.level, OnboardCheckLevel::Fail);
    assert!(
        check.detail.contains("OpenAI [openai]"),
        "onboard failures should still identify the active provider context: {check:#?}"
    );
    assert!(
        check.detail.contains("model = auto"),
        "auto-model probe failures should explain why onboarding cannot continue with an unresolved automatic model: {check:#?}"
    );
    assert!(
        check.detail.contains("provider.model"),
        "auto-model probe failures should point users to an explicit provider.model remediation path: {check:#?}"
    );
    assert!(
        check.detail.contains("preferred_models"),
        "auto-model probe failures should point users to preferred_models when catalog probing is unavailable: {check:#?}"
    );
}

#[test]
fn provider_model_probe_failure_warns_for_preferred_model_fallbacks() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Minimax;
    config.provider.model = "auto".to_owned();
    config.provider.preferred_models = vec![
        "MiniMax-M2.5".to_owned(),
        "MiniMax-M2.5".to_owned(),
        "MiniMax-M2.7-highspeed".to_owned(),
    ];

    let check =
        provider_model_probe_failure_check(&config, "provider rejected the model list".to_owned());

    assert_eq!(check.name, "provider model probe");
    assert_eq!(check.level, OnboardCheckLevel::Warn);
    assert!(
        check.detail.contains("configured preferred"),
        "onboarding should only advertise fallback continuation for explicitly configured preferred models: {check:#?}"
    );
    assert!(
        check.detail.contains("MiniMax-M2.5"),
        "onboard warning should surface the first fallback model to keep the first-run path actionable: {check:#?}"
    );
}

#[test]
fn provider_model_probe_failure_guides_reviewed_default_for_auto_model() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Deepseek;
    config.provider.model = "auto".to_owned();

    let check =
        provider_model_probe_failure_check(&config, "provider rejected the model list".to_owned());

    assert_eq!(check.name, "provider model probe");
    assert_eq!(check.level, OnboardCheckLevel::Fail);
    assert_eq!(
        check.non_interactive_warning_policy,
        OnboardNonInteractiveWarningPolicy::RequiresExplicitModel
    );
    assert!(
        check.detail.contains("deepseek-chat"),
        "reviewed providers should point users to the reviewed onboarding default when catalog probing is unavailable: {check:#?}"
    );
    assert!(
        check.detail.contains("rerun onboarding"),
        "reviewed providers should suggest rerunning onboarding to accept the reviewed model instead of leaving recovery implicit: {check:#?}"
    );
}

#[test]
fn provider_model_probe_failure_includes_region_hint_for_minimax() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Minimax;
    config.provider.model = "auto".to_owned();

    let check =
        provider_model_probe_failure_check(&config, "provider returned status 401".to_owned());

    assert_eq!(check.name, "provider model probe");
    assert_eq!(check.level, OnboardCheckLevel::Fail);
    assert!(
        check.detail.contains("https://api.minimax.io"),
        "onboard probe failures for region-sensitive providers should surface the alternate endpoint: {check:#?}"
    );
    assert!(
        check.detail.contains("provider.base_url"),
        "onboard probe failures should explain the concrete config knob to change: {check:#?}"
    );
}

#[test]
fn provider_model_probe_failure_skips_region_hint_for_non_auth_errors() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Minimax;
    config.provider.model = "auto".to_owned();

    let check =
        provider_model_probe_failure_check(&config, "provider returned status 503".to_owned());

    assert_eq!(check.name, "provider model probe");
    assert_eq!(check.level, OnboardCheckLevel::Fail);
    assert!(
        !check.detail.contains("provider.base_url"),
        "non-auth probe failures should not steer operators toward region endpoint changes: {check:#?}"
    );
}

#[test]
fn explicit_model_probe_warning_is_accepted_non_interactively() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.model = "openai/gpt-5.1-codex".to_owned();
    let check =
        provider_model_probe_failure_check(&config, "provider rejected the model list".to_owned());
    let options = OnboardCommandOptions {
        output: None,
        force: false,
        non_interactive: true,
        accept_risk: true,
        provider: None,
        model: None,
        api_key_env: None,
        web_search_provider: None,
        web_search_api_key_env: None,
        personality: None,
        memory_profile: None,
        system_prompt: None,
        skip_model_probe: false,
    };

    assert!(
        is_explicitly_accepted_non_interactive_warning(&check, &options),
        "explicit-model probe warnings should not block non-interactive onboarding because model discovery is advisory: {check:#?}"
    );
}

#[test]
fn configured_preferred_model_probe_warning_is_accepted_non_interactively() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Minimax;
    config.provider.model = "auto".to_owned();
    config.provider.preferred_models = vec!["MiniMax-M2.5".to_owned()];
    let check =
        provider_model_probe_failure_check(&config, "provider rejected the model list".to_owned());
    let options = OnboardCommandOptions {
        output: None,
        force: false,
        non_interactive: true,
        accept_risk: true,
        provider: None,
        model: None,
        api_key_env: None,
        web_search_provider: None,
        web_search_api_key_env: None,
        personality: None,
        memory_profile: None,
        system_prompt: None,
        skip_model_probe: false,
    };

    assert!(
        is_explicitly_accepted_non_interactive_warning(&check, &options),
        "configured preferred-model fallback warnings should not block non-interactive onboarding because runtime can still try the operator-configured models: {check:#?}"
    );
}

#[test]
fn non_interactive_preflight_failure_message_uses_first_failing_check_detail() {
    let checks = vec![
        OnboardCheck {
            name: "provider credentials",
            level: OnboardCheckLevel::Pass,
            detail: "credentials ok".to_owned(),
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        },
        OnboardCheck {
            name: "provider model probe",
            level: OnboardCheckLevel::Fail,
            detail: "DeepSeek [deepseek]: model catalog probe failed (401 Unauthorized); current config still uses `model = auto`; rerun onboarding and accept reviewed model `deepseek-chat`, or set `provider.model` / `preferred_models` explicitly".to_owned(),
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        },
    ];

    let message = non_interactive_preflight_failure_message(&checks);

    assert!(
        message.contains("onboard preflight failed: DeepSeek [deepseek]"),
        "non-interactive onboarding should return the actionable failing-check detail instead of a generic probe hint: {message}"
    );
    assert!(
        message.contains("provider.model"),
        "non-interactive onboarding should preserve the explicit remediation from the failing check: {message}"
    );
}

#[test]
fn non_interactive_preflight_failure_message_appends_provider_route_probe_detail_for_transport_failures()
 {
    let checks = vec![
        OnboardCheck {
            name: "provider model probe",
            level: OnboardCheckLevel::Fail,
            detail:
                "OpenAI [openai]: model catalog transport failed (provider model-list request failed on attempt 3/3: operation timed out); runtime could not verify the provider route. inspect provider route diagnostics and retry once dns / proxy / TUN routing is stable"
                    .to_owned(),
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        },
        OnboardCheck {
            name: "provider route probe",
            level: OnboardCheckLevel::Warn,
            detail:
                "request/models host api.openai.com:443: dns resolved to 198.18.0.2 (fake-ip-style); tcp connect ok via 198.18.0.2. the route currently depends on local fake-ip/TUN interception, so intermittent long-request failures usually point to proxy health or direct/bypass rules."
                    .to_owned(),
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        },
    ];

    let message = non_interactive_preflight_failure_message(&checks);

    assert!(
        message.contains("provider route probe"),
        "non-interactive onboarding should mention the collected provider route probe when transport diagnostics are available: {message}"
    );
    assert!(
        message.contains("fake-ip-style"),
        "non-interactive onboarding should include the route-probe detail instead of dropping it behind the first failing check: {message}"
    );
}

#[test]
fn non_interactive_preflight_warning_message_uses_first_blocking_warning_detail() {
    let checks = vec![
        OnboardCheck {
            name: "web search provider",
            level: OnboardCheckLevel::Warn,
            detail: "Tavily: TAVILY_API_KEY (expected). web.search will stay unavailable until the provider credential is supplied".to_owned(),
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        },
    ];
    let options = OnboardCommandOptions {
        output: None,
        force: false,
        non_interactive: true,
        accept_risk: true,
        provider: None,
        model: None,
        api_key_env: None,
        web_search_provider: Some("tavily".to_owned()),
        web_search_api_key_env: None,
        personality: None,
        memory_profile: None,
        system_prompt: None,
        skip_model_probe: false,
    };

    let message = non_interactive_preflight_warning_message(&checks, &options);

    assert!(
        message.contains("web search provider: Tavily"),
        "non-interactive warning failures should surface the first blocking warning detail instead of collapsing to a generic message: {message}"
    );
    assert!(
        message.contains("rerun without --non-interactive"),
        "non-interactive warning failures should still tell the user how to continue interactively: {message}"
    );
}

#[test]
fn config_validation_failure_message_only_matches_config_validation_failures() {
    let checks = vec![
        OnboardCheck {
            name: "provider credentials",
            level: OnboardCheckLevel::Fail,
            detail: "credentials missing".to_owned(),
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        },
        OnboardCheck {
            name: "config validation",
            level: OnboardCheckLevel::Fail,
            detail: "provider.api_key_env must be an environment variable name".to_owned(),
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        },
    ];

    assert_eq!(
        config_validation_failure_message(&checks),
        Some(
            "onboard preflight failed: provider.api_key_env must be an environment variable name"
                .to_owned()
        ),
        "config validation failures should be surfaced as terminal preflight errors"
    );
}

#[test]
fn provider_credential_check_adds_volcengine_auth_guidance_when_missing() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::VolcengineCoding;
    config.provider.api_key = None;
    config.provider.api_key_env = None;
    config.provider.oauth_access_token = None;
    config.provider.oauth_access_token_env = None;
    let auth_env_names = config.provider.auth_hint_env_names();
    let mut env = ScopedEnv::new();
    for env_name in auth_env_names {
        env.remove(env_name);
    }

    let check = provider_credential_check(&config);

    assert_eq!(check.name, "provider credentials");
    assert_eq!(check.level, OnboardCheckLevel::Warn);
    assert!(check.detail.contains("ARK_API_KEY"));
    assert!(check.detail.contains("Authorization: Bearer <ARK_API_KEY>"));
}

#[test]
fn preferred_api_key_env_default_ignores_invalid_configured_secret_literal() {
    let secret = "sk-live-direct-secret-value";
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Openai;
    config.provider.api_key_env = Some(secret.to_owned());

    let default_env = preferred_api_key_env_default(&config);

    assert_eq!(
        default_env, "OPENAI_CODEX_OAUTH_TOKEN",
        "invalid configured credential env values should fall back to the provider's safe onboarding default instead of being reused as the interactive prompt default"
    );
    assert!(
        !default_env.contains(secret),
        "prompt defaults must never echo the rejected secret-like value"
    );
}

#[test]
fn build_onboarding_success_summary_does_not_echo_invalid_credential_env_value() {
    let secret = "sk-live-direct-secret-value";
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Openai;
    config.provider.api_key_env = Some(secret.to_owned());

    let summary = build_onboarding_success_summary(Path::new("/tmp/loongclaw.toml"), &config, None);
    let credential = summary
        .credential
        .expect("summary should still describe the configured credential lane");

    assert_eq!(
        credential.value, "environment variable",
        "success summary should redact invalid configured env pointers instead of inventing a provider default binding"
    );
    assert!(
        !credential.value.contains(secret),
        "success summary must never echo invalid secret-like env input: {credential:#?}"
    );
}

#[test]
fn apply_selected_api_key_env_routes_openai_oauth_env_to_oauth_binding() {
    let mut provider = mvp::config::ProviderConfig {
        kind: mvp::config::ProviderKind::Openai,
        api_key: Some(SecretRef::Env {
            env: "OPENAI_API_KEY".to_owned(),
        }),
        ..mvp::config::ProviderConfig::default()
    };

    apply_selected_api_key_env(&mut provider, "OPENAI_CODEX_OAUTH_TOKEN".to_owned());

    assert_eq!(
        provider.oauth_access_token,
        Some(SecretRef::Env {
            env: "OPENAI_CODEX_OAUTH_TOKEN".to_owned(),
        })
    );
    assert_eq!(
        provider.api_key_env, None,
        "switching to the OpenAI oauth env should clear the stale api-key env binding"
    );
    assert_eq!(provider.api_key, None);
}

#[test]
fn apply_selected_api_key_env_routes_unknown_openai_env_to_api_key_binding() {
    let mut provider = mvp::config::ProviderConfig {
        kind: mvp::config::ProviderKind::Openai,
        oauth_access_token: Some(SecretRef::Env {
            env: "OPENAI_CODEX_OAUTH_TOKEN".to_owned(),
        }),
        ..mvp::config::ProviderConfig::default()
    };

    apply_selected_api_key_env(&mut provider, "OPENAI_ALT_BEARER".to_owned());

    assert_eq!(
        provider.api_key,
        Some(SecretRef::Env {
            env: "OPENAI_ALT_BEARER".to_owned(),
        }),
        "unknown env names should stay on the explicit api-key field instead of being silently rebound to oauth"
    );
    assert_eq!(
        provider.oauth_access_token_env, None,
        "switching to a custom env name should clear the stale oauth binding"
    );
    assert_eq!(provider.oauth_access_token, None);
}

#[test]
fn provider_matches_for_review_ignores_credential_field_explicitness() {
    let current = mvp::config::ProviderConfig {
        kind: mvp::config::ProviderKind::Openai,
        model: "gpt-4.1".to_owned(),
        api_key: Some(SecretRef::Inline("inline-secret".to_owned())),
        ..mvp::config::ProviderConfig::default()
    };

    let mut api_key_env_update = current.clone();
    apply_selected_api_key_env(&mut api_key_env_update, "OPENAI_API_KEY".to_owned());
    assert_eq!(
        api_key_env_update.api_key,
        Some(SecretRef::Env {
            env: "OPENAI_API_KEY".to_owned(),
        })
    );
    assert!(!api_key_env_update.api_key_env_explicit);
    assert!(
        provider_matches_for_review(&current, &api_key_env_update),
        "review matching should ignore credential binding rewrites when the provider identity is otherwise unchanged"
    );

    let mut oauth_env_update = current.clone();
    apply_selected_api_key_env(&mut oauth_env_update, "OPENAI_CODEX_OAUTH_TOKEN".to_owned());
    assert_eq!(
        oauth_env_update.oauth_access_token,
        Some(SecretRef::Env {
            env: "OPENAI_CODEX_OAUTH_TOKEN".to_owned(),
        })
    );
    assert!(!oauth_env_update.oauth_access_token_env_explicit);
    assert!(
        provider_matches_for_review(&current, &oauth_env_update),
        "review matching should ignore credential binding rewrites when the provider identity is otherwise unchanged"
    );
}

#[test]
fn apply_selected_system_prompt_restore_uses_rendered_native_prompt() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.cli.system_prompt = "custom review prompt".to_owned();
    config.cli.system_prompt_addendum = Some("Prefer concrete remediation steps.".to_owned());
    let expected = config.cli.rendered_native_system_prompt();

    apply_selected_system_prompt(&mut config, SystemPromptSelection::RestoreBuiltIn);

    assert_eq!(
        config.cli.system_prompt, expected,
        "restoring the built-in prompt should respect the active native prompt rendering inputs"
    );
}

#[test]
fn accepted_non_interactive_warnings_do_not_depend_on_display_text() {
    let check = OnboardCheck {
        name: "provider model probe",
        level: OnboardCheckLevel::Warn,
        detail: "display text changed".to_owned(),
        non_interactive_warning_policy:
            OnboardNonInteractiveWarningPolicy::AcceptedBySkipModelProbe,
    };
    let options = OnboardCommandOptions {
        output: None,
        force: false,
        non_interactive: true,
        accept_risk: true,
        provider: None,
        model: None,
        api_key_env: None,
        web_search_provider: None,
        web_search_api_key_env: None,
        personality: None,
        memory_profile: None,
        system_prompt: None,
        skip_model_probe: true,
    };

    assert!(
        is_explicitly_accepted_non_interactive_warning(&check, &options),
        "non-interactive warning acceptance should follow structured policy rather than fragile display strings"
    );
}

#[test]
fn apply_selected_web_search_credential_formats_env_reference() {
    let mut config = mvp::config::LoongClawConfig::default();

    apply_selected_web_search_credential(
        &mut config,
        mvp::config::WEB_SEARCH_PROVIDER_TAVILY,
        WebSearchCredentialSelection::UseEnv("TEAM_TAVILY_KEY".to_owned()),
    );

    assert_eq!(
        config.tools.web_search.tavily_api_key.as_deref(),
        Some("${TEAM_TAVILY_KEY}")
    );
}

#[test]
fn recommend_web_search_provider_from_available_credentials_prefers_unique_ready_provider() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.tools.web_search.perplexity_api_key = Some("${PERPLEXITY_API_KEY}".to_owned());

    let mut env = ScopedEnv::new();
    env.set("PERPLEXITY_API_KEY", "perplexity-test-token");

    let recommendation = recommend_web_search_provider_from_available_credentials(&config)
        .expect("a unique ready provider should be recommended");

    assert_eq!(
        recommendation.provider,
        mvp::config::WEB_SEARCH_PROVIDER_PERPLEXITY
    );
    assert_eq!(
        recommendation.source,
        WebSearchProviderRecommendationSource::DetectedCredential
    );
    assert!(
        recommendation.reason.contains("Perplexity Search"),
        "recommendation reason should identify the provider that already has a ready credential: {recommendation:?}"
    );
}

#[test]
fn recommend_web_search_provider_from_available_credentials_returns_none_when_multiple_ready() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.tools.web_search.tavily_api_key = Some("${TAVILY_API_KEY}".to_owned());
    config.tools.web_search.perplexity_api_key = Some("${PERPLEXITY_API_KEY}".to_owned());

    let mut env = ScopedEnv::new();
    env.set("TAVILY_API_KEY", "tavily-test-token");
    env.set("PERPLEXITY_API_KEY", "perplexity-test-token");

    let recommendation = recommend_web_search_provider_from_available_credentials(&config);

    assert_eq!(
        recommendation, None,
        "multiple ready providers should fall back to the environment heuristic instead of relying on an arbitrary hidden priority"
    );
}

#[test]
fn explicit_web_search_provider_override_prefers_cli_option_over_env() {
    let options = OnboardCommandOptions {
        output: None,
        force: false,
        non_interactive: false,
        accept_risk: true,
        provider: None,
        model: None,
        api_key_env: None,
        web_search_provider: Some("exa".to_owned()),
        web_search_api_key_env: None,
        personality: None,
        memory_profile: None,
        system_prompt: None,
        skip_model_probe: false,
    };
    let mut env = ScopedEnv::new();
    env.set("LOONGCLAW_WEB_SEARCH_PROVIDER", "tavily");

    let recommendation = explicit_web_search_provider_override(&options)
        .expect("cli override should parse")
        .expect("cli override should win");

    assert_eq!(
        recommendation.provider,
        mvp::config::WEB_SEARCH_PROVIDER_EXA
    );
    assert_eq!(
        recommendation.source,
        WebSearchProviderRecommendationSource::ExplicitCli
    );
}

#[test]
fn render_web_search_provider_selection_screen_uses_actual_default_provider_in_footer() {
    let config = mvp::config::LoongClawConfig::default();
    let current_provider = mvp::config::WEB_SEARCH_PROVIDER_DUCKDUCKGO;
    let recommended_provider = mvp::config::WEB_SEARCH_PROVIDER_TAVILY;
    let current_provider_label = web_search_provider_display_name(current_provider);
    let recommended_provider_label = web_search_provider_display_name(recommended_provider);
    let footer_description = format!("keep {current_provider_label}");
    let expected_footer = render_default_choice_footer_line("Enter", footer_description.as_str());
    let lines = render_web_search_provider_selection_screen_lines_with_style(
        &config,
        recommended_provider,
        current_provider,
        "found a ready credential",
        GuidedPromptPath::NativePromptPack,
        80,
        false,
    );

    assert!(
        lines
            .iter()
            .any(|line| line == &format!("- current provider: {current_provider_label}")),
        "web search provider screen should show the current provider separately: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == &format!("- recommended provider: {recommended_provider_label}")),
        "web search provider screen should show the recommendation separately: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == &expected_footer),
        "web search provider footer should describe the real Enter default instead of the recommendation: {lines:#?}"
    );
}

#[test]
fn resolve_effective_web_search_default_provider_keeps_explicit_non_interactive_provider() {
    let options = OnboardCommandOptions {
        output: None,
        force: false,
        non_interactive: true,
        accept_risk: true,
        provider: None,
        model: None,
        api_key_env: None,
        web_search_provider: Some("tavily".to_owned()),
        web_search_api_key_env: None,
        personality: None,
        memory_profile: None,
        system_prompt: None,
        skip_model_probe: false,
    };
    let config = mvp::config::LoongClawConfig::default();
    let recommendation = WebSearchProviderRecommendation {
        provider: mvp::config::WEB_SEARCH_PROVIDER_TAVILY,
        reason: "set by --web-search-provider".to_owned(),
        source: WebSearchProviderRecommendationSource::ExplicitCli,
    };

    let selected =
        resolve_effective_web_search_default_provider(&options, &config, &recommendation);

    assert_eq!(
        selected,
        mvp::config::WEB_SEARCH_PROVIDER_TAVILY,
        "non-interactive onboarding should keep an explicit web-search provider choice instead of silently falling back"
    );
}

#[test]
fn resolve_effective_web_search_default_provider_falls_back_for_detected_tavily_without_credential()
{
    let options = OnboardCommandOptions {
        output: None,
        force: false,
        non_interactive: true,
        accept_risk: true,
        provider: None,
        model: None,
        api_key_env: None,
        web_search_provider: None,
        web_search_api_key_env: None,
        personality: None,
        memory_profile: None,
        system_prompt: None,
        skip_model_probe: false,
    };
    let config = mvp::config::LoongClawConfig::default();
    let recommendation = WebSearchProviderRecommendation {
        provider: mvp::config::WEB_SEARCH_PROVIDER_TAVILY,
        reason: "domestic locale or timezone was detected".to_owned(),
        source: WebSearchProviderRecommendationSource::DetectedSignals,
    };

    let selected =
        resolve_effective_web_search_default_provider(&options, &config, &recommendation);

    assert_eq!(
        selected,
        mvp::config::WEB_SEARCH_PROVIDER_DUCKDUCKGO,
        "detected Tavily recommendations should still fall back to the key-free provider in non-interactive mode when no Tavily credential is ready"
    );
}

#[test]
fn validate_select_one_state_rejects_empty_options() {
    let error = validate_select_one_state(0, None)
        .expect_err("select_one should reject empty option lists before prompting");

    assert!(
        error.contains("no selection options"),
        "empty option lists should return a clear error: {error}"
    );
}

#[test]
fn validate_select_one_state_rejects_out_of_bounds_default() {
    let error = validate_select_one_state(2, Some(2))
        .expect_err("select_one should reject a default index that is outside the option list");

    assert!(
        error.contains("default selection index"),
        "invalid default index should be reported clearly: {error}"
    );
}

#[test]
fn default_choice_footer_avoids_bracket_default_syntax() {
    assert_eq!(
        render_default_choice_footer_line("1", "keep current setup"),
        "press Enter to use default 1, keep current setup"
    );
}

#[test]
fn prompt_with_default_text_avoids_bracket_default_syntax() {
    assert_eq!(
        render_prompt_with_default_text("Setup path", "1"),
        "Setup path (default: 1): "
    );
}

#[test]
fn render_onboard_option_lines_avoid_bracketed_choice_tokens() {
    let lines = render_onboard_option_lines(
        &[OnboardScreenOption {
            key: "1".to_owned(),
            label: "Keep current setup".to_owned(),
            detail_lines: vec!["reuse the detected setup".to_owned()],
            recommended: true,
        }],
        80,
    );

    assert!(
        lines
            .iter()
            .any(|line| line.contains("1) Keep current setup (recommended)")),
        "choice rows should present plain option markers instead of bracket wrappers: {lines:#?}"
    );
    assert!(
        lines.iter().all(|line| !line.contains("[1]")),
        "choice rows should not imply that brackets are part of the expected input syntax: {lines:#?}"
    );
}

#[test]
fn render_onboard_option_lines_align_wrapped_labels_with_option_prefix() {
    let lines = render_onboard_option_lines(
        &[OnboardScreenOption {
            key: "friendly_collab".to_owned(),
            label: "friendly collab keeps longer wrapped labels aligned".to_owned(),
            detail_lines: Vec::new(),
            recommended: false,
        }],
        28,
    );
    let continuation = lines
        .iter()
        .find(|line| line.starts_with(' ') && !line.trim().is_empty())
        .expect("wrapped option labels should emit a continuation line");

    assert!(
        continuation.starts_with(
            &" ".repeat(
                render_onboard_option_prefix("friendly_collab")
                    .chars()
                    .count()
            )
        ),
        "wrapped option labels should continue under the label text instead of snapping back to a fixed indent: {lines:#?}"
    );
}

#[test]
fn interactive_entry_screen_omits_static_options_when_selection_widget_handles_choices() {
    let options = recommended_import_entry_options();
    let lines = render_onboard_entry_interactive_screen_lines_with_style(
        crate::migration::CurrentSetupState::Absent,
        None,
        &[],
        &options,
        None,
        80,
        false,
    );

    assert!(
        lines
            .iter()
            .any(|line| line == crate::onboard_cli::presentation::entry_choice_section_heading()),
        "interactive entry screen should keep the section heading even when the chooser renders options separately: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("Continue current setup")),
        "interactive entry screen should not duplicate option labels before the selection widget renders them: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("press Enter to use default")),
        "interactive entry screen should omit the redundant static default footer: {lines:#?}"
    );
}

#[test]
fn interactive_starting_point_screen_omits_static_options_when_selection_widget_handles_choices() {
    let candidate = ImportCandidate {
        source_kind: crate::migration::ImportSourceKind::CodexConfig,
        source: "Codex config at ~/.codex/config.toml".to_owned(),
        config: mvp::config::LoongClawConfig::default(),
        surfaces: Vec::new(),
        domains: Vec::new(),
        channel_candidates: Vec::new(),
        workspace_guidance: Vec::new(),
    };
    let lines = render_starting_point_selection_header_lines_with_style(&[candidate], 80, false);

    assert!(
        lines
            .iter()
            .any(|line| line == crate::onboard_cli::presentation::starting_point_selection_title()),
        "interactive starting-point screen should keep the title even when choices render separately: {lines:#?}"
    );
    assert!(
        lines.iter().all(|line| !line.contains("(recommended)")),
        "interactive starting-point screen should not duplicate static choice rows before the selection widget renders them: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("press Enter to use default")),
        "interactive starting-point screen should omit the redundant static default footer: {lines:#?}"
    );
}

#[test]
fn interactive_existing_config_write_screen_omits_static_options_when_selection_widget_handles_choices()
 {
    let lines = render_existing_config_write_header_lines_with_style(
        "/tmp/loongclaw-config.toml",
        80,
        false,
    );

    assert!(
        lines.iter().any(|line| line == "existing config found"),
        "interactive existing-config screen should keep its heading: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("Replace existing config")),
        "interactive existing-config screen should let the selection widget own the actual options: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("press Enter to use default")),
        "interactive existing-config screen should omit the redundant static default footer: {lines:#?}"
    );
}

#[test]
fn parse_select_one_input_accepts_custom_alias_for_custom_model_option() {
    let options = vec![
        SelectOption {
            label: "gpt-5.2".to_owned(),
            slug: "openai/gpt-5.2".to_owned(),
            description: String::new(),
            recommended: true,
        },
        SelectOption {
            label: "enter custom model id".to_owned(),
            slug: ONBOARD_CUSTOM_MODEL_OPTION_SLUG.to_owned(),
            description: String::new(),
            recommended: false,
        },
    ];

    assert_eq!(parse_select_one_input("custom", &options), Some(1));
    assert_eq!(
        parse_select_one_input(ONBOARD_CUSTOM_MODEL_OPTION_SLUG, &options),
        Some(1),
        "the internal sentinel may still appear in older scripted flows and should stay backward compatible"
    );
}

#[test]
fn render_select_one_invalid_input_message_hides_internal_custom_model_slug() {
    let options = vec![
        SelectOption {
            label: "gpt-5.2".to_owned(),
            slug: "openai/gpt-5.2".to_owned(),
            description: String::new(),
            recommended: true,
        },
        SelectOption {
            label: "enter custom model id".to_owned(),
            slug: ONBOARD_CUSTOM_MODEL_OPTION_SLUG.to_owned(),
            description: String::new(),
            recommended: false,
        },
    ];

    let message = render_select_one_invalid_input_message(&options);
    assert!(
        message.contains("custom"),
        "invalid-input help should surface a friendly custom alias: {message}"
    );
    assert!(
        !message.contains(ONBOARD_CUSTOM_MODEL_OPTION_SLUG),
        "invalid-input help must not leak the internal custom sentinel: {message}"
    );
}

#[test]
fn resolve_select_one_eof_returns_default_when_available() {
    let idx = resolve_select_one_eof(Some(1)).expect("EOF should fall back to the default");
    assert_eq!(idx, 1);
}

#[test]
fn resolve_select_one_eof_errors_when_selection_is_required() {
    let error = resolve_select_one_eof(None)
        .expect_err("EOF without a default should terminate instead of looping forever");

    assert!(
        error.contains("stdin closed"),
        "required selections should surface EOF as a terminal error: {error}"
    );
}

#[test]
fn shortcut_screen_footer_mentions_escape_cancel() {
    let lines =
        render_continue_current_setup_screen_lines(&mvp::config::LoongClawConfig::default(), 80);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("Esc") && line.contains("cancel")),
        "choice screens should teach the exit gesture explicitly: {lines:#?}"
    );
}

#[test]
fn shortcut_header_footer_mentions_escape_cancel() {
    let lines = render_onboard_shortcut_header_lines_with_style(
        OnboardShortcutKind::CurrentSetup,
        &mvp::config::LoongClawConfig::default(),
        None,
        80,
        false,
    );

    assert!(
        lines
            .iter()
            .any(|line| line.contains("Esc") && line.contains("cancel")),
        "header-only shortcut screens should keep the exit gesture visible before the chooser opens: {lines:#?}"
    );
}

#[test]
fn detected_shortcut_snapshot_wraps_starting_point_like_review_rows() {
    let config = mvp::config::LoongClawConfig::default();
    let import_source =
        "Codex config at /very/long/path/to/a/workspace/with/a/deeply/nested/config.toml";
    let expected_label = onboard_starting_point_label(None, import_source);
    let expected_lines =
        mvp::presentation::render_wrapped_text_line("- starting point: ", &expected_label, 48);
    let lines = render_onboard_shortcut_screen_lines_with_style(
        OnboardShortcutKind::DetectedSetup,
        &config,
        Some(import_source),
        48,
        false,
    );

    for expected_line in expected_lines {
        assert!(
            lines.iter().any(|line| line == &expected_line),
            "detected shortcut snapshots should wrap the starting-point row with the same helper used by the review digest: {lines:#?}"
        );
    }
}

#[test]
fn preflight_summary_screen_footer_mentions_escape_cancel() {
    let checks = vec![OnboardCheck {
        name: "provider model probe",
        level: OnboardCheckLevel::Warn,
        detail: "catalog probe failed".to_owned(),
        non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
    }];

    let lines = render_preflight_summary_screen_lines(&checks, 80);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("Esc") && line.contains("cancel")),
        "interactive preflight review should teach the exit gesture explicitly: {lines:#?}"
    );
}

#[test]
fn preflight_summary_uses_explicit_model_guidance_for_reviewed_auto_failures() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Deepseek;
    config.provider.model = "auto".to_owned();

    let check =
        provider_model_probe_failure_check(&config, "provider rejected the model list".to_owned());
    let lines = render_preflight_summary_screen_lines(&[check], 80);

    assert!(
        lines.iter().any(|line| {
            line.contains("rerun onboarding to choose a reviewed model")
                || line.contains("set provider.model / preferred_models explicitly")
        }),
        "reviewed auto-model failures should keep the explicit-model remediation visible in the summary: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("--skip-model-probe")),
        "reviewed auto-model failures should not suggest --skip-model-probe because that contradicts the explicit-model recovery path: {lines:#?}"
    );
}

#[test]
fn preflight_summary_uses_explicit_model_only_guidance_without_reviewed_default() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Custom;
    config.provider.model = "auto".to_owned();

    let check =
        provider_model_probe_failure_check(&config, "provider rejected the model list".to_owned());
    let lines = render_preflight_summary_screen_lines(&[check], 80);

    assert!(
        lines.iter().any(|line| {
            line == crate::onboard_cli::presentation::preflight_explicit_model_only_rerun_hint()
        }),
        "providers without a reviewed model should keep the summary hint aligned with the explicit-model-only recovery path: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("choose a reviewed model")),
        "providers without a reviewed model should not advertise a reviewed-model recovery path that does not exist: {lines:#?}"
    );
}

#[test]
fn preflight_summary_omits_skip_model_probe_rerun_hint_after_probe_is_already_skipped() {
    let lines = render_preflight_summary_screen_lines(
        &[OnboardCheck {
            name: "provider model probe",
            level: OnboardCheckLevel::Warn,
            detail: "skipped by --skip-model-probe".to_owned(),
            non_interactive_warning_policy:
                OnboardNonInteractiveWarningPolicy::AcceptedBySkipModelProbe,
        }],
        80,
    );

    assert!(
        lines.iter().all(|line| {
            line.as_str() != crate::onboard_cli::presentation::preflight_probe_rerun_hint()
        }),
        "preflight should not suggest rerunning with --skip-model-probe after the current run already skipped the probe: {lines:#?}"
    );
}

#[test]
fn entry_screen_footer_mentions_escape_cancel() {
    let options = build_onboard_entry_options(crate::migration::CurrentSetupState::Absent, &[]);
    let lines = render_onboard_entry_screen_lines(
        crate::migration::CurrentSetupState::Absent,
        None,
        &[],
        &options,
        None,
        80,
    );

    assert!(
        lines
            .iter()
            .any(|line| line.contains("Esc") && line.contains("cancel")),
        "interactive entry selection should teach the exit gesture explicitly: {lines:#?}"
    );
}

#[test]
fn write_confirmation_screen_footer_mentions_escape_cancel() {
    let lines = render_write_confirmation_screen_lines("/tmp/loongclaw.toml", false, 80);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("Esc") && line.contains("cancel")),
        "write confirmation should teach the exit gesture explicitly: {lines:#?}"
    );
}

#[test]
fn append_escape_cancel_hint_dedupes_case_insensitively() {
    let footer_lines = append_escape_cancel_hint(vec![
        "- press esc then enter to cancel onboarding".to_owned(),
    ]);

    assert_eq!(
        footer_lines,
        vec!["- press esc then enter to cancel onboarding".to_owned()],
        "case-only changes should not duplicate the escape cancel footer: {footer_lines:#?}"
    );
}

#[test]
fn model_selection_screen_tells_users_to_type_auto_for_fallbacks() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Minimax;
    config.provider.model = "auto".to_owned();
    config.provider.preferred_models = vec!["MiniMax-M2.5".to_owned()];

    let lines = render_model_selection_screen_lines_with_default(&config, "MiniMax-M2.7", 80);
    let rendered = lines.join("\n");

    assert!(
        rendered.contains("type `auto`")
            && rendered.contains("configured preferred fallbacks first")
            && rendered.contains("MiniMax-M2.5"),
        "explicit prefill flows should tell users to type `auto` when they want configured fallback behavior: {lines:#?}"
    );
    assert!(
        !rendered.contains("leave `auto`"),
        "explicit prefill flows should not imply Enter keeps `auto`: {lines:#?}"
    );
}

#[test]
fn select_non_interactive_starting_config_uses_sorted_detected_candidate_priority() {
    let codex_candidate = import_candidate_with_domain_status(
        crate::migration::ImportSourceKind::CodexConfig,
        "Codex config at ~/.codex/config.toml",
        [(
            crate::migration::SetupDomainKind::Provider,
            crate::migration::PreviewStatus::Ready,
        )],
    );
    let environment_candidate = import_candidate_with_domain_status(
        crate::migration::ImportSourceKind::Environment,
        "your current environment",
        [
            (
                crate::migration::SetupDomainKind::Provider,
                crate::migration::PreviewStatus::Ready,
            ),
            (
                crate::migration::SetupDomainKind::Channels,
                crate::migration::PreviewStatus::Ready,
            ),
            (
                crate::migration::SetupDomainKind::WorkspaceGuidance,
                crate::migration::PreviewStatus::Ready,
            ),
        ],
    );
    let all_candidates = vec![codex_candidate, environment_candidate];

    let selection = select_non_interactive_starting_config(
        crate::migration::CurrentSetupState::Absent,
        &recommended_import_entry_options(),
        None,
        all_candidates.clone(),
        &all_candidates,
    );

    assert_eq!(
        selection
            .review_candidate
            .as_ref()
            .map(|candidate| candidate.source_kind),
        Some(crate::migration::ImportSourceKind::Environment),
        "non-interactive onboarding should reuse the same sorted detected-candidate priority as the interactive chooser: {selection:#?}"
    );
}

#[test]
fn format_backup_timestamp_at_matches_existing_filename_shape() {
    let timestamp = time::macros::datetime!(2026-03-14 01:23:45 +08:00);

    let formatted = match format_backup_timestamp_at(timestamp) {
        Ok(value) => value,
        Err(error) => panic!("formatting should succeed: {error}"),
    };

    assert_eq!(formatted, "20260314-012345");
}

#[test]
fn resolve_backup_path_at_uses_formatted_timestamp() {
    let original = Path::new("/tmp/loongclaw.toml");
    let timestamp = time::macros::datetime!(2026-03-14 01:23:45 +08:00);

    let path = match resolve_backup_path_at(original, timestamp) {
        Ok(value) => value,
        Err(error) => panic!("backup path should resolve: {error}"),
    };

    assert_eq!(
        path,
        PathBuf::from("/tmp/loongclaw.toml.bak-20260314-012345")
    );
}

#[test]
fn rollback_removes_partial_first_write_config() {
    let output_path = std::env::temp_dir().join(format!(
        "loongclaw-first-write-rollback-{}.toml",
        std::process::id()
    ));
    std::fs::write(&output_path, "partial = true\n").expect("write partial config");

    let recovery = OnboardWriteRecovery {
        output_preexisted: false,
        backup_path: None,
        keep_backup_on_success: false,
    };

    recovery
        .rollback(&output_path)
        .expect("first-write rollback should succeed");

    assert!(
        !output_path.exists(),
        "first-write rollback should remove the partially written config"
    );
}

#[test]
fn rollback_failure_produces_compound_error_message() {
    let output_path = std::env::temp_dir().join(format!(
        "loongclaw-compound-rollback-{}.toml",
        std::process::id()
    ));
    std::fs::write(&output_path, "original = true\n").expect("write original config");

    // Point backup to a non-existent directory so rollback copy fails.
    let recovery = OnboardWriteRecovery {
        output_preexisted: true,
        backup_path: Some(
            std::env::temp_dir()
                .join("nonexistent-rollback-dir")
                .join("backup.toml"),
        ),
        keep_backup_on_success: false,
    };

    let error =
        rollback_onboard_write_failure(&output_path, &recovery, "config write failed: disk full");

    assert!(
        error.contains("config write failed: disk full"),
        "compound error should include the original failure"
    );
    assert!(
        error.contains("additionally failed to restore original config"),
        "compound error should include the rollback failure"
    );

    // Cleanup
    let _ = std::fs::remove_file(&output_path);
}

#[test]
fn rollback_success_returns_original_failure_only() {
    let dir =
        std::env::temp_dir().join(format!("loongclaw-rollback-success-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");

    let output_path = dir.join("config.toml");
    let backup_path = dir.join("config.toml.bak");
    std::fs::write(&output_path, "modified = true\n").expect("write modified config");
    std::fs::write(&backup_path, "original = true\n").expect("write backup config");

    let recovery = OnboardWriteRecovery {
        output_preexisted: true,
        backup_path: Some(backup_path),
        keep_backup_on_success: false,
    };

    let error = rollback_onboard_write_failure(&output_path, &recovery, "verification failed");

    assert_eq!(
        error, "verification failed",
        "when rollback succeeds, only the original failure should be returned"
    );
    assert_eq!(
        std::fs::read_to_string(&output_path).unwrap(),
        "original = true\n",
        "rollback should restore the original config"
    );

    // Cleanup
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prompt_addendum_screen_mentions_single_line_terminal_input() {
    let lines =
        render_prompt_addendum_selection_screen_lines(&mvp::config::LoongClawConfig::default(), 80);

    assert!(
        lines.iter().any(|line| line == "- single-line input only"),
        "prompt addendum screen should keep the terminal input note concise: {lines:#?}"
    );
}

#[test]
fn system_prompt_screen_mentions_single_line_terminal_input() {
    let lines =
        render_system_prompt_selection_screen_lines(&mvp::config::LoongClawConfig::default(), 80);

    assert!(
        lines.iter().any(|line| line == "- single-line input only"),
        "system prompt screen should keep the terminal input note concise: {lines:#?}"
    );
}
