use std::{collections::BTreeMap, env, path::PathBuf};

use serde::{Deserialize, Deserializer, Serialize};

use super::shared::{
    ConfigValidationIssue, EnvPointerValidationHint, default_loongclaw_home, expand_path,
    parse_explicit_env_reference, validate_env_pointer_field,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderProfile {
    pub kind: ProviderKind,
    pub id: &'static str,
    pub aliases: &'static [&'static str],
    pub base_url: &'static str,
    pub chat_completions_path: &'static str,
    pub models_path: Option<&'static str>,
    pub protocol_family: ProviderProtocolFamily,
    pub auth_scheme: ProviderAuthScheme,
    pub default_headers: &'static [(&'static str, &'static str)],
    pub default_api_key_env: Option<&'static str>,
    pub api_key_env_aliases: &'static [&'static str],
    pub default_user_agent: Option<&'static str>,
    pub default_oauth_access_token_env: Option<&'static str>,
    pub oauth_access_token_env_aliases: &'static [&'static str],
    pub feature_family: ProviderFeatureFamily,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderProtocolFamily {
    OpenAiChatCompletions,
    AnthropicMessages,
    BedrockConverse,
}

impl ProviderProtocolFamily {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiChatCompletions => "openai_chat_completions",
            Self::AnthropicMessages => "anthropic_messages",
            Self::BedrockConverse => "bedrock_converse",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderAuthScheme {
    Bearer,
    XApiKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderFeatureFamily {
    OpenAiCompatible,
    Anthropic,
    Bedrock,
    Volcengine,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderWireApi {
    #[default]
    ChatCompletions,
    Responses,
}

impl ProviderWireApi {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ChatCompletions => "chat_completions",
            Self::Responses => "responses",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "chat_completions" => Some(Self::ChatCompletions),
            "responses" => Some(Self::Responses),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderTransportReadinessLevel {
    Ready,
    Review,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderTransportReadiness {
    pub level: ProviderTransportReadinessLevel,
    pub summary: String,
    pub detail: String,
    pub auto_fallback_to_chat_completions: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderTransportFallback {
    pub wire_api: ProviderWireApi,
    pub endpoint: String,
    pub provider: ProviderConfig,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderTransportPolicy {
    pub request_wire_api: ProviderWireApi,
    pub request_endpoint: String,
    pub models_endpoint: String,
    pub readiness: ProviderTransportReadiness,
    pub fallback: Option<ProviderTransportFallback>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelCatalogProbeRecovery {
    ExplicitModel(String),
    ConfiguredPreferredModels(Vec<String>),
    RequiresExplicitModel {
        recommended_onboarding_model: Option<&'static str>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProviderRegionEndpointVariant {
    label: &'static str,
    base_url: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProviderRegionEndpointGuide {
    family_label: &'static str,
    default_variant: ProviderRegionEndpointVariant,
    alternate_variant: ProviderRegionEndpointVariant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProviderRegionEndpointSelection {
    BaseUrl(String),
    Endpoint(String),
    ModelsEndpoint(String),
    EndpointAndModels {
        endpoint: String,
        models_endpoint: String,
    },
}

impl ProviderRegionEndpointGuide {
    fn note(self, provider: &ProviderConfig) -> String {
        match self.selection(provider) {
            ProviderRegionEndpointSelection::BaseUrl(resolved_base_url) => {
                self.base_url_note(provider, resolved_base_url.as_str())
            }
            ProviderRegionEndpointSelection::Endpoint(endpoint) => {
                self.override_note("provider.endpoint", endpoint.as_str())
            }
            ProviderRegionEndpointSelection::ModelsEndpoint(models_endpoint) => {
                self.override_note("provider.models_endpoint", models_endpoint.as_str())
            }
            ProviderRegionEndpointSelection::EndpointAndModels {
                endpoint,
                models_endpoint,
            } => format!(
                "{} region endpoint: explicit endpoint overrides are in use (`provider.endpoint` = `{endpoint}`, `provider.models_endpoint` = `{models_endpoint}`); official {} endpoint `{}`; official {} endpoint `{}`",
                self.family_label,
                self.default_variant.label,
                self.default_variant.base_url,
                self.alternate_variant.label,
                self.alternate_variant.base_url
            ),
        }
    }

    fn failure_hint(self, provider: &ProviderConfig) -> String {
        match self.selection(provider) {
            ProviderRegionEndpointSelection::BaseUrl(_) => self.base_url_failure_hint(),
            ProviderRegionEndpointSelection::Endpoint(endpoint) => {
                self.override_failure_hint("provider.endpoint", endpoint.as_str())
            }
            ProviderRegionEndpointSelection::ModelsEndpoint(models_endpoint) => {
                self.override_failure_hint("provider.models_endpoint", models_endpoint.as_str())
            }
            ProviderRegionEndpointSelection::EndpointAndModels {
                endpoint,
                models_endpoint,
            } => format!(
                "{} keys can be region-scoped. Verify the explicit endpoint overrides match your account region: use `{}` for {} accounts or `{}` for {} accounts. Changing `provider.base_url` alone will not affect `provider.endpoint` (`{endpoint}`) or `provider.models_endpoint` (`{models_endpoint}`).",
                self.family_label,
                self.default_variant.base_url,
                self.default_variant.label,
                self.alternate_variant.base_url,
                self.alternate_variant.label
            ),
        }
    }

    fn request_failure_hint(self, provider: &ProviderConfig) -> String {
        if provider.endpoint_explicit {
            return self.override_failure_hint("provider.endpoint", provider.endpoint().as_str());
        }

        self.base_url_failure_hint()
    }

    fn selection(self, provider: &ProviderConfig) -> ProviderRegionEndpointSelection {
        match (
            provider.endpoint_explicit,
            provider.models_endpoint_explicit,
        ) {
            (true, true) => ProviderRegionEndpointSelection::EndpointAndModels {
                endpoint: provider.endpoint(),
                models_endpoint: provider.models_endpoint(),
            },
            (true, false) => ProviderRegionEndpointSelection::Endpoint(provider.endpoint()),
            (false, true) => {
                ProviderRegionEndpointSelection::ModelsEndpoint(provider.models_endpoint())
            }
            (false, false) => {
                ProviderRegionEndpointSelection::BaseUrl(provider.resolved_base_url())
            }
        }
    }

    fn base_url_note(self, provider: &ProviderConfig, resolved_base_url: &str) -> String {
        if is_same_base_url(resolved_base_url, self.alternate_variant.base_url) {
            return format!(
                "{} region endpoint: using {} endpoint (`{}`); use `{}` for {} accounts",
                self.family_label,
                self.alternate_variant.label,
                self.alternate_variant.base_url,
                self.default_variant.base_url,
                self.default_variant.label
            );
        }
        if is_same_base_url(resolved_base_url, self.default_variant.base_url)
            || provider.base_url_is_profile_default_like()
        {
            return format!(
                "{} region endpoint: {} default (`{}`); switch `provider.base_url` to `{}` for {} accounts",
                self.family_label,
                self.default_variant.label,
                self.default_variant.base_url,
                self.alternate_variant.base_url,
                self.alternate_variant.label
            );
        }

        format!(
            "{} region endpoint: using custom endpoint (`{}`); official {} endpoint `{}`; official {} endpoint `{}`",
            self.family_label,
            resolved_base_url,
            self.default_variant.label,
            self.default_variant.base_url,
            self.alternate_variant.label,
            self.alternate_variant.base_url
        )
    }

    fn override_note(self, field_name: &str, endpoint: &str) -> String {
        if let Some(active_variant) = self.override_variant(endpoint) {
            let alternate_variant = if active_variant == self.default_variant {
                self.alternate_variant
            } else {
                self.default_variant
            };
            return format!(
                "{} region endpoint: using explicit `{field_name}` {} endpoint (`{endpoint}`); use `{}` for {} accounts",
                self.family_label,
                active_variant.label,
                alternate_variant.base_url,
                alternate_variant.label
            );
        }

        format!(
            "{} region endpoint: using explicit `{field_name}` (`{endpoint}`); official {} endpoint `{}`; official {} endpoint `{}`",
            self.family_label,
            self.default_variant.label,
            self.default_variant.base_url,
            self.alternate_variant.label,
            self.alternate_variant.base_url
        )
    }

    fn base_url_failure_hint(self) -> String {
        format!(
            "{} keys can be region-scoped. Verify `provider.base_url` matches your account region: use `{}` for {} accounts or `{}` for {} accounts.",
            self.family_label,
            self.default_variant.base_url,
            self.default_variant.label,
            self.alternate_variant.base_url,
            self.alternate_variant.label
        )
    }

    fn override_failure_hint(self, field_name: &str, endpoint: &str) -> String {
        format!(
            "{} keys can be region-scoped. Verify explicit `{field_name}` matches your account region: use `{}` for {} accounts or `{}` for {} accounts. Changing `provider.base_url` alone will not affect `{field_name}` (`{endpoint}`).",
            self.family_label,
            self.default_variant.base_url,
            self.default_variant.label,
            self.alternate_variant.base_url,
            self.alternate_variant.label
        )
    }

    fn override_variant(self, endpoint: &str) -> Option<ProviderRegionEndpointVariant> {
        if matches_region_endpoint_url(endpoint, self.default_variant.base_url) {
            return Some(self.default_variant);
        }
        if matches_region_endpoint_url(endpoint, self.alternate_variant.base_url) {
            return Some(self.alternate_variant);
        }
        None
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
}

impl ReasoningEffort {
    pub const fn as_str(self) -> &'static str {
        match self {
            ReasoningEffort::None => "none",
            ReasoningEffort::Minimal => "minimal",
            ReasoningEffort::Low => "low",
            ReasoningEffort::Medium => "medium",
            ReasoningEffort::High => "high",
            ReasoningEffort::Xhigh => "xhigh",
        }
    }
}

const COHERE_REASONING_EFFORTS: &[ReasoningEffort] =
    &[ReasoningEffort::None, ReasoningEffort::High];
const ARK_REASONING_EFFORTS: &[ReasoningEffort] = &[
    ReasoningEffort::None,
    ReasoningEffort::Minimal,
    ReasoningEffort::Low,
    ReasoningEffort::Medium,
    ReasoningEffort::High,
];
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    #[serde(alias = "anthropic_compatible")]
    Anthropic,
    #[serde(alias = "aws-bedrock", alias = "aws_bedrock")]
    Bedrock,
    #[serde(alias = "byteplus_compatible")]
    Byteplus,
    #[serde(alias = "byteplus_coding_compatible")]
    ByteplusCoding,
    #[serde(alias = "cerebras_compatible")]
    Cerebras,
    #[serde(
        alias = "cloudflare_ai",
        alias = "cloudflare-ai",
        alias = "cloudflare_ai_gateway",
        alias = "cloudflare-ai-gateway"
    )]
    CloudflareAiGateway,
    #[serde(alias = "cohere_compatible")]
    Cohere,
    #[serde(alias = "openai_custom", alias = "custom_openai")]
    Custom,
    #[serde(
        alias = "gemini_compatible",
        alias = "google",
        alias = "google_gemini",
        alias = "google-gemini"
    )]
    Gemini,
    #[serde(alias = "kimi_compatible")]
    #[serde(alias = "moonshot", alias = "moonshot_compatible")]
    Kimi,
    #[serde(alias = "kimi_coding_compatible")]
    KimiCoding,
    #[serde(alias = "groq_compatible")]
    Groq,
    #[serde(alias = "fireworks_compatible")]
    Fireworks,
    #[serde(alias = "mistral_compatible")]
    Mistral,
    #[serde(alias = "minimax_compatible")]
    Minimax,
    #[serde(alias = "novita_compatible")]
    Novita,
    #[serde(alias = "nvidia_compatible", alias = "nvidia_nim")]
    Nvidia,
    #[serde(alias = "llama.cpp", alias = "llama_cpp")]
    Llamacpp,
    #[serde(alias = "lmstudio", alias = "lm-studio")]
    LmStudio,
    #[serde(alias = "ollama_compatible")]
    Ollama,
    #[default]
    #[serde(alias = "openai_compatible")]
    Openai,
    #[serde(alias = "openrouter_compatible")]
    Openrouter,
    #[serde(alias = "perplexity_compatible")]
    Perplexity,
    #[serde(alias = "qianfan_compatible", alias = "baidu")]
    Qianfan,
    #[serde(alias = "qwen_compatible", alias = "dashscope")]
    Qwen,
    #[serde(alias = "sambanova_compatible", alias = "samba_nova")]
    Sambanova,
    #[serde(alias = "sglang_compatible")]
    Sglang,
    #[serde(alias = "siliconflow_compatible")]
    Siliconflow,
    #[serde(alias = "stepfun_compatible")]
    Stepfun,
    #[serde(alias = "together_compatible", alias = "together_ai")]
    Together,
    #[serde(alias = "venice_compatible")]
    Venice,
    #[serde(
        alias = "vercel_ai",
        alias = "vercel-ai",
        alias = "vercel_ai_gateway",
        alias = "vercel-ai-gateway"
    )]
    VercelAiGateway,
    #[serde(alias = "volcengine_custom", alias = "volcengine_compatible")]
    Volcengine,
    #[serde(alias = "volcengine_coding_compatible")]
    VolcengineCoding,
    #[serde(alias = "xai_compatible")]
    Xai,
    #[serde(alias = "zai_compatible")]
    Zai,
    #[serde(alias = "zhipu_compatible")]
    Zhipu,
    #[serde(alias = "deepseek_compatible")]
    Deepseek,
    #[serde(alias = "vllm_compatible")]
    Vllm,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderProfileStateBackendKind {
    #[default]
    File,
    Sqlite,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderProfileHealthModeConfig {
    #[default]
    ProviderDefault,
    Enforce,
    ObserveOnly,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderToolSchemaModeConfig {
    #[default]
    ProviderDefault,
    Disabled,
    EnabledStrict,
    EnabledWithDowngrade,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderReasoningExtraBodyModeConfig {
    #[default]
    ProviderDefault,
    Omit,
    KimiThinking,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ProviderConfig {
    #[serde(default)]
    pub kind: ProviderKind,
    #[serde(default = "default_provider_model")]
    pub model: String,
    #[serde(default = "default_provider_base_url")]
    pub base_url: String,
    #[serde(skip_serializing, default)]
    pub base_url_explicit: bool,
    #[serde(default)]
    pub wire_api: ProviderWireApi,
    #[serde(default = "default_openai_chat_path")]
    pub chat_completions_path: String,
    #[serde(skip_serializing, default)]
    pub chat_completions_path_explicit: bool,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(skip_serializing, default)]
    pub endpoint_explicit: bool,
    #[serde(default)]
    pub models_endpoint: Option<String>,
    #[serde(skip_serializing, default)]
    pub models_endpoint_explicit: bool,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(skip_serializing, default)]
    pub api_key_env_explicit: bool,
    #[serde(default)]
    pub oauth_access_token: Option<String>,
    #[serde(default)]
    pub oauth_access_token_env: Option<String>,
    #[serde(skip_serializing, default)]
    pub oauth_access_token_env_explicit: bool,
    #[serde(default)]
    pub preferred_models: Vec<String>,
    #[serde(default)]
    pub reasoning_effort: Option<ReasoningEffort>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default = "default_provider_timeout_ms")]
    pub request_timeout_ms: u64,
    #[serde(default = "default_provider_retry_max_attempts")]
    pub retry_max_attempts: usize,
    #[serde(default = "default_provider_retry_initial_backoff_ms")]
    pub retry_initial_backoff_ms: u64,
    #[serde(default = "default_provider_retry_max_backoff_ms")]
    pub retry_max_backoff_ms: u64,
    #[serde(default = "default_model_catalog_cache_ttl_ms")]
    pub model_catalog_cache_ttl_ms: u64,
    #[serde(default = "default_model_catalog_stale_if_error_ms")]
    pub model_catalog_stale_if_error_ms: u64,
    #[serde(default = "default_model_catalog_cache_max_entries")]
    pub model_catalog_cache_max_entries: usize,
    #[serde(default = "default_model_candidate_cooldown_ms")]
    pub model_candidate_cooldown_ms: u64,
    #[serde(default = "default_model_candidate_cooldown_max_ms")]
    pub model_candidate_cooldown_max_ms: u64,
    #[serde(default = "default_model_candidate_cooldown_max_entries")]
    pub model_candidate_cooldown_max_entries: usize,
    #[serde(default = "default_profile_cooldown_ms")]
    pub profile_cooldown_ms: u64,
    #[serde(default = "default_profile_cooldown_max_ms")]
    pub profile_cooldown_max_ms: u64,
    #[serde(default = "default_profile_auth_reject_disable_ms")]
    pub profile_auth_reject_disable_ms: u64,
    #[serde(default = "default_profile_state_max_entries")]
    pub profile_state_max_entries: usize,
    #[serde(default)]
    pub profile_state_backend: ProviderProfileStateBackendKind,
    #[serde(default)]
    pub profile_state_sqlite_path: Option<String>,
    #[serde(default)]
    pub profile_health_mode: ProviderProfileHealthModeConfig,
    #[serde(default)]
    pub tool_schema_mode: ProviderToolSchemaModeConfig,
    #[serde(default)]
    pub reasoning_extra_body_mode: ProviderReasoningExtraBodyModeConfig,
    #[serde(default)]
    pub tool_schema_disabled_model_hints: Vec<String>,
    #[serde(default)]
    pub tool_schema_strict_model_hints: Vec<String>,
    #[serde(default)]
    pub reasoning_extra_body_kimi_model_hints: Vec<String>,
    #[serde(default)]
    pub reasoning_extra_body_omit_model_hints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ProviderProfileConfig {
    #[serde(default)]
    pub default_for_kind: bool,
    #[serde(flatten)]
    pub provider: ProviderConfig,
}

impl ProviderProfileConfig {
    pub fn from_provider(provider: ProviderConfig) -> Self {
        Self {
            default_for_kind: false,
            provider,
        }
    }
}
impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            kind: ProviderKind::Openai,
            model: default_provider_model(),
            base_url: default_provider_base_url(),
            base_url_explicit: false,
            wire_api: ProviderWireApi::ChatCompletions,
            chat_completions_path: default_openai_chat_path(),
            chat_completions_path_explicit: false,
            endpoint: None,
            endpoint_explicit: false,
            models_endpoint: None,
            models_endpoint_explicit: false,
            api_key: None,
            api_key_env: None,
            api_key_env_explicit: false,
            oauth_access_token: None,
            oauth_access_token_env: None,
            oauth_access_token_env_explicit: false,
            preferred_models: Vec::new(),
            reasoning_effort: None,
            headers: BTreeMap::new(),
            temperature: default_temperature(),
            max_tokens: None,
            request_timeout_ms: default_provider_timeout_ms(),
            retry_max_attempts: default_provider_retry_max_attempts(),
            retry_initial_backoff_ms: default_provider_retry_initial_backoff_ms(),
            retry_max_backoff_ms: default_provider_retry_max_backoff_ms(),
            model_catalog_cache_ttl_ms: default_model_catalog_cache_ttl_ms(),
            model_catalog_stale_if_error_ms: default_model_catalog_stale_if_error_ms(),
            model_catalog_cache_max_entries: default_model_catalog_cache_max_entries(),
            model_candidate_cooldown_ms: default_model_candidate_cooldown_ms(),
            model_candidate_cooldown_max_ms: default_model_candidate_cooldown_max_ms(),
            model_candidate_cooldown_max_entries: default_model_candidate_cooldown_max_entries(),
            profile_cooldown_ms: default_profile_cooldown_ms(),
            profile_cooldown_max_ms: default_profile_cooldown_max_ms(),
            profile_auth_reject_disable_ms: default_profile_auth_reject_disable_ms(),
            profile_state_max_entries: default_profile_state_max_entries(),
            profile_state_backend: ProviderProfileStateBackendKind::default(),
            profile_state_sqlite_path: None,
            profile_health_mode: ProviderProfileHealthModeConfig::default(),
            tool_schema_mode: ProviderToolSchemaModeConfig::default(),
            reasoning_extra_body_mode: ProviderReasoningExtraBodyModeConfig::default(),
            tool_schema_disabled_model_hints: Vec::new(),
            tool_schema_strict_model_hints: Vec::new(),
            reasoning_extra_body_kimi_model_hints: Vec::new(),
            reasoning_extra_body_omit_model_hints: Vec::new(),
        }
    }
}

impl<'de> Deserialize<'de> for ProviderConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct ProviderConfigDe {
            #[serde(default)]
            kind: ProviderKind,
            #[serde(default = "default_provider_model")]
            model: String,
            #[serde(default)]
            base_url: Option<String>,
            #[serde(default)]
            wire_api: ProviderWireApi,
            #[serde(default)]
            chat_completions_path: Option<String>,
            #[serde(default)]
            endpoint: Option<String>,
            #[serde(default)]
            models_endpoint: Option<String>,
            #[serde(default)]
            api_key: Option<String>,
            #[serde(default)]
            api_key_env: Option<String>,
            #[serde(default)]
            oauth_access_token: Option<String>,
            #[serde(default)]
            oauth_access_token_env: Option<String>,
            #[serde(default)]
            preferred_models: Vec<String>,
            #[serde(default)]
            reasoning_effort: Option<ReasoningEffort>,
            #[serde(default)]
            headers: BTreeMap<String, String>,
            #[serde(default = "default_temperature")]
            temperature: f64,
            #[serde(default)]
            max_tokens: Option<u32>,
            #[serde(default = "default_provider_timeout_ms")]
            request_timeout_ms: u64,
            #[serde(default = "default_provider_retry_max_attempts")]
            retry_max_attempts: usize,
            #[serde(default = "default_provider_retry_initial_backoff_ms")]
            retry_initial_backoff_ms: u64,
            #[serde(default = "default_provider_retry_max_backoff_ms")]
            retry_max_backoff_ms: u64,
            #[serde(default = "default_model_catalog_cache_ttl_ms")]
            model_catalog_cache_ttl_ms: u64,
            #[serde(default = "default_model_catalog_stale_if_error_ms")]
            model_catalog_stale_if_error_ms: u64,
            #[serde(default = "default_model_catalog_cache_max_entries")]
            model_catalog_cache_max_entries: usize,
            #[serde(default = "default_model_candidate_cooldown_ms")]
            model_candidate_cooldown_ms: u64,
            #[serde(default = "default_model_candidate_cooldown_max_ms")]
            model_candidate_cooldown_max_ms: u64,
            #[serde(default = "default_model_candidate_cooldown_max_entries")]
            model_candidate_cooldown_max_entries: usize,
            #[serde(default = "default_profile_cooldown_ms")]
            profile_cooldown_ms: u64,
            #[serde(default = "default_profile_cooldown_max_ms")]
            profile_cooldown_max_ms: u64,
            #[serde(default = "default_profile_auth_reject_disable_ms")]
            profile_auth_reject_disable_ms: u64,
            #[serde(default = "default_profile_state_max_entries")]
            profile_state_max_entries: usize,
            #[serde(default)]
            profile_state_backend: ProviderProfileStateBackendKind,
            #[serde(default)]
            profile_health_mode: ProviderProfileHealthModeConfig,
            #[serde(default)]
            tool_schema_mode: ProviderToolSchemaModeConfig,
            #[serde(default)]
            reasoning_extra_body_mode: ProviderReasoningExtraBodyModeConfig,
            #[serde(default)]
            tool_schema_disabled_model_hints: Vec<String>,
            #[serde(default)]
            tool_schema_strict_model_hints: Vec<String>,
            #[serde(default)]
            reasoning_extra_body_kimi_model_hints: Vec<String>,
            #[serde(default)]
            reasoning_extra_body_omit_model_hints: Vec<String>,
            #[serde(default)]
            profile_state_sqlite_path: Option<String>,
        }

        let raw = ProviderConfigDe::deserialize(deserializer)?;
        let base_url_explicit = raw
            .base_url
            .as_deref()
            .map(|value| is_explicit_base_url(raw.kind, value))
            .unwrap_or(false);
        let chat_completions_path_explicit = raw
            .chat_completions_path
            .as_deref()
            .map(|value| is_explicit_chat_completions_path(raw.kind, value))
            .unwrap_or(false);
        let base_url = raw.base_url.unwrap_or_else(default_provider_base_url);
        let chat_completions_path = raw
            .chat_completions_path
            .unwrap_or_else(default_openai_chat_path);
        let api_key_env_explicit = raw
            .api_key_env
            .as_deref()
            .map(|value| is_explicit_api_key_env_name(raw.kind, value))
            .unwrap_or(false);
        let oauth_access_token_env_explicit = raw
            .oauth_access_token_env
            .as_deref()
            .map(|value| is_explicit_oauth_access_token_env_name(raw.kind, value))
            .unwrap_or(false);

        let mut config = Self {
            kind: raw.kind,
            model: raw.model,
            base_url,
            base_url_explicit,
            wire_api: raw.wire_api,
            chat_completions_path,
            chat_completions_path_explicit,
            endpoint: raw.endpoint,
            endpoint_explicit: false,
            models_endpoint: raw.models_endpoint,
            models_endpoint_explicit: false,
            api_key: raw.api_key,
            api_key_env: raw.api_key_env,
            api_key_env_explicit,
            oauth_access_token: raw.oauth_access_token,
            oauth_access_token_env: raw.oauth_access_token_env,
            oauth_access_token_env_explicit,
            preferred_models: raw.preferred_models,
            reasoning_effort: raw.reasoning_effort,
            headers: raw.headers,
            temperature: raw.temperature,
            max_tokens: raw.max_tokens,
            request_timeout_ms: raw.request_timeout_ms,
            retry_max_attempts: raw.retry_max_attempts,
            retry_initial_backoff_ms: raw.retry_initial_backoff_ms,
            retry_max_backoff_ms: raw.retry_max_backoff_ms,
            model_catalog_cache_ttl_ms: raw.model_catalog_cache_ttl_ms,
            model_catalog_stale_if_error_ms: raw.model_catalog_stale_if_error_ms,
            model_catalog_cache_max_entries: raw.model_catalog_cache_max_entries,
            model_candidate_cooldown_ms: raw.model_candidate_cooldown_ms,
            model_candidate_cooldown_max_ms: raw.model_candidate_cooldown_max_ms,
            model_candidate_cooldown_max_entries: raw.model_candidate_cooldown_max_entries,
            profile_cooldown_ms: raw.profile_cooldown_ms,
            profile_cooldown_max_ms: raw.profile_cooldown_max_ms,
            profile_auth_reject_disable_ms: raw.profile_auth_reject_disable_ms,
            profile_state_max_entries: raw.profile_state_max_entries,
            profile_state_backend: raw.profile_state_backend,
            profile_health_mode: raw.profile_health_mode,
            tool_schema_mode: raw.tool_schema_mode,
            reasoning_extra_body_mode: raw.reasoning_extra_body_mode,
            tool_schema_disabled_model_hints: raw.tool_schema_disabled_model_hints,
            tool_schema_strict_model_hints: raw.tool_schema_strict_model_hints,
            reasoning_extra_body_kimi_model_hints: raw.reasoning_extra_body_kimi_model_hints,
            reasoning_extra_body_omit_model_hints: raw.reasoning_extra_body_omit_model_hints,
            profile_state_sqlite_path: raw.profile_state_sqlite_path,
        };
        config.refresh_endpoint_override_flags();
        Ok(config)
    }
}

impl ProviderConfig {
    pub fn set_kind(&mut self, kind: ProviderKind) {
        self.kind = kind;
        self.base_url_explicit = is_explicit_base_url(self.kind, self.base_url.as_str());
        self.chat_completions_path_explicit =
            is_explicit_chat_completions_path(self.kind, self.chat_completions_path.as_str());
        self.api_key_env_explicit = self
            .api_key_env
            .as_deref()
            .map(|value| is_explicit_api_key_env_name(self.kind, value))
            .unwrap_or(false);
        self.oauth_access_token_env_explicit = self
            .oauth_access_token_env
            .as_deref()
            .map(|value| is_explicit_oauth_access_token_env_name(self.kind, value))
            .unwrap_or(false);
        self.refresh_endpoint_override_flags();
    }

    pub fn set_base_url(&mut self, base_url: String) {
        self.base_url_explicit = is_explicit_base_url(self.kind, base_url.as_str());
        self.base_url = base_url;
        self.refresh_endpoint_override_flags();
    }

    pub fn set_chat_completions_path(&mut self, chat_completions_path: String) {
        self.chat_completions_path_explicit =
            is_explicit_chat_completions_path(self.kind, chat_completions_path.as_str());
        self.chat_completions_path = chat_completions_path;
        self.refresh_endpoint_override_flags();
    }

    pub fn set_endpoint(&mut self, endpoint: Option<String>) {
        self.endpoint = endpoint;
        self.refresh_endpoint_override_flags();
    }

    pub fn set_models_endpoint(&mut self, models_endpoint: Option<String>) {
        self.models_endpoint = models_endpoint;
        self.refresh_endpoint_override_flags();
    }

    pub fn set_api_key_env(&mut self, api_key_env: Option<String>) {
        self.api_key_env_explicit = api_key_env
            .as_deref()
            .map(|value| is_explicit_api_key_env_name(self.kind, value))
            .unwrap_or(false);
        self.api_key_env = api_key_env;
    }

    pub fn set_oauth_access_token_env(&mut self, oauth_access_token_env: Option<String>) {
        self.oauth_access_token_env_explicit = oauth_access_token_env
            .as_deref()
            .map(|value| is_explicit_oauth_access_token_env_name(self.kind, value))
            .unwrap_or(false);
        self.oauth_access_token_env = oauth_access_token_env;
    }

    pub fn fresh_for_kind(kind: ProviderKind) -> Self {
        let mut provider = Self::default();
        provider.set_kind(kind);
        provider.model = kind.default_model().unwrap_or("auto").to_owned();
        provider.selection_baseline()
    }

    pub(super) fn validate(&self) -> Vec<ConfigValidationIssue> {
        self.validate_with_field_prefix("provider")
    }

    pub(super) fn validate_with_field_prefix(
        &self,
        field_prefix: &str,
    ) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        let api_key_env_field_path = format!("{field_prefix}.api_key_env");
        let api_key_inline_field_path = format!("{field_prefix}.api_key");
        let api_key_example = self
            .kind
            .default_api_key_env()
            .unwrap_or("PROVIDER_API_KEY");
        if let Err(issue) = validate_env_pointer_field(
            api_key_env_field_path.as_str(),
            self.api_key_env.as_deref(),
            EnvPointerValidationHint {
                inline_field_path: api_key_inline_field_path.as_str(),
                example_env_name: api_key_example,
                detect_telegram_token_shape: false,
            },
        ) {
            issues.push(*issue);
        }
        let oauth_env_field_path = format!("{field_prefix}.oauth_access_token_env");
        let oauth_inline_field_path = format!("{field_prefix}.oauth_access_token");
        let oauth_example = self
            .kind
            .default_oauth_access_token_env()
            .unwrap_or("PROVIDER_OAUTH_ACCESS_TOKEN");
        if let Err(issue) = validate_env_pointer_field(
            oauth_env_field_path.as_str(),
            self.oauth_access_token_env.as_deref(),
            EnvPointerValidationHint {
                inline_field_path: oauth_inline_field_path.as_str(),
                example_env_name: oauth_example,
                detect_telegram_token_shape: false,
            },
        ) {
            issues.push(*issue);
        }
        issues
    }

    pub fn endpoint(&self) -> String {
        if self.endpoint_explicit
            && let Some(endpoint) = non_empty(self.endpoint.as_deref())
        {
            return endpoint.to_owned();
        }

        self.derived_endpoint()
    }

    pub fn models_endpoint(&self) -> String {
        if self.models_endpoint_explicit
            && let Some(endpoint) = non_empty(self.models_endpoint.as_deref())
        {
            return endpoint.to_owned();
        }

        self.derived_models_endpoint()
    }

    fn derived_endpoint(&self) -> String {
        let profile = self.kind.profile();
        let resolved_base_url = self.resolved_base_url();
        let resolved_chat_path = self.resolve_chat_path(
            profile.chat_completions_path,
            default_openai_chat_path().as_str(),
            default_provider_base_url().as_str(),
        );
        let resolved_chat_path =
            maybe_normalize_custom_chat_path(self.kind, &resolved_base_url, &resolved_chat_path);
        let resolved_request_path = match self.wire_api {
            ProviderWireApi::ChatCompletions => resolved_chat_path,
            ProviderWireApi::Responses => derive_responses_path(&resolved_chat_path),
        };
        join_base_with_path(
            &resolved_base_url,
            &resolved_request_path,
            default_request_path_for_wire_api(self.wire_api).as_str(),
        )
    }

    fn derived_models_endpoint(&self) -> String {
        let profile = self.kind.profile();
        if let Some(models_endpoint) = profile
            .models_path
            .and_then(|path| non_empty(Some(path)))
            .filter(|path| is_absolute_url(path))
        {
            return resolve_provider_template(self.kind, models_endpoint);
        }
        let resolved_base_url = self.resolved_base_url();
        let resolved_chat_path = self.resolve_chat_path(
            profile.chat_completions_path,
            default_openai_chat_path().as_str(),
            default_provider_base_url().as_str(),
        );
        let resolved_chat_path =
            maybe_normalize_custom_chat_path(self.kind, &resolved_base_url, &resolved_chat_path);
        let request_path = match self.wire_api {
            ProviderWireApi::ChatCompletions => resolved_chat_path,
            ProviderWireApi::Responses => derive_responses_path(&resolved_chat_path),
        };
        let models_path = profile
            .models_path
            .map(normalize_api_path)
            .unwrap_or_else(|| derive_models_path(&request_path));
        join_base_with_path(&resolved_base_url, &models_path, "/v1/models")
    }

    #[cfg(test)]
    pub fn default_api_key_env(&self) -> Option<String> {
        self.kind.default_api_key_env().map(str::to_owned)
    }

    #[cfg(test)]
    pub fn default_oauth_access_token_env(&self) -> Option<String> {
        self.kind
            .default_oauth_access_token_env()
            .map(str::to_owned)
    }

    pub fn authorization_header(&self) -> Option<String> {
        if self.kind.auth_scheme() != ProviderAuthScheme::Bearer {
            return None;
        }
        self.resolved_auth_secret()
            .map(|value| format!("Bearer {value}"))
    }

    pub fn resolved_auth_secret(&self) -> Option<String> {
        match self.kind.auth_scheme() {
            ProviderAuthScheme::Bearer => {
                if let Some(token) = self.oauth_access_token() {
                    return Some(token);
                }
                self.api_key()
            }
            ProviderAuthScheme::XApiKey => self.api_key(),
        }
    }

    pub fn resolved_auth_env_name(&self) -> Option<String> {
        match self.kind.auth_scheme() {
            ProviderAuthScheme::Bearer => {
                if self
                    .oauth_access_token
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
                {
                    return None;
                }
                if let Some(env_name) =
                    first_non_empty_env_name(&self.oauth_access_token_env_names())
                {
                    return Some(env_name);
                }
                if self
                    .api_key
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
                {
                    return None;
                }
                first_non_empty_env_name(&self.api_key_env_names())
            }
            ProviderAuthScheme::XApiKey => {
                if self
                    .api_key
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
                {
                    return None;
                }
                first_non_empty_env_name(&self.api_key_env_names())
            }
        }
    }

    pub fn transport_policy(&self) -> ProviderTransportPolicy {
        let request_endpoint = self.endpoint();
        let models_endpoint = self.models_endpoint();
        let fallback = self.build_responses_fallback();

        let readiness = match self.wire_api {
            ProviderWireApi::ChatCompletions => ProviderTransportReadiness {
                level: ProviderTransportReadinessLevel::Ready,
                summary: "chat_completions compatibility mode".to_owned(),
                detail: format!(
                    "`{}` uses the broadly compatible chat-completions transport at {}",
                    self.kind.profile().id,
                    request_endpoint
                ),
                auto_fallback_to_chat_completions: false,
            },
            ProviderWireApi::Responses => {
                if self.kind == ProviderKind::KimiCoding {
                    ProviderTransportReadiness {
                        level: ProviderTransportReadinessLevel::Unsupported,
                        summary: "responses unsupported for kimi_coding".to_owned(),
                        detail:
                            "kimi_coding currently supports only chat_completions; switch wire_api to `chat_completions`"
                                .to_owned(),
                        auto_fallback_to_chat_completions: false,
                    }
                } else if self.kind == ProviderKind::Openai
                    && !self.uses_explicit_endpoint_override()
                    && self.base_url_is_profile_default_like()
                    && self.chat_completions_path_is_profile_default_like()
                {
                    ProviderTransportReadiness {
                        level: ProviderTransportReadinessLevel::Ready,
                        summary: "responses native mode".to_owned(),
                        detail: format!(
                            "native OpenAI Responses endpoint {} is configured",
                            request_endpoint
                        ),
                        auto_fallback_to_chat_completions: false,
                    }
                } else if let Some(fallback) = fallback.as_ref() {
                    ProviderTransportReadiness {
                        level: ProviderTransportReadinessLevel::Review,
                        summary: "responses compatibility mode with chat fallback".to_owned(),
                        detail: format!(
                            "Responses endpoint {} is running in compatibility mode; LoongClaw will retry chat_completions automatically via {} if Responses is rejected",
                            request_endpoint, fallback.endpoint
                        ),
                        auto_fallback_to_chat_completions: true,
                    }
                } else {
                    ProviderTransportReadiness {
                        level: ProviderTransportReadinessLevel::Review,
                        summary: "responses custom endpoint needs review".to_owned(),
                        detail: format!(
                            "Responses uses an explicit endpoint override ({}); verify it accepts Responses or switch to chat_completions manually",
                            request_endpoint
                        ),
                        auto_fallback_to_chat_completions: false,
                    }
                }
            }
        };

        ProviderTransportPolicy {
            request_wire_api: self.wire_api,
            request_endpoint,
            models_endpoint,
            readiness,
            fallback,
        }
    }

    pub fn transport_readiness(&self) -> ProviderTransportReadiness {
        self.transport_policy().readiness
    }

    pub fn preview_transport_summary(&self) -> Option<String> {
        match self.wire_api {
            ProviderWireApi::Responses => Some(self.transport_readiness().summary),
            ProviderWireApi::ChatCompletions => None,
        }
    }

    pub fn responses_fallback_provider(&self) -> Option<Self> {
        self.transport_policy()
            .fallback
            .map(|fallback| fallback.provider)
    }

    fn build_responses_fallback(&self) -> Option<ProviderTransportFallback> {
        if self.wire_api != ProviderWireApi::Responses
            || self.kind == ProviderKind::KimiCoding
            || self.uses_explicit_endpoint_override()
        {
            return None;
        }

        let mut fallback = self.clone();
        fallback.wire_api = ProviderWireApi::ChatCompletions;
        fallback.endpoint = None;
        Some(ProviderTransportFallback {
            wire_api: ProviderWireApi::ChatCompletions,
            endpoint: fallback.endpoint(),
            provider: fallback,
        })
    }

    pub fn explicit_model(&self) -> Option<String> {
        let trimmed = self.model.trim();
        if !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("auto") {
            return Some(trimmed.to_owned());
        }
        None
    }

    pub fn configured_model_value(&self) -> String {
        let trimmed = self.model.trim();
        if trimmed.is_empty() {
            return "auto".to_owned();
        }
        trimmed.to_owned()
    }

    pub fn selection_strategy_id(&self) -> &'static str {
        if self.explicit_model().is_some() {
            "explicit_model"
        } else {
            "auto_discovery"
        }
    }

    pub fn configured_auto_model_candidates(&self) -> Vec<String> {
        if self.explicit_model().is_some() {
            return Vec::new();
        }

        let mut models = Vec::new();
        for raw in &self.preferred_models {
            let trimmed = raw.trim();
            if trimmed.is_empty() || models.iter().any(|existing| existing == trimmed) {
                continue;
            }
            models.push(trimmed.to_owned());
        }
        models
    }

    pub fn model_catalog_probe_recovery(&self) -> ModelCatalogProbeRecovery {
        if let Some(model) = self.explicit_model() {
            return ModelCatalogProbeRecovery::ExplicitModel(model);
        }

        let preferred_models = self.configured_auto_model_candidates();
        if !preferred_models.is_empty() {
            return ModelCatalogProbeRecovery::ConfiguredPreferredModels(preferred_models);
        }

        ModelCatalogProbeRecovery::RequiresExplicitModel {
            recommended_onboarding_model: self.kind.recommended_onboarding_model(),
        }
    }

    pub fn resolved_model(&self) -> Option<String> {
        self.explicit_model()
    }

    pub fn model_selection_requires_fetch(&self) -> bool {
        self.explicit_model().is_none()
    }

    pub fn resolved_model_catalog_cache_ttl_ms(&self) -> u64 {
        clamp_non_negative_u64(self.model_catalog_cache_ttl_ms, 300_000)
    }

    pub fn resolved_model_catalog_stale_if_error_ms(&self) -> u64 {
        clamp_non_negative_u64(self.model_catalog_stale_if_error_ms, 600_000)
    }

    pub fn resolved_model_catalog_cache_max_entries(&self) -> usize {
        clamp_usize_at_least_one(self.model_catalog_cache_max_entries, 256)
    }

    pub fn resolved_model_candidate_cooldown_ms(&self) -> u64 {
        clamp_non_negative_u64(self.model_candidate_cooldown_ms, 3_600_000)
    }

    pub fn resolved_model_candidate_cooldown_max_ms(&self) -> u64 {
        let base = self.resolved_model_candidate_cooldown_ms();
        clamp_u64_with_floor(self.model_candidate_cooldown_max_ms, 86_400_000, base)
    }

    pub fn resolved_model_candidate_cooldown_max_entries(&self) -> usize {
        clamp_usize_at_least_one(self.model_candidate_cooldown_max_entries, 512)
    }

    pub fn resolved_profile_cooldown_ms(&self) -> u64 {
        clamp_non_negative_u64(self.profile_cooldown_ms, 3_600_000)
    }

    pub fn resolved_profile_cooldown_max_ms(&self) -> u64 {
        let base = self.resolved_profile_cooldown_ms();
        clamp_u64_with_floor(self.profile_cooldown_max_ms, 86_400_000, base)
    }

    pub fn resolved_profile_auth_reject_disable_ms(&self) -> u64 {
        self.profile_auth_reject_disable_ms
            .clamp(60_000, 604_800_000)
    }

    pub fn resolved_profile_state_max_entries(&self) -> usize {
        clamp_usize_at_least_one(self.profile_state_max_entries, 1024)
    }

    pub fn resolved_profile_state_backend(&self) -> ProviderProfileStateBackendKind {
        self.profile_state_backend
    }

    pub fn resolved_profile_state_sqlite_path(&self) -> Option<PathBuf> {
        normalize_sqlite_path(self.profile_state_sqlite_path.as_deref())
    }

    pub fn resolved_profile_state_sqlite_path_with_default(&self) -> PathBuf {
        self.resolved_profile_state_sqlite_path()
            .unwrap_or_else(|| default_loongclaw_home().join("provider-profile-state.sqlite3"))
    }

    pub fn resolved_profile_health_mode_config(&self) -> ProviderProfileHealthModeConfig {
        self.profile_health_mode
    }

    pub fn resolved_tool_schema_mode_config(&self) -> ProviderToolSchemaModeConfig {
        self.tool_schema_mode
    }

    pub fn resolved_reasoning_extra_body_mode_config(
        &self,
    ) -> ProviderReasoningExtraBodyModeConfig {
        self.reasoning_extra_body_mode
    }

    pub fn resolved_tool_schema_disabled_model_hints(&self) -> Vec<String> {
        normalize_hint_values(&self.tool_schema_disabled_model_hints)
    }

    pub fn resolved_tool_schema_strict_model_hints(&self) -> Vec<String> {
        normalize_hint_values(&self.tool_schema_strict_model_hints)
    }

    pub fn resolved_reasoning_extra_body_kimi_model_hints(&self) -> Vec<String> {
        normalize_hint_values(&self.reasoning_extra_body_kimi_model_hints)
    }

    pub fn resolved_reasoning_extra_body_omit_model_hints(&self) -> Vec<String> {
        normalize_hint_values(&self.reasoning_extra_body_omit_model_hints)
    }

    pub fn selection_baseline(&self) -> Self {
        let profile = self.kind.profile();
        Self {
            kind: self.kind,
            model: self.model.clone(),
            preferred_models: self.preferred_models.clone(),
            base_url: profile.base_url.to_owned(),
            wire_api: self.wire_api,
            chat_completions_path: profile.chat_completions_path.to_owned(),
            api_key_env: self.kind.default_api_key_env().map(str::to_owned),
            oauth_access_token_env: self
                .kind
                .default_oauth_access_token_env()
                .map(str::to_owned),
            ..Self::default()
        }
    }

    pub fn has_only_selection_changes(&self) -> bool {
        self == &self.selection_baseline()
    }

    pub fn differs_from_default(&self) -> bool {
        self != &Self::default()
    }

    pub fn base_url_is_profile_default_like(&self) -> bool {
        let profile = self.kind.profile();
        self.base_url.trim().is_empty()
            || is_same_base_url(self.base_url.as_str(), profile.base_url)
    }

    pub fn chat_completions_path_is_profile_default_like(&self) -> bool {
        let profile = self.kind.profile();
        self.chat_completions_path.trim().is_empty()
            || is_same_chat_path(
                self.chat_completions_path.as_str(),
                profile.chat_completions_path,
            )
    }

    pub fn oauth_access_token(&self) -> Option<String> {
        if let Some(raw) = self.oauth_access_token.as_deref() {
            let value = raw.trim();
            if !value.is_empty() {
                return match resolve_inline_secret(value) {
                    InlineSecretResolution::Resolved(secret) => Some(secret),
                    InlineSecretResolution::ExplicitEnvMissing => None,
                    InlineSecretResolution::NotInlineEnvReference => Some(value.to_owned()),
                };
            }
        }

        first_non_empty_env_value(&self.oauth_access_token_env_names())
    }

    fn uses_explicit_endpoint_override(&self) -> bool {
        self.endpoint_explicit && non_empty(self.endpoint.as_deref()).is_some()
    }

    fn resolve_base_url(&self, profile_default: &str, openai_default: &str) -> String {
        let base = self.base_url.trim();
        if base.is_empty() {
            return profile_default.to_owned();
        }
        if !self.base_url_explicit && is_provider_managed_base_url(base) {
            return profile_default.to_owned();
        }
        if self.kind != ProviderKind::Openai
            && is_same_base_url(base, openai_default)
            && (self.chat_completions_path.trim().is_empty()
                || is_same_chat_path(
                    self.chat_completions_path.as_str(),
                    default_openai_chat_path().as_str(),
                ))
        {
            return profile_default.to_owned();
        }
        base.to_owned()
    }

    fn resolve_chat_path(
        &self,
        profile_default: &str,
        openai_default_path: &str,
        openai_default_base: &str,
    ) -> String {
        let path = self.chat_completions_path.trim();
        if path.is_empty() {
            return profile_default.to_owned();
        }
        if !self.chat_completions_path_explicit && is_provider_managed_chat_path(path) {
            return profile_default.to_owned();
        }
        if self.kind != ProviderKind::Openai
            && is_same_chat_path(path, openai_default_path)
            && (self.base_url.trim().is_empty()
                || is_same_base_url(self.base_url.as_str(), openai_default_base))
        {
            return profile_default.to_owned();
        }
        normalize_api_path(path)
    }

    pub fn api_key(&self) -> Option<String> {
        self.api_key_candidates().into_iter().next()
    }

    pub fn api_key_candidates(&self) -> Vec<String> {
        if let Some(raw) = self.api_key.as_deref() {
            let value = raw.trim();
            if !value.is_empty() {
                return match resolve_inline_secret(value) {
                    InlineSecretResolution::Resolved(secret) => split_secret_candidates(&secret),
                    InlineSecretResolution::ExplicitEnvMissing => Vec::new(),
                    InlineSecretResolution::NotInlineEnvReference => split_secret_candidates(value),
                };
            }
        }

        let mut env_keys = Vec::new();
        push_unique_env_key(&mut env_keys, self.configured_api_key_env_name());
        push_unique_env_key(&mut env_keys, self.kind.default_api_key_env());
        for alias in self.kind.api_key_env_aliases() {
            push_unique_env_key(&mut env_keys, Some(alias));
        }

        collect_non_empty_env_values(&env_keys)
    }

    pub fn credential_env_names(&self) -> Vec<String> {
        let mut env_names = self.oauth_access_token_env_names();
        for name in self.api_key_env_names() {
            if !env_names.iter().any(|existing| existing == &name) {
                env_names.push(name);
            }
        }
        env_names
    }

    pub fn resolved_base_url(&self) -> String {
        let profile = self.kind.profile();
        resolve_provider_template(
            self.kind,
            self.resolve_base_url(profile.base_url, default_provider_base_url().as_str())
                .as_str(),
        )
    }

    pub fn header_value(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn inferred_profile_id(&self) -> String {
        self.kind.profile().id.to_owned()
    }

    pub fn has_unresolved_custom_base_url(&self) -> bool {
        if !self.kind.requires_custom_base_url() {
            return false;
        }
        let resolved_base_url = self.resolved_base_url();
        resolved_base_url == self.kind.profile().base_url
            || contains_template_placeholder(resolved_base_url.as_str())
    }

    pub fn configuration_hint(&self) -> Option<String> {
        if self.kind == ProviderKind::Byteplus && self.uses_byteplus_coding_plan_path() {
            return Some(
                "byteplus uses the standard ModelArk path and should not target `/api/coding` or `/api/coding/v3`; switch to `kind = \"byteplus_coding\"` for the dedicated OpenAI-compatible Coding Plan endpoint"
                    .to_owned(),
            );
        }
        if self.kind == ProviderKind::Volcengine && self.uses_volcengine_coding_plan_path() {
            return Some(
                "volcengine uses the standard Ark API path under `/api/v3` and should not target `/api/coding` or `/api/coding/v3`; switch to `kind = \"volcengine_coding\"` for the dedicated OpenAI-compatible Coding Plan endpoint"
                    .to_owned(),
            );
        }
        if self.kind == ProviderKind::ByteplusCoding
            && (self.uses_generic_byteplus_modelark_v3_path()
                || self.uses_ark_coding_anthropic_path())
        {
            return Some(
                "byteplus_coding must use the dedicated BytePlus Coding path under `/api/coding/v3`; do not point it at the unsupported Anthropic-compatible `/api/coding` or generic `/api/v3` ModelArk endpoints because that bypasses Coding Plan quota and can incur standard model charges"
                    .to_owned(),
            );
        }
        if self.kind == ProviderKind::VolcengineCoding
            && (self.uses_generic_volcengine_modelark_v3_path()
                || self.uses_ark_coding_anthropic_path())
        {
            return Some(
                "volcengine_coding must use the dedicated Volcengine Coding Plan path under `/api/coding/v3`; do not point it at the Anthropic-compatible `/api/coding` or generic `/api/v3` Ark endpoints because that bypasses Coding Plan quota and can incur standard charges"
                    .to_owned(),
            );
        }
        if self.has_unresolved_custom_base_url() {
            let template = self.kind.profile().base_url;
            let base = self.kind.configuration_hint().unwrap_or(
                "replace the provider base URL template with a concrete account-scoped endpoint",
            );
            return Some(format!(
                "{} requires tenant-scoped base_url configuration: {base}; current template: `{template}`",
                self.kind.as_str()
            ));
        }
        None
    }

    pub fn region_endpoint_note(&self) -> Option<String> {
        Some(self.kind.region_endpoint_guide()?.note(self))
    }

    pub fn region_endpoint_failure_hint(&self) -> Option<String> {
        Some(self.kind.region_endpoint_guide()?.failure_hint(self))
    }

    pub fn request_region_endpoint_failure_hint(&self) -> Option<String> {
        Some(
            self.kind
                .region_endpoint_guide()?
                .request_failure_hint(self),
        )
    }

    fn uses_byteplus_coding_plan_path(&self) -> bool {
        if self.kind != ProviderKind::Byteplus {
            return false;
        }

        let resolved_base_url = self.resolved_base_url();
        let endpoint = self.endpoint();
        let models_endpoint = self.models_endpoint();
        [
            resolved_base_url.as_str(),
            endpoint.as_str(),
            models_endpoint.as_str(),
        ]
        .into_iter()
        .any(is_ark_coding_plan_path)
    }

    fn uses_generic_byteplus_modelark_v3_path(&self) -> bool {
        if self.kind != ProviderKind::ByteplusCoding {
            return false;
        }

        let resolved_base_url = self.resolved_base_url();
        let endpoint = self.endpoint();
        let models_endpoint = self.models_endpoint();
        [
            resolved_base_url.as_str(),
            endpoint.as_str(),
            models_endpoint.as_str(),
        ]
        .into_iter()
        .any(is_generic_ark_modelark_v3_path)
    }

    fn uses_volcengine_coding_plan_path(&self) -> bool {
        if self.kind != ProviderKind::Volcengine {
            return false;
        }

        let resolved_base_url = self.resolved_base_url();
        let endpoint = self.endpoint();
        let models_endpoint = self.models_endpoint();
        [
            resolved_base_url.as_str(),
            endpoint.as_str(),
            models_endpoint.as_str(),
        ]
        .into_iter()
        .any(is_ark_coding_plan_path)
    }

    fn uses_generic_volcengine_modelark_v3_path(&self) -> bool {
        if self.kind != ProviderKind::VolcengineCoding {
            return false;
        }

        let resolved_base_url = self.resolved_base_url();
        let endpoint = self.endpoint();
        let models_endpoint = self.models_endpoint();
        [
            resolved_base_url.as_str(),
            endpoint.as_str(),
            models_endpoint.as_str(),
        ]
        .into_iter()
        .any(is_generic_ark_modelark_v3_path)
    }

    fn uses_ark_coding_anthropic_path(&self) -> bool {
        if !matches!(
            self.kind,
            ProviderKind::ByteplusCoding | ProviderKind::VolcengineCoding
        ) {
            return false;
        }

        let resolved_base_url = self.resolved_base_url();
        let endpoint = self.endpoint();
        let models_endpoint = self.models_endpoint();
        [
            resolved_base_url.as_str(),
            endpoint.as_str(),
            models_endpoint.as_str(),
        ]
        .into_iter()
        .any(is_ark_coding_anthropic_path)
    }

    pub fn model_selection_fallback_hint(&self) -> Option<String> {
        if let Some(model) = self.explicit_model() {
            return Some(format!("explicit model `{model}`"));
        }

        let configured = self.configured_auto_model_candidates();
        if !configured.is_empty() {
            return Some(format!("preferred_models ({})", configured.join(", ")));
        }
        None
    }

    fn oauth_access_token_env_names(&self) -> Vec<String> {
        let mut env_keys = Vec::new();
        let configured_oauth_env = self.configured_oauth_access_token_env_name();
        push_unique_env_key(&mut env_keys, configured_oauth_env);
        if configured_oauth_env.is_none()
            && self.configured_api_key_env_name().is_none()
            && self
                .api_key
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
        {
            push_unique_env_key(&mut env_keys, self.kind.default_oauth_access_token_env());
            for alias in self.kind.oauth_access_token_env_aliases() {
                push_unique_env_key(&mut env_keys, Some(alias));
            }
        }
        env_keys
    }

    fn api_key_env_names(&self) -> Vec<String> {
        let mut env_keys = Vec::new();
        push_unique_env_key(&mut env_keys, self.configured_api_key_env_name());
        push_unique_env_key(&mut env_keys, self.kind.default_api_key_env());
        for alias in self.kind.api_key_env_aliases() {
            push_unique_env_key(&mut env_keys, Some(alias));
        }
        env_keys
    }

    fn configured_api_key_env_name(&self) -> Option<&str> {
        let env_name = non_empty(self.api_key_env.as_deref())?;
        if !self.api_key_env_explicit && is_provider_managed_api_key_env_name(env_name) {
            return None;
        }
        Some(env_name)
    }

    fn configured_oauth_access_token_env_name(&self) -> Option<&str> {
        let env_name = non_empty(self.oauth_access_token_env.as_deref())?;
        if !self.oauth_access_token_env_explicit
            && is_provider_managed_oauth_access_token_env_name(env_name)
        {
            return None;
        }
        Some(env_name)
    }

    pub fn configured_api_key_env_override(&self) -> Option<String> {
        self.configured_api_key_env_name().map(str::to_owned)
    }

    pub fn configured_oauth_access_token_env_override(&self) -> Option<String> {
        self.configured_oauth_access_token_env_name()
            .map(str::to_owned)
    }

    pub fn normalized_for_persistence(&self) -> Self {
        let profile = self.kind.profile();
        let base_url =
            self.resolve_base_url(profile.base_url, default_provider_base_url().as_str());
        let chat_completions_path = maybe_normalize_custom_chat_path(
            self.kind,
            &base_url,
            &self.resolve_chat_path(
                profile.chat_completions_path,
                default_openai_chat_path().as_str(),
                default_provider_base_url().as_str(),
            ),
        );

        let mut normalized = self.clone();
        normalized.base_url = base_url;
        normalized.chat_completions_path = chat_completions_path;
        normalized.endpoint = self.normalized_endpoint_for_persistence();
        normalized.models_endpoint = self.normalized_models_endpoint_for_persistence();
        normalized.api_key_env = self.normalized_api_key_env_for_persistence();
        normalized.oauth_access_token_env =
            self.normalized_oauth_access_token_env_for_persistence();
        normalized
    }

    fn normalized_endpoint_for_persistence(&self) -> Option<String> {
        if self.endpoint_explicit {
            return non_empty(self.endpoint.as_deref()).map(str::to_owned);
        }
        None
    }

    fn normalized_models_endpoint_for_persistence(&self) -> Option<String> {
        if self.models_endpoint_explicit {
            return non_empty(self.models_endpoint.as_deref()).map(str::to_owned);
        }
        None
    }

    fn normalized_api_key_env_for_persistence(&self) -> Option<String> {
        let configured = non_empty(self.api_key_env.as_deref()).map(str::to_owned);
        if self.api_key_env_explicit {
            return configured;
        }
        if let Some(configured_override) = self.configured_api_key_env_name().map(str::to_owned) {
            return Some(configured_override);
        }
        self.kind.default_api_key_env().map(str::to_owned)
    }

    fn normalized_oauth_access_token_env_for_persistence(&self) -> Option<String> {
        let configured = non_empty(self.oauth_access_token_env.as_deref()).map(str::to_owned);
        if self.oauth_access_token_env_explicit {
            return configured;
        }
        if let Some(configured_override) = self
            .configured_oauth_access_token_env_name()
            .map(str::to_owned)
        {
            return Some(configured_override);
        }
        self.kind
            .default_oauth_access_token_env()
            .map(str::to_owned)
    }

    fn refresh_endpoint_override_flags(&mut self) {
        self.endpoint_explicit = self
            .endpoint
            .as_deref()
            .map(|value| is_explicit_endpoint(self, value))
            .unwrap_or(false);
        self.models_endpoint_explicit = self
            .models_endpoint
            .as_deref()
            .map(|value| is_explicit_models_endpoint(self, value))
            .unwrap_or(false);
    }
}

fn contains_template_placeholder(value: &str) -> bool {
    value.contains('<') && value.contains('>')
}

fn is_explicit_api_key_env_name(kind: ProviderKind, env_name: &str) -> bool {
    let Some(env_name) = non_empty(Some(env_name)) else {
        return false;
    };
    !is_current_provider_api_key_env_name(kind, env_name)
}

fn is_explicit_base_url(kind: ProviderKind, base_url: &str) -> bool {
    let Some(base_url) = non_empty(Some(base_url)) else {
        return false;
    };
    !is_current_provider_base_url(kind, base_url)
}

fn is_explicit_chat_completions_path(kind: ProviderKind, path: &str) -> bool {
    let Some(path) = non_empty(Some(path)) else {
        return false;
    };
    !is_current_provider_chat_completions_path(kind, path)
}

fn is_explicit_endpoint(provider: &ProviderConfig, endpoint: &str) -> bool {
    let Some(endpoint) = non_empty(Some(endpoint)) else {
        return false;
    };
    !is_same_base_url(endpoint, provider.derived_endpoint().as_str())
}

fn is_explicit_models_endpoint(provider: &ProviderConfig, endpoint: &str) -> bool {
    let Some(endpoint) = non_empty(Some(endpoint)) else {
        return false;
    };
    !is_same_base_url(endpoint, provider.derived_models_endpoint().as_str())
}

fn is_explicit_oauth_access_token_env_name(kind: ProviderKind, env_name: &str) -> bool {
    let Some(env_name) = non_empty(Some(env_name)) else {
        return false;
    };
    !is_current_provider_oauth_access_token_env_name(kind, env_name)
}

fn is_current_provider_base_url(kind: ProviderKind, base_url: &str) -> bool {
    is_same_base_url(base_url, kind.profile().base_url)
}

fn is_current_provider_chat_completions_path(kind: ProviderKind, path: &str) -> bool {
    is_same_chat_path(path, kind.profile().chat_completions_path)
}

fn is_current_provider_api_key_env_name(kind: ProviderKind, env_name: &str) -> bool {
    kind.default_api_key_env() == Some(env_name) || kind.api_key_env_aliases().contains(&env_name)
}

fn is_current_provider_oauth_access_token_env_name(kind: ProviderKind, env_name: &str) -> bool {
    kind.default_oauth_access_token_env() == Some(env_name)
        || kind.oauth_access_token_env_aliases().contains(&env_name)
}

fn is_provider_managed_api_key_env_name(env_name: &str) -> bool {
    PROVIDER_PROFILES.iter().any(|profile| {
        profile.default_api_key_env == Some(env_name)
            || profile.api_key_env_aliases.contains(&env_name)
    })
}

fn is_provider_managed_base_url(base_url: &str) -> bool {
    PROVIDER_PROFILES
        .iter()
        .any(|profile| is_same_base_url(base_url, profile.base_url))
}

fn is_provider_managed_chat_path(path: &str) -> bool {
    PROVIDER_PROFILES
        .iter()
        .any(|profile| is_same_chat_path(path, profile.chat_completions_path))
}

fn is_provider_managed_oauth_access_token_env_name(env_name: &str) -> bool {
    PROVIDER_PROFILES.iter().any(|profile| {
        profile.default_oauth_access_token_env == Some(env_name)
            || profile.oauth_access_token_env_aliases.contains(&env_name)
    })
}

fn maybe_normalize_custom_chat_path(kind: ProviderKind, base_url: &str, path: &str) -> String {
    let normalized = normalize_api_path(path);
    if kind != ProviderKind::Custom {
        return normalized;
    }
    let trimmed_base = base_url.trim_end_matches('/');
    if trimmed_base.to_ascii_lowercase().ends_with("/v1") && normalized.starts_with("/v1/") {
        return normalized
            .strip_prefix("/v1")
            .unwrap_or(normalized.as_str())
            .to_owned();
    }
    normalized
}

fn is_ark_coding_plan_path(value: &str) -> bool {
    value.trim().to_ascii_lowercase().contains("/api/coding")
}

fn is_ark_coding_anthropic_path(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    normalized.contains("/api/coding") && !normalized.contains("/api/coding/v3")
}

fn is_generic_ark_modelark_v3_path(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    normalized.contains("/api/v3") && !normalized.contains("/api/coding/v3")
}

impl ProviderKind {
    pub fn all_sorted() -> &'static [ProviderKind] {
        &PROVIDER_KIND_ORDER
    }

    pub fn as_str(self) -> &'static str {
        self.profile().id
    }

    pub fn display_name(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "Anthropic",
            ProviderKind::Bedrock => "Bedrock",
            ProviderKind::Byteplus => "BytePlus",
            ProviderKind::ByteplusCoding => "BytePlus Coding",
            ProviderKind::Cerebras => "Cerebras",
            ProviderKind::CloudflareAiGateway => "Cloudflare AI Gateway",
            ProviderKind::Cohere => "Cohere",
            ProviderKind::Custom => "Custom",
            ProviderKind::Deepseek => "DeepSeek",
            ProviderKind::Fireworks => "Fireworks",
            ProviderKind::Gemini => "Gemini",
            ProviderKind::Groq => "Groq",
            ProviderKind::Kimi => "Kimi",
            ProviderKind::KimiCoding => "Kimi Coding",
            ProviderKind::Mistral => "Mistral",
            ProviderKind::Minimax => "MiniMax",
            ProviderKind::Novita => "Novita",
            ProviderKind::Nvidia => "NVIDIA",
            ProviderKind::Llamacpp => "llama.cpp",
            ProviderKind::LmStudio => "LM Studio",
            ProviderKind::Ollama => "Ollama",
            ProviderKind::Openai => "OpenAI",
            ProviderKind::Openrouter => "OpenRouter",
            ProviderKind::Perplexity => "Perplexity",
            ProviderKind::Qianfan => "Qianfan",
            ProviderKind::Qwen => "Qwen",
            ProviderKind::Sambanova => "SambaNova",
            ProviderKind::Sglang => "SGLang",
            ProviderKind::Siliconflow => "SiliconFlow",
            ProviderKind::Stepfun => "StepFun",
            ProviderKind::Together => "Together",
            ProviderKind::Venice => "Venice",
            ProviderKind::VercelAiGateway => "Vercel AI Gateway",
            ProviderKind::Vllm => "vLLM",
            ProviderKind::Volcengine => "Volcengine",
            ProviderKind::VolcengineCoding => "Volcengine Coding",
            ProviderKind::Xai => "xAI",
            ProviderKind::Zai => "Z.ai",
            ProviderKind::Zhipu => "Zhipu",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        parse_provider_kind_id(raw)
    }

    pub fn profile(self) -> &'static ProviderProfile {
        let [
            anthropic,
            bedrock,
            byteplus,
            byteplus_coding,
            cerebras,
            cloudflare_ai_gateway,
            cohere,
            custom,
            deepseek,
            fireworks,
            gemini,
            groq,
            kimi,
            kimi_coding,
            mistral,
            minimax,
            novita,
            nvidia,
            llamacpp,
            lm_studio,
            ollama,
            openai,
            openrouter,
            perplexity,
            qianfan,
            qwen,
            sambanova,
            sglang,
            siliconflow,
            stepfun,
            together,
            venice,
            vercel_ai_gateway,
            vllm,
            volcengine,
            volcengine_coding,
            xai,
            zai,
            zhipu,
        ] = &PROVIDER_PROFILES;

        match self {
            ProviderKind::Anthropic => anthropic,
            ProviderKind::Bedrock => bedrock,
            ProviderKind::Byteplus => byteplus,
            ProviderKind::ByteplusCoding => byteplus_coding,
            ProviderKind::Cerebras => cerebras,
            ProviderKind::CloudflareAiGateway => cloudflare_ai_gateway,
            ProviderKind::Cohere => cohere,
            ProviderKind::Custom => custom,
            ProviderKind::Gemini => gemini,
            ProviderKind::Kimi => kimi,
            ProviderKind::KimiCoding => kimi_coding,
            ProviderKind::Groq => groq,
            ProviderKind::Fireworks => fireworks,
            ProviderKind::Mistral => mistral,
            ProviderKind::Minimax => minimax,
            ProviderKind::Novita => novita,
            ProviderKind::Nvidia => nvidia,
            ProviderKind::Llamacpp => llamacpp,
            ProviderKind::LmStudio => lm_studio,
            ProviderKind::Ollama => ollama,
            ProviderKind::Openai => openai,
            ProviderKind::Openrouter => openrouter,
            ProviderKind::Perplexity => perplexity,
            ProviderKind::Qianfan => qianfan,
            ProviderKind::Qwen => qwen,
            ProviderKind::Sambanova => sambanova,
            ProviderKind::Sglang => sglang,
            ProviderKind::Siliconflow => siliconflow,
            ProviderKind::Stepfun => stepfun,
            ProviderKind::Together => together,
            ProviderKind::Venice => venice,
            ProviderKind::VercelAiGateway => vercel_ai_gateway,
            ProviderKind::Volcengine => volcengine,
            ProviderKind::VolcengineCoding => volcengine_coding,
            ProviderKind::Xai => xai,
            ProviderKind::Zai => zai,
            ProviderKind::Zhipu => zhipu,
            ProviderKind::Deepseek => deepseek,
            ProviderKind::Vllm => vllm,
        }
    }

    pub fn auth_scheme(self) -> ProviderAuthScheme {
        self.profile().auth_scheme
    }

    pub fn protocol_family(self) -> ProviderProtocolFamily {
        self.profile().protocol_family
    }

    pub fn feature_family(self) -> ProviderFeatureFamily {
        self.profile().feature_family
    }

    pub fn default_headers(self) -> &'static [(&'static str, &'static str)] {
        self.profile().default_headers
    }

    pub fn default_api_key_env(self) -> Option<&'static str> {
        self.profile().default_api_key_env
    }

    pub fn api_key_env_aliases(self) -> &'static [&'static str] {
        self.profile().api_key_env_aliases
    }

    pub fn default_user_agent(self) -> Option<&'static str> {
        self.profile().default_user_agent
    }

    pub fn default_oauth_access_token_env(self) -> Option<&'static str> {
        self.profile().default_oauth_access_token_env
    }

    pub fn oauth_access_token_env_aliases(self) -> &'static [&'static str] {
        self.profile().oauth_access_token_env_aliases
    }

    pub fn auth_optional(self) -> bool {
        matches!(
            self,
            ProviderKind::Llamacpp
                | ProviderKind::LmStudio
                | ProviderKind::Ollama
                | ProviderKind::Sglang
                | ProviderKind::Vllm
        )
    }

    pub fn model_probe_auth_optional(self) -> bool {
        self.auth_optional()
            || matches!(self, ProviderKind::Cerebras | ProviderKind::VercelAiGateway)
    }

    pub fn allowed_reasoning_efforts(self) -> Option<&'static [ReasoningEffort]> {
        if self == ProviderKind::Cohere {
            Some(COHERE_REASONING_EFFORTS)
        } else if self.feature_family() == ProviderFeatureFamily::Volcengine {
            Some(ARK_REASONING_EFFORTS)
        } else {
            None
        }
    }

    pub fn supports_reasoning_effort(self, effort: ReasoningEffort) -> bool {
        self.allowed_reasoning_efforts()
            .is_none_or(|allowed| allowed.contains(&effort))
    }

    pub fn prefers_max_completion_tokens(self) -> bool {
        matches!(self, ProviderKind::Openai | ProviderKind::Cerebras)
    }

    pub fn preferred_token_limit_field_id(self) -> &'static str {
        if self.prefers_max_completion_tokens() {
            "max_completion_tokens"
        } else {
            "max_tokens"
        }
    }

    pub fn requires_custom_base_url(self) -> bool {
        matches!(
            self,
            ProviderKind::CloudflareAiGateway | ProviderKind::Custom
        )
    }

    pub fn configuration_hint(self) -> Option<&'static str> {
        if self == ProviderKind::Bedrock {
            Some(
                "set `AWS_REGION`/`AWS_DEFAULT_REGION` or replace `<region>` in `provider.base_url` with your Bedrock runtime region",
            )
        } else if self == ProviderKind::CloudflareAiGateway {
            Some(
                "replace `<account_id>` and `<gateway_name>` in `provider.base_url` with your real Cloudflare AI Gateway path",
            )
        } else if self == ProviderKind::Custom {
            Some(
                "replace `<openai-compatible-host>` in `provider.base_url` with your real OpenAI-compatible endpoint root such as `https://api.example.com/v1`",
            )
        } else {
            None
        }
    }

    fn region_endpoint_guide(self) -> Option<ProviderRegionEndpointGuide> {
        let profile = self.profile();
        match self {
            ProviderKind::Kimi => Some(ProviderRegionEndpointGuide {
                family_label: "Moonshot Kimi",
                default_variant: ProviderRegionEndpointVariant {
                    label: "CN",
                    base_url: profile.base_url,
                },
                alternate_variant: ProviderRegionEndpointVariant {
                    label: "Global",
                    base_url: "https://api.moonshot.ai",
                },
            }),
            ProviderKind::Minimax => Some(ProviderRegionEndpointGuide {
                family_label: "MiniMax",
                default_variant: ProviderRegionEndpointVariant {
                    label: "CN",
                    base_url: profile.base_url,
                },
                alternate_variant: ProviderRegionEndpointVariant {
                    label: "Global",
                    base_url: "https://api.minimax.io",
                },
            }),
            ProviderKind::Zai => Some(ProviderRegionEndpointGuide {
                family_label: "Z.ai / BigModel",
                default_variant: ProviderRegionEndpointVariant {
                    label: "Global",
                    base_url: profile.base_url,
                },
                alternate_variant: ProviderRegionEndpointVariant {
                    label: "CN",
                    base_url: "https://open.bigmodel.cn",
                },
            }),
            ProviderKind::Zhipu => Some(ProviderRegionEndpointGuide {
                family_label: "Z.ai / BigModel",
                default_variant: ProviderRegionEndpointVariant {
                    label: "CN",
                    base_url: profile.base_url,
                },
                alternate_variant: ProviderRegionEndpointVariant {
                    label: "Global",
                    base_url: "https://api.z.ai",
                },
            }),
            ProviderKind::Anthropic
            | ProviderKind::Bedrock
            | ProviderKind::Byteplus
            | ProviderKind::ByteplusCoding
            | ProviderKind::Cerebras
            | ProviderKind::CloudflareAiGateway
            | ProviderKind::Cohere
            | ProviderKind::Custom
            | ProviderKind::Deepseek
            | ProviderKind::Fireworks
            | ProviderKind::Gemini
            | ProviderKind::Groq
            | ProviderKind::KimiCoding
            | ProviderKind::Llamacpp
            | ProviderKind::LmStudio
            | ProviderKind::Mistral
            | ProviderKind::Novita
            | ProviderKind::Nvidia
            | ProviderKind::Ollama
            | ProviderKind::Openai
            | ProviderKind::Openrouter
            | ProviderKind::Perplexity
            | ProviderKind::Qianfan
            | ProviderKind::Qwen
            | ProviderKind::Sambanova
            | ProviderKind::Sglang
            | ProviderKind::Siliconflow
            | ProviderKind::Stepfun
            | ProviderKind::Together
            | ProviderKind::Venice
            | ProviderKind::VercelAiGateway
            | ProviderKind::Vllm
            | ProviderKind::Volcengine
            | ProviderKind::VolcengineCoding
            | ProviderKind::Xai => None,
        }
    }

    pub const fn default_model(self) -> Option<&'static str> {
        if matches!(self, ProviderKind::KimiCoding) {
            Some("kimi-for-coding")
        } else {
            None
        }
    }

    pub const fn recommended_onboarding_model(self) -> Option<&'static str> {
        if matches!(self, ProviderKind::Deepseek) {
            Some("deepseek-chat")
        } else if matches!(self, ProviderKind::Minimax) {
            Some("MiniMax-M2.5")
        } else {
            None
        }
    }
}

pub fn parse_provider_kind_id(raw: &str) -> Option<ProviderKind> {
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }

    for profile in &PROVIDER_PROFILES {
        if normalized == profile.id {
            return Some(profile.kind);
        }
        if profile.aliases.iter().any(|alias| normalized == *alias) {
            return Some(profile.kind);
        }
    }

    None
}

const PROVIDER_KIND_ORDER: [ProviderKind; 39] = [
    ProviderKind::Anthropic,
    ProviderKind::Bedrock,
    ProviderKind::Byteplus,
    ProviderKind::ByteplusCoding,
    ProviderKind::Cerebras,
    ProviderKind::CloudflareAiGateway,
    ProviderKind::Cohere,
    ProviderKind::Custom,
    ProviderKind::Deepseek,
    ProviderKind::Fireworks,
    ProviderKind::Gemini,
    ProviderKind::Groq,
    ProviderKind::Kimi,
    ProviderKind::KimiCoding,
    ProviderKind::Llamacpp,
    ProviderKind::LmStudio,
    ProviderKind::Mistral,
    ProviderKind::Minimax,
    ProviderKind::Novita,
    ProviderKind::Nvidia,
    ProviderKind::Ollama,
    ProviderKind::Openai,
    ProviderKind::Openrouter,
    ProviderKind::Perplexity,
    ProviderKind::Qianfan,
    ProviderKind::Qwen,
    ProviderKind::Sambanova,
    ProviderKind::Sglang,
    ProviderKind::Siliconflow,
    ProviderKind::Stepfun,
    ProviderKind::Together,
    ProviderKind::Venice,
    ProviderKind::VercelAiGateway,
    ProviderKind::Vllm,
    ProviderKind::Volcengine,
    ProviderKind::VolcengineCoding,
    ProviderKind::Xai,
    ProviderKind::Zai,
    ProviderKind::Zhipu,
];

const PROVIDER_PROFILES: [ProviderProfile; 39] = [
    ProviderProfile {
        kind: ProviderKind::Anthropic,
        id: "anthropic",
        aliases: &["anthropic_compatible"],
        base_url: "https://api.anthropic.com",
        chat_completions_path: "/v1/messages",
        models_path: Some("/v1/models"),
        protocol_family: ProviderProtocolFamily::AnthropicMessages,
        auth_scheme: ProviderAuthScheme::XApiKey,
        default_headers: &[("anthropic-version", "2023-06-01")],
        default_api_key_env: Some("ANTHROPIC_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::Anthropic,
    },
    ProviderProfile {
        kind: ProviderKind::Bedrock,
        id: "bedrock",
        aliases: &["aws-bedrock", "aws_bedrock"],
        base_url: "https://bedrock-runtime.<region>.amazonaws.com",
        chat_completions_path: "/model/{modelId}/converse",
        models_path: Some("https://bedrock.<region>.amazonaws.com/foundation-models"),
        protocol_family: ProviderProtocolFamily::BedrockConverse,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("AWS_BEARER_TOKEN_BEDROCK"),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::Bedrock,
    },
    ProviderProfile {
        kind: ProviderKind::Byteplus,
        id: "byteplus",
        aliases: &["byteplus_compatible"],
        base_url: "https://ark.ap-southeast.bytepluses.com/api/v3",
        chat_completions_path: "/chat/completions",
        models_path: Some("/models"),
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("BYTEPLUS_API_KEY"),
        api_key_env_aliases: &["ARK_API_KEY"],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::Volcengine,
    },
    ProviderProfile {
        kind: ProviderKind::ByteplusCoding,
        id: "byteplus_coding",
        aliases: &["byteplus_coding_compatible"],
        base_url: "https://ark.ap-southeast.bytepluses.com/api/coding/v3",
        chat_completions_path: "/chat/completions",
        models_path: Some("/models"),
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("BYTEPLUS_API_KEY"),
        api_key_env_aliases: &["ARK_API_KEY"],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::Volcengine,
    },
    ProviderProfile {
        kind: ProviderKind::Cerebras,
        id: "cerebras",
        aliases: &["cerebras_compatible"],
        base_url: "https://api.cerebras.ai",
        chat_completions_path: "/v1/chat/completions",
        models_path: Some("/public/v1/models"),
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("CEREBRAS_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::CloudflareAiGateway,
        id: "cloudflare_ai_gateway",
        aliases: &[
            "cloudflare-ai-gateway",
            "cloudflare_ai",
            "cloudflare-ai",
            "cloudflare",
        ],
        base_url: "https://gateway.ai.cloudflare.com/v1/<account_id>/<gateway_name>/openai/compat",
        chat_completions_path: "/chat/completions",
        models_path: Some("/models"),
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("CLOUDFLARE_API_KEY"),
        api_key_env_aliases: &["CLOUDFLARE_AI_GATEWAY_API_KEY"],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Cohere,
        id: "cohere",
        aliases: &["cohere_compatible"],
        base_url: "https://api.cohere.ai/compatibility",
        chat_completions_path: "/v1/chat/completions",
        models_path: Some("https://api.cohere.com/v1/models"),
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("COHERE_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Custom,
        id: "custom",
        aliases: &["openai_custom", "custom_openai"],
        base_url: "https://<openai-compatible-host>/v1",
        chat_completions_path: "/chat/completions",
        models_path: Some("/models"),
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("CUSTOM_PROVIDER_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Deepseek,
        id: "deepseek",
        aliases: &["deepseek_compatible"],
        base_url: "https://api.deepseek.com",
        chat_completions_path: "/v1/chat/completions",
        models_path: None,
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("DEEPSEEK_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Fireworks,
        id: "fireworks",
        aliases: &["fireworks_compatible", "fireworks-ai"],
        base_url: "https://api.fireworks.ai",
        chat_completions_path: "/inference/v1/chat/completions",
        models_path: None,
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("FIREWORKS_API_KEY"),
        api_key_env_aliases: &["FIREWORKS_AI_API_KEY"],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Gemini,
        id: "gemini",
        aliases: &[
            "gemini_compatible",
            "google",
            "google_gemini",
            "google-gemini",
        ],
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        chat_completions_path: "/chat/completions",
        models_path: Some("/models"),
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("GEMINI_API_KEY"),
        api_key_env_aliases: &["GOOGLE_API_KEY"],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Groq,
        id: "groq",
        aliases: &["groq_compatible"],
        base_url: "https://api.groq.com",
        chat_completions_path: "/openai/v1/chat/completions",
        models_path: None,
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("GROQ_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Kimi,
        id: "kimi",
        aliases: &["kimi_compatible", "moonshot", "moonshot_compatible"],
        base_url: "https://api.moonshot.cn",
        chat_completions_path: "/v1/chat/completions",
        models_path: None,
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("MOONSHOT_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::KimiCoding,
        id: "kimi_coding",
        aliases: &["kimi_coding_compatible"],
        base_url: "https://api.kimi.com",
        chat_completions_path: "/coding/v1/chat/completions",
        models_path: Some("/coding/v1/models"),
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("KIMI_CODING_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: Some("KimiCLI/LoongClaw"),
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Mistral,
        id: "mistral",
        aliases: &["mistral_compatible"],
        base_url: "https://api.mistral.ai",
        chat_completions_path: "/v1/chat/completions",
        models_path: None,
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("MISTRAL_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Minimax,
        id: "minimax",
        aliases: &["minimax_compatible"],
        base_url: "https://api.minimaxi.com",
        chat_completions_path: "/v1/chat/completions",
        models_path: None,
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("MINIMAX_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Novita,
        id: "novita",
        aliases: &["novita_compatible"],
        base_url: "https://api.novita.ai",
        chat_completions_path: "/v3/openai/chat/completions",
        models_path: Some("/v3/openai/models"),
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("NOVITA_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Nvidia,
        id: "nvidia",
        aliases: &[
            "nvidia_compatible",
            "nvidia_nim",
            "nvidia-nim",
            "build.nvidia.com",
        ],
        base_url: "https://integrate.api.nvidia.com",
        chat_completions_path: "/v1/chat/completions",
        models_path: None,
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("NVIDIA_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Llamacpp,
        id: "llamacpp",
        aliases: &["llama.cpp", "llama_cpp"],
        base_url: "http://127.0.0.1:8080",
        chat_completions_path: "/v1/chat/completions",
        models_path: None,
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: None,
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::LmStudio,
        id: "lm_studio",
        aliases: &["lmstudio", "lm-studio"],
        base_url: "http://127.0.0.1:1234",
        chat_completions_path: "/v1/chat/completions",
        models_path: None,
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: None,
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Ollama,
        id: "ollama",
        aliases: &["ollama_compatible"],
        base_url: "http://127.0.0.1:11434",
        chat_completions_path: "/v1/chat/completions",
        models_path: None,
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: None,
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Openai,
        id: "openai",
        aliases: &["openai_compatible"],
        base_url: "https://api.openai.com",
        chat_completions_path: "/v1/chat/completions",
        models_path: None,
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("OPENAI_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: Some("OPENAI_CODEX_OAUTH_TOKEN"),
        oauth_access_token_env_aliases: &["OPENAI_OAUTH_ACCESS_TOKEN"],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Openrouter,
        id: "openrouter",
        aliases: &["openrouter_compatible"],
        base_url: "https://openrouter.ai",
        chat_completions_path: "/api/v1/chat/completions",
        models_path: None,
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("OPENROUTER_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Perplexity,
        id: "perplexity",
        aliases: &["perplexity_compatible"],
        base_url: "https://api.perplexity.ai",
        chat_completions_path: "/chat/completions",
        models_path: Some("/models"),
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("PERPLEXITY_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Qianfan,
        id: "qianfan",
        aliases: &["qianfan_compatible", "baidu"],
        base_url: "https://qianfan.baidubce.com",
        chat_completions_path: "/v2/chat/completions",
        models_path: Some("/v2/models"),
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("QIANFAN_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Qwen,
        id: "qwen",
        aliases: &["qwen_compatible", "dashscope"],
        base_url: "https://dashscope.aliyuncs.com",
        chat_completions_path: "/compatible-mode/v1/chat/completions",
        models_path: Some("/compatible-mode/v1/models"),
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("DASHSCOPE_API_KEY"),
        api_key_env_aliases: &["QWEN_API_KEY"],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Sambanova,
        id: "sambanova",
        aliases: &["sambanova_compatible", "samba_nova"],
        base_url: "https://api.sambanova.ai",
        chat_completions_path: "/v1/chat/completions",
        models_path: None,
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("SAMBANOVA_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Sglang,
        id: "sglang",
        aliases: &["sglang_compatible"],
        base_url: "http://127.0.0.1:30000",
        chat_completions_path: "/v1/chat/completions",
        models_path: Some("/v1/models"),
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: None,
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Siliconflow,
        id: "siliconflow",
        aliases: &["siliconflow_compatible"],
        base_url: "https://api.siliconflow.com",
        chat_completions_path: "/v1/chat/completions",
        models_path: Some("/v1/models"),
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("SILICONFLOW_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Stepfun,
        id: "stepfun",
        aliases: &["stepfun_compatible"],
        base_url: "https://api.stepfun.com",
        chat_completions_path: "/v1/chat/completions",
        models_path: Some("/v1/models"),
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("STEP_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Together,
        id: "together",
        aliases: &["together_compatible", "together_ai", "together-ai"],
        base_url: "https://api.together.xyz",
        chat_completions_path: "/v1/chat/completions",
        models_path: None,
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("TOGETHER_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Venice,
        id: "venice",
        aliases: &["venice_compatible"],
        base_url: "https://api.venice.ai",
        chat_completions_path: "/api/v1/chat/completions",
        models_path: Some("/api/v1/models"),
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("VENICE_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::VercelAiGateway,
        id: "vercel_ai_gateway",
        aliases: &["vercel-ai-gateway", "vercel_ai", "vercel-ai", "vercel"],
        base_url: "https://ai-gateway.vercel.sh/v1",
        chat_completions_path: "/chat/completions",
        models_path: Some("/models"),
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("AI_GATEWAY_API_KEY"),
        api_key_env_aliases: &["VERCEL_API_KEY"],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Vllm,
        id: "vllm",
        aliases: &["vllm_compatible"],
        base_url: "http://127.0.0.1:8000",
        chat_completions_path: "/v1/chat/completions",
        models_path: None,
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: None,
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Volcengine,
        id: "volcengine",
        aliases: &[
            "volcengine_custom",
            "volcengine_compatible",
            "doubao",
            "ark",
        ],
        base_url: "https://ark.cn-beijing.volces.com",
        chat_completions_path: "/api/v3/chat/completions",
        models_path: None,
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("ARK_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::Volcengine,
    },
    ProviderProfile {
        kind: ProviderKind::VolcengineCoding,
        id: "volcengine_coding",
        aliases: &["volcengine_coding_compatible"],
        base_url: "https://ark.cn-beijing.volces.com/api/coding/v3",
        chat_completions_path: "/chat/completions",
        models_path: Some("/models"),
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("ARK_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::Volcengine,
    },
    ProviderProfile {
        kind: ProviderKind::Xai,
        id: "xai",
        aliases: &["xai_compatible", "grok"],
        base_url: "https://api.x.ai",
        chat_completions_path: "/v1/chat/completions",
        models_path: Some("/v1/language-models"),
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("XAI_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Zai,
        id: "zai",
        aliases: &["zai_compatible", "z.ai"],
        base_url: "https://api.z.ai",
        chat_completions_path: "/api/paas/v4/chat/completions",
        models_path: None,
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("ZAI_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::Zhipu,
        id: "zhipu",
        aliases: &["zhipu_compatible", "glm", "bigmodel"],
        base_url: "https://open.bigmodel.cn",
        chat_completions_path: "/api/paas/v4/chat/completions",
        models_path: None,
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("ZHIPUAI_API_KEY"),
        api_key_env_aliases: &["ZHIPU_API_KEY"],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
];

fn default_provider_model() -> String {
    "auto".to_owned()
}

fn default_provider_base_url() -> String {
    "https://api.openai.com".to_owned()
}

fn resolve_provider_template(kind: ProviderKind, value: &str) -> String {
    if kind == ProviderKind::Bedrock {
        resolve_bedrock_template(value)
    } else {
        value.to_owned()
    }
}

fn resolve_bedrock_template(value: &str) -> String {
    let Some(region) = resolved_bedrock_region() else {
        return value.to_owned();
    };
    value.replace("<region>", region.as_str())
}

fn resolved_bedrock_region() -> Option<String> {
    first_non_empty_env_value(&[
        "BEDROCK_AWS_REGION".to_owned(),
        "AWS_REGION".to_owned(),
        "AWS_DEFAULT_REGION".to_owned(),
    ])
}

fn default_openai_chat_path() -> String {
    "/v1/chat/completions".to_owned()
}

fn default_openai_responses_path() -> String {
    "/v1/responses".to_owned()
}

fn default_request_path_for_wire_api(wire_api: ProviderWireApi) -> String {
    match wire_api {
        ProviderWireApi::ChatCompletions => default_openai_chat_path(),
        ProviderWireApi::Responses => default_openai_responses_path(),
    }
}

const fn default_temperature() -> f64 {
    0.2
}

const fn default_provider_timeout_ms() -> u64 {
    30_000
}

const fn default_provider_retry_max_attempts() -> usize {
    3
}

const fn default_provider_retry_initial_backoff_ms() -> u64 {
    300
}

const fn default_provider_retry_max_backoff_ms() -> u64 {
    3_000
}

const fn default_model_catalog_cache_ttl_ms() -> u64 {
    30_000
}

const fn default_model_catalog_stale_if_error_ms() -> u64 {
    120_000
}

const fn default_model_catalog_cache_max_entries() -> usize {
    32
}

const fn default_model_candidate_cooldown_ms() -> u64 {
    300_000
}

const fn default_model_candidate_cooldown_max_ms() -> u64 {
    3_600_000
}

const fn default_model_candidate_cooldown_max_entries() -> usize {
    64
}

const fn default_profile_cooldown_ms() -> u64 {
    60_000
}

const fn default_profile_cooldown_max_ms() -> u64 {
    3_600_000
}

const fn default_profile_auth_reject_disable_ms() -> u64 {
    21_600_000
}

const fn default_profile_state_max_entries() -> usize {
    256
}

fn collect_non_empty_env_values(keys: &[String]) -> Vec<String> {
    let mut values = Vec::new();
    for key in keys {
        if let Ok(value) = env::var(key) {
            for candidate in split_secret_candidates(&value) {
                push_unique_value(&mut values, &candidate);
            }
        }
    }
    values
}

fn first_non_empty_env_value(keys: &[String]) -> Option<String> {
    collect_non_empty_env_values(keys).into_iter().next()
}

fn first_non_empty_env_name(keys: &[String]) -> Option<String> {
    for key in keys {
        if env::var(key)
            .ok()
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Some(key.clone());
        }
    }
    None
}

fn push_unique_env_key(keys: &mut Vec<String>, maybe_key: Option<&str>) {
    let Some(raw) = maybe_key else {
        return;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return;
    }
    if keys.iter().any(|existing| existing == trimmed) {
        return;
    }
    keys.push(trimmed.to_owned());
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    let raw = value?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed)
}

fn is_absolute_url(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.starts_with("http://") || trimmed.starts_with("https://")
}

fn clamp_non_negative_u64(value: u64, max: u64) -> u64 {
    if value == 0 { 0 } else { value.min(max) }
}

fn clamp_u64_with_floor(value: u64, max: u64, floor: u64) -> u64 {
    value.clamp(floor, max)
}

fn clamp_usize_at_least_one(value: usize, max: usize) -> usize {
    value.clamp(1, max)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum InlineSecretResolution {
    Resolved(String),
    ExplicitEnvMissing,
    NotInlineEnvReference,
}

fn resolve_inline_secret(raw: &str) -> InlineSecretResolution {
    let Some(env_key) = parse_explicit_env_reference(raw) else {
        return InlineSecretResolution::NotInlineEnvReference;
    };
    match env::var(env_key) {
        Ok(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                InlineSecretResolution::ExplicitEnvMissing
            } else {
                InlineSecretResolution::Resolved(trimmed.to_owned())
            }
        }
        Err(_) => InlineSecretResolution::ExplicitEnvMissing,
    }
}

fn split_secret_candidates(raw: &str) -> Vec<String> {
    let mut values = Vec::new();
    for value in raw.split([',', ';', '\n', '\r']) {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        push_unique_value(&mut values, trimmed);
    }
    values
}

fn push_unique_value(values: &mut Vec<String>, raw: &str) {
    if values.iter().any(|existing| existing == raw) {
        return;
    }
    values.push(raw.to_owned());
}

fn normalize_hint_values(values: &[String]) -> Vec<String> {
    let mut normalized = Vec::new();
    for raw in values {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lowercased = trimmed.to_ascii_lowercase();
        if normalized.iter().any(|existing| existing == &lowercased) {
            continue;
        }
        normalized.push(lowercased);
    }
    normalized
}

fn normalize_sqlite_path(raw: Option<&str>) -> Option<PathBuf> {
    let trimmed = non_empty(raw)?;
    if trimmed.eq_ignore_ascii_case("memory") || trimmed == ":memory:" {
        return Some(PathBuf::from(":memory:"));
    }
    Some(expand_path(trimmed))
}

fn normalize_api_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.starts_with('/') {
        return trimmed.to_owned();
    }
    format!("/{trimmed}")
}

fn is_same_base_url(left: &str, right: &str) -> bool {
    left.trim().trim_end_matches('/') == right.trim().trim_end_matches('/')
}

fn matches_region_endpoint_url(endpoint: &str, base_url: &str) -> bool {
    let endpoint = endpoint.trim().trim_end_matches('/');
    let base_url = base_url.trim().trim_end_matches('/');
    endpoint == base_url
        || endpoint
            .strip_prefix(base_url)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn is_same_chat_path(left: &str, right: &str) -> bool {
    normalize_api_path(left) == normalize_api_path(right)
}

fn join_base_with_path(base_url: &str, path: &str, fallback_path: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    let path = normalize_api_path(path);
    if path.is_empty() {
        return format!("{base}{}", normalize_api_path(fallback_path));
    }
    format!("{base}{path}")
}

fn derive_models_path(chat_path: &str) -> String {
    let normalized = normalize_api_path(chat_path);
    if normalized.is_empty() {
        return "/v1/models".to_owned();
    }

    if let Some(prefix) = normalized.strip_suffix("/chat/completions") {
        let prefix = if prefix.is_empty() { "" } else { prefix };
        return format!("{prefix}/models");
    }
    if let Some(prefix) = normalized.strip_suffix("/completions") {
        let prefix = if prefix.is_empty() { "" } else { prefix };
        return format!("{prefix}/models");
    }
    if let Some(prefix) = normalized.strip_suffix("/responses") {
        let prefix = if prefix.is_empty() { "" } else { prefix };
        return format!("{prefix}/models");
    }

    "/v1/models".to_owned()
}

fn derive_responses_path(chat_path: &str) -> String {
    let normalized = normalize_api_path(chat_path);
    if normalized.is_empty() {
        return default_openai_responses_path();
    }

    if let Some(prefix) = normalized.strip_suffix("/chat/completions") {
        let prefix = if prefix.is_empty() { "" } else { prefix };
        return format!("{prefix}/responses");
    }
    if let Some(prefix) = normalized.strip_suffix("/completions") {
        let prefix = if prefix.is_empty() { "" } else { prefix };
        return format!("{prefix}/responses");
    }
    if normalized.ends_with("/responses") {
        return normalized;
    }

    default_openai_responses_path()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScopedEnv;

    #[test]
    fn provider_profile_lookup_matches_kind() {
        for kind in ProviderKind::all_sorted() {
            assert_eq!(kind.profile().kind, *kind);
        }
    }

    #[test]
    fn custom_models_endpoint_avoids_double_v1_suffix() {
        let config = ProviderConfig {
            kind: ProviderKind::Custom,
            base_url: "https://example.test/v1".to_owned(),
            ..ProviderConfig::default()
        };

        assert_eq!(
            config.endpoint(),
            "https://example.test/v1/chat/completions"
        );
        assert_eq!(config.models_endpoint(), "https://example.test/v1/models");
    }

    #[test]
    fn explicit_api_key_binding_beats_default_oauth_fallback() {
        let mut env = ScopedEnv::new();
        env.set("OPENAI_API_KEY", "api-key-wins");
        env.set("OPENAI_CODEX_OAUTH_TOKEN", "oauth-fallback-should-not-win");

        let config = ProviderConfig {
            kind: ProviderKind::Openai,
            api_key: Some("${OPENAI_API_KEY}".to_owned()),
            ..ProviderConfig::default()
        };

        assert_eq!(config.oauth_access_token(), None);
        assert_eq!(config.api_key().as_deref(), Some("api-key-wins"));
        assert_eq!(
            config.resolved_auth_secret().as_deref(),
            Some("api-key-wins")
        );
        assert_eq!(
            config.authorization_header().as_deref(),
            Some("Bearer api-key-wins")
        );
    }

    #[test]
    fn fresh_minimax_provider_does_not_seed_hidden_preferred_models() {
        let config = ProviderConfig::fresh_for_kind(ProviderKind::Minimax);

        assert_eq!(config.model, "auto");
        assert!(
            config.preferred_models.is_empty(),
            "provider defaults should not inject hidden runtime fallback models: {config:#?}"
        );
    }

    #[test]
    fn configured_auto_model_candidates_require_explicit_preferred_models() {
        let config = ProviderConfig {
            kind: ProviderKind::Minimax,
            model: "auto".to_owned(),
            ..ProviderConfig::default()
        };

        assert!(
            config.configured_auto_model_candidates().is_empty(),
            "auto-model fallback candidates should only exist when the operator configured preferred_models explicitly"
        );
    }

    #[test]
    fn only_reviewed_providers_expose_onboarding_models() {
        assert_eq!(
            ProviderKind::Deepseek.recommended_onboarding_model(),
            Some("deepseek-chat")
        );
        assert_eq!(
            ProviderKind::Minimax.recommended_onboarding_model(),
            Some("MiniMax-M2.5")
        );
        assert_eq!(
            ProviderKind::KimiCoding.recommended_onboarding_model(),
            None
        );
        assert_eq!(ProviderKind::Openai.recommended_onboarding_model(), None);
    }

    #[test]
    fn model_catalog_probe_recovery_requires_explicit_model_for_reviewed_auto_provider() {
        let config = ProviderConfig {
            kind: ProviderKind::Deepseek,
            model: "auto".to_owned(),
            ..ProviderConfig::default()
        };

        assert_eq!(
            config.model_catalog_probe_recovery(),
            ModelCatalogProbeRecovery::RequiresExplicitModel {
                recommended_onboarding_model: Some("deepseek-chat"),
            }
        );
    }

    #[test]
    fn model_catalog_probe_recovery_prefers_explicit_runtime_configuration() {
        let explicit = ProviderConfig {
            kind: ProviderKind::Deepseek,
            model: "deepseek-chat".to_owned(),
            ..ProviderConfig::default()
        };
        assert_eq!(
            explicit.model_catalog_probe_recovery(),
            ModelCatalogProbeRecovery::ExplicitModel("deepseek-chat".to_owned())
        );

        let preferred = ProviderConfig {
            kind: ProviderKind::Deepseek,
            model: "auto".to_owned(),
            preferred_models: vec![
                "deepseek-chat".to_owned(),
                "deepseek-chat".to_owned(),
                "deepseek-reasoner".to_owned(),
            ],
            ..ProviderConfig::default()
        };
        assert_eq!(
            preferred.model_catalog_probe_recovery(),
            ModelCatalogProbeRecovery::ConfiguredPreferredModels(vec![
                "deepseek-chat".to_owned(),
                "deepseek-reasoner".to_owned(),
            ])
        );
    }
}
