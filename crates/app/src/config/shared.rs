use std::{
    collections::BTreeMap,
    env,
    path::{Path, PathBuf},
};

pub(super) const DEFAULT_CONFIG_FILE: &str = "config.toml";
pub(super) const DEFAULT_SQLITE_FILE: &str = "memory.sqlite3";
pub const CLI_COMMAND_NAME: &str = "loongclaw";
pub(super) const DEFAULT_FEISHU_SQLITE_FILE: &str = "feishu.sqlite3";

pub(super) struct EnvPointerValidationHint<'a> {
    pub inline_field_path: &'a str,
    pub example_env_name: &'a str,
    pub detect_telegram_token_shape: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConfigValidationLocale {
    En,
}

impl ConfigValidationLocale {
    pub fn from_tag(raw: &str) -> Self {
        let normalized = raw.trim().to_ascii_lowercase();
        if normalized.is_empty() || normalized.starts_with("en") {
            return Self::En;
        }
        // Keep compatibility for unknown locale tags until additional catalogs are added.
        Self::En
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::En => "en",
        }
    }

    pub const fn supported_tags() -> &'static [&'static str] {
        &["en"]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConfigValidationCode {
    Assignment,
    DollarPrefix,
    PercentWrapped,
    SecretLiteral,
    InvalidName,
    NumericRange,
    DuplicateChannelAccountId,
    UnknownChannelDefaultAccount,
}

impl ConfigValidationCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            ConfigValidationCode::Assignment => "config.env_pointer.assignment",
            ConfigValidationCode::DollarPrefix => "config.env_pointer.dollar_prefix",
            ConfigValidationCode::PercentWrapped => "config.env_pointer.percent_wrapped",
            ConfigValidationCode::SecretLiteral => "config.env_pointer.secret_literal",
            ConfigValidationCode::InvalidName => "config.env_pointer.invalid_name",
            ConfigValidationCode::NumericRange => "config.numeric_range",
            ConfigValidationCode::DuplicateChannelAccountId => {
                "config.channel_account.duplicate_id"
            }
            ConfigValidationCode::UnknownChannelDefaultAccount => {
                "config.channel_account.unknown_default"
            }
        }
    }

    pub const fn problem_type_uri(self) -> &'static str {
        match self {
            ConfigValidationCode::Assignment => {
                "urn:loongclaw:problem:config.env_pointer.assignment"
            }
            ConfigValidationCode::DollarPrefix => {
                "urn:loongclaw:problem:config.env_pointer.dollar_prefix"
            }
            ConfigValidationCode::PercentWrapped => {
                "urn:loongclaw:problem:config.env_pointer.percent_wrapped"
            }
            ConfigValidationCode::SecretLiteral => {
                "urn:loongclaw:problem:config.env_pointer.secret_literal"
            }
            ConfigValidationCode::InvalidName => {
                "urn:loongclaw:problem:config.env_pointer.invalid_name"
            }
            ConfigValidationCode::NumericRange => "urn:loongclaw:problem:config.numeric_range",
            ConfigValidationCode::DuplicateChannelAccountId => {
                "urn:loongclaw:problem:config.channel_account.duplicate_id"
            }
            ConfigValidationCode::UnknownChannelDefaultAccount => {
                "urn:loongclaw:problem:config.channel_account.unknown_default"
            }
        }
    }

    pub const fn title_key(self) -> &'static str {
        match self {
            ConfigValidationCode::Assignment => "config.env_pointer.assignment.title",
            ConfigValidationCode::DollarPrefix => "config.env_pointer.dollar_prefix.title",
            ConfigValidationCode::PercentWrapped => "config.env_pointer.percent_wrapped.title",
            ConfigValidationCode::SecretLiteral => "config.env_pointer.secret_literal.title",
            ConfigValidationCode::InvalidName => "config.env_pointer.invalid_name.title",
            ConfigValidationCode::NumericRange => "config.numeric_range.title",
            ConfigValidationCode::DuplicateChannelAccountId => {
                "config.channel_account.duplicate_id.title"
            }
            ConfigValidationCode::UnknownChannelDefaultAccount => {
                "config.channel_account.unknown_default.title"
            }
        }
    }

    fn localized_title(self, locale: ConfigValidationLocale) -> &'static str {
        lookup_validation_message(locale, self.title_key())
            .unwrap_or_else(|| self.fallback_title_en())
    }

    fn localized_detail_template(self, locale: ConfigValidationLocale) -> &'static str {
        lookup_validation_message(locale, self.as_str())
            .unwrap_or_else(|| self.fallback_detail_template_en())
    }

    const fn fallback_title_en(self) -> &'static str {
        match self {
            ConfigValidationCode::Assignment => "Assignment Used In Env Pointer",
            ConfigValidationCode::DollarPrefix => "Dollar Prefix Used In Env Pointer",
            ConfigValidationCode::PercentWrapped => "Percent-Wrapped Env Pointer Notation",
            ConfigValidationCode::SecretLiteral => "Secret Literal Used In Env Pointer",
            ConfigValidationCode::InvalidName => "Invalid Env Pointer Name",
            ConfigValidationCode::NumericRange => "Config Value Out Of Range",
            ConfigValidationCode::DuplicateChannelAccountId => {
                "Duplicate Normalized Channel Account ID"
            }
            ConfigValidationCode::UnknownChannelDefaultAccount => "Unknown Channel Default Account",
        }
    }

    const fn fallback_detail_template_en(self) -> &'static str {
        match self {
            ConfigValidationCode::Assignment => {
                "[{code}] {field_path} expects an environment variable name, not `KEY=VALUE`. use `{field_path} = \"{suggested_env_name}\"` and place the secret value in that env var"
            }
            ConfigValidationCode::DollarPrefix => {
                "[{code}] {field_path} expects an environment variable name without `$`. use `{field_path} = \"{suggested_env_name}\"`"
            }
            ConfigValidationCode::PercentWrapped => {
                "[{code}] {field_path} expects an environment variable name, not `%VAR%` notation. use `{field_path} = \"{suggested_env_name}\"`"
            }
            ConfigValidationCode::SecretLiteral => {
                "[{code}] {field_path} expects an environment variable name, not a secret literal. move the value to `{inline_field_path}` or set `{field_path}` to a name like `{example_env_name}`"
            }
            ConfigValidationCode::InvalidName => {
                "[{code}] {field_path} is not a valid environment variable name reference. use `{field_path}` with a name like `{example_env_name}`"
            }
            ConfigValidationCode::NumericRange => {
                "[{code}] {field_path} must be between {min} and {max}; got {actual_value}"
            }
            ConfigValidationCode::DuplicateChannelAccountId => {
                "[{code}] {field_path} contains duplicate configured accounts that normalize to `{normalized_account_id}`: {raw_account_labels}. rename one of the account keys so each normalized account id is unique"
            }
            ConfigValidationCode::UnknownChannelDefaultAccount => {
                "[{code}] {field_path} points to `{requested_account_id}`, but configured accounts are: {configured_account_ids}. set `{field_path}` to one of the configured account ids"
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ConfigValidationIssue {
    pub code: ConfigValidationCode,
    pub field_path: String,
    pub inline_field_path: String,
    pub example_env_name: String,
    pub suggested_env_name: Option<String>,
    pub extra_message_variables: BTreeMap<String, String>,
}

impl ConfigValidationIssue {
    pub(super) const fn message_key(&self) -> &'static str {
        self.code.as_str()
    }

    pub(super) const fn title_key(&self) -> &'static str {
        self.code.title_key()
    }

    pub(super) fn title(&self, locale: ConfigValidationLocale) -> String {
        self.code.localized_title(locale).to_owned()
    }

    pub(super) fn message_variables(&self) -> BTreeMap<String, String> {
        let mut variables = BTreeMap::new();
        variables.insert("code".to_owned(), self.code.as_str().to_owned());
        variables.insert("field_path".to_owned(), self.field_path.clone());
        variables.insert(
            "inline_field_path".to_owned(),
            self.inline_field_path.clone(),
        );
        variables.insert("example_env_name".to_owned(), self.example_env_name.clone());
        let suggested = self
            .suggested_env_name
            .as_deref()
            .unwrap_or(self.example_env_name.as_str());
        variables.insert("suggested_env_name".to_owned(), suggested.to_owned());
        variables.extend(self.extra_message_variables.clone());
        variables
    }

    pub(super) fn render_with_variables(
        &self,
        locale: ConfigValidationLocale,
        variables: &BTreeMap<String, String>,
    ) -> String {
        let template = self.code.localized_detail_template(locale);
        render_message_template(template, variables)
    }

    pub(super) fn render(&self, locale: ConfigValidationLocale) -> String {
        let variables = self.message_variables();
        self.render_with_variables(locale, &variables)
    }
}

#[derive(Debug, Clone, Copy)]
struct ConfigValidationCatalogEntry {
    key: &'static str,
    value: &'static str,
}

impl ConfigValidationCatalogEntry {
    const fn new(key: &'static str, value: &'static str) -> Self {
        Self { key, value }
    }
}

const EN_VALIDATION_MESSAGE_CATALOG: &[ConfigValidationCatalogEntry] = &[
    ConfigValidationCatalogEntry::new(
        "config.env_pointer.assignment.title",
        "Assignment Used In Env Pointer",
    ),
    ConfigValidationCatalogEntry::new(
        "config.env_pointer.dollar_prefix.title",
        "Dollar Prefix Used In Env Pointer",
    ),
    ConfigValidationCatalogEntry::new(
        "config.env_pointer.percent_wrapped.title",
        "Percent-Wrapped Env Pointer Notation",
    ),
    ConfigValidationCatalogEntry::new(
        "config.env_pointer.secret_literal.title",
        "Secret Literal Used In Env Pointer",
    ),
    ConfigValidationCatalogEntry::new(
        "config.env_pointer.invalid_name.title",
        "Invalid Env Pointer Name",
    ),
    ConfigValidationCatalogEntry::new("config.numeric_range.title", "Config Value Out Of Range"),
    ConfigValidationCatalogEntry::new(
        "config.channel_account.duplicate_id.title",
        "Duplicate Normalized Channel Account ID",
    ),
    ConfigValidationCatalogEntry::new(
        "config.channel_account.unknown_default.title",
        "Unknown Channel Default Account",
    ),
    ConfigValidationCatalogEntry::new(
        "config.env_pointer.assignment",
        "[{code}] {field_path} expects an environment variable name, not `KEY=VALUE`. use `{field_path} = \"{suggested_env_name}\"` and place the secret value in that env var",
    ),
    ConfigValidationCatalogEntry::new(
        "config.env_pointer.dollar_prefix",
        "[{code}] {field_path} expects an environment variable name without `$`. use `{field_path} = \"{suggested_env_name}\"`",
    ),
    ConfigValidationCatalogEntry::new(
        "config.env_pointer.percent_wrapped",
        "[{code}] {field_path} expects an environment variable name, not `%VAR%` notation. use `{field_path} = \"{suggested_env_name}\"`",
    ),
    ConfigValidationCatalogEntry::new(
        "config.env_pointer.secret_literal",
        "[{code}] {field_path} expects an environment variable name, not a secret literal. move the value to `{inline_field_path}` or set `{field_path}` to a name like `{example_env_name}`",
    ),
    ConfigValidationCatalogEntry::new(
        "config.env_pointer.invalid_name",
        "[{code}] {field_path} is not a valid environment variable name reference. use `{field_path}` with a name like `{example_env_name}`",
    ),
    ConfigValidationCatalogEntry::new(
        "config.numeric_range",
        "[{code}] {field_path} must be between {min} and {max}; got {actual_value}",
    ),
    ConfigValidationCatalogEntry::new(
        "config.channel_account.duplicate_id",
        "[{code}] {field_path} contains duplicate configured accounts that normalize to `{normalized_account_id}`: {raw_account_labels}. rename one of the account keys so each normalized account id is unique",
    ),
    ConfigValidationCatalogEntry::new(
        "config.channel_account.unknown_default",
        "[{code}] {field_path} points to `{requested_account_id}`, but configured accounts are: {configured_account_ids}. set `{field_path}` to one of the configured account ids",
    ),
];

fn lookup_validation_message(locale: ConfigValidationLocale, key: &str) -> Option<&'static str> {
    let catalog = match locale {
        ConfigValidationLocale::En => EN_VALIDATION_MESSAGE_CATALOG,
    };
    lookup_catalog_value(catalog, key)
}

fn lookup_catalog_value(
    catalog: &[ConfigValidationCatalogEntry],
    key: &str,
) -> Option<&'static str> {
    for entry in catalog {
        if entry.key == key {
            return Some(entry.value);
        }
    }
    None
}

fn render_message_template(template: &str, variables: &BTreeMap<String, String>) -> String {
    let mut rendered = template.to_owned();
    for (key, value) in variables {
        let placeholder = format!("{{{key}}}");
        rendered = rendered.replace(&placeholder, value);
    }
    rendered
}

pub(super) fn format_config_validation_issues(issues: &[ConfigValidationIssue]) -> String {
    format_config_validation_issues_with_locale(issues, ConfigValidationLocale::En)
}

pub(super) fn format_config_validation_issues_with_locale(
    issues: &[ConfigValidationIssue],
    locale: ConfigValidationLocale,
) -> String {
    let details = issues
        .iter()
        .map(|issue| issue.render(locale))
        .collect::<Vec<_>>()
        .join("; ");
    format!("invalid configuration: {details}")
}

fn get_user_home() -> PathBuf {
    resolve_user_home(
        env::var_os("HOME").as_deref(),
        env::var_os("USERPROFILE").as_deref(),
    )
}

fn resolve_user_home(
    home: Option<&std::ffi::OsStr>,
    userprofile: Option<&std::ffi::OsStr>,
) -> PathBuf {
    home.or(userprofile)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub(super) fn default_loongclaw_home() -> PathBuf {
    get_user_home().join(".loongclaw")
}

pub fn expand_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed == "~" {
        return get_user_home();
    }
    if let Some(stripped) = trimmed.strip_prefix("~/") {
        return get_user_home().join(stripped);
    }
    Path::new(trimmed).to_path_buf()
}

pub(super) fn validate_env_pointer_field(
    field_path: &str,
    env_key: Option<&str>,
    hint: EnvPointerValidationHint<'_>,
) -> Result<(), Box<ConfigValidationIssue>> {
    let Some(raw) = env_key else {
        return Ok(());
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(());
    }

    if let Some((name, _value)) = parse_env_assignment(trimmed) {
        return Err(Box::new(ConfigValidationIssue {
            code: ConfigValidationCode::Assignment,
            field_path: field_path.to_owned(),
            inline_field_path: hint.inline_field_path.to_owned(),
            example_env_name: hint.example_env_name.to_owned(),
            suggested_env_name: Some(name.to_owned()),
            extra_message_variables: BTreeMap::new(),
        }));
    }

    if let Some(raw_name) = trimmed.strip_prefix('$') {
        let suggested = normalize_dollar_prefixed_env_name(raw_name, hint.example_env_name);
        return Err(Box::new(ConfigValidationIssue {
            code: ConfigValidationCode::DollarPrefix,
            field_path: field_path.to_owned(),
            inline_field_path: hint.inline_field_path.to_owned(),
            example_env_name: hint.example_env_name.to_owned(),
            suggested_env_name: Some(suggested),
            extra_message_variables: BTreeMap::new(),
        }));
    }

    if let Some(raw_name) = parse_percent_wrapped_env_name(trimmed) {
        let suggested = if raw_name.is_empty() {
            hint.example_env_name.to_owned()
        } else {
            raw_name.to_owned()
        };
        return Err(Box::new(ConfigValidationIssue {
            code: ConfigValidationCode::PercentWrapped,
            field_path: field_path.to_owned(),
            inline_field_path: hint.inline_field_path.to_owned(),
            example_env_name: hint.example_env_name.to_owned(),
            suggested_env_name: Some(suggested),
            extra_message_variables: BTreeMap::new(),
        }));
    }

    if looks_like_secret_literal(trimmed, hint.detect_telegram_token_shape) {
        return Err(Box::new(ConfigValidationIssue {
            code: ConfigValidationCode::SecretLiteral,
            field_path: field_path.to_owned(),
            inline_field_path: hint.inline_field_path.to_owned(),
            example_env_name: hint.example_env_name.to_owned(),
            suggested_env_name: None,
            extra_message_variables: BTreeMap::new(),
        }));
    }

    if !looks_like_compatible_env_name(trimmed) {
        return Err(Box::new(ConfigValidationIssue {
            code: ConfigValidationCode::InvalidName,
            field_path: field_path.to_owned(),
            inline_field_path: hint.inline_field_path.to_owned(),
            example_env_name: hint.example_env_name.to_owned(),
            suggested_env_name: Some(hint.example_env_name.to_owned()),
            extra_message_variables: BTreeMap::new(),
        }));
    }

    Ok(())
}

pub(super) fn validate_numeric_range(
    field_path: &str,
    actual_value: usize,
    min: usize,
    max: usize,
) -> Result<(), Box<ConfigValidationIssue>> {
    if (min..=max).contains(&actual_value) {
        return Ok(());
    }

    let mut extra_message_variables = BTreeMap::new();
    extra_message_variables.insert("actual_value".to_owned(), actual_value.to_string());
    extra_message_variables.insert("min".to_owned(), min.to_string());
    extra_message_variables.insert("max".to_owned(), max.to_string());

    Err(Box::new(ConfigValidationIssue {
        code: ConfigValidationCode::NumericRange,
        field_path: field_path.to_owned(),
        inline_field_path: field_path.to_owned(),
        example_env_name: String::new(),
        suggested_env_name: None,
        extra_message_variables,
    }))
}

fn looks_like_secret_literal(raw: &str, detect_telegram_token_shape: bool) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return false;
    }

    let lowered = trimmed.to_ascii_lowercase();
    if lowered.starts_with("sk-")
        || lowered.starts_with("rk-")
        || lowered.starts_with("pk-")
        || lowered.starts_with("ya29.")
        || lowered.starts_with("bearer ")
        || lowered.starts_with("ghp_")
        || lowered.starts_with("glpat-")
    {
        return true;
    }

    if detect_telegram_token_shape && looks_like_telegram_bot_token(trimmed) {
        return true;
    }

    if looks_like_compatible_env_name(trimmed) {
        return false;
    }

    // Token-like strings are usually long and contain punctuation not typical for env names.
    trimmed.len() >= 24
        && trimmed
            .chars()
            .any(|ch| matches!(ch, '-' | '.' | ':' | '/' | '+'))
}

fn looks_like_compatible_env_name(raw: &str) -> bool {
    let mut chars = raw.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphanumeric() || first == '_') {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.')
}

fn looks_like_telegram_bot_token(raw: &str) -> bool {
    let (left, right) = match raw.split_once(':') {
        Some(parts) => parts,
        None => return false,
    };

    if left.len() < 6 || left.len() > 12 || !left.chars().all(|ch| ch.is_ascii_digit()) {
        return false;
    }
    if right.len() < 12 {
        return false;
    }
    right
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn parse_env_assignment(raw: &str) -> Option<(&str, &str)> {
    let (name, value) = raw.split_once('=')?;
    let name = normalize_assignment_name(name.trim());
    let value = value.trim();
    if name.is_empty() || value.is_empty() {
        return None;
    }
    Some((name, value))
}

fn normalize_assignment_name(raw: &str) -> &str {
    if raw.len() > 7 && raw[..7].eq_ignore_ascii_case("export ") {
        return raw[7..].trim();
    }
    if raw.len() > 4 && raw[..4].eq_ignore_ascii_case("set ") {
        return raw[4..].trim();
    }
    raw
}

fn parse_percent_wrapped_env_name(raw: &str) -> Option<&str> {
    if raw.len() < 2 {
        return None;
    }
    let body = raw.strip_prefix('%')?.strip_suffix('%')?;
    Some(body.trim())
}

fn normalize_dollar_prefixed_env_name(raw: &str, fallback: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return fallback.to_owned();
    }
    if let Some(inner) = trimmed
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
    {
        let inner = inner.trim();
        if !inner.is_empty() {
            return inner.to_owned();
        }
    }
    trimmed.to_owned()
}

enum InlineSecretInput<'a> {
    Literal(&'a str),
    EnvReference(&'a str),
}

fn classify_inline_secret_input(raw: &str) -> Option<InlineSecretInput<'_>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(env_key) = parse_explicit_env_reference(trimmed) {
        return Some(InlineSecretInput::EnvReference(env_key));
    }
    Some(InlineSecretInput::Literal(trimmed))
}

pub(super) fn parse_explicit_env_reference(raw: &str) -> Option<&str> {
    parse_dollar_env_reference(raw)
        .or_else(|| parse_env_prefix_reference(raw))
        .or_else(|| parse_percent_env_reference(raw))
}

fn parse_dollar_env_reference(raw: &str) -> Option<&str> {
    let stripped = raw.trim().strip_prefix('$')?.trim();
    if stripped.is_empty() {
        return None;
    }
    let candidate = stripped
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
        .map(str::trim)
        .unwrap_or(stripped);
    looks_like_compatible_env_name(candidate).then_some(candidate)
}

fn parse_env_prefix_reference(raw: &str) -> Option<&str> {
    if raw.len() < 4 || !raw[..4].eq_ignore_ascii_case("env:") {
        return None;
    }
    let candidate = raw[4..].trim();
    looks_like_compatible_env_name(candidate).then_some(candidate)
}

fn parse_percent_env_reference(raw: &str) -> Option<&str> {
    let candidate = parse_percent_wrapped_env_name(raw)?;
    looks_like_compatible_env_name(candidate).then_some(candidate)
}

fn read_non_empty_env_value(key: &str) -> Option<String> {
    let trimmed_key = key.trim();
    if trimmed_key.is_empty() {
        return None;
    }
    let value = env::var(trimmed_key).ok()?;
    let trimmed_value = value.trim();
    if trimmed_value.is_empty() {
        return None;
    }
    Some(trimmed_value.to_owned())
}

pub(super) fn read_secret_prefer_inline(
    inline: Option<&str>,
    env_key: Option<&str>,
) -> Option<String> {
    if let Some(raw) = inline {
        match classify_inline_secret_input(raw) {
            Some(InlineSecretInput::Literal(value)) => return Some(value.to_owned()),
            Some(InlineSecretInput::EnvReference(key)) => return read_non_empty_env_value(key),
            None => {}
        }
    }
    env_key.and_then(read_non_empty_env_value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_template_interpolation_replaces_known_placeholders() {
        let mut variables = BTreeMap::new();
        variables.insert(
            "code".to_owned(),
            "config.env_pointer.assignment".to_owned(),
        );
        variables.insert("field_path".to_owned(), "provider.api_key_env".to_owned());
        variables.insert("suggested_env_name".to_owned(), "OPENAI_API_KEY".to_owned());
        let rendered = render_message_template(
            "[{code}] use `{field_path}` with `{suggested_env_name}`",
            &variables,
        );
        assert!(rendered.contains("config.env_pointer.assignment"));
        assert!(rendered.contains("provider.api_key_env"));
        assert!(rendered.contains("OPENAI_API_KEY"));
    }

    #[test]
    fn env_pointer_issue_render_uses_catalog_title_and_template() {
        let issue = ConfigValidationIssue {
            code: ConfigValidationCode::DollarPrefix,
            field_path: "provider.api_key_env".to_owned(),
            inline_field_path: "provider.api_key".to_owned(),
            example_env_name: "OPENAI_API_KEY".to_owned(),
            suggested_env_name: Some("OPENAI_API_KEY".to_owned()),
            extra_message_variables: BTreeMap::new(),
        };
        assert_eq!(
            issue.title(ConfigValidationLocale::En),
            "Dollar Prefix Used In Env Pointer"
        );
        let rendered = issue.render(ConfigValidationLocale::En);
        assert!(rendered.contains("without `$`"));
        assert!(rendered.contains("provider.api_key_env"));
        assert!(rendered.contains("OPENAI_API_KEY"));
    }

    #[test]
    fn env_pointer_assignment_normalization_handles_export_and_set() {
        assert_eq!(
            parse_env_assignment("export OPENAI_API_KEY=sk-value"),
            Some(("OPENAI_API_KEY", "sk-value"))
        );
        assert_eq!(
            parse_env_assignment("set OPENAI_API_KEY=sk-value"),
            Some(("OPENAI_API_KEY", "sk-value"))
        );
    }

    #[test]
    fn numeric_range_validation_reports_actual_and_bounds() {
        let issue = validate_numeric_range("memory.sliding_window", 129, 1, 128)
            .expect_err("out-of-range values should be rejected");
        let rendered = issue.render(ConfigValidationLocale::En);
        assert!(rendered.contains("memory.sliding_window"));
        assert!(rendered.contains("between 1 and 128"));
        assert!(rendered.contains("129"));
    }

    /// Deterministic regression test for the user-home fallback logic without
    /// mutating process-global environment variables.
    #[test]
    fn get_user_home_deterministic_fallback_scenarios() {
        let home = if cfg!(windows) {
            PathBuf::from(r"C:\Users\loongclaw-test-home")
        } else {
            PathBuf::from("/tmp/loongclaw-test-home")
        };
        let userprofile = PathBuf::from(if cfg!(windows) {
            r"C:\Users\loongclaw-test-synthetic"
        } else {
            "/tmp/loongclaw-test-synthetic"
        });

        let result_home = resolve_user_home(Some(home.as_os_str()), Some(userprofile.as_os_str()));
        let result_userprofile = resolve_user_home(None, Some(userprofile.as_os_str()));
        let result_dot = resolve_user_home(None, None);

        assert_eq!(
            result_home, home,
            "resolve_user_home() should prefer HOME when both values are present"
        );
        assert_eq!(
            result_userprofile, userprofile,
            "get_user_home() should fall back to USERPROFILE when HOME is absent"
        );
        assert_eq!(
            result_dot,
            PathBuf::from("."),
            "get_user_home() should return \".\" when both HOME and USERPROFILE are absent"
        );
    }
}
