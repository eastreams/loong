#![allow(unsafe_code)]
#![allow(
    clippy::disallowed_methods,
    clippy::multiple_unsafe_ops_per_block,
    clippy::undocumented_unsafe_blocks,
    clippy::indexing_slicing
)]

use super::*;
use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::MutexGuard;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_PATH_COUNTER: AtomicU64 = AtomicU64::new(0);

fn assert_compact_loongclaw_header(lines: &[String], context: &str) {
    assert!(
        lines
            .first()
            .is_some_and(|line| line.starts_with("LOONGCLAW")),
        "{context} should start with the compact LOONGCLAW header: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .take_while(|line| !line.is_empty())
            .any(|line| line.contains(concat!("v", env!("CARGO_PKG_VERSION")))),
        "{context} should keep the current build version visible even when the branch name wraps: {lines:#?}"
    );
}

fn unique_temp_path(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    let counter = TEMP_PATH_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "loongclaw-onboard-{label}-{}-{nanos}-{counter}",
        std::process::id(),
    ))
}

fn provider_choice_input(kind: mvp::config::ProviderKind) -> String {
    let index = mvp::config::ProviderKind::all_sorted()
        .iter()
        .position(|candidate| *candidate == kind)
        .expect("provider kind should exist in the interactive onboarding order");
    (index + 1).to_string()
}

fn scripted_input_not_cancelled(raw: String) -> loongclaw_daemon::CliResult<String> {
    if raw.trim() == "\u{1b}" {
        return Err("onboarding cancelled: escape input received".to_owned());
    }
    Ok(raw)
}

struct DetectedEnvironmentGuard {
    _lock: MutexGuard<'static, ()>,
    saved: Vec<(String, Option<OsString>)>,
}

impl DetectedEnvironmentGuard {
    fn without_detected_environment() -> Self {
        let lock = super::lock_daemon_test_environment();
        let mut keys = std::collections::BTreeSet::new();
        let default_config = mvp::config::LoongClawConfig::default();

        for provider_kind in mvp::config::ProviderKind::all_sorted() {
            if let Some(key) = provider_kind.default_api_key_env() {
                keys.insert(key.to_owned());
            }
            for alias in provider_kind.api_key_env_aliases() {
                keys.insert((*alias).to_owned());
            }
            if let Some(key) = provider_kind.default_oauth_access_token_env() {
                keys.insert(key.to_owned());
            }
            for alias in provider_kind.oauth_access_token_env_aliases() {
                keys.insert((*alias).to_owned());
            }
        }
        if let Some(key) = default_config.telegram.bot_token_env.as_deref() {
            keys.insert(key.to_owned());
        }
        if let Some(key) = default_config.feishu.app_id_env.as_deref() {
            keys.insert(key.to_owned());
        }
        if let Some(key) = default_config.feishu.app_secret_env.as_deref() {
            keys.insert(key.to_owned());
        }
        for descriptor in mvp::config::web_search_provider_descriptors() {
            for env_name in descriptor.api_key_env_names {
                keys.insert((*env_name).to_owned());
            }
        }

        let saved = keys
            .into_iter()
            .map(|key| {
                let value = std::env::var_os(&key);
                unsafe {
                    std::env::remove_var(&key);
                }
                (key, value)
            })
            .collect();

        Self { _lock: lock, saved }
    }
}

impl Drop for DetectedEnvironmentGuard {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..) {
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

struct EnvVarGuard {
    _lock: Option<MutexGuard<'static, ()>>,
    key: String,
    saved: Option<OsString>,
}

impl EnvVarGuard {
    fn set(key: &str, value: &str) -> Self {
        let lock = super::lock_daemon_test_environment();
        Self::set_inner(Some(lock), key, value)
    }

    /// # Safety
    ///
    /// The caller must already hold the daemon test environment lock, or an
    /// equivalent external guard, for the full lifetime of the returned
    /// `EnvVarGuard`. This wraps process-global environment mutation without
    /// taking the lock itself.
    unsafe fn set_unlocked(key: &str, value: &str) -> Self {
        Self::set_inner(None, key, value)
    }

    fn set_inner(lock: Option<MutexGuard<'static, ()>>, key: &str, value: &str) -> Self {
        let saved = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value);
        }
        Self {
            _lock: lock,
            key: key.to_owned(),
            saved,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match self.saved.take() {
            Some(value) => unsafe {
                std::env::set_var(&self.key, value);
            },
            None => unsafe {
                std::env::remove_var(&self.key);
            },
        }
    }
}

fn import_candidate_with_kind(
    source_kind: loongclaw_daemon::migration::types::ImportSourceKind,
    source: &str,
) -> loongclaw_daemon::onboard_cli::ImportCandidate {
    loongclaw_daemon::onboard_cli::ImportCandidate {
        source_kind,
        source: source.to_owned(),
        config: mvp::config::LoongClawConfig::default(),
        surfaces: Vec::new(),
        domains: Vec::new(),
        channel_candidates: Vec::new(),
        workspace_guidance: Vec::new(),
    }
}

fn import_candidate_with_provider(
    source_kind: loongclaw_daemon::migration::types::ImportSourceKind,
    source: &str,
    kind: mvp::config::ProviderKind,
    model: &str,
    credential_env: &str,
) -> loongclaw_daemon::onboard_cli::ImportCandidate {
    let mut candidate = import_candidate_with_kind(source_kind, source);
    let profile = kind.profile();
    candidate.config.provider.kind = kind;
    candidate.config.provider.base_url = profile.base_url.to_owned();
    candidate.config.provider.chat_completions_path = profile.chat_completions_path.to_owned();
    candidate.config.provider.model = model.to_owned();
    candidate.config.provider.api_key_env = Some(credential_env.to_owned());
    candidate
        .domains
        .push(loongclaw_daemon::migration::types::DomainPreview {
            kind: loongclaw_daemon::migration::types::SetupDomainKind::Provider,
            status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
            decision: Some(loongclaw_daemon::migration::types::PreviewDecision::UseDetected),
            source: source.to_owned(),
            summary: loongclaw_daemon::provider_presentation::provider_identity_summary(
                &candidate.config.provider,
            ),
        });
    candidate
}

struct ScriptedOnboardUi {
    inputs: VecDeque<String>,
    outputs: Vec<String>,
}

impl ScriptedOnboardUi {
    fn new(inputs: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            inputs: inputs.into_iter().map(Into::into).collect(),
            outputs: Vec::new(),
        }
    }

    fn transcript(self) -> Vec<String> {
        self.outputs
    }

    fn next_input(&mut self, label: &str) -> loongclaw_daemon::CliResult<String> {
        self.inputs.pop_front().ok_or_else(|| {
            format!(
                "missing scripted input for {label}; transcript so far:\n{}",
                self.outputs.join("\n")
            )
        })
    }
}

impl loongclaw_daemon::onboard_cli::OnboardUi for ScriptedOnboardUi {
    fn print_line(&mut self, line: &str) -> loongclaw_daemon::CliResult<()> {
        self.outputs.push(line.to_owned());
        Ok(())
    }

    fn prompt_with_default(
        &mut self,
        label: &str,
        default: &str,
    ) -> loongclaw_daemon::CliResult<String> {
        self.outputs
            .push(format!("PROMPT {label} (default: {default})"));
        let value = self.next_input(label)?;
        if value.trim().is_empty() {
            return Ok(default.to_owned());
        }
        Ok(value)
    }

    fn prompt_required(&mut self, label: &str) -> loongclaw_daemon::CliResult<String> {
        self.outputs.push(format!("PROMPT {label}"));
        self.next_input(label)
    }

    fn prompt_confirm(
        &mut self,
        message: &str,
        default: bool,
    ) -> loongclaw_daemon::CliResult<bool> {
        self.outputs.push(format!(
            "PROMPT {message} {}",
            if default { "[Y/n]" } else { "[y/N]" }
        ));
        let value = self.next_input(message)?;
        let trimmed = value.trim().to_ascii_lowercase();
        if trimmed.is_empty() {
            return Ok(default);
        }
        Ok(matches!(trimmed.as_str(), "y" | "yes"))
    }

    fn select_one(
        &mut self,
        label: &str,
        options: &[loongclaw_daemon::onboard_cli::SelectOption],
        default: Option<usize>,
        _interaction_mode: loongclaw_daemon::onboard_cli::SelectInteractionMode,
    ) -> loongclaw_daemon::CliResult<usize> {
        if options.is_empty() {
            return Err("no selection options available".to_owned());
        }
        if let Some(idx) = default
            && idx >= options.len()
        {
            return Err(format!(
                "default selection index {idx} out of range 0..{}",
                options.len() - 1
            ));
        }
        self.outputs.push(format!("SELECT {label}"));
        let value = scripted_input_not_cancelled(self.next_input(label)?)?;
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return default.ok_or_else(|| "no default for required selection".to_owned());
        }
        if let Ok(n) = trimmed.parse::<usize>() {
            if n >= 1 && n <= options.len() {
                return Ok(n - 1);
            }
            return Err(format!(
                "scripted selection {n} out of range 1..={}",
                options.len()
            ));
        }
        options
            .iter()
            .position(|option| option.slug.eq_ignore_ascii_case(trimmed))
            .ok_or_else(|| format!("invalid scripted selection input: {trimmed}"))
    }
}

async fn run_scripted_onboard_flow(
    options: loongclaw_daemon::onboard_cli::OnboardCommandOptions,
    inputs: impl IntoIterator<Item = impl Into<String>>,
    workspace_root: Option<PathBuf>,
    codex_config_path: Option<PathBuf>,
) -> loongclaw_daemon::CliResult<Vec<String>> {
    run_scripted_onboard_flow_with_context(
        options,
        inputs,
        loongclaw_daemon::onboard_cli::OnboardRuntimeContext::new_for_tests(
            80,
            workspace_root,
            codex_config_path,
        ),
    )
    .await
}

async fn run_scripted_onboard_flow_with_context(
    options: loongclaw_daemon::onboard_cli::OnboardCommandOptions,
    inputs: impl IntoIterator<Item = impl Into<String>>,
    context: loongclaw_daemon::onboard_cli::OnboardRuntimeContext,
) -> loongclaw_daemon::CliResult<Vec<String>> {
    let mut ui = ScriptedOnboardUi::new(inputs);
    loongclaw_daemon::onboard_cli::run_onboard_cli_with_ui(options, &mut ui, &context).await?;
    Ok(ui.transcript())
}

fn extract_review_section_lines(transcript: &[String], progress_line: &str) -> Vec<String> {
    let start = transcript
        .windows(2)
        .position(|window| window[0] == "review setup" && window[1] == progress_line)
        .expect("transcript should include review section");
    let end = transcript[start..]
        .iter()
        .position(|line| line == "preflight checks")
        .map(|offset| start + offset)
        .unwrap_or(transcript.len());
    transcript[start..end].to_vec()
}

fn extract_success_section_lines(transcript: &[String]) -> Vec<String> {
    let start = transcript
        .iter()
        .position(|line| line == "onboarding complete")
        .expect("transcript should include success section");
    transcript[start..].to_vec()
}

fn start_local_model_probe_server(
    expected_requests: usize,
) -> (SocketAddr, std::thread::JoinHandle<Vec<String>>) {
    start_local_model_probe_server_with_models_response(
        expected_requests,
        "HTTP/1.1 200 OK",
        r#"{"data":[{"id":"openai/gpt-5.1-codex"}]}"#,
    )
}

fn start_local_model_probe_server_with_models_response(
    expected_requests: usize,
    models_status_line: &str,
    models_body: &str,
) -> (SocketAddr, std::thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local provider test listener");
    let addr = listener.local_addr().expect("local addr");
    let models_status_line = models_status_line.to_owned();
    let models_body = models_body.to_owned();
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for _ in 0..expected_requests {
            let (mut stream, _) = listener.accept().expect("accept local provider request");
            let mut request_buf = [0_u8; 8192];
            let len = stream.read(&mut request_buf).expect("read request");
            let request = String::from_utf8_lossy(&request_buf[..len]).to_string();
            requests.push(request.clone());

            let (status_line, body) = if request.starts_with("GET /v1/models ") {
                (models_status_line.as_str(), models_body.clone())
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
    (addr, server)
}

fn default_non_interactive_onboard_options(
    output: &std::path::Path,
) -> loongclaw_daemon::onboard_cli::OnboardCommandOptions {
    loongclaw_daemon::onboard_cli::OnboardCommandOptions {
        output: Some(output.display().to_string()),
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
    }
}

#[test]
fn scripted_onboard_ui_select_one_accepts_slug_input() {
    let mut ui = ScriptedOnboardUi::new(["friendly_collab"]);
    let options = vec![
        loongclaw_daemon::onboard_cli::SelectOption {
            label: "calm engineering".to_owned(),
            slug: "calm_engineering".to_owned(),
            description: String::new(),
            recommended: true,
        },
        loongclaw_daemon::onboard_cli::SelectOption {
            label: "friendly collab".to_owned(),
            slug: "friendly_collab".to_owned(),
            description: String::new(),
            recommended: false,
        },
    ];

    let index = loongclaw_daemon::onboard_cli::OnboardUi::select_one(
        &mut ui,
        "Personality",
        &options,
        Some(0),
        loongclaw_daemon::onboard_cli::SelectInteractionMode::List,
    )
    .expect("scripted selection should accept slug input so integration tests stay aligned");

    assert_eq!(index, 1);
    assert_eq!(ui.transcript(), vec!["SELECT Personality".to_owned()]);
}

#[test]
fn parse_provider_kind_accepts_primary_and_legacy_aliases() {
    assert_eq!(
        loongclaw_daemon::onboard_cli::parse_provider_kind("openai"),
        Some(mvp::config::ProviderKind::Openai)
    );
    assert_eq!(
        loongclaw_daemon::onboard_cli::parse_provider_kind("bedrock"),
        Some(mvp::config::ProviderKind::Bedrock)
    );
    assert_eq!(
        loongclaw_daemon::onboard_cli::parse_provider_kind("byteplus"),
        Some(mvp::config::ProviderKind::Byteplus)
    );
    assert_eq!(
        loongclaw_daemon::onboard_cli::parse_provider_kind("byteplus_coding_compatible"),
        Some(mvp::config::ProviderKind::ByteplusCoding)
    );
    assert_eq!(
        loongclaw_daemon::onboard_cli::parse_provider_kind("custom"),
        Some(mvp::config::ProviderKind::Custom)
    );
    assert_eq!(
        loongclaw_daemon::onboard_cli::parse_provider_kind("openrouter_compatible"),
        Some(mvp::config::ProviderKind::Openrouter)
    );
    assert_eq!(
        loongclaw_daemon::onboard_cli::parse_provider_kind("volcengine_custom"),
        Some(mvp::config::ProviderKind::Volcengine)
    );
    assert_eq!(
        loongclaw_daemon::onboard_cli::parse_provider_kind("kimi_coding"),
        Some(mvp::config::ProviderKind::KimiCoding)
    );
    assert_eq!(
        loongclaw_daemon::onboard_cli::parse_provider_kind("kimi_coding_compatible"),
        Some(mvp::config::ProviderKind::KimiCoding)
    );
    assert_eq!(
        loongclaw_daemon::onboard_cli::parse_provider_kind("volcengine_coding"),
        Some(mvp::config::ProviderKind::VolcengineCoding)
    );
    assert_eq!(
        loongclaw_daemon::onboard_cli::parse_provider_kind("unsupported"),
        None
    );
}

#[test]
fn provider_default_env_mapping_is_stable() {
    assert_eq!(
        loongclaw_daemon::onboard_cli::provider_default_api_key_env(
            mvp::config::ProviderKind::Openai
        ),
        Some("OPENAI_API_KEY")
    );
    assert_eq!(
        loongclaw_daemon::onboard_cli::provider_default_api_key_env(
            mvp::config::ProviderKind::Anthropic
        ),
        Some("ANTHROPIC_API_KEY")
    );
    assert_eq!(
        loongclaw_daemon::onboard_cli::provider_default_api_key_env(
            mvp::config::ProviderKind::Bedrock
        ),
        Some("AWS_BEARER_TOKEN_BEDROCK")
    );
    assert_eq!(
        loongclaw_daemon::onboard_cli::provider_default_api_key_env(
            mvp::config::ProviderKind::Byteplus
        ),
        Some("BYTEPLUS_API_KEY")
    );
    assert_eq!(
        loongclaw_daemon::onboard_cli::provider_default_api_key_env(
            mvp::config::ProviderKind::ByteplusCoding
        ),
        Some("BYTEPLUS_API_KEY")
    );
    assert_eq!(
        loongclaw_daemon::onboard_cli::provider_default_api_key_env(
            mvp::config::ProviderKind::Custom
        ),
        Some("CUSTOM_PROVIDER_API_KEY")
    );
    assert_eq!(
        loongclaw_daemon::onboard_cli::provider_default_api_key_env(
            mvp::config::ProviderKind::Openrouter
        ),
        Some("OPENROUTER_API_KEY")
    );
    assert_eq!(
        loongclaw_daemon::onboard_cli::provider_default_api_key_env(
            mvp::config::ProviderKind::KimiCoding
        ),
        Some("KIMI_CODING_API_KEY")
    );
}

#[test]
fn provider_kind_id_mapping_includes_kimi_coding() {
    assert_eq!(
        loongclaw_daemon::onboard_cli::provider_kind_id(mvp::config::ProviderKind::KimiCoding),
        "kimi_coding"
    );
    assert_eq!(
        loongclaw_daemon::onboard_cli::provider_kind_id(mvp::config::ProviderKind::Byteplus),
        "byteplus"
    );
    assert_eq!(
        loongclaw_daemon::onboard_cli::provider_kind_id(mvp::config::ProviderKind::ByteplusCoding),
        "byteplus_coding"
    );
    assert_eq!(
        loongclaw_daemon::onboard_cli::provider_kind_id(
            mvp::config::ProviderKind::VolcengineCoding
        ),
        "volcengine_coding"
    );
    assert_eq!(
        loongclaw_daemon::onboard_cli::provider_kind_id(mvp::config::ProviderKind::Bedrock),
        "bedrock"
    );
    assert_eq!(
        loongclaw_daemon::onboard_cli::provider_kind_id(mvp::config::ProviderKind::Custom),
        "custom"
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
fn supported_provider_list_matches_canonical_provider_catalog() {
    let expected = mvp::config::ProviderKind::all_sorted()
        .iter()
        .map(|kind| kind.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    assert_eq!(
        loongclaw_daemon::onboard_cli::supported_provider_list(),
        expected
    );
}

#[test]
fn non_interactive_requires_explicit_risk_acknowledgement() {
    let denied = loongclaw_daemon::onboard_cli::validate_non_interactive_risk_gate(true, false)
        .expect_err("risk gate should reject non-interactive without acknowledgement");
    assert!(denied.contains("--accept-risk"));

    loongclaw_daemon::onboard_cli::validate_non_interactive_risk_gate(true, true)
        .expect("risk gate should pass after acknowledgement");
    loongclaw_daemon::onboard_cli::validate_non_interactive_risk_gate(false, false)
        .expect("interactive mode should not require explicit flag");
}

#[tokio::test(flavor = "current_thread")]
async fn non_interactive_personality_and_memory_profile_are_persisted() {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    unsafe {
        std::env::set_var("OPENAI_API_KEY", "openai-test-token");
    }

    let output_path = unique_temp_path("non-interactive-personality-memory-config.toml");
    let transcript = run_scripted_onboard_flow(
        crate::onboard_cli::OnboardCommandOptions {
            output: output_path.to_str().map(str::to_owned),
            force: false,
            non_interactive: true,
            accept_risk: true,
            provider: Some("openai".to_owned()),
            model: Some("openai/gpt-5.1".to_owned()),
            api_key_env: Some("OPENAI_API_KEY".to_owned()),
            web_search_provider: None,
            web_search_api_key_env: None,
            personality: Some("friendly_collab".to_owned()),
            memory_profile: Some("profile_plus_window".to_owned()),
            system_prompt: None,
            skip_model_probe: true,
        },
        std::iter::empty::<String>(),
        None,
        None,
    )
    .await
    .expect("run non-interactive onboarding with personality and memory profile");

    assert!(
        transcript
            .iter()
            .any(|line| line.contains("onboarding complete")),
        "non-interactive personality/memory path should still complete successfully: {transcript:#?}"
    );

    let (_, config) = mvp::config::load(output_path.to_str())
        .expect("load non-interactive personality/memory config");
    assert_eq!(
        config.cli.personality,
        Some(mvp::prompt::PromptPersonality::FriendlyCollab)
    );
    assert_eq!(
        config.memory.profile,
        mvp::config::MemoryProfile::ProfilePlusWindow
    );
}

#[tokio::test(flavor = "current_thread")]
async fn non_interactive_system_prompt_override_disables_prompt_pack() {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    unsafe {
        std::env::set_var("OPENAI_API_KEY", "openai-test-token");
    }

    let output_path = unique_temp_path("non-interactive-inline-prompt-config.toml");
    let transcript = run_scripted_onboard_flow(
        crate::onboard_cli::OnboardCommandOptions {
            output: output_path.to_str().map(str::to_owned),
            force: false,
            non_interactive: true,
            accept_risk: true,
            provider: Some("openai".to_owned()),
            model: Some("openai/gpt-5.1".to_owned()),
            api_key_env: Some("OPENAI_API_KEY".to_owned()),
            web_search_provider: None,
            web_search_api_key_env: None,
            personality: None,
            memory_profile: None,
            system_prompt: Some("Stay concise and technical.".to_owned()),
            skip_model_probe: true,
        },
        std::iter::empty::<String>(),
        None,
        None,
    )
    .await
    .expect("run non-interactive onboarding with an inline system prompt override");

    assert!(
        transcript
            .iter()
            .any(|line| line.contains("onboarding complete")),
        "non-interactive inline override path should still complete successfully: {transcript:#?}"
    );

    let (_, config) = mvp::config::load(output_path.to_str()).expect("load inline override config");

    assert!(
        !config.cli.uses_native_prompt_pack(),
        "explicit inline prompt override should disable the native prompt pack metadata"
    );
    assert_eq!(
        config.cli.system_prompt_addendum, None,
        "inline override should not keep a native prompt addendum behind"
    );
    assert_eq!(config.cli.system_prompt, "Stay concise and technical.");
}

#[tokio::test(flavor = "current_thread")]
async fn non_interactive_onboard_rejects_unresolved_preflight_warnings() {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    let root = unique_temp_path("non-interactive-warning-root");
    std::fs::create_dir_all(&root).expect("create test root");
    let output = root.join("loongclaw.toml");

    let (addr, server) = start_local_model_probe_server(1);

    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.base_url = format!("http://{addr}");
    config.provider.model = "openai/gpt-5.1-codex".to_owned();
    config.provider.wire_api = mvp::config::ProviderWireApi::Responses;
    config.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "test-openai-key".to_owned(),
    ));
    mvp::config::write(Some(output.to_string_lossy().as_ref()), &config, true)
        .expect("write existing config");

    let mut options = default_non_interactive_onboard_options(&output);
    options.system_prompt = Some("force a pending write".to_owned());
    let mut ui = ScriptedOnboardUi::new(std::iter::empty::<String>());
    let context =
        loongclaw_daemon::onboard_cli::OnboardRuntimeContext::new_for_tests(80, None, None);
    let error = loongclaw_daemon::onboard_cli::run_onboard_cli_with_ui(options, &mut ui, &context)
        .await
        .expect_err("non-interactive onboarding should stop on unresolved warnings");

    assert!(
        error.contains("provider transport:"),
        "non-interactive warning-gate errors should surface the first blocking warning detail: {error}"
    );
    assert!(
        error.contains("rerun without --non-interactive"),
        "unexpected warning-gate error: {error}"
    );

    let requests = server.join().expect("join local provider server");
    assert!(
        requests.iter().any(|request| {
            let normalized = request.to_ascii_lowercase();
            request.starts_with("GET /v1/models ")
                && normalized.contains("authorization: bearer test-openai-key")
        }),
        "warning reproduction should still perform the model probe before the warning gate: {requests:#?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn non_interactive_explicit_web_search_provider_does_not_silently_fall_back() {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    let output = unique_temp_path("non-interactive-explicit-web-search.toml");
    let _openai_env = unsafe { EnvVarGuard::set_unlocked("OPENAI_API_KEY", "openai-test-token") };

    let error = run_scripted_onboard_flow(
        crate::onboard_cli::OnboardCommandOptions {
            output: output.to_str().map(str::to_owned),
            force: false,
            non_interactive: true,
            accept_risk: true,
            provider: Some("openai".to_owned()),
            model: Some("openai/gpt-5.1".to_owned()),
            api_key_env: Some("OPENAI_API_KEY".to_owned()),
            web_search_provider: Some("tavily".to_owned()),
            web_search_api_key_env: None,
            personality: None,
            memory_profile: None,
            system_prompt: None,
            skip_model_probe: true,
        },
        std::iter::empty::<String>(),
        None,
        None,
    )
    .await
    .expect_err("missing Tavily credentials should fail instead of silently falling back");

    assert!(
        error.contains("Tavily") || error.contains("TAVILY_API_KEY"),
        "preflight should keep the explicit Tavily selection visible in the failure path: {error}"
    );
    assert!(
        !error.contains("DuckDuckGo"),
        "explicit web-search selection should not be silently replaced by DuckDuckGo in the failure path: {error}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn non_interactive_onboard_keeps_matching_existing_config_despite_persistent_warnings() {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    let root = unique_temp_path("non-interactive-warning-noop-root");
    std::fs::create_dir_all(&root).expect("create test root");
    let output = root.join("loongclaw.toml");
    unsafe {
        std::env::set_var("DEEPSEEK_API_KEY", "test-deepseek-key");
    }

    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Deepseek;
    config.provider.model = "deepseek-chat".to_owned();
    config.provider.wire_api = mvp::config::ProviderWireApi::Responses;
    config.provider.api_key_env = Some("DEEPSEEK_API_KEY".to_owned());
    mvp::config::write(Some(output.to_string_lossy().as_ref()), &config, true)
        .expect("write existing config");

    let raw = std::fs::read_to_string(&output).expect("read written config");
    let legacy_raw = raw.replace(
        "api_key = \"${DEEPSEEK_API_KEY}\"",
        "api_key_env = \"DEEPSEEK_API_KEY\"",
    );
    std::fs::write(&output, legacy_raw).expect("rewrite config to legacy api_key_env form");
    let original_body = std::fs::read_to_string(&output).expect("read original config");

    let mut options = default_non_interactive_onboard_options(&output);
    options.skip_model_probe = true;

    let mut ui = ScriptedOnboardUi::new(std::iter::empty::<String>());
    let context =
        loongclaw_daemon::onboard_cli::OnboardRuntimeContext::new_for_tests(80, None, None);
    loongclaw_daemon::onboard_cli::run_onboard_cli_with_ui(options, &mut ui, &context)
        .await
        .expect("matching existing config should stay a successful no-op even when persistent warnings remain");

    assert_eq!(
        std::fs::read_to_string(&output).expect("read config after no-op"),
        original_body,
        "no-op onboarding should not rewrite the existing config just to re-encode the same settings"
    );
    let transcript = ui.transcript();
    assert!(
        transcript
            .iter()
            .any(|line| line.contains("existing config kept; no changes were needed")),
        "successful no-op path should still report that the existing config was reused: {:#?}",
        transcript
    );
}

#[tokio::test(flavor = "current_thread")]
async fn non_interactive_onboard_allows_explicit_skip_model_probe_warning() {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    let root = unique_temp_path("non-interactive-skip-model-probe-root");
    std::fs::create_dir_all(&root).expect("create test root");
    let output = root.join("loongclaw.toml");
    unsafe {
        std::env::set_var("OPENAI_API_KEY", "test-openai-key");
    }

    let mut options = default_non_interactive_onboard_options(&output);
    options.skip_model_probe = true;
    options.model = Some("openai/gpt-5.1-codex".to_owned());

    let mut ui = ScriptedOnboardUi::new(std::iter::empty::<String>());
    let context =
        loongclaw_daemon::onboard_cli::OnboardRuntimeContext::new_for_tests(80, None, None);
    loongclaw_daemon::onboard_cli::run_onboard_cli_with_ui(options, &mut ui, &context)
        .await
        .expect("explicitly skipped model probe should not block non-interactive onboarding");

    let raw = std::fs::read_to_string(&output).expect("read written onboarding config");
    assert!(
        raw.contains("oauth_access_token_env = \"OPENAI_CODEX_OAUTH_TOKEN\""),
        "onboarding should persist the openai oauth env name after provider-aligned credential routing: {raw}"
    );
    assert!(
        !raw.contains("oauth_access_token = "),
        "provider-aligned onboarding should not fall back to the legacy oauth_access_token field for the openai oauth route: {raw}"
    );
    assert!(
        !raw.contains("api_key = "),
        "provider-aligned onboarding should not fall back to the legacy api_key field for the openai oauth route: {raw}"
    );

    let (_, config) = mvp::config::load(Some(output.to_string_lossy().as_ref()))
        .expect("load written onboarding config");
    assert_eq!(config.provider.model, "openai/gpt-5.1-codex");
    assert_eq!(
        config.provider.oauth_access_token, None,
        "reloaded config should not repopulate the legacy oauth_access_token field for the oauth-backed openai route"
    );
    assert_eq!(
        config.provider.oauth_access_token_env.as_deref(),
        Some("OPENAI_CODEX_OAUTH_TOKEN"),
        "reloaded config should keep the routed oauth env name after provider-aligned onboarding"
    );
    assert_eq!(
        config.provider.authorization_header(),
        Some("Bearer test-openai-key".to_owned()),
        "runtime auth resolution should still fall back to OPENAI_API_KEY when the oauth env is unset"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn non_interactive_onboard_applies_reviewed_default_when_probe_is_skipped() {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    let root = unique_temp_path("non-interactive-reviewed-auto-skip-root");
    std::fs::create_dir_all(&root).expect("create test root");
    let output = root.join("loongclaw.toml");
    unsafe {
        std::env::set_var("DEEPSEEK_API_KEY", "test-deepseek-key");
    }

    let mut options = default_non_interactive_onboard_options(&output);
    options.provider = Some("deepseek".to_owned());
    options.skip_model_probe = true;

    let mut ui = ScriptedOnboardUi::new(std::iter::empty::<String>());
    let context =
        loongclaw_daemon::onboard_cli::OnboardRuntimeContext::new_for_tests(80, None, None);
    loongclaw_daemon::onboard_cli::run_onboard_cli_with_ui(options, &mut ui, &context)
        .await
        .expect("skip-model-probe should allow non-interactive onboarding to materialize the reviewed default model");

    let raw = std::fs::read_to_string(&output).expect("read written onboarding config");
    let (_, config) = mvp::config::load(Some(output.to_string_lossy().as_ref()))
        .expect("load written onboarding config");
    assert_eq!(config.provider.kind, mvp::config::ProviderKind::Deepseek);
    assert_eq!(
        config.provider.model, "deepseek-chat",
        "non-interactive onboarding should materialize the reviewed provider default so skip-model-probe still leaves a usable explicit model"
    );
    assert!(
        raw.contains("model = \"deepseek-chat\""),
        "skip-model-probe onboarding should persist the reviewed model explicitly instead of leaving the config on auto: {raw}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn non_interactive_onboard_allows_explicit_model_probe_warning() {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    let root = unique_temp_path("non-interactive-explicit-model-warning-root");
    std::fs::create_dir_all(&root).expect("create test root");
    let output = root.join("loongclaw.toml");

    let (addr, server) = start_local_model_probe_server_with_models_response(
        1,
        "HTTP/1.1 401 Unauthorized",
        r#"{"error":{"message":"No cookie auth credentials found"}}"#,
    );

    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.base_url = format!("http://{addr}");
    config.provider.model = "openai/gpt-5.1-codex".to_owned();
    config.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "test-openai-key".to_owned(),
    ));
    mvp::config::write(Some(output.to_string_lossy().as_ref()), &config, true)
        .expect("write existing config");

    let mut options = default_non_interactive_onboard_options(&output);
    options.system_prompt = Some("force a pending write".to_owned());
    options.force = true;

    let mut ui = ScriptedOnboardUi::new(std::iter::empty::<String>());
    let context = crate::onboard_cli::OnboardRuntimeContext::new_for_tests(80, None, None);
    crate::onboard_cli::run_onboard_cli_with_ui(options, &mut ui, &context)
        .await
        .expect("explicit-model probe warnings should not block non-interactive onboarding");

    let requests = server.join().expect("join local provider server");
    assert!(
        requests.iter().any(|request| {
            let normalized = request.to_ascii_lowercase();
            request.starts_with("GET /v1/models ")
                && normalized.contains("authorization: bearer test-openai-key")
        }),
        "explicit-model warning path should still perform the model probe with resolved auth before allowing onboarding to continue: {requests:#?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn non_interactive_onboard_applies_reviewed_default_when_probe_fails() {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    let root = unique_temp_path("non-interactive-reviewed-auto-failure-root");
    std::fs::create_dir_all(&root).expect("create test root");
    let output = root.join("loongclaw.toml");

    let (addr, server) = start_local_model_probe_server_with_models_response(
        1,
        "HTTP/1.1 401 Unauthorized",
        r#"{"error":{"message":"No cookie auth credentials found"}}"#,
    );

    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Deepseek;
    config.provider.base_url = format!("http://{addr}");
    config.provider.model = "auto".to_owned();
    config.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "test-deepseek-key".to_owned(),
    ));
    mvp::config::write(Some(output.to_string_lossy().as_ref()), &config, true)
        .expect("write existing config");

    let mut options = default_non_interactive_onboard_options(&output);
    options.force = true;
    options.system_prompt = Some("force a pending write".to_owned());

    let mut ui = ScriptedOnboardUi::new(std::iter::empty::<String>());
    let context = crate::onboard_cli::OnboardRuntimeContext::new_for_tests(80, None, None);
    crate::onboard_cli::run_onboard_cli_with_ui(options, &mut ui, &context)
        .await
        .expect("reviewed onboarding defaults should let non-interactive onboarding pin a usable explicit model even when catalog probing fails");

    let written = std::fs::read_to_string(&output).expect("read config after onboarding");
    assert!(
        written.contains("model = \"deepseek-chat\""),
        "non-interactive onboarding should persist the reviewed model explicitly instead of leaving the provider on auto after a probe warning: {written}"
    );
    let (_, written_config) = mvp::config::load(Some(output.to_string_lossy().as_ref()))
        .expect("load written onboarding config");
    assert_eq!(written_config.provider.model, "deepseek-chat");

    let requests = server.join().expect("join local provider server");
    assert!(
        requests.iter().any(|request| {
            let normalized = request.to_ascii_lowercase();
            request.starts_with("GET /v1/models ")
                && normalized.contains("authorization: bearer test-deepseek-key")
        }),
        "reviewed-default onboarding should still attempt the provider model probe with resolved auth before falling back to the explicit reviewed model: {requests:#?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn non_interactive_api_key_env_override_clears_existing_oauth_credentials() {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    let root = unique_temp_path("non-interactive-api-key-env-override-root");
    std::fs::create_dir_all(&root).expect("create test root");
    let output = root.join("loongclaw.toml");
    unsafe {
        std::env::set_var("OPENAI_API_KEY", "test-openai-key");
    }

    let mut existing = mvp::config::LoongClawConfig::default();
    existing.provider.model = "openai/gpt-5.1-codex".to_owned();
    existing.provider.oauth_access_token = Some(loongclaw_contracts::SecretRef::Inline(
        "${OPENAI_CODEX_OAUTH_TOKEN}".to_owned(),
    ));
    mvp::config::write(Some(output.to_string_lossy().as_ref()), &existing, true)
        .expect("write existing config with oauth credential");

    let mut options = default_non_interactive_onboard_options(&output);
    options.force = true;
    options.skip_model_probe = true;
    options.api_key_env = Some("OPENAI_API_KEY".to_owned());
    options.model = Some("openai/gpt-5.1-codex".to_owned());

    let mut ui = ScriptedOnboardUi::new(std::iter::empty::<String>());
    let context =
        loongclaw_daemon::onboard_cli::OnboardRuntimeContext::new_for_tests(80, None, None);
    loongclaw_daemon::onboard_cli::run_onboard_cli_with_ui(options, &mut ui, &context)
        .await
        .expect("explicit api key env override should succeed");

    let raw = std::fs::read_to_string(&output).expect("read written onboarding config");
    assert!(
        raw.contains("api_key_env = \"OPENAI_API_KEY\""),
        "api key env override should persist the selected api key source: {raw}"
    );
    assert!(
        !raw.contains("OPENAI_CODEX_OAUTH_TOKEN"),
        "api key env override should clear stale oauth credentials instead of keeping both sources: {raw}"
    );

    let (_, config) = mvp::config::load(Some(output.to_string_lossy().as_ref()))
        .expect("load written onboarding config");
    assert_eq!(config.provider.oauth_access_token, None);
    assert_eq!(config.provider.oauth_access_token_env, None);
    assert_eq!(
        config.provider.api_key, None,
        "reloaded config should not keep the legacy inline api_key field once the env name field is persisted"
    );
    assert_eq!(
        config.provider.api_key_env.as_deref(),
        Some("OPENAI_API_KEY"),
        "reloaded config should keep only the selected api key credential source"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn non_interactive_api_key_env_override_clears_existing_inline_api_key() {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    let root = unique_temp_path("non-interactive-inline-api-key-override-root");
    std::fs::create_dir_all(&root).expect("create test root");
    let output = root.join("loongclaw.toml");
    unsafe {
        std::env::set_var("OPENAI_API_KEY", "test-openai-key");
    }

    let mut existing = mvp::config::LoongClawConfig::default();
    existing.provider.model = "openai/gpt-5.1-codex".to_owned();
    existing.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "inline-secret".to_owned(),
    ));
    mvp::config::write(Some(output.to_string_lossy().as_ref()), &existing, true)
        .expect("write existing config with inline api key");

    let mut options = default_non_interactive_onboard_options(&output);
    options.force = true;
    options.skip_model_probe = true;
    options.api_key_env = Some("OPENAI_API_KEY".to_owned());
    options.model = Some("openai/gpt-5.1-codex".to_owned());

    let mut ui = ScriptedOnboardUi::new(std::iter::empty::<String>());
    let context =
        loongclaw_daemon::onboard_cli::OnboardRuntimeContext::new_for_tests(80, None, None);
    loongclaw_daemon::onboard_cli::run_onboard_cli_with_ui(options, &mut ui, &context)
        .await
        .expect("explicit api key env override should succeed");

    let raw = std::fs::read_to_string(&output).expect("read written onboarding config");
    assert!(
        raw.contains("api_key_env = \"OPENAI_API_KEY\""),
        "api key env override should persist the selected api key source: {raw}"
    );
    assert!(
        !raw.contains("inline-secret"),
        "api key env override should remove the old inline secret instead of persisting both credential forms: {raw}"
    );

    let (_, config) = mvp::config::load(Some(output.to_string_lossy().as_ref()))
        .expect("load written onboarding config");
    assert_eq!(
        config.provider.api_key, None,
        "reloaded config should not keep the legacy inline api_key field once the env name field is persisted"
    );
    assert_eq!(
        config.provider.api_key_env.as_deref(),
        Some("OPENAI_API_KEY"),
        "reloaded config should keep only the selected api key env reference"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn non_interactive_api_key_env_clear_keeps_existing_inline_credential() {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    let root = unique_temp_path("non-interactive-api-key-env-clear-root");
    std::fs::create_dir_all(&root).expect("create test root");
    let output = root.join("loongclaw.toml");

    let mut existing = mvp::config::LoongClawConfig::default();
    existing.provider.model = "openai/gpt-5.1-codex".to_owned();
    existing.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "inline-secret".to_owned(),
    ));
    existing.provider.api_key_env = Some("OPENAI_API_KEY".to_owned());
    mvp::config::write(Some(output.to_string_lossy().as_ref()), &existing, true)
        .expect("write existing config with inline credential and env binding");

    let mut options = default_non_interactive_onboard_options(&output);
    options.force = true;
    options.skip_model_probe = true;
    options.model = Some("openai/gpt-5.1-codex".to_owned());
    options.api_key_env = Some(":clear".to_owned());

    let mut ui = ScriptedOnboardUi::new(std::iter::empty::<String>());
    let context =
        loongclaw_daemon::onboard_cli::OnboardRuntimeContext::new_for_tests(80, None, None);
    loongclaw_daemon::onboard_cli::run_onboard_cli_with_ui(options, &mut ui, &context)
        .await
        .expect("explicit clear token should keep the existing inline credential");

    let raw = std::fs::read_to_string(&output).expect("read written onboarding config");
    assert!(
        !raw.contains("OPENAI_API_KEY"),
        "explicit clear token should remove the api-key env binding in non-interactive onboarding: {raw}"
    );

    let (_, config) = mvp::config::load(Some(output.to_string_lossy().as_ref()))
        .expect("load written onboarding config");
    assert_eq!(
        config
            .provider
            .api_key
            .as_ref()
            .and_then(|value| value.inline_literal_value()),
        Some("inline-secret"),
        "explicit clear token should preserve the existing inline provider credential"
    );
    assert_eq!(config.provider.api_key_env, None);
}

#[tokio::test(flavor = "current_thread")]
async fn non_interactive_system_prompt_clear_restores_builtin_prompt() {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    let root = unique_temp_path("non-interactive-system-prompt-clear-root");
    std::fs::create_dir_all(&root).expect("create test root");
    let output = root.join("loongclaw.toml");

    let mut existing = mvp::config::LoongClawConfig::default();
    existing.provider.model = "openai/gpt-5.1-codex".to_owned();
    existing.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "inline-secret".to_owned(),
    ));
    existing.cli.system_prompt = "custom review prompt".to_owned();
    mvp::config::write(Some(output.to_string_lossy().as_ref()), &existing, true)
        .expect("write existing config with custom system prompt");

    let mut options = default_non_interactive_onboard_options(&output);
    options.force = true;
    options.skip_model_probe = true;
    options.model = Some("openai/gpt-5.1-codex".to_owned());
    options.system_prompt = Some(":clear".to_owned());

    let mut ui = ScriptedOnboardUi::new(std::iter::empty::<String>());
    let context =
        loongclaw_daemon::onboard_cli::OnboardRuntimeContext::new_for_tests(80, None, None);
    loongclaw_daemon::onboard_cli::run_onboard_cli_with_ui(options, &mut ui, &context)
        .await
        .expect("explicit clear token should restore the built-in system prompt");

    let (_, config) = mvp::config::load(Some(output.to_string_lossy().as_ref()))
        .expect("load written onboarding config");
    assert_eq!(
        config.cli.system_prompt,
        mvp::config::CliChannelConfig::default().system_prompt,
        "non-interactive clear token should restore the built-in CLI system prompt"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn interactive_onboard_clear_token_keeps_inline_provider_credential() {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    let output_path = unique_temp_path("interactive-clear-inline-credential.toml");
    let mut existing = mvp::config::LoongClawConfig::default();
    existing.provider.model = "gpt-4.1".to_owned();
    existing.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "inline-secret".to_owned(),
    ));
    existing.provider.api_key_env = Some("OPENAI_API_KEY".to_owned());
    mvp::config::write(output_path.to_str(), &existing, true).expect("write existing config");

    let transcript = run_scripted_onboard_flow(
        loongclaw_daemon::onboard_cli::OnboardCommandOptions {
            output: output_path.to_str().map(str::to_owned),
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
            skip_model_probe: true,
        },
        vec![
            "1".to_owned(),
            "2".to_owned(),
            provider_choice_input(mvp::config::ProviderKind::Openai),
            "gpt-4.1".to_owned(),
            ":clear".to_owned(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            "y".to_owned(),
            "y".to_owned(),
            "o".to_owned(),
        ],
        None,
        None,
    )
    .await
    .expect("run scripted onboarding with explicit credential clear token");

    let joined = transcript.join("\n");
    assert!(
        joined.contains("SELECT Provider"),
        "provider fallback should use numbered selection even without detected provider choices: {transcript:#?}"
    );
    assert!(
        !joined.contains("PROMPT Provider"),
        "provider fallback should no longer ask for free-form provider text input: {transcript:#?}"
    );

    let raw = std::fs::read_to_string(&output_path).expect("read written onboarding config");
    assert!(
        !raw.contains("OPENAI_API_KEY"),
        "explicit :clear should remove the api-key env binding instead of persisting it: {raw}"
    );

    let (_, config) =
        mvp::config::load(output_path.to_str()).expect("load interactive onboarding config");
    assert_eq!(
        config
            .provider
            .api_key
            .as_ref()
            .and_then(|value| value.inline_literal_value()),
        Some("inline-secret"),
        "explicit :clear should keep the existing inline provider credential in the saved config: {transcript:#?}"
    );
    assert_eq!(config.provider.api_key_env, None);
}

#[tokio::test(flavor = "current_thread")]
async fn interactive_onboard_clear_token_restores_builtin_system_prompt() {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    let output_path = unique_temp_path("interactive-clear-system-prompt.toml");
    let mut existing = mvp::config::LoongClawConfig::default();
    existing.provider.model = "gpt-4.1".to_owned();
    existing.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "inline-secret".to_owned(),
    ));
    existing.cli.system_prompt = "custom review prompt".to_owned();
    mvp::config::write(output_path.to_str(), &existing, true).expect("write existing config");

    let mut ui = ScriptedOnboardUi::new(vec![
        "1".to_owned(),
        provider_choice_input(mvp::config::ProviderKind::Openai),
        "gpt-4.1".to_owned(),
        String::new(),
        ":clear".to_owned(),
        String::new(),
        String::new(),
        "y".to_owned(),
        "y".to_owned(),
        "o".to_owned(),
    ]);
    let context =
        loongclaw_daemon::onboard_cli::OnboardRuntimeContext::new_for_tests(80, None, None);
    loongclaw_daemon::onboard_cli::run_onboard_cli_with_ui(
        loongclaw_daemon::onboard_cli::OnboardCommandOptions {
            output: output_path.to_str().map(str::to_owned),
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
            system_prompt: Some(existing.cli.system_prompt.clone()),
            skip_model_probe: true,
        },
        &mut ui,
        &context,
    )
    .await
    .unwrap_or_else(|error| {
        panic!(
            "run scripted onboarding with explicit system-prompt clear token: {error}; transcript: {:#?}",
            ui.transcript()
        )
    });

    let (_, config) =
        mvp::config::load(output_path.to_str()).expect("load interactive onboarding config");
    assert_eq!(
        config.cli.system_prompt,
        mvp::config::CliChannelConfig::default().system_prompt,
        "explicit :clear should restore the built-in CLI system prompt"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn interactive_onboard_web_search_custom_env_persists_explicit_env_reference() {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    let _openai_env = unsafe { EnvVarGuard::set_unlocked("OPENAI_API_KEY", "openai-test-token") };
    let _tavily_env = unsafe { EnvVarGuard::set_unlocked("TEAM_TAVILY_KEY", "tavily-test-token") };
    let output_path = unique_temp_path("interactive-web-search-env.toml");
    let mut existing = mvp::config::LoongClawConfig::default();
    existing.provider.model = "gpt-4.1".to_owned();
    existing.provider.api_key_env = Some("OPENAI_API_KEY".to_owned());
    mvp::config::write(output_path.to_str(), &existing, true).expect("write existing config");

    let transcript = run_scripted_onboard_flow(
        loongclaw_daemon::onboard_cli::OnboardCommandOptions {
            output: output_path.to_str().map(str::to_owned),
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
            skip_model_probe: true,
        },
        vec![
            "1".to_owned(),
            "2".to_owned(),
            provider_choice_input(mvp::config::ProviderKind::Openai),
            "gpt-4.1".to_owned(),
            "OPENAI_API_KEY".to_owned(),
            String::new(),
            String::new(),
            String::new(),
            "tavily".to_owned(),
            "TEAM_TAVILY_KEY".to_owned(),
            "y".to_owned(),
            "y".to_owned(),
            "o".to_owned(),
        ],
        None,
        None,
    )
    .await
    .expect("run scripted onboarding with custom web search env");

    let joined = transcript.join("\n");
    assert!(
        joined.contains("choose web search credential"),
        "interactive onboarding should prompt for a web-search credential source when the selected provider requires one: {transcript:#?}"
    );

    let (_, config) =
        mvp::config::load(output_path.to_str()).expect("load onboarding config with web search");
    assert_eq!(
        config.tools.web_search.default_provider,
        mvp::config::WEB_SEARCH_PROVIDER_TAVILY
    );
    assert_eq!(
        config.tools.web_search.tavily_api_key.as_deref(),
        Some("${TEAM_TAVILY_KEY}"),
        "interactive onboarding should persist the selected web-search env as an explicit env reference"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn interactive_onboard_web_search_blank_input_keeps_inline_credential() {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    let _openai_env = unsafe { EnvVarGuard::set_unlocked("OPENAI_API_KEY", "openai-test-token") };
    let output_path = unique_temp_path("interactive-web-search-inline.toml");
    let mut existing = mvp::config::LoongClawConfig::default();
    existing.provider.model = "gpt-4.1".to_owned();
    existing.provider.api_key_env = Some("OPENAI_API_KEY".to_owned());
    existing.tools.web_search.default_provider = mvp::config::WEB_SEARCH_PROVIDER_TAVILY.to_owned();
    existing.tools.web_search.tavily_api_key = Some("inline-web-search-secret".to_owned());
    mvp::config::write(output_path.to_str(), &existing, true).expect("write existing config");

    let transcript = run_scripted_onboard_flow(
        loongclaw_daemon::onboard_cli::OnboardCommandOptions {
            output: output_path.to_str().map(str::to_owned),
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
            skip_model_probe: true,
        },
        vec![
            "1".to_owned(),
            "2".to_owned(),
            provider_choice_input(mvp::config::ProviderKind::Openai),
            "gpt-4.1".to_owned(),
            "OPENAI_API_KEY".to_owned(),
            String::new(),
            String::new(),
            String::new(),
            "tavily".to_owned(),
            String::new(),
            "y".to_owned(),
            "y".to_owned(),
            "o".to_owned(),
        ],
        None,
        None,
    )
    .await
    .expect("run scripted onboarding while keeping inline web search credential");

    let joined = transcript.join("\n");
    assert!(
        joined.contains("leave this blank to keep inline credentials"),
        "web-search credential onboarding should explain how blank input preserves inline credentials: {transcript:#?}"
    );

    let (_, config) = mvp::config::load(output_path.to_str())
        .expect("load onboarding config with inline web search credential");
    assert_eq!(
        config.tools.web_search.tavily_api_key.as_deref(),
        Some("inline-web-search-secret"),
        "blank web-search credential input should preserve the existing inline credential"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn interactive_onboard_only_shows_large_logo_on_the_initial_screen() {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    unsafe {
        std::env::set_var("OPENAI_API_KEY", "openai-test-token");
    }

    let output_path = unique_temp_path("interactive-single-banner.toml");
    let mut existing = mvp::config::LoongClawConfig::default();
    existing.provider.model = "gpt-4.1".to_owned();
    existing.provider.api_key_env = Some("OPENAI_API_KEY".to_owned());
    mvp::config::write(output_path.to_str(), &existing, true).expect("write existing config");

    let transcript = run_scripted_onboard_flow(
        crate::onboard_cli::OnboardCommandOptions {
            output: output_path.to_str().map(str::to_owned),
            force: false,
            non_interactive: false,
            accept_risk: false,
            provider: None,
            model: None,
            api_key_env: None,
            web_search_provider: None,
            web_search_api_key_env: None,
            personality: None,
            memory_profile: None,
            system_prompt: None,
            skip_model_probe: true,
        },
        ["y", "1", "2", "", "", "", "", "", "", "", "y"],
        None,
        None,
    )
    .await
    .expect("run interactive onboarding with the risk gate enabled");

    assert_eq!(
        transcript
            .iter()
            .filter(|line| line.contains("██╗      ██████╗"))
            .count(),
        1,
        "interactive onboarding should show the large LOONGCLAW banner only once, on the initial risk screen: {transcript:#?}"
    );
    assert!(
        transcript
            .iter()
            .filter(|line| line.contains("LOONGCLAW"))
            .count()
            >= 3,
        "follow-up screens should keep using the compact LOONGCLAW header instead of dropping branding entirely: {transcript:#?}"
    );
    assert!(
        transcript.iter().any(|line| line == "choose personality"),
        "regression flow should still reach the later onboarding steps where repeated banner reports came from: {transcript:#?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn non_interactive_onboard_uses_the_same_detected_starting_point_order_as_interactive_default()
 {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    unsafe {
        std::env::set_var("OPENAI_API_KEY", "openai-test-token");
        std::env::set_var("DEEPSEEK_API_KEY", "deepseek-test-token");
    }

    let root = unique_temp_path("non-interactive-starting-point-order");
    std::fs::create_dir_all(&root).expect("create test root");
    let interactive_output = root.join("interactive.toml");
    let non_interactive_output = root.join("non-interactive.toml");

    let (addr, server) = start_local_model_probe_server(2);

    let z_openai_codex = root.join("z-openai.toml");
    std::fs::write(
        &z_openai_codex,
        format!(
            r#"
model_provider = "openai"
model = "openai/gpt-5.1-codex"

[model_providers.openai]
base_url = "http://{addr}"
wire_api = "chat_completions"
requires_openai_auth = true
"#
        ),
    )
    .expect("write openai codex config");

    let a_deepseek_codex = root.join("a-deepseek.toml");
    std::fs::write(
        &a_deepseek_codex,
        format!(
            r#"
model_provider = "deepseek"
model = "deepseek-chat"

[model_providers.deepseek]
base_url = "http://{addr}"
wire_api = "chat_completions"
requires_openai_auth = true
"#
        ),
    )
    .expect("write deepseek codex config");

    let codex_paths = vec![z_openai_codex.clone(), a_deepseek_codex.clone()];
    let interactive_context = loongclaw_daemon::onboard_cli::OnboardRuntimeContext::new_for_tests(
        80,
        None,
        codex_paths.clone(),
    );
    let interactive_transcript = run_scripted_onboard_flow_with_context(
        loongclaw_daemon::onboard_cli::OnboardCommandOptions {
            output: Some(interactive_output.display().to_string()),
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
        vec![
            "1".to_owned(),
            "1".to_owned(),
            "1".to_owned(),
            "y".to_owned(),
        ],
        interactive_context,
    )
    .await
    .expect("run interactive onboarding");

    let (_, interactive_config) = mvp::config::load(Some(
        interactive_output
            .to_str()
            .expect("interactive output path should be valid utf-8"),
    ))
    .expect("load interactive onboarding config");
    assert_eq!(
        interactive_config.provider.kind,
        mvp::config::ProviderKind::Deepseek,
        "interactive default should follow the sorted starting-point order and pick the alphabetically first detected source: {interactive_transcript:#?}"
    );
    assert_eq!(interactive_config.provider.model, "deepseek-chat");

    let non_interactive_context =
        loongclaw_daemon::onboard_cli::OnboardRuntimeContext::new_for_tests(80, None, codex_paths);
    let mut ui = ScriptedOnboardUi::new(std::iter::empty::<String>());
    loongclaw_daemon::onboard_cli::run_onboard_cli_with_ui(
        loongclaw_daemon::onboard_cli::OnboardCommandOptions {
            output: Some(non_interactive_output.display().to_string()),
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
        &mut ui,
        &non_interactive_context,
    )
    .await
    .expect("run non-interactive onboarding");

    let (_, non_interactive_config) = mvp::config::load(Some(
        non_interactive_output
            .to_str()
            .expect("non-interactive output path should be valid utf-8"),
    ))
    .expect("load non-interactive onboarding config");
    assert_eq!(
        non_interactive_config.provider.kind, interactive_config.provider.kind,
        "non-interactive onboarding should reuse the same detected starting-point ordering as the interactive default"
    );
    assert_eq!(
        non_interactive_config.provider.model,
        interactive_config.provider.model
    );

    let requests = server.join().expect("join local provider server");
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.starts_with("GET /v1/models "))
            .count(),
        2,
        "both onboarding runs should probe exactly one selected provider each: {requests:#?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn onboard_restores_original_config_when_memory_bootstrap_fails_after_write() {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    let root = unique_temp_path("memory-bootstrap-rollback-root");
    std::fs::create_dir_all(&root).expect("create test root");
    let output = root.join("loongclaw.toml");
    let invalid_sqlite_dir = root.join("memory-dir");
    std::fs::create_dir_all(&invalid_sqlite_dir).expect("create invalid sqlite directory");

    let (addr, server) = start_local_model_probe_server(1);

    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.base_url = format!("http://{addr}");
    config.provider.model = "openai/gpt-5.1-codex".to_owned();
    config.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "test-openai-key".to_owned(),
    ));
    config.memory.sqlite_path = invalid_sqlite_dir.display().to_string();
    mvp::config::write(Some(output.to_string_lossy().as_ref()), &config, true)
        .expect("write existing config");
    let original_body = std::fs::read_to_string(&output).expect("read original config");

    let mut options = default_non_interactive_onboard_options(&output);
    options.force = true;
    options.model = Some("gpt-4.1-mini".to_owned());

    let mut ui = ScriptedOnboardUi::new(std::iter::empty::<String>());
    let context =
        loongclaw_daemon::onboard_cli::OnboardRuntimeContext::new_for_tests(80, None, None);
    let error = loongclaw_daemon::onboard_cli::run_onboard_cli_with_ui(options, &mut ui, &context)
        .await
        .expect_err("memory bootstrap failure should abort onboarding");

    assert!(
        error.contains("failed to bootstrap sqlite memory"),
        "unexpected bootstrap failure error: {error}"
    );
    assert_eq!(
        std::fs::read_to_string(&output).expect("read config after rollback"),
        original_body,
        "onboarding should restore the original config when post-write bootstrap fails"
    );

    let requests = server.join().expect("join local provider server");
    assert!(
        requests
            .iter()
            .any(|request| request.starts_with("GET /v1/models ")),
        "post-write rollback path should still reach the provider model probe before bootstrap: {requests:#?}"
    );
}

#[test]
fn provider_credential_check_accepts_inline_api_key() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "inline-secret".to_owned(),
    ));
    config.provider.api_key_env = None;

    let check = loongclaw_daemon::onboard_cli::provider_credential_check(&config);

    assert_eq!(
        check.level,
        loongclaw_daemon::onboard_cli::OnboardCheckLevel::Pass
    );
    assert!(
        check.detail.contains("inline api key"),
        "inline provider credentials should pass preflight without forcing an env var: {check:#?}"
    );
}

#[test]
fn provider_credential_check_mentions_active_profile_id_when_saved_profiles_exist() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.set_active_provider_profile(
        "volcengine-coding",
        mvp::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: mvp::config::ProviderConfig {
                kind: mvp::config::ProviderKind::VolcengineCoding,
                model: "ark-code-latest".to_owned(),
                api_key: Some(loongclaw_contracts::SecretRef::Inline(
                    "inline-secret".to_owned(),
                )),
                base_url: "https://ark.cn-beijing.volces.com/api/coding/v3".to_owned(),
                wire_api: mvp::config::ProviderWireApi::ChatCompletions,
                chat_completions_path: "/chat/completions".to_owned(),
                ..mvp::config::ProviderConfig::default()
            },
        },
    );
    config.providers.insert(
        "openrouter".to_owned(),
        mvp::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: mvp::config::ProviderConfig {
                kind: mvp::config::ProviderKind::Openrouter,
                model: "z-ai/glm-4.5-air:free".to_owned(),
                ..mvp::config::ProviderConfig::default()
            },
        },
    );

    let check = crate::onboard_cli::provider_credential_check(&config);

    assert!(
        check.detail.contains("volcengine-coding"),
        "provider credential diagnostics should identify the active saved profile, not just the provider kind: {check:#?}"
    );
}

#[test]
fn preferred_api_key_env_default_stays_blank_for_inline_credentials() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "inline-secret".to_owned(),
    ));
    config.provider.api_key_env = None;

    let value = loongclaw_daemon::onboard_cli::preferred_api_key_env_default(&config);

    assert!(
        value.is_empty(),
        "inline credentials should not be replaced with a default API key env prompt value: {value:?}"
    );
}

#[test]
fn preferred_api_key_env_default_stays_blank_when_provider_has_no_default_env() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Ollama;
    config.provider.api_key = None;
    config.provider.api_key_env = None;
    config.provider.oauth_access_token = None;
    config.provider.oauth_access_token_env = None;

    let value = loongclaw_daemon::onboard_cli::preferred_api_key_env_default(&config);

    assert!(
        value.is_empty(),
        "providers without a canonical default env should not surface a fake suggested env: {value:?}"
    );
}

#[test]
fn preferred_api_key_env_default_prefers_oauth_default_for_fresh_openai() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Openai;
    config.provider.api_key = None;
    config.provider.api_key_env = None;
    config.provider.oauth_access_token = None;
    config.provider.oauth_access_token_env = None;

    let value = loongclaw_daemon::onboard_cli::preferred_api_key_env_default(&config);

    assert_eq!(
        value, "OPENAI_CODEX_OAUTH_TOKEN",
        "fresh OpenAI onboarding should surface the provider-preferred oauth env before the api-key fallback: {value:?}"
    );
}

#[test]
fn directory_preflight_check_has_no_filesystem_side_effects() {
    let base = unique_temp_path("preflight-root");
    let target = base.join("nested").join("tool-root");
    std::fs::create_dir_all(&base).expect("create existing ancestor");

    let check = loongclaw_daemon::onboard_cli::directory_preflight_check("tool file root", &target);

    assert_eq!(
        check.level,
        loongclaw_daemon::onboard_cli::OnboardCheckLevel::Pass
    );
    assert!(
        !target.exists(),
        "preflight inspection should not create the target directory before confirmation"
    );
    assert!(
        check.detail.contains("would create under"),
        "side-effect-free preflight should explain where the directory would be created: {check:#?}"
    );
}

#[test]
fn backup_existing_config_copies_without_removing_original() {
    let original = unique_temp_path("original-config.toml");
    let backup = unique_temp_path("backup-config.toml");
    std::fs::write(&original, "provider = \"openai\"\n").expect("write original config");

    loongclaw_daemon::onboard_cli::backup_existing_config(&original, &backup)
        .expect("backup copy should succeed");

    assert!(
        original.exists(),
        "backup flow should leave the original config in place until the new write happens"
    );
    assert_eq!(
        std::fs::read_to_string(&backup).expect("read backup"),
        "provider = \"openai\"\n"
    );
}

#[test]
fn onboard_risk_screen_uses_brand_header_and_continue_cancel_options() {
    let lines = loongclaw_daemon::onboard_cli::render_onboarding_risk_screen_lines(80);

    assert!(
        lines[0].starts_with("██╗"),
        "risk screen should keep the oversized LOONGCLAW brand banner on the initial guard screen: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .take_while(|line| !line.is_empty())
            .any(|line| line.contains(concat!("v", env!("CARGO_PKG_VERSION")))),
        "risk screen should keep the current build version visible under the brand banner: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "security check"),
        "risk screen should use a focused title: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("invoke tools and read local files")),
        "risk screen should explain the core execution risk in plain language: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("y) Continue onboarding")),
        "risk screen should show the affirmative path explicitly: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("n) Cancel")),
        "risk screen should keep cancellation explicit: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "press Enter to use default n, cancel"),
        "risk screen should make the safe default explicit on the screen itself: {lines:#?}"
    );
}

#[test]
fn import_surfaces_include_ready_provider_and_channels() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "provider-secret".to_owned(),
    ));
    config.telegram.enabled = true;
    config.telegram.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
        "123456:test-token".to_owned(),
    ));
    config.telegram.allowed_chat_ids = vec![42];
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

    let surfaces = loongclaw_daemon::onboard_cli::collect_import_surfaces(&config);
    assert!(
        surfaces.iter().any(|surface| surface.name == "provider"
            && surface.level == loongclaw_daemon::onboard_cli::ImportSurfaceLevel::Ready),
        "provider import surface should be ready: {surfaces:#?}"
    );
    assert!(
        surfaces
            .iter()
            .any(|surface| surface.name == "telegram channel"
                && surface.level == loongclaw_daemon::onboard_cli::ImportSurfaceLevel::Ready),
        "telegram import surface should be ready: {surfaces:#?}"
    );
    assert!(
        surfaces
            .iter()
            .any(|surface| surface.name == "feishu channel"
                && surface.level == loongclaw_daemon::onboard_cli::ImportSurfaceLevel::Ready),
        "feishu import surface should be ready: {surfaces:#?}"
    );
}

#[test]
fn import_surfaces_mark_missing_channel_secret_for_review() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.telegram.enabled = true;
    config.telegram.bot_token = None;
    config.telegram.bot_token_env = Some("LOONGCLAW_TEST_MISSING_TELEGRAM_TOKEN".to_owned());

    let surfaces = loongclaw_daemon::onboard_cli::collect_import_surfaces(&config);
    assert!(
        surfaces.iter().any(|surface| {
            surface.name == "telegram channel"
                && surface.level == loongclaw_daemon::onboard_cli::ImportSurfaceLevel::Review
                && surface.detail.contains("token missing")
        }),
        "telegram import surface should require review when the token is missing: {surfaces:#?}"
    );
}

#[test]
fn channel_preflight_checks_report_enabled_channels() {
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

    let checks = loongclaw_daemon::onboard_cli::collect_channel_preflight_checks(&config);
    assert!(
        checks.iter().any(|check| {
            check.name == "telegram channel"
                && check.level == loongclaw_daemon::onboard_cli::OnboardCheckLevel::Pass
                && check.detail.contains("bot token resolved")
        }),
        "telegram preflight should pass when token is resolved: {checks:#?}"
    );
    assert!(
        checks.iter().any(|check| {
            check.name == "feishu channel"
                && check.level == loongclaw_daemon::onboard_cli::OnboardCheckLevel::Pass
                && check.detail.contains("app credentials resolved")
        }),
        "feishu credentials should pass when app credentials are present: {checks:#?}"
    );
    assert!(
        checks.iter().any(|check| {
            check.name == "feishu inbound transport"
                && check.level == loongclaw_daemon::onboard_cli::OnboardCheckLevel::Pass
        }),
        "feishu inbound transport should pass when a supported transport is configured: {checks:#?}"
    );
}

#[test]
fn import_surfaces_detect_ready_channels_from_environment_only() {
    let config = mvp::config::LoongClawConfig::default();
    let surfaces = loongclaw_daemon::onboard_cli::collect_import_surfaces_with_channel_readiness(
        &config,
        loongclaw_daemon::onboard_cli::ChannelImportReadiness::default()
            .with_state(
                "telegram",
                loongclaw_daemon::migration::ChannelCredentialState::Ready,
            )
            .with_state(
                "feishu",
                loongclaw_daemon::migration::ChannelCredentialState::Ready,
            ),
    );
    assert!(
        surfaces.iter().any(|surface| {
            surface.name == "telegram channel"
                && surface.level == loongclaw_daemon::onboard_cli::ImportSurfaceLevel::Ready
        }),
        "telegram env should surface as import-ready even without an existing config: {surfaces:#?}"
    );
    assert!(
        surfaces.iter().any(|surface| {
            surface.name == "feishu channel"
                && surface.level == loongclaw_daemon::onboard_cli::ImportSurfaceLevel::Ready
        }),
        "feishu env should surface as import-ready even without an existing config: {surfaces:#?}"
    );
}

#[test]
fn detect_env_import_starting_config_enables_ready_channels() {
    let imported =
        loongclaw_daemon::onboard_cli::detect_import_starting_config_with_channel_readiness(
            loongclaw_daemon::onboard_cli::ChannelImportReadiness::default()
                .with_state(
                    "telegram",
                    loongclaw_daemon::migration::ChannelCredentialState::Ready,
                )
                .with_state(
                    "feishu",
                    loongclaw_daemon::migration::ChannelCredentialState::Ready,
                ),
        );
    assert!(
        imported.telegram.enabled,
        "telegram should be enabled when onboarding can reuse TELEGRAM_BOT_TOKEN"
    );
    assert!(
        imported.feishu.enabled,
        "feishu should be enabled when onboarding can reuse FEISHU_APP_ID and FEISHU_APP_SECRET"
    );
    assert_eq!(
        imported.telegram.bot_token_env.as_deref(),
        Some("TELEGRAM_BOT_TOKEN")
    );
    assert_eq!(imported.feishu.app_id_env.as_deref(), Some("FEISHU_APP_ID"));
    assert_eq!(
        imported.feishu.app_secret_env.as_deref(),
        Some("FEISHU_APP_SECRET")
    );
}

#[test]
fn detect_env_import_starting_config_only_enables_ready_channels() {
    let imported =
        loongclaw_daemon::onboard_cli::detect_import_starting_config_with_channel_readiness(
            loongclaw_daemon::onboard_cli::ChannelImportReadiness::default()
                .with_state(
                    "telegram",
                    loongclaw_daemon::migration::ChannelCredentialState::Ready,
                )
                .with_state(
                    "feishu",
                    loongclaw_daemon::migration::ChannelCredentialState::Partial,
                ),
        );

    assert!(
        imported.telegram.enabled,
        "telegram should be enabled when its credentials are ready"
    );
    assert!(
        !imported.feishu.enabled,
        "feishu should stay disabled when only part of its credentials are resolved"
    );
}

#[test]
fn collect_import_candidates_include_codex_config_with_env_channels() {
    let output_path = unique_temp_path("output-missing.toml");
    let codex_path = unique_temp_path("codex-config.toml");
    std::fs::write(
        &codex_path,
        r#"
model_provider = "sub2api"
model = "openai/gpt-5.1-codex"

[model_providers.sub2api]
name = "Sub2API"
base_url = "https://codex.example.com/v1"
chat_completions_path = "/codex/chat/completions"
wire_api = "responses"
requires_openai_auth = true
"#,
    )
    .expect("write codex config");

    let candidates = loongclaw_daemon::onboard_cli::collect_import_candidates_with_paths(
        &output_path,
        Some(&codex_path),
        loongclaw_daemon::onboard_cli::ChannelImportReadiness::default()
            .with_state(
                "telegram",
                loongclaw_daemon::migration::ChannelCredentialState::Ready,
            )
            .with_state(
                "feishu",
                loongclaw_daemon::migration::ChannelCredentialState::Ready,
            ),
    )
    .expect("collect import candidates");

    let codex_candidate = candidates
        .iter()
        .find(|candidate| candidate.source.contains("Codex config"))
        .expect("codex candidate");
    assert_eq!(
        codex_candidate.config.provider.kind,
        mvp::config::ProviderKind::Openai
    );
    assert_eq!(
        codex_candidate.config.provider.model,
        "openai/gpt-5.1-codex"
    );
    assert_eq!(
        codex_candidate.config.provider.base_url,
        "https://codex.example.com/v1"
    );
    assert_eq!(
        codex_candidate.config.provider.chat_completions_path,
        "/codex/chat/completions"
    );
    assert!(
        codex_candidate.config.telegram.enabled,
        "env-backed telegram readiness should carry into codex import candidate"
    );
    assert!(
        codex_candidate.config.feishu.enabled,
        "env-backed feishu readiness should carry into codex import candidate"
    );
}

#[test]
fn collect_import_candidates_maps_codex_provider_names_with_canonical_catalog() {
    let output_path = unique_temp_path("output-missing.toml");
    let codex_path = unique_temp_path("codex-kimi-coding-config.toml");
    std::fs::write(
        &codex_path,
        r#"
model_provider = "kimi_coding"
model = "kimi-coder"

[model_providers.kimi_coding]
base_url = "https://kimi-coding.example.com/v1"
"#,
    )
    .expect("write codex config");

    let candidates = loongclaw_daemon::onboard_cli::collect_import_candidates_with_paths(
        &output_path,
        Some(&codex_path),
        loongclaw_daemon::onboard_cli::ChannelImportReadiness::default(),
    )
    .expect("collect import candidates");

    let codex_candidate = candidates
        .iter()
        .find(|candidate| candidate.source.contains("Codex config"))
        .expect("codex candidate");
    assert_eq!(
        codex_candidate.config.provider.kind,
        mvp::config::ProviderKind::KimiCoding
    );
    assert_eq!(codex_candidate.config.provider.model, "kimi-coder");
}

#[test]
fn collect_import_candidates_uses_provider_default_auth_env_for_codex_provider() {
    let output_path = unique_temp_path("output-missing.toml");
    let codex_path = unique_temp_path("codex-kimi-coding-auth-config.toml");
    std::fs::write(
        &codex_path,
        r#"
model_provider = "kimi_coding"
model = "kimi-coder"

[model_providers.kimi_coding]
base_url = "https://kimi-coding.example.com/v1"
requires_openai_auth = true
"#,
    )
    .expect("write codex config");

    let candidates = loongclaw_daemon::onboard_cli::collect_import_candidates_with_paths(
        &output_path,
        Some(&codex_path),
        loongclaw_daemon::onboard_cli::ChannelImportReadiness::default(),
    )
    .expect("collect import candidates");

    let codex_candidate = candidates
        .iter()
        .find(|candidate| candidate.source.contains("Codex config"))
        .expect("codex candidate");
    assert_eq!(
        codex_candidate.config.provider.kind,
        mvp::config::ProviderKind::KimiCoding
    );
    assert_eq!(
        codex_candidate.config.provider.api_key_env.as_deref(),
        Some("KIMI_CODING_API_KEY")
    );
}

#[test]
fn collect_import_candidates_prepend_recommended_plan_before_detected_sources() {
    let output_path = unique_temp_path("existing-config.toml");
    let codex_path = unique_temp_path("codex-config.toml");

    let mut existing = mvp::config::LoongClawConfig::default();
    existing.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "provider-secret".to_owned(),
    ));
    let output_str = output_path
        .to_str()
        .expect("temp output path should be valid utf-8");
    mvp::config::write(Some(output_str), &existing, true).expect("write existing config");

    std::fs::write(
        &codex_path,
        r#"
model_provider = "sub2api"
model = "openai/gpt-5.1-codex"

[model_providers.sub2api]
name = "Sub2API"
base_url = "https://codex.example.com/v1"
wire_api = "responses"
requires_openai_auth = true
"#,
    )
    .expect("write codex config");

    let candidates = loongclaw_daemon::onboard_cli::collect_import_candidates_with_paths(
        &output_path,
        Some(&codex_path),
        loongclaw_daemon::onboard_cli::ChannelImportReadiness::default().with_state(
            "telegram",
            loongclaw_daemon::migration::ChannelCredentialState::Ready,
        ),
    )
    .expect("collect import candidates");

    assert!(
        candidates.len() >= 4,
        "expected recommended plan plus existing config, codex config, and environment candidates: {candidates:#?}"
    );
    assert_eq!(
        candidates[0].source_kind,
        loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan,
        "recommended composed plan should be the first import option: {candidates:#?}"
    );
    assert!(
        candidates[1].source.contains("existing config"),
        "existing loongclaw config should remain the first detected source after the recommended plan: {candidates:#?}"
    );
    assert!(
        candidates[2].source.contains("Codex config"),
        "codex config should remain the second detected source: {candidates:#?}"
    );
    assert_eq!(
        candidates[3].source, "your current environment",
        "environment import should remain the fallback candidate"
    );
}

#[test]
fn onboard_entry_prefers_current_setup_when_it_is_healthy() {
    let options = loongclaw_daemon::onboard_cli::build_onboard_entry_options(
        loongclaw_daemon::migration::types::CurrentSetupState::Healthy,
        &[
            import_candidate_with_kind(
                loongclaw_daemon::migration::types::ImportSourceKind::ExistingLoongClawConfig,
                "existing config at ~/.config/loongclaw/config.toml",
            ),
            import_candidate_with_kind(
                loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
                "Codex config at ~/.codex/config.toml",
            ),
        ],
    );

    assert_eq!(
        options[0].choice,
        loongclaw_daemon::onboard_cli::OnboardEntryChoice::ContinueCurrentSetup
    );
    assert!(
        options[0].recommended,
        "healthy current setup should be the recommended first choice: {options:#?}"
    );
    assert!(
        options
            .iter()
            .any(|option| option.choice
                == loongclaw_daemon::onboard_cli::OnboardEntryChoice::StartFresh),
        "start fresh should remain available: {options:#?}"
    );
}

#[test]
fn onboard_entry_prefers_import_when_current_setup_is_absent() {
    let options = loongclaw_daemon::onboard_cli::build_onboard_entry_options(
        loongclaw_daemon::migration::types::CurrentSetupState::Absent,
        &[import_candidate_with_kind(
            loongclaw_daemon::migration::types::ImportSourceKind::Environment,
            "your current environment",
        )],
    );

    assert_eq!(
        options[0].choice,
        loongclaw_daemon::onboard_cli::OnboardEntryChoice::ImportDetectedSetup
    );
    assert!(
        options[0].recommended,
        "import should be recommended when current setup is absent and reusable sources exist: {options:#?}"
    );
    assert!(
        options.iter().all(|option| option.choice
            != loongclaw_daemon::onboard_cli::OnboardEntryChoice::ContinueCurrentSetup),
        "continue current setup should not appear when no current setup exists: {options:#?}"
    );
    assert!(
        options
            .iter()
            .any(|option| option.choice
                == loongclaw_daemon::onboard_cli::OnboardEntryChoice::StartFresh),
        "start fresh should remain available: {options:#?}"
    );
}

#[test]
fn onboard_entry_prefers_import_when_current_setup_is_repairable_and_sources_exist() {
    let options = loongclaw_daemon::onboard_cli::build_onboard_entry_options(
        loongclaw_daemon::migration::types::CurrentSetupState::Repairable,
        &[
            import_candidate_with_kind(
                loongclaw_daemon::migration::types::ImportSourceKind::ExistingLoongClawConfig,
                "existing config at ~/.config/loongclaw/config.toml",
            ),
            import_candidate_with_kind(
                loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan,
                "recommended import plan",
            ),
            import_candidate_with_kind(
                loongclaw_daemon::migration::types::ImportSourceKind::Environment,
                "your current environment",
            ),
        ],
    );

    let import_option = options
        .iter()
        .find(|option| {
            option.choice == loongclaw_daemon::onboard_cli::OnboardEntryChoice::ImportDetectedSetup
        })
        .expect("import option");

    assert_eq!(import_option.label, "Use detected starting point");
    assert!(
        import_option.recommended,
        "repairable current setup should recommend import instead of falling through to start fresh: {options:#?}"
    );
    assert!(
        !import_option.detail.contains("import"),
        "main onboarding wording should describe detected setup without exposing import terminology: {options:#?}"
    );
}

#[test]
fn onboard_presentation_review_and_shortcut_copy_stays_canonical() {
    let guided = loongclaw_daemon::onboard_presentation::review_flow_copy(
        loongclaw_daemon::onboard_presentation::ReviewFlowKind::Guided,
    );
    assert_eq!(guided.progress_line, "step 8 of 8 · review");
    assert_eq!(guided.header_subtitle, "review setup");

    let quick_current = loongclaw_daemon::onboard_presentation::review_flow_copy(
        loongclaw_daemon::onboard_presentation::ReviewFlowKind::QuickCurrentSetup,
    );
    assert_eq!(quick_current.progress_line, "quick review · current setup");
    assert_eq!(quick_current.header_subtitle, "review current setup");

    let quick_detected = loongclaw_daemon::onboard_presentation::review_flow_copy(
        loongclaw_daemon::onboard_presentation::ReviewFlowKind::QuickDetectedSetup,
    );
    assert_eq!(
        quick_detected.progress_line,
        "quick review · detected starting point"
    );
    assert_eq!(
        quick_detected.header_subtitle,
        "review detected starting point"
    );

    let current_shortcut = loongclaw_daemon::onboard_presentation::shortcut_copy(
        loongclaw_daemon::onboard_presentation::ShortcutKind::CurrentSetup,
    );
    assert_eq!(
        current_shortcut.subtitle,
        "keep the current setup or fine-tune it"
    );
    assert_eq!(current_shortcut.title, "continue current setup");
    assert_eq!(
        current_shortcut.summary_line,
        "you can keep moving with this setup through a quick review, or adjust a few settings first"
    );
    assert_eq!(current_shortcut.primary_label, "Keep current setup");
    assert_eq!(
        current_shortcut.default_choice_description,
        "keep current setup"
    );

    let detected_shortcut = loongclaw_daemon::onboard_presentation::shortcut_copy(
        loongclaw_daemon::onboard_presentation::ShortcutKind::DetectedSetup,
    );
    assert_eq!(
        detected_shortcut.subtitle,
        "use the detected starting point or fine-tune it"
    );
    assert_eq!(
        detected_shortcut.title,
        "continue with detected starting point"
    );
    assert_eq!(
        detected_shortcut.summary_line,
        "you can keep moving with this detected starting point through a quick review, or adjust a few settings first"
    );
    assert_eq!(
        detected_shortcut.primary_label,
        "Use detected starting point"
    );
    assert_eq!(
        detected_shortcut.default_choice_description,
        "the detected starting point"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::single_detected_starting_point_preview_subtitle(),
        "review the detected starting point"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::single_detected_starting_point_preview_title(),
        "review detected starting point"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::single_detected_starting_point_preview_footer(),
        "continuing with the only detected starting point"
    );
}

#[test]
fn onboard_presentation_entry_and_digest_copy_stays_canonical() {
    assert_eq!(
        loongclaw_daemon::onboard_presentation::current_setup_option_label(),
        "Continue current setup"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::detected_setup_option_label(),
        "Use detected starting point"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::start_fresh_option_label(),
        "Start fresh"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::start_fresh_option_detail(),
        "Configure provider, channels, and local behavior from scratch."
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::current_setup_state_label(
            loongclaw_daemon::migration::types::CurrentSetupState::LegacyOrIncomplete,
        ),
        "legacy or incomplete"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::current_setup_option_detail(
            loongclaw_daemon::migration::types::CurrentSetupState::Repairable,
        ),
        "Current config exists, but a few settings should be reviewed."
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::import_option_detail(true, true, 1),
        "A suggested starting point can supplement the current config with 1 reusable source."
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::import_option_detail(false, true, 2),
        "A suggested starting point is ready, built from 2 reusable sources."
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::import_option_detail(false, false, 1),
        "1 reusable source was detected for provider, channels, or guidance."
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::import_option_detail(false, false, 2),
        "2 reusable sources were detected for provider, channels, or guidance."
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::detected_coverage_prefix(true),
        "- suggested starting point covers: "
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::detected_coverage_prefix(false),
        "- detected coverage: "
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::suggested_starting_point_ready_line(),
        "- suggested starting point: ready"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::entry_default_choice_description(
            loongclaw_daemon::onboard_presentation::EntryChoiceKind::CurrentSetup,
        ),
        "continue current setup"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::entry_default_choice_description(
            loongclaw_daemon::onboard_presentation::EntryChoiceKind::DetectedSetup,
        ),
        "the detected starting point"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::entry_default_choice_description(
            loongclaw_daemon::onboard_presentation::EntryChoiceKind::StartFresh,
        ),
        "start fresh"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::starting_point_footer_description(
            loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan,
        ),
        "the suggested starting point"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::starting_point_footer_description(
            loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
        ),
        "the first starting point"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::starting_point_selection_subtitle(),
        "choose the starting point for this setup"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::starting_point_selection_title(),
        "choose detected starting point"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::starting_point_selection_hint(),
        "detected settings can still supplement the chosen starting point when they do not conflict"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::detected_settings_section_heading(),
        "Detected settings"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::entry_choice_section_heading(),
        "Choose how to start"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::adjust_settings_label(),
        "Adjust settings"
    );
}

#[test]
fn onboard_presentation_risk_preflight_and_write_copy_stays_canonical() {
    let risk = loongclaw_daemon::onboard_presentation::risk_screen_copy();
    assert_eq!(risk.subtitle, "security check before setup");
    assert_eq!(risk.title, "security check");
    assert_eq!(risk.continue_label, "Continue onboarding");
    assert_eq!(
        risk.continue_detail,
        "review provider, channels, and local behavior now"
    );
    assert_eq!(risk.cancel_label, "Cancel");
    assert_eq!(
        risk.cancel_detail,
        "stop before changing or writing any config"
    );
    assert_eq!(risk.default_choice_description, "cancel");
    assert_eq!(risk.confirm_prompt, "Continue");

    assert_eq!(
        loongclaw_daemon::onboard_presentation::preflight_header_title(),
        "verify before write"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::preflight_section_title(),
        "preflight checks"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::preflight_attention_summary_line(),
        "- some checks need attention before write"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::preflight_green_summary_line(),
        "- all checks are green for this draft"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::preflight_probe_rerun_hint(),
        "- rerun with --skip-model-probe if your provider blocks model listing during setup"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::preflight_explicit_model_rerun_hint(),
        "- rerun onboarding to choose a reviewed model, or set provider.model / preferred_models explicitly"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::preflight_explicit_model_only_rerun_hint(),
        "- set provider.model / preferred_models explicitly before retrying"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::preflight_continue_label(),
        "Continue anyway"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::preflight_continue_detail(),
        "accept the remaining warnings and continue with this draft"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::preflight_cancel_label(),
        "Cancel"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::preflight_cancel_detail(),
        "stop here and return without writing any config"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::preflight_default_choice_description(),
        "cancel"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::preflight_confirm_prompt(),
        "Continue anyway"
    );

    assert_eq!(
        loongclaw_daemon::onboard_presentation::write_confirmation_title(),
        "ready to write config"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::write_confirmation_status_line(true),
        "- warnings were kept by choice"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::write_confirmation_status_line(false),
        "- preflight is green for this draft"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::write_confirmation_label(),
        "Write config"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::write_confirmation_detail(),
        "persist this onboarding draft to the target path"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::write_confirmation_cancel_label(),
        "Cancel"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::write_confirmation_cancel_detail(),
        "return without writing any config"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::write_confirmation_default_choice_description(),
        "write config"
    );
    assert_eq!(
        loongclaw_daemon::onboard_presentation::write_confirmation_prompt(),
        "Write config"
    );
}

#[test]
fn onboard_entry_avoids_double_recommendation_when_suggested_starting_point_has_rollup_sources() {
    let current = import_candidate_with_kind(
        loongclaw_daemon::migration::types::ImportSourceKind::ExistingLoongClawConfig,
        "existing config at ~/.config/loongclaw/config.toml",
    );
    let mut recommended = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan,
        "recommended import plan",
        mvp::config::ProviderKind::Openai,
        "openai/gpt-5.1-codex",
        "OPENAI_API_KEY",
    );
    recommended.workspace_guidance.push(
        loongclaw_daemon::migration::types::WorkspaceGuidanceCandidate {
            kind: loongclaw_daemon::migration::types::WorkspaceGuidanceKind::Agents,
            path: "/tmp/project/AGENTS.md".to_owned(),
        },
    );

    let options = loongclaw_daemon::onboard_cli::build_onboard_entry_options(
        loongclaw_daemon::migration::types::CurrentSetupState::Repairable,
        &[current, recommended],
    );

    let current_option = options
        .iter()
        .find(|option| {
            option.choice == loongclaw_daemon::onboard_cli::OnboardEntryChoice::ContinueCurrentSetup
        })
        .expect("current option");
    let import_option = options
        .iter()
        .find(|option| {
            option.choice == loongclaw_daemon::onboard_cli::OnboardEntryChoice::ImportDetectedSetup
        })
        .expect("import option");

    assert!(
        !current_option.recommended,
        "repairable current setup should stop being recommended once the suggested starting point has reusable rollup sources: {options:#?}"
    );
    assert!(
        import_option.recommended,
        "the suggested starting point should remain the single recommended path in that case: {options:#?}"
    );
}

#[test]
fn onboard_entry_import_option_explains_detected_additions_when_current_setup_exists() {
    let options = loongclaw_daemon::onboard_cli::build_onboard_entry_options(
        loongclaw_daemon::migration::types::CurrentSetupState::Healthy,
        &[
            import_candidate_with_kind(
                loongclaw_daemon::migration::types::ImportSourceKind::ExistingLoongClawConfig,
                "existing config at ~/.config/loongclaw/config.toml",
            ),
            import_candidate_with_kind(
                loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan,
                "recommended import plan",
            ),
            import_candidate_with_kind(
                loongclaw_daemon::migration::types::ImportSourceKind::Environment,
                "your current environment",
            ),
        ],
    );

    let import_option = options
        .iter()
        .find(|option| {
            option.choice == loongclaw_daemon::onboard_cli::OnboardEntryChoice::ImportDetectedSetup
        })
        .expect("import option");

    assert!(
        import_option
            .detail
            .contains("supplement the current config"),
        "when a current config already exists, the detected-setup path should explain that it adds reusable settings on top instead of sounding like a parallel fresh-start path: {options:#?}"
    );
}

#[test]
fn onboard_entry_screen_uses_compact_header_and_detected_setup_digest() {
    let current = import_candidate_with_kind(
        loongclaw_daemon::migration::types::ImportSourceKind::ExistingLoongClawConfig,
        "existing config at ~/.config/loongclaw/config.toml",
    );
    let mut recommended = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan,
        "recommended import plan",
        mvp::config::ProviderKind::Openai,
        "openai/gpt-5.1-codex",
        "OPENAI_API_KEY",
    );
    recommended.workspace_guidance.push(
        loongclaw_daemon::migration::types::WorkspaceGuidanceCandidate {
            kind: loongclaw_daemon::migration::types::WorkspaceGuidanceKind::Agents,
            path: "/tmp/project/AGENTS.md".to_owned(),
        },
    );
    recommended
        .channel_candidates
        .push(loongclaw_daemon::migration::types::ChannelCandidate {
            id: "telegram",
            label: "telegram",
            status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
            source: "your current environment".to_owned(),
            summary: "enabled · token resolved".to_owned(),
        });
    recommended
        .domains
        .push(loongclaw_daemon::migration::types::DomainPreview {
            kind: loongclaw_daemon::migration::types::SetupDomainKind::Channels,
            status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
            decision: Some(loongclaw_daemon::migration::types::PreviewDecision::Supplement),
            source: "your current environment".to_owned(),
            summary: "telegram Ready".to_owned(),
        });
    let options = loongclaw_daemon::onboard_cli::build_onboard_entry_options(
        loongclaw_daemon::migration::types::CurrentSetupState::Repairable,
        &[current.clone(), recommended.clone()],
    );

    let lines = loongclaw_daemon::onboard_cli::render_onboard_entry_screen_lines(
        loongclaw_daemon::migration::types::CurrentSetupState::Repairable,
        Some(&current),
        &[recommended],
        &options,
        Some(std::path::Path::new("/tmp/project")),
        80,
    );

    assert_compact_loongclaw_header(&lines, "entry screen");
    assert!(
        lines.iter().all(|line| !line.starts_with("██╗")),
        "entry screen should not repeat the large LOONGCLAW banner after the first screen: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "guided setup for provider, channels, and workspace guidance"),
        "entry screen should include the new onboarding subtitle: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- workspace: /tmp/project")),
        "entry screen should anchor the flow to the current workspace: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- workspace guidance: AGENTS.md")),
        "entry screen should summarize detected workspace guidance files: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| {
            line.contains("- suggested starting point covers:")
                && line.contains("provider")
                && line.contains("channels")
                && line.contains("workspace guidance")
        }),
        "entry screen should summarize what the suggested starting point already covers: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- channels detected: telegram")),
        "entry screen should summarize detected channel handoffs separately from the higher-level starting-point coverage: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("2) Use detected starting point (recommended)")),
        "entry screen should keep the detected-setup path visible and recommended: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "press Enter to use default 2, the detected starting point"),
        "entry screen should make the recommended default path explicit instead of hiding it only in the prompt default: {lines:#?}"
    );
}

#[test]
fn onboard_entry_screen_compacts_to_plain_wordmark_on_narrow_width() {
    let options = loongclaw_daemon::onboard_cli::build_onboard_entry_options(
        loongclaw_daemon::migration::types::CurrentSetupState::Absent,
        &[import_candidate_with_kind(
            loongclaw_daemon::migration::types::ImportSourceKind::Environment,
            "your current environment",
        )],
    );

    let lines = loongclaw_daemon::onboard_cli::render_onboard_entry_screen_lines(
        loongclaw_daemon::migration::types::CurrentSetupState::Absent,
        None,
        &[import_candidate_with_kind(
            loongclaw_daemon::migration::types::ImportSourceKind::Environment,
            "your current environment",
        )],
        &options,
        None,
        40,
    );

    assert_compact_loongclaw_header(&lines, "narrow entry screen");
    assert!(
        lines.iter().any(|line| line == "Detected settings"),
        "narrow layout should retain the detected-settings section heading: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "1) Use detected starting point"),
        "narrow layout should keep the primary entry choice readable: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "   (recommended)"),
        "narrow layout should still surface the recommendation marker when the longer label wraps: {lines:#?}"
    );
}

#[test]
fn onboard_entry_screen_wraps_detected_setup_digest_and_option_details() {
    let mut recommended = import_candidate_with_kind(
        loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan,
        "recommended import plan",
    );
    recommended.workspace_guidance.push(
        loongclaw_daemon::migration::types::WorkspaceGuidanceCandidate {
            kind: loongclaw_daemon::migration::types::WorkspaceGuidanceKind::Agents,
            path: "/tmp/project/AGENTS.md".to_owned(),
        },
    );
    recommended.workspace_guidance.push(
        loongclaw_daemon::migration::types::WorkspaceGuidanceCandidate {
            kind: loongclaw_daemon::migration::types::WorkspaceGuidanceKind::Claude,
            path: "/tmp/project/CLAUDE.md".to_owned(),
        },
    );
    recommended.workspace_guidance.push(
        loongclaw_daemon::migration::types::WorkspaceGuidanceCandidate {
            kind: loongclaw_daemon::migration::types::WorkspaceGuidanceKind::Gemini,
            path: "/tmp/project/GEMINI.md".to_owned(),
        },
    );
    let options = loongclaw_daemon::onboard_cli::build_onboard_entry_options(
        loongclaw_daemon::migration::types::CurrentSetupState::Absent,
        &[recommended.clone()],
    );

    let lines = loongclaw_daemon::onboard_cli::render_onboard_entry_screen_lines(
        loongclaw_daemon::migration::types::CurrentSetupState::Absent,
        None,
        &[recommended],
        &options,
        Some(std::path::Path::new("/tmp/project with shared guidance")),
        42,
    );

    assert!(
        lines
            .iter()
            .any(|line| line == "- workspace: /tmp/project with shared"),
        "entry screen should keep the workspace label visible before wrapping long paths: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "  guidance"),
        "entry screen should continue wrapped workspace paths on an indented line: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "- workspace guidance: AGENTS.md,"),
        "entry screen should wrap long workspace-guidance digests instead of overflowing them: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "  CLAUDE.md, GEMINI.md"),
        "entry screen should continue workspace-guidance digests on readable continuation lines: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "    A suggested starting point is ready,"),
        "entry screen should wrap long option details instead of keeping them on one overflowing line: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "    built from 1 reusable source."),
        "entry screen should keep wrapped option-detail continuations aligned under the option: {lines:#?}"
    );
}

#[test]
fn onboard_provider_selection_plan_requires_explicit_choice_for_conflicting_recommended_import() {
    let mut recommended = import_candidate_with_kind(
        loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan,
        "recommended import plan",
    );
    recommended
        .domains
        .push(loongclaw_daemon::migration::types::DomainPreview {
            kind: loongclaw_daemon::migration::types::SetupDomainKind::Channels,
            status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
            decision: Some(loongclaw_daemon::migration::types::PreviewDecision::UseDetected),
            source: "Codex config at ~/.codex/config.toml".to_owned(),
            summary: "telegram Ready".to_owned(),
        });
    recommended.config.telegram.enabled = true;
    recommended.config.telegram.bot_token_env = Some("TELEGRAM_BOT_TOKEN".to_owned());

    let codex = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
        "Codex config at ~/.codex/config.toml",
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

    let plan = loongclaw_daemon::onboard_cli::build_provider_selection_plan_for_candidate(
        &recommended,
        &[recommended.clone(), codex, env],
    );

    assert!(
        plan.requires_explicit_choice,
        "recommended import should require an explicit provider choice when multiple imported providers conflict and no safe provider was composed: {plan:#?}"
    );
    assert_eq!(
        plan.default_kind, None,
        "there should be no silent fallback provider in a conflicted recommended import: {plan:#?}"
    );
    assert_eq!(plan.imported_choices.len(), 2);
    assert_eq!(
        plan.imported_choices[0].kind,
        mvp::config::ProviderKind::Openai
    );
    assert_eq!(
        plan.imported_choices[1].kind,
        mvp::config::ProviderKind::Deepseek
    );
}

#[test]
fn onboard_provider_selection_plan_retains_same_kind_profiles_and_defaults_to_selected_profile() {
    let recommended = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan,
        "recommended import plan",
        mvp::config::ProviderKind::Openai,
        "gpt-5",
        "OPENAI_MAIN_API_KEY",
    );
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

    let plan = loongclaw_daemon::onboard_cli::build_provider_selection_plan_for_candidate(
        &recommended,
        &[recommended.clone(), codex, env],
    );

    assert_eq!(
        plan.imported_choices.len(),
        2,
        "recommended imports should retain distinct same-kind provider profiles instead of collapsing them into one choice: {plan:#?}"
    );
    assert_eq!(
        plan.default_profile_id.as_deref(),
        Some("openai-gpt-5"),
        "the selected recommended provider should stay the default profile even when another same-kind profile is also detected: {plan:#?}"
    );
    assert!(
        plan.imported_choices
            .iter()
            .any(|choice| choice.profile_id == "openai-o4-mini"),
        "same-kind alternate profiles should receive stable, model-derived ids: {plan:#?}"
    );
}

#[test]
fn onboard_provider_selection_screen_includes_focus_title_and_choices() {
    let recommended = import_candidate_with_kind(
        loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan,
        "recommended import plan",
    );
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
    let plan = loongclaw_daemon::onboard_cli::build_provider_selection_plan_for_candidate(
        &recommended,
        &[recommended.clone(), openai, deepseek],
    );

    let lines = loongclaw_daemon::onboard_cli::render_provider_selection_screen_lines(&plan, 80);

    assert_compact_loongclaw_header(&lines, "provider choice screen");
    assert!(
        lines.iter().all(|line| !line.starts_with("██╗")),
        "provider choice screen should not re-render the large LOONGCLAW banner mid-onboarding: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "choose active provider"),
        "provider choice screen should use a focused decision title: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "step 1 of 8 · provider"),
        "provider choice screen should keep the guided-flow progress context inside the screen: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("other detected settings stay merged")),
        "provider choice screen should reassure users that non-provider domains stay merged: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("try one of: openai, deepseek")),
        "provider choice screen should surface quick selector picks instead of forcing users to scan the full selector catalog: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("accepted selectors: openai")),
        "wide provider choice screens should still expose the full selector catalog in the footer: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line
            .contains("you can also enter a unique model name, model suffix, or provider kind")),
        "wide provider choice screens should keep the full selector guidance sentence when there is enough room for it: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("OpenAI")),
        "provider choice screen should list the first candidate: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("DeepSeek")),
        "provider choice screen should list the second candidate: {lines:#?}"
    );
}

#[test]
fn onboard_provider_selection_screen_shows_default_enter_choice_when_provider_is_resolved() {
    let plan = loongclaw_daemon::migration::ProviderSelectionPlan {
        imported_choices: vec![
            loongclaw_daemon::migration::ImportedProviderChoice {
                profile_id: "openai".to_owned(),
                kind: mvp::config::ProviderKind::Openai,
                source: "Codex config at ~/.codex/config.toml".to_owned(),
                summary: "OpenAI · openai/gpt-5.1-codex · credentials resolved".to_owned(),
                config: mvp::config::ProviderConfig {
                    kind: mvp::config::ProviderKind::Openai,
                    model: "openai/gpt-5.1-codex".to_owned(),
                    api_key_env: Some("OPENAI_API_KEY".to_owned()),
                    ..mvp::config::ProviderConfig::default()
                },
            },
            loongclaw_daemon::migration::ImportedProviderChoice {
                profile_id: "deepseek".to_owned(),
                kind: mvp::config::ProviderKind::Deepseek,
                source: "your current environment".to_owned(),
                summary: "DeepSeek · deepseek-chat · credentials resolved".to_owned(),
                config: mvp::config::ProviderConfig {
                    kind: mvp::config::ProviderKind::Deepseek,
                    model: "deepseek-chat".to_owned(),
                    api_key_env: Some("DEEPSEEK_API_KEY".to_owned()),
                    ..mvp::config::ProviderConfig::default()
                },
            },
        ],
        default_kind: Some(mvp::config::ProviderKind::Openai),
        default_profile_id: Some("openai".to_owned()),
        requires_explicit_choice: false,
    };

    let lines = loongclaw_daemon::onboard_cli::render_provider_selection_screen_lines(&plan, 80);

    assert!(
        lines
            .iter()
            .any(|line| line == "press Enter to use default openai, the OpenAI provider"),
        "provider choice screen should make the resolved default provider explicit instead of relying only on the prompt default: {lines:#?}"
    );
}

#[test]
fn onboard_provider_selection_screen_uses_profile_ids_for_same_kind_choices() {
    let plan = loongclaw_daemon::migration::ProviderSelectionPlan {
        imported_choices: vec![
            loongclaw_daemon::migration::ImportedProviderChoice {
                profile_id: "openai-gpt-5".to_owned(),
                kind: mvp::config::ProviderKind::Openai,
                source: "Codex config at ~/.codex/config.toml".to_owned(),
                summary: "OpenAI · gpt-5 · credentials resolved".to_owned(),
                config: mvp::config::ProviderConfig {
                    kind: mvp::config::ProviderKind::Openai,
                    model: "gpt-5".to_owned(),
                    api_key_env: Some("OPENAI_MAIN_API_KEY".to_owned()),
                    ..mvp::config::ProviderConfig::default()
                },
            },
            loongclaw_daemon::migration::ImportedProviderChoice {
                profile_id: "openai-o4-mini".to_owned(),
                kind: mvp::config::ProviderKind::Openai,
                source: "your current environment".to_owned(),
                summary: "OpenAI · o4-mini · credentials resolved".to_owned(),
                config: mvp::config::ProviderConfig {
                    kind: mvp::config::ProviderKind::Openai,
                    model: "o4-mini".to_owned(),
                    api_key_env: Some("OPENAI_REASONING_API_KEY".to_owned()),
                    ..mvp::config::ProviderConfig::default()
                },
            },
        ],
        default_kind: Some(mvp::config::ProviderKind::Openai),
        default_profile_id: Some("openai-o4-mini".to_owned()),
        requires_explicit_choice: false,
    };

    let lines = loongclaw_daemon::onboard_cli::render_provider_selection_screen_lines(&plan, 80);

    assert!(
        lines.iter().any(|line| line == "openai-gpt-5) OpenAI"),
        "same-kind provider choices should expose the stable profile id instead of only the provider kind: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "openai-o4-mini) OpenAI (recommended)"),
        "only the resolved default profile should be marked recommended when same-kind choices coexist: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("selectors: openai-gpt-5, gpt-5")),
        "each choice should show the selectors a human can type, not only the stable profile id: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("selectors: openai-o4-mini, o4-mini, openai")),
        "the default-for-kind choice should surface its provider-kind selector alongside profile/model aliases: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "press Enter to use default openai-o4-mini, the OpenAI provider"),
        "the Enter shortcut should point at the concrete default profile id, not only the provider kind: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line
            .contains(loongclaw_daemon::migration::provider_selection::PROVIDER_SELECTOR_NOTE)),
        "provider choice screen should explain the broader selector grammar without forcing users to memorize only profile ids: {lines:#?}"
    );
}

#[test]
fn onboard_provider_selection_screen_prefers_short_human_selectors_on_narrow_width() {
    let plan = loongclaw_daemon::migration::ProviderSelectionPlan {
        imported_choices: vec![
            loongclaw_daemon::migration::ImportedProviderChoice {
                profile_id: "openai-o4-mini".to_owned(),
                kind: mvp::config::ProviderKind::Openai,
                source: "your current environment".to_owned(),
                summary: "OpenAI · o4-mini · credentials resolved".to_owned(),
                config: mvp::config::ProviderConfig {
                    kind: mvp::config::ProviderKind::Openai,
                    model: "o4-mini".to_owned(),
                    api_key_env: Some("OPENAI_REASONING_API_KEY".to_owned()),
                    ..mvp::config::ProviderConfig::default()
                },
            },
            loongclaw_daemon::migration::ImportedProviderChoice {
                profile_id: "openai-gpt-5".to_owned(),
                kind: mvp::config::ProviderKind::Openai,
                source: "Codex config at ~/.codex/config.toml".to_owned(),
                summary: "OpenAI · gpt-5 · credentials resolved".to_owned(),
                config: mvp::config::ProviderConfig {
                    kind: mvp::config::ProviderKind::Openai,
                    model: "gpt-5".to_owned(),
                    api_key_env: Some("OPENAI_MAIN_API_KEY".to_owned()),
                    ..mvp::config::ProviderConfig::default()
                },
            },
        ],
        default_kind: Some(mvp::config::ProviderKind::Openai),
        default_profile_id: Some("openai-o4-mini".to_owned()),
        requires_explicit_choice: false,
    };

    let lines = loongclaw_daemon::onboard_cli::render_provider_selection_screen_lines(&plan, 52);

    assert!(
        lines.iter().any(|line| line == "    selector: openai"),
        "the default same-kind profile should surface the short provider-kind selector on narrow screens: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "    selector: gpt-5"),
        "non-default same-kind profiles should surface the concise model alias instead of the longer profile id on narrow screens: {lines:#?}"
    );
}

#[test]
fn onboard_provider_selector_reports_ambiguous_model_name() {
    let plan = loongclaw_daemon::migration::ProviderSelectionPlan {
        imported_choices: vec![
            loongclaw_daemon::migration::ImportedProviderChoice {
                profile_id: "openai-gpt-5".to_owned(),
                kind: mvp::config::ProviderKind::Openai,
                source: "Codex config at ~/.codex/config.toml".to_owned(),
                summary: "OpenAI · gpt-5 · credentials resolved".to_owned(),
                config: mvp::config::ProviderConfig {
                    kind: mvp::config::ProviderKind::Openai,
                    model: "gpt-5".to_owned(),
                    api_key_env: Some("OPENAI_API_KEY".to_owned()),
                    ..mvp::config::ProviderConfig::default()
                },
            },
            loongclaw_daemon::migration::ImportedProviderChoice {
                profile_id: "openrouter-gpt-5".to_owned(),
                kind: mvp::config::ProviderKind::Openrouter,
                source: "your current environment".to_owned(),
                summary: "OpenRouter · gpt-5 · credentials resolved".to_owned(),
                config: mvp::config::ProviderConfig {
                    kind: mvp::config::ProviderKind::Openrouter,
                    model: "gpt-5".to_owned(),
                    api_key_env: Some("OPENROUTER_API_KEY".to_owned()),
                    ..mvp::config::ProviderConfig::default()
                },
            },
        ],
        default_kind: Some(mvp::config::ProviderKind::Openai),
        default_profile_id: Some("openai-gpt-5".to_owned()),
        requires_explicit_choice: false,
    };

    let error = loongclaw_daemon::onboard_cli::resolve_provider_config_from_selector(
        &mvp::config::ProviderConfig::default(),
        &plan,
        "gpt-5",
    )
    .expect_err("duplicate model selectors should surface an ambiguity error");

    assert!(error.contains("ambiguous"));
    assert!(error.contains("try one of:"));
    assert!(error.contains("openai-gpt-5"));
    assert!(error.contains("openrouter-gpt-5"));
    assert!(error.contains("model=gpt-5"));
    assert!(error.contains("selectors=openai-gpt-5, openai"));
    assert!(error.contains("selectors=openrouter-gpt-5, openrouter"));
}

#[test]
fn onboard_provider_selection_screen_wraps_long_choice_details() {
    let plan = loongclaw_daemon::migration::ProviderSelectionPlan {
        imported_choices: vec![
            loongclaw_daemon::migration::ImportedProviderChoice {
                profile_id: "openai".to_owned(),
                kind: mvp::config::ProviderKind::Openai,
                source: "Codex config at ~/.codex/agents/loongclaw/config.toml".to_owned(),
                summary: "OpenAI · openai/gpt-5.1-codex · credentials resolved".to_owned(),
                config: mvp::config::ProviderConfig {
                    kind: mvp::config::ProviderKind::Openai,
                    model: "openai/gpt-5.1-codex".to_owned(),
                    api_key_env: Some("OPENAI_API_KEY".to_owned()),
                    ..mvp::config::ProviderConfig::default()
                },
            },
            loongclaw_daemon::migration::ImportedProviderChoice {
                profile_id: "deepseek".to_owned(),
                kind: mvp::config::ProviderKind::Deepseek,
                source: "your current environment".to_owned(),
                summary: "DeepSeek · deepseek-chat · credentials resolved".to_owned(),
                config: mvp::config::ProviderConfig {
                    kind: mvp::config::ProviderKind::Deepseek,
                    model: "deepseek-chat".to_owned(),
                    api_key_env: Some("DEEPSEEK_API_KEY".to_owned()),
                    ..mvp::config::ProviderConfig::default()
                },
            },
        ],
        default_kind: None,
        default_profile_id: None,
        requires_explicit_choice: true,
    };

    let lines = loongclaw_daemon::onboard_cli::render_provider_selection_screen_lines(&plan, 52);

    assert!(
        lines
            .iter()
            .any(|line| line == "    source: Codex config at"),
        "provider choice screen should wrap long source labels instead of overflowing them: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "      ~/.codex/agents/loongclaw/config.toml"),
        "provider choice screen should continue long source paths on an indented line: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "    summary: OpenAI · openai/gpt-5.1-codex ·"),
        "provider choice screen should keep the summary label visible before wrapping: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "    selector: openai"),
        "narrow provider choice screens should collapse selector aliases into one preferred selector per choice to stay scannable: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.starts_with("    selectors: ")),
        "narrow provider choice screens should avoid repeating the full selector catalog inside each choice row while still allowing the footer to expose the complete selector catalog: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.starts_with("accepted selectors: ")),
        "narrow provider choice screens should drop the full footer selector catalog once each choice already shows a preferred selector and the footer offers quick picks: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("type a model, suffix, or provider kind")),
        "narrow provider choice screens should use a shorter selector guidance line that is easier to scan than the full long-form explanation: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("you can also enter a unique model name")),
        "narrow provider choice screens should avoid wrapping the longer selector grammar sentence once the compact variant is available: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("try one of: openai, deepseek")),
        "narrow provider choice screens should keep the quick-pick footer guidance after hiding the full selector catalog: {lines:#?}"
    );
}

#[test]
fn onboard_provider_selection_screen_surfaces_responses_transport_for_choices() {
    let plan = loongclaw_daemon::migration::ProviderSelectionPlan {
        imported_choices: vec![loongclaw_daemon::migration::ImportedProviderChoice {
            profile_id: "deepseek".to_owned(),
            kind: mvp::config::ProviderKind::Deepseek,
            source: "Codex config at ~/.codex/config.toml".to_owned(),
            summary: "DeepSeek · deepseek-chat · credentials resolved".to_owned(),
            config: mvp::config::ProviderConfig {
                kind: mvp::config::ProviderKind::Deepseek,
                model: "deepseek-chat".to_owned(),
                wire_api: mvp::config::ProviderWireApi::Responses,
                api_key_env: Some("DEEPSEEK_API_KEY".to_owned()),
                ..mvp::config::ProviderConfig::default()
            },
        }],
        default_kind: Some(mvp::config::ProviderKind::Deepseek),
        default_profile_id: Some("deepseek".to_owned()),
        requires_explicit_choice: false,
    };

    let lines = loongclaw_daemon::onboard_cli::render_provider_selection_screen_lines(&plan, 80);

    assert!(
        lines.iter().any(|line| {
            line == "    transport: responses compatibility mode with chat fallback"
        }),
        "provider choice screen should surface Responses compatibility transport before the user confirms a provider: {lines:#?}"
    );
}

#[test]
fn onboard_current_setup_shortcut_screen_summarizes_existing_setup_and_choices() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.model = "gpt-4.1".to_owned();
    config.telegram.enabled = true;

    let lines =
        loongclaw_daemon::onboard_cli::render_continue_current_setup_screen_lines(&config, 80);

    assert_compact_loongclaw_header(&lines, "current-setup shortcut");
    assert!(
        lines.iter().any(|line| line == "continue current setup"),
        "current-setup shortcut should use a focused title: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("- provider: OpenAI")),
        "current-setup shortcut should summarize the active provider with the guided display name: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("- model: gpt-4.1")),
        "current-setup shortcut should summarize the active model: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- channels: telegram")),
        "current-setup shortcut should summarize enabled non-cli channels: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("1) Keep current setup (recommended)")),
        "current-setup shortcut should make the keep-as-is path primary: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("2) Adjust settings")),
        "current-setup shortcut should keep an explicit path into detailed edits: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "press Enter to use default 1, keep current setup"),
        "current-setup shortcut should make the fast-lane default explicit on the screen: {lines:#?}"
    );
}

#[test]
fn onboard_current_setup_shortcut_is_limited_to_healthy_interactive_keep_flow() {
    let _guard = EnvVarGuard::set("LOONGCLAW_WEB_SEARCH_PROVIDER", "");
    let base_options = loongclaw_daemon::onboard_cli::OnboardCommandOptions {
        output: None,
        force: false,
        non_interactive: false,
        accept_risk: false,
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
        loongclaw_daemon::onboard_cli::should_offer_current_setup_shortcut(
            &base_options,
            loongclaw_daemon::migration::types::CurrentSetupState::Healthy,
            loongclaw_daemon::onboard_cli::OnboardEntryChoice::ContinueCurrentSetup,
        ),
        "healthy interactive continue-current-setup should offer the fast lane"
    );

    let mut override_options = base_options.clone();
    override_options.model = Some("gpt-5".to_owned());
    assert!(
        !loongclaw_daemon::onboard_cli::should_offer_current_setup_shortcut(
            &override_options,
            loongclaw_daemon::migration::types::CurrentSetupState::Healthy,
            loongclaw_daemon::onboard_cli::OnboardEntryChoice::ContinueCurrentSetup,
        ),
        "explicit overrides should go straight into detailed editing instead of the fast lane"
    );

    assert!(
        !loongclaw_daemon::onboard_cli::should_offer_current_setup_shortcut(
            &base_options,
            loongclaw_daemon::migration::types::CurrentSetupState::Repairable,
            loongclaw_daemon::onboard_cli::OnboardEntryChoice::ContinueCurrentSetup,
        ),
        "repairable setups should stay on the explicit review/edit path"
    );
}

#[test]
fn onboard_current_setup_shortcut_is_disabled_by_web_search_provider_override_env() {
    let _guard = EnvVarGuard::set("LOONGCLAW_WEB_SEARCH_PROVIDER", "tavily");
    let options = loongclaw_daemon::onboard_cli::OnboardCommandOptions {
        output: None,
        force: false,
        non_interactive: false,
        accept_risk: false,
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
        !loongclaw_daemon::onboard_cli::should_offer_current_setup_shortcut(
            &options,
            loongclaw_daemon::migration::types::CurrentSetupState::Healthy,
            loongclaw_daemon::onboard_cli::OnboardEntryChoice::ContinueCurrentSetup,
        ),
        "an explicit web-search provider override should force the detailed onboarding path"
    );
}

#[test]
fn onboard_current_setup_shortcut_is_disabled_by_web_search_provider_option() {
    let options = loongclaw_daemon::onboard_cli::OnboardCommandOptions {
        output: None,
        force: false,
        non_interactive: false,
        accept_risk: false,
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

    assert!(
        !loongclaw_daemon::onboard_cli::should_offer_current_setup_shortcut(
            &options,
            loongclaw_daemon::migration::types::CurrentSetupState::Healthy,
            loongclaw_daemon::onboard_cli::OnboardEntryChoice::ContinueCurrentSetup,
        ),
        "an explicit --web-search-provider option should force the detailed onboarding path"
    );
}

#[test]
fn onboard_detected_setup_shortcut_screen_summarizes_starting_point_and_choices() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.model = "gpt-5.4".to_owned();
    config.telegram.enabled = true;

    let lines = loongclaw_daemon::onboard_cli::render_continue_detected_setup_screen_lines(
        &config,
        "Codex config at ~/.codex/config.toml",
        80,
    );

    assert_compact_loongclaw_header(&lines, "detected-setup shortcut");
    assert!(
        lines
            .iter()
            .any(|line| line == "continue with detected starting point"),
        "detected-setup shortcut should use a focused title: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- starting point: Codex config at ~/.codex/config.toml")),
        "detected-setup shortcut should keep the chosen starting point visible: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("- provider: OpenAI")),
        "detected-setup shortcut should summarize the active provider with the guided display name: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("- model: gpt-5.4")),
        "detected-setup shortcut should summarize the active model: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- channels: telegram")),
        "detected-setup shortcut should summarize enabled non-cli channels: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("1) Use detected starting point (recommended)")),
        "detected-setup shortcut should make the detected fast lane primary: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("skip detailed edits and continue to quick review")),
        "detected-setup shortcut should explain that the fast lane still goes through a quick review: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("go straight to verification and next actions")),
        "detected-setup shortcut should not imply that review is skipped entirely: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("2) Adjust settings")),
        "detected-setup shortcut should keep an explicit path into detailed edits: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "press Enter to use default 1, the detected starting point"),
        "detected-setup shortcut should make the fast-lane default explicit on the screen: {lines:#?}"
    );
}

#[test]
fn onboard_detected_setup_shortcut_is_limited_to_interactive_import_flow_with_default_provider_choice()
 {
    let _guard = EnvVarGuard::set("LOONGCLAW_WEB_SEARCH_PROVIDER", "");
    let base_options = loongclaw_daemon::onboard_cli::OnboardCommandOptions {
        output: None,
        force: false,
        non_interactive: false,
        accept_risk: false,
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
    let default_provider_plan = loongclaw_daemon::migration::ProviderSelectionPlan {
        imported_choices: Vec::new(),
        default_kind: Some(mvp::config::ProviderKind::Openai),
        default_profile_id: Some("openai".to_owned()),
        requires_explicit_choice: false,
    };

    assert!(
        loongclaw_daemon::onboard_cli::should_offer_detected_setup_shortcut(
            &base_options,
            loongclaw_daemon::onboard_cli::OnboardEntryChoice::ImportDetectedSetup,
            &default_provider_plan,
        ),
        "interactive detected-setup flows with a default provider should offer the fast lane"
    );

    let mut override_options = base_options.clone();
    override_options.api_key_env = Some("DEEPSEEK_API_KEY".to_owned());
    assert!(
        !loongclaw_daemon::onboard_cli::should_offer_detected_setup_shortcut(
            &override_options,
            loongclaw_daemon::onboard_cli::OnboardEntryChoice::ImportDetectedSetup,
            &default_provider_plan,
        ),
        "explicit overrides should go straight into detailed editing instead of the fast lane"
    );

    let explicit_choice_plan = loongclaw_daemon::migration::ProviderSelectionPlan {
        imported_choices: Vec::new(),
        default_kind: None,
        default_profile_id: None,
        requires_explicit_choice: true,
    };
    assert!(
        !loongclaw_daemon::onboard_cli::should_offer_detected_setup_shortcut(
            &base_options,
            loongclaw_daemon::onboard_cli::OnboardEntryChoice::ImportDetectedSetup,
            &explicit_choice_plan,
        ),
        "detected setups that still need an explicit provider choice should not skip the guided path"
    );

    assert!(
        !loongclaw_daemon::onboard_cli::should_offer_detected_setup_shortcut(
            &base_options,
            loongclaw_daemon::onboard_cli::OnboardEntryChoice::ContinueCurrentSetup,
            &default_provider_plan,
        ),
        "the detected-setup fast lane should stay scoped to detected-setup entry choices"
    );
}

#[test]
fn onboard_detected_setup_shortcut_is_disabled_by_web_search_provider_override_env() {
    let _guard = EnvVarGuard::set("LOONGCLAW_WEB_SEARCH_PROVIDER", "tavily");
    let options = loongclaw_daemon::onboard_cli::OnboardCommandOptions {
        output: None,
        force: false,
        non_interactive: false,
        accept_risk: false,
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
    let plan = loongclaw_daemon::migration::ProviderSelectionPlan {
        imported_choices: Vec::new(),
        default_kind: Some(mvp::config::ProviderKind::Openai),
        default_profile_id: Some("openai".to_owned()),
        requires_explicit_choice: false,
    };

    assert!(
        !loongclaw_daemon::onboard_cli::should_offer_detected_setup_shortcut(
            &options,
            loongclaw_daemon::onboard_cli::OnboardEntryChoice::ImportDetectedSetup,
            &plan,
        ),
        "an explicit web-search provider override should force the detailed detected-setup path"
    );
}

#[test]
fn onboard_detected_setup_shortcut_is_disabled_by_web_search_provider_option() {
    let options = loongclaw_daemon::onboard_cli::OnboardCommandOptions {
        output: None,
        force: false,
        non_interactive: false,
        accept_risk: false,
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
    let plan = loongclaw_daemon::migration::ProviderSelectionPlan {
        imported_choices: Vec::new(),
        default_kind: Some(mvp::config::ProviderKind::Openai),
        default_profile_id: Some("openai".to_owned()),
        requires_explicit_choice: false,
    };

    assert!(
        !loongclaw_daemon::onboard_cli::should_offer_detected_setup_shortcut(
            &options,
            loongclaw_daemon::onboard_cli::OnboardEntryChoice::ImportDetectedSetup,
            &plan,
        ),
        "an explicit --web-search-provider option should force the detailed detected-setup path"
    );
}

#[test]
fn onboard_starting_point_selection_screen_uses_compact_header_and_detected_options() {
    let mut recommended = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan,
        "recommended import plan",
        mvp::config::ProviderKind::Openai,
        "openai/gpt-5.1-codex",
        "OPENAI_API_KEY",
    );
    recommended
        .domains
        .push(loongclaw_daemon::migration::types::DomainPreview {
            kind: loongclaw_daemon::migration::types::SetupDomainKind::WorkspaceGuidance,
            status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
            decision: Some(loongclaw_daemon::migration::types::PreviewDecision::UseDetected),
            source: "/tmp/project/AGENTS.md".to_owned(),
            summary: "AGENTS.md detected".to_owned(),
        });
    let env = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::Environment,
        "your current environment",
        mvp::config::ProviderKind::Deepseek,
        "deepseek-chat",
        "DEEPSEEK_API_KEY",
    );

    let lines = loongclaw_daemon::onboard_cli::render_starting_point_selection_screen_lines(
        &[recommended, env],
        80,
    );

    assert_compact_loongclaw_header(&lines, "starting-point screen");
    assert!(
        lines
            .iter()
            .any(|line| line == "choose detected starting point"),
        "starting-point screen should use a focused decision title: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("1) suggested starting point (recommended)")),
        "starting-point screen should promote the suggested starting point first: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("OpenAI · openai/gpt-5.1-codex")),
        "starting-point screen should summarize provider/model details for the first option: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("AGENTS.md detected")),
        "starting-point screen should surface workspace guidance signals in the option details: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "press Enter to use default 1, the suggested starting point"),
        "starting-point screen should make the default Enter behavior explicit when a suggested starting point is available: {lines:#?}"
    );
}

#[test]
fn onboard_starting_point_selection_screen_deduplicates_workspace_guidance_and_channel_rollups() {
    let mut recommended = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan,
        "recommended import plan",
        mvp::config::ProviderKind::Deepseek,
        "deepseek-chat",
        "DEEPSEEK_API_KEY",
    );
    recommended
        .channel_candidates
        .push(loongclaw_daemon::migration::types::ChannelCandidate {
            id: "telegram",
            label: "telegram",
            status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
            source: "your current environment".to_owned(),
            summary: "enabled · token resolved · 0 allowed chat id(s)".to_owned(),
        });
    recommended
        .domains
        .push(loongclaw_daemon::migration::types::DomainPreview {
            kind: loongclaw_daemon::migration::types::SetupDomainKind::Channels,
            status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
            decision: Some(loongclaw_daemon::migration::types::PreviewDecision::Supplement),
            source: "multiple sources".to_owned(),
            summary: "telegram Ready from your current environment".to_owned(),
        });
    recommended.workspace_guidance.push(
        loongclaw_daemon::migration::types::WorkspaceGuidanceCandidate {
            kind: loongclaw_daemon::migration::types::WorkspaceGuidanceKind::Agents,
            path: "/tmp/project/AGENTS.md".to_owned(),
        },
    );
    recommended
        .domains
        .push(loongclaw_daemon::migration::types::DomainPreview {
            kind: loongclaw_daemon::migration::types::SetupDomainKind::WorkspaceGuidance,
            status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
            decision: Some(loongclaw_daemon::migration::types::PreviewDecision::UseDetected),
            source: "workspace".to_owned(),
            summary: "AGENTS.md".to_owned(),
        });

    let lines = loongclaw_daemon::onboard_cli::render_starting_point_selection_screen_lines(
        &[recommended],
        80,
    );

    assert!(
        lines
            .iter()
            .filter(|line| line.contains("workspace guidance"))
            .count()
            == 1,
        "starting-point details should not repeat workspace guidance when a candidate already lists the detected files: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("channels: telegram Ready from")),
        "starting-point details should avoid a redundant channel rollup when the per-channel detail lines are already present: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("telegram: enabled · token resolved")),
        "starting-point details should keep the readable per-channel summary: {lines:#?}"
    );
}

#[test]
fn onboard_starting_point_selection_screen_summarizes_multi_source_origin() {
    let mut recommended = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan,
        "recommended import plan",
        mvp::config::ProviderKind::Openai,
        "openai/gpt-5.1-codex",
        "OPENAI_API_KEY",
    );
    recommended.domains[0].source = "existing config at ~/.config/loongclaw/config.toml".to_owned();
    recommended
        .channel_candidates
        .push(loongclaw_daemon::migration::types::ChannelCandidate {
            id: "telegram",
            label: "telegram",
            status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
            source: "your current environment".to_owned(),
            summary: "enabled · token resolved · 0 allowed chat id(s)".to_owned(),
        });
    recommended.workspace_guidance.push(
        loongclaw_daemon::migration::types::WorkspaceGuidanceCandidate {
            kind: loongclaw_daemon::migration::types::WorkspaceGuidanceKind::Agents,
            path: "/tmp/project/AGENTS.md".to_owned(),
        },
    );

    let joined = loongclaw_daemon::onboard_cli::render_starting_point_selection_screen_lines(
        &[recommended],
        80,
    )
    .join("\n");

    assert!(
        joined.contains("sources:"),
        "starting-point details should summarize the origin of a composed detected setup: {joined}"
    );
    assert!(
        joined.contains("existing config at ~/.config/loongclaw/config.toml"),
        "starting-point details should keep the current-config contribution visible: {joined}"
    );
    assert!(
        joined.contains("your current") && joined.contains("environment"),
        "starting-point details should keep environment-derived contributions visible: {joined}"
    );
    assert!(
        joined.contains("workspace guidance"),
        "starting-point details should call out workspace guidance as one of the composed sources: {joined}"
    );
}

#[test]
fn onboard_single_detected_setup_preview_screen_uses_compact_follow_up_layout() {
    let candidate = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
        "Codex config at ~/.codex/config.toml",
        mvp::config::ProviderKind::Openai,
        "openai/gpt-5.1-codex",
        "OPENAI_API_KEY",
    );

    let lines = loongclaw_daemon::onboard_cli::render_single_detected_setup_preview_screen_lines(
        &candidate,
        std::slice::from_ref(&candidate),
        80,
    );

    assert_compact_loongclaw_header(&lines, "single detected-setup preview");
    assert!(
        lines
            .iter()
            .any(|line| line == "review detected starting point"),
        "single detected-setup preview should use a focused title instead of a bare inline label: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("continuing with the only detected starting point")),
        "single detected-setup preview should explain why no separate starting-point chooser is shown: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("source: Codex config at ~/.codex/config.toml")),
        "single detected-setup preview should still show the detected source attribution: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("good fit: reuse Codex config as your starting point")),
        "single detected-setup preview should explain why this detected starting point is being carried forward: {lines:#?}"
    );
}

#[test]
fn onboard_starting_point_selection_screen_surfaces_keep_and_supplement_actions() {
    let mut recommended = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan,
        "recommended import plan",
        mvp::config::ProviderKind::Openai,
        "openai/gpt-5.1-codex",
        "OPENAI_API_KEY",
    );
    recommended.domains[0].decision =
        Some(loongclaw_daemon::migration::types::PreviewDecision::KeepCurrent);
    recommended
        .domains
        .push(loongclaw_daemon::migration::types::DomainPreview {
            kind: loongclaw_daemon::migration::types::SetupDomainKind::Channels,
            status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
            decision: Some(loongclaw_daemon::migration::types::PreviewDecision::Supplement),
            source: "multiple sources".to_owned(),
            summary: "telegram Ready".to_owned(),
        });

    let lines = loongclaw_daemon::onboard_cli::render_starting_point_selection_screen_lines(
        &[recommended],
        80,
    );

    assert!(
        lines.iter().any(|line| {
            line.contains("provider: keep current value")
                || line.contains("provider: use detected value")
        }),
        "starting-point details should expose how the provider domain will be handled: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("channels: supplement with detected values")),
        "starting-point details should expose when channels are supplemented across sources: {lines:#?}"
    );
}

#[test]
fn onboard_starting_point_selection_screen_explains_why_suggested_starting_point_is_recommended() {
    let mut recommended = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan,
        "recommended import plan",
        mvp::config::ProviderKind::Openai,
        "openai/gpt-5.1-codex",
        "OPENAI_API_KEY",
    );
    recommended.domains[0].decision =
        Some(loongclaw_daemon::migration::types::PreviewDecision::KeepCurrent);
    recommended
        .domains
        .push(loongclaw_daemon::migration::types::DomainPreview {
            kind: loongclaw_daemon::migration::types::SetupDomainKind::Channels,
            status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
            decision: Some(loongclaw_daemon::migration::types::PreviewDecision::Supplement),
            source: "your current environment".to_owned(),
            summary: "telegram Ready".to_owned(),
        });
    recommended.workspace_guidance.push(
        loongclaw_daemon::migration::types::WorkspaceGuidanceCandidate {
            kind: loongclaw_daemon::migration::types::WorkspaceGuidanceKind::Agents,
            path: "/tmp/project/AGENTS.md".to_owned(),
        },
    );

    let lines = loongclaw_daemon::onboard_cli::render_starting_point_selection_screen_lines(
        &[recommended],
        80,
    );
    let joined = lines.join("\n");

    assert!(
        joined
            .contains("good fit: keep current provider + add detected channels + reuse workspace")
            && joined.contains("guidance"),
        "starting-point screen should explain why the suggested starting point is the recommended choice: {lines:#?}"
    );
}

#[test]
fn onboard_starting_point_selection_screen_explains_when_direct_source_is_a_good_fit() {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    unsafe {
        std::env::set_var("OPENAI_API_KEY", "openai-test-token");
        std::env::set_var("DEEPSEEK_API_KEY", "deepseek-test-token");
    }
    let codex = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
        "Codex config at ~/.codex/config.toml",
        mvp::config::ProviderKind::Openai,
        "openai/gpt-5.1-codex",
        "OPENAI_API_KEY",
    );
    let environment = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::Environment,
        "your current environment",
        mvp::config::ProviderKind::Deepseek,
        "deepseek-chat",
        "DEEPSEEK_API_KEY",
    );

    let joined = loongclaw_daemon::onboard_cli::render_starting_point_selection_screen_lines(
        &[codex, environment],
        80,
    )
    .join("\n");

    assert!(
        joined.contains("good fit: reuse Codex config as your starting point"),
        "starting-point screen should explain when a Codex-derived starting point is the right choice: {joined}"
    );
    assert!(
        joined.contains("good fit: start from detected environment settings"),
        "starting-point screen should explain when the environment-derived starting point is the right choice: {joined}"
    );
    assert!(
        joined.contains("provider: OpenAI · openai/gpt-5.1-codex · credentials resolved"),
        "direct-source starting points should summarize provider details with the guided display name without repeating decision jargon: {joined}"
    );
    assert!(
        !joined.contains("provider: use detected value · OpenAI"),
        "direct-source starting points should avoid repeating detected-decision wording once the card already explains the fit: {joined}"
    );
}

#[test]
fn onboard_starting_point_selection_screen_explains_explicit_path_fit() {
    let explicit = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::ExplicitPath,
        "selected config at /tmp/loongclaw-import.toml",
        mvp::config::ProviderKind::Openai,
        "openai/gpt-5.1-codex",
        "OPENAI_API_KEY",
    );

    let joined = loongclaw_daemon::onboard_cli::render_starting_point_selection_screen_lines(
        &[explicit],
        80,
    )
    .join("\n");

    assert!(
        joined.contains("good fit: reuse the selected config file as your starting point"),
        "starting-point screen should keep explicit-path copy ready for future path-based import entry points: {joined}"
    );
}

#[test]
fn onboard_starting_point_selection_screen_explains_when_direct_source_can_supplement_setup() {
    let mut environment = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::Environment,
        "your current environment",
        mvp::config::ProviderKind::Deepseek,
        "deepseek-chat",
        "DEEPSEEK_API_KEY",
    );
    environment
        .channel_candidates
        .push(loongclaw_daemon::migration::types::ChannelCandidate {
            id: "telegram",
            label: "telegram",
            status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
            source: "your current environment".to_owned(),
            summary: "enabled · token resolved".to_owned(),
        });
    environment
        .domains
        .push(loongclaw_daemon::migration::types::DomainPreview {
            kind: loongclaw_daemon::migration::types::SetupDomainKind::Channels,
            status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
            decision: Some(loongclaw_daemon::migration::types::PreviewDecision::Supplement),
            source: "your current environment".to_owned(),
            summary: "telegram Ready".to_owned(),
        });
    environment.workspace_guidance.push(
        loongclaw_daemon::migration::types::WorkspaceGuidanceCandidate {
            kind: loongclaw_daemon::migration::types::WorkspaceGuidanceKind::Agents,
            path: "/tmp/project/AGENTS.md".to_owned(),
        },
    );

    let joined = loongclaw_daemon::onboard_cli::render_starting_point_selection_screen_lines(
        &[environment],
        120,
    )
    .join("\n");

    assert!(
        joined.contains(
            "good fit: start from detected environment settings + add detected channels + reuse workspace guidance"
        ),
        "starting-point screen should explain both the direct source and the supplemental setup it brings: {joined}"
    );
}

#[test]
fn onboard_starting_point_selection_screen_explains_why_starting_fresh_is_a_good_fit() {
    let candidate = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
        "Codex config at ~/.codex/config.toml",
        mvp::config::ProviderKind::Openai,
        "openai/gpt-5.1-codex",
        "OPENAI_API_KEY",
    );

    let joined = loongclaw_daemon::onboard_cli::render_starting_point_selection_screen_lines(
        &[candidate],
        80,
    )
    .join("\n");

    assert!(
        joined.contains("good fit: start clean with full control"),
        "starting-point screen should explain why starting fresh is the right choice for users who want a manual setup: {joined}"
    );
}

#[test]
fn onboard_starting_point_selection_screen_prioritizes_richer_direct_sources() {
    let codex = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
        "Codex config at ~/.codex/config.toml",
        mvp::config::ProviderKind::Openai,
        "openai/gpt-5.1-codex",
        "OPENAI_API_KEY",
    );
    let mut environment = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::Environment,
        "your current environment",
        mvp::config::ProviderKind::Deepseek,
        "deepseek-chat",
        "DEEPSEEK_API_KEY",
    );
    environment
        .channel_candidates
        .push(loongclaw_daemon::migration::types::ChannelCandidate {
            id: "telegram",
            label: "telegram",
            status: loongclaw_daemon::migration::types::PreviewStatus::Ready,
            source: "your current environment".to_owned(),
            summary: "enabled · token resolved".to_owned(),
        });
    environment.workspace_guidance.push(
        loongclaw_daemon::migration::types::WorkspaceGuidanceCandidate {
            kind: loongclaw_daemon::migration::types::WorkspaceGuidanceKind::Agents,
            path: "/tmp/project/AGENTS.md".to_owned(),
        },
    );

    let joined = loongclaw_daemon::onboard_cli::render_starting_point_selection_screen_lines(
        &[codex, environment],
        80,
    )
    .join("\n");

    let environment_index = joined
        .find("1) your current environment")
        .expect("environment option should render first when it covers more setup domains");
    let codex_index = joined
        .find("2) Codex config at ~/.codex/config.toml")
        .expect("codex option should render after the richer environment candidate");

    assert!(
        environment_index < codex_index,
        "starting-point screen should prioritize broader direct sources ahead of narrower ones: {joined}"
    );
}

#[test]
fn onboard_starting_point_selection_screen_prefers_explicit_config_sources_when_coverage_ties() {
    let codex = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
        "Codex config at ~/.codex/config.toml",
        mvp::config::ProviderKind::Openai,
        "openai/gpt-5.1-codex",
        "OPENAI_API_KEY",
    );
    let environment = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::Environment,
        "your current environment",
        mvp::config::ProviderKind::Deepseek,
        "deepseek-chat",
        "DEEPSEEK_API_KEY",
    );

    let joined = loongclaw_daemon::onboard_cli::render_starting_point_selection_screen_lines(
        &[environment, codex],
        80,
    )
    .join("\n");

    let codex_index = joined
        .find("1) Codex config at ~/.codex/config.toml")
        .expect("codex option should render first when direct-source coverage is tied");
    let environment_index = joined
        .find("2) your current environment")
        .expect("environment option should render after codex when coverage is tied");

    assert!(
        codex_index < environment_index,
        "starting-point screen should prefer explicit config sources over ambient environment sources when they are otherwise equally complete: {joined}"
    );
}

#[test]
fn onboard_starting_point_selection_screen_wraps_long_option_labels_and_details() {
    let mut codex = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
        "Codex config at ~/.codex/agents/loongclaw/config.toml",
        mvp::config::ProviderKind::Openai,
        "openai/gpt-5.1-codex",
        "OPENAI_API_KEY",
    );
    codex.domains[0].summary =
        "OpenAI · openai/gpt-5.1-codex · credentials resolved from environment".to_owned();

    let lines =
        loongclaw_daemon::onboard_cli::render_starting_point_selection_screen_lines(&[codex], 48);

    assert!(
        lines.iter().any(|line| line == "1) Codex config at"),
        "starting-point screen should wrap long option labels instead of overflowing them: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "   ~/.codex/agents/loongclaw/config.toml"),
        "starting-point screen should continue long option labels on an indented line: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "    provider: OpenAI · openai/gpt-5.1-codex ·"),
        "direct-source starting-point cards should keep the concise detail label visible before wrapping long summaries, using the guided display name: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "      credentials resolved from environment"),
        "starting-point screen should continue long detail summaries on readable continuation lines: {lines:#?}"
    );
}

#[test]
fn onboard_starting_point_selection_screen_wraps_header_title_and_subtitle_on_narrow_width() {
    let candidate = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan,
        "recommended import plan",
        mvp::config::ProviderKind::Openai,
        "openai/gpt-5.1-codex",
        "OPENAI_API_KEY",
    );

    let lines = loongclaw_daemon::onboard_cli::render_starting_point_selection_screen_lines(
        &[candidate],
        22,
    );

    assert!(
        lines.iter().all(|line| line.len() <= 22),
        "starting-point screen should keep brand subtitle and title within narrow widths: {lines:#?}"
    );
    assert_eq!(lines[0], "LOONGCLAW");
    assert!(
        lines.iter().any(|line| line == "choose detected"),
        "narrow starting-point screen should wrap the long title instead of leaving it on one overflowing line: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "starting point"),
        "narrow starting-point screen should continue the wrapped title on a readable second line: {lines:#?}"
    );
}

#[test]
fn onboard_model_selection_screen_keeps_provider_context() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Deepseek;
    config.provider.model = "deepseek-reasoner".to_owned();

    let lines = loongclaw_daemon::onboard_cli::render_model_selection_screen_lines(&config, 80);

    assert_compact_loongclaw_header(&lines, "model screen");
    assert!(
        lines.iter().any(|line| line == "choose model"),
        "model screen should use a focused title: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "step 2 of 8 · model"),
        "model screen should include guided progress context without relying on an external step header: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- provider: DeepSeek")),
        "model screen should keep provider context visible with the guided display name: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- current model: deepseek-reasoner")),
        "model screen should show the current model before prompting: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "- press Enter to keep current model"),
        "model screen should explain that Enter keeps the current model instead of using a vague current-value hint: {lines:#?}"
    );
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.as_str() == "choose model")
            .count(),
        1,
        "model screen should avoid duplicating the title in both the compact header and the body: {lines:#?}"
    );
    assert!(
        lines.iter().all(|line| line.as_str() != "choose the model"),
        "model screen should not repeat a near-identical subtitle above the main title: {lines:#?}"
    );
}

#[test]
fn onboard_model_selection_screen_shows_prefilled_model_when_enter_default_differs() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Openai;
    config.provider.model = "openai/gpt-5.1-codex".to_owned();

    let lines = loongclaw_daemon::onboard_cli::render_model_selection_screen_lines_with_default(
        &config,
        "openai/gpt-5.2",
        80,
    );

    assert!(
        lines
            .iter()
            .any(|line| line == "- press Enter to use prefilled model: openai/gpt-5.2"),
        "model screen should surface the actual Enter default when it differs from the current model: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| line != "- press Enter to keep current model"),
        "model screen should not claim Enter keeps the current model when a different default is prefilled: {lines:#?}"
    );
}

#[test]
fn onboard_model_selection_screen_wraps_compact_header_and_progress_on_narrow_width() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Openai;
    config.provider.model = "openai/gpt-5.1-codex".to_owned();

    let lines = loongclaw_daemon::onboard_cli::render_model_selection_screen_lines(&config, 22);

    assert!(
        lines.iter().all(|line| line.len() <= 22),
        "model screen should keep compact header and progress copy within narrow terminal widths: {lines:#?}"
    );
    assert_eq!(
        lines[0], "LOONGCLAW",
        "narrow model screen should split the compact header instead of forcing brand and version onto one line: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "step 2 of 8 · model"),
        "narrow model screen should still keep the step context visible: {lines:#?}"
    );
}

#[test]
fn onboard_api_key_env_screen_explains_suggested_env_and_blank_behavior() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Openai;
    config.provider.api_key_env = None;
    config.provider.oauth_access_token_env = Some("OPENAI_CODEX_OAUTH_TOKEN".to_owned());

    let lines = loongclaw_daemon::onboard_cli::render_api_key_env_selection_screen_lines(
        &config,
        "OPENAI_API_KEY",
        80,
    );

    assert_compact_loongclaw_header(&lines, "credential-env screen");
    assert!(
        lines.iter().any(|line| line == "choose credential source"),
        "credential-env screen should use a focused title: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "step 3 of 8 · credential source"),
        "credential-env screen should include guided progress context inside the screen: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("- provider: OpenAI")),
        "credential-env screen should keep provider context visible with the guided display name: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- current source: OPENAI_CODEX_OAUTH_TOKEN")),
        "credential-env screen should show the active oauth credential source instead of hiding it behind api-key-only rendering: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- suggested source: OPENAI_API_KEY")),
        "credential-env screen should surface the suggested env var name: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "- press Enter to use suggested source: OPENAI_API_KEY"),
        "credential-env screen should state that Enter uses the suggested env when no current env is set: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("env var name")
                && line.contains("not the secret value itself")),
        "credential-env screen should clearly distinguish the env var name from the underlying secret value: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "- type :clear to clear the configured credential env"),
        "credential-env screen should explain the explicit clear token when another credential env is already configured: {lines:#?}"
    );
}

#[test]
fn onboard_api_key_env_screen_shows_prefilled_env_when_enter_default_is_overridden() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Openai;
    config.provider.api_key_env = Some("OPENAI_API_KEY".to_owned());

    let lines =
        loongclaw_daemon::onboard_cli::render_api_key_env_selection_screen_lines_with_default(
            &config,
            "OPENAI_API_KEY",
            "TEAM_OPENAI_KEY",
            80,
        );

    assert!(
        lines
            .iter()
            .any(|line| line == "- press Enter to use prefilled source: TEAM_OPENAI_KEY"),
        "credential-env screen should surface the actual prefilled env when it differs from both the current and suggested env: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| line != "- press Enter to keep current source"),
        "credential-env screen should not claim Enter keeps the current env when another env is prefilled: {lines:#?}"
    );
}

#[test]
fn onboard_api_key_env_screen_wraps_long_unbroken_env_names() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Openai;
    config.provider.api_key_env =
        Some("OPENAI_COMPATIBLE_PROVIDER_SUPER_LONG_ENV_POINTER".to_owned());

    let lines = loongclaw_daemon::onboard_cli::render_api_key_env_selection_screen_lines(
        &config,
        "OPENAI_COMPATIBLE_PROVIDER_DEFAULT_ENV_POINTER",
        36,
    );

    assert!(
        lines.iter().all(|line| line.len() <= 36),
        "credential-env screen should split long env tokens instead of letting them overflow narrow widths: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("- current source: ")),
        "credential-env screen should keep the current-env label visible while wrapping long values: {lines:#?}"
    );
}

#[test]
fn onboard_api_key_env_screen_wraps_progress_line_on_narrow_width() {
    let config = mvp::config::LoongClawConfig::default();

    let lines = loongclaw_daemon::onboard_cli::render_api_key_env_selection_screen_lines(
        &config,
        "OPENAI_API_KEY",
        22,
    );

    assert!(
        lines.iter().all(|line| line.len() <= 22),
        "credential-env screen should keep the progress line within narrow terminal widths: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "step 3 of 8 ·"),
        "narrow credential-env screen should keep the step label on the first wrapped line: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "credential source"),
        "narrow credential-env screen should continue the wrapped progress line on a second line: {lines:#?}"
    );
}

#[test]
fn onboard_api_key_env_screen_redacts_invalid_current_source_and_keeps_clear_hint() {
    let secret = "sk-live-direct-secret-value";
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Openai;
    config.provider.api_key_env = Some(secret.to_owned());

    let lines = loongclaw_daemon::onboard_cli::render_api_key_env_selection_screen_lines(
        &config,
        "OPENAI_API_KEY",
        80,
    );

    assert!(
        lines
            .iter()
            .any(|line| line == "- current source: environment variable"),
        "credential-env screen should redact invalid configured env pointers instead of hiding them or inventing defaults: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "- type :clear to clear the configured credential env"),
        "credential-env screen should still offer the clear token when the configured env pointer is present but redacted: {lines:#?}"
    );
    assert!(
        lines.iter().all(|line| !line.contains(secret)),
        "credential-env screen must never echo the invalid secret-like configured env pointer: {lines:#?}"
    );
}

#[test]
fn onboard_system_prompt_screen_explains_blank_behavior() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.cli.system_prompt = "be terse and code-focused".to_owned();

    let lines =
        loongclaw_daemon::onboard_cli::render_system_prompt_selection_screen_lines(&config, 80);

    assert_compact_loongclaw_header(&lines, "system-prompt screen");
    assert!(
        lines.iter().any(|line| line == "adjust cli behavior"),
        "system-prompt screen should frame this as a behavior adjustment: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "step 4 of 7 · system prompt"),
        "system-prompt screen should include guided progress context inside the screen: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- current prompt: be terse and code-focused")),
        "system-prompt screen should show the current prompt value before editing: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "- press Enter to keep current prompt"),
        "system-prompt screen should explain that Enter keeps the current prompt when no other default is prefilled: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "- type :clear to use the built-in behavior"),
        "system-prompt screen should explain how to restore the built-in behavior when Enter keeps the current prompt: {lines:#?}"
    );
}

#[test]
fn onboard_system_prompt_screen_shows_prefilled_prompt_when_enter_default_differs() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.cli.system_prompt = "be terse and code-focused".to_owned();

    let lines =
        loongclaw_daemon::onboard_cli::render_system_prompt_selection_screen_lines_with_default(
            &config,
            "speak with concise release-manager tone",
            80,
        );

    assert!(
        lines.iter().any(|line| {
            line == "- press Enter to use prefilled prompt: speak with concise release-manager tone"
        }),
        "system-prompt screen should surface the actual prefilled prompt when Enter no longer keeps the current prompt: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| line != "- press Enter to keep current prompt"),
        "system-prompt screen should not claim Enter keeps the current prompt when another prompt is prefilled: {lines:#?}"
    );
}

#[test]
fn onboard_system_prompt_screen_wraps_long_current_prompt() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.cli.system_prompt =
        "keep replies short and code-focused when reviewing repo state".to_owned();

    let lines =
        loongclaw_daemon::onboard_cli::render_system_prompt_selection_screen_lines(&config, 48);

    assert_compact_loongclaw_header(&lines, "system-prompt screen");
    assert!(
        lines
            .iter()
            .any(|line| line == "- current prompt: keep replies short and"),
        "system-prompt screen should keep the current-prompt label visible before wrapping long text: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "  code-focused when reviewing repo state"),
        "system-prompt screen should continue wrapped prompt text on an indented line: {lines:#?}"
    );
}

#[test]
fn onboard_personality_selection_screen_shows_native_personality_choices() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.cli.personality = Some(mvp::prompt::PromptPersonality::FriendlyCollab);

    let lines = crate::onboard_cli::render_personality_selection_screen_lines(&config, 80);

    assert_compact_loongclaw_header(&lines, "personality screen");
    assert!(
        lines.iter().all(|line| !line.starts_with("██╗")),
        "personality screen should not repeat the large LOONGCLAW banner mid-onboarding: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "choose personality"),
        "personality screen should use a focused title: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "step 4 of 8 · personality"),
        "personality screen should surface the native prompt-pack progress step: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("friendly_collab)")),
        "personality screen should keep the canonical friendly_collab selector visible without bracket syntax: {lines:#?}"
    );
    assert!(
        lines.iter().all(|line| !line.contains("[friendly_collab]")),
        "personality screen should not imply that brackets are part of the expected selector syntax: {lines:#?}"
    );
}

#[test]
fn onboard_prompt_addendum_screen_explains_keep_and_clear_behavior() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.cli.system_prompt_addendum = Some("Keep answers direct.".to_owned());

    let lines = crate::onboard_cli::render_prompt_addendum_selection_screen_lines(&config, 80);

    assert!(
        lines.iter().any(|line| line == "adjust prompt addendum"),
        "prompt-addendum screen should use a focused title: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "step 5 of 8 · prompt addendum"),
        "prompt-addendum screen should surface the native prompt-pack progress step: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "- press Enter to keep current addendum"),
        "prompt-addendum screen should explain the Enter behavior in the same style as other input screens: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("type '-' to clear it")),
        "prompt-addendum screen should explain how to clear the current addendum: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "- single-line input only"),
        "prompt-addendum screen should keep the input instruction concise and consistent: {lines:#?}"
    );
}

#[test]
fn onboard_memory_profile_screen_shows_supported_profiles() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.memory.profile = mvp::config::MemoryProfile::ProfilePlusWindow;

    let lines = crate::onboard_cli::render_memory_profile_selection_screen_lines(&config, 80);

    assert_compact_loongclaw_header(&lines, "memory-profile screen");
    assert!(
        lines.iter().any(|line| line == "choose memory profile"),
        "memory-profile screen should use a focused title: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "step 6 of 8 · memory profile"),
        "memory-profile screen should surface the native prompt-pack progress step: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("profile_plus_window)")),
        "memory-profile screen should keep the canonical profile_plus_window selector visible without bracket syntax: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("[profile_plus_window]")),
        "memory-profile screen should not imply that brackets are part of the expected selector syntax: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("only load the recent conversation turns")),
        "memory-profile screen should explain the lightest profile in plain language: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("plus a short summary of earlier context")),
        "memory-profile screen should explain the summary profile in plain language: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("plus durable profile notes")),
        "memory-profile screen should explain the durable profile option in plain language: {lines:#?}"
    );
}

#[test]
fn onboard_provider_selection_uses_imported_provider_config_for_selected_choice() {
    let recommended = import_candidate_with_kind(
        loongclaw_daemon::migration::types::ImportSourceKind::RecommendedPlan,
        "recommended import plan",
    );
    let deepseek = import_candidate_with_provider(
        loongclaw_daemon::migration::types::ImportSourceKind::Environment,
        "your current environment",
        mvp::config::ProviderKind::Deepseek,
        "deepseek-chat",
        "DEEPSEEK_API_KEY",
    );
    let plan = loongclaw_daemon::onboard_cli::build_provider_selection_plan_for_candidate(
        &recommended,
        &[recommended.clone(), deepseek],
    );

    let resolved = loongclaw_daemon::onboard_cli::resolve_provider_config_from_selection(
        &mvp::config::ProviderConfig::default(),
        &plan,
        mvp::config::ProviderKind::Deepseek,
    );

    assert_eq!(resolved.kind, mvp::config::ProviderKind::Deepseek);
    assert_eq!(resolved.model, "deepseek-chat");
    assert_eq!(resolved.api_key_env.as_deref(), Some("DEEPSEEK_API_KEY"));
}

#[test]
fn onboard_provider_selection_manual_override_resets_model_for_new_provider() {
    let current = mvp::config::ProviderConfig {
        kind: mvp::config::ProviderKind::Openai,
        model: "openai/gpt-5.1-codex".to_owned(),
        api_key_env: Some("OPENAI_API_KEY".to_owned()),
        ..mvp::config::ProviderConfig::default()
    };
    let plan = loongclaw_daemon::onboard_cli::build_provider_selection_plan_for_candidate(
        &import_candidate_with_provider(
            loongclaw_daemon::migration::types::ImportSourceKind::CodexConfig,
            "Codex config at ~/.codex/config.toml",
            mvp::config::ProviderKind::Openai,
            "openai/gpt-5.1-codex",
            "OPENAI_API_KEY",
        ),
        &[],
    );

    let resolved = loongclaw_daemon::onboard_cli::resolve_provider_config_from_selection(
        &current,
        &plan,
        mvp::config::ProviderKind::Anthropic,
    );

    assert_eq!(resolved.kind, mvp::config::ProviderKind::Anthropic);
    assert_eq!(
        resolved.model, "auto",
        "manual provider overrides should reset the inherited model when switching away from the imported provider"
    );
    assert_eq!(resolved.base_url, "https://api.anthropic.com");
    assert_eq!(resolved.chat_completions_path, "/v1/messages");
    assert_eq!(resolved.api_key_env.as_deref(), Some("ANTHROPIC_API_KEY"));
}

#[test]
fn onboarding_success_summary_reports_import_source_and_enabled_channels() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.model = "openai/gpt-5.1-codex".to_owned();
    config.telegram.enabled = true;
    config.feishu.enabled = true;

    let path = PathBuf::from("/tmp/loongclaw-config.toml");
    let summary = loongclaw_daemon::onboard_cli::build_onboarding_success_summary(
        &path,
        &config,
        Some("Codex config at ~/.codex/config.toml"),
    );

    assert_eq!(
        summary.import_source.as_deref(),
        Some("Codex config at ~/.codex/config.toml")
    );
    assert_eq!(
        summary.channels,
        vec!["cli".to_owned(), "telegram".to_owned(), "feishu".to_owned()]
    );
    assert!(summary.suggested_channels.is_empty());
    assert!(
        summary.next_actions.iter().any(|action| action
            .command
            .contains("loongclaw ask --config '/tmp/loongclaw-config.toml' --message")),
        "success summary should keep a direct ask handoff: {summary:#?}"
    );
}

#[test]
fn onboarding_success_summary_derives_structured_actions() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.telegram.enabled = true;
    config.feishu.enabled = true;

    let path = PathBuf::from("/tmp/loongclaw-config.toml");
    let summary =
        loongclaw_daemon::onboard_cli::build_onboarding_success_summary(&path, &config, None);

    assert_eq!(
        summary.next_actions[0].kind,
        loongclaw_daemon::onboard_cli::OnboardingActionKind::Ask
    );
    assert_eq!(
        summary.next_actions[1].kind,
        loongclaw_daemon::onboard_cli::OnboardingActionKind::Chat
    );
    assert_eq!(
        summary.next_actions[2].kind,
        loongclaw_daemon::onboard_cli::OnboardingActionKind::Channel
    );
    assert_eq!(
        summary.next_actions[3].kind,
        loongclaw_daemon::onboard_cli::OnboardingActionKind::Channel
    );
    assert_eq!(
        summary.next_actions[4].kind,
        crate::onboard_cli::OnboardingActionKind::BrowserPreview
    );
    assert_eq!(summary.next_actions[0].label, "first answer");
    assert_eq!(summary.next_actions[1].label, "chat");
    assert_eq!(summary.next_actions[2].label, "Telegram");
    assert_eq!(summary.next_actions[3].label, "Feishu/Lark");
    assert_eq!(summary.next_actions[4].label, "enable browser preview");
}

#[test]
fn onboarding_success_summary_suggests_registry_backed_channels_when_none_are_enabled() {
    let path = PathBuf::from("/tmp/loongclaw-config.toml");
    let summary = crate::onboard_cli::build_onboarding_success_summary(
        &path,
        &mvp::config::LoongClawConfig::default(),
        None,
    );
    let lines = crate::onboard_cli::render_onboarding_success_summary_with_width(&summary, 140);
    let saved_setup_index = lines
        .iter()
        .position(|line| line == "saved setup")
        .expect("saved setup heading");

    assert_eq!(
        summary.suggested_channels,
        vec![
            "Telegram (personal and group chat bot)".to_owned(),
            "Feishu/Lark (enterprise chat app)".to_owned(),
            "Matrix (federated room sync bot)".to_owned(),
        ]
    );
    assert_eq!(
        summary.next_actions[2].kind,
        crate::onboard_cli::OnboardingActionKind::Channel
    );
    assert_eq!(summary.next_actions[2].label, "channels");
    assert_eq!(
        summary.next_actions[2].command,
        "loongclaw channels --config '/tmp/loongclaw-config.toml'"
    );
    assert!(
        lines
            .iter()
            .skip(saved_setup_index + 1)
            .all(|line| !line.starts_with("- channels: ")),
        "success summary should not render cli-only channel state as a service-channel list: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| {
            line == "- suggested channels: Telegram (personal and group chat bot), Feishu/Lark (enterprise chat app), Matrix (federated room sync bot)"
        }),
        "success summary should render registry-backed suggested runtime channels when no service channels are enabled: {lines:#?}"
    );
}

#[test]
fn onboarding_success_summary_advertises_browser_preview_enable_action() {
    let path = PathBuf::from("/tmp/loongclaw-config.toml");
    let summary = crate::onboard_cli::build_onboarding_success_summary(
        &path,
        &mvp::config::LoongClawConfig::default(),
        None,
    );
    let lines = crate::onboard_cli::render_onboarding_success_summary_with_width(&summary, 80);

    assert!(
        summary.next_actions.iter().any(|action| {
            action.kind == crate::onboard_cli::OnboardingActionKind::BrowserPreview
                && action.label == "enable browser preview"
                && action.command
                    == "loongclaw skills enable-browser-preview --config '/tmp/loongclaw-config.toml'"
        }),
        "onboarding should surface a concrete browser preview enable step for operators: {summary:#?}"
    );
    assert!(
        lines.iter().any(|line| {
            line.contains("enable browser preview")
                && line.contains("loongclaw skills enable-browser-preview --config")
        }) && lines
            .iter()
            .any(|line| line.contains("/tmp/loongclaw-config.toml")),
        "success summary should render the browser preview enable action in the follow-up section: {lines:#?}"
    );
}

#[test]
fn onboard_existing_config_write_screen_offers_replace_backup_and_cancel() {
    let lines = loongclaw_daemon::onboard_cli::render_existing_config_write_screen_lines(
        "/tmp/loongclaw-config.toml",
        80,
    );

    assert_compact_loongclaw_header(&lines, "existing-config write screen");
    assert!(
        lines.iter().all(|line| !line.starts_with("██╗")),
        "existing-config write screen should not repeat the large LOONGCLAW banner after the first screen: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "existing config found"),
        "existing-config write screen should use a focused title: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- config: /tmp/loongclaw-config.toml")),
        "existing-config write screen should show the target path: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("o) Replace existing config")),
        "existing-config write screen should keep the replace path visible: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("b) Create backup and replace")),
        "existing-config write screen should keep the safer backup path visible: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("c) Cancel")),
        "existing-config write screen should keep cancellation explicit: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("b) Create backup and replace (recommended)")),
        "existing-config write screen should steer users toward the safe write path once they have reached the final overwrite decision: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "press Enter to use default b, create backup and replace"),
        "existing-config write screen should make the backup-first default explicit on the screen itself: {lines:#?}"
    );
}

#[test]
fn onboard_preflight_screen_summarizes_status_counts_and_guidance() {
    let checks = vec![
        loongclaw_daemon::onboard_cli::OnboardCheck {
            name: "provider credentials",
            level: loongclaw_daemon::onboard_cli::OnboardCheckLevel::Pass,
            detail: "OPENAI_API_KEY is available".to_owned(),
            non_interactive_warning_policy:
                loongclaw_daemon::onboard_cli::OnboardNonInteractiveWarningPolicy::Block,
        },
        loongclaw_daemon::onboard_cli::OnboardCheck {
            name: "provider model probe",
            level: loongclaw_daemon::onboard_cli::OnboardCheckLevel::Fail,
            detail: "provider rejected the model list".to_owned(),
            non_interactive_warning_policy:
                loongclaw_daemon::onboard_cli::OnboardNonInteractiveWarningPolicy::Block,
        },
        loongclaw_daemon::onboard_cli::OnboardCheck {
            name: "telegram channel",
            level: loongclaw_daemon::onboard_cli::OnboardCheckLevel::Warn,
            detail: "enabled but bot token is missing".to_owned(),
            non_interactive_warning_policy:
                loongclaw_daemon::onboard_cli::OnboardNonInteractiveWarningPolicy::Block,
        },
    ];

    let lines = loongclaw_daemon::onboard_cli::render_preflight_summary_screen_lines(&checks, 80);

    assert_compact_loongclaw_header(&lines, "preflight screen");
    assert!(
        lines.iter().any(|line| line == "preflight checks"),
        "preflight screen should use a focused title: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "step 8 of 8 · review"),
        "preflight screen should stay anchored to the review step: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- status: 1 pass · 1 warn · 1 fail")),
        "preflight screen should summarize pass/warn/fail counts before the raw checks: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("[FAIL] provider model probe")),
        "preflight screen should preserve per-check failure detail: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("some checks need attention before write")),
        "preflight screen should explain the decision context when warnings or failures exist: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("y) Continue anyway")),
        "preflight screen should show the continue path explicitly when attention is still required: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("n) Cancel")),
        "preflight screen should keep the cancel path explicit when checks are not green: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "press Enter to use default n, cancel"),
        "preflight screen should make the safe default explicit when attention is still required: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("--skip-model-probe")),
        "generic failing preflight checks should not suggest --skip-model-probe unless the underlying recovery policy explicitly allows it: {lines:#?}"
    );
}

#[test]
fn onboard_preflight_screen_omits_continue_cancel_choices_when_all_checks_are_green() {
    let checks = vec![loongclaw_daemon::onboard_cli::OnboardCheck {
        name: "provider credentials",
        level: loongclaw_daemon::onboard_cli::OnboardCheckLevel::Pass,
        detail: "OPENAI_API_KEY is available".to_owned(),
        non_interactive_warning_policy:
            loongclaw_daemon::onboard_cli::OnboardNonInteractiveWarningPolicy::Block,
    }];

    let lines = loongclaw_daemon::onboard_cli::render_preflight_summary_screen_lines(&checks, 80);

    assert!(
        lines
            .iter()
            .all(|line| !line.contains("y) Continue anyway")),
        "fully green preflight should not render a continue-anyway choice that will never be asked: {lines:#?}"
    );
    assert!(
        lines.iter().all(|line| !line.contains("n) Cancel")),
        "fully green preflight should not render a cancellation choice that does not apply on this screen: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| line.as_str() != "press Enter to use default n, cancel"),
        "fully green preflight should not show a default-cancel hint when the flow proceeds automatically: {lines:#?}"
    );
}

#[test]
fn onboard_preflight_screen_falls_back_to_stacked_rows_when_details_overflow() {
    let checks = vec![loongclaw_daemon::onboard_cli::OnboardCheck {
        name: "provider model probe",
        level: loongclaw_daemon::onboard_cli::OnboardCheckLevel::Fail,
        detail:
            "provider rejected the model list because the configured endpoint requires a different compatibility mode"
                .to_owned(),
        non_interactive_warning_policy:
            loongclaw_daemon::onboard_cli::OnboardNonInteractiveWarningPolicy::Block,
    }];

    let lines = loongclaw_daemon::onboard_cli::render_preflight_summary_screen_lines(&checks, 80);

    assert!(
        lines
            .iter()
            .any(|line| line == "[FAIL] provider model probe"),
        "preflight screen should fall back to a stacked row when a wide check line would overflow: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line
            == "  provider rejected the model list because the configured endpoint requires a"),
        "stacked preflight fallback should wrap the long detail across readable continuation lines: {lines:#?}"
    );
}

#[test]
fn current_setup_preflight_screen_uses_quick_review_progress_copy() {
    let checks = vec![loongclaw_daemon::onboard_cli::OnboardCheck {
        name: "provider credentials",
        level: loongclaw_daemon::onboard_cli::OnboardCheckLevel::Pass,
        detail: "OPENAI_API_KEY is available".to_owned(),
        non_interactive_warning_policy:
            loongclaw_daemon::onboard_cli::OnboardNonInteractiveWarningPolicy::Block,
    }];

    let lines = loongclaw_daemon::onboard_cli::render_current_setup_preflight_summary_screen_lines(
        &checks, 80,
    );

    assert!(
        lines
            .iter()
            .any(|line| line == "quick review · current setup"),
        "current-setup preflight should use quick-review progress copy: {lines:#?}"
    );
    assert!(
        lines.iter().all(|line| line != "step 8 of 8 · review"),
        "current-setup preflight should not reuse the guided step progress copy: {lines:#?}"
    );
}

#[test]
fn detected_setup_preflight_screen_uses_quick_review_progress_copy() {
    let checks = vec![loongclaw_daemon::onboard_cli::OnboardCheck {
        name: "provider credentials",
        level: loongclaw_daemon::onboard_cli::OnboardCheckLevel::Pass,
        detail: "OPENAI_API_KEY is available".to_owned(),
        non_interactive_warning_policy:
            loongclaw_daemon::onboard_cli::OnboardNonInteractiveWarningPolicy::Block,
    }];

    let lines = loongclaw_daemon::onboard_cli::render_detected_setup_preflight_summary_screen_lines(
        &checks, 80,
    );

    assert!(
        lines
            .iter()
            .any(|line| line == "quick review · detected starting point"),
        "detected-setup preflight should use quick-review progress copy: {lines:#?}"
    );
    assert!(
        lines.iter().all(|line| line != "step 8 of 8 · review"),
        "detected-setup preflight should not reuse the guided step progress copy: {lines:#?}"
    );
}

#[test]
fn onboard_write_confirmation_screen_shows_target_path_and_write_choice() {
    let lines = loongclaw_daemon::onboard_cli::render_write_confirmation_screen_lines(
        "/tmp/loongclaw-config.toml",
        true,
        80,
    );

    assert_compact_loongclaw_header(&lines, "write-confirm screen");
    assert!(
        lines.iter().any(|line| line == "ready to write config"),
        "write-confirm screen should use a focused title: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- config: /tmp/loongclaw-config.toml")),
        "write-confirm screen should show the target config path: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("warnings were kept by choice")),
        "write-confirm screen should remind users when they are writing despite warnings: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("y) Write config")),
        "write-confirm screen should show the affirmative path explicitly: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "press Enter to use default y, write config"),
        "write-confirm screen should make the default write action explicit instead of relying only on the prompt suffix: {lines:#?}"
    );
}

#[test]
fn onboard_write_confirmation_screen_wraps_long_path_and_option_copy() {
    let lines = loongclaw_daemon::onboard_cli::render_write_confirmation_screen_lines(
        "/tmp/shared workspace/loongclaw config.toml",
        true,
        42,
    );

    assert!(
        lines
            .iter()
            .any(|line| line == "- config: /tmp/shared workspace/loongclaw"),
        "write-confirm screen should keep the config label visible before wrapping long paths: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "  config.toml"),
        "write-confirm screen should continue wrapped config paths on an indented line: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "    persist this onboarding draft to the"),
        "write-confirm screen should wrap long action copy instead of overflowing it: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "    target path"),
        "write-confirm screen should keep wrapped action-copy continuations aligned under the option: {lines:#?}"
    );
}

#[test]
fn current_setup_write_confirmation_screen_uses_quick_review_progress_copy() {
    let lines = loongclaw_daemon::onboard_cli::render_current_setup_write_confirmation_screen_lines(
        "/tmp/loongclaw-config.toml",
        true,
        80,
    );

    assert!(
        lines
            .iter()
            .any(|line| line == "quick review · current setup"),
        "current-setup write-confirm should use quick-review progress copy: {lines:#?}"
    );
    assert!(
        lines.iter().all(|line| line != "step 8 of 8 · review"),
        "current-setup write-confirm should not reuse the guided step progress copy: {lines:#?}"
    );
}

#[test]
fn detected_setup_write_confirmation_screen_uses_quick_review_progress_copy() {
    let lines =
        loongclaw_daemon::onboard_cli::render_detected_setup_write_confirmation_screen_lines(
            "/tmp/loongclaw-config.toml",
            true,
            80,
        );

    assert!(
        lines
            .iter()
            .any(|line| line == "quick review · detected starting point"),
        "detected-setup write-confirm should use quick-review progress copy: {lines:#?}"
    );
    assert!(
        lines.iter().all(|line| line != "step 8 of 8 · review"),
        "detected-setup write-confirm should not reuse the guided step progress copy: {lines:#?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn onboard_current_setup_shortcut_flow_skips_detailed_edit_screens() {
    let workspace_root = unique_temp_path("current-shortcut-workspace");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    std::fs::write(workspace_root.join("AGENTS.md"), "# local guidance\n")
        .expect("write workspace guidance");

    let output_path = unique_temp_path("current-shortcut-config.toml");
    let mut existing = mvp::config::LoongClawConfig::default();
    existing.provider.model = "gpt-4.1".to_owned();
    existing.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "inline-secret".to_owned(),
    ));
    existing.telegram.enabled = true;
    existing.telegram.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
        "123456:test-token".to_owned(),
    ));
    mvp::config::write(output_path.to_str(), &existing, true).expect("write existing config");

    let transcript = run_scripted_onboard_flow(
        loongclaw_daemon::onboard_cli::OnboardCommandOptions {
            output: output_path.to_str().map(str::to_owned),
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
            skip_model_probe: true,
        },
        ["1", "1", "y"],
        Some(workspace_root),
        None,
    )
    .await
    .expect("run scripted current-setup onboarding");

    let joined = transcript.join("\n");
    let review_index = joined
        .find("review setup\nquick review · current setup")
        .expect("current-setup flow should include a quick-review section");
    let review_section = &joined[review_index..];
    assert!(
        joined.contains("continue current setup"),
        "current-setup fast lane should render its shortcut screen: {transcript:#?}"
    );
    assert!(
        joined.contains("quick review · current setup"),
        "current-setup fast lane should stay on quick-review copy: {transcript:#?}"
    );
    assert!(
        review_section.contains("keep current value"),
        "current-setup review should preserve how detected domains are being handled: {transcript:#?}"
    );
    assert!(
        joined.contains("existing config kept; no changes were needed"),
        "current-setup fast lane should reuse the current config when nothing changed: {transcript:#?}"
    );
    assert!(
        !joined.contains("choose active provider"),
        "current-setup fast lane should skip the provider screen: {transcript:#?}"
    );
    assert!(
        !joined.contains("choose model"),
        "current-setup fast lane should skip the model screen: {transcript:#?}"
    );
    assert!(
        !joined.contains("choose credential env"),
        "current-setup fast lane should skip the credential env screen: {transcript:#?}"
    );
    assert!(
        !joined.contains("adjust cli behavior"),
        "current-setup fast lane should skip the CLI behavior screen: {transcript:#?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn onboard_detected_setup_shortcut_flow_skips_detailed_edit_screens() {
    let workspace_root = unique_temp_path("detected-shortcut-workspace");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    std::fs::write(workspace_root.join("AGENTS.md"), "# local guidance\n")
        .expect("write workspace guidance");

    let output_path = unique_temp_path("detected-shortcut-config.toml");
    let codex_path = unique_temp_path("detected-shortcut-codex.toml");
    std::fs::write(
        &codex_path,
        r#"
model_provider = "sub2api"
model = "openai/gpt-5.1-codex"

[model_providers.sub2api]
base_url = "https://codex.example.com/v1"
requires_openai_auth = true
"#,
    )
    .expect("write codex config");

    let screen_candidates = loongclaw_daemon::onboard_cli::collect_import_candidates_with_paths(
        &output_path,
        Some(&codex_path),
        loongclaw_daemon::onboard_cli::ChannelImportReadiness::default().with_state(
            "telegram",
            loongclaw_daemon::migration::ChannelCredentialState::Ready,
        ),
    )
    .expect("collect import candidates for screen ordering");
    let screen_lines = loongclaw_daemon::onboard_cli::render_starting_point_selection_screen_lines(
        &screen_candidates,
        80,
    );
    assert!(
        screen_lines
            .iter()
            .any(|line| line.contains("2) Codex config at")),
        "the rendered starting-point screen should keep the Codex candidate at index 2: {screen_lines:#?}"
    );

    let transcript = run_scripted_onboard_flow(
        loongclaw_daemon::onboard_cli::OnboardCommandOptions {
            output: output_path.to_str().map(str::to_owned),
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
            skip_model_probe: true,
        },
        ["1", "1", "1", "y", "y"],
        Some(workspace_root),
        Some(codex_path),
    )
    .await
    .expect("run scripted detected-setup onboarding");

    let joined = transcript.join("\n");
    let review_index = joined
        .find("review setup\nquick review · detected starting point")
        .expect("detected-setup flow should include a quick-review section");
    let review_section = &joined[review_index..];
    assert!(
        joined.contains("choose detected starting point"),
        "detected-setup flow should still show the starting-point chooser before the shortcut: {transcript:#?}"
    );
    assert!(
        joined.contains("continue with detected starting point"),
        "detected-setup fast lane should render its shortcut screen: {transcript:#?}"
    );
    assert!(
        joined.contains("quick review · detected starting point"),
        "detected-setup fast lane should stay on quick-review copy: {transcript:#?}"
    );
    assert!(
        joined.contains("starting point: suggested starting point"),
        "detected-setup fast lane should keep the selected starting point visible through review: {transcript:#?}"
    );
    assert!(
        review_section.contains("use detected value"),
        "detected-setup review should preserve how detected domains are being applied: {transcript:#?}"
    );
    assert!(
        !joined.contains("choose active provider"),
        "detected-setup fast lane should skip the provider screen when the provider is already resolved: {transcript:#?}"
    );
    assert!(
        !joined.contains("choose model"),
        "detected-setup fast lane should skip the model screen: {transcript:#?}"
    );
    assert!(
        !joined.contains("choose credential env"),
        "detected-setup fast lane should skip the credential env screen: {transcript:#?}"
    );
    assert!(
        !joined.contains("adjust cli behavior"),
        "detected-setup fast lane should skip the CLI behavior screen: {transcript:#?}"
    );
    assert!(
        output_path.exists(),
        "detected-setup fast lane should still write the config after quick review: {}",
        output_path.display()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn onboard_detected_setup_selection_uses_the_same_order_the_screen_shows() {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    unsafe {
        std::env::set_var("DEEPSEEK_API_KEY", "deepseek-test-token");
        std::env::set_var("TELEGRAM_BOT_TOKEN", "123456:test-token");
    }

    let workspace_root = unique_temp_path("detected-selection-order-workspace");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    std::fs::write(workspace_root.join("AGENTS.md"), "# local guidance\n")
        .expect("write workspace guidance");

    let output_path = unique_temp_path("detected-selection-order-config.toml");
    let codex_path = unique_temp_path("detected-selection-order-codex.toml");
    std::fs::write(
        &codex_path,
        r#"
model_provider = "sub2api"
model = "openai/gpt-5.1-codex"

[model_providers.sub2api]
base_url = "https://codex.example.com/v1"
requires_openai_auth = true
"#,
    )
    .expect("write codex config");

    let transcript = run_scripted_onboard_flow(
        loongclaw_daemon::onboard_cli::OnboardCommandOptions {
            output: output_path.to_str().map(str::to_owned),
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
            skip_model_probe: true,
        },
        ["1", "2", "1", "y", "y"],
        Some(workspace_root),
        Some(codex_path),
    )
    .await
    .expect("run scripted detected-setup onboarding with explicit starting-point selection");

    let joined = transcript.join("\n");
    assert!(
        joined.contains("choose detected starting point")
            && joined.contains("SELECT Starting point"),
        "the interactive flow should still show the starting-point selection stage before applying the chosen path: {transcript:#?}"
    );
    assert!(
        joined.contains("starting point: Codex config at")
            && joined.contains("selection-order-codex.toml"),
        "after choosing [2], the rest of onboarding should carry the displayed Codex option forward, not some internal candidate order: {transcript:#?}"
    );
    assert!(
        !joined.contains("starting point: your current environment"),
        "selection should stay aligned with the on-screen numbering when candidates are reordered for UX: {transcript:#?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn onboard_single_detected_setup_flow_uses_preview_screen_instead_of_plain_label() {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    let output_path = unique_temp_path("single-detected-config.toml");
    let codex_path = unique_temp_path("single-detected-codex.toml");
    std::fs::write(
        &codex_path,
        r#"
model_provider = "sub2api"
model = "openai/gpt-5.1-codex"

[model_providers.sub2api]
base_url = "https://codex.example.com/v1"
requires_openai_auth = true
"#,
    )
    .expect("write codex config");

    let transcript = run_scripted_onboard_flow(
        loongclaw_daemon::onboard_cli::OnboardCommandOptions {
            output: output_path.to_str().map(str::to_owned),
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
            skip_model_probe: true,
        },
        ["1", "1", "y", "y"],
        None,
        Some(codex_path),
    )
    .await
    .expect("run scripted onboarding with a single detected setup");

    let joined = transcript.join("\n");
    assert!(
        joined.contains("review detected starting point"),
        "single detected-setup flow should render a branded preview screen before continuing: {transcript:#?}"
    );
    assert!(
        joined.contains("continuing with the only detected starting point"),
        "single detected-setup flow should explain why it skips the starting-point chooser: {transcript:#?}"
    );
    assert!(
        !joined.contains("\nDetected setup:\n"),
        "single detected-setup flow should no longer fall back to the old plain inline preview label: {transcript:#?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn onboard_current_setup_adjustments_preserve_unchanged_domain_actions_in_review() {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    let workspace_root = unique_temp_path("current-adjusted-review-workspace");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    std::fs::write(workspace_root.join("AGENTS.md"), "# local guidance\n")
        .expect("write workspace guidance");

    let output_path = unique_temp_path("current-adjusted-review-config.toml");
    let mut existing = mvp::config::LoongClawConfig::default();
    existing.provider.model = "gpt-4.1".to_owned();
    existing.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "inline-secret".to_owned(),
    ));
    existing.telegram.enabled = true;
    existing.telegram.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
        "123456:test-token".to_owned(),
    ));
    mvp::config::write(output_path.to_str(), &existing, true).expect("write existing config");

    let transcript = run_scripted_onboard_flow(
        loongclaw_daemon::onboard_cli::OnboardCommandOptions {
            output: output_path.to_str().map(str::to_owned),
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
            skip_model_probe: true,
        },
        vec![
            "1".to_owned(),
            "2".to_owned(),
            provider_choice_input(mvp::config::ProviderKind::Openai),
            "gpt-4.1".to_owned(),
            "OPENAI_API_KEY".to_owned(),
            String::new(),
            "custom review prompt".to_owned(),
            String::new(),
            String::new(),
            "y".to_owned(),
            "y".to_owned(),
            "o".to_owned(),
        ],
        Some(workspace_root),
        None,
    )
    .await
    .expect("run scripted current-setup onboarding with adjustments");

    let review_lines = extract_review_section_lines(&transcript, "step 8 of 8 · review");
    let has_domain_action = |domain_label: &str, action_label: &str| {
        review_lines.iter().enumerate().any(|(index, line)| {
            line.contains(&format!("- {domain_label} ["))
                && review_lines[index + 1..review_lines.len().min(index + 4)]
                    .iter()
                    .any(|candidate| candidate.contains(action_label))
        })
    };

    assert!(
        review_lines
            .iter()
            .any(|line| line == "source: current onboarding draft"),
        "after edits, review should present the whole draft as a current onboarding draft: {review_lines:#?}"
    );
    assert!(
        has_domain_action("provider", "keep current value"),
        "unchanged provider settings should keep their current-setup action label in review: {review_lines:#?}"
    );
    assert!(
        has_domain_action("channels", "keep current value"),
        "unchanged channels should keep their current-setup action label in review: {review_lines:#?}"
    );
    assert!(
        has_domain_action("workspace guidance", "keep current value"),
        "unchanged workspace guidance should keep its current-setup action label in review: {review_lines:#?}"
    );
    assert!(
        has_domain_action("cli", "adjusted in this setup"),
        "the edited cli domain should be called out as manually adjusted in this setup: {review_lines:#?}"
    );

    let success_lines = extract_success_section_lines(&transcript);
    assert!(
        success_lines.iter().any(|line| line == "setup outcome"),
        "success summary should include a compact setup outcome section when decision context exists: {success_lines:#?}"
    );
    assert!(
        success_lines
            .iter()
            .any(|line| line == "- kept current: provider, channels, workspace guidance"),
        "success summary should group unchanged current-setup domains into a readable outcome line: {success_lines:#?}"
    );
    assert!(
        success_lines
            .iter()
            .any(|line| line == "- adjusted now: cli"),
        "success summary should group domains adjusted during onboarding: {success_lines:#?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn onboard_current_setup_adjustments_capture_personality_and_memory_profile() {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    let workspace_root = unique_temp_path("current-adjusted-personality-memory-workspace");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    std::fs::write(workspace_root.join("AGENTS.md"), "# local guidance\n")
        .expect("write workspace guidance");

    let output_path = unique_temp_path("current-adjusted-personality-memory-config.toml");
    let mut existing = mvp::config::LoongClawConfig::default();
    existing.provider.model = "gpt-4.1".to_owned();
    existing.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
        "inline-secret".to_owned(),
    ));
    mvp::config::write(output_path.to_str(), &existing, true).expect("write existing config");

    let transcript = run_scripted_onboard_flow(
        crate::onboard_cli::OnboardCommandOptions {
            output: output_path.to_str().map(str::to_owned),
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
            skip_model_probe: true,
        },
        vec![
            "1".to_owned(),
            "2".to_owned(),
            provider_choice_input(mvp::config::ProviderKind::Openai),
            "gpt-4.1".to_owned(),
            "OPENAI_API_KEY".to_owned(),
            "2".to_owned(),
            String::new(),
            "3".to_owned(),
            String::new(),
            "y".to_owned(),
            "y".to_owned(),
            "o".to_owned(),
        ],
        Some(workspace_root),
        None,
    )
    .await
    .expect("run scripted current-setup onboarding with personality and memory profile changes");

    let joined = transcript.join("\n");
    assert!(
        joined.contains("step 4 of 8 · personality"),
        "guided current-setup adjustments should expose a dedicated personality step: {transcript:#?}"
    );
    assert!(
        joined.contains("step 5 of 8 · prompt addendum"),
        "guided current-setup adjustments should expose a dedicated prompt-addendum step: {transcript:#?}"
    );
    assert!(
        joined.contains("step 6 of 8 · memory profile"),
        "guided current-setup adjustments should expose a dedicated memory-profile step: {transcript:#?}"
    );

    let (_, config) = mvp::config::load(output_path.to_str())
        .expect("load current-setup personality/memory config");
    assert_eq!(
        config.cli.personality,
        Some(mvp::prompt::PromptPersonality::FriendlyCollab)
    );
    assert_eq!(
        config.memory.profile,
        mvp::config::MemoryProfile::ProfilePlusWindow
    );
}

#[test]
fn onboard_interactive_flow_defaults_back_to_native_prompt_pack_even_from_inline_override() {
    let mut existing = mvp::config::LoongClawConfig::default();
    existing.cli.prompt_pack_id = Some(String::new());
    existing.cli.personality = None;
    existing.cli.system_prompt_addendum = None;
    existing.cli.system_prompt = "Stay terse and imperative.".to_owned();

    let path = crate::onboard_cli::resolve_guided_prompt_path_label_for_test(
        &crate::onboard_cli::OnboardCommandOptions {
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
            skip_model_probe: true,
        },
        &existing,
    );

    assert_eq!(
        path, "native",
        "interactive onboarding should default back to the native prompt-pack path even when the current config uses an inline override"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn onboard_detected_setup_adjustments_preserve_unchanged_detected_actions_in_review() {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    let workspace_root = unique_temp_path("detected-adjusted-review-workspace");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    std::fs::write(workspace_root.join("AGENTS.md"), "# local guidance\n")
        .expect("write workspace guidance");

    let output_path = unique_temp_path("detected-adjusted-review-config.toml");
    let codex_path = unique_temp_path("detected-adjusted-review-codex.toml");
    std::fs::write(
        &codex_path,
        r#"
model_provider = "sub2api"
model = "openai/gpt-5.1-codex"

[model_providers.sub2api]
base_url = "https://codex.example.com/v1"
requires_openai_auth = true
"#,
    )
    .expect("write codex config");

    let transcript = run_scripted_onboard_flow(
        loongclaw_daemon::onboard_cli::OnboardCommandOptions {
            output: output_path.to_str().map(str::to_owned),
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
            skip_model_probe: true,
        },
        [
            "1",
            "1",
            "2",
            "1",
            "openai/gpt-5.1-codex-preview",
            "OPENAI_API_KEY",
            "",
            "",
            "",
            "",
            "y",
            "y",
        ],
        Some(workspace_root),
        Some(codex_path),
    )
    .await
    .expect("run scripted detected-setup onboarding with adjustments");

    let review_lines = extract_review_section_lines(&transcript, "step 8 of 8 · review");
    let has_domain_action = |domain_label: &str, action_label: &str| {
        review_lines.iter().enumerate().any(|(index, line)| {
            line.contains(&format!("- {domain_label} ["))
                && review_lines[index + 1..review_lines.len().min(index + 4)]
                    .iter()
                    .any(|candidate| candidate.contains(action_label))
        })
    };

    assert!(
        review_lines
            .iter()
            .any(|line| line == "source: current onboarding draft"),
        "after edits, guided review should present the whole draft as a current onboarding draft: {review_lines:#?}"
    );
    assert!(
        has_domain_action("workspace guidance", "use detected value"),
        "unchanged workspace guidance should keep its detected action label in review: {review_lines:#?}"
    );
    assert!(
        has_domain_action("provider", "adjusted in this setup"),
        "the edited provider domain should be called out as manually adjusted in this setup: {review_lines:#?}"
    );

    let success_lines = extract_success_section_lines(&transcript);
    assert!(
        success_lines.iter().any(|line| line == "setup outcome"),
        "success summary should include a compact setup outcome section when detected decisions exist: {success_lines:#?}"
    );
    assert!(
        success_lines
            .iter()
            .any(|line| line == "- adjusted now: provider"),
        "success summary should group manually adjusted domains in the final handoff: {success_lines:#?}"
    );
    assert!(
        success_lines
            .iter()
            .any(|line| line == "- used detected: workspace guidance"),
        "success summary should group unchanged detected domains into a readable outcome line: {success_lines:#?}"
    );
}

#[test]
fn onboard_review_lines_include_starting_point_and_domain_preview() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.api_key_env = Some("OPENAI_API_KEY".to_owned());
    config.provider.model = "openai/gpt-5.1-codex".to_owned();
    config.telegram.enabled = true;
    config.telegram.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
        "123456:test-token".to_owned(),
    ));

    let lines = loongclaw_daemon::onboard_cli::render_onboard_review_lines_with_guidance(
        &config,
        Some("Codex config at ~/.codex/config.toml"),
        &[
            loongclaw_daemon::migration::types::WorkspaceGuidanceCandidate {
                kind: loongclaw_daemon::migration::types::WorkspaceGuidanceKind::Agents,
                path: "/tmp/project/AGENTS.md".to_owned(),
            },
        ],
        80,
    );

    assert!(
        lines
            .iter()
            .any(|line| line.contains("starting point: Codex config at ~/.codex/config.toml")),
        "review should preserve the detected starting point: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("provider")),
        "review should include typed provider preview lines: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("workspace guidance")),
        "review should include workspace guidance as its own domain: {lines:#?}"
    );
}

#[test]
fn onboard_review_lines_group_details_into_named_sections() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.api_key_env = Some("OPENAI_API_KEY".to_owned());
    config.provider.model = "openai/gpt-5.1-codex".to_owned();

    let lines = loongclaw_daemon::onboard_cli::render_onboard_review_lines_with_guidance(
        &config,
        Some("Codex config at ~/.codex/config.toml"),
        &[],
        80,
    );

    let starting_point_index = lines
        .iter()
        .position(|line| line == "starting point")
        .expect("review should include a starting-point section");
    let configuration_index = lines
        .iter()
        .position(|line| line == "configuration")
        .expect("review should include a configuration section");
    let draft_source_index = lines
        .iter()
        .position(|line| line == "draft source")
        .expect("review should include a draft-source section");

    assert!(
        starting_point_index < configuration_index && configuration_index < draft_source_index,
        "review should separate starting point, config digest, and draft-source details into clear sections: {lines:#?}"
    );
}

#[test]
fn onboard_review_lines_use_compact_header() {
    let lines = loongclaw_daemon::onboard_cli::render_onboard_review_lines_with_guidance(
        &mvp::config::LoongClawConfig::default(),
        None,
        &[],
        80,
    );

    assert_compact_loongclaw_header(&lines, "review screen");
    assert!(
        lines.iter().all(|line| !line.starts_with("██╗")),
        "review screen should not repeat the large LOONGCLAW banner: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "review setup"),
        "review screen should retain a clear review heading under the brand block: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "step 8 of 8 · review"),
        "review screen should include guided progress context inside the screen: {lines:#?}"
    );
}

#[test]
fn onboard_review_lines_include_core_setup_summary_for_fresh_setup() {
    let lines = loongclaw_daemon::onboard_cli::render_onboard_review_lines_with_guidance(
        &mvp::config::LoongClawConfig::default(),
        None,
        &[],
        80,
    );

    assert!(
        lines.iter().any(|line| line.contains("- provider: OpenAI")),
        "review should summarize the active provider with the guided display name even when the draft is still close to defaults: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("- model: auto")),
        "review should summarize the active model for first-run onboarding: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- credential source: OPENAI_CODEX_OAUTH_TOKEN")),
        "review should keep the provider-preferred credential env visible for fresh setup flows: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- prompt mode: native prompt pack")),
        "review should surface the active prompt mode for fresh setup flows: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- personality: calm_engineering")),
        "review should surface the active native personality during onboarding: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- memory profile: window_only")),
        "review should surface the selected memory profile during onboarding: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- web search: DuckDuckGo")),
        "review should surface the selected web-search provider during onboarding: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- web search credential: not required")),
        "review should explain when the chosen web-search provider does not require credentials: {lines:#?}"
    );
}

#[test]
fn onboard_review_lines_prefer_oauth_env_over_api_key_env_when_both_are_configured() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.oauth_access_token_env = Some("OPENAI_CODEX_OAUTH_TOKEN".to_owned());

    let lines = loongclaw_daemon::onboard_cli::render_onboard_review_lines_with_guidance(
        &config,
        None,
        &[],
        80,
    );

    assert!(
        lines
            .iter()
            .any(|line| line.contains("- credential source: OPENAI_CODEX_OAUTH_TOKEN")),
        "review should reflect the higher-priority oauth credential path when both bindings exist: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("- credential source: OPENAI_API_KEY")),
        "review should not keep advertising the api key env as primary once oauth is configured: {lines:#?}"
    );
}

#[test]
fn onboard_review_lines_include_active_provider_and_saved_profiles_when_multiple_profiles_exist() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Openai;
    config.provider.model = "gpt-5".to_owned();
    config.active_provider = Some("openai".to_owned());
    config.providers.insert(
        "openai".to_owned(),
        mvp::config::ProviderProfileConfig::from_provider(config.provider.clone()),
    );
    config.providers.insert(
        "deepseek".to_owned(),
        mvp::config::ProviderProfileConfig::from_provider(mvp::config::ProviderConfig {
            kind: mvp::config::ProviderKind::Deepseek,
            model: "deepseek-chat".to_owned(),
            api_key_env: Some("DEEPSEEK_API_KEY".to_owned()),
            ..mvp::config::ProviderConfig::default()
        }),
    );

    let lines = loongclaw_daemon::onboard_cli::render_onboard_review_lines_with_guidance(
        &config,
        None,
        &[],
        80,
    );

    assert!(
        lines
            .iter()
            .any(|line| line.contains("- active provider: OpenAI")),
        "review should make the active provider explicit once multiple provider profiles exist: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- saved provider profiles: openai, deepseek")),
        "review should summarize retained provider profiles instead of implying only one survives: {lines:#?}"
    );
}

#[test]
fn current_setup_review_lines_use_quick_review_progress_copy() {
    let lines = loongclaw_daemon::onboard_cli::render_current_setup_review_lines_with_guidance(
        &mvp::config::LoongClawConfig::default(),
        None,
        &[],
        80,
    );

    assert!(
        lines
            .iter()
            .any(|line| line == "quick review · current setup"),
        "current-setup review should use quick-review progress copy: {lines:#?}"
    );
    assert!(
        lines.iter().all(|line| line != "step 8 of 8 · review"),
        "current-setup review should not reuse the guided step progress copy: {lines:#?}"
    );
}

#[test]
fn detected_setup_review_lines_use_quick_review_progress_copy() {
    let lines = loongclaw_daemon::onboard_cli::render_detected_setup_review_lines_with_guidance(
        &mvp::config::LoongClawConfig::default(),
        Some("Codex config at ~/.codex/config.toml"),
        &[],
        80,
    );

    assert!(
        lines
            .iter()
            .any(|line| line == "quick review · detected starting point"),
        "detected-setup review should use quick-review progress copy: {lines:#?}"
    );
    assert!(
        lines.iter().all(|line| line != "step 8 of 8 · review"),
        "detected-setup review should not reuse the guided step progress copy: {lines:#?}"
    );
}

#[test]
fn onboard_review_lines_sanitize_suggested_starting_point_label() {
    let lines = loongclaw_daemon::onboard_cli::render_onboard_review_lines_with_guidance(
        &mvp::config::LoongClawConfig::default(),
        Some("recommended import plan"),
        &[],
        80,
    );

    assert!(
        lines
            .iter()
            .any(|line| line.contains("starting point: suggested starting point")),
        "guided onboarding review should translate the internal recommended-plan label into user-facing suggested-starting-point wording: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("recommended import plan")),
        "guided onboarding review should not leak the internal recommended import label: {lines:#?}"
    );
}

#[test]
fn onboard_review_lines_compact_on_narrow_width() {
    let _env_guard = DetectedEnvironmentGuard::without_detected_environment();
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.api_key_env = Some("OPENAI_API_KEY".to_owned());
    config.telegram.enabled = true;
    config.telegram.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
        "123456:test-token".to_owned(),
    ));

    let lines = loongclaw_daemon::onboard_cli::render_onboard_review_lines_with_guidance(
        &config,
        None,
        &[],
        54,
    );

    assert!(
        lines.iter().any(|line| line.starts_with("- provider [")),
        "narrow review should use compact domain rows: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("source: current onboarding draft")),
        "narrow review should keep source attribution on a separate line: {lines:#?}"
    );
}

#[test]
fn onboard_review_lines_wrap_long_starting_point_on_narrow_width() {
    let lines = loongclaw_daemon::onboard_cli::render_detected_setup_review_lines_with_guidance(
        &mvp::config::LoongClawConfig::default(),
        Some("Codex config at ~/.codex/agents/loongclaw/config.toml"),
        &[],
        48,
    );

    assert!(
        lines
            .iter()
            .any(|line| line == "- starting point: Codex config at"),
        "narrow review should keep the starting-point label readable before wrapping the path: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "  ~/.codex/agents/loongclaw/config.toml"),
        "narrow review should continue long starting-point paths on an indented line: {lines:#?}"
    );
}

#[test]
fn onboard_should_skip_config_write_when_existing_config_matches_draft() {
    let mut existing = mvp::config::LoongClawConfig::default();
    existing.provider.model = "openai/gpt-5.1".to_owned();
    existing.cli.system_prompt = "keep current setup".to_owned();

    assert!(
        loongclaw_daemon::onboard_cli::should_skip_config_write(Some(&existing), &existing),
        "matching draft and existing config should reuse the current file instead of forcing another write decision"
    );

    let mut changed = existing.clone();
    changed.provider.model = "openai/gpt-5.2".to_owned();
    assert!(
        !loongclaw_daemon::onboard_cli::should_skip_config_write(Some(&existing), &changed),
        "a changed draft should still go through the normal write flow"
    );
}

#[test]
fn render_onboarding_success_summary_compacts_for_narrow_width() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.telegram.enabled = true;
    config.feishu.enabled = true;

    let path = PathBuf::from("/tmp/loongclaw-config.toml");
    let summary =
        loongclaw_daemon::onboard_cli::build_onboarding_success_summary(&path, &config, None);
    let lines =
        loongclaw_daemon::onboard_cli::render_onboarding_success_summary_with_width(&summary, 48);
    assert!(
        lines.iter().any(|line| line == "start here"),
        "narrow renderer should explicitly call out the primary next action: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "- first answer: loongclaw ask --config")
            && lines
                .iter()
                .any(|line| line == "  '/tmp/loongclaw-config.toml' --message")
            && lines
                .iter()
                .any(|line| line == "  'Summarize this repository and suggest the")
            && lines.iter().any(|line| line == "  best next step.'"),
        "narrow renderer should keep the primary first-answer handoff readable even when the command wraps: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "also available"),
        "narrow renderer should group secondary channel actions under a separate heading: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "- chat: loongclaw chat --config")
            && lines
                .iter()
                .any(|line| line == "- Telegram: loongclaw telegram-serve --config"),
        "narrow renderer should keep secondary chat and channel actions visible after the primary ask example: {lines:#?}"
    );
}

#[test]
fn onboarding_success_summary_surfaces_primary_handoff_before_saved_setup_details() {
    let path = PathBuf::from("/tmp/loongclaw-config.toml");
    let summary = loongclaw_daemon::onboard_cli::build_onboarding_success_summary(
        &path,
        &mvp::config::LoongClawConfig::default(),
        None,
    );

    let lines =
        loongclaw_daemon::onboard_cli::render_onboarding_success_summary_with_width(&summary, 80);
    let start_here_index = lines
        .iter()
        .position(|line| line.starts_with("start here:"))
        .expect("start here line should exist");
    let saved_setup_index = lines
        .iter()
        .position(|line| line == "saved setup")
        .expect("saved setup heading should exist");

    assert!(
        start_here_index < saved_setup_index,
        "onboarding should show the first runnable handoff before the saved setup inventory: {lines:#?}"
    );
}

#[test]
fn onboarding_success_summary_uses_starting_point_language() {
    let path = PathBuf::from("/tmp/loongclaw-config.toml");
    let summary = loongclaw_daemon::onboard_cli::build_onboarding_success_summary(
        &path,
        &mvp::config::LoongClawConfig::default(),
        Some("Codex config at ~/.codex/config.toml"),
    );

    let lines =
        loongclaw_daemon::onboard_cli::render_onboarding_success_summary_with_width(&summary, 80);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("starting point: Codex config at ~/.codex/config.toml")),
        "onboarding summary should use starting-point wording instead of import terminology: {lines:#?}"
    );
    assert!(
        lines.iter().all(|line| !line.contains("imported from")),
        "onboarding summary should avoid import language in the main guided flow: {lines:#?}"
    );
}

#[test]
fn onboarding_success_summary_uses_compact_header() {
    let path = PathBuf::from("/tmp/loongclaw-config.toml");
    let summary = loongclaw_daemon::onboard_cli::build_onboarding_success_summary(
        &path,
        &mvp::config::LoongClawConfig::default(),
        None,
    );

    let lines =
        loongclaw_daemon::onboard_cli::render_onboarding_success_summary_with_width(&summary, 80);
    assert_compact_loongclaw_header(&lines, "success summary");
    assert!(
        lines.iter().all(|line| !line.starts_with("██╗")),
        "success summary should not repeat the large LOONGCLAW banner after onboarding has already started: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "onboarding complete"),
        "success summary should retain a clear completion heading: {lines:#?}"
    );
    assert!(
        lines
            .join(" ")
            .contains("start here: loongclaw ask --config '/tmp/loongclaw-config.toml' --message")
            && lines
                .join(" ")
                .contains("Summarize this repository and suggest the best next step."),
        "success summary should elevate ask as the primary handoff command even when wrapping is needed: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- prompt mode: native prompt pack")),
        "success summary should include the prompt mode that onboarding persisted: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- personality: calm_engineering")),
        "success summary should include the selected native personality: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- memory profile: window_only")),
        "success summary should include the selected memory profile: {lines:#?}"
    );
}

#[test]
fn onboarding_success_summary_shell_quotes_config_paths_with_single_quotes() {
    let path = PathBuf::from("/tmp/loongclaw's config.toml");
    let summary = loongclaw_daemon::onboard_cli::build_onboarding_success_summary(
        &path,
        &mvp::config::LoongClawConfig::default(),
        None,
    );
    let lines =
        loongclaw_daemon::onboard_cli::render_onboarding_success_summary_with_width(&summary, 160);
    let rendered = lines.join(" ");

    assert!(
        rendered.contains(
            "start here: loongclaw ask --config '/tmp/loongclaw'\"'\"'s config.toml' --message"
        ),
        "success summary should shell-quote single quotes in the primary ask handoff: {lines:#?}"
    );
    assert!(
        rendered.contains("- chat: loongclaw chat --config '/tmp/loongclaw'\"'\"'s config.toml'"),
        "success summary should shell-quote single quotes in the secondary chat handoff: {lines:#?}"
    );
}

#[test]
fn onboarding_success_summary_prefers_oauth_env_over_api_key_env_when_both_are_configured() {
    let path = PathBuf::from("/tmp/loongclaw-config.toml");
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.oauth_access_token_env = Some("OPENAI_CODEX_OAUTH_TOKEN".to_owned());

    let summary =
        loongclaw_daemon::onboard_cli::build_onboarding_success_summary(&path, &config, None);
    let lines =
        loongclaw_daemon::onboard_cli::render_onboarding_success_summary_with_width(&summary, 80);

    assert!(
        lines
            .iter()
            .any(|line| line == "- credential source: OPENAI_CODEX_OAUTH_TOKEN"),
        "success summary should surface the primary oauth binding when oauth and api key envs both exist: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| line != "- credential source: OPENAI_API_KEY"),
        "success summary should not keep the api key env as the primary credential line once oauth is configured: {lines:#?}"
    );
}

#[test]
fn onboarding_success_summary_reports_existing_config_kept() {
    let summary = loongclaw_daemon::onboard_cli::OnboardingSuccessSummary {
        import_source: None,
        config_path: "/tmp/loongclaw-config.toml".to_owned(),
        config_status: Some("existing config kept; no changes were needed".to_owned()),
        provider: "openai".to_owned(),
        saved_provider_profiles: Vec::new(),
        model: "auto".to_owned(),
        transport: "chat_completions compatibility mode".to_owned(),
        provider_endpoint: None,
        credential: Some(loongclaw_daemon::onboard_cli::OnboardingCredentialSummary {
            label: "credential source",
            value: "OPENAI_API_KEY".to_owned(),
        }),
        prompt_mode: "native prompt pack".to_owned(),
        personality: Some("calm_engineering".to_owned()),
        prompt_addendum: None,
        memory_profile: "window_only".to_owned(),
        web_search_provider: "DuckDuckGo".to_owned(),
        web_search_credential: Some(loongclaw_daemon::onboard_cli::OnboardingCredentialSummary {
            label: "web search credential",
            value: "not required".to_owned(),
        }),
        memory_path: None,
        channels: vec!["cli".to_owned()],
        suggested_channels: Vec::new(),
        domain_outcomes: Vec::new(),
        next_actions: vec![loongclaw_daemon::onboard_cli::OnboardingAction {
            kind: loongclaw_daemon::onboard_cli::OnboardingActionKind::Ask,
            label: "ask".to_owned(),
            command: "loongclaw ask --config /tmp/loongclaw-config.toml --message \"Summarize this repository and suggest the best next step.\"".to_owned(),
        }],
    };

    let lines =
        loongclaw_daemon::onboard_cli::render_onboarding_success_summary_with_width(&summary, 80);

    assert!(
        lines
            .iter()
            .any(|line| line == "- config status: existing config kept; no changes were needed"),
        "success summary should tell the user when onboarding reused the current config without rewriting it: {lines:#?}"
    );
}

#[test]
fn onboarding_success_summary_reports_active_provider_and_saved_profiles() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Openai;
    config.provider.model = "gpt-5".to_owned();
    config.active_provider = Some("openai".to_owned());
    config.providers.insert(
        "openai".to_owned(),
        mvp::config::ProviderProfileConfig::from_provider(config.provider.clone()),
    );
    config.providers.insert(
        "deepseek".to_owned(),
        mvp::config::ProviderProfileConfig::from_provider(mvp::config::ProviderConfig {
            kind: mvp::config::ProviderKind::Deepseek,
            model: "deepseek-chat".to_owned(),
            api_key_env: Some("DEEPSEEK_API_KEY".to_owned()),
            ..mvp::config::ProviderConfig::default()
        }),
    );

    let path = PathBuf::from("/tmp/loongclaw-config.toml");
    let summary =
        loongclaw_daemon::onboard_cli::build_onboarding_success_summary(&path, &config, None);
    let lines =
        loongclaw_daemon::onboard_cli::render_onboarding_success_summary_with_width(&summary, 80);

    assert!(
        lines.iter().any(|line| line == "- active provider: OpenAI"),
        "success summary should tell the user which provider remains active after saving multiple profiles: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "- saved provider profiles: openai, deepseek"),
        "success summary should summarize retained provider profiles once onboarding saves more than one: {lines:#?}"
    );
}

#[test]
fn onboarding_success_summary_reports_web_search_provider_and_credential() {
    let path = PathBuf::from("/tmp/loongclaw-config.toml");
    let summary = loongclaw_daemon::onboard_cli::build_onboarding_success_summary(
        &path,
        &mvp::config::LoongClawConfig::default(),
        None,
    );
    let lines =
        loongclaw_daemon::onboard_cli::render_onboarding_success_summary_with_width(&summary, 80);

    assert!(
        lines.iter().any(|line| line == "- web search: DuckDuckGo"),
        "success summary should surface the selected web-search provider: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "- web search credential: not required"),
        "success summary should explain when the selected web-search provider does not need a credential: {lines:#?}"
    );
}

#[test]
fn onboarding_success_summary_groups_domain_outcomes_by_decision() {
    let summary = loongclaw_daemon::onboard_cli::OnboardingSuccessSummary {
        import_source: Some("suggested starting point".to_owned()),
        config_path: "/tmp/loongclaw-config.toml".to_owned(),
        config_status: None,
        provider: "openai".to_owned(),
        saved_provider_profiles: Vec::new(),
        model: "openai/gpt-5.1-codex".to_owned(),
        transport: "chat_completions compatibility mode".to_owned(),
        provider_endpoint: None,
        credential: Some(loongclaw_daemon::onboard_cli::OnboardingCredentialSummary {
            label: "credential source",
            value: "OPENAI_API_KEY".to_owned(),
        }),
        prompt_mode: "native prompt pack".to_owned(),
        personality: Some("friendly_collab".to_owned()),
        prompt_addendum: Some("Keep answers direct.".to_owned()),
        memory_profile: "profile_plus_window".to_owned(),
        web_search_provider: "DuckDuckGo".to_owned(),
        web_search_credential: Some(loongclaw_daemon::onboard_cli::OnboardingCredentialSummary {
            label: "web search credential",
            value: "not required".to_owned(),
        }),
        memory_path: None,
        channels: vec!["cli".to_owned()],
        suggested_channels: Vec::new(),
        domain_outcomes: vec![
            loongclaw_daemon::onboard_cli::OnboardingDomainOutcome {
                kind: loongclaw_daemon::migration::types::SetupDomainKind::Provider,
                decision: loongclaw_daemon::migration::types::PreviewDecision::AdjustedInSession,
            },
            loongclaw_daemon::onboard_cli::OnboardingDomainOutcome {
                kind: loongclaw_daemon::migration::types::SetupDomainKind::Channels,
                decision: loongclaw_daemon::migration::types::PreviewDecision::Supplement,
            },
            loongclaw_daemon::onboard_cli::OnboardingDomainOutcome {
                kind: loongclaw_daemon::migration::types::SetupDomainKind::WorkspaceGuidance,
                decision: loongclaw_daemon::migration::types::PreviewDecision::UseDetected,
            },
        ],
        next_actions: vec![loongclaw_daemon::onboard_cli::OnboardingAction {
            kind: loongclaw_daemon::onboard_cli::OnboardingActionKind::Ask,
            label: "ask".to_owned(),
            command: "loongclaw ask --config /tmp/loongclaw-config.toml --message \"Summarize this repository and suggest the best next step.\"".to_owned(),
        }],
    };

    let lines =
        loongclaw_daemon::onboard_cli::render_onboarding_success_summary_with_width(&summary, 80);

    assert!(
        lines.iter().any(|line| line == "setup outcome"),
        "success summary should include a dedicated setup outcome heading when domain decisions exist: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "- adjusted now: provider"),
        "success summary should group adjusted domains under one readable label: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "- supplemented: channels"),
        "success summary should group supplemented domains under one readable label: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "- used detected: workspace guidance"),
        "success summary should group detected domains under one readable label: {lines:#?}"
    );
}

#[test]
fn onboarding_success_summary_wraps_domain_outcomes_for_narrow_width() {
    let summary = loongclaw_daemon::onboard_cli::OnboardingSuccessSummary {
        import_source: Some("suggested starting point".to_owned()),
        config_path: "/tmp/loongclaw-config.toml".to_owned(),
        config_status: None,
        provider: "openai".to_owned(),
        saved_provider_profiles: Vec::new(),
        model: "openai/gpt-5.1-codex".to_owned(),
        transport: "chat_completions compatibility mode".to_owned(),
        provider_endpoint: None,
        credential: Some(loongclaw_daemon::onboard_cli::OnboardingCredentialSummary {
            label: "credential source",
            value: "OPENAI_API_KEY".to_owned(),
        }),
        prompt_mode: "native prompt pack".to_owned(),
        personality: Some("friendly_collab".to_owned()),
        prompt_addendum: Some("Keep answers direct.".to_owned()),
        memory_profile: "profile_plus_window".to_owned(),
        web_search_provider: "DuckDuckGo".to_owned(),
        web_search_credential: Some(loongclaw_daemon::onboard_cli::OnboardingCredentialSummary {
            label: "web search credential",
            value: "not required".to_owned(),
        }),
        memory_path: None,
        channels: vec!["cli".to_owned()],
        suggested_channels: Vec::new(),
        domain_outcomes: vec![
            loongclaw_daemon::onboard_cli::OnboardingDomainOutcome {
                kind: loongclaw_daemon::migration::types::SetupDomainKind::Provider,
                decision: loongclaw_daemon::migration::types::PreviewDecision::AdjustedInSession,
            },
            loongclaw_daemon::onboard_cli::OnboardingDomainOutcome {
                kind: loongclaw_daemon::migration::types::SetupDomainKind::Channels,
                decision: loongclaw_daemon::migration::types::PreviewDecision::AdjustedInSession,
            },
            loongclaw_daemon::onboard_cli::OnboardingDomainOutcome {
                kind: loongclaw_daemon::migration::types::SetupDomainKind::WorkspaceGuidance,
                decision: loongclaw_daemon::migration::types::PreviewDecision::AdjustedInSession,
            },
        ],
        next_actions: vec![loongclaw_daemon::onboard_cli::OnboardingAction {
            kind: loongclaw_daemon::onboard_cli::OnboardingActionKind::Ask,
            label: "ask".to_owned(),
            command: "loongclaw ask --config /tmp/loongclaw-config.toml --message \"Summarize this repository and suggest the best next step.\"".to_owned(),
        }],
    };

    let lines =
        loongclaw_daemon::onboard_cli::render_onboarding_success_summary_with_width(&summary, 48);

    assert!(
        lines.iter().any(|line| line == "setup outcome"),
        "narrow renderer should keep the setup outcome heading visible: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "- adjusted now: provider, channels"),
        "narrow renderer should keep as many related outcome items together as fit: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "  workspace guidance"),
        "narrow renderer should wrap remaining outcome items onto an indented continuation line: {lines:#?}"
    );
}

#[test]
fn onboarding_success_summary_groups_secondary_channel_actions_after_primary_handoff() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.telegram.enabled = true;
    config.feishu.enabled = true;

    let path = PathBuf::from("/tmp/loongclaw-config.toml");
    let summary =
        loongclaw_daemon::onboard_cli::build_onboarding_success_summary(&path, &config, None);
    let lines =
        loongclaw_daemon::onboard_cli::render_onboarding_success_summary_with_width(&summary, 80);
    let rendered = lines.join(" ");

    assert!(
        rendered
            .contains("start here: loongclaw ask --config '/tmp/loongclaw-config.toml' --message")
            && rendered.contains("Summarize this repository and suggest the best next step."),
        "wide success summary should call out a single primary ask action even when wrapping is needed: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "also available"),
        "wide success summary should group secondary channel actions under a separate heading: {lines:#?}"
    );
    assert!(
        rendered.contains("- chat: loongclaw chat --config '/tmp/loongclaw-config.toml'"),
        "wide success summary should still surface interactive chat as a secondary follow-up: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line
            == "- Telegram: loongclaw telegram-serve --config '/tmp/loongclaw-config.toml'"),
        "wide success summary should list telegram as a secondary action: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line
            == "- Feishu/Lark: loongclaw feishu-serve --config '/tmp/loongclaw-config.toml'"),
        "wide success summary should list feishu as a secondary action: {lines:#?}"
    );
}

#[test]
fn onboarding_success_summary_uses_channel_handoff_when_cli_is_disabled() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.cli.enabled = false;
    config.telegram.enabled = true;

    let path = PathBuf::from("/tmp/loongclaw-config.toml");
    let summary =
        loongclaw_daemon::onboard_cli::build_onboarding_success_summary(&path, &config, None);
    let lines =
        loongclaw_daemon::onboard_cli::render_onboarding_success_summary_with_width(&summary, 80);

    assert_eq!(
        summary.next_actions[0].kind,
        loongclaw_daemon::onboard_cli::OnboardingActionKind::Channel,
        "structured actions should promote the first enabled channel when cli is disabled: {summary:#?}"
    );
    assert!(
        lines.iter().any(|line| line
            == "start here: loongclaw telegram-serve --config '/tmp/loongclaw-config.toml'"),
        "success summary should guide users into the first enabled channel when cli is disabled: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| line != "start here: loongclaw chat --config '/tmp/loongclaw-config.toml'"),
        "success summary should not keep chat as the primary handoff once cli is disabled: {lines:#?}"
    );
}

#[test]
fn onboarding_success_summary_uses_channel_catalog_handoff_when_cli_is_disabled_and_no_service_channels_are_enabled()
 {
    let mut config = mvp::config::LoongClawConfig::default();
    config.cli.enabled = false;

    let path = PathBuf::from("/tmp/loongclaw-config.toml");
    let summary =
        loongclaw_daemon::onboard_cli::build_onboarding_success_summary(&path, &config, None);
    let lines =
        loongclaw_daemon::onboard_cli::render_onboarding_success_summary_with_width(&summary, 80);

    assert_eq!(
        summary.next_actions[0].kind,
        loongclaw_daemon::onboard_cli::OnboardingActionKind::Channel,
        "channel catalog should become the primary handoff when cli and service channels are both unavailable: {summary:#?}"
    );
    assert_eq!(summary.next_actions[0].label, "channels");
    assert!(
        lines
            .iter()
            .any(|line| line
                == "start here: loongclaw channels --config '/tmp/loongclaw-config.toml'"),
        "success summary should fall back to the channel catalog when no direct cli or service-channel handoff exists: {lines:#?}"
    );
}

#[test]
fn onboarding_success_summary_sanitizes_suggested_starting_point_label() {
    let path = PathBuf::from("/tmp/loongclaw-config.toml");
    let summary = loongclaw_daemon::onboard_cli::build_onboarding_success_summary(
        &path,
        &mvp::config::LoongClawConfig::default(),
        Some("recommended import plan"),
    );

    let lines =
        loongclaw_daemon::onboard_cli::render_onboarding_success_summary_with_width(&summary, 80);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("starting point: suggested starting point")),
        "guided onboarding summary should translate the internal recommended-plan label into suggested-starting-point wording: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("recommended import plan")),
        "guided onboarding summary should not leak the internal recommended import label: {lines:#?}"
    );
}

#[test]
fn onboard_review_lines_surface_transport_summary_for_responses_compatibility_mode() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Deepseek;
    config.provider.model = "deepseek-chat".to_owned();
    config.provider.wire_api = mvp::config::ProviderWireApi::Responses;

    let lines = loongclaw_daemon::onboard_cli::render_onboard_review_lines_with_guidance(
        &config,
        None,
        &[],
        80,
    );

    assert!(
        lines
            .iter()
            .any(|line| { line == "- transport: responses compatibility mode with chat fallback" }),
        "review screen should surface the active provider transport before writing config: {lines:#?}"
    );
}

#[test]
fn onboard_review_lines_surface_region_endpoint_note_for_minimax() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Minimax;

    let lines = loongclaw_daemon::onboard_cli::render_onboard_review_lines_with_guidance(
        &config,
        None,
        &[],
        80,
    );

    assert!(
        lines.iter().any(|line| line.contains("api.minimaxi.com"))
            && lines.iter().any(|line| line.contains("api.minimax.io")),
        "review screen should show the current and alternate MiniMax regional endpoints: {lines:#?}"
    );
}

#[test]
fn onboard_review_lines_redact_invalid_configured_credential_env_value() {
    let secret = "sk-live-direct-secret-value";
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Openai;
    config.provider.api_key_env = Some(secret.to_owned());

    let lines = loongclaw_daemon::onboard_cli::render_onboard_review_lines_with_guidance(
        &config,
        None,
        &[],
        80,
    );

    assert!(
        lines
            .iter()
            .any(|line| line == "- credential source: environment variable"),
        "review screen should redact invalid configured env pointers instead of inventing a provider default binding: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| line != "- credential source: ${OPENAI_CODEX_OAUTH_TOKEN}"),
        "review screen should not replace an invalid configured env pointer with the provider default binding: {lines:#?}"
    );
    assert!(
        lines.iter().all(|line| !line.contains(secret)),
        "review screen must never echo the invalid secret-like configured env pointer: {lines:#?}"
    );
}

#[test]
fn onboarding_success_summary_surfaces_transport_summary() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Deepseek;
    config.provider.model = "deepseek-chat".to_owned();
    config.provider.wire_api = mvp::config::ProviderWireApi::Responses;

    let path = PathBuf::from("/tmp/loongclaw-config.toml");
    let summary =
        loongclaw_daemon::onboard_cli::build_onboarding_success_summary(&path, &config, None);
    let lines =
        loongclaw_daemon::onboard_cli::render_onboarding_success_summary_with_width(&summary, 80);

    assert!(
        lines
            .iter()
            .any(|line| { line == "- transport: responses compatibility mode with chat fallback" }),
        "success summary should preserve the transport mode so imported Responses configs stay explainable: {lines:#?}"
    );
}

#[test]
fn onboarding_success_summary_surfaces_region_endpoint_note_for_zhipu() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Zhipu;

    let path = PathBuf::from("/tmp/loongclaw-config.toml");
    let summary =
        loongclaw_daemon::onboard_cli::build_onboarding_success_summary(&path, &config, None);
    let lines =
        loongclaw_daemon::onboard_cli::render_onboarding_success_summary_with_width(&summary, 80);

    let rendered = lines.join("\n");
    assert!(
        rendered.contains("open.bigmodel.cn") && rendered.contains("api.z.ai"),
        "success summary should preserve region endpoint guidance for region-sensitive providers: {lines:#?}"
    );
}

#[test]
fn build_channel_onboarding_follow_up_lines_reports_manual_and_planned_channels() {
    let lines = loongclaw_daemon::onboard_cli::build_channel_onboarding_follow_up_lines(
        &mvp::config::LoongClawConfig::default(),
    );

    assert_eq!(
        lines.first().map(String::as_str),
        Some("channel next steps:")
    );
    assert!(lines.iter().any(|line| {
        line.contains("Telegram [telegram]")
            && line.contains("selection_order=10")
            && line.contains("selection_label=\"personal and group chat bot\"")
            && line.contains("strategy=manual_config")
            && line.contains("status_command=\"loongclaw doctor\"")
            && line.contains("repair_command=\"loongclaw doctor --fix\"")
    }));
    assert!(lines.iter().any(|line| {
        line.contains("Feishu/Lark [feishu]")
            && line.contains("strategy=manual_config")
            && line.contains("aliases=lark")
    }));
    assert!(lines.iter().any(|line| {
        line.contains("Discord [discord]")
            && line.contains("selection_order=40")
            && line.contains("selection_label=\"community server bot\"")
            && line.contains("strategy=planned")
            && line.contains("repair_command=-")
            && line.contains("status_command=\"loongclaw channels --json\"")
            && line.contains("blurb=\"Planned Discord gateway")
    }));
    assert!(lines.iter().any(|line| {
        line.contains("Webhook [webhook]")
            && line.contains("selection_order=110")
            && line.contains("selection_label=\"generic http integration\"")
            && line.contains("strategy=planned")
    }));
}
