use std::{collections::BTreeMap, fs, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::CliResult;

use super::{
    channels::{CliChannelConfig, FeishuChannelConfig, TelegramChannelConfig},
    conversation::ConversationConfig,
    provider::ProviderConfig,
    shared::{
        ConfigValidationIssue, ConfigValidationLocale, DEFAULT_CONFIG_FILE,
        default_loongclaw_home as shared_default_loongclaw_home, expand_path,
        format_config_validation_issues,
    },
    tools_memory::{ExternalSkillsConfig, MemoryConfig, ToolConfig},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigValidationDiagnostic {
    pub code: String,
    pub problem_type: String,
    pub title_key: String,
    pub title: String,
    pub message_key: String,
    pub message_locale: String,
    pub message_variables: BTreeMap<String, String>,
    pub field_path: String,
    pub inline_field_path: String,
    pub example_env_name: String,
    pub suggested_env_name: Option<String>,
    pub message: String,
}

impl ConfigValidationDiagnostic {
    fn from_issue(issue: &ConfigValidationIssue, locale: ConfigValidationLocale) -> Self {
        let message_variables = issue.message_variables();
        Self {
            code: issue.code.as_str().to_owned(),
            problem_type: issue.code.problem_type_uri().to_owned(),
            title_key: issue.title_key().to_owned(),
            title: issue.title(locale),
            message_key: issue.message_key().to_owned(),
            message_locale: locale.as_str().to_owned(),
            message_variables: message_variables.clone(),
            field_path: issue.field_path.clone(),
            inline_field_path: issue.inline_field_path.clone(),
            example_env_name: issue.example_env_name.clone(),
            suggested_env_name: issue.suggested_env_name.clone(),
            message: issue.render_with_variables(locale, &message_variables),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LoongClawConfig {
    #[serde(default)]
    pub provider: ProviderConfig,
    #[serde(default)]
    pub cli: CliChannelConfig,
    #[serde(default)]
    pub telegram: TelegramChannelConfig,
    #[serde(default)]
    pub feishu: FeishuChannelConfig,
    #[serde(default)]
    pub conversation: ConversationConfig,
    #[serde(default)]
    pub tools: ToolConfig,
    #[serde(default)]
    pub external_skills: ExternalSkillsConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
}

impl LoongClawConfig {
    fn collect_validation_issues(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        issues.extend(self.provider.validate());
        issues.extend(self.telegram.validate());
        issues.extend(self.feishu.validate());
        issues.extend(self.memory.validate());
        issues
    }

    pub fn validate(&self) -> CliResult<()> {
        let issues = self.collect_validation_issues();
        if issues.is_empty() {
            return Ok(());
        }
        Err(format_config_validation_issues(&issues))
    }

    pub fn validation_diagnostics(&self) -> Vec<ConfigValidationDiagnostic> {
        self.validation_diagnostics_with_locale(ConfigValidationLocale::En)
    }

    fn validation_diagnostics_with_locale(
        &self,
        locale: ConfigValidationLocale,
    ) -> Vec<ConfigValidationDiagnostic> {
        self.collect_validation_issues()
            .iter()
            .map(|issue| ConfigValidationDiagnostic::from_issue(issue, locale))
            .collect()
    }
}

pub fn load(path: Option<&str>) -> CliResult<(PathBuf, LoongClawConfig)> {
    let config_path = path.map(expand_path).unwrap_or_else(default_config_path);
    let raw = fs::read_to_string(&config_path).map_err(|error| {
        format!(
            "failed to read config {}: {error}. run `loongclaw setup` first",
            config_path.display()
        )
    })?;
    parse_toml_config(&raw).map(|config| (config_path, config))
}

pub fn validate_file(path: Option<&str>) -> CliResult<(PathBuf, Vec<ConfigValidationDiagnostic>)> {
    validate_file_with_locale(path, ConfigValidationLocale::En.as_str())
}

pub fn normalize_validation_locale(locale_tag: &str) -> String {
    ConfigValidationLocale::from_tag(locale_tag)
        .as_str()
        .to_owned()
}

pub fn supported_validation_locales() -> Vec<&'static str> {
    ConfigValidationLocale::supported_tags().to_vec()
}

pub fn validate_file_with_locale(
    path: Option<&str>,
    locale_tag: &str,
) -> CliResult<(PathBuf, Vec<ConfigValidationDiagnostic>)> {
    let config_path = path.map(expand_path).unwrap_or_else(default_config_path);
    let raw = fs::read_to_string(&config_path).map_err(|error| {
        format!(
            "failed to read config {}: {error}. run `loongclaw setup` first",
            config_path.display()
        )
    })?;
    let config = parse_toml_config_without_validation(&raw)?;
    let locale = ConfigValidationLocale::from_tag(locale_tag);
    Ok((
        config_path,
        config.validation_diagnostics_with_locale(locale),
    ))
}

pub fn write_template(path: Option<&str>, force: bool) -> CliResult<PathBuf> {
    let output_path = path.map(expand_path).unwrap_or_else(default_config_path);
    if output_path.exists() && !force {
        return Err(format!(
            "config {} already exists (use --force to overwrite)",
            output_path.display()
        ));
    }

    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create config directory: {error}"))?;
    }

    let encoded = format!(
        "{}{}",
        template_secret_usage_comment(),
        encode_toml_config(&LoongClawConfig::default())?
    );
    fs::write(&output_path, encoded).map_err(|error| {
        format!(
            "failed to write config file {}: {error}",
            output_path.display()
        )
    })?;
    Ok(output_path)
}

pub fn write(path: Option<&str>, config: &LoongClawConfig, force: bool) -> CliResult<PathBuf> {
    let output_path = path.map(expand_path).unwrap_or_else(default_config_path);
    if output_path.exists() && !force {
        return Err(format!(
            "config {} already exists (use --force to overwrite)",
            output_path.display()
        ));
    }

    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create config directory: {error}"))?;
    }

    let encoded = encode_toml_config(config)?;
    fs::write(&output_path, encoded).map_err(|error| {
        format!(
            "failed to write config file {}: {error}",
            output_path.display()
        )
    })?;
    Ok(output_path)
}

pub fn render(config: &LoongClawConfig) -> CliResult<String> {
    encode_toml_config(config)
}

pub fn default_config_path() -> PathBuf {
    default_loongclaw_home().join(DEFAULT_CONFIG_FILE)
}

pub fn default_loongclaw_home() -> PathBuf {
    shared_default_loongclaw_home()
}

#[cfg(feature = "config-toml")]
fn parse_toml_config(raw: &str) -> CliResult<LoongClawConfig> {
    let config = parse_toml_config_without_validation(raw)?;
    config.validate()?;
    Ok(config)
}

#[cfg(feature = "config-toml")]
fn parse_toml_config_without_validation(raw: &str) -> CliResult<LoongClawConfig> {
    toml::from_str::<LoongClawConfig>(raw)
        .map_err(|error| format!("failed to parse TOML config: {error}"))
}

#[cfg(not(feature = "config-toml"))]
fn parse_toml_config(_raw: &str) -> CliResult<LoongClawConfig> {
    Err("config-toml feature is disabled for this build".to_owned())
}

#[cfg(not(feature = "config-toml"))]
fn parse_toml_config_without_validation(_raw: &str) -> CliResult<LoongClawConfig> {
    Err("config-toml feature is disabled for this build".to_owned())
}

#[cfg(feature = "config-toml")]
fn encode_toml_config(config: &LoongClawConfig) -> CliResult<String> {
    toml::to_string_pretty(config).map_err(|error| format!("failed to encode TOML config: {error}"))
}

#[cfg(not(feature = "config-toml"))]
fn encode_toml_config(_config: &LoongClawConfig) -> CliResult<String> {
    Err("config-toml feature is disabled for this build".to_owned())
}

fn template_secret_usage_comment() -> &'static str {
    "# Secret configuration notes:\n\
# - Preferred provider credential form: `provider.api_key = \"${PROVIDER_API_KEY}\"`.\n\
# - `provider.api_key` also accepts direct literals and explicit env refs like `$VAR`, `env:VAR`, and `%VAR%`.\n\
# - Legacy `*_env` fields stay supported for compatibility, but new configs should prefer the non-`_env` fields.\n\
\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_config_path(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should move forward")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}.toml"))
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn load_rejects_secret_literal_in_env_pointer_fields() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("loongclaw-config-validate-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("create temp directory");
        let config_path = temp_dir.join("config.toml");
        let raw = r#"
[provider]
api_key_env = "sk-inline-secret-literal"

[telegram]
bot_token_env = "123456789:telegram-inline-secret-literal"
"#;
        std::fs::write(&config_path, raw).expect("write test config");

        let error = load(Some(config_path.to_string_lossy().as_ref()))
            .expect_err("load should fail for misplaced secret literals");
        assert!(error.contains("provider.api_key_env"));
        assert!(error.contains("telegram.bot_token_env"));

        std::fs::remove_file(&config_path).ok();
        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn write_template_includes_secret_usage_comment() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("loongclaw-template-comment-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("create temp directory");
        let config_path = temp_dir.join("config.toml");

        write_template(Some(config_path.to_string_lossy().as_ref()), true)
            .expect("write template should succeed");

        let raw = std::fs::read_to_string(&config_path).expect("read template");
        assert!(raw.contains("# Secret configuration notes:"));
        assert!(raw.contains("Preferred provider credential form"));

        std::fs::remove_file(&config_path).ok();
        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn write_template_prefers_generic_provider_api_key_reference_example() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("loongclaw-template-api-key-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("create temp directory");
        let config_path = temp_dir.join("config.toml");

        write_template(Some(config_path.to_string_lossy().as_ref()), true)
            .expect("write template should succeed");

        let raw = std::fs::read_to_string(&config_path).expect("read template");
        assert!(raw.contains("provider.api_key = \"${PROVIDER_API_KEY}\""));
        assert!(!raw.contains("provider.api_key_env = \"PROVIDER_API_KEY\""));

        std::fs::remove_file(&config_path).ok();
        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn validate_file_returns_structured_diagnostics() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("loongclaw-config-diagnostics-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("create temp directory");
        let config_path = temp_dir.join("config.toml");
        let raw = r#"
[provider]
api_key_env = "$OPENAI_API_KEY"
"#;
        std::fs::write(&config_path, raw).expect("write test config");

        let (_, diagnostics) = validate_file(Some(config_path.to_string_lossy().as_ref()))
            .expect("validate_file should parse and return diagnostics");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "config.env_pointer.dollar_prefix");
        assert_eq!(
            diagnostics[0].problem_type,
            "urn:loongclaw:problem:config.env_pointer.dollar_prefix"
        );
        assert_eq!(
            diagnostics[0].title_key,
            "config.env_pointer.dollar_prefix.title"
        );
        assert_eq!(diagnostics[0].title, "Dollar Prefix Used In Env Pointer");
        assert_eq!(
            diagnostics[0].message_key,
            "config.env_pointer.dollar_prefix"
        );
        assert_eq!(diagnostics[0].message_locale, "en");
        assert_eq!(diagnostics[0].field_path, "provider.api_key_env");
        assert_eq!(
            diagnostics[0].message_variables.get("field_path"),
            Some(&"provider.api_key_env".to_owned())
        );
        assert_eq!(
            diagnostics[0].message_variables.get("code"),
            Some(&"config.env_pointer.dollar_prefix".to_owned())
        );
        assert!(diagnostics[0].message.contains("without `$`"));

        std::fs::remove_file(&config_path).ok();
        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn validate_file_returns_channel_account_diagnostics() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let temp_dir =
            std::env::temp_dir().join(format!("loongclaw-config-channel-account-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("create temp directory");
        let config_path = temp_dir.join("config.toml");
        let raw = r#"
[telegram.accounts."Work Bot"]
bot_token_env = "WORK_TELEGRAM_TOKEN"

[telegram.accounts."work-bot"]
bot_token_env = "WORK_TELEGRAM_TOKEN_DUP"
"#;
        std::fs::write(&config_path, raw).expect("write test config");

        let (_, diagnostics) = validate_file(Some(config_path.to_string_lossy().as_ref()))
            .expect("validate_file should parse and return diagnostics");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "config.channel_account.duplicate_id");
        assert_eq!(
            diagnostics[0].problem_type,
            "urn:loongclaw:problem:config.channel_account.duplicate_id"
        );
        assert_eq!(diagnostics[0].field_path, "telegram.accounts");
        assert_eq!(
            diagnostics[0]
                .message_variables
                .get("normalized_account_id"),
            Some(&"work-bot".to_owned())
        );
        assert_eq!(
            diagnostics[0].message_variables.get("raw_account_labels"),
            Some(&"Work Bot, work-bot".to_owned())
        );

        std::fs::remove_file(&config_path).ok();
        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn validate_file_locale_tag_aliases_normalize_to_en() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("loongclaw-config-locale-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("create temp directory");
        let config_path = temp_dir.join("config.toml");
        let raw = r#"
[provider]
api_key_env = "$OPENAI_API_KEY"
"#;
        std::fs::write(&config_path, raw).expect("write test config");

        let (_, diagnostics) =
            validate_file_with_locale(Some(config_path.to_string_lossy().as_ref()), "en-US")
                .expect("validate_file_with_locale should parse and return diagnostics");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].message_locale, "en");

        std::fs::remove_file(&config_path).ok();
        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn normalize_validation_locale_falls_back_to_en() {
        assert_eq!(normalize_validation_locale("en-US"), "en");
        assert_eq!(normalize_validation_locale("zh-CN"), "en");
        assert_eq!(normalize_validation_locale(""), "en");
    }

    #[test]
    fn supported_validation_locales_stays_stable() {
        assert_eq!(supported_validation_locales(), vec!["en"]);
    }

    #[test]
    fn load_missing_config_guides_user_to_loongclaw_setup() {
        let missing = unique_config_path("loongclaw-config-missing");
        let path_string = missing.display().to_string();

        let error = load(Some(&path_string)).expect_err("missing config should fail");
        assert!(error.contains("run `loongclaw setup` first"));
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn validate_file_reports_percent_wrapped_pointer_code() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("loongclaw-config-percent-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("create temp directory");
        let config_path = temp_dir.join("config.toml");
        let raw = r#"
[provider]
api_key_env = "%OPENAI_API_KEY%"
"#;
        std::fs::write(&config_path, raw).expect("write test config");

        let (_, diagnostics) = validate_file(Some(config_path.to_string_lossy().as_ref()))
            .expect("validate_file should parse and return diagnostics");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "config.env_pointer.percent_wrapped");
        assert_eq!(
            diagnostics[0].problem_type,
            "urn:loongclaw:problem:config.env_pointer.percent_wrapped"
        );

        std::fs::remove_file(&config_path).ok();
        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn validate_file_diagnostic_does_not_echo_secret_literal() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let temp_dir =
            std::env::temp_dir().join(format!("loongclaw-config-no-secret-echo-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("create temp directory");
        let config_path = temp_dir.join("config.toml");
        let secret = "sk-inline-super-secret-token";
        let raw = format!(
            r#"
[provider]
api_key_env = "{secret}"
"#
        );
        std::fs::write(&config_path, raw).expect("write test config");

        let (_, diagnostics) = validate_file(Some(config_path.to_string_lossy().as_ref()))
            .expect("validate_file should parse and return diagnostics");
        assert_eq!(diagnostics.len(), 1);
        assert!(!diagnostics[0].message.contains(secret));
        for value in diagnostics[0].message_variables.values() {
            assert!(!value.contains(secret));
        }

        std::fs::remove_file(&config_path).ok();
        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn write_persists_custom_model_and_prompt() {
        let path = unique_config_path("loongclaw-config-runtime");
        let path_string = path.display().to_string();
        let mut config = LoongClawConfig::default();
        config.provider.model = "openai/gpt-5.1-codex".to_owned();
        config.cli.system_prompt = "You are an onboarding assistant.".to_owned();

        let written = write(Some(&path_string), &config, true).expect("config write should pass");
        assert_eq!(written, path);

        let (_, loaded) = load(Some(&path_string)).expect("config load should pass");
        assert_eq!(loaded.provider.model, "openai/gpt-5.1-codex");
        assert_eq!(loaded.cli.system_prompt, "You are an onboarding assistant.");

        let _ = fs::remove_file(path);
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn write_persists_prompt_pack_and_personality_metadata() {
        let path = unique_config_path("loongclaw-prompt-config");
        let path_string = path.display().to_string();
        let mut config = LoongClawConfig::default();
        config.cli.prompt_pack_id = Some("loongclaw-core-v1".to_owned());
        config.cli.personality = Some(crate::prompt::PromptPersonality::AutonomousExecutor);

        write(Some(&path_string), &config, true).expect("config write should pass");
        let (_, loaded) = load(Some(&path_string)).expect("config load should pass");

        assert_eq!(
            loaded.cli.prompt_pack_id.as_deref(),
            Some("loongclaw-core-v1")
        );
        assert_eq!(
            loaded.cli.personality,
            Some(crate::prompt::PromptPersonality::AutonomousExecutor)
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn write_persists_memory_profile_metadata() {
        let path = unique_config_path("loongclaw-memory-config");
        let path_string = path.display().to_string();
        let mut config = LoongClawConfig::default();
        config.memory.profile = crate::config::MemoryProfile::WindowPlusSummary;
        config.memory.summary_max_chars = 900;
        config.memory.profile_note = Some("Imported NanoBot preferences".to_owned());

        write(Some(&path_string), &config, true).expect("config write should pass");
        let (_, loaded) = load(Some(&path_string)).expect("config load should pass");

        assert_eq!(
            loaded.memory.profile,
            crate::config::MemoryProfile::WindowPlusSummary
        );
        assert_eq!(loaded.memory.summary_max_chars, 900);
        assert_eq!(
            loaded.memory.profile_note.as_deref(),
            Some("Imported NanoBot preferences")
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn write_default_config_omits_legacy_provider_api_key_env_field() {
        let path = unique_config_path("loongclaw-config-runtime-default");
        let path_string = path.display().to_string();

        write(Some(&path_string), &LoongClawConfig::default(), true)
            .expect("default config write should pass");

        let raw = fs::read_to_string(&path).expect("read written config");
        assert!(!raw.contains("api_key_env = \"OPENAI_API_KEY\""));

        let _ = fs::remove_file(path);
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn write_default_config_keeps_external_skills_guardrails() {
        let path = unique_config_path("loongclaw-config-runtime-external-skills");
        let path_string = path.display().to_string();

        write(Some(&path_string), &LoongClawConfig::default(), true)
            .expect("default config write should pass");

        let raw = fs::read_to_string(&path).expect("read written config");
        assert!(raw.contains("[external_skills]"));
        assert!(raw.contains("enabled = false"));
        assert!(raw.contains("require_download_approval = true"));

        let (_, loaded) = load(Some(&path_string)).expect("config load should pass");
        assert!(!loaded.external_skills.enabled);
        assert!(loaded.external_skills.require_download_approval);
        assert!(loaded.external_skills.allowed_domains.is_empty());
        assert!(loaded.external_skills.blocked_domains.is_empty());

        let _ = fs::remove_file(path);
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn write_rejects_overwrite_without_force() {
        let path = unique_config_path("loongclaw-config-runtime");
        let path_string = path.display().to_string();
        let first = LoongClawConfig::default();
        write(Some(&path_string), &first, true).expect("initial config write should pass");

        let mut updated = LoongClawConfig::default();
        updated.provider.model = "openai/gpt-5".to_owned();
        let error = write(Some(&path_string), &updated, false)
            .expect_err("overwrite without --force should fail");
        assert!(error.contains("already exists"));

        let _ = fs::remove_file(path);
    }
}
