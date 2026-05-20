use std::{collections::BTreeMap, env, path::PathBuf};

use loong_contracts::SecretRef;
use serde::{Deserialize, Deserializer, Serialize};

mod catalog;

pub use self::catalog::parse_provider_kind_id;
use self::catalog::{
    PROVIDER_PROFILES, build_provider_descriptor_feature, find_cross_routed_validation_profile,
    provider_descriptor_aliases, provider_descriptor_env_aliases, provider_descriptor_headers,
    provider_descriptor_region_variants,
};
use super::shared::{
    ConfigValidationIssue, EnvPointerValidationHint, default_loong_home, expand_path,
    validate_env_pointer_field, validate_secret_ref_env_pointer_field,
};
use crate::secrets::{
    SecretLookup, has_configured_secret_ref, resolve_secret_lookup, secret_ref_env_name,
};

pub(crate) const GITHUB_COPILOT_EDITOR_VERSION: &str = "vscode/1.85.1";
pub(crate) const GITHUB_COPILOT_EDITOR_PLUGIN_VERSION: &str = "copilot/1.155.0";
pub(crate) const GITHUB_COPILOT_INTEGRATION_ID: &str = "vscode-chat";
pub(crate) const GITHUB_COPILOT_USER_AGENT: &str = "GithubCopilot/1.155.0";
pub(crate) const GITHUB_COPILOT_OAUTH_TOKEN_ENV: &str = "GITHUB_COPILOT_OAUTH_TOKEN";
pub(crate) const ANTHROPIC_DEFAULT_HEADERS: [(&str, &str); 1] =
    [("anthropic-version", "2023-06-01")];
pub(crate) const OPENCODE_API_KEY_ENV: &str = "OPENCODE_API_KEY";
pub(crate) const OPENCODE_ZEN_BASE_URL: &str = "https://opencode.ai/zen/v1";
pub(crate) const OPENCODE_GO_BASE_URL: &str = "https://opencode.ai/zen/go/v1";
pub(crate) const GITHUB_COPILOT_DEFAULT_HEADERS: [(&str, &str); 3] = [
    ("Editor-Version", GITHUB_COPILOT_EDITOR_VERSION),
    (
        "Editor-Plugin-Version",
        GITHUB_COPILOT_EDITOR_PLUGIN_VERSION,
    ),
    ("Copilot-Integration-Id", GITHUB_COPILOT_INTEGRATION_ID),
];

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

impl ProviderProfile {
    pub fn alternative_auth_configuration_hint(self) -> Option<&'static str> {
        let kind = self.kind;
        if kind == ProviderKind::Bedrock {
            return Some(
                "configure AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY with BEDROCK_AWS_REGION, AWS_REGION, or AWS_DEFAULT_REGION for SigV4",
            );
        }
        if kind == ProviderKind::Custom {
            return Some("add `Authorization` / `X-API-Key` in `provider.headers`");
        }
        None
    }

    pub fn auth_guidance_hint(self) -> Option<String> {
        let feature_family = self.feature_family;
        if feature_family != ProviderFeatureFamily::Volcengine {
            return None;
        }

        let provider_label = self.auth_guidance_provider_label();
        let env_name = self.default_api_key_env.unwrap_or("PROVIDER_API_KEY");
        let hint = format!(
            "Loong's {provider_label} OpenAI-compatible path uses `provider.api_key` / `{env_name}` and sends `Authorization: Bearer <{env_name}>`; AK/SK request signing is not used on this path"
        );
        Some(hint)
    }

    fn auth_guidance_provider_label(self) -> &'static str {
        let kind = self.kind;
        if matches!(kind, ProviderKind::Byteplus | ProviderKind::ByteplusCoding) {
            return "BytePlus";
        }
        "Volcengine"
    }
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
    XGoogApiKey,
}

impl ProviderAuthScheme {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bearer => "bearer",
            Self::XApiKey => "x_api_key",
            Self::XGoogApiKey => "x_goog_api_key",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderFeatureFamily {
    OpenAiCompatible,
    Anthropic,
    Bedrock,
    Volcengine,
}

impl ProviderFeatureFamily {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "openai_compatible",
            Self::Anthropic => "anthropic",
            Self::Bedrock => "bedrock",
            Self::Volcengine => "volcengine",
        }
    }

    pub fn support_facts(self) -> ProviderFeatureSupportFacts {
        let gate_name = self.feature_gate_name();
        let enabled_in_build = self.is_enabled_in_build();
        let disabled_message = self.disabled_message();

        ProviderFeatureSupportFacts {
            family: self,
            gate_name,
            enabled_in_build,
            disabled_message,
        }
    }

    pub fn feature_gate_name(self) -> &'static str {
        match self {
            Self::Anthropic => "provider-anthropic",
            Self::Bedrock => "provider-bedrock",
            Self::Volcengine => "provider-volcengine",
            Self::OpenAiCompatible => "provider-openai",
        }
    }

    pub fn disabled_message(self) -> String {
        let subject = self.disabled_message_subject();
        let feature_name = self.feature_gate_name();
        let message = format!("{subject} is disabled (enable feature `{feature_name}`)");
        message
    }

    pub fn is_enabled_in_build(self) -> bool {
        match self {
            Self::Anthropic => cfg!(feature = "provider-anthropic"),
            Self::Bedrock => cfg!(feature = "provider-bedrock"),
            Self::Volcengine => cfg!(feature = "provider-volcengine"),
            Self::OpenAiCompatible => cfg!(feature = "provider-openai"),
        }
    }

    fn disabled_message_subject(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic provider family",
            Self::Bedrock => "bedrock provider family",
            Self::Volcengine => "volcengine provider family",
            Self::OpenAiCompatible => "openai-compatible provider family",
        }
    }
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
struct ProviderEffectiveUrlValues {
    resolved_base_url: String,
    endpoint: String,
    models_endpoint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProviderUrlValidationProfile {
    kind: ProviderKind,
    extra_canonical_url_fingerprints: &'static [&'static str],
    required_path_fragments: &'static [&'static str],
    forbidden_path_fragments: &'static [&'static str],
    forbidden_path_exceptions: &'static [&'static str],
    route_expectation: &'static str,
    path_validation_hint: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderFeatureSupportFacts {
    pub family: ProviderFeatureFamily,
    pub gate_name: &'static str,
    pub enabled_in_build: bool,
    pub disabled_message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAuthSupportFacts {
    pub hint_env_names: Vec<String>,
    pub requires_explicit_configuration: bool,
    pub guidance_hint: Option<String>,
    pub alternative_configuration_hint: Option<String>,
    pub missing_configuration_message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRegionEndpointSupportFacts {
    pub note: Option<String>,
    pub catalog_failure_hint: Option<String>,
    pub request_failure_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSupportFacts {
    pub feature: ProviderFeatureSupportFacts,
    pub auth: ProviderAuthSupportFacts,
    pub region_endpoint: ProviderRegionEndpointSupportFacts,
}

pub const PROVIDER_DESCRIPTOR_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProviderDescriptorSchema {
    pub version: u32,
    pub surface: &'static str,
    pub purpose: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProviderDescriptorHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProviderDescriptorFeature {
    pub family: String,
    pub gate_name: String,
    pub enabled_in_build: bool,
    pub disabled_message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProviderDescriptorAuth {
    pub scheme: String,
    pub auth_optional: bool,
    pub model_probe_auth_optional: bool,
    pub default_api_key_env: Option<String>,
    pub api_key_env_aliases: Vec<String>,
    pub default_oauth_access_token_env: Option<String>,
    pub oauth_access_token_env_aliases: Vec<String>,
    pub hint_env_names: Vec<String>,
    pub requires_explicit_configuration: bool,
    pub guidance_hint: Option<String>,
    pub alternative_configuration_hint: Option<String>,
    pub missing_configuration_message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProviderDescriptorRegionVariant {
    pub label: String,
    pub base_url: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProviderDescriptorRegionEndpoint {
    pub family_label: Option<String>,
    pub variants: Vec<ProviderDescriptorRegionVariant>,
    pub note: Option<String>,
    pub catalog_failure_hint: Option<String>,
    pub request_failure_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProviderDescriptorDocument {
    pub schema: ProviderDescriptorSchema,
    pub kind: String,
    pub display_name: String,
    pub aliases: Vec<String>,
    pub protocol_family: String,
    pub default_headers: Vec<ProviderDescriptorHeader>,
    pub default_user_agent: Option<String>,
    pub configuration_hint: Option<String>,
    pub default_model: Option<String>,
    pub recommended_onboarding_model: Option<String>,
    pub feature: ProviderDescriptorFeature,
    pub auth: ProviderDescriptorAuth,
    pub region_endpoint: ProviderDescriptorRegionEndpoint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelCatalogProbeRecovery {
    ExplicitModel(String),
    ConfiguredPreferredModels(Vec<String>),
    RequiresExplicitModel {
        recommended_onboarding_model: Option<&'static str>,
    },
}

/// Information about a provider's region endpoint variants.
/// Used to allow users to select between different regional endpoints (e.g., CN vs Global).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRegionEndpointInfo {
    /// Display name for the provider family (e.g., "MiniMax", "Moonshot Kimi").
    pub family_label: &'static str,
    /// Region variants ordered with the default endpoint first.
    pub variants: Vec<RegionVariant>,
}

/// A region endpoint variant with label and base URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionVariant {
    /// Label for the region (e.g., "CN", "Global").
    pub label: &'static str,
    /// Base URL for the region endpoint.
    pub base_url: &'static str,
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
        alias = "cloudflare-ai-gateway",
        alias = "cloudflare"
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
    #[serde(rename = "github-copilot", alias = "github_copilot", alias = "copilot")]
    GithubCopilot,
    #[serde(alias = "fireworks_compatible", alias = "fireworks-ai")]
    Fireworks,
    #[serde(alias = "mistral_compatible")]
    Mistral,
    #[serde(alias = "minimax_compatible")]
    Minimax,
    #[serde(alias = "novita_compatible")]
    Novita,
    #[serde(
        alias = "nvidia_compatible",
        alias = "nvidia_nim",
        alias = "nvidia-nim",
        alias = "build.nvidia.com"
    )]
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
    #[serde(alias = "opencode", alias = "opencode-zen")]
    OpencodeZen,
    #[serde(alias = "opencode-go", alias = "opencode_go")]
    OpencodeGo,
    #[serde(alias = "openrouter_compatible")]
    Openrouter,
    #[serde(alias = "perplexity_compatible")]
    Perplexity,
    #[serde(alias = "qianfan_compatible", alias = "baidu")]
    Qianfan,
    #[serde(alias = "qwen_compatible", alias = "dashscope")]
    Qwen,
    #[serde(alias = "bailian_coding_compatible")]
    BailianCoding,
    #[serde(alias = "sambanova_compatible", alias = "samba_nova")]
    Sambanova,
    #[serde(alias = "sglang_compatible")]
    Sglang,
    #[serde(alias = "siliconflow_compatible")]
    Siliconflow,
    #[serde(alias = "stepfun_compatible")]
    Stepfun,
    #[serde(alias = "stepfun_step_plan", alias = "step_plan")]
    StepPlan,
    #[serde(
        alias = "together_compatible",
        alias = "together_ai",
        alias = "together-ai"
    )]
    Together,
    #[serde(alias = "venice_compatible")]
    Venice,
    #[serde(
        alias = "vercel_ai",
        alias = "vercel-ai",
        alias = "vercel_ai_gateway",
        alias = "vercel-ai-gateway",
        alias = "vercel"
    )]
    VercelAiGateway,
    #[serde(
        alias = "volcengine_custom",
        alias = "volcengine_compatible",
        alias = "doubao",
        alias = "ark"
    )]
    Volcengine,
    #[serde(alias = "volcengine_coding_compatible")]
    VolcengineCoding,
    #[serde(alias = "xai_compatible", alias = "grok")]
    Xai,
    #[serde(
        alias = "xiaomi_compatible",
        alias = "xiaomi_mimo",
        alias = "xiaomi-mimo",
        alias = "mimo",
        alias = "mimo_compatible"
    )]
    Xiaomi,
    #[serde(alias = "zai_compatible", alias = "z.ai")]
    Zai,
    #[serde(alias = "zhipu_compatible", alias = "glm", alias = "bigmodel")]
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
    pub api_key: Option<SecretRef>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(skip_serializing, default)]
    pub api_key_env_explicit: bool,
    #[serde(default)]
    pub oauth_access_token: Option<SecretRef>,
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
    #[serde(default)]
    pub stop: Vec<String>,
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
            stop: Vec::new(),
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
            api_key: Option<SecretRef>,
            #[serde(default)]
            api_key_env: Option<String>,
            #[serde(default)]
            oauth_access_token: Option<SecretRef>,
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
            #[serde(default)]
            stop: Vec<String>,
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
        let api_key_env_explicit = raw.api_key_env.is_some();
        let oauth_access_token_env_explicit = raw.oauth_access_token_env.is_some();

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
            stop: raw.stop,
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
        self.api_key_env_explicit = self.api_key_env.is_some();
        self.oauth_access_token_env_explicit = self.oauth_access_token_env.is_some();
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
        self.api_key_env_explicit = api_key_env.is_some();
        self.api_key_env = api_key_env;
    }

    pub fn set_api_key_env_binding(&mut self, api_key_env: Option<String>) {
        let normalized = api_key_env
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        self.api_key = normalized.map(|env| SecretRef::Env { env });
        self.set_api_key_env(None);
    }

    pub fn clear_api_key_env_binding(&mut self) {
        if secret_ref_env_name(self.api_key.as_ref()).is_some() {
            self.api_key = None;
        }
        self.set_api_key_env(None);
    }

    pub fn set_oauth_access_token_env(&mut self, oauth_access_token_env: Option<String>) {
        self.oauth_access_token_env_explicit = oauth_access_token_env.is_some();
        self.oauth_access_token_env = oauth_access_token_env;
    }

    pub fn set_oauth_access_token_env_binding(&mut self, oauth_access_token_env: Option<String>) {
        let normalized = oauth_access_token_env
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        self.oauth_access_token = normalized.map(|env| SecretRef::Env { env });
        self.set_oauth_access_token_env(None);
    }

    pub fn clear_oauth_access_token_env_binding(&mut self) {
        if secret_ref_env_name(self.oauth_access_token.as_ref()).is_some() {
            self.oauth_access_token = None;
        }
        self.set_oauth_access_token_env(None);
    }

    pub fn canonicalize_configured_auth_env_bindings(&mut self) {
        let configured_api_key_env = self.configured_api_key_env_override();
        let api_key_has_non_env_secret = has_configured_secret_ref(self.api_key.as_ref())
            && secret_ref_env_name(self.api_key.as_ref()).is_none();
        if api_key_has_non_env_secret {
            self.set_api_key_env(None);
        } else {
            self.set_api_key_env_binding(configured_api_key_env);
        }

        let configured_oauth_env = self.configured_oauth_access_token_env_override();
        let oauth_has_non_env_secret = has_configured_secret_ref(self.oauth_access_token.as_ref())
            && secret_ref_env_name(self.oauth_access_token.as_ref()).is_none();
        if oauth_has_non_env_secret {
            self.set_oauth_access_token_env(None);
        } else {
            self.set_oauth_access_token_env_binding(configured_oauth_env);
        }
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
        if let Err(issue) = validate_secret_ref_env_pointer_field(
            api_key_inline_field_path.as_str(),
            self.api_key.as_ref(),
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
        if let Err(issue) = validate_secret_ref_env_pointer_field(
            oauth_inline_field_path.as_str(),
            self.oauth_access_token.as_ref(),
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
            ProviderAuthScheme::XApiKey | ProviderAuthScheme::XGoogApiKey => self.api_key(),
        }
    }

    pub fn resolved_auth_env_name(&self) -> Option<String> {
        match self.kind.auth_scheme() {
            ProviderAuthScheme::Bearer => {
                let oauth_env_name = secret_ref_env_name(self.oauth_access_token.as_ref());
                if let Some(oauth_env_name) = oauth_env_name {
                    return Some(oauth_env_name);
                }
                if has_configured_secret_ref(self.oauth_access_token.as_ref()) {
                    return None;
                }
                if let Some(env_name) =
                    first_non_empty_env_name(&self.oauth_access_token_env_names())
                {
                    return Some(env_name);
                }
                let api_key_env_name = secret_ref_env_name(self.api_key.as_ref());
                if let Some(api_key_env_name) = api_key_env_name {
                    return Some(api_key_env_name);
                }
                if has_configured_secret_ref(self.api_key.as_ref()) {
                    return None;
                }
                first_non_empty_env_name(&self.api_key_env_names())
            }
            ProviderAuthScheme::XApiKey | ProviderAuthScheme::XGoogApiKey => {
                let api_key_env_name = secret_ref_env_name(self.api_key.as_ref());
                if let Some(api_key_env_name) = api_key_env_name {
                    return Some(api_key_env_name);
                }
                if has_configured_secret_ref(self.api_key.as_ref()) {
                    return None;
                }
                first_non_empty_env_name(&self.api_key_env_names())
            }
        }
    }

    pub fn auth_hint_env_names(&self) -> Vec<String> {
        let mut env_names = Vec::new();
        match self.kind.auth_scheme() {
            ProviderAuthScheme::Bearer => {
                self.push_oauth_access_token_hint_env_names(&mut env_names);
                self.push_api_key_hint_env_names(&mut env_names);
            }
            ProviderAuthScheme::XApiKey | ProviderAuthScheme::XGoogApiKey => {
                self.push_api_key_hint_env_names(&mut env_names);
            }
        }
        env_names
    }

    pub fn support_facts(&self) -> ProviderSupportFacts {
        let feature = self.kind.feature_family().support_facts();
        let auth = self.build_auth_support_facts();
        let region_endpoint = self.build_region_endpoint_support_facts();

        ProviderSupportFacts {
            feature,
            auth,
            region_endpoint,
        }
    }

    pub fn descriptor_document(&self) -> ProviderDescriptorDocument {
        let profile = self.kind.profile();
        let support_facts = self.support_facts();
        let schema = ProviderDescriptorSchema {
            version: PROVIDER_DESCRIPTOR_SCHEMA_VERSION,
            surface: "provider_descriptor",
            purpose: "internal_sdk_contract",
        };
        let kind = self.kind.as_str().to_owned();
        let display_name = self.kind.display_name().to_owned();
        let aliases = provider_descriptor_aliases(profile);
        let protocol_family = self.kind.protocol_family().as_str().to_owned();
        let default_headers = provider_descriptor_headers(profile);
        let default_user_agent = self.kind.default_user_agent().map(str::to_owned);
        let configuration_hint = self
            .configuration_hint()
            .or_else(|| self.kind.configuration_hint().map(str::to_owned));
        let default_model = self.kind.default_model().map(str::to_owned);
        let recommended_onboarding_model =
            self.kind.recommended_onboarding_model().map(str::to_owned);
        let feature = build_provider_descriptor_feature(&support_facts.feature);
        let auth = self.build_provider_descriptor_auth(&support_facts.auth);
        let region_endpoint =
            self.build_provider_descriptor_region_endpoint(&support_facts.region_endpoint);

        ProviderDescriptorDocument {
            schema,
            kind,
            display_name,
            aliases,
            protocol_family,
            default_headers,
            default_user_agent,
            configuration_hint,
            default_model,
            recommended_onboarding_model,
            feature,
            auth,
            region_endpoint,
        }
    }

    pub fn requires_explicit_auth_configuration(&self) -> bool {
        let support_facts = self.support_facts();
        support_facts.auth.requires_explicit_configuration
    }

    pub fn auth_guidance_hint(&self) -> Option<String> {
        let support_facts = self.support_facts();
        support_facts.auth.guidance_hint
    }

    pub fn missing_auth_configuration_message(&self) -> String {
        let support_facts = self.support_facts();
        support_facts.auth.missing_configuration_message
    }

    fn build_auth_support_facts(&self) -> ProviderAuthSupportFacts {
        let env_names = self.auth_hint_env_names();
        let requires_explicit_configuration = !env_names.is_empty();
        let guidance_hint = self.build_auth_guidance_hint();
        let alternative_configuration_hint = self.build_alternative_auth_configuration_hint();
        let missing_configuration_message = self.build_missing_auth_configuration_message(
            &env_names,
            guidance_hint.as_deref(),
            alternative_configuration_hint.as_deref(),
        );

        ProviderAuthSupportFacts {
            hint_env_names: env_names,
            requires_explicit_configuration,
            guidance_hint,
            alternative_configuration_hint,
            missing_configuration_message,
        }
    }

    fn build_provider_descriptor_auth(
        &self,
        auth_support: &ProviderAuthSupportFacts,
    ) -> ProviderDescriptorAuth {
        let scheme = self.kind.auth_scheme().as_str().to_owned();
        let auth_optional = self.kind.auth_optional();
        let model_probe_auth_optional = self.kind.model_probe_auth_optional();
        let default_api_key_env = self.kind.default_api_key_env().map(str::to_owned);
        let api_key_env_aliases = provider_descriptor_env_aliases(self.kind.api_key_env_aliases());
        let default_oauth_access_token_env = self
            .kind
            .default_oauth_access_token_env()
            .map(str::to_owned);
        let oauth_access_token_env_aliases =
            provider_descriptor_env_aliases(self.kind.oauth_access_token_env_aliases());
        let hint_env_names = auth_support.hint_env_names.clone();
        let requires_explicit_configuration = auth_support.requires_explicit_configuration;
        let guidance_hint = auth_support.guidance_hint.clone();
        let alternative_configuration_hint = auth_support.alternative_configuration_hint.clone();
        let missing_configuration_message = auth_support.missing_configuration_message.clone();

        ProviderDescriptorAuth {
            scheme,
            auth_optional,
            model_probe_auth_optional,
            default_api_key_env,
            api_key_env_aliases,
            default_oauth_access_token_env,
            oauth_access_token_env_aliases,
            hint_env_names,
            requires_explicit_configuration,
            guidance_hint,
            alternative_configuration_hint,
            missing_configuration_message,
        }
    }

    fn build_missing_auth_configuration_message(
        &self,
        env_names: &[String],
        guidance_hint: Option<&str>,
        alternative_configuration_hint: Option<&str>,
    ) -> String {
        let mut configuration_paths = vec!["configure provider credentials".to_owned()];
        if !env_names.is_empty() {
            configuration_paths.push(format!("set {} in env", env_names.join(", ")));
        }
        if let Some(hint) = alternative_configuration_hint {
            configuration_paths.push(hint.to_owned());
        }
        let mut message = "provider credentials are missing".to_owned();
        if let Some(detail) = self.missing_auth_runtime_detail() {
            message.push_str("; ");
            message.push_str(detail.as_str());
        }
        message.push_str("; ");
        message.push_str(join_guidance_options(&configuration_paths).as_str());
        if let Some(hint) = guidance_hint {
            message.push(' ');
            message.push_str(hint);
        }
        message
    }

    fn build_auth_guidance_hint(&self) -> Option<String> {
        let profile = self.kind.profile();
        profile.auth_guidance_hint()
    }

    fn push_oauth_access_token_hint_env_names(&self, env_names: &mut Vec<String>) {
        push_secret_ref_env_name(env_names, self.oauth_access_token.as_ref());
        if has_configured_secret_ref(self.oauth_access_token.as_ref()) {
            return;
        }

        let oauth_env_names = self.oauth_access_token_env_names();
        for oauth_env_name in oauth_env_names {
            push_unique_env_key(env_names, Some(oauth_env_name.as_str()));
        }
    }

    fn push_api_key_hint_env_names(&self, env_names: &mut Vec<String>) {
        push_secret_ref_env_name(env_names, self.api_key.as_ref());
        if has_configured_secret_ref(self.api_key.as_ref()) {
            return;
        }

        let api_key_env_names = self.api_key_env_names();
        for api_key_env_name in api_key_env_names {
            push_unique_env_key(env_names, Some(api_key_env_name.as_str()));
        }
    }

    fn build_alternative_auth_configuration_hint(&self) -> Option<String> {
        let profile = self.kind.profile();
        let hint = profile.alternative_auth_configuration_hint();
        hint.map(str::to_owned)
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
                            "Responses endpoint {} is running in compatibility mode; Loong will retry chat_completions automatically via {} if Responses is rejected",
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
            .unwrap_or_else(|| default_loong_home().join("provider-profile-state.sqlite3"))
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
        let secret_lookup = resolve_secret_lookup(self.oauth_access_token.as_ref());
        match secret_lookup {
            SecretLookup::Value(value) => return Some(value),
            SecretLookup::Missing => return None,
            SecretLookup::Absent => {}
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
        let secret_lookup = resolve_secret_lookup(self.api_key.as_ref());
        match secret_lookup {
            SecretLookup::Value(value) => return split_secret_candidates(value.as_str()),
            SecretLookup::Missing => return Vec::new(),
            SecretLookup::Absent => {}
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
        let effective_urls = self.effective_url_values();
        let cross_routing_hint = self.cross_routing_configuration_hint(&effective_urls);
        if let Some(hint) = cross_routing_hint {
            return Some(hint);
        }

        let path_validation_hint = self.path_validation_configuration_hint(&effective_urls);
        if let Some(hint) = path_validation_hint {
            return Some(hint);
        }
        if let Some(hint) = self.opencode_configuration_hint() {
            return Some(hint);
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

    fn effective_url_values(&self) -> ProviderEffectiveUrlValues {
        let resolved_base_url = self.resolved_base_url();
        let endpoint = self.endpoint();
        let models_endpoint = self.models_endpoint();

        ProviderEffectiveUrlValues {
            resolved_base_url,
            endpoint,
            models_endpoint,
        }
    }

    fn cross_routing_configuration_hint(
        &self,
        effective_urls: &ProviderEffectiveUrlValues,
    ) -> Option<String> {
        let current_profile = self.kind.url_validation_profile()?;
        let sources = effective_urls.sources();

        for (source_name, source_value, allow_host_only_base_match) in sources {
            let current_match = current_profile
                .matching_canonical_fingerprint(source_value, allow_host_only_base_match);
            let candidate_match = find_cross_routed_validation_profile(
                self.kind,
                source_value,
                allow_host_only_base_match,
            );
            let Some((candidate_profile, candidate_fingerprint)) = candidate_match else {
                continue;
            };
            if current_match.is_some() {
                continue;
            }

            let candidate_kind_id = candidate_profile.kind.as_str();
            let current_kind_id = self.kind.as_str();
            let route_expectation = current_profile.route_expectation;
            let hint = format!(
                "{current_kind_id} is pointing at the canonical `{candidate_kind_id}` {source_name} `{candidate_fingerprint}`; switch to `kind = \"{candidate_kind_id}\"` if that is intentional, or restore {route_expectation}"
            );
            return Some(hint);
        }

        None
    }

    fn path_validation_configuration_hint(
        &self,
        effective_urls: &ProviderEffectiveUrlValues,
    ) -> Option<String> {
        let validation_profile = self.kind.url_validation_profile()?;
        let sources = effective_urls.sources();
        let requires_path_validation = !validation_profile.required_path_fragments.is_empty();
        if requires_path_validation {
            for (_, value, _) in sources {
                let path_match = validation_profile.matches_required_path_fragment(value);
                if !path_match {
                    return Some(validation_profile.path_validation_hint.to_owned());
                }
            }
        }

        for (_, value, _) in sources {
            let path_match = validation_profile.matches_forbidden_path_fragment(value);
            if path_match {
                return Some(validation_profile.path_validation_hint.to_owned());
            }
        }

        None
    }

    fn opencode_configuration_hint(&self) -> Option<String> {
        let explicit_model = self.explicit_model();
        let configured_base_url = self.base_url.trim().to_ascii_lowercase();
        let resolved_base_url = self.resolved_base_url().to_ascii_lowercase();

        if self.kind == ProviderKind::OpencodeZen {
            if explicit_model.as_deref().is_some_and(|model| {
                model
                    .trim()
                    .to_ascii_lowercase()
                    .starts_with("opencode-go/")
            }) {
                return Some(
                    "opencode_zen expects Zen model ids; switch to `kind = \"opencode_go\"` for `opencode-go/*` models or remove the copied OpenCode prefix and keep the matching provider kind"
                        .to_owned(),
                );
            }
            if configured_base_url.contains("/zen/go/") || resolved_base_url.contains("/zen/go/") {
                return Some(
                    "opencode_zen should point at the Zen root (`https://opencode.ai/zen/v1`), not the Go path; switch to `kind = \"opencode_go\"` or reset `provider.base_url`"
                        .to_owned(),
                );
            }
        }

        if self.kind == ProviderKind::OpencodeGo {
            if explicit_model
                .as_deref()
                .is_some_and(|model| model.trim().to_ascii_lowercase().starts_with("opencode/"))
            {
                return Some(
                    "opencode_go expects Go model ids; switch to `kind = \"opencode_zen\"` for `opencode/*` models or remove the copied OpenCode prefix and keep the matching provider kind"
                        .to_owned(),
                );
            }
            let points_at_zen_root = configured_base_url.ends_with("/zen/v1")
                || (resolved_base_url.ends_with("/zen/v1")
                    && !resolved_base_url.contains("/zen/go/"));
            if points_at_zen_root {
                return Some(
                    "opencode_go should point at the Go root (`https://opencode.ai/zen/go/v1`), not the Zen root; switch to `kind = \"opencode_zen\"` or reset `provider.base_url`"
                        .to_owned(),
                );
            }
        }

        None
    }

    fn build_region_endpoint_support_facts(&self) -> ProviderRegionEndpointSupportFacts {
        let guide = self.kind.region_endpoint_guide();
        let note = guide.map(|value| value.note(self));
        let catalog_failure_hint = guide.map(|value| value.failure_hint(self));
        let request_failure_hint = guide.map(|value| value.request_failure_hint(self));

        ProviderRegionEndpointSupportFacts {
            note,
            catalog_failure_hint,
            request_failure_hint,
        }
    }

    fn build_provider_descriptor_region_endpoint(
        &self,
        region_endpoint_support: &ProviderRegionEndpointSupportFacts,
    ) -> ProviderDescriptorRegionEndpoint {
        let region_endpoint_info = self.kind.region_endpoint_info();
        let family_label = region_endpoint_info
            .as_ref()
            .map(|info| info.family_label.to_owned());
        let variants = provider_descriptor_region_variants(region_endpoint_info);
        let note = region_endpoint_support.note.clone();
        let catalog_failure_hint = region_endpoint_support.catalog_failure_hint.clone();
        let request_failure_hint = region_endpoint_support.request_failure_hint.clone();

        ProviderDescriptorRegionEndpoint {
            family_label,
            variants,
            note,
            catalog_failure_hint,
            request_failure_hint,
        }
    }

    pub fn region_endpoint_note(&self) -> Option<String> {
        let support_facts = self.support_facts();
        support_facts.region_endpoint.note
    }

    pub fn region_endpoint_failure_hint(&self) -> Option<String> {
        let support_facts = self.support_facts();
        support_facts.region_endpoint.catalog_failure_hint
    }

    pub fn request_region_endpoint_failure_hint(&self) -> Option<String> {
        let support_facts = self.support_facts();
        support_facts.region_endpoint.request_failure_hint
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
            && !has_configured_secret_ref(self.api_key.as_ref())
            && !has_configured_secret_ref(self.oauth_access_token.as_ref())
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

    fn missing_auth_source_runtime_detail(
        &self,
        label: &str,
        secret_ref: Option<&SecretRef>,
        configured_env_name: Option<&str>,
    ) -> Option<String> {
        match resolve_secret_lookup(secret_ref) {
            SecretLookup::Value(_) => return None,
            SecretLookup::Missing => {
                if let Some(env_name) = secret_ref_env_name(secret_ref) {
                    return Some(format!(
                        "configured provider {label} env `{env_name}` is unset, empty, or not visible to the current process"
                    ));
                }
                if has_configured_secret_ref(secret_ref) {
                    return Some(format!(
                        "configured provider {label} secret reference could not be resolved at runtime"
                    ));
                }
            }
            SecretLookup::Absent => {}
        }

        let env_name = configured_env_name?;
        match env::var(env_name) {
            Ok(value) if !value.trim().is_empty() => None,
            _ => Some(format!(
                "configured provider {label} env `{env_name}` is unset, empty, or not visible to the current process"
            )),
        }
    }

    pub fn configured_api_key_env_override(&self) -> Option<String> {
        let explicit_secret_env = secret_ref_env_name(self.api_key.as_ref());
        if let Some(explicit_secret_env) = explicit_secret_env {
            return Some(explicit_secret_env);
        }
        self.configured_api_key_env_name().map(str::to_owned)
    }

    pub fn configured_oauth_access_token_env_override(&self) -> Option<String> {
        let explicit_secret_env = secret_ref_env_name(self.oauth_access_token.as_ref());
        if let Some(explicit_secret_env) = explicit_secret_env {
            return Some(explicit_secret_env);
        }
        self.configured_oauth_access_token_env_name()
            .map(str::to_owned)
    }

    fn missing_auth_runtime_detail(&self) -> Option<String> {
        match self.kind.auth_scheme() {
            ProviderAuthScheme::Bearer => self
                .missing_auth_source_runtime_detail(
                    "oauth access token",
                    self.oauth_access_token.as_ref(),
                    self.configured_oauth_access_token_env_name(),
                )
                .or_else(|| {
                    self.missing_auth_source_runtime_detail(
                        "api key",
                        self.api_key.as_ref(),
                        self.configured_api_key_env_name(),
                    )
                }),
            ProviderAuthScheme::XApiKey | ProviderAuthScheme::XGoogApiKey => self
                .missing_auth_source_runtime_detail(
                    "api key",
                    self.api_key.as_ref(),
                    self.configured_api_key_env_name(),
                ),
        }
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
        let api_key_has_explicit_env_reference =
            self.api_key_env_explicit || secret_ref_env_name(self.api_key.as_ref()).is_some();
        let oauth_has_explicit_env_reference = self.oauth_access_token_env_explicit
            || secret_ref_env_name(self.oauth_access_token.as_ref()).is_some();

        let mut normalized = self.clone();
        normalized.base_url = base_url;
        normalized.chat_completions_path = chat_completions_path;
        normalized.endpoint = self.normalized_endpoint_for_persistence();
        normalized.models_endpoint = self.normalized_models_endpoint_for_persistence();
        normalized.api_key = self.normalized_api_key_for_persistence();
        normalized.api_key_env = self.normalized_api_key_env_for_persistence();
        normalized.oauth_access_token = self.normalized_oauth_access_token_for_persistence();
        normalized.oauth_access_token_env =
            self.normalized_oauth_access_token_env_for_persistence();
        if api_key_has_explicit_env_reference {
            canonicalize_secret_env_reference_for_persistence(
                &mut normalized.api_key,
                &mut normalized.api_key_env,
            );
        } else {
            normalized.api_key_env = None;
        }
        if oauth_has_explicit_env_reference {
            canonicalize_secret_env_reference_for_persistence(
                &mut normalized.oauth_access_token,
                &mut normalized.oauth_access_token_env,
            );
        } else {
            normalized.oauth_access_token_env = None;
        }
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

    fn normalized_api_key_for_persistence(&self) -> Option<SecretRef> {
        normalize_secret_ref_for_persistence(self.api_key.as_ref(), self.api_key_env.as_deref())
    }

    fn normalized_api_key_env_for_persistence(&self) -> Option<String> {
        let explicit_secret_env = secret_ref_env_name(self.api_key.as_ref());
        if let Some(explicit_secret_env) = explicit_secret_env {
            return Some(explicit_secret_env);
        }
        let configured = non_empty(self.api_key_env.as_deref()).map(str::to_owned);
        if self.api_key_env_explicit {
            return configured;
        }
        if let Some(configured_override) = self.configured_api_key_env_name().map(str::to_owned) {
            return Some(configured_override);
        }
        self.kind.default_api_key_env().map(str::to_owned)
    }

    fn normalized_oauth_access_token_for_persistence(&self) -> Option<SecretRef> {
        normalize_secret_ref_for_persistence(
            self.oauth_access_token.as_ref(),
            self.oauth_access_token_env.as_deref(),
        )
    }

    fn normalized_oauth_access_token_env_for_persistence(&self) -> Option<String> {
        let explicit_secret_env = secret_ref_env_name(self.oauth_access_token.as_ref());
        if let Some(explicit_secret_env) = explicit_secret_env {
            return Some(explicit_secret_env);
        }
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

fn is_current_provider_base_url(kind: ProviderKind, base_url: &str) -> bool {
    is_same_base_url(base_url, kind.profile().base_url)
}

fn is_current_provider_chat_completions_path(kind: ProviderKind, path: &str) -> bool {
    is_same_chat_path(path, kind.profile().chat_completions_path)
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

impl ProviderEffectiveUrlValues {
    fn sources(&self) -> [(&'static str, &str, bool); 3] {
        [
            ("endpoint", self.endpoint.as_str(), false),
            ("models endpoint", self.models_endpoint.as_str(), false),
            ("base url", self.resolved_base_url.as_str(), true),
        ]
    }
}

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

fn push_secret_ref_env_name(keys: &mut Vec<String>, maybe_secret: Option<&SecretRef>) {
    let env_name = secret_ref_env_name(maybe_secret);
    let Some(env_name) = env_name else {
        return;
    };
    push_unique_env_key(keys, Some(env_name.as_str()));
}

fn join_guidance_options(options: &[String]) -> String {
    let Some((last, rest)) = options.split_last() else {
        return String::new();
    };

    if let [first] = rest {
        return format!("{first} or {last}");
    }
    if rest.is_empty() {
        return last.clone();
    }

    let mut joined = rest.join(", ");
    joined.push_str(", or ");
    joined.push_str(last);
    joined
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

fn normalize_secret_ref_for_persistence(
    secret_ref: Option<&SecretRef>,
    env_name: Option<&str>,
) -> Option<SecretRef> {
    let secret_ref = secret_ref.filter(|value| value.is_configured());
    let explicit_secret_env = secret_ref_env_name(secret_ref);
    let Some(explicit_secret_env) = explicit_secret_env.as_deref() else {
        return secret_ref.cloned();
    };

    let configured_env_name = non_empty(env_name);
    match configured_env_name {
        None => None,
        Some(configured_env_name) if configured_env_name == explicit_secret_env => None,
        Some(_) => secret_ref.cloned(),
    }
}

fn canonicalize_secret_env_reference_for_persistence(
    secret_ref: &mut Option<SecretRef>,
    env_name: &mut Option<String>,
) {
    if let Some(explicit_env_name) = secret_ref_env_name(secret_ref.as_ref()) {
        *secret_ref = Some(SecretRef::Env {
            env: explicit_env_name,
        });
        *env_name = None;
        return;
    }

    if secret_ref.as_ref().is_some_and(SecretRef::is_configured) {
        *env_name = None;
        return;
    }

    let normalized_env_name = env_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if let Some(normalized_env_name) = normalized_env_name {
        *secret_ref = Some(SecretRef::Env {
            env: normalized_env_name,
        });
    }
    *env_name = None;
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
mod tests;
