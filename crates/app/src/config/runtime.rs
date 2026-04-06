use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    path::PathBuf,
};

use serde::{Deserialize, Serialize};

use crate::CliResult;
use crate::mcp::McpConfig;
use crate::secrets::DefaultSecretResolver;
use loongclaw_contracts::{SecretRef, SecretResolver};

use super::{
    OnebotChannelConfig, QqbotChannelConfig, WeixinChannelConfig,
    audit::AuditConfig,
    channels::{
        CliChannelConfig, DingtalkChannelConfig, DiscordChannelConfig, EmailChannelConfig,
        FeishuChannelConfig, GoogleChatChannelConfig, ImessageChannelConfig, IrcChannelConfig,
        LineChannelConfig, MatrixChannelConfig, MattermostChannelConfig,
        NextcloudTalkChannelConfig, NostrChannelConfig, SignalChannelConfig, SlackChannelConfig,
        SynologyChatChannelConfig, TeamsChannelConfig, TelegramChannelConfig, TlonChannelConfig,
        TwitchChannelConfig, WebhookChannelConfig, WecomChannelConfig, WhatsappChannelConfig,
    },
    conversation::ConversationConfig,
    feishu_integration::FeishuIntegrationConfig,
    memory::MemoryConfig,
    outbound_http::OutboundHttpConfig,
    provider::{ProviderConfig, ProviderKind, ProviderProfileConfig},
    shared::{
        ConfigValidationIssue, ConfigValidationLocale, ConfigValidationSeverity,
        DEFAULT_CONFIG_FILE, default_loongclaw_home as shared_default_loongclaw_home, expand_path,
        format_config_validation_issues,
    },
    tools::{
        DEFAULT_WEB_SEARCH_PROVIDER, ExternalSkillsConfig, RuntimePluginsConfig, ToolConfig,
        WEB_SEARCH_BRAVE_API_KEY_ENV, WEB_SEARCH_EXA_API_KEY_ENV,
        WEB_SEARCH_FIRECRAWL_API_KEY_ENV, WEB_SEARCH_JINA_API_KEY_ENV,
        WEB_SEARCH_JINA_AUTH_TOKEN_ENV, WEB_SEARCH_PERPLEXITY_API_KEY_ENV,
        WEB_SEARCH_PROVIDER_VALID_VALUES, WEB_SEARCH_TAVILY_API_KEY_ENV,
    },
};
use crate::secrets::{canonicalize_env_secret_reference, secret_ref_env_name};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigValidationDiagnostic {
    pub severity: String,
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
            severity: issue.severity_str().to_owned(),
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct LoongClawConfig {
    #[serde(default, skip_serializing)]
    pub provider: ProviderConfig,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub providers: BTreeMap<String, ProviderProfileConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_provider: Option<String>,
    #[serde(default)]
    pub cli: CliChannelConfig,
    #[serde(default)]
    pub telegram: TelegramChannelConfig,
    #[serde(default)]
    pub feishu: FeishuChannelConfig,
    #[serde(default)]
    pub matrix: MatrixChannelConfig,
    #[serde(default)]
    pub wecom: WecomChannelConfig,
    #[serde(default)]
    pub weixin: WeixinChannelConfig,
    #[serde(default)]
    pub qqbot: QqbotChannelConfig,
    #[serde(default)]
    pub onebot: OnebotChannelConfig,
    #[serde(default)]
    pub discord: DiscordChannelConfig,
    #[serde(default)]
    pub line: LineChannelConfig,
    #[serde(default)]
    pub dingtalk: DingtalkChannelConfig,
    #[serde(default)]
    pub webhook: WebhookChannelConfig,
    #[serde(default)]
    pub slack: SlackChannelConfig,
    #[serde(default)]
    pub google_chat: GoogleChatChannelConfig,
    #[serde(default)]
    pub mattermost: MattermostChannelConfig,
    #[serde(default)]
    pub nextcloud_talk: NextcloudTalkChannelConfig,
    #[serde(default)]
    pub synology_chat: SynologyChatChannelConfig,
    #[serde(default)]
    pub irc: IrcChannelConfig,
    #[serde(default)]
    pub signal: SignalChannelConfig,
    #[serde(default)]
    pub twitch: TwitchChannelConfig,
    #[serde(default)]
    pub teams: TeamsChannelConfig,
    #[serde(default)]
    pub tlon: TlonChannelConfig,
    #[serde(default)]
    pub imessage: ImessageChannelConfig,
    #[serde(default)]
    pub nostr: NostrChannelConfig,
    #[serde(default)]
    pub whatsapp: WhatsappChannelConfig,
    #[serde(default)]
    pub email: EmailChannelConfig,
    #[serde(default)]
    pub feishu_integration: FeishuIntegrationConfig,
    #[serde(default)]
    pub conversation: ConversationConfig,
    #[serde(default, skip_serializing_if = "OutboundHttpConfig::is_default")]
    pub outbound_http: OutboundHttpConfig,
    #[serde(default)]
    pub tools: ToolConfig,
    #[serde(default)]
    pub external_skills: ExternalSkillsConfig,
    #[serde(default)]
    pub runtime_plugins: RuntimePluginsConfig,
    #[serde(default)]
    pub mcp: McpConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub audit: AuditConfig,
    #[serde(default, skip_serializing_if = "ControlPlaneConfig::is_default")]
    pub control_plane: ControlPlaneConfig,
    #[serde(default)]
    pub acp: AcpConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ControlPlaneConfig {
    #[serde(default)]
    pub allow_remote: bool,
    #[serde(default)]
    pub shared_token: Option<SecretRef>,
}

impl ControlPlaneConfig {
    fn is_default(config: &Self) -> bool {
        *config == Self::default()
    }

    pub fn resolved_shared_token(&self) -> CliResult<Option<String>> {
        let Some(secret_ref) = self.shared_token.as_ref() else {
            return Ok(None);
        };
        let resolver = DefaultSecretResolver::default();
        let resolved_value = resolver
            .resolve(secret_ref)
            .map_err(|error| format!("resolve control-plane shared token failed: {error}"))?;
        let shared_token = resolved_value.map(|secret| secret.into_inner());
        Ok(shared_token)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AcpConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub dispatch: AcpDispatchConfig,
    #[serde(default)]
    pub default_agent: Option<String>,
    #[serde(default)]
    pub allowed_agents: Vec<String>,
    #[serde(default)]
    pub max_concurrent_sessions: Option<usize>,
    #[serde(default)]
    pub session_idle_ttl_ms: Option<u64>,
    #[serde(default)]
    pub startup_timeout_ms: Option<u64>,
    #[serde(default)]
    pub turn_timeout_ms: Option<u64>,
    #[serde(default)]
    pub queue_owner_ttl_ms: Option<u64>,
    #[serde(default)]
    pub bindings_enabled: bool,
    #[serde(default)]
    pub emit_runtime_events: bool,
    #[serde(default)]
    pub allow_mcp_server_injection: bool,
    #[serde(default)]
    pub backends: AcpBackendProfilesConfig,
}

impl AcpConfig {
    pub fn backend_id(&self) -> Option<String> {
        self.backend
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase())
    }

    pub fn dispatch_enabled(&self) -> bool {
        self.enabled && self.dispatch.enabled
    }

    pub fn max_concurrent_sessions(&self) -> usize {
        self.max_concurrent_sessions
            .filter(|value| *value > 0)
            .unwrap_or_else(default_acp_max_concurrent_sessions)
    }

    pub fn resolved_default_agent(&self) -> CliResult<String> {
        let raw = normalize_optional_string(self.default_agent.as_deref())
            .unwrap_or_else(|| "codex".to_owned());
        normalize_acp_agent_id(raw.as_str()).ok_or_else(|| {
            format!("ACP default agent `{raw}` is invalid; use letters, numbers, `-`, or `_`")
        })
    }

    pub fn allowed_agent_ids(&self) -> CliResult<Vec<String>> {
        let default_agent = self.resolved_default_agent()?;
        if self.allowed_agents.is_empty() {
            return Ok(vec![default_agent]);
        }

        let mut seen = BTreeSet::new();
        let mut agents = Vec::new();
        for raw in &self.allowed_agents {
            let trimmed = raw.trim();
            let normalized = normalize_acp_agent_id(trimmed).ok_or_else(|| {
                format!(
                    "ACP allowed agent `{trimmed}` is invalid; use letters, numbers, `-`, or `_`"
                )
            })?;
            if seen.insert(normalized.clone()) {
                agents.push(normalized);
            }
        }

        if !agents.iter().any(|agent| agent == &default_agent) {
            return Err(format!(
                "ACP default agent `{default_agent}` must be included in allowed_agents"
            ));
        }

        Ok(agents)
    }

    pub fn resolve_allowed_agent(&self, raw: &str) -> CliResult<String> {
        let normalized = normalize_acp_agent_id(raw).ok_or_else(|| {
            format!("ACP agent `{raw}` is invalid; use letters, numbers, `-`, or `_`")
        })?;
        let allowed = self.allowed_agent_ids()?;
        if allowed.iter().any(|agent| agent == &normalized) {
            return Ok(normalized);
        }
        Err(format!(
            "ACP agent `{normalized}` is not in the allowed ACP agents ({})",
            allowed.join(", ")
        ))
    }

    pub fn session_idle_ttl_ms(&self) -> u64 {
        self.session_idle_ttl_ms
            .filter(|value| *value > 0)
            .unwrap_or_else(default_acp_session_idle_ttl_ms)
    }

    pub fn startup_timeout_ms(&self) -> u64 {
        self.startup_timeout_ms
            .filter(|value| *value > 0)
            .unwrap_or_else(default_acp_startup_timeout_ms)
    }

    pub fn turn_timeout_ms(&self) -> u64 {
        self.turn_timeout_ms
            .filter(|value| *value > 0)
            .unwrap_or_else(default_acp_turn_timeout_ms)
    }

    pub fn queue_owner_ttl_ms(&self) -> u64 {
        self.queue_owner_ttl_ms
            .filter(|value| *value > 0)
            .unwrap_or_else(default_acp_queue_owner_ttl_ms)
    }

    pub fn acpx_profile(&self) -> Option<&AcpxBackendConfig> {
        self.backends.acpx.as_ref()
    }
}

impl Default for AcpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: None,
            dispatch: AcpDispatchConfig::default(),
            default_agent: None,
            allowed_agents: Vec::new(),
            max_concurrent_sessions: Some(default_acp_max_concurrent_sessions()),
            session_idle_ttl_ms: Some(default_acp_session_idle_ttl_ms()),
            startup_timeout_ms: Some(default_acp_startup_timeout_ms()),
            turn_timeout_ms: Some(default_acp_turn_timeout_ms()),
            queue_owner_ttl_ms: Some(default_acp_queue_owner_ttl_ms()),
            bindings_enabled: false,
            emit_runtime_events: false,
            allow_mcp_server_injection: false,
            backends: AcpBackendProfilesConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcpDispatchConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub conversation_routing: AcpConversationRoutingMode,
    #[serde(default)]
    pub allowed_channels: Vec<String>,
    #[serde(default)]
    pub allowed_account_ids: Vec<String>,
    #[serde(default)]
    pub bootstrap_mcp_servers: Vec<String>,
    #[serde(default)]
    pub working_directory: Option<String>,
    #[serde(default)]
    pub thread_routing: AcpDispatchThreadRoutingMode,
}

impl Default for AcpDispatchConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            conversation_routing: AcpConversationRoutingMode::default(),
            allowed_channels: Vec::new(),
            allowed_account_ids: Vec::new(),
            bootstrap_mcp_servers: Vec::new(),
            working_directory: None,
            thread_routing: AcpDispatchThreadRoutingMode::default(),
        }
    }
}

impl AcpDispatchConfig {
    pub fn allowed_channel_ids(&self) -> CliResult<Vec<String>> {
        let mut seen = BTreeSet::new();
        let mut channels = Vec::new();
        for raw in &self.allowed_channels {
            let trimmed = raw.trim();
            let normalized = normalize_dispatch_channel_id(trimmed).ok_or_else(|| {
                format!(
                    "ACP dispatch allowed channel `{trimmed}` is invalid; use letters, numbers, `-`, or `_`"
                )
            })?;
            if seen.insert(normalized.clone()) {
                channels.push(normalized);
            }
        }
        Ok(channels)
    }

    pub fn allows_channel_id(&self, channel_id: Option<&str>) -> CliResult<bool> {
        let allowed = self.allowed_channel_ids()?;
        if allowed.is_empty() {
            return Ok(true);
        }
        let Some(channel_id) = channel_id.and_then(normalize_dispatch_channel_id) else {
            return Ok(false);
        };
        Ok(allowed.iter().any(|channel| channel == &channel_id))
    }

    pub fn allowed_account_ids(&self) -> CliResult<Vec<String>> {
        let mut seen = BTreeSet::new();
        let mut accounts = Vec::new();
        for raw in &self.allowed_account_ids {
            let trimmed = raw.trim();
            let normalized = normalize_dispatch_account_id(trimmed).ok_or_else(|| {
                format!(
                    "ACP dispatch allowed account `{trimmed}` is invalid; use a configured account identity or label"
                )
            })?;
            if seen.insert(normalized.clone()) {
                accounts.push(normalized);
            }
        }
        Ok(accounts)
    }

    pub fn bootstrap_mcp_server_names(&self) -> CliResult<Vec<String>> {
        self.bootstrap_mcp_server_names_with_additions(&[])
    }

    pub fn resolved_working_directory(&self) -> Option<PathBuf> {
        self.working_directory
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(Path::new)
            .map(Path::to_path_buf)
    }

    pub fn bootstrap_mcp_server_names_with_additions(
        &self,
        additional: &[String],
    ) -> CliResult<Vec<String>> {
        let mut seen = BTreeSet::new();
        let mut names = Vec::new();
        for raw in self
            .bootstrap_mcp_servers
            .iter()
            .map(String::as_str)
            .chain(additional.iter().map(String::as_str))
        {
            let Some(normalized) =
                normalize_optional_string(Some(raw)).map(|value| value.to_ascii_lowercase())
            else {
                return Err(
                    "ACP dispatch bootstrap MCP server names must not contain empty entries"
                        .to_owned(),
                );
            };
            if seen.insert(normalized.clone()) {
                names.push(normalized);
            }
        }
        Ok(names)
    }

    pub fn allows_account_id(&self, account_id: Option<&str>) -> CliResult<bool> {
        let allowed = self.allowed_account_ids()?;
        if allowed.is_empty() {
            return Ok(true);
        }
        let Some(account_id) = account_id.and_then(normalize_dispatch_account_id) else {
            return Ok(false);
        };
        Ok(allowed.iter().any(|candidate| candidate == &account_id))
    }

    pub fn allows_thread_id(&self, thread_id: Option<&str>) -> bool {
        self.thread_routing.allows_thread_id(thread_id)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AcpConversationRoutingMode {
    #[default]
    AgentPrefixedOnly,
    All,
}

impl AcpConversationRoutingMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::AgentPrefixedOnly => "agent_prefixed_only",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AcpDispatchThreadRoutingMode {
    #[default]
    All,
    ThreadOnly,
    RootOnly,
}

impl AcpDispatchThreadRoutingMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::ThreadOnly => "thread_only",
            Self::RootOnly => "root_only",
        }
    }

    pub fn allows_thread_id(self, thread_id: Option<&str>) -> bool {
        let has_thread = thread_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some();
        match self {
            Self::All => true,
            Self::ThreadOnly => has_thread,
            Self::RootOnly => !has_thread,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct AcpBackendProfilesConfig {
    #[serde(default)]
    pub acpx: Option<AcpxBackendConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AcpxBackendConfig {
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub expected_version: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub non_interactive_permissions: Option<String>,
    #[serde(default)]
    pub strict_windows_cmd_wrapper: Option<bool>,
    #[serde(default)]
    pub timeout_seconds: Option<f64>,
    #[serde(default)]
    pub queue_owner_ttl_seconds: Option<f64>,
    #[serde(default)]
    pub mcp_servers: BTreeMap<String, AcpxMcpServerConfig>,
}

impl AcpxBackendConfig {
    pub fn command(&self) -> Option<String> {
        normalize_optional_string(self.command.as_deref())
    }

    pub fn expected_version(&self) -> Option<String> {
        normalize_optional_string(self.expected_version.as_deref())
    }

    pub fn cwd(&self) -> Option<String> {
        normalize_optional_string(self.cwd.as_deref())
    }

    pub fn permission_mode(&self) -> Option<String> {
        normalize_optional_string(self.permission_mode.as_deref())
    }

    pub fn non_interactive_permissions(&self) -> Option<String> {
        normalize_optional_string(self.non_interactive_permissions.as_deref())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcpxMcpServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

const fn default_acp_max_concurrent_sessions() -> usize {
    8
}

const fn default_true() -> bool {
    true
}

const fn default_acp_session_idle_ttl_ms() -> u64 {
    900_000
}

const fn default_acp_startup_timeout_ms() -> u64 {
    15_000
}

const fn default_acp_turn_timeout_ms() -> u64 {
    120_000
}

const fn default_acp_queue_owner_ttl_ms() -> u64 {
    30_000
}

fn normalize_optional_string(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_owned())
}

fn normalize_provider_profile_id(raw: &str) -> Option<String> {
    normalize_dispatch_channel_id(raw)
}

#[derive(Debug, Clone, Default)]
struct RawProviderSelectionIntent {
    legacy_provider_explicit: bool,
    active_provider_explicit: bool,
    raw_active_provider: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveProviderSelectionBasis {
    ExplicitActiveProvider,
    ExplicitLegacyProvider,
    FirstSavedProfile,
    LegacyOnly,
}

impl ActiveProviderSelectionBasis {
    const fn diagnostic_summary(self) -> &'static str {
        match self {
            Self::ExplicitActiveProvider => "the explicit active_provider value",
            Self::ExplicitLegacyProvider => "the explicit legacy [provider] table",
            Self::FirstSavedProfile => "the first saved provider profile in sorted order",
            Self::LegacyOnly => "the legacy [provider] table",
        }
    }
}

#[derive(Debug, Clone, Default)]
struct ProviderSelectionNormalizationReport {
    legacy_provider_explicit: bool,
    active_provider_explicit: bool,
    requested_active_provider: Option<String>,
    selected_active_provider: Option<String>,
    configured_profile_ids: Vec<String>,
    selection_basis: Option<ActiveProviderSelectionBasis>,
    warn_implicit_active_provider: bool,
    warn_unknown_active_provider: bool,
    recovered_legacy_profile_id: Option<String>,
    legacy_profile_inserted: bool,
    legacy_provider_validation_issues: Vec<ConfigValidationIssue>,
}

impl ProviderSelectionNormalizationReport {
    fn validation_issues(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = self.legacy_provider_validation_issues.clone();
        let configured_profile_ids = self.configured_profile_ids.join(", ");
        let selected_profile_id = self
            .selected_active_provider
            .as_deref()
            .unwrap_or("unknown")
            .to_owned();
        let selection_basis = self
            .selection_basis
            .map(ActiveProviderSelectionBasis::diagnostic_summary)
            .unwrap_or("provider profile normalization")
            .to_owned();

        if self.warn_implicit_active_provider {
            let mut extra_message_variables = BTreeMap::new();
            extra_message_variables.insert("selected_profile_id".to_owned(), selected_profile_id);
            extra_message_variables.insert("selection_basis".to_owned(), selection_basis.clone());
            extra_message_variables.insert(
                "configured_profile_ids".to_owned(),
                configured_profile_ids.clone(),
            );
            issues.push(ConfigValidationIssue {
                severity: ConfigValidationSeverity::Warn,
                code: super::shared::ConfigValidationCode::ImplicitActiveProvider,
                field_path: "active_provider".to_owned(),
                inline_field_path: "providers".to_owned(),
                example_env_name: String::new(),
                suggested_env_name: self.selected_active_provider.clone(),
                extra_message_variables,
            });
        }

        if self.warn_unknown_active_provider {
            let mut extra_message_variables = BTreeMap::new();
            extra_message_variables.insert(
                "requested_profile_id".to_owned(),
                self.requested_active_provider
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("(blank)")
                    .to_owned(),
            );
            extra_message_variables.insert(
                "selected_profile_id".to_owned(),
                self.selected_active_provider
                    .clone()
                    .unwrap_or_else(|| "unknown".to_owned()),
            );
            extra_message_variables.insert("selection_basis".to_owned(), selection_basis);
            extra_message_variables
                .insert("configured_profile_ids".to_owned(), configured_profile_ids);
            issues.push(ConfigValidationIssue {
                severity: ConfigValidationSeverity::Warn,
                code: super::shared::ConfigValidationCode::UnknownActiveProvider,
                field_path: "active_provider".to_owned(),
                inline_field_path: "providers".to_owned(),
                example_env_name: String::new(),
                suggested_env_name: self.selected_active_provider.clone(),
                extra_message_variables,
            });
        }

        issues
    }
}

pub const PROVIDER_SELECTOR_PLACEHOLDER: &str = "<profile|model|kind>";
pub const PROVIDER_SELECTOR_HUMAN_SUMMARY: &str =
    "profile id, unique model name or suffix, or provider kind";
pub const PROVIDER_SELECTOR_TARGET_SUMMARY: &str =
    "target profile id, unique model name or suffix, or provider kind";
pub const PROVIDER_SELECTOR_NOTE: &str =
    "you can also enter a unique model name, model suffix, or provider kind";
pub const PROVIDER_SELECTOR_COMPACT_NOTE: &str = "type a model, suffix, or provider kind";

fn normalize_provider_selector_token(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_ascii_lowercase())
}

fn provider_model_suffix(raw: &str) -> Option<String> {
    let normalized = normalize_provider_selector_token(raw)?;
    normalized
        .rsplit('/')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn push_unique_selector(selectors: &mut Vec<String>, candidate: &str) {
    if candidate.trim().is_empty() {
        return;
    }
    if selectors
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(candidate))
    {
        return;
    }
    selectors.push(candidate.to_owned());
}

#[derive(Debug, Clone, Copy)]
pub struct ProviderSelectorProfileRef<'a> {
    pub profile_id: &'a str,
    pub kind: ProviderKind,
    pub model: &'a str,
    pub default_for_kind: bool,
}

impl<'a> ProviderSelectorProfileRef<'a> {
    pub const fn new(
        profile_id: &'a str,
        kind: ProviderKind,
        model: &'a str,
        default_for_kind: bool,
    ) -> Self {
        Self {
            profile_id,
            kind,
            model,
            default_for_kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderSelectorResolution {
    Match(String),
    Ambiguous(Vec<String>),
    NoMatch,
}

pub fn accepted_provider_selectors<'a, I>(profiles: I, target_profile_id: &str) -> Vec<String>
where
    I: IntoIterator<Item = ProviderSelectorProfileRef<'a>>,
{
    ProviderSelectorIndex::new(profiles).accepted_selectors(target_profile_id)
}

pub fn provider_selector_catalog<'a, I>(profiles: I) -> Vec<String>
where
    I: IntoIterator<Item = ProviderSelectorProfileRef<'a>>,
{
    ProviderSelectorIndex::new(profiles).selector_catalog()
}

pub fn preferred_provider_selector<'a, I>(profiles: I, target_profile_id: &str) -> Option<String>
where
    I: IntoIterator<Item = ProviderSelectorProfileRef<'a>>,
{
    ProviderSelectorIndex::new(profiles).preferred_selector(target_profile_id)
}

pub fn describe_provider_selector_target<'a, I>(
    profiles: I,
    target_profile_id: &str,
) -> Option<String>
where
    I: IntoIterator<Item = ProviderSelectorProfileRef<'a>>,
{
    ProviderSelectorIndex::new(profiles).describe_profile(target_profile_id)
}

pub fn provider_selector_recommendation_hint<'a, I, J, S>(
    profiles: I,
    target_profile_ids: J,
) -> Option<String>
where
    I: IntoIterator<Item = ProviderSelectorProfileRef<'a>>,
    J: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    ProviderSelectorIndex::new(profiles).recommendation_hint(target_profile_ids)
}

pub fn resolve_provider_selector<'a, I>(profiles: I, selector: &str) -> ProviderSelectorResolution
where
    I: IntoIterator<Item = ProviderSelectorProfileRef<'a>>,
{
    ProviderSelectorIndex::new(profiles).resolve(selector)
}

struct ProviderSelectorIndex<'a> {
    profiles: Vec<ProviderSelectorProfileRef<'a>>,
}

impl<'a> ProviderSelectorIndex<'a> {
    fn new<I>(profiles: I) -> Self
    where
        I: IntoIterator<Item = ProviderSelectorProfileRef<'a>>,
    {
        Self {
            profiles: profiles.into_iter().collect(),
        }
    }

    fn accepted_selectors(&self, target_profile_id: &str) -> Vec<String> {
        let Some(profile) = self.find_profile(target_profile_id) else {
            return Vec::new();
        };

        let mut selectors = Vec::new();
        push_unique_selector(&mut selectors, profile.profile_id);

        if let Some(model) = normalize_provider_selector_token(profile.model)
            && self.model_matches(model.as_str()).len() == 1
        {
            push_unique_selector(&mut selectors, model.as_str());
        }

        if let Some(suffix) = provider_model_suffix(profile.model)
            && self.model_suffix_matches(suffix.as_str()).len() == 1
        {
            push_unique_selector(&mut selectors, suffix.as_str());
        }

        if self.kind_resolves_to_profile_id(profile.kind, profile.profile_id) {
            push_unique_selector(&mut selectors, profile.kind.as_str());
        }

        selectors
    }

    fn resolve(&self, selector: &str) -> ProviderSelectorResolution {
        let Some(normalized) = normalize_provider_selector_token(selector) else {
            return ProviderSelectorResolution::NoMatch;
        };

        if let Some(profile) = self
            .profiles
            .iter()
            .find(|profile| profile.profile_id.eq_ignore_ascii_case(normalized.as_str()))
        {
            return ProviderSelectorResolution::Match(profile.profile_id.to_owned());
        }

        let model_matches = self.model_matches(normalized.as_str());
        match model_matches.as_slice() {
            [profile_id] => return ProviderSelectorResolution::Match(profile_id.clone()),
            matches if matches.len() > 1 => {
                return ProviderSelectorResolution::Ambiguous(matches.to_vec());
            }
            _ => {}
        }

        let suffix_matches = self.model_suffix_matches(normalized.as_str());
        match suffix_matches.as_slice() {
            [profile_id] => return ProviderSelectorResolution::Match(profile_id.clone()),
            matches if matches.len() > 1 => {
                return ProviderSelectorResolution::Ambiguous(matches.to_vec());
            }
            _ => {}
        }

        let Some(kind) = ProviderKind::parse(normalized.as_str()) else {
            return ProviderSelectorResolution::NoMatch;
        };
        self.resolve_kind(kind)
    }

    fn selector_catalog(&self) -> Vec<String> {
        let mut selectors = Vec::new();
        for profile in &self.profiles {
            for selector in self.accepted_selectors(profile.profile_id) {
                push_unique_selector(&mut selectors, selector.as_str());
            }
        }
        selectors
    }

    fn preferred_selector(&self, target_profile_id: &str) -> Option<String> {
        let profile = self.find_profile(target_profile_id)?;
        let selectors = self.accepted_selectors(target_profile_id);
        if selectors.is_empty() {
            return None;
        }

        let profile_id = normalize_provider_selector_token(profile.profile_id);
        let model = normalize_provider_selector_token(profile.model);
        let suffix = provider_model_suffix(profile.model);
        let kind = normalize_provider_selector_token(profile.kind.as_str());
        let profile_id_len = profile_id.as_ref().map_or(usize::MAX, String::len);

        let preferred_candidates = [
            kind.as_deref(),
            suffix.as_deref(),
            model
                .as_deref()
                .filter(|model| model.len() <= profile_id_len),
            profile_id.as_deref(),
            model.as_deref(),
        ];

        for candidate in preferred_candidates.into_iter().flatten() {
            if let Some(selector) = selectors
                .iter()
                .find(|existing| existing.eq_ignore_ascii_case(candidate))
            {
                return Some(selector.clone());
            }
        }

        selectors.into_iter().next()
    }

    fn describe_profile(&self, target_profile_id: &str) -> Option<String> {
        let profile = self.find_profile(target_profile_id)?;
        let selectors = self.accepted_selectors(target_profile_id);
        let mut description = format!("{} [model={}", profile.profile_id, profile.model);
        if !selectors.is_empty() {
            description.push_str("; selectors=");
            description.push_str(selectors.join(", ").as_str());
        }
        description.push(']');
        Some(description)
    }

    fn recommendation_hint<J, S>(&self, target_profile_ids: J) -> Option<String>
    where
        J: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut selectors = Vec::new();
        for profile_id in target_profile_ids {
            let Some(selector) = self.preferred_selector(profile_id.as_ref()) else {
                continue;
            };
            push_unique_selector(&mut selectors, selector.as_str());
            if selectors.len() >= 3 {
                break;
            }
        }
        (!selectors.is_empty()).then(|| format!("try one of: {}", selectors.join(", ")))
    }

    fn find_profile(&self, profile_id: &str) -> Option<&ProviderSelectorProfileRef<'a>> {
        self.profiles
            .iter()
            .find(|profile| profile.profile_id == profile_id)
    }

    fn model_matches(&self, selector: &str) -> Vec<String> {
        self.profiles
            .iter()
            .filter(|profile| {
                normalize_provider_selector_token(profile.model).as_deref() == Some(selector)
            })
            .map(|profile| profile.profile_id.to_owned())
            .collect()
    }

    fn model_suffix_matches(&self, selector: &str) -> Vec<String> {
        self.profiles
            .iter()
            .filter(|profile| provider_model_suffix(profile.model).as_deref() == Some(selector))
            .map(|profile| profile.profile_id.to_owned())
            .collect()
    }

    fn resolve_kind(&self, kind: ProviderKind) -> ProviderSelectorResolution {
        let matches = self
            .profiles
            .iter()
            .filter(|profile| profile.kind == kind)
            .collect::<Vec<_>>();
        let Some(first) = matches.first().copied() else {
            return ProviderSelectorResolution::NoMatch;
        };
        if matches.len() == 1 {
            return ProviderSelectorResolution::Match(first.profile_id.to_owned());
        }

        let default_matches = matches
            .iter()
            .copied()
            .filter(|profile| profile.default_for_kind)
            .collect::<Vec<_>>();
        if let [default_match] = default_matches.as_slice() {
            return ProviderSelectorResolution::Match(default_match.profile_id.to_owned());
        }

        ProviderSelectorResolution::Ambiguous(
            matches
                .into_iter()
                .map(|profile| profile.profile_id.to_owned())
                .collect(),
        )
    }

    fn kind_resolves_to_profile_id(&self, kind: ProviderKind, profile_id: &str) -> bool {
        matches!(
            self.resolve_kind(kind),
            ProviderSelectorResolution::Match(resolved) if resolved == profile_id
        )
    }
}

fn canonicalize_provider_profile_for_encoding(profile: &mut ProviderProfileConfig) {
    canonicalize_provider_secret_env_reference(
        &mut profile.provider.api_key,
        &mut profile.provider.api_key_env,
    );
    canonicalize_provider_secret_env_reference(
        &mut profile.provider.oauth_access_token,
        &mut profile.provider.oauth_access_token_env,
    );
}

fn canonicalize_channel_configs_for_encoding(config: &mut LoongClawConfig) {
    canonicalize_telegram_channel_for_encoding(&mut config.telegram);
    canonicalize_feishu_channel_for_encoding(&mut config.feishu);
    canonicalize_matrix_channel_for_encoding(&mut config.matrix);
    canonicalize_wecom_channel_for_encoding(&mut config.wecom);
    canonicalize_weixin_channel_for_encoding(&mut config.weixin);
    canonicalize_qqbot_channel_for_encoding(&mut config.qqbot);
    canonicalize_onebot_channel_for_encoding(&mut config.onebot);
    canonicalize_discord_channel_for_encoding(&mut config.discord);
    canonicalize_line_channel_for_encoding(&mut config.line);
    canonicalize_dingtalk_channel_for_encoding(&mut config.dingtalk);
    canonicalize_webhook_channel_for_encoding(&mut config.webhook);
    canonicalize_email_channel_for_encoding(&mut config.email);
    canonicalize_slack_channel_for_encoding(&mut config.slack);
    canonicalize_google_chat_channel_for_encoding(&mut config.google_chat);
    canonicalize_teams_channel_for_encoding(&mut config.teams);
    canonicalize_tlon_channel_for_encoding(&mut config.tlon);
    canonicalize_imessage_channel_for_encoding(&mut config.imessage);
    canonicalize_nostr_channel_for_encoding(&mut config.nostr);
    canonicalize_whatsapp_channel_for_encoding(&mut config.whatsapp);
    canonicalize_mattermost_channel_for_encoding(&mut config.mattermost);
    canonicalize_nextcloud_talk_channel_for_encoding(&mut config.nextcloud_talk);
    canonicalize_synology_chat_channel_for_encoding(&mut config.synology_chat);
    canonicalize_irc_channel_for_encoding(&mut config.irc);
    canonicalize_twitch_channel_for_encoding(&mut config.twitch);
}

fn canonicalize_telegram_channel_for_encoding(config: &mut TelegramChannelConfig) {
    canonicalize_env_secret_reference(&mut config.bot_token, &mut config.bot_token_env);
    for account in config.accounts.values_mut() {
        canonicalize_env_secret_reference(&mut account.bot_token, &mut account.bot_token_env);
    }
}

fn canonicalize_feishu_channel_for_encoding(config: &mut FeishuChannelConfig) {
    canonicalize_env_secret_reference(&mut config.app_id, &mut config.app_id_env);
    canonicalize_env_secret_reference(&mut config.app_secret, &mut config.app_secret_env);
    canonicalize_env_secret_reference(
        &mut config.verification_token,
        &mut config.verification_token_env,
    );
    canonicalize_env_secret_reference(&mut config.encrypt_key, &mut config.encrypt_key_env);

    for account in config.accounts.values_mut() {
        canonicalize_env_secret_reference(&mut account.app_id, &mut account.app_id_env);
        canonicalize_env_secret_reference(&mut account.app_secret, &mut account.app_secret_env);
        canonicalize_env_secret_reference(
            &mut account.verification_token,
            &mut account.verification_token_env,
        );
        canonicalize_env_secret_reference(&mut account.encrypt_key, &mut account.encrypt_key_env);
    }
}

fn canonicalize_matrix_channel_for_encoding(config: &mut MatrixChannelConfig) {
    canonicalize_env_secret_reference(&mut config.access_token, &mut config.access_token_env);
    for account in config.accounts.values_mut() {
        canonicalize_env_secret_reference(&mut account.access_token, &mut account.access_token_env);
    }
}

fn canonicalize_wecom_channel_for_encoding(config: &mut WecomChannelConfig) {
    canonicalize_env_secret_reference(&mut config.bot_id, &mut config.bot_id_env);
    canonicalize_env_secret_reference(&mut config.secret, &mut config.secret_env);

    for account in config.accounts.values_mut() {
        canonicalize_env_secret_reference(&mut account.bot_id, &mut account.bot_id_env);
        canonicalize_env_secret_reference(&mut account.secret, &mut account.secret_env);
    }
}

fn canonicalize_weixin_channel_for_encoding(config: &mut WeixinChannelConfig) {
    canonicalize_env_string_reference(&mut config.bridge_url, &mut config.bridge_url_env);
    canonicalize_env_secret_reference(
        &mut config.bridge_access_token,
        &mut config.bridge_access_token_env,
    );

    for account in config.accounts.values_mut() {
        canonicalize_env_string_reference(&mut account.bridge_url, &mut account.bridge_url_env);
        canonicalize_env_secret_reference(
            &mut account.bridge_access_token,
            &mut account.bridge_access_token_env,
        );
    }
}

fn canonicalize_qqbot_channel_for_encoding(config: &mut QqbotChannelConfig) {
    canonicalize_env_secret_reference(&mut config.app_id, &mut config.app_id_env);
    canonicalize_env_secret_reference(&mut config.client_secret, &mut config.client_secret_env);

    for account in config.accounts.values_mut() {
        canonicalize_env_secret_reference(&mut account.app_id, &mut account.app_id_env);
        canonicalize_env_secret_reference(
            &mut account.client_secret,
            &mut account.client_secret_env,
        );
    }
}

fn canonicalize_onebot_channel_for_encoding(config: &mut OnebotChannelConfig) {
    canonicalize_env_string_reference(&mut config.websocket_url, &mut config.websocket_url_env);
    canonicalize_env_secret_reference(&mut config.access_token, &mut config.access_token_env);

    for account in config.accounts.values_mut() {
        canonicalize_env_string_reference(
            &mut account.websocket_url,
            &mut account.websocket_url_env,
        );
        canonicalize_env_secret_reference(&mut account.access_token, &mut account.access_token_env);
    }
}

fn canonicalize_discord_channel_for_encoding(config: &mut DiscordChannelConfig) {
    canonicalize_env_secret_reference(&mut config.bot_token, &mut config.bot_token_env);

    for account in config.accounts.values_mut() {
        canonicalize_env_secret_reference(&mut account.bot_token, &mut account.bot_token_env);
    }
}

fn canonicalize_line_channel_for_encoding(config: &mut LineChannelConfig) {
    canonicalize_env_secret_reference(
        &mut config.channel_access_token,
        &mut config.channel_access_token_env,
    );
    canonicalize_env_secret_reference(&mut config.channel_secret, &mut config.channel_secret_env);

    for account in config.accounts.values_mut() {
        canonicalize_env_secret_reference(
            &mut account.channel_access_token,
            &mut account.channel_access_token_env,
        );
        canonicalize_env_secret_reference(
            &mut account.channel_secret,
            &mut account.channel_secret_env,
        );
    }
}

fn canonicalize_dingtalk_channel_for_encoding(config: &mut DingtalkChannelConfig) {
    canonicalize_env_secret_reference(&mut config.webhook_url, &mut config.webhook_url_env);
    canonicalize_env_secret_reference(&mut config.secret, &mut config.secret_env);

    for account in config.accounts.values_mut() {
        canonicalize_env_secret_reference(&mut account.webhook_url, &mut account.webhook_url_env);
        canonicalize_env_secret_reference(&mut account.secret, &mut account.secret_env);
    }
}

fn canonicalize_webhook_channel_for_encoding(config: &mut WebhookChannelConfig) {
    canonicalize_env_secret_reference(&mut config.endpoint_url, &mut config.endpoint_url_env);
    canonicalize_env_secret_reference(&mut config.auth_token, &mut config.auth_token_env);
    canonicalize_env_secret_reference(&mut config.signing_secret, &mut config.signing_secret_env);

    for account in config.accounts.values_mut() {
        canonicalize_env_secret_reference(&mut account.endpoint_url, &mut account.endpoint_url_env);
        canonicalize_env_secret_reference(&mut account.auth_token, &mut account.auth_token_env);
        canonicalize_env_secret_reference(
            &mut account.signing_secret,
            &mut account.signing_secret_env,
        );
    }
}

fn canonicalize_email_channel_for_encoding(config: &mut EmailChannelConfig) {
    canonicalize_env_secret_reference(&mut config.smtp_username, &mut config.smtp_username_env);
    canonicalize_env_secret_reference(&mut config.smtp_password, &mut config.smtp_password_env);
    canonicalize_env_secret_reference(&mut config.imap_username, &mut config.imap_username_env);
    canonicalize_env_secret_reference(&mut config.imap_password, &mut config.imap_password_env);

    for account in config.accounts.values_mut() {
        canonicalize_env_secret_reference(
            &mut account.smtp_username,
            &mut account.smtp_username_env,
        );
        canonicalize_env_secret_reference(
            &mut account.smtp_password,
            &mut account.smtp_password_env,
        );
        canonicalize_env_secret_reference(
            &mut account.imap_username,
            &mut account.imap_username_env,
        );
        canonicalize_env_secret_reference(
            &mut account.imap_password,
            &mut account.imap_password_env,
        );
    }
}

fn canonicalize_slack_channel_for_encoding(config: &mut SlackChannelConfig) {
    canonicalize_env_secret_reference(&mut config.bot_token, &mut config.bot_token_env);

    for account in config.accounts.values_mut() {
        canonicalize_env_secret_reference(&mut account.bot_token, &mut account.bot_token_env);
    }
}

fn canonicalize_google_chat_channel_for_encoding(config: &mut GoogleChatChannelConfig) {
    canonicalize_env_secret_reference(&mut config.webhook_url, &mut config.webhook_url_env);

    for account in config.accounts.values_mut() {
        canonicalize_env_secret_reference(&mut account.webhook_url, &mut account.webhook_url_env);
    }
}

fn canonicalize_teams_channel_for_encoding(config: &mut TeamsChannelConfig) {
    canonicalize_env_secret_reference(&mut config.webhook_url, &mut config.webhook_url_env);
    canonicalize_env_secret_reference(&mut config.app_id, &mut config.app_id_env);
    canonicalize_env_secret_reference(&mut config.app_password, &mut config.app_password_env);

    for account in config.accounts.values_mut() {
        canonicalize_env_secret_reference(&mut account.webhook_url, &mut account.webhook_url_env);
        canonicalize_env_secret_reference(&mut account.app_id, &mut account.app_id_env);
        canonicalize_env_secret_reference(&mut account.app_password, &mut account.app_password_env);
    }
}

fn canonicalize_tlon_channel_for_encoding(config: &mut TlonChannelConfig) {
    canonicalize_optional_env_name(&mut config.ship_env);
    canonicalize_optional_env_name(&mut config.url_env);
    canonicalize_env_secret_reference(&mut config.code, &mut config.code_env);

    for account in config.accounts.values_mut() {
        canonicalize_optional_env_name(&mut account.ship_env);
        canonicalize_optional_env_name(&mut account.url_env);
        canonicalize_env_secret_reference(&mut account.code, &mut account.code_env);
    }
}

fn canonicalize_imessage_channel_for_encoding(config: &mut ImessageChannelConfig) {
    canonicalize_env_secret_reference(&mut config.bridge_token, &mut config.bridge_token_env);

    for account in config.accounts.values_mut() {
        canonicalize_env_secret_reference(&mut account.bridge_token, &mut account.bridge_token_env);
    }
}

fn canonicalize_nostr_channel_for_encoding(config: &mut NostrChannelConfig) {
    canonicalize_optional_env_name(&mut config.relay_urls_env);
    canonicalize_env_secret_reference(&mut config.private_key, &mut config.private_key_env);

    for account in config.accounts.values_mut() {
        canonicalize_optional_env_name(&mut account.relay_urls_env);
        canonicalize_env_secret_reference(&mut account.private_key, &mut account.private_key_env);
    }
}

fn canonicalize_whatsapp_channel_for_encoding(config: &mut WhatsappChannelConfig) {
    canonicalize_env_secret_reference(&mut config.access_token, &mut config.access_token_env);
    canonicalize_env_secret_reference(&mut config.verify_token, &mut config.verify_token_env);
    canonicalize_env_secret_reference(&mut config.app_secret, &mut config.app_secret_env);

    for account in config.accounts.values_mut() {
        canonicalize_env_secret_reference(&mut account.access_token, &mut account.access_token_env);
        canonicalize_env_secret_reference(&mut account.verify_token, &mut account.verify_token_env);
        canonicalize_env_secret_reference(&mut account.app_secret, &mut account.app_secret_env);
    }
}

fn canonicalize_mattermost_channel_for_encoding(config: &mut MattermostChannelConfig) {
    canonicalize_env_secret_reference(&mut config.bot_token, &mut config.bot_token_env);

    for account in config.accounts.values_mut() {
        canonicalize_env_secret_reference(&mut account.bot_token, &mut account.bot_token_env);
    }
}

fn canonicalize_nextcloud_talk_channel_for_encoding(config: &mut NextcloudTalkChannelConfig) {
    canonicalize_env_secret_reference(&mut config.shared_secret, &mut config.shared_secret_env);

    for account in config.accounts.values_mut() {
        canonicalize_env_secret_reference(
            &mut account.shared_secret,
            &mut account.shared_secret_env,
        );
    }
}

fn canonicalize_synology_chat_channel_for_encoding(config: &mut SynologyChatChannelConfig) {
    canonicalize_env_secret_reference(&mut config.token, &mut config.token_env);
    canonicalize_env_secret_reference(&mut config.incoming_url, &mut config.incoming_url_env);

    for account in config.accounts.values_mut() {
        canonicalize_env_secret_reference(&mut account.token, &mut account.token_env);
        canonicalize_env_secret_reference(&mut account.incoming_url, &mut account.incoming_url_env);
    }
}

fn canonicalize_irc_channel_for_encoding(config: &mut IrcChannelConfig) {
    canonicalize_optional_env_name(&mut config.server_env);
    canonicalize_optional_env_name(&mut config.nickname_env);
    canonicalize_env_secret_reference(&mut config.password, &mut config.password_env);

    for account in config.accounts.values_mut() {
        canonicalize_optional_env_name(&mut account.server_env);
        canonicalize_optional_env_name(&mut account.nickname_env);
        canonicalize_env_secret_reference(&mut account.password, &mut account.password_env);
    }
}

fn canonicalize_twitch_channel_for_encoding(config: &mut TwitchChannelConfig) {
    canonicalize_env_secret_reference(&mut config.access_token, &mut config.access_token_env);
    for account in config.accounts.values_mut() {
        canonicalize_env_secret_reference(&mut account.access_token, &mut account.access_token_env);
    }
}

fn canonicalize_optional_env_name(env_name: &mut Option<String>) {
    let normalized_env_name = env_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    *env_name = normalized_env_name;
}

fn canonicalize_provider_secret_env_reference(
    inline_secret: &mut Option<loongclaw_contracts::SecretRef>,
    env_name: &mut Option<String>,
) {
    if let Some(explicit_env_name) = secret_ref_env_name(inline_secret.as_ref()) {
        *inline_secret = Some(loongclaw_contracts::SecretRef::Env {
            env: explicit_env_name,
        });
        *env_name = None;
        return;
    }

    if inline_secret
        .as_ref()
        .is_some_and(loongclaw_contracts::SecretRef::is_configured)
    {
        *env_name = None;
        return;
    }

    let normalized_env_name = env_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if let Some(normalized_env_name) = normalized_env_name {
        *inline_secret = Some(loongclaw_contracts::SecretRef::Env {
            env: normalized_env_name,
        });
    }
    *env_name = None;
}

fn canonicalize_env_string_reference(value: &mut Option<String>, env_name: &mut Option<String>) {
    let explicit_env_name = value
        .as_deref()
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
        .and_then(|raw| {
            let secret_ref = loongclaw_contracts::SecretRef::Inline(raw.to_owned());
            secret_ref_env_name(Some(&secret_ref))
        });
    let Some(explicit_env_name) = explicit_env_name else {
        return;
    };

    let configured_env_name = env_name
        .as_deref()
        .map(str::trim)
        .filter(|configured| !configured.is_empty());

    match configured_env_name {
        None => {
            *env_name = Some(explicit_env_name);
            *value = None;
        }
        Some(configured_env_name) if configured_env_name == explicit_env_name => {
            *env_name = Some(explicit_env_name);
            *value = None;
        }
        Some(_) => {}
    }
}

fn normalize_acp_agent_id(raw: &str) -> Option<String> {
    let normalized = raw.trim().to_ascii_lowercase();
    let mut chars = normalized.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphanumeric() {
        return None;
    }
    if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') {
        return None;
    }
    Some(normalized)
}

pub(crate) fn normalize_dispatch_channel_id(raw: &str) -> Option<String> {
    let normalized = raw.trim().to_ascii_lowercase();
    let mut chars = normalized.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphanumeric() {
        return None;
    }
    if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') {
        return None;
    }
    Some(normalized)
}

pub(crate) fn normalize_dispatch_account_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut normalized = String::with_capacity(trimmed.len());
    let mut last_was_separator = false;
    let mut saw_alphanumeric = false;
    for value in trimmed.chars() {
        if value.is_ascii_alphanumeric() {
            normalized.push(value.to_ascii_lowercase());
            last_was_separator = false;
            saw_alphanumeric = true;
            continue;
        }
        if matches!(value, '_' | '-') {
            if !normalized.is_empty() && !last_was_separator {
                normalized.push(value);
                last_was_separator = true;
            }
            continue;
        }
        if !normalized.is_empty() && !last_was_separator {
            normalized.push('-');
            last_was_separator = true;
        }
    }

    while matches!(normalized.chars().last(), Some('-' | '_')) {
        normalized.pop();
    }

    if !saw_alphanumeric || normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

impl LoongClawConfig {
    fn collect_validation_issues_with_report(
        &self,
        selection_report: Option<&ProviderSelectionNormalizationReport>,
    ) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        if self.providers.is_empty() {
            issues.extend(self.provider.validate());
        } else {
            for (profile_id, profile) in &self.providers {
                if selection_report.is_some_and(|report| {
                    report.legacy_profile_inserted
                        && report.recovered_legacy_profile_id.as_deref()
                            == Some(profile_id.as_str())
                }) {
                    continue;
                }
                issues.extend(
                    profile
                        .provider
                        .validate_with_field_prefix(format!("providers.{profile_id}").as_str()),
                );
            }
        }
        if let Some(report) = selection_report {
            issues.extend(report.validation_issues());
        }
        issues.extend(super::channels::collect_channel_validation_issues(self));
        issues.extend(self.feishu_integration.validate());
        issues.extend(self.memory.validate());
        issues.extend(self.tools.validate());
        issues.extend(self.runtime_plugins.validate());
        issues
    }

    pub fn validate(&self) -> CliResult<()> {
        self.validate_with_report(None)
    }

    fn validate_with_report(
        &self,
        selection_report: Option<&ProviderSelectionNormalizationReport>,
    ) -> CliResult<()> {
        let issues = self.collect_validation_issues_with_report(selection_report);
        let errors = issues
            .into_iter()
            .filter(ConfigValidationIssue::is_error)
            .collect::<Vec<_>>();
        if errors.is_empty() {
            return Ok(());
        }
        Err(format_config_validation_issues(&errors))
    }

    pub fn validation_diagnostics(&self) -> Vec<ConfigValidationDiagnostic> {
        self.validation_diagnostics_with_locale(ConfigValidationLocale::En)
    }

    fn validation_diagnostics_with_locale(
        &self,
        locale: ConfigValidationLocale,
    ) -> Vec<ConfigValidationDiagnostic> {
        self.validation_diagnostics_with_locale_and_report(locale, None)
    }

    fn validation_diagnostics_with_locale_and_report(
        &self,
        locale: ConfigValidationLocale,
        selection_report: Option<&ProviderSelectionNormalizationReport>,
    ) -> Vec<ConfigValidationDiagnostic> {
        self.collect_validation_issues_with_report(selection_report)
            .iter()
            .map(|issue| ConfigValidationDiagnostic::from_issue(issue, locale))
            .collect()
    }

    pub fn enabled_channel_ids(&self) -> Vec<String> {
        super::channels::enabled_channel_ids(self)
    }

    pub fn enabled_service_channel_ids(&self) -> Vec<String> {
        super::channels::enabled_service_channel_ids(self)
    }

    pub fn active_provider_id(&self) -> Option<&str> {
        if let Some(active_provider) = self.active_provider.as_deref() {
            let trimmed = active_provider.trim();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
        if self.providers.is_empty() {
            return Some(self.provider.kind.profile().id);
        }
        self.providers.keys().next().map(String::as_str)
    }

    pub fn last_provider_id(&self) -> Option<&str> {
        self.last_provider
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub fn resolve_provider_switch_target(&self, selector: &str) -> CliResult<String> {
        let mut normalized = self.clone();
        normalized.normalize_provider_profiles();
        normalized.resolve_provider_switch_target_from_normalized(selector)
    }

    pub fn switch_active_provider(&mut self, selector: &str) -> CliResult<String> {
        self.normalize_provider_profiles();
        let target_profile_id = self.resolve_provider_switch_target_from_normalized(selector)?;
        let previous_active = self.active_provider_id().map(str::to_owned);
        let target_profile = self
            .providers
            .get(&target_profile_id)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "provider switch target `{target_profile_id}` is unavailable in the current config"
                )
            })?;

        for profile in self
            .providers
            .values_mut()
            .filter(|profile| profile.provider.kind == target_profile.provider.kind)
        {
            profile.default_for_kind = false;
        }
        if let Some(profile) = self.providers.get_mut(&target_profile_id) {
            profile.default_for_kind = true;
        }

        self.provider = target_profile.provider;
        self.active_provider = Some(target_profile_id.clone());
        if previous_active.as_deref() != Some(target_profile_id.as_str()) {
            self.last_provider = previous_active;
        }
        Ok(target_profile_id)
    }

    pub fn accepted_provider_selectors(&self, target_profile_id: &str) -> Vec<String> {
        accepted_provider_selectors(self.provider_selector_profiles(), target_profile_id)
    }

    pub fn preferred_provider_selector(&self, target_profile_id: &str) -> Option<String> {
        preferred_provider_selector(self.provider_selector_profiles(), target_profile_id)
    }

    pub fn clone_with_provider_runtime_state(
        &self,
        provider_runtime_state: &LoongClawConfig,
    ) -> Self {
        let mut merged = self.clone();
        merged.provider = provider_runtime_state.provider.clone();
        merged.providers = provider_runtime_state.providers.clone();
        merged.active_provider = provider_runtime_state.active_provider.clone();
        merged.last_provider = provider_runtime_state.last_provider.clone();
        merged.normalize_provider_profiles();
        merged
    }

    pub fn reload_provider_runtime_state_from_path(&self, path: &Path) -> CliResult<Self> {
        let raw = fs::read_to_string(path).map_err(|error| {
            format!(
                "failed to read provider runtime config {}: {error}",
                path.display()
            )
        })?;
        let reloaded = parse_toml_config_without_validation(&raw)?;
        Ok(self.clone_with_provider_runtime_state(&reloaded))
    }

    pub fn set_active_provider_profile(
        &mut self,
        profile_id: impl Into<String>,
        profile: ProviderProfileConfig,
    ) {
        let profile_id = normalize_provider_profile_id(profile_id.into().as_str())
            .unwrap_or_else(|| profile.provider.inferred_profile_id());
        self.provider = profile.provider.clone();
        self.providers.insert(profile_id.clone(), profile);
        self.active_provider = Some(profile_id);
    }

    fn provider_selector_profiles(&self) -> Vec<ProviderSelectorProfileRef<'_>> {
        self.providers
            .iter()
            .map(|(profile_id, profile)| {
                ProviderSelectorProfileRef::new(
                    profile_id,
                    profile.provider.kind,
                    profile.provider.model.as_str(),
                    profile.default_for_kind,
                )
            })
            .collect()
    }

    fn normalize_provider_profiles(&mut self) {
        let _ = self.normalize_provider_profiles_with_intent(None);
    }

    fn normalize_provider_profiles_with_intent(
        &mut self,
        intent: Option<&RawProviderSelectionIntent>,
    ) -> ProviderSelectionNormalizationReport {
        let normalized_last_provider = self
            .last_provider
            .as_deref()
            .and_then(normalize_provider_profile_id);
        let mut report = ProviderSelectionNormalizationReport {
            legacy_provider_explicit: intent
                .is_some_and(|selection| selection.legacy_provider_explicit),
            active_provider_explicit: intent
                .is_some_and(|selection| selection.active_provider_explicit),
            requested_active_provider: intent
                .and_then(|selection| selection.raw_active_provider.clone()),
            ..ProviderSelectionNormalizationReport::default()
        };

        if self.providers.is_empty() {
            let active_provider = self
                .active_provider
                .as_deref()
                .and_then(normalize_provider_profile_id)
                .unwrap_or_else(|| self.provider.inferred_profile_id());
            let mut active_profile = ProviderProfileConfig::from_provider(self.provider.clone());
            active_profile.default_for_kind = true;
            self.providers
                .insert(active_provider.clone(), active_profile);
            self.active_provider = Some(active_provider);
            self.last_provider = normalized_last_provider;
            report.selected_active_provider = self.active_provider.clone();
            report.configured_profile_ids = self.providers.keys().cloned().collect();
            report.selection_basis = Some(ActiveProviderSelectionBasis::LegacyOnly);
            return report;
        }

        let mut normalized_profiles = BTreeMap::new();
        for (profile_id, profile) in &self.providers {
            let normalized_profile_id = normalize_provider_profile_id(profile_id.as_str())
                .unwrap_or_else(|| profile.provider.inferred_profile_id());
            normalized_profiles.insert(normalized_profile_id, profile.clone());
        }
        self.providers = normalized_profiles;
        report.configured_profile_ids = self.providers.keys().cloned().collect();

        let explicit_active_provider = self
            .active_provider
            .as_deref()
            .and_then(normalize_provider_profile_id)
            .filter(|profile_id| self.providers.contains_key(profile_id))
            .inspect(|_| {
                report.selection_basis = Some(ActiveProviderSelectionBasis::ExplicitActiveProvider);
            });
        if report.active_provider_explicit && explicit_active_provider.is_none() {
            report.warn_unknown_active_provider = true;
        }
        let active_provider = explicit_active_provider
            .or_else(|| {
                if !report.legacy_provider_explicit {
                    return None;
                }
                let (legacy_profile_id, inserted) =
                    recover_active_provider_from_legacy_config(&self.provider, &mut self.providers);
                report.configured_profile_ids = self.providers.keys().cloned().collect();
                report.selection_basis = Some(ActiveProviderSelectionBasis::ExplicitLegacyProvider);
                report.recovered_legacy_profile_id = Some(legacy_profile_id.clone());
                report.legacy_profile_inserted = inserted;
                Some(legacy_profile_id)
            })
            .or_else(|| {
                let first_profile_id = self.providers.keys().next().cloned();
                if first_profile_id.is_some() {
                    report.selection_basis = Some(ActiveProviderSelectionBasis::FirstSavedProfile);
                }
                first_profile_id
            });
        let Some(active_provider) = active_provider else {
            return report;
        };
        self.active_provider = Some(active_provider.clone());
        self.last_provider =
            normalized_last_provider.filter(|profile_id| self.providers.contains_key(profile_id));
        if let Some(active_profile) = self.providers.get(&active_provider) {
            self.provider = active_profile.provider.clone();
        }
        report.selected_active_provider = Some(active_provider);
        report.warn_implicit_active_provider =
            !report.active_provider_explicit && report.configured_profile_ids.len() > 1;
        report
    }

    fn resolve_provider_switch_target_from_normalized(&self, selector: &str) -> CliResult<String> {
        let trimmed = selector.trim();
        if trimmed.is_empty() {
            return Err("provider selector cannot be empty".to_owned());
        }
        let selector_profiles = self.provider_selector_profiles();
        match resolve_provider_selector(selector_profiles.iter().copied(), trimmed) {
            ProviderSelectorResolution::Match(profile_id) => Ok(profile_id),
            ProviderSelectorResolution::Ambiguous(profile_ids) => {
                let recommendation = provider_selector_recommendation_hint(
                    selector_profiles.iter().copied(),
                    &profile_ids,
                )
                .map(|hint| format!("; {hint}"))
                .unwrap_or_default();
                Err(format!(
                    "provider selector `{trimmed}` is ambiguous; matching profiles: {}{}",
                    profile_ids
                        .iter()
                        .filter_map(|profile_id| {
                            describe_provider_selector_target(
                                selector_profiles.iter().copied(),
                                profile_id,
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", "),
                    recommendation
                ))
            }
            ProviderSelectorResolution::NoMatch => {
                let recommendation = provider_selector_recommendation_hint(
                    selector_profiles.iter().copied(),
                    selector_profiles.iter().map(|profile| profile.profile_id),
                )
                .map(|hint| format!("; {hint}"))
                .unwrap_or_default();
                Err(format!(
                    "unknown provider selector `{trimmed}`; accepted selectors: {}{}",
                    provider_selector_catalog(selector_profiles.iter().copied()).join(", "),
                    recommendation
                ))
            }
        }
    }

    fn clone_for_encoding(&self) -> Self {
        let mut cloned = self.clone();
        let active_provider = cloned
            .active_provider
            .as_deref()
            .and_then(normalize_provider_profile_id)
            .unwrap_or_else(|| cloned.provider.inferred_profile_id());
        let mut active_profile = cloned
            .providers
            .remove(&active_provider)
            .unwrap_or_else(|| ProviderProfileConfig::from_provider(cloned.provider.clone()));
        active_profile.provider = cloned.provider.clone();
        if !cloned
            .providers
            .values()
            .any(|profile| profile.provider.kind == active_profile.provider.kind)
        {
            active_profile.default_for_kind = true;
        }
        canonicalize_provider_profile_for_encoding(&mut active_profile);
        for profile in cloned.providers.values_mut() {
            canonicalize_provider_profile_for_encoding(profile);
        }
        cloned
            .providers
            .insert(active_provider.clone(), active_profile);
        cloned.active_provider = Some(active_provider);
        cloned.last_provider = cloned
            .last_provider
            .as_deref()
            .and_then(normalize_provider_profile_id);
        canonicalize_channel_configs_for_encoding(&mut cloned);
        cloned
    }
}

fn normalized_inferred_profile_id(provider: &ProviderConfig) -> String {
    normalize_provider_profile_id(provider.inferred_profile_id().as_str())
        .unwrap_or_else(|| provider.inferred_profile_id())
}

fn matching_legacy_provider_profile_id(
    providers: &BTreeMap<String, ProviderProfileConfig>,
    legacy_provider: &ProviderConfig,
) -> Option<String> {
    let inferred_profile_id = normalized_inferred_profile_id(legacy_provider);
    if providers
        .get(&inferred_profile_id)
        .is_some_and(|profile| profile.provider == *legacy_provider)
    {
        return Some(inferred_profile_id);
    }

    let exact_matches = providers
        .iter()
        .filter(|(_profile_id, profile)| profile.provider == *legacy_provider)
        .map(|(profile_id, _profile)| profile_id.clone())
        .collect::<Vec<_>>();
    if exact_matches.len() == 1 {
        return exact_matches.into_iter().next();
    }
    exact_matches.into_iter().next()
}

fn next_available_provider_profile_id(
    providers: &BTreeMap<String, ProviderProfileConfig>,
    base_profile_id: &str,
) -> String {
    if !providers.contains_key(base_profile_id) {
        return base_profile_id.to_owned();
    }
    let max_suffix = providers.len().saturating_add(2);
    for suffix in 2..=max_suffix {
        let candidate = format!("{base_profile_id}-{suffix}");
        if !providers.contains_key(&candidate) {
            return candidate;
        }
    }
    format!("{base_profile_id}-{max_suffix}")
}

fn recover_active_provider_from_legacy_config(
    legacy_provider: &ProviderConfig,
    providers: &mut BTreeMap<String, ProviderProfileConfig>,
) -> (String, bool) {
    if let Some(profile_id) = matching_legacy_provider_profile_id(providers, legacy_provider) {
        return (profile_id, false);
    }

    let profile_id = next_available_provider_profile_id(
        providers,
        normalized_inferred_profile_id(legacy_provider).as_str(),
    );
    let mut recovered_profile = ProviderProfileConfig::from_provider(legacy_provider.clone());
    recovered_profile.default_for_kind = !providers
        .values()
        .any(|profile| profile.provider.kind == recovered_profile.provider.kind);
    providers.insert(profile_id.clone(), recovered_profile);
    (profile_id, true)
}

#[cfg(feature = "config-toml")]
fn inspect_raw_provider_selection_intent(raw: &str) -> CliResult<RawProviderSelectionIntent> {
    let value = toml::from_str::<toml::Value>(raw)
        .map_err(|error| format!("failed to parse TOML config: {error}"))?;
    let table = value.as_table();
    Ok(RawProviderSelectionIntent {
        legacy_provider_explicit: table.is_some_and(|root| root.contains_key("provider")),
        active_provider_explicit: table.is_some_and(|root| root.contains_key("active_provider")),
        raw_active_provider: table
            .and_then(|root| root.get("active_provider"))
            .and_then(toml::Value::as_str)
            .map(str::to_owned),
    })
}

#[cfg(feature = "config-toml")]
fn parse_toml_config_components(
    raw: &str,
) -> CliResult<(LoongClawConfig, ProviderSelectionNormalizationReport)> {
    let mut config = toml::from_str::<LoongClawConfig>(raw)
        .map_err(|error| format!("failed to parse TOML config: {error}"))?;
    let selection_intent = inspect_raw_provider_selection_intent(raw)?;
    let had_saved_provider_profiles = !config.providers.is_empty();
    let legacy_provider_before_normalization = config.provider.clone();
    let mut selection_report =
        config.normalize_provider_profiles_with_intent(Some(&selection_intent));
    if selection_intent.legacy_provider_explicit && had_saved_provider_profiles {
        selection_report.legacy_provider_validation_issues =
            legacy_provider_before_normalization.validate();
    }
    Ok((config, selection_report))
}

#[cfg(not(feature = "config-toml"))]
fn parse_toml_config_components(
    _raw: &str,
) -> CliResult<(LoongClawConfig, ProviderSelectionNormalizationReport)> {
    Err("config-toml feature is disabled for this build".to_owned())
}

pub fn load(path: Option<&str>) -> CliResult<(PathBuf, LoongClawConfig)> {
    let config_path = path.map(expand_path).unwrap_or_else(default_config_path);
    let raw = fs::read_to_string(&config_path).map_err(|error| {
        format!(
            "failed to read config {}: {error}. run `{} onboard` first",
            config_path.display(),
            crate::config::active_cli_command_name(),
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
            "failed to read config {}: {error}. run `{} onboard` first",
            config_path.display(),
            crate::config::active_cli_command_name(),
        )
    })?;
    let (config, selection_report) = parse_toml_config_components(&raw)?;
    let locale = ConfigValidationLocale::from_tag(locale_tag);
    Ok((
        config_path,
        config.validation_diagnostics_with_locale_and_report(locale, Some(&selection_report)),
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
        "{}{}{}",
        template_secret_usage_comment(),
        template_web_search_usage_comment(),
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
    let (config, selection_report) = parse_toml_config_components(raw)?;
    config.validate_with_report(Some(&selection_report))?;
    Ok(config)
}

#[cfg(feature = "config-toml")]
fn parse_toml_config_without_validation(raw: &str) -> CliResult<LoongClawConfig> {
    parse_toml_config_components(raw).map(|(config, _selection_report)| config)
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
    let encoded = config.clone_for_encoding();
    toml::to_string_pretty(&encoded)
        .map_err(|error| format!("failed to encode TOML config: {error}"))
}

#[cfg(not(feature = "config-toml"))]
fn encode_toml_config(_config: &LoongClawConfig) -> CliResult<String> {
    Err("config-toml feature is disabled for this build".to_owned())
}

fn template_secret_usage_comment() -> &'static str {
    "# Secret configuration notes:\n\
# - Preferred provider credential form: `providers.<profile_id>.api_key = { env = \"PROVIDER_API_KEY\" }`.\n\
# - `providers.<profile_id>.api_key` still accepts direct literals and explicit env refs like `$VAR`, `env:VAR`, and `%VAR%`.\n\
# - Legacy `*_env` provider fields still load, but LoongClaw now writes provider env refs back into the main secret field.\n\
\n"
}

fn template_web_search_usage_comment() -> String {
    format!(
        "# Web search provider notes:\n\
# - `[tools.web_search].default_provider` accepts {WEB_SEARCH_PROVIDER_VALID_VALUES}.\n\
# - The default provider is `{DEFAULT_WEB_SEARCH_PROVIDER}`.\n\
# - Brave credentials can use `tools.web_search.brave_api_key = \"${{{WEB_SEARCH_BRAVE_API_KEY_ENV}}}\"` or the `{WEB_SEARCH_BRAVE_API_KEY_ENV}` environment variable.\n\
# - Tavily credentials can use `tools.web_search.tavily_api_key = \"${{{WEB_SEARCH_TAVILY_API_KEY_ENV}}}\"` or the `{WEB_SEARCH_TAVILY_API_KEY_ENV}` environment variable.\n\
# - Perplexity credentials can use `tools.web_search.perplexity_api_key = \"${{{WEB_SEARCH_PERPLEXITY_API_KEY_ENV}}}\"` or the `{WEB_SEARCH_PERPLEXITY_API_KEY_ENV}` environment variable.\n\
# - Exa credentials can use `tools.web_search.exa_api_key = \"${{{WEB_SEARCH_EXA_API_KEY_ENV}}}\"` or the `{WEB_SEARCH_EXA_API_KEY_ENV}` environment variable.\n\
# - Firecrawl credentials can use `tools.web_search.firecrawl_api_key = \"${{{WEB_SEARCH_FIRECRAWL_API_KEY_ENV}}}\"` or the `{WEB_SEARCH_FIRECRAWL_API_KEY_ENV}` environment variable.\n\
# - Jina credentials can use `tools.web_search.jina_api_key = \"${{{WEB_SEARCH_JINA_API_KEY_ENV}}}\"` or the `{WEB_SEARCH_JINA_API_KEY_ENV}` / `{WEB_SEARCH_JINA_AUTH_TOKEN_ENV}` environment variable.\n\
\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProviderKind;
    use crate::config::{
        ONEBOT_ACCESS_TOKEN_ENV, ONEBOT_WEBSOCKET_URL_ENV, QQBOT_APP_ID_ENV,
        QQBOT_CLIENT_SECRET_ENV, WEIXIN_BRIDGE_ACCESS_TOKEN_ENV, WEIXIN_BRIDGE_URL_ENV,
    };
    use loongclaw_contracts::SecretRef;
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
        assert!(error.contains("providers.openai.api_key_env"));
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
    fn write_template_prefers_generic_provider_api_key_env_secret_ref_example() {
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
        assert!(raw.contains("providers.<profile_id>.api_key = { env = \"PROVIDER_API_KEY\" }"));
        assert!(!raw.contains("providers.<profile_id>.api_key_env = \"PROVIDER_API_KEY\""));

        std::fs::remove_file(&config_path).ok();
        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn write_template_includes_web_search_provider_notes() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let temp_dir =
            std::env::temp_dir().join(format!("loongclaw-template-web-search-notes-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("create temp directory");
        let config_path = temp_dir.join("config.toml");

        write_template(Some(config_path.to_string_lossy().as_ref()), true)
            .expect("write template should succeed");

        let raw = std::fs::read_to_string(&config_path).expect("read template");
        assert!(raw.contains("# Web search provider notes:"));
        assert!(raw.contains(WEB_SEARCH_PROVIDER_VALID_VALUES));
        assert!(raw.contains(WEB_SEARCH_BRAVE_API_KEY_ENV));
        assert!(raw.contains(WEB_SEARCH_TAVILY_API_KEY_ENV));
        assert!(raw.contains(WEB_SEARCH_PERPLEXITY_API_KEY_ENV));
        assert!(raw.contains(WEB_SEARCH_EXA_API_KEY_ENV));
        assert!(raw.contains(WEB_SEARCH_FIRECRAWL_API_KEY_ENV));
        assert!(raw.contains(WEB_SEARCH_JINA_API_KEY_ENV));
        assert!(raw.contains(WEB_SEARCH_JINA_AUTH_TOKEN_ENV));

        std::fs::remove_file(&config_path).ok();
        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn write_template_includes_tool_result_payload_summary_limit_default() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let temp_dir =
            std::env::temp_dir().join(format!("loongclaw-template-tool-summary-limit-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("create temp directory");
        let config_path = temp_dir.join("config.toml");

        write_template(Some(config_path.to_string_lossy().as_ref()), true)
            .expect("write template should succeed");

        let raw = std::fs::read_to_string(&config_path).expect("read template");
        assert!(raw.contains("[conversation]"));
        assert!(raw.contains("tool_result_payload_summary_limit_chars = 2048"));
        assert!(raw.contains("safe_lane_health_truncation_warn_threshold = 0.3"));
        assert!(raw.contains("safe_lane_health_truncation_critical_threshold = 0.6"));
        assert!(raw.contains("safe_lane_health_verify_failure_warn_threshold = 0.4"));
        assert!(raw.contains("safe_lane_health_replan_warn_threshold = 0.5"));

        std::fs::remove_file(&config_path).ok();
        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn write_template_includes_fast_lane_parallel_tool_execution_defaults() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!(
            "loongclaw-template-fast-lane-parallel-execution-{unique}"
        ));
        std::fs::create_dir_all(&temp_dir).expect("create temp directory");
        let config_path = temp_dir.join("config.toml");

        write_template(Some(config_path.to_string_lossy().as_ref()), true)
            .expect("write template should succeed");

        let raw = std::fs::read_to_string(&config_path).expect("read template");
        assert!(raw.contains("[conversation]"));
        assert!(raw.contains("fast_lane_parallel_tool_execution_enabled = false"));
        assert!(raw.contains("fast_lane_parallel_tool_execution_max_in_flight = 4"));

        std::fs::remove_file(&config_path).ok();
        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn write_template_includes_matrix_channel_defaults() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("loongclaw-template-matrix-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("create temp directory");
        let config_path = temp_dir.join("config.toml");

        write_template(Some(config_path.to_string_lossy().as_ref()), true)
            .expect("write template should succeed");

        let raw = std::fs::read_to_string(&config_path).expect("read template");
        assert!(raw.contains("[matrix]"));
        assert!(raw.contains("access_token_env = \"MATRIX_ACCESS_TOKEN\""));
        assert!(raw.contains("sync_timeout_s = 30"));
        assert!(raw.contains("ignore_self_messages = true"));

        std::fs::remove_file(&config_path).ok();
        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn write_template_includes_plugin_backed_bridge_channel_defaults() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let temp_dir =
            std::env::temp_dir().join(format!("loongclaw-template-plugin-bridges-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("create temp directory");
        let config_path = temp_dir.join("config.toml");

        write_template(Some(config_path.to_string_lossy().as_ref()), true)
            .expect("write template should succeed");

        let raw = std::fs::read_to_string(&config_path).expect("read template");

        assert!(raw.contains("[weixin]"));
        assert!(raw.contains(format!("bridge_url_env = \"{WEIXIN_BRIDGE_URL_ENV}\"").as_str()));
        assert!(raw.contains(
            format!("bridge_access_token_env = \"{WEIXIN_BRIDGE_ACCESS_TOKEN_ENV}\"").as_str()
        ));

        assert!(raw.contains("[qqbot]"));
        assert!(raw.contains(format!("app_id_env = \"{QQBOT_APP_ID_ENV}\"").as_str()));
        assert!(
            raw.contains(format!("client_secret_env = \"{QQBOT_CLIENT_SECRET_ENV}\"").as_str())
        );

        assert!(raw.contains("[onebot]"));
        assert!(
            raw.contains(format!("websocket_url_env = \"{ONEBOT_WEBSOCKET_URL_ENV}\"").as_str())
        );
        assert!(raw.contains(format!("access_token_env = \"{ONEBOT_ACCESS_TOKEN_ENV}\"").as_str()));

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
        assert_eq!(diagnostics[0].severity, "error");
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
        assert_eq!(diagnostics[0].field_path, "providers.openai.api_key_env");
        assert_eq!(
            diagnostics[0].message_variables.get("field_path"),
            Some(&"providers.openai.api_key_env".to_owned())
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
    fn validate_file_returns_typed_secret_ref_env_diagnostics() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let temp_dir =
            std::env::temp_dir().join(format!("loongclaw-config-typed-env-diagnostics-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("create temp directory");
        let config_path = temp_dir.join("config.toml");
        let raw = r#"
[provider]
api_key = { env = "$OPENAI_API_KEY" }
"#;
        std::fs::write(&config_path, raw).expect("write test config");

        let (_, diagnostics) = validate_file(Some(config_path.to_string_lossy().as_ref()))
            .expect("validate_file should parse and return diagnostics");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, "error");
        assert_eq!(diagnostics[0].code, "config.env_pointer.dollar_prefix");
        assert_eq!(diagnostics[0].field_path, "providers.openai.api_key.env");
        assert!(
            diagnostics[0].message.contains("without `$`"),
            "expected dollar-prefix guidance for typed env refs"
        );

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
        assert_eq!(diagnostics[0].severity, "error");
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
    fn load_missing_config_guides_user_to_loongclaw_onboard() {
        let missing = unique_config_path("loongclaw-config-missing");
        let path_string = missing.display().to_string();

        let error = load(Some(&path_string)).expect_err("missing config should fail");
        assert!(error.contains("run `loong onboard` first"));
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
    fn write_persists_typed_personalization_metadata() {
        let path = unique_config_path("loongclaw-personalization-config");
        let path_string = path.display().to_string();
        let mut config = LoongClawConfig::default();
        let personalization = crate::config::PersonalizationConfig {
            preferred_name: Some("Chum".to_owned()),
            response_density: Some(crate::config::ResponseDensity::Thorough),
            initiative_level: Some(crate::config::InitiativeLevel::HighInitiative),
            standing_boundaries: Some("Ask before destructive actions.".to_owned()),
            timezone: Some("Asia/Shanghai".to_owned()),
            locale: Some("zh-CN".to_owned()),
            prompt_state: crate::config::PersonalizationPromptState::Configured,
            schema_version: 1,
            updated_at_epoch_seconds: Some(1_775_095_200),
        };

        config.memory.profile = crate::config::MemoryProfile::ProfilePlusWindow;
        config.memory.personalization = Some(personalization);

        write(Some(&path_string), &config, true).expect("config write should pass");

        let load_result = load(Some(&path_string));
        let (_, loaded) = load_result.expect("config load should pass");
        let loaded_personalization = loaded
            .memory
            .personalization
            .expect("typed personalization should persist");
        let preferred_name = loaded_personalization.preferred_name.as_deref();
        let response_density = loaded_personalization.response_density;
        let initiative_level = loaded_personalization.initiative_level;
        let standing_boundaries = loaded_personalization.standing_boundaries.as_deref();
        let timezone = loaded_personalization.timezone.as_deref();
        let locale = loaded_personalization.locale.as_deref();
        let prompt_state = loaded_personalization.prompt_state;
        let schema_version = loaded_personalization.schema_version;
        let updated_at_epoch_seconds = loaded_personalization.updated_at_epoch_seconds;

        assert_eq!(preferred_name, Some("Chum"));
        assert_eq!(
            response_density,
            Some(crate::config::ResponseDensity::Thorough)
        );
        assert_eq!(
            initiative_level,
            Some(crate::config::InitiativeLevel::HighInitiative)
        );
        assert_eq!(standing_boundaries, Some("Ask before destructive actions."));
        assert_eq!(timezone, Some("Asia/Shanghai"));
        assert_eq!(locale, Some("zh-CN"));
        assert_eq!(
            prompt_state,
            crate::config::PersonalizationPromptState::Configured
        );
        assert_eq!(schema_version, 1);
        assert_eq!(updated_at_epoch_seconds, Some(1_775_095_200));

        let _ = fs::remove_file(path);
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn load_legacy_provider_table_populates_active_provider_profile_storage() {
        let path = unique_config_path("loongclaw-config-legacy-provider");
        let raw = r#"
[provider]
kind = "deepseek"
model = "deepseek-chat"
api_key = "${DEEPSEEK_API_KEY}"
"#;
        fs::write(&path, raw).expect("write legacy config");

        let (_, loaded) = load(Some(path.to_string_lossy().as_ref())).expect("config load");
        assert_eq!(loaded.active_provider_id(), Some("deepseek"));
        assert_eq!(loaded.providers.len(), 1);
        let profile = loaded
            .providers
            .get("deepseek")
            .expect("deepseek provider profile");
        assert_eq!(profile.provider.kind, ProviderKind::Deepseek);
        assert_eq!(profile.provider.model, "deepseek-chat");
        assert_eq!(loaded.provider.kind, ProviderKind::Deepseek);

        let _ = fs::remove_file(path);
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn mixed_legacy_and_profile_config_preserves_explicit_legacy_provider_when_active_provider_missing()
     {
        let raw = r#"
[provider]
kind = "volcengine_coding"
model = "ark-code-latest"
base_url = "https://ark.cn-beijing.volces.com/api/coding/v3"
wire_api = "chat_completions"
chat_completions_path = "/chat/completions"

[providers.openrouter]
default_for_kind = true
kind = "openrouter"
model = "z-ai/glm-4.5-air:free"
base_url = "https://openrouter.ai"
wire_api = "chat_completions"
chat_completions_path = "/api/v1/chat/completions"
"#;

        let config = parse_toml_config_without_validation(raw).expect("config should parse");

        assert_eq!(config.active_provider_id(), Some("volcengine_coding"));
        assert_eq!(config.provider.kind, ProviderKind::VolcengineCoding);
        assert_eq!(config.provider.model, "ark-code-latest");
        assert!(
            config.providers.contains_key("volcengine_coding"),
            "legacy provider intent should be preserved as a normalized saved profile: {config:#?}"
        );
        assert!(
            config.providers.contains_key("openrouter"),
            "existing saved profiles should be retained during normalization: {config:#?}"
        );
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn validate_file_reports_warning_for_implicit_active_provider_recovery_in_mixed_config() {
        let path = unique_config_path("loongclaw-config-provider-selection-warning");
        let raw = r#"
[provider]
kind = "volcengine_coding"
model = "ark-code-latest"
base_url = "https://ark.cn-beijing.volces.com/api/coding/v3"
wire_api = "chat_completions"
chat_completions_path = "/chat/completions"

[providers.openrouter]
default_for_kind = true
kind = "openrouter"
model = "z-ai/glm-4.5-air:free"
base_url = "https://openrouter.ai"
wire_api = "chat_completions"
chat_completions_path = "/api/v1/chat/completions"
"#;
        fs::write(&path, raw).expect("write mixed provider config");

        let (_, diagnostics) = validate_file(Some(path.to_string_lossy().as_ref()))
            .expect("validate_file should parse and return diagnostics");

        assert!(
            diagnostics.iter().any(|diagnostic| diagnostic.code
                == "config.provider_selection.implicit_active"
                && diagnostic.severity == "warn"
                && diagnostic.field_path == "active_provider"),
            "mixed configs without an explicit active_provider should surface a warning-level provider-selection diagnostic: {diagnostics:#?}"
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn mixed_config_recovers_invalid_explicit_active_provider_from_legacy_provider() {
        let raw = r#"
active_provider = "missing-profile"

[provider]
kind = "volcengine_coding"
model = "ark-code-latest"
base_url = "https://ark.cn-beijing.volces.com/api/coding/v3"
wire_api = "chat_completions"
chat_completions_path = "/chat/completions"

[providers.openrouter]
default_for_kind = true
kind = "openrouter"
model = "z-ai/glm-4.5-air:free"
base_url = "https://openrouter.ai"
wire_api = "chat_completions"
chat_completions_path = "/api/v1/chat/completions"
"#;

        let config = parse_toml_config_without_validation(raw).expect("config should parse");

        assert_eq!(config.active_provider_id(), Some("volcengine_coding"));
        assert_eq!(config.provider.kind, ProviderKind::VolcengineCoding);
        assert_eq!(config.provider.model, "ark-code-latest");
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn validate_file_reports_warning_for_unknown_active_provider_recovery_in_mixed_config() {
        let path = unique_config_path("loongclaw-config-provider-selection-unknown-active");
        let raw = r#"
active_provider = "missing-profile"

[provider]
kind = "volcengine_coding"
model = "ark-code-latest"
base_url = "https://ark.cn-beijing.volces.com/api/coding/v3"
wire_api = "chat_completions"
chat_completions_path = "/chat/completions"

[providers.openrouter]
default_for_kind = true
kind = "openrouter"
model = "z-ai/glm-4.5-air:free"
base_url = "https://openrouter.ai"
wire_api = "chat_completions"
chat_completions_path = "/api/v1/chat/completions"
"#;
        fs::write(&path, raw).expect("write mixed provider config");

        let (_, diagnostics) = validate_file(Some(path.to_string_lossy().as_ref()))
            .expect("validate_file should parse and return diagnostics");

        assert!(
            diagnostics.iter().any(|diagnostic| diagnostic.code
                == "config.provider_selection.unknown_active"
                && diagnostic.severity == "warn"
                && diagnostic.field_path == "active_provider"),
            "mixed configs with an invalid explicit active_provider should surface a warning-level recovery diagnostic: {diagnostics:#?}"
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn validate_file_reports_legacy_provider_field_errors_even_when_provider_profiles_exist() {
        let path = unique_config_path("loongclaw-config-legacy-provider-validation");
        let raw = r#"
active_provider = "openrouter"

[provider]
kind = "volcengine_coding"
model = "ark-code-latest"
api_key_env = "$VOLCENGINE_CODING_API_KEY"
base_url = "https://ark.cn-beijing.volces.com/api/coding/v3"
wire_api = "chat_completions"
chat_completions_path = "/chat/completions"

[providers.openrouter]
default_for_kind = true
kind = "openrouter"
model = "z-ai/glm-4.5-air:free"
base_url = "https://openrouter.ai"
wire_api = "chat_completions"
chat_completions_path = "/api/v1/chat/completions"
"#;
        fs::write(&path, raw).expect("write mixed provider config");

        let (_, diagnostics) = validate_file(Some(path.to_string_lossy().as_ref()))
            .expect("validate_file should parse and return diagnostics");

        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "config.env_pointer.dollar_prefix"
                    && diagnostic.field_path == "provider.api_key_env"
                    && diagnostic.severity == "error"
            }),
            "explicit legacy provider fields should still be validated when saved provider profiles also exist: {diagnostics:#?}"
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn mixed_config_recovers_explicit_legacy_provider_without_overwriting_conflicting_profile_id() {
        let raw = r#"
[provider]
kind = "openrouter"
model = "z-ai/glm-4.5-air:free"
base_url = "https://openrouter.ai"
wire_api = "chat_completions"
chat_completions_path = "/api/v1/chat/completions"

[providers.openrouter]
default_for_kind = true
kind = "openai"
model = "gpt-5"
"#;

        let config = parse_toml_config_without_validation(raw).expect("config should parse");

        assert_eq!(config.provider.kind, ProviderKind::Openrouter);
        assert_eq!(config.provider.model, "z-ai/glm-4.5-air:free");
        assert_eq!(config.active_provider_id(), Some("openrouter-2"));
        assert_eq!(
            config
                .providers
                .get("openrouter")
                .expect("conflicting saved profile should be retained")
                .provider
                .kind,
            ProviderKind::Openai
        );
        assert_eq!(
            config
                .providers
                .get("openrouter-2")
                .expect("legacy provider should be recovered into a fresh profile id")
                .provider
                .kind,
            ProviderKind::Openrouter
        );
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn write_default_config_does_not_eagerly_persist_provider_api_key_env_field() {
        let path = unique_config_path("loongclaw-config-runtime-default");
        let path_string = path.display().to_string();

        write(Some(&path_string), &LoongClawConfig::default(), true)
            .expect("default config write should pass");

        let raw = fs::read_to_string(&path).expect("read written config");
        assert!(!raw.contains("api_key_env = \"OPENAI_API_KEY\""));
        assert!(!raw.contains("api_key = \"${OPENAI_API_KEY}\""));

        let _ = fs::remove_file(path);
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn write_default_config_uses_provider_profiles_and_active_provider() {
        let path = unique_config_path("loongclaw-config-runtime-profiles");
        let path_string = path.display().to_string();

        write(Some(&path_string), &LoongClawConfig::default(), true)
            .expect("default config write should pass");

        let raw = fs::read_to_string(&path).expect("read written config");
        assert!(raw.contains("active_provider = \"openai\""));
        assert!(raw.contains("[providers.openai]"));
        assert!(!raw.contains("\n[provider]\n"));

        let _ = fs::remove_file(path);
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn write_default_config_includes_durable_audit_defaults() {
        let path = unique_config_path("loongclaw-config-runtime-audit");
        let path_string = path.display().to_string();

        write(Some(&path_string), &LoongClawConfig::default(), true)
            .expect("default config write should pass");

        let raw = fs::read_to_string(&path).expect("read written config");
        assert!(raw.contains("[audit]"));
        assert!(raw.contains("mode = \"fanout\""));
        assert!(raw.contains("path = "));

        let (_, loaded) = load(Some(&path_string)).expect("config load should pass");
        assert_eq!(loaded.audit.mode, crate::config::AuditMode::Fanout);
        assert!(loaded.audit.resolved_path().ends_with("audit/events.jsonl"));

        let _ = fs::remove_file(path);
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn write_persists_provider_env_pointers_as_secret_refs() {
        let path = unique_config_path("loongclaw-config-runtime-canonical-provider-env");
        let path_string = path.display().to_string();
        let mut config = LoongClawConfig::default();
        config.provider.api_key_env = Some("OPENAI_API_KEY".to_owned());

        write(Some(&path_string), &config, true).expect("config write should pass");

        let raw = fs::read_to_string(&path).expect("read written config");
        assert!(raw.contains("api_key"));
        assert!(raw.contains("env = \"OPENAI_API_KEY\""));
        assert!(!raw.contains("api_key_env = \"OPENAI_API_KEY\""));

        let _ = fs::remove_file(path);
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn write_migrates_inline_provider_env_references_to_secret_refs() {
        let path = unique_config_path("loongclaw-config-runtime-inline-provider-env");
        let path_string = path.display().to_string();
        let mut config = LoongClawConfig::default();
        let inline_references = [
            "${TEAM_OPENAI_KEY}",
            "$TEAM_OPENAI_KEY",
            "env:TEAM_OPENAI_KEY",
            "%TEAM_OPENAI_KEY%",
        ];

        for inline_reference in inline_references {
            config.provider.api_key = Some(SecretRef::Inline(inline_reference.to_owned()));

            write(Some(&path_string), &config, true).expect("config write should pass");

            let raw = fs::read_to_string(&path).expect("read written config");
            assert!(
                raw.contains("api_key") && raw.contains("env = \"TEAM_OPENAI_KEY\""),
                "expected api_key secret-ref writeback for {inline_reference}"
            );
            assert!(
                !raw.contains(inline_reference),
                "expected inline env reference removal for {inline_reference}"
            );
        }

        let _ = fs::remove_file(path);
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn write_canonicalizes_matching_provider_env_name_fields_into_secret_refs() {
        let path = unique_config_path("loongclaw-config-runtime-trimmed-provider-env");
        let path_string = path.display().to_string();
        let mut config = LoongClawConfig::default();
        config.provider.api_key = Some(SecretRef::Inline("${TEAM_OPENAI_KEY}".to_owned()));
        config.provider.api_key_env = Some(" TEAM_OPENAI_KEY ".to_owned());

        write(Some(&path_string), &config, true).expect("config write should pass");

        let raw = fs::read_to_string(&path).expect("read written config");
        assert!(raw.contains("api_key"));
        assert!(raw.contains("env = \"TEAM_OPENAI_KEY\""));
        assert!(!raw.contains("api_key_env = \"TEAM_OPENAI_KEY\""));
        assert!(!raw.contains("api_key = \"${TEAM_OPENAI_KEY}\""));

        let _ = fs::remove_file(path);
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn write_canonicalizes_matching_wecom_env_name_fields() {
        let path = unique_config_path("loongclaw-config-runtime-trimmed-wecom-env");
        let path_string = path.display().to_string();
        let mut config = LoongClawConfig::default();
        config.wecom.bot_id = Some(SecretRef::Inline("${WECOM_BOT_ID}".to_owned()));
        config.wecom.bot_id_env = Some(" WECOM_BOT_ID ".to_owned());
        config.wecom.secret = Some(SecretRef::Inline("${WECOM_SECRET}".to_owned()));
        config.wecom.secret_env = Some(" WECOM_SECRET ".to_owned());

        write(Some(&path_string), &config, true).expect("config write should pass");

        let raw = fs::read_to_string(&path).expect("read written config");
        assert!(raw.contains("bot_id_env = \"WECOM_BOT_ID\""));
        assert!(raw.contains("secret_env = \"WECOM_SECRET\""));
        assert!(!raw.contains("bot_id_env = \" WECOM_BOT_ID \""));
        assert!(!raw.contains("secret_env = \" WECOM_SECRET \""));
        assert!(!raw.contains("bot_id = \"${WECOM_BOT_ID}\""));
        assert!(!raw.contains("secret = \"${WECOM_SECRET}\""));

        let _ = fs::remove_file(path);
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn write_canonicalizes_matching_config_backed_channel_env_name_fields() {
        let path = unique_config_path("loongclaw-config-runtime-trimmed-config-backed-env");
        let path_string = path.display().to_string();
        let mut config = LoongClawConfig::default();
        let mut ops_nostr_account = crate::config::NostrAccountConfig::default();

        config.discord.bot_token = Some(SecretRef::Inline("${DISCORD_BOT_TOKEN}".to_owned()));
        config.discord.bot_token_env = Some(" DISCORD_BOT_TOKEN ".to_owned());

        config.slack.bot_token = Some(SecretRef::Inline("${SLACK_BOT_TOKEN}".to_owned()));
        config.slack.bot_token_env = Some(" SLACK_BOT_TOKEN ".to_owned());

        config.nostr.relay_urls_env = Some(" NOSTR_RELAY_URLS ".to_owned());
        config.nostr.private_key = Some(SecretRef::Inline("${NOSTR_PRIVATE_KEY}".to_owned()));
        config.nostr.private_key_env = Some(" NOSTR_PRIVATE_KEY ".to_owned());

        ops_nostr_account.relay_urls_env = Some(" OPS_NOSTR_RELAY_URLS ".to_owned());
        ops_nostr_account.private_key =
            Some(SecretRef::Inline("${OPS_NOSTR_PRIVATE_KEY}".to_owned()));
        ops_nostr_account.private_key_env = Some(" OPS_NOSTR_PRIVATE_KEY ".to_owned());
        config
            .nostr
            .accounts
            .insert("ops".to_owned(), ops_nostr_account);

        config.whatsapp.access_token =
            Some(SecretRef::Inline("${WHATSAPP_ACCESS_TOKEN}".to_owned()));
        config.whatsapp.access_token_env = Some(" WHATSAPP_ACCESS_TOKEN ".to_owned());
        config.whatsapp.verify_token =
            Some(SecretRef::Inline("${WHATSAPP_VERIFY_TOKEN}".to_owned()));
        config.whatsapp.verify_token_env = Some(" WHATSAPP_VERIFY_TOKEN ".to_owned());
        config.whatsapp.app_secret = Some(SecretRef::Inline("${WHATSAPP_APP_SECRET}".to_owned()));
        config.whatsapp.app_secret_env = Some(" WHATSAPP_APP_SECRET ".to_owned());

        write(Some(&path_string), &config, true).expect("config write should pass");

        let raw = fs::read_to_string(&path).expect("read written config");

        assert!(raw.contains("bot_token_env = \"DISCORD_BOT_TOKEN\""));
        assert!(raw.contains("bot_token_env = \"SLACK_BOT_TOKEN\""));
        assert!(raw.contains("relay_urls_env = \"NOSTR_RELAY_URLS\""));
        assert!(raw.contains("private_key_env = \"NOSTR_PRIVATE_KEY\""));
        assert!(raw.contains("relay_urls_env = \"OPS_NOSTR_RELAY_URLS\""));
        assert!(raw.contains("private_key_env = \"OPS_NOSTR_PRIVATE_KEY\""));
        assert!(raw.contains("access_token_env = \"WHATSAPP_ACCESS_TOKEN\""));
        assert!(raw.contains("verify_token_env = \"WHATSAPP_VERIFY_TOKEN\""));
        assert!(raw.contains("app_secret_env = \"WHATSAPP_APP_SECRET\""));

        assert!(!raw.contains("bot_token = \"${DISCORD_BOT_TOKEN}\""));
        assert!(!raw.contains("bot_token = \"${SLACK_BOT_TOKEN}\""));
        assert!(!raw.contains("relay_urls_env = \" NOSTR_RELAY_URLS \""));
        assert!(!raw.contains("private_key = \"${NOSTR_PRIVATE_KEY}\""));
        assert!(!raw.contains("private_key = \"${OPS_NOSTR_PRIVATE_KEY}\""));
        assert!(!raw.contains("access_token = \"${WHATSAPP_ACCESS_TOKEN}\""));
        assert!(!raw.contains("verify_token = \"${WHATSAPP_VERIFY_TOKEN}\""));
        assert!(!raw.contains("app_secret = \"${WHATSAPP_APP_SECRET}\""));

        let _ = fs::remove_file(path);
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn write_canonicalizes_matching_plugin_backed_channel_env_name_fields() {
        let path = unique_config_path("loongclaw-config-runtime-trimmed-plugin-bridge-env");
        let path_string = path.display().to_string();
        let mut config = LoongClawConfig::default();

        config.weixin.bridge_url = Some("${WEIXIN_BRIDGE_URL}".to_owned());
        config.weixin.bridge_url_env = Some(" WEIXIN_BRIDGE_URL ".to_owned());
        config.weixin.bridge_access_token = Some(SecretRef::Inline(
            "${WEIXIN_BRIDGE_ACCESS_TOKEN}".to_owned(),
        ));
        config.weixin.bridge_access_token_env = Some(" WEIXIN_BRIDGE_ACCESS_TOKEN ".to_owned());

        config.qqbot.app_id = Some(SecretRef::Inline("${QQBOT_APP_ID}".to_owned()));
        config.qqbot.app_id_env = Some(" QQBOT_APP_ID ".to_owned());
        config.qqbot.client_secret = Some(SecretRef::Inline("${QQBOT_CLIENT_SECRET}".to_owned()));
        config.qqbot.client_secret_env = Some(" QQBOT_CLIENT_SECRET ".to_owned());

        config.onebot.websocket_url = Some("${ONEBOT_WEBSOCKET_URL}".to_owned());
        config.onebot.websocket_url_env = Some(" ONEBOT_WEBSOCKET_URL ".to_owned());
        config.onebot.access_token = Some(SecretRef::Inline("${ONEBOT_ACCESS_TOKEN}".to_owned()));
        config.onebot.access_token_env = Some(" ONEBOT_ACCESS_TOKEN ".to_owned());

        write(Some(&path_string), &config, true).expect("config write should pass");

        let raw = fs::read_to_string(&path).expect("read written config");

        assert!(raw.contains(format!("bridge_url_env = \"{WEIXIN_BRIDGE_URL_ENV}\"").as_str()));
        assert!(raw.contains(
            format!("bridge_access_token_env = \"{WEIXIN_BRIDGE_ACCESS_TOKEN_ENV}\"").as_str()
        ));
        assert!(raw.contains(format!("app_id_env = \"{QQBOT_APP_ID_ENV}\"").as_str()));
        assert!(
            raw.contains(format!("client_secret_env = \"{QQBOT_CLIENT_SECRET_ENV}\"").as_str())
        );
        assert!(
            raw.contains(format!("websocket_url_env = \"{ONEBOT_WEBSOCKET_URL_ENV}\"").as_str())
        );
        assert!(raw.contains(format!("access_token_env = \"{ONEBOT_ACCESS_TOKEN_ENV}\"").as_str()));

        assert!(!raw.contains("bridge_url = \"${WEIXIN_BRIDGE_URL}\""));
        assert!(!raw.contains("bridge_access_token = \"${WEIXIN_BRIDGE_ACCESS_TOKEN}\""));
        assert!(!raw.contains("app_id = \"${QQBOT_APP_ID}\""));
        assert!(!raw.contains("client_secret = \"${QQBOT_CLIENT_SECRET}\""));
        assert!(!raw.contains("websocket_url = \"${ONEBOT_WEBSOCKET_URL}\""));
        assert!(!raw.contains("access_token = \"${ONEBOT_ACCESS_TOKEN}\""));

        let _ = fs::remove_file(path);
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn write_preserves_provider_env_binding_when_inline_secret_is_blank() {
        let path = unique_config_path("loongclaw-config-runtime-blank-inline-provider-env");
        let path_string = path.display().to_string();
        let mut config = LoongClawConfig::default();
        config.provider.api_key = Some(SecretRef::Inline("   ".to_owned()));
        config.provider.api_key_env = Some("TEAM_OPENAI_KEY".to_owned());

        write(Some(&path_string), &config, true).expect("config write should pass");

        let raw = fs::read_to_string(&path).expect("read written config");
        assert!(raw.contains("api_key"));
        assert!(raw.contains("env = \"TEAM_OPENAI_KEY\""));
        assert!(!raw.contains("api_key = \"   \""));
        assert!(!raw.contains("api_key_env = \"TEAM_OPENAI_KEY\""));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn resolve_provider_switch_target_prefers_profile_id_then_kind_default() {
        let mut config = LoongClawConfig::default();
        config.set_active_provider_profile(
            "openai-main",
            ProviderProfileConfig {
                default_for_kind: false,
                provider: ProviderConfig {
                    kind: ProviderKind::Openai,
                    model: "gpt-5".to_owned(),
                    ..ProviderConfig::default()
                },
            },
        );
        config.providers.insert(
            "openai-reasoning".to_owned(),
            ProviderProfileConfig {
                default_for_kind: true,
                provider: ProviderConfig {
                    kind: ProviderKind::Openai,
                    model: "o4-mini".to_owned(),
                    ..ProviderConfig::default()
                },
            },
        );
        config.providers.insert(
            "deepseek-cn".to_owned(),
            ProviderProfileConfig {
                default_for_kind: true,
                provider: ProviderConfig {
                    kind: ProviderKind::Deepseek,
                    model: "deepseek-chat".to_owned(),
                    ..ProviderConfig::default()
                },
            },
        );

        assert_eq!(
            config.resolve_provider_switch_target("openai-reasoning"),
            Ok("openai-reasoning".to_owned())
        );
        assert_eq!(
            config.resolve_provider_switch_target("openai"),
            Ok("openai-reasoning".to_owned())
        );
        assert_eq!(
            config.resolve_provider_switch_target("deepseek"),
            Ok("deepseek-cn".to_owned())
        );
    }

    #[test]
    fn resolve_provider_switch_target_accepts_unique_model_selector() {
        let mut config = LoongClawConfig::default();
        config.set_active_provider_profile(
            "openai-main",
            ProviderProfileConfig {
                default_for_kind: true,
                provider: ProviderConfig {
                    kind: ProviderKind::Openai,
                    model: "gpt-5".to_owned(),
                    ..ProviderConfig::default()
                },
            },
        );
        config.providers.insert(
            "deepseek-cn".to_owned(),
            ProviderProfileConfig {
                default_for_kind: true,
                provider: ProviderConfig {
                    kind: ProviderKind::Deepseek,
                    model: "deepseek-chat".to_owned(),
                    ..ProviderConfig::default()
                },
            },
        );

        assert_eq!(
            config.resolve_provider_switch_target("gpt-5"),
            Ok("openai-main".to_owned())
        );
        assert_eq!(
            config.resolve_provider_switch_target("deepseek-chat"),
            Ok("deepseek-cn".to_owned())
        );
    }

    #[test]
    fn resolve_provider_switch_target_accepts_unique_model_suffix_selector() {
        let mut config = LoongClawConfig::default();
        config.set_active_provider_profile(
            "openrouter-main",
            ProviderProfileConfig {
                default_for_kind: true,
                provider: ProviderConfig {
                    kind: ProviderKind::Openrouter,
                    model: "openai/gpt-5.1-codex".to_owned(),
                    ..ProviderConfig::default()
                },
            },
        );
        config.providers.insert(
            "deepseek-cn".to_owned(),
            ProviderProfileConfig {
                default_for_kind: true,
                provider: ProviderConfig {
                    kind: ProviderKind::Deepseek,
                    model: "deepseek-chat".to_owned(),
                    ..ProviderConfig::default()
                },
            },
        );

        assert_eq!(
            config.resolve_provider_switch_target("gpt-5.1-codex"),
            Ok("openrouter-main".to_owned())
        );
    }

    #[test]
    fn resolve_provider_switch_target_rejects_ambiguous_kind_without_default() {
        let mut config = LoongClawConfig::default();
        config.set_active_provider_profile(
            "openai-main",
            ProviderProfileConfig {
                default_for_kind: false,
                provider: ProviderConfig {
                    kind: ProviderKind::Openai,
                    model: "gpt-5".to_owned(),
                    ..ProviderConfig::default()
                },
            },
        );
        config.providers.insert(
            "openai-azure".to_owned(),
            ProviderProfileConfig {
                default_for_kind: false,
                provider: ProviderConfig {
                    kind: ProviderKind::Openai,
                    model: "gpt-4.1".to_owned(),
                    ..ProviderConfig::default()
                },
            },
        );

        let error = config
            .resolve_provider_switch_target("openai")
            .expect_err("ambiguous same-kind provider switch should require clarification");
        assert!(error.contains("ambiguous"));
        assert!(error.contains("openai-main"));
        assert!(error.contains("openai-azure"));
        assert!(error.contains("model=gpt-5"));
        assert!(error.contains("selectors=openai-main, gpt-5"));
        assert!(error.contains("model=gpt-4.1"));
        assert!(error.contains("selectors=openai-azure, gpt-4.1"));
    }

    #[test]
    fn preferred_provider_selector_prefers_human_friendly_aliases() {
        let profiles = [
            ProviderSelectorProfileRef::new("openai-main", ProviderKind::Openai, "gpt-5", false),
            ProviderSelectorProfileRef::new(
                "openai-reasoning",
                ProviderKind::Openai,
                "o4-mini",
                true,
            ),
            ProviderSelectorProfileRef::new(
                "openrouter-main",
                ProviderKind::Openrouter,
                "openai/gpt-5.1-codex",
                true,
            ),
        ];

        assert_eq!(
            preferred_provider_selector(profiles.iter().copied(), "openai-main"),
            Some("gpt-5".to_owned())
        );
        assert_eq!(
            preferred_provider_selector(profiles.iter().copied(), "openai-reasoning"),
            Some("openai".to_owned())
        );
        assert_eq!(
            preferred_provider_selector(profiles.iter().copied(), "openrouter-main"),
            Some("openrouter".to_owned())
        );
    }

    #[test]
    fn provider_selector_recommendation_hint_prefers_human_friendly_aliases() {
        let profiles = [
            ProviderSelectorProfileRef::new("openai-main", ProviderKind::Openai, "gpt-5", false),
            ProviderSelectorProfileRef::new(
                "openai-reasoning",
                ProviderKind::Openai,
                "o4-mini",
                true,
            ),
            ProviderSelectorProfileRef::new(
                "deepseek-cn",
                ProviderKind::Deepseek,
                "deepseek-chat",
                true,
            ),
        ];

        assert_eq!(
            provider_selector_recommendation_hint(
                profiles.iter().copied(),
                ["openai-reasoning", "openai-main", "deepseek-cn"],
            ),
            Some("try one of: openai, gpt-5, deepseek".to_owned())
        );
    }

    #[test]
    fn resolve_provider_switch_target_unknown_selector_lists_accepted_selectors() {
        let mut config = LoongClawConfig::default();
        config.set_active_provider_profile(
            "openai-main",
            ProviderProfileConfig {
                default_for_kind: true,
                provider: ProviderConfig {
                    kind: ProviderKind::Openai,
                    model: "gpt-5".to_owned(),
                    ..ProviderConfig::default()
                },
            },
        );
        config.providers.insert(
            "deepseek-cn".to_owned(),
            ProviderProfileConfig {
                default_for_kind: true,
                provider: ProviderConfig {
                    kind: ProviderKind::Deepseek,
                    model: "deepseek-chat".to_owned(),
                    ..ProviderConfig::default()
                },
            },
        );

        let error = config
            .resolve_provider_switch_target("unknown-provider")
            .expect_err("unknown selector should surface accepted selectors");
        assert!(error.contains("accepted selectors"));
        assert!(error.contains("try one of:"));
        assert!(error.contains("openai-main"));
        assert!(error.contains("gpt-5"));
        assert!(error.contains("openai"));
        assert!(error.contains("deepseek-cn"));
        assert!(error.contains("deepseek-chat"));
        assert!(error.contains("deepseek"));
    }

    #[test]
    fn switch_active_provider_updates_last_provider_and_kind_default() {
        let mut config = LoongClawConfig::default();
        config.set_active_provider_profile(
            "openai-main",
            ProviderProfileConfig {
                default_for_kind: true,
                provider: ProviderConfig {
                    kind: ProviderKind::Openai,
                    model: "gpt-5".to_owned(),
                    ..ProviderConfig::default()
                },
            },
        );
        config.providers.insert(
            "openai-reasoning".to_owned(),
            ProviderProfileConfig {
                default_for_kind: false,
                provider: ProviderConfig {
                    kind: ProviderKind::Openai,
                    model: "o4-mini".to_owned(),
                    ..ProviderConfig::default()
                },
            },
        );
        config.providers.insert(
            "deepseek-cn".to_owned(),
            ProviderProfileConfig {
                default_for_kind: true,
                provider: ProviderConfig {
                    kind: ProviderKind::Deepseek,
                    model: "deepseek-chat".to_owned(),
                    ..ProviderConfig::default()
                },
            },
        );

        let selected = config
            .switch_active_provider("openai-reasoning")
            .expect("profile switch should succeed");

        assert_eq!(selected, "openai-reasoning");
        assert_eq!(config.active_provider_id(), Some("openai-reasoning"));
        assert_eq!(config.last_provider_id(), Some("openai-main"));
        assert_eq!(config.provider.kind, ProviderKind::Openai);
        assert_eq!(config.provider.model, "o4-mini");
        assert!(
            config
                .providers
                .get("openai-reasoning")
                .expect("new active profile")
                .default_for_kind
        );
        assert!(
            !config
                .providers
                .get("openai-main")
                .expect("old active profile")
                .default_for_kind
        );
        assert_eq!(
            config.resolve_provider_switch_target("openai"),
            Ok("openai-reasoning".to_owned())
        );
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
        assert!(raw.contains("auto_expose_installed = false"));

        let (_, loaded) = load(Some(&path_string)).expect("config load should pass");
        assert!(!loaded.external_skills.enabled);
        assert!(loaded.external_skills.require_download_approval);
        assert!(loaded.external_skills.allowed_domains.is_empty());
        assert_eq!(
            loaded.external_skills.blocked_domains,
            vec!["*.clawhub.io".to_owned()]
        );
        assert!(loaded.external_skills.install_root.is_none());
        assert!(!loaded.external_skills.auto_expose_installed);

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

    #[test]
    #[cfg(feature = "config-toml")]
    fn tool_config_round_trips_session_and_delegate_settings() {
        let mut config = LoongClawConfig::default();
        config.tools.sessions.visibility = crate::config::tools::SessionVisibility::SelfOnly;
        config.tools.sessions.list_limit = 12;
        config.tools.sessions.history_limit = 34;
        config.tools.messages.enabled = true;
        config.tools.delegate.enabled = false;
        config.tools.delegate.max_depth = 2;
        config.tools.delegate.timeout_seconds = 90;
        config.tools.delegate.allow_shell_in_child = true;
        config.tools.delegate.child_tool_allowlist =
            vec!["file.read".to_owned(), "shell.exec".to_owned()];

        let encoded = encode_toml_config(&config).expect("encode config");
        let parsed = toml::from_str::<LoongClawConfig>(&encoded).expect("parse encoded config");

        assert_eq!(parsed.tools.sessions, config.tools.sessions);
        assert_eq!(parsed.tools.messages, config.tools.messages);
        assert_eq!(parsed.tools.delegate, config.tools.delegate);
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn audit_config_round_trips_mode_and_path_settings() {
        let mut config = LoongClawConfig::default();
        config.audit.mode = crate::config::AuditMode::Jsonl;
        config.audit.path = "~/.loongclaw/audit/custom-events.jsonl".to_owned();
        config.audit.retain_in_memory = false;

        let encoded = encode_toml_config(&config).expect("encode config");
        let parsed = toml::from_str::<LoongClawConfig>(&encoded).expect("parse encoded config");

        assert_eq!(parsed.audit, config.audit);
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn outbound_http_config_round_trips_private_host_override() {
        let mut config = LoongClawConfig::default();
        config.outbound_http.allow_private_hosts = true;

        let encoded = encode_toml_config(&config).expect("encode config");
        let parsed = toml::from_str::<LoongClawConfig>(&encoded).expect("parse encoded config");

        assert!(parsed.outbound_http.allow_private_hosts);
    }
}
