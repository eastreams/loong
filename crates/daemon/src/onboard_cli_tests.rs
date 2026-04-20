use super::*;
use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, MutexGuard};

use crate::test_support::ScopedEnv;

struct TestOnboardUi {
    inputs: VecDeque<String>,
}

impl TestOnboardUi {
    fn with_inputs(inputs: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            inputs: inputs.into_iter().map(Into::into).collect(),
        }
    }
}

struct SelectOnlyTestUi {
    inputs: VecDeque<String>,
}

impl SelectOnlyTestUi {
    fn with_inputs(inputs: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            inputs: inputs.into_iter().map(Into::into).collect(),
        }
    }
}

struct AllowEmptyOnlyTestUi {
    inputs: VecDeque<String>,
}

impl AllowEmptyOnlyTestUi {
    fn with_inputs(inputs: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            inputs: inputs.into_iter().map(Into::into).collect(),
        }
    }
}

fn interactive_onboard_options() -> OnboardCommandOptions {
    OnboardCommandOptions {
        output: None,
        force: false,
        non_interactive: false,
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
    }
}

fn onboard_test_context() -> OnboardRuntimeContext {
    OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>())
}

fn uuid_shaped_secret_fixture() -> String {
    let first = "9f479837";
    let second = "0a12";
    let third = "4b56";
    let fourth = "89ab";
    let fifth = "cdef01234567";
    format!("{first}-{second}-{third}-{fourth}-{fifth}")
}

impl OnboardUi for TestOnboardUi {
    fn print_line(&mut self, _line: &str) -> CliResult<()> {
        Ok(())
    }

    fn prompt_with_default(&mut self, _label: &str, default: &str) -> CliResult<String> {
        let value =
            ensure_onboard_input_not_cancelled(self.inputs.pop_front().unwrap_or_default())?;
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Ok(default.to_owned());
        }
        Ok(trimmed.to_owned())
    }

    fn prompt_required(&mut self, _label: &str) -> CliResult<String> {
        let value = self
            .inputs
            .pop_front()
            .ok_or_else(|| "missing required test input".to_owned())?;
        Ok(ensure_onboard_input_not_cancelled(value)?.trim().to_owned())
    }

    fn prompt_allow_empty(&mut self, label: &str) -> CliResult<String> {
        match self.inputs.front() {
            Some(value)
                if label == PREINSTALLED_SKILLS_PROMPT_LABEL
                    && parse_preinstalled_skill_selection(value.as_str()).is_err() =>
            {
                Ok(String::new())
            }
            Some(_) => {
                let value = self
                    .inputs
                    .pop_front()
                    .ok_or_else(|| "missing allow-empty test input".to_owned())?;
                Ok(ensure_onboard_input_not_cancelled(value)?.trim().to_owned())
            }
            None if label == PREINSTALLED_SKILLS_PROMPT_LABEL => Ok(String::new()),
            None => Err("missing allow-empty test input".to_owned()),
        }
    }

    fn prompt_confirm(&mut self, _message: &str, default: bool) -> CliResult<bool> {
        let Some(value) = self.inputs.pop_front() else {
            return Ok(default);
        };
        let value = ensure_onboard_input_not_cancelled(value)?;
        let value = value.trim().to_ascii_lowercase();
        if value.is_empty() {
            return Ok(default);
        }
        Ok(matches!(value.as_str(), "y" | "yes"))
    }

    fn select_one(
        &mut self,
        _label: &str,
        options: &[SelectOption],
        default: Option<usize>,
        _interaction_mode: SelectInteractionMode,
    ) -> CliResult<usize> {
        let default = validate_select_one_state(options.len(), default)?;
        match self.inputs.pop_front() {
            Some(value) => {
                let value = ensure_onboard_input_not_cancelled(value)?;
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    return default.ok_or_else(|| "no default for required selection".to_owned());
                }
                if let Ok(n) = trimmed.parse::<usize>() {
                    if n >= 1 && n <= options.len() {
                        return Ok(n - 1);
                    }
                    return Err(format!(
                        "test selection {n} out of range 1..={}",
                        options.len()
                    ));
                }
                parse_select_one_input(trimmed, options)
                    .ok_or_else(|| format!("invalid test selection input: {trimmed}"))
            }
            None => default.ok_or_else(|| "missing test input for required selection".to_owned()),
        }
    }
}

impl OnboardUi for SelectOnlyTestUi {
    fn print_line(&mut self, _line: &str) -> CliResult<()> {
        Ok(())
    }

    fn prompt_with_default(&mut self, _label: &str, _default: &str) -> CliResult<String> {
        Err("test expected interactive select widget instead of prompt_with_default".to_owned())
    }

    fn prompt_required(&mut self, _label: &str) -> CliResult<String> {
        Err("test expected interactive select widget instead of prompt_required".to_owned())
    }

    fn prompt_allow_empty(&mut self, label: &str) -> CliResult<String> {
        if label == PREINSTALLED_SKILLS_PROMPT_LABEL {
            return Ok(String::new());
        }
        Err("test expected interactive select widget instead of prompt_allow_empty".to_owned())
    }

    fn prompt_confirm(&mut self, _message: &str, _default: bool) -> CliResult<bool> {
        Err("test expected interactive select widget instead of prompt_confirm".to_owned())
    }

    fn select_one(
        &mut self,
        _label: &str,
        options: &[SelectOption],
        default: Option<usize>,
        _interaction_mode: SelectInteractionMode,
    ) -> CliResult<usize> {
        let default = validate_select_one_state(options.len(), default)?;
        match self.inputs.pop_front() {
            Some(value) => {
                let value = ensure_onboard_input_not_cancelled(value)?;
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    return default.ok_or_else(|| "no default for required selection".to_owned());
                }
                if let Ok(n) = trimmed.parse::<usize>() {
                    if n >= 1 && n <= options.len() {
                        return Ok(n - 1);
                    }
                    return Err(format!(
                        "test selection {n} out of range 1..={}",
                        options.len()
                    ));
                }
                parse_select_one_input(trimmed, options)
                    .ok_or_else(|| format!("invalid test selection input: {trimmed}"))
            }
            None => default.ok_or_else(|| "missing test input for required selection".to_owned()),
        }
    }
}

impl OnboardUi for AllowEmptyOnlyTestUi {
    fn print_line(&mut self, _line: &str) -> CliResult<()> {
        Ok(())
    }

    fn prompt_with_default(&mut self, _label: &str, _default: &str) -> CliResult<String> {
        Err("test expected prompt_allow_empty instead of prompt_with_default".to_owned())
    }

    fn prompt_required(&mut self, _label: &str) -> CliResult<String> {
        Err("test expected prompt_allow_empty instead of prompt_required".to_owned())
    }

    fn prompt_allow_empty(&mut self, _label: &str) -> CliResult<String> {
        let value = self
            .inputs
            .pop_front()
            .ok_or_else(|| "missing allow-empty test input".to_owned())?;
        Ok(ensure_onboard_input_not_cancelled(value)?.trim().to_owned())
    }

    fn prompt_confirm(&mut self, _message: &str, _default: bool) -> CliResult<bool> {
        Err("test expected prompt_allow_empty instead of prompt_confirm".to_owned())
    }

    fn select_one(
        &mut self,
        _label: &str,
        _options: &[SelectOption],
        _default: Option<usize>,
        _interaction_mode: SelectInteractionMode,
    ) -> CliResult<usize> {
        Err("test expected prompt_allow_empty instead of select_one".to_owned())
    }
}

struct TestPromptLineReader {
    blocking_reads: VecDeque<OnboardPromptRead>,
    pending_lines: VecDeque<String>,
}

impl TestPromptLineReader {
    fn new(
        blocking_reads: impl IntoIterator<Item = OnboardPromptRead>,
        pending_lines: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            blocking_reads: blocking_reads.into_iter().collect(),
            pending_lines: pending_lines.into_iter().map(Into::into).collect(),
        }
    }
}

impl OnboardPromptLineReader for TestPromptLineReader {
    fn read_blocking_line(&mut self) -> CliResult<OnboardPromptRead> {
        Ok(self
            .blocking_reads
            .pop_front()
            .unwrap_or(OnboardPromptRead::Eof))
    }

    fn read_pending_line(&mut self) -> CliResult<Option<String>> {
        Ok(self.pending_lines.pop_front())
    }
}

struct BrowserCompanionEnvGuard {
    _lock: MutexGuard<'static, ()>,
    saved_ready: Option<OsString>,
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
        let key = "LOONG_BROWSER_COMPANION_READY";
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

struct PasteDrainWindowEnvGuard {
    _lock: MutexGuard<'static, ()>,
    saved_value: Option<OsString>,
}

impl PasteDrainWindowEnvGuard {
    fn set(value: Option<&str>) -> Self {
        let lock = crate::test_support::lock_daemon_test_environment();
        let saved_value = std::env::var_os(ONBOARD_PASTE_DRAIN_WINDOW_ENV);
        match value {
            Some(value) => set_browser_companion_env_var(ONBOARD_PASTE_DRAIN_WINDOW_ENV, value),
            None => remove_browser_companion_env_var(ONBOARD_PASTE_DRAIN_WINDOW_ENV),
        }
        Self {
            _lock: lock,
            saved_value,
        }
    }
}

impl Drop for PasteDrainWindowEnvGuard {
    fn drop(&mut self) {
        match &self.saved_value {
            Some(value) => {
                set_browser_companion_env_var(
                    ONBOARD_PASTE_DRAIN_WINDOW_ENV,
                    &value.to_string_lossy(),
                );
            }
            None => remove_browser_companion_env_var(ONBOARD_PASTE_DRAIN_WINDOW_ENV),
        }
    }
}

impl Drop for BrowserCompanionEnvGuard {
    fn drop(&mut self) {
        let key = "LOONG_BROWSER_COMPANION_READY";
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
        config: mvp::config::LoongConfig::default(),
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
    let mut config = mvp::config::LoongConfig::default();
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
    let mut config = mvp::config::LoongConfig::default();
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
    let mut config = mvp::config::LoongConfig::default();
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

    let mut config = mvp::config::LoongConfig::default();
    config.provider.api_key = Some(SecretRef::Inline("inline-openai-key".to_owned()));
    config.tools.browser_companion.enabled = true;
    config.tools.browser_companion.command =
        Some(crate::browser_companion_diagnostics::fake_browser_companion_version_command("1.5.0"));
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

    let mut config = mvp::config::LoongConfig::default();
    config.provider.api_key = Some(SecretRef::Inline("inline-openai-key".to_owned()));
    config.tools.browser_companion.enabled = true;
    config.tools.browser_companion.command =
        Some(crate::browser_companion_diagnostics::fake_browser_companion_version_command("1.5.0"));
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
    let mut config = mvp::config::LoongConfig::default();
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
    let mut config = mvp::config::LoongConfig::default();
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
    let mut config = mvp::config::LoongConfig::default();
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
    let mut config = mvp::config::LoongConfig::default();
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
    let mut config = mvp::config::LoongConfig::default();
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
    let mut config = mvp::config::LoongConfig::default();
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
    let mut config = mvp::config::LoongConfig::default();
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
    let mut config = mvp::config::LoongConfig::default();
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
    let mut config = mvp::config::LoongConfig::default();
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
    let mut config = mvp::config::LoongConfig::default();
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
fn provider_credential_check_accepts_x_api_key_provider_env_credentials() {
    let mut env = ScopedEnv::new();
    env.set("ANTHROPIC_API_KEY", "test-anthropic-key");
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Anthropic;
    config.provider.api_key = None;
    config.provider.api_key_env = None;
    config.provider.oauth_access_token = None;
    config.provider.oauth_access_token_env = None;

    let check = provider_credential_check(&config);

    assert_eq!(check.name, "provider credentials");
    assert_eq!(check.level, OnboardCheckLevel::Pass);
    assert!(check.detail.contains("ANTHROPIC_API_KEY is available"));
}

#[test]
fn provider_credential_check_passes_for_auth_optional_provider() {
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Ollama;
    config.provider.api_key = None;
    config.provider.api_key_env = None;
    config.provider.oauth_access_token = None;
    config.provider.oauth_access_token_env = None;

    let check = provider_credential_check(&config);

    assert_eq!(check.name, "provider credentials");
    assert_eq!(check.level, OnboardCheckLevel::Pass);
    assert!(check.detail.contains("optional for this provider"));
}

#[test]
fn preferred_api_key_env_default_ignores_invalid_configured_secret_literal() {
    let secret = "sk-live-direct-secret-value";
    let mut config = mvp::config::LoongConfig::default();
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
    let mut config = mvp::config::LoongConfig::default();
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
fn resolve_api_key_env_selection_accepts_explicit_clear_token_in_interactive_mode() {
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Openai;
    config.provider.api_key = Some(SecretRef::Inline("inline-secret".to_owned()));
    let mut ui = TestOnboardUi::with_inputs([":clear"]);
    let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

    let selected = resolve_api_key_env_selection(
        &OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: false,
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
        },
        &config,
        "OPENAI_API_KEY".to_owned(),
        GuidedPromptPath::NativePromptPack,
        &mut ui,
        &context,
    )
    .expect("resolve api key env selection");

    assert!(
        selected.is_empty(),
        "typing :clear should explicitly clear the api-key env selection instead of persisting the literal token: {selected:?}"
    );
}

#[test]
fn resolve_api_key_env_selection_reprompts_after_secret_literal_interactively() {
    let secret = "sk-live-direct-secret-value";
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Openai;
    let mut ui = TestOnboardUi::with_inputs([secret, "OPENAI_API_KEY"]);
    let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

    let selected = resolve_api_key_env_selection(
        &OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: false,
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
        },
        &config,
        "OPENAI_API_KEY".to_owned(),
        GuidedPromptPath::NativePromptPack,
        &mut ui,
        &context,
    )
    .expect("interactive credential selection should reprompt on invalid secret-like input");

    assert_eq!(
        selected, "OPENAI_API_KEY",
        "interactive onboarding should reject secret-like input and keep asking for an env var name"
    );
}

#[test]
fn resolve_api_key_env_selection_rejects_secret_literal_non_interactively() {
    let secret = "sk-live-direct-secret-value";
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Openai;
    let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
    let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

    let error = resolve_api_key_env_selection(
        &OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: true,
            accept_risk: true,
            provider: None,
            model: None,
            api_key_env: Some(secret.to_owned()),
            web_search_provider: None,
            web_search_api_key_env: None,
            personality: None,
            memory_profile: None,
            system_prompt: None,
            skip_model_probe: false,
        },
        &config,
        "OPENAI_API_KEY".to_owned(),
        GuidedPromptPath::NativePromptPack,
        &mut ui,
        &context,
    )
    .expect_err("non-interactive onboarding should reject secret-like env selections");

    assert!(
        error.contains("provider.api_key.env"),
        "the validation error should identify the bad field: {error}"
    );
    assert!(
        !error.contains(secret),
        "non-interactive validation must not echo the secret-like input: {error}"
    );
}

#[test]
fn resolve_api_key_env_selection_reprompts_after_uuid_secret_literal_interactively() {
    let secret = uuid_shaped_secret_fixture();
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::VolcengineCoding;
    let mut ui = TestOnboardUi::with_inputs([secret.as_str(), "ARK_API_KEY"]);
    let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

    let selected = resolve_api_key_env_selection(
        &OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: false,
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
        },
        &config,
        "ARK_API_KEY".to_owned(),
        GuidedPromptPath::NativePromptPack,
        &mut ui,
        &context,
    )
    .expect("uuid-shaped credential input should be rejected and reprompted");

    assert_eq!(selected, "ARK_API_KEY");
}

#[test]
fn resolve_api_key_env_selection_rejects_uuid_secret_literal_non_interactively() {
    let secret = uuid_shaped_secret_fixture();
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::VolcengineCoding;
    let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
    let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

    let error = resolve_api_key_env_selection(
        &OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: true,
            accept_risk: true,
            provider: None,
            model: None,
            api_key_env: Some(secret.clone()),
            web_search_provider: None,
            web_search_api_key_env: None,
            personality: None,
            memory_profile: None,
            system_prompt: None,
            skip_model_probe: false,
        },
        &config,
        "ARK_API_KEY".to_owned(),
        GuidedPromptPath::NativePromptPack,
        &mut ui,
        &context,
    )
    .expect_err("uuid-shaped env selections should be rejected non-interactively");

    assert!(error.contains("provider.api_key.env"));
    assert!(!error.contains(secret.as_str()));
}

#[test]
fn resolve_web_search_credential_selection_accepts_clear_token_interactively() {
    let mut config = mvp::config::LoongConfig::default();
    config.tools.web_search.default_provider = mvp::config::WEB_SEARCH_PROVIDER_TAVILY.to_owned();
    config.tools.web_search.tavily_api_key = Some("${TEAM_TAVILY_KEY}".to_owned());
    let mut ui = TestOnboardUi::with_inputs([":clear"]);
    let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());
    let options = OnboardCommandOptions {
        output: None,
        force: false,
        non_interactive: false,
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

    let selected = resolve_web_search_credential_selection(
        &options,
        &config,
        mvp::config::WEB_SEARCH_PROVIDER_TAVILY,
        GuidedPromptPath::NativePromptPack,
        false,
        &mut ui,
        &context,
    )
    .expect("resolve web search credential selection");

    assert_eq!(selected, WebSearchCredentialSelection::ClearConfigured);
}

#[test]
fn resolve_web_search_credential_selection_reprompts_after_secret_literal_interactively() {
    let mut config = mvp::config::LoongConfig::default();
    config.tools.web_search.default_provider = mvp::config::WEB_SEARCH_PROVIDER_TAVILY.to_owned();
    let mut ui = TestOnboardUi::with_inputs(["sk-live-direct-secret-value", "TEAM_TAVILY_KEY"]);
    let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());
    let options = OnboardCommandOptions {
        output: None,
        force: false,
        non_interactive: false,
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

    let selected = resolve_web_search_credential_selection(
        &options,
        &config,
        mvp::config::WEB_SEARCH_PROVIDER_TAVILY,
        GuidedPromptPath::NativePromptPack,
        false,
        &mut ui,
        &context,
    )
    .expect("interactive web search credential selection should reprompt");

    assert_eq!(
        selected,
        WebSearchCredentialSelection::UseEnv("TEAM_TAVILY_KEY".to_owned())
    );
}

#[test]
fn resolve_web_search_credential_selection_keeps_inline_secret_on_blank_input() {
    let mut config = mvp::config::LoongConfig::default();
    config.tools.web_search.default_provider = mvp::config::WEB_SEARCH_PROVIDER_TAVILY.to_owned();
    config.tools.web_search.tavily_api_key = Some("inline-web-secret".to_owned());
    let mut ui = TestOnboardUi::with_inputs([""]);
    let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());
    let options = OnboardCommandOptions {
        output: None,
        force: false,
        non_interactive: false,
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

    let selected = resolve_web_search_credential_selection(
        &options,
        &config,
        mvp::config::WEB_SEARCH_PROVIDER_TAVILY,
        GuidedPromptPath::NativePromptPack,
        false,
        &mut ui,
        &context,
    )
    .expect("blank input should keep current inline web search credential");

    assert_eq!(selected, WebSearchCredentialSelection::KeepCurrent);
}

#[test]
fn apply_selected_web_search_credential_formats_env_reference() {
    let mut config = mvp::config::LoongConfig::default();

    apply_selected_web_search_credential(
        &mut config,
        mvp::config::WEB_SEARCH_PROVIDER_TAVILY,
        WebSearchCredentialSelection::UseEnv("TEAM_TAVILY_KEY".to_owned()),
    )
    .expect("apply tavily web search credential");

    assert_eq!(
        config.tools.web_search.tavily_api_key.as_deref(),
        Some("${TEAM_TAVILY_KEY}")
    );
}

#[test]
fn apply_selected_web_search_credential_updates_firecrawl_field() {
    let mut config = mvp::config::LoongConfig::default();
    let provider = mvp::config::WEB_SEARCH_PROVIDER_FIRECRAWL;
    let credential_env = "TEAM_FIRECRAWL_KEY".to_owned();
    let selection = WebSearchCredentialSelection::UseEnv(credential_env);

    apply_selected_web_search_credential(&mut config, provider, selection)
        .expect("apply firecrawl web search credential");

    let configured_credential = config.tools.web_search.firecrawl_api_key.as_deref();
    assert_eq!(configured_credential, Some("${TEAM_FIRECRAWL_KEY}"));
}

#[test]
fn apply_selected_web_search_credential_rejects_unknown_provider() {
    let mut config = mvp::config::LoongConfig::default();
    let error = apply_selected_web_search_credential(
        &mut config,
        "unknown-provider",
        WebSearchCredentialSelection::UseEnv("TEAM_UNKNOWN_KEY".to_owned()),
    )
    .expect_err("reject unsupported web search provider");

    assert!(error.contains("unsupported web.search provider"));
    assert!(error.contains("unknown-provider"));
}

fn clear_web_search_credential_envs(env: &mut ScopedEnv) {
    for descriptor in mvp::config::web_search_provider_descriptors() {
        if let Some(default_env) = descriptor.default_api_key_env {
            env.remove(default_env);
        }
        for env_name in descriptor.api_key_env_names {
            env.remove(*env_name);
        }
    }
}
#[test]
fn recommend_web_search_provider_from_available_credentials_prefers_unique_ready_provider() {
    let mut config = mvp::config::LoongConfig::default();
    config.tools.web_search.perplexity_api_key = Some("${PERPLEXITY_API_KEY}".to_owned());

    let mut env = ScopedEnv::new();
    clear_web_search_credential_envs(&mut env);
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
    let mut config = mvp::config::LoongConfig::default();
    config.tools.web_search.tavily_api_key = Some("${TAVILY_API_KEY}".to_owned());
    config.tools.web_search.perplexity_api_key = Some("${PERPLEXITY_API_KEY}".to_owned());

    let mut env = ScopedEnv::new();
    clear_web_search_credential_envs(&mut env);
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

#[tokio::test(flavor = "current_thread")]
async fn resolve_web_search_provider_selection_keeps_current_provider_on_blank_interactive_input_when_recommendation_differs()
 {
    let options = interactive_onboard_options();
    let mut config = mvp::config::LoongConfig::default();
    config.tools.web_search.tavily_api_key = Some("${TAVILY_API_KEY}".to_owned());

    let mut env = ScopedEnv::new();
    clear_web_search_credential_envs(&mut env);
    env.set("TAVILY_API_KEY", "tavily-test-token");

    let mut ui = TestOnboardUi::with_inputs([""]);
    let context = onboard_test_context();
    let selected = resolve_web_search_provider_selection(
        &options,
        &config,
        GuidedPromptPath::NativePromptPack,
        &mut ui,
        &context,
    )
    .await
    .expect("blank interactive input should keep the current web search provider");

    assert_eq!(
        selected,
        mvp::config::WEB_SEARCH_PROVIDER_DUCKDUCKGO,
        "interactive enter should preserve the current provider even when another provider is recommended"
    );
}

#[test]
fn render_web_search_provider_selection_screen_uses_actual_default_provider_in_footer() {
    let config = mvp::config::LoongConfig::default();
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
    let config = mvp::config::LoongConfig::default();
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
    let config = mvp::config::LoongConfig::default();
    let mut env = ScopedEnv::new();
    clear_web_search_credential_envs(&mut env);
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
fn resolve_web_search_credential_selection_uses_explicit_option_non_interactively() {
    let options = OnboardCommandOptions {
        output: None,
        force: false,
        non_interactive: true,
        accept_risk: true,
        provider: None,
        model: None,
        api_key_env: None,
        web_search_provider: Some("tavily".to_owned()),
        web_search_api_key_env: Some("TEAM_TAVILY_KEY".to_owned()),
        personality: None,
        memory_profile: None,
        system_prompt: None,
        skip_model_probe: false,
    };
    let config = mvp::config::LoongConfig::default();
    let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
    let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

    let selected = resolve_web_search_credential_selection(
        &options,
        &config,
        mvp::config::WEB_SEARCH_PROVIDER_TAVILY,
        GuidedPromptPath::NativePromptPack,
        true,
        &mut ui,
        &context,
    )
    .expect("non-interactive explicit web-search credential env should be accepted");

    assert_eq!(
        selected,
        WebSearchCredentialSelection::UseEnv("TEAM_TAVILY_KEY".to_owned())
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
fn resolve_system_prompt_selection_accepts_explicit_clear_token_in_interactive_mode() {
    let mut config = mvp::config::LoongConfig::default();
    config.cli.system_prompt = "be terse and code-focused".to_owned();
    let mut ui = TestOnboardUi::with_inputs([":clear"]);
    let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

    let selected = resolve_system_prompt_selection(
        &OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: false,
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
        },
        &config,
        &mut ui,
        &context,
    )
    .expect("resolve system prompt selection");

    assert_eq!(
        selected,
        SystemPromptSelection::RestoreBuiltIn,
        "typing :clear should restore the built-in system prompt instead of keeping the literal token"
    );
}

#[test]
fn resolve_system_prompt_selection_keeps_current_prompt_when_interactive_default_is_used() {
    let mut config = mvp::config::LoongConfig::default();
    config.cli.system_prompt = "be terse and code-focused".to_owned();
    let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
    let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

    let selected = resolve_system_prompt_selection(
        &OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: false,
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
        },
        &config,
        &mut ui,
        &context,
    )
    .expect("resolve system prompt selection");

    assert_eq!(
        selected,
        SystemPromptSelection::KeepCurrent,
        "using the prompt default should keep the current system prompt when no override is prefilled"
    );
}

#[test]
fn resolve_system_prompt_selection_keeps_prefilled_override_when_interactive_default_is_used() {
    let mut config = mvp::config::LoongConfig::default();
    config.cli.system_prompt = "be terse and code-focused".to_owned();
    let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
    let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

    let selected = resolve_system_prompt_selection(
        &OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: false,
            accept_risk: true,
            provider: None,
            model: None,
            api_key_env: None,
            web_search_provider: None,
            web_search_api_key_env: None,
            personality: None,
            memory_profile: None,
            system_prompt: Some("prefer concise code reviews".to_owned()),
            skip_model_probe: false,
        },
        &config,
        &mut ui,
        &context,
    )
    .expect("resolve system prompt selection");

    assert_eq!(
        selected,
        SystemPromptSelection::Set("prefer concise code reviews".to_owned()),
        "using the prompt default should still apply a prefilled system prompt override"
    );
}

#[test]
fn resolve_prompt_addendum_selection_keeps_current_addendum_when_blank_input_is_used() {
    let mut config = mvp::config::LoongConfig::default();
    config.cli.system_prompt_addendum = Some("Keep answers direct.".to_owned());
    let mut ui = TestOnboardUi::with_inputs([""]);
    let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

    let selected = resolve_prompt_addendum_selection(
        &OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: false,
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
        },
        &config,
        &mut ui,
        &context,
    )
    .expect("resolve prompt addendum selection");

    assert_eq!(
        selected.as_deref(),
        Some("Keep answers direct."),
        "blank optional input should keep the current addendum"
    );
}

#[test]
fn resolve_prompt_addendum_selection_uses_allow_empty_prompt_path_for_blank_first_run_input() {
    let config = mvp::config::LoongConfig::default();
    let mut ui = AllowEmptyOnlyTestUi::with_inputs([""]);
    let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

    let selected = resolve_prompt_addendum_selection(
        &OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: false,
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
        },
        &config,
        &mut ui,
        &context,
    )
    .expect("resolve prompt addendum selection");

    assert_eq!(
        selected, None,
        "blank first-run optional input should preserve the absence of an addendum"
    );
}

#[test]
fn resolve_prompt_addendum_selection_uses_allow_empty_prompt_path_for_clear_input() {
    let mut config = mvp::config::LoongConfig::default();
    config.cli.system_prompt_addendum = Some("Keep answers direct.".to_owned());
    let mut ui = AllowEmptyOnlyTestUi::with_inputs(["-"]);
    let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

    let selected = resolve_prompt_addendum_selection(
        &OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: false,
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
        },
        &config,
        &mut ui,
        &context,
    )
    .expect("resolve prompt addendum selection");

    assert_eq!(
        selected, None,
        "allow-empty prompt handling should still respect the explicit clear token"
    );
}

#[test]
fn resolve_prompt_addendum_selection_clears_current_addendum_when_dash_input_is_used() {
    let mut config = mvp::config::LoongConfig::default();
    config.cli.system_prompt_addendum = Some("Keep answers direct.".to_owned());
    let mut ui = TestOnboardUi::with_inputs(["-"]);
    let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

    let selected = resolve_prompt_addendum_selection(
        &OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: false,
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
        },
        &config,
        &mut ui,
        &context,
    )
    .expect("resolve prompt addendum selection");

    assert_eq!(
        selected, None,
        "typing '-' should still clear the current addendum"
    );
}

#[test]
fn apply_selected_system_prompt_restore_uses_rendered_native_prompt() {
    let mut config = mvp::config::LoongConfig::default();
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
fn resolve_provider_selection_keeps_zai_available_in_interactive_list() {
    let config = mvp::config::LoongConfig::default();
    let options = interactive_onboard_options();
    let provider_selection = crate::migration::ProviderSelectionPlan::default();
    let context = onboard_test_context();
    let mut ui = TestOnboardUi::with_inputs(["zai"]);

    let selected = resolve_provider_selection(
        &options,
        &config,
        &provider_selection,
        GuidedPromptPath::NativePromptPack,
        &mut ui,
        &context,
    )
    .expect("z.ai should stay selectable in the interactive provider list");

    assert_eq!(selected.kind, mvp::config::ProviderKind::Zai);
    assert_eq!(selected.base_url, "https://api.z.ai");
}

#[test]
fn resolve_provider_selection_preserves_kimi_coding_default_variant() {
    let mut config = mvp::config::LoongConfig::default();
    let options = interactive_onboard_options();
    let provider_selection = crate::migration::ProviderSelectionPlan::default();
    let context = onboard_test_context();
    let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
    config.provider =
        mvp::config::ProviderConfig::fresh_for_kind(mvp::config::ProviderKind::KimiCoding);

    let selected = resolve_provider_selection(
        &options,
        &config,
        &provider_selection,
        GuidedPromptPath::NativePromptPack,
        &mut ui,
        &context,
    )
    .expect("default kimi coding selection should stay stable");

    assert_eq!(selected.kind, mvp::config::ProviderKind::KimiCoding);
}

#[test]
fn resolve_provider_selection_preserves_step_plan_default_variant() {
    let mut config = mvp::config::LoongConfig::default();
    let options = interactive_onboard_options();
    let provider_selection = crate::migration::ProviderSelectionPlan::default();
    let context = onboard_test_context();
    let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
    config.provider =
        mvp::config::ProviderConfig::fresh_for_kind(mvp::config::ProviderKind::StepPlan);

    let selected = resolve_provider_selection(
        &options,
        &config,
        &provider_selection,
        GuidedPromptPath::NativePromptPack,
        &mut ui,
        &context,
    )
    .expect("default step plan selection should stay stable");

    assert_eq!(selected.kind, mvp::config::ProviderKind::StepPlan);
}

#[test]
fn resolve_provider_selection_preserves_existing_region_endpoint_default() {
    let mut config = mvp::config::LoongConfig::default();
    let options = interactive_onboard_options();
    let provider_selection = crate::migration::ProviderSelectionPlan::default();
    let context = onboard_test_context();
    let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
    let global_minimax_base_url = "https://api.minimax.io".to_owned();
    config.provider =
        mvp::config::ProviderConfig::fresh_for_kind(mvp::config::ProviderKind::Minimax);
    config.provider.base_url = global_minimax_base_url.clone();

    let selected = resolve_provider_selection(
        &options,
        &config,
        &provider_selection,
        GuidedPromptPath::NativePromptPack,
        &mut ui,
        &context,
    )
    .expect("region selection should preserve the current endpoint when accepting defaults");

    assert_eq!(selected.kind, mvp::config::ProviderKind::Minimax);
    assert_eq!(selected.base_url, global_minimax_base_url);
}

#[test]
fn resolve_provider_selection_allows_switching_step_plan_region_endpoint() {
    let mut config = mvp::config::LoongConfig::default();
    let options = interactive_onboard_options();
    let provider_selection = crate::migration::ProviderSelectionPlan::default();
    let context = onboard_test_context();
    let mut ui = TestOnboardUi::with_inputs(["", "", "2"]);

    config.provider =
        mvp::config::ProviderConfig::fresh_for_kind(mvp::config::ProviderKind::StepPlan);

    let selected = resolve_provider_selection(
        &options,
        &config,
        &provider_selection,
        GuidedPromptPath::NativePromptPack,
        &mut ui,
        &context,
    )
    .expect("step plan region selection should accept the global endpoint");

    assert_eq!(selected.kind, mvp::config::ProviderKind::StepPlan);
    assert_eq!(selected.base_url, "https://api.stepfun.ai");
}

#[test]
fn resolve_provider_selection_prompts_for_custom_base_url_when_unresolved() {
    let config = mvp::config::LoongConfig::default();
    let options = interactive_onboard_options();
    let provider_selection = crate::migration::ProviderSelectionPlan::default();
    let context = onboard_test_context();
    let mut ui = TestOnboardUi::with_inputs(["custom", "https://api.example.com/v1"]);

    let selected = resolve_provider_selection(
        &options,
        &config,
        &provider_selection,
        GuidedPromptPath::NativePromptPack,
        &mut ui,
        &context,
    )
    .expect("custom provider selection should accept an explicit base URL");

    assert_eq!(selected.kind, mvp::config::ProviderKind::Custom);
    assert_eq!(selected.base_url, "https://api.example.com/v1");
}

#[test]
fn resolve_provider_selection_preserves_existing_custom_base_url_by_default() {
    let mut config = mvp::config::LoongConfig::default();
    let options = interactive_onboard_options();
    let provider_selection = crate::migration::ProviderSelectionPlan::default();
    let context = onboard_test_context();
    let mut ui = TestOnboardUi::with_inputs(["", ""]);

    config.provider =
        mvp::config::ProviderConfig::fresh_for_kind(mvp::config::ProviderKind::Custom);
    config.provider.base_url = "https://api.example.com/v1".to_owned();

    let selected = resolve_provider_selection(
        &options,
        &config,
        &provider_selection,
        GuidedPromptPath::NativePromptPack,
        &mut ui,
        &context,
    )
    .expect("custom provider selection should keep the existing base URL on default accept");

    assert_eq!(selected.kind, mvp::config::ProviderKind::Custom);
    assert_eq!(selected.base_url, "https://api.example.com/v1");
}

#[test]
fn resolve_provider_selection_rejects_invalid_custom_base_url() {
    let config = mvp::config::LoongConfig::default();
    let options = interactive_onboard_options();
    let provider_selection = crate::migration::ProviderSelectionPlan::default();
    let context = onboard_test_context();
    let mut ui = TestOnboardUi::with_inputs(["custom", "not-a-url"]);

    let error = resolve_provider_selection(
        &options,
        &config,
        &provider_selection,
        GuidedPromptPath::NativePromptPack,
        &mut ui,
        &context,
    )
    .expect_err("custom provider selection should reject invalid base URLs");

    assert!(error.contains("provider base URL is invalid"));
}

#[test]
fn preinstalled_skills_screen_only_surfaces_the_onboarding_subset() {
    let lines = render_preinstalled_skills_selection_screen_lines_with_style(100, false);
    let joined = lines.join("\n");

    for expected in [
        "systematic-debugging",
        "plan",
        "github-issues",
        "Anthropic Office pack",
        "Minimax Office pack",
    ] {
        assert!(
            joined.contains(expected),
            "expected onboarding preinstall screen to advertise `{expected}`: {joined}"
        );
    }

    for hidden in [
        "native-mcp)",
        "mcporter)",
        "docx)",
        "pdf)",
        "pptx)",
        "xlsx)",
    ] {
        assert!(
            !joined.contains(hidden),
            "did not expect onboarding preinstall screen to advertise `{hidden}`: {joined}"
        );
    }
}

#[test]
fn onboarding_preinstall_targets_are_derived_from_app_registry() {
    let anthropic = mvp::tools::bundled_preinstall_targets()
        .iter()
        .find(|target| target.install_id == "anthropic-office")
        .expect("anthropic office pack should be exposed by app registry");
    assert_eq!(anthropic.skill_ids, &["docx", "pdf", "pptx", "xlsx"]);
}

#[test]
fn resolve_model_selection_prefills_minimax_recommended_model_interactively() {
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Minimax;
    config.provider.model = "auto".to_owned();
    let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
    let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

    let selected = resolve_model_selection(
        &OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: false,
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
        },
        &config,
        GuidedPromptPath::NativePromptPack,
        &[],
        &mut ui,
        &context,
    )
    .expect("resolve model selection");

    assert!(
        selected == "MiniMax-M2.7",
        "interactive onboarding should prefill the provider-recommended explicit model for MiniMax instead of leaving the operator on hidden runtime fallbacks: {selected:?}"
    );
}

#[test]
fn resolve_model_selection_applies_minimax_recommended_model_non_interactively() {
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Minimax;
    config.provider.model = "auto".to_owned();
    let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
    let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

    let selected = resolve_model_selection(
        &OnboardCommandOptions {
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
        },
        &config,
        GuidedPromptPath::NativePromptPack,
        &[],
        &mut ui,
        &context,
    )
    .expect("resolve model selection");

    assert!(
        selected == "MiniMax-M2.7",
        "non-interactive onboarding should use the reviewed provider default for MiniMax instead of carrying auto into preflight: {selected:?}"
    );
}

#[test]
fn resolve_model_selection_prefills_deepseek_recommended_model_interactively() {
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Deepseek;
    config.provider.model = "auto".to_owned();
    let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
    let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

    let selected = resolve_model_selection(
        &OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: false,
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
        },
        &config,
        GuidedPromptPath::NativePromptPack,
        &[],
        &mut ui,
        &context,
    )
    .expect("resolve model selection");

    assert!(
        selected == "deepseek-chat",
        "interactive onboarding should prefill the provider-recommended explicit model for DeepSeek instead of leaving the operator on auto: {selected:?}"
    );
}

#[test]
fn resolve_model_selection_applies_deepseek_recommended_model_non_interactively() {
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Deepseek;
    config.provider.model = "auto".to_owned();
    let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
    let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

    let selected = resolve_model_selection(
        &OnboardCommandOptions {
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
        },
        &config,
        GuidedPromptPath::NativePromptPack,
        &[],
        &mut ui,
        &context,
    )
    .expect("resolve model selection");

    assert!(
        selected == "deepseek-chat",
        "non-interactive onboarding should use the reviewed provider default for DeepSeek instead of carrying auto into preflight: {selected:?}"
    );
}

#[test]
fn resolve_model_selection_prefills_reviewed_model_for_mixed_case_auto_interactively() {
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Deepseek;
    config.provider.model = "  AUTO  ".to_owned();
    let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
    let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

    let selected = resolve_model_selection(
        &OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: false,
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
        },
        &config,
        GuidedPromptPath::NativePromptPack,
        &[],
        &mut ui,
        &context,
    )
    .expect("resolve model selection");

    assert_eq!(
        selected, "deepseek-chat",
        "interactive onboarding should treat mixed-case auto the same as auto when choosing a reviewed provider default"
    );
}

#[test]
fn resolve_model_selection_rejects_blank_explicit_model_non_interactively() {
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Deepseek;
    config.provider.model = "auto".to_owned();
    let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
    let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

    let error = resolve_model_selection(
        &OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: true,
            accept_risk: true,
            provider: None,
            model: Some("   ".to_owned()),
            api_key_env: None,
            web_search_provider: None,
            web_search_api_key_env: None,
            personality: None,
            memory_profile: None,
            system_prompt: None,
            skip_model_probe: false,
        },
        &config,
        GuidedPromptPath::NativePromptPack,
        &[],
        &mut ui,
        &context,
    )
    .expect_err(
        "blank explicit --model should fail instead of falling back to a recommended model",
    );

    assert_eq!(error, "model cannot be empty");
}

#[test]
fn resolve_model_selection_uses_catalog_choices_when_available_interactively() {
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Deepseek;
    config.provider.model = "auto".to_owned();
    let mut ui = TestOnboardUi::with_inputs(["2"]);
    let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());
    let available_models = vec!["deepseek-chat".to_owned(), "deepseek-reasoner".to_owned()];

    let selected = resolve_model_selection(
        &OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: false,
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
        },
        &config,
        GuidedPromptPath::NativePromptPack,
        &available_models,
        &mut ui,
        &context,
    )
    .expect("resolve model selection");

    assert_eq!(
        selected, "deepseek-reasoner",
        "interactive onboarding should use the probed model catalog instead of treating numeric selection input as a literal model id"
    );
}

#[test]
fn resolve_model_selection_keeps_auto_visible_for_noncanonical_volcengine_catalog() {
    let mut config = mvp::config::LoongConfig::default();
    config.provider =
        mvp::config::ProviderConfig::fresh_for_kind(mvp::config::ProviderKind::VolcengineCoding);
    config.provider.base_url =
        "https://proxy.example.com/forward/ark.cn-beijing.volces.com/api/coding/v3".to_owned();
    config.provider.model = "auto".to_owned();
    let mut ui = TestOnboardUi::with_inputs(["1"]);
    let context = onboard_test_context();
    let available_models = vec![
        "doubao-seed-2.0-code".to_owned(),
        "doubao-seed-2.0-pro".to_owned(),
    ];

    let selected = resolve_model_selection(
        &interactive_onboard_options(),
        &config,
        GuidedPromptPath::NativePromptPack,
        &available_models,
        &mut ui,
        &context,
    )
    .expect("resolve model selection");

    assert_eq!(
        selected, "auto",
        "noncanonical Volcengine endpoints should not hide the `auto` choice just because the returned models contain a static-catalog model id"
    );
}

#[test]
fn resolve_model_selection_rejects_blank_custom_override_when_auto_is_hidden() {
    let mut config = mvp::config::LoongConfig::default();
    config.provider =
        mvp::config::ProviderConfig::fresh_for_kind(mvp::config::ProviderKind::VolcengineCoding);
    config.provider.model = "auto".to_owned();
    let mut ui = TestOnboardUi::with_inputs(["3", ""]);
    let context = onboard_test_context();
    let available_models = vec![
        "ark-code-latest".to_owned(),
        "doubao-seed-2.0-code".to_owned(),
    ];

    let error = resolve_model_selection(
        &interactive_onboard_options(),
        &config,
        GuidedPromptPath::NativePromptPack,
        &available_models,
        &mut ui,
        &context,
    )
    .expect_err("blank custom entry should not round-trip hidden auto");

    assert_eq!(error, "model cannot be empty");
}

#[tokio::test(flavor = "current_thread")]
async fn load_onboarding_model_catalog_returns_static_list_for_canonical_volcengine_endpoint() {
    let mut options = interactive_onboard_options();
    options.skip_model_probe = true;
    let mut config = mvp::config::LoongConfig::default();
    config.provider =
        mvp::config::ProviderConfig::fresh_for_kind(mvp::config::ProviderKind::VolcengineCoding);

    let models = load_onboarding_model_catalog(&options, &config).await;

    assert_eq!(
        models,
        vec![
            "ark-code-latest".to_owned(),
            "doubao-seed-2.0-code".to_owned(),
            "doubao-seed-2.0-pro".to_owned(),
            "doubao-seed-2.0-lite".to_owned(),
            "doubao-seed-code".to_owned(),
            "minimax-m2.5".to_owned(),
            "glm-4.7".to_owned(),
            "deepseek-v3.2".to_owned(),
            "kimi-k2.5".to_owned(),
        ],
        "the canonical Volcengine Coding endpoint should still use the static onboarding catalog"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn load_onboarding_model_catalog_skips_static_list_for_noncanonical_volcengine_endpoint() {
    let mut options = interactive_onboard_options();
    options.skip_model_probe = true;
    let mut config = mvp::config::LoongConfig::default();
    config.provider =
        mvp::config::ProviderConfig::fresh_for_kind(mvp::config::ProviderKind::VolcengineCoding);
    config.provider.base_url =
        "https://proxy.example.com/forward/ark.cn-beijing.volces.com/api/coding/v3".to_owned();

    let models = load_onboarding_model_catalog(&options, &config).await;

    assert!(
        models.is_empty(),
        "noncanonical Volcengine endpoints should follow normal probe-skip behavior instead of forcing the hardcoded static catalog: {models:?}"
    );
}

#[test]
fn resolve_model_selection_allows_custom_override_when_catalog_is_available() {
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Openai;
    config.provider.model = "openai/gpt-5.1-codex".to_owned();
    let mut ui = TestOnboardUi::with_inputs(["2", "openai/gpt-5.2"]);
    let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());
    let available_models = vec!["openai/gpt-5.1-codex".to_owned()];

    let selected = resolve_model_selection(
        &OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: false,
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
        },
        &config,
        GuidedPromptPath::NativePromptPack,
        &available_models,
        &mut ui,
        &context,
    )
    .expect("resolve model selection");

    assert_eq!(
        selected, "openai/gpt-5.2",
        "interactive onboarding should keep a manual override path even when a searchable model catalog is available"
    );
}

#[test]
fn prompt_onboard_entry_choice_uses_select_widget() {
    let options = vec![
        OnboardEntryOption {
            choice: OnboardEntryChoice::ContinueCurrentSetup,
            label: "continue current setup",
            detail: "reuse current draft".to_owned(),
            recommended: true,
        },
        OnboardEntryOption {
            choice: OnboardEntryChoice::StartFresh,
            label: "start fresh",
            detail: "ignore detected setup".to_owned(),
            recommended: false,
        },
    ];
    let mut ui = SelectOnlyTestUi::with_inputs(["2"]);

    let choice = prompt_onboard_entry_choice(&mut ui, &options)
        .expect("entry choice should route through select_one");

    assert_eq!(choice, OnboardEntryChoice::StartFresh);
}

#[test]
fn prompt_import_candidate_choice_uses_select_widget() {
    let mut ui = SelectOnlyTestUi::with_inputs(["3"]);
    let candidates = vec![
        ImportCandidate {
            source_kind: crate::migration::ImportSourceKind::RecommendedPlan,
            source: "recommended plan".to_owned(),
            config: mvp::config::LoongConfig::default(),
            surfaces: Vec::new(),
            domains: Vec::new(),
            channel_candidates: Vec::new(),
            workspace_guidance: Vec::new(),
        },
        ImportCandidate {
            source_kind: crate::migration::ImportSourceKind::CodexConfig,
            source: "codex config".to_owned(),
            config: mvp::config::LoongConfig::default(),
            surfaces: Vec::new(),
            domains: Vec::new(),
            channel_candidates: Vec::new(),
            workspace_guidance: Vec::new(),
        },
    ];

    let choice = prompt_import_candidate_choice(&mut ui, &candidates, 80)
        .expect("starting-point choice should route through select_one");

    assert_eq!(choice, None);
}

#[test]
fn prompt_onboard_shortcut_choice_uses_select_widget() {
    let mut ui = SelectOnlyTestUi::with_inputs(["2"]);

    let choice = prompt_onboard_shortcut_choice(&mut ui, OnboardShortcutKind::CurrentSetup)
        .expect("shortcut choice should route through select_one");

    assert_eq!(choice, OnboardShortcutChoice::AdjustSettings);
}

#[test]
fn resolve_write_plan_uses_select_widget_for_existing_config() {
    let temp_dir = std::env::temp_dir().join(format!(
        "loongclaw-onboard-write-plan-{}",
        OffsetDateTime::now_utc().unix_timestamp_nanos()
    ));
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let output_path = temp_dir.join("loongclaw.toml");
    fs::write(&output_path, "provider = 'openai'\n").expect("seed existing config");
    let mut ui = SelectOnlyTestUi::with_inputs(["2"]);
    let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

    let plan = resolve_write_plan(
        &output_path,
        &OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: false,
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
        },
        &mut ui,
        &context,
    )
    .expect("existing-config confirmation should route through select_one");

    assert!(plan.force);
    assert!(
        plan.backup_path.is_some(),
        "backup selection should preserve the safer write path"
    );
    fs::remove_dir_all(&temp_dir).expect("cleanup temp dir");
}

#[test]
fn prompt_onboard_shortcut_choice_cancels_on_escape_input() {
    let mut ui = TestOnboardUi::with_inputs(["\u{1b}"]);

    let error = prompt_onboard_shortcut_choice(&mut ui, OnboardShortcutKind::CurrentSetup)
        .expect_err("escape input should cancel instead of silently falling through");

    assert!(
        error.contains("cancelled"),
        "escape cancellation should produce a user-facing cancel error: {error}"
    );
}

#[test]
fn test_onboard_ui_prompt_with_default_only_checks_user_input_for_cancel() {
    let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());

    let value = ui
        .prompt_with_default("Provider", "\u{1b}")
        .expect("missing input should keep the configured default");

    assert_eq!(value, "\u{1b}");
}

#[test]
fn explicit_onboard_cancel_input_requires_escape_byte() {
    assert!(is_explicit_onboard_cancel_input("\u{1b}"));
    assert!(
        !is_explicit_onboard_cancel_input("esc"),
        "literal text should remain valid operator input instead of being treated as an escape keystroke"
    );
    assert!(
        !is_explicit_onboard_cancel_input("ESC"),
        "case variants of plain text should not trigger onboarding cancellation"
    );
}

#[test]
fn literal_esc_text_is_not_treated_as_cancel_input() {
    let value = ensure_onboard_input_not_cancelled("esc".to_owned())
        .expect("literal esc text should remain valid input");

    assert_eq!(value, "esc");
}

#[test]
fn test_onboard_ui_prompt_required_trims_input_like_stdio() {
    let mut ui = TestOnboardUi::with_inputs(["  minimax  "]);

    let value = ui
        .prompt_required("Provider")
        .expect("required prompt should preserve stdio trimming semantics");

    assert_eq!(value, "minimax");
}

#[test]
fn single_line_prompt_capture_drains_follow_up_paste_before_next_prompt() {
    let mut reader = TestPromptLineReader::new(
        [
            OnboardPromptRead::Line("You are helpful.\n".to_owned()),
            OnboardPromptRead::Line("window-plus-summary\n".to_owned()),
        ],
        ["Always be concise.\n"],
    );

    let first =
        read_single_line_prompt_capture(&mut reader).expect("first prompt capture should succeed");
    let second = read_single_line_prompt_capture(&mut reader)
        .expect("second prompt capture should consume the next real prompt line");

    assert_eq!(first.raw, "You are helpful.\n");
    assert_eq!(first.dropped_line_count, 1);
    assert!(!first.reached_eof);
    assert_eq!(second.raw, "window-plus-summary\n");
    assert_eq!(second.dropped_line_count, 0);
    assert!(!second.reached_eof);
}

#[test]
fn onboard_paste_drain_window_prefers_valid_env_override() {
    let _guard = PasteDrainWindowEnvGuard::set(Some("125"));

    assert_eq!(onboard_paste_drain_window(), Duration::from_millis(125));
}

#[test]
fn onboard_paste_drain_window_falls_back_for_invalid_env_values() {
    let _guard = PasteDrainWindowEnvGuard::set(Some("not-a-number"));

    assert_eq!(
        onboard_paste_drain_window(),
        DEFAULT_ONBOARD_PASTE_DRAIN_WINDOW
    );
}

#[test]
fn onboard_paste_drain_window_rejects_zero_millisecond_override() {
    let _guard = PasteDrainWindowEnvGuard::set(Some("0"));

    assert_eq!(
        onboard_paste_drain_window(),
        DEFAULT_ONBOARD_PASTE_DRAIN_WINDOW
    );
}

#[test]
fn onboard_line_channel_applies_backpressure_after_buffer_limit() {
    let (sender, receiver) = onboard_line_channel_with_capacity(1);
    let second_send_completed = Arc::new(AtomicBool::new(false));
    let completed_flag = Arc::clone(&second_send_completed);
    let producer = thread::spawn(move || {
        sender
            .send(StdioOnboardLineMessage::Line("system prompt\n".to_owned()))
            .expect("send first line");
        sender
            .send(StdioOnboardLineMessage::Line(
                "follow-up paste\n".to_owned(),
            ))
            .expect("send second line after receiver drains");
        completed_flag.store(true, Ordering::SeqCst);
    });

    for _ in 0..1_000 {
        if second_send_completed.load(Ordering::SeqCst) {
            break;
        }
        thread::yield_now();
    }
    assert!(
        !second_send_completed.load(Ordering::SeqCst),
        "bounded onboarding queue should apply backpressure once the first buffered line is occupied"
    );

    let mut reader = StdioOnboardLineReader::background_from_receiver(receiver);
    let capture = read_single_line_prompt_capture(&mut reader)
        .expect("capture should drain the queued follow-up line");
    producer.join().expect("producer join");

    assert_eq!(capture.raw, "system prompt\n");
    assert_eq!(capture.dropped_line_count, 1);
    assert!(!capture.reached_eof);
    assert!(
        second_send_completed.load(Ordering::SeqCst),
        "receiver drain should unblock the producer once capacity is freed"
    );
}

#[test]
fn stdio_onboard_line_reader_warns_once_when_background_spawn_fails() {
    let mut reader =
        StdioOnboardLineReader::from_spawn_result(Err(io::Error::other("thread quota exhausted")));

    assert!(
        matches!(reader, StdioOnboardLineReader::Direct { .. }),
        "spawn failure should fall back to direct reads instead of constructing a broken background reader"
    );

    let first_notice = reader
        .take_degraded_notice()
        .expect("spawn failure should surface a degraded-mode notice");
    assert!(
        first_notice.contains("single-line paste draining is disabled"),
        "spawn failure notice should explain the lost hardening: {first_notice}"
    );
    assert_eq!(
        reader.take_degraded_notice(),
        None,
        "degraded-mode notice should only be emitted once per session"
    );
}

#[test]
fn prompt_addendum_screen_mentions_single_line_terminal_input() {
    let lines =
        render_prompt_addendum_selection_screen_lines(&mvp::config::LoongConfig::default(), 80);

    assert!(
        lines.iter().any(|line| line == "- single-line input only"),
        "prompt addendum screen should keep the terminal input note concise: {lines:#?}"
    );
}

#[test]
fn system_prompt_screen_mentions_single_line_terminal_input() {
    let lines =
        render_system_prompt_selection_screen_lines(&mvp::config::LoongConfig::default(), 80);

    assert!(
        lines.iter().any(|line| line == "- single-line input only"),
        "system prompt screen should keep the terminal input note concise: {lines:#?}"
    );
}

#[test]
fn test_onboard_ui_select_one_cancels_on_escape_input() {
    let mut ui = TestOnboardUi::with_inputs(["\u{1b}"]);
    let options = vec![SelectOption {
        label: "OpenAI".to_owned(),
        slug: "openai".to_owned(),
        description: String::new(),
        recommended: true,
    }];

    let error = ui
        .select_one("Provider", &options, Some(0), SelectInteractionMode::List)
        .expect_err("escape input should cancel selection instead of surfacing a parse error");

    assert!(
        error.contains("cancelled"),
        "escape cancellation should stay user-facing for selection prompts: {error}"
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
            key: "classicist".to_owned(),
            label: "classicist keeps longer wrapped labels aligned".to_owned(),
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
        continuation
            .starts_with(&" ".repeat(render_onboard_option_prefix("classicist").chars().count())),
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
            .any(|line| line == crate::onboard_presentation::entry_choice_section_heading()),
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
        config: mvp::config::LoongConfig::default(),
        surfaces: Vec::new(),
        domains: Vec::new(),
        channel_candidates: Vec::new(),
        workspace_guidance: Vec::new(),
    };
    let lines = render_starting_point_selection_header_lines_with_style(&[candidate], 80, false);

    assert!(
        lines
            .iter()
            .any(|line| line == crate::onboard_presentation::starting_point_selection_title()),
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
fn stdio_onboard_ui_starts_without_initializing_line_reader() {
    let ui = StdioOnboardUi::default();

    assert!(
        ui.line_reader.is_none(),
        "stdio ui should not create a stdin reader until the stdio fallback path is actually used"
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
fn test_onboard_ui_select_one_accepts_slug_input() {
    let mut ui = TestOnboardUi::with_inputs(["hermit"]);
    let options = vec![
        SelectOption {
            label: "classicist".to_owned(),
            slug: "classicist".to_owned(),
            description: String::new(),
            recommended: true,
        },
        SelectOption {
            label: "hermit".to_owned(),
            slug: "hermit".to_owned(),
            description: String::new(),
            recommended: false,
        },
    ];

    let index = ui
        .select_one(
            "Personality",
            &options,
            Some(0),
            SelectInteractionMode::List,
        )
        .expect("test ui should stay aligned with shared slug-selection behavior");

    assert_eq!(index, 1);
}

#[test]
fn test_onboard_ui_select_one_accepts_legacy_personality_alias_input() {
    let mut ui = TestOnboardUi::with_inputs(["friendly_collab"]);
    let options = vec![
        SelectOption {
            label: "classicist".to_owned(),
            slug: "classicist".to_owned(),
            description: String::new(),
            recommended: true,
        },
        SelectOption {
            label: "hermit".to_owned(),
            slug: "hermit".to_owned(),
            description: String::new(),
            recommended: false,
        },
    ];

    let index = ui
        .select_one(
            "Personality",
            &options,
            Some(0),
            SelectInteractionMode::List,
        )
        .expect("legacy personality aliases should still resolve in selector mode");

    assert_eq!(index, 1);
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
        render_continue_current_setup_screen_lines(&mvp::config::LoongConfig::default(), 80);

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
        &mvp::config::LoongConfig::default(),
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
    let config = mvp::config::LoongConfig::default();
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
    let mut config = mvp::config::LoongConfig::default();
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
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Custom;
    config.provider.model = "auto".to_owned();

    let check =
        provider_model_probe_failure_check(&config, "provider rejected the model list".to_owned());
    let lines = render_preflight_summary_screen_lines(&[check], 80);

    assert!(
        lines.iter().any(|line| {
            line == crate::onboard_presentation::preflight_explicit_model_only_rerun_hint()
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
            line.as_str() != crate::onboard_presentation::preflight_probe_rerun_hint()
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
    let mut config = mvp::config::LoongConfig::default();
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
    fs::write(&output_path, "partial = true\n").expect("write partial config");

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
