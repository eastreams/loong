use std::{collections::BTreeMap, env, path::PathBuf};

use loong_contracts::SecretRef;
use serde::{Deserialize, Deserializer, Serialize};

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderCatalogEntry {
    pub kind: String,
    pub display_name: String,
    pub default_base_url: String,
    pub default_chat_path: String,
    pub default_models_path: Option<String>,
    pub default_model: Option<String>,
    pub recommended_onboarding_model: Option<String>,
    pub auth_scheme: String,
    pub protocol_family: String,
    pub feature_family: String,
    pub is_coding_variant: bool,
    pub aliases: Vec<String>,
    pub configuration_hint: Option<String>,
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

    pub fn kind_route_mismatch_hint(&self) -> Option<String> {
        let effective_urls = self.effective_url_values();
        self.path_validation_configuration_hint(&effective_urls)
    }

    pub fn configuration_hint(&self) -> Option<String> {
        if let Some(hint) = self.kind_route_mismatch_hint() {
            return Some(hint);
        }

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

fn default_models_path_for_kind(kind: ProviderKind) -> Option<String> {
    let profile = kind.profile();
    if let Some(path) = profile.models_path {
        return Some(path.to_owned());
    }

    let provider = ProviderConfig::fresh_for_kind(kind);
    let base_url = provider.resolved_base_url();
    let models_endpoint = provider.models_endpoint();
    models_endpoint
        .strip_prefix(base_url.as_str())
        .map(str::to_owned)
}

pub fn provider_catalog_entries() -> Vec<ProviderCatalogEntry> {
    ProviderKind::all_sorted()
        .iter()
        .copied()
        .map(ProviderKind::catalog_entry)
        .collect()
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

impl ProviderUrlValidationProfile {
    fn matching_canonical_fingerprint(
        self,
        value: &str,
        allow_host_only_base_match: bool,
    ) -> Option<&'static str> {
        let profile = self.kind.profile();
        let base_fingerprint = profile.base_url;
        let base_match = matches_base_url_validation_fingerprint(
            value,
            base_fingerprint,
            allow_host_only_base_match,
        );
        if base_match {
            return Some(base_fingerprint);
        }

        for fingerprint in self.extra_canonical_url_fingerprints {
            let fingerprint_match = matches_url_validation_fingerprint(value, fingerprint);
            if fingerprint_match {
                return Some(fingerprint);
            }
        }

        None
    }

    fn matches_required_path_fragment(self, value: &str) -> bool {
        for fragment in self.required_path_fragments {
            let fragment_match = url_contains_validation_fragment(value, fragment);
            if fragment_match {
                return true;
            }
        }

        false
    }

    fn matches_forbidden_path_fragment(self, value: &str) -> bool {
        for fragment in self.forbidden_path_exceptions {
            let exception_match = url_contains_validation_fragment(value, fragment);
            if exception_match {
                return false;
            }
        }

        for fragment in self.forbidden_path_fragments {
            let fragment_match = url_contains_validation_fragment(value, fragment);
            if fragment_match {
                return true;
            }
        }

        false
    }
}

impl ProviderKind {
    pub fn all_sorted() -> &'static [ProviderKind] {
        &PROVIDER_KIND_ORDER
    }

    fn url_validation_profile(self) -> Option<&'static ProviderUrlValidationProfile> {
        PROVIDER_URL_VALIDATION_PROFILES
            .iter()
            .find(|profile| profile.kind == self)
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
            ProviderKind::GithubCopilot => "GitHub Copilot",
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
            ProviderKind::OpencodeZen => "OpenCode Zen",
            ProviderKind::OpencodeGo => "OpenCode Go",
            ProviderKind::Openrouter => "OpenRouter",
            ProviderKind::Perplexity => "Perplexity",
            ProviderKind::Qianfan => "Qianfan",
            ProviderKind::Qwen => "Qwen",
            ProviderKind::BailianCoding => "Bailian Coding",
            ProviderKind::Sambanova => "SambaNova",
            ProviderKind::Sglang => "SGLang",
            ProviderKind::Siliconflow => "SiliconFlow",
            ProviderKind::Stepfun => "StepFun",
            ProviderKind::StepPlan => "Step Plan",
            ProviderKind::Together => "Together",
            ProviderKind::Venice => "Venice",
            ProviderKind::VercelAiGateway => "Vercel AI Gateway",
            ProviderKind::Vllm => "vLLM",
            ProviderKind::Volcengine => "Volcengine",
            ProviderKind::VolcengineCoding => "Volcengine Coding",
            ProviderKind::Xai => "xAI",
            ProviderKind::Xiaomi => "Xiaomi",
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
            bailian_coding,
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
            github_copilot,
            groq,
            kimi,
            kimi_coding,
            llamacpp,
            lm_studio,
            mistral,
            minimax,
            novita,
            nvidia,
            ollama,
            openai,
            opencode_go,
            opencode_zen,
            openrouter,
            perplexity,
            qianfan,
            qwen,
            sambanova,
            sglang,
            siliconflow,
            stepfun,
            step_plan,
            together,
            venice,
            vercel_ai_gateway,
            vllm,
            volcengine,
            volcengine_coding,
            xai,
            xiaomi,
            zai,
            zhipu,
        ] = &PROVIDER_PROFILES;

        match self {
            ProviderKind::Anthropic => anthropic,
            ProviderKind::BailianCoding => bailian_coding,
            ProviderKind::Bedrock => bedrock,
            ProviderKind::Byteplus => byteplus,
            ProviderKind::ByteplusCoding => byteplus_coding,
            ProviderKind::Cerebras => cerebras,
            ProviderKind::CloudflareAiGateway => cloudflare_ai_gateway,
            ProviderKind::Cohere => cohere,
            ProviderKind::Custom => custom,
            ProviderKind::Deepseek => deepseek,
            ProviderKind::Fireworks => fireworks,
            ProviderKind::Gemini => gemini,
            ProviderKind::GithubCopilot => github_copilot,
            ProviderKind::Groq => groq,
            ProviderKind::Kimi => kimi,
            ProviderKind::KimiCoding => kimi_coding,
            ProviderKind::Llamacpp => llamacpp,
            ProviderKind::LmStudio => lm_studio,
            ProviderKind::Mistral => mistral,
            ProviderKind::Minimax => minimax,
            ProviderKind::Novita => novita,
            ProviderKind::Nvidia => nvidia,
            ProviderKind::Ollama => ollama,
            ProviderKind::Openai => openai,
            ProviderKind::OpencodeZen => opencode_zen,
            ProviderKind::OpencodeGo => opencode_go,
            ProviderKind::Openrouter => openrouter,
            ProviderKind::Perplexity => perplexity,
            ProviderKind::Qianfan => qianfan,
            ProviderKind::Qwen => qwen,
            ProviderKind::Sambanova => sambanova,
            ProviderKind::Sglang => sglang,
            ProviderKind::Siliconflow => siliconflow,
            ProviderKind::Stepfun => stepfun,
            ProviderKind::StepPlan => step_plan,
            ProviderKind::Together => together,
            ProviderKind::Venice => venice,
            ProviderKind::VercelAiGateway => vercel_ai_gateway,
            ProviderKind::Vllm => vllm,
            ProviderKind::Volcengine => volcengine,
            ProviderKind::VolcengineCoding => volcengine_coding,
            ProviderKind::Xai => xai,
            ProviderKind::Xiaomi => xiaomi,
            ProviderKind::Zai => zai,
            ProviderKind::Zhipu => zhipu,
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

    pub const fn is_coding_variant(self) -> bool {
        matches!(
            self,
            ProviderKind::BailianCoding
                | ProviderKind::ByteplusCoding
                | ProviderKind::KimiCoding
                | ProviderKind::VolcengineCoding
        )
    }

    pub fn catalog_entry(self) -> ProviderCatalogEntry {
        let profile = self.profile();
        ProviderCatalogEntry {
            kind: self.as_str().to_owned(),
            display_name: self.display_name().to_owned(),
            default_base_url: profile.base_url.to_owned(),
            default_chat_path: profile.chat_completions_path.to_owned(),
            default_models_path: default_models_path_for_kind(self),
            default_model: self.default_model().map(str::to_owned),
            recommended_onboarding_model: self.recommended_onboarding_model().map(str::to_owned),
            auth_scheme: self.auth_scheme().as_str().to_owned(),
            protocol_family: self.protocol_family().as_str().to_owned(),
            feature_family: self.feature_family().as_str().to_owned(),
            is_coding_variant: self.is_coding_variant(),
            aliases: profile
                .aliases
                .iter()
                .map(|alias| (*alias).to_owned())
                .collect(),
            configuration_hint: self.configuration_hint().map(str::to_owned),
        }
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
                "set `BEDROCK_AWS_REGION`/`AWS_REGION`/`AWS_DEFAULT_REGION` or replace `<region>` in `provider.base_url` with your Bedrock runtime region",
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
            ProviderKind::Stepfun | ProviderKind::StepPlan => Some(ProviderRegionEndpointGuide {
                family_label: "Stepfun",
                default_variant: ProviderRegionEndpointVariant {
                    label: "CN",
                    base_url: profile.base_url,
                },
                alternate_variant: ProviderRegionEndpointVariant {
                    label: "Global",
                    base_url: "https://api.stepfun.ai",
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
            | ProviderKind::GithubCopilot
            | ProviderKind::KimiCoding
            | ProviderKind::Llamacpp
            | ProviderKind::LmStudio
            | ProviderKind::Mistral
            | ProviderKind::Novita
            | ProviderKind::Nvidia
            | ProviderKind::Ollama
            | ProviderKind::Openai
            | ProviderKind::OpencodeZen
            | ProviderKind::OpencodeGo
            | ProviderKind::Openrouter
            | ProviderKind::Perplexity
            | ProviderKind::Qianfan
            | ProviderKind::Qwen
            | ProviderKind::BailianCoding
            | ProviderKind::Sambanova
            | ProviderKind::Sglang
            | ProviderKind::Siliconflow
            | ProviderKind::Together
            | ProviderKind::Venice
            | ProviderKind::VercelAiGateway
            | ProviderKind::Vllm
            | ProviderKind::Volcengine
            | ProviderKind::VolcengineCoding
            | ProviderKind::Xai
            | ProviderKind::Xiaomi => None,
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
            Some("MiniMax-M2.7")
        } else if matches!(self, ProviderKind::VolcengineCoding) {
            Some("ark-code-latest")
        } else if matches!(self, ProviderKind::Xiaomi) {
            Some("mimo-v2-pro")
        } else {
            None
        }
    }

    pub fn region_endpoint_info(self) -> Option<ProviderRegionEndpointInfo> {
        let guide = self.region_endpoint_guide()?;
        let family_label = if matches!(self, ProviderKind::Zai | ProviderKind::Zhipu) {
            "Z.ai"
        } else {
            guide.family_label
        };
        let variants = vec![
            RegionVariant {
                label: guide.default_variant.label,
                base_url: guide.default_variant.base_url,
            },
            RegionVariant {
                label: guide.alternate_variant.label,
                base_url: guide.alternate_variant.base_url,
            },
        ];
        Some(ProviderRegionEndpointInfo {
            family_label,
            variants,
        })
    }
}

const NON_CODING_ARK_FORBIDDEN_PATH_FRAGMENTS: [&str; 1] = ["/api/coding"];
const CODING_ARK_REQUIRED_PATH_FRAGMENTS: [&str; 1] = ["/api/coding/v3"];
const CODING_ARK_FORBIDDEN_PATH_FRAGMENTS: [&str; 2] = ["/api/v3", "/api/coding"];
const CODING_ARK_FORBIDDEN_PATH_EXCEPTIONS: [&str; 1] = ["/api/coding/v3"];
const VOLCENGINE_STANDARD_CANONICAL_URL_FINGERPRINTS: [&str; 1] =
    ["https://ark.cn-beijing.volces.com/api/v3"];

const PROVIDER_URL_VALIDATION_PROFILES: [ProviderUrlValidationProfile; 4] = [
    ProviderUrlValidationProfile {
        kind: ProviderKind::Byteplus,
        extra_canonical_url_fingerprints: &[],
        required_path_fragments: &[],
        forbidden_path_fragments: &NON_CODING_ARK_FORBIDDEN_PATH_FRAGMENTS,
        forbidden_path_exceptions: &[],
        route_expectation: "the standard BytePlus ModelArk route under `/api/v3`",
        path_validation_hint: "byteplus uses the standard ModelArk path and should not target `/api/coding` or `/api/coding/v3`; switch to `kind = \"byteplus_coding\"` for the dedicated OpenAI-compatible Coding Plan endpoint",
    },
    ProviderUrlValidationProfile {
        kind: ProviderKind::ByteplusCoding,
        extra_canonical_url_fingerprints: &[],
        required_path_fragments: &CODING_ARK_REQUIRED_PATH_FRAGMENTS,
        forbidden_path_fragments: &CODING_ARK_FORBIDDEN_PATH_FRAGMENTS,
        forbidden_path_exceptions: &CODING_ARK_FORBIDDEN_PATH_EXCEPTIONS,
        route_expectation: "the dedicated BytePlus Coding path under `/api/coding/v3`",
        path_validation_hint: "byteplus_coding must use the dedicated BytePlus Coding path under `/api/coding/v3`; do not point it at the unsupported Anthropic-compatible `/api/coding` or generic `/api/v3` ModelArk endpoints because that bypasses Coding Plan quota and can incur standard model charges",
    },
    ProviderUrlValidationProfile {
        kind: ProviderKind::Volcengine,
        extra_canonical_url_fingerprints: &VOLCENGINE_STANDARD_CANONICAL_URL_FINGERPRINTS,
        required_path_fragments: &[],
        forbidden_path_fragments: &NON_CODING_ARK_FORBIDDEN_PATH_FRAGMENTS,
        forbidden_path_exceptions: &[],
        route_expectation: "the standard Volcengine Ark route under `/api/v3`",
        path_validation_hint: "volcengine uses the standard Ark API path under `/api/v3` and should not target `/api/coding` or `/api/coding/v3`; switch to `kind = \"volcengine_coding\"` for the dedicated OpenAI-compatible Coding Plan endpoint",
    },
    ProviderUrlValidationProfile {
        kind: ProviderKind::VolcengineCoding,
        extra_canonical_url_fingerprints: &[],
        required_path_fragments: &CODING_ARK_REQUIRED_PATH_FRAGMENTS,
        forbidden_path_fragments: &CODING_ARK_FORBIDDEN_PATH_FRAGMENTS,
        forbidden_path_exceptions: &CODING_ARK_FORBIDDEN_PATH_EXCEPTIONS,
        route_expectation: "the dedicated Volcengine Coding Plan path under `/api/coding/v3`",
        path_validation_hint: "volcengine_coding must use the dedicated Volcengine Coding Plan path under `/api/coding/v3`; do not point it at the Anthropic-compatible `/api/coding` or generic `/api/v3` Ark endpoints because that bypasses Coding Plan quota and can incur standard charges",
    },
];

fn find_cross_routed_validation_profile(
    current_kind: ProviderKind,
    source_value: &str,
    allow_host_only_base_match: bool,
) -> Option<(&'static ProviderUrlValidationProfile, &'static str)> {
    let mut best_match: Option<(&'static ProviderUrlValidationProfile, &'static str)> = None;

    for profile in &PROVIDER_URL_VALIDATION_PROFILES {
        let candidate_kind = profile.kind;
        if candidate_kind == current_kind {
            continue;
        }

        let matching_fingerprint =
            profile.matching_canonical_fingerprint(source_value, allow_host_only_base_match);
        if let Some(fingerprint) = matching_fingerprint {
            let mut should_replace = true;
            if let Some((_, best_fingerprint)) = best_match {
                should_replace = fingerprint.len() > best_fingerprint.len();
            }
            if should_replace {
                best_match = Some((profile, fingerprint));
            }
        }
    }

    best_match
}

fn matches_url_validation_fingerprint(value: &str, fingerprint: &str) -> bool {
    let normalized_value = normalize_url_validation_value(value);
    let normalized_fingerprint = normalize_url_validation_value(fingerprint);

    matches_region_endpoint_url(normalized_value.as_str(), normalized_fingerprint.as_str())
}

fn matches_base_url_validation_fingerprint(
    value: &str,
    fingerprint: &str,
    allow_host_only_base_match: bool,
) -> bool {
    let fingerprint_has_non_root_path = url_has_non_root_path(fingerprint);
    if fingerprint_has_non_root_path {
        return matches_url_validation_fingerprint(value, fingerprint);
    }
    if !allow_host_only_base_match {
        return false;
    }
    is_same_base_url(value, fingerprint)
}

fn url_contains_validation_fragment(value: &str, fragment: &str) -> bool {
    let normalized_value = normalize_url_validation_value(value);
    let normalized_fragment = normalize_url_validation_value(fragment);

    normalized_value.contains(normalized_fragment.as_str())
}

fn url_has_non_root_path(value: &str) -> bool {
    let trimmed = value.trim();
    let after_scheme = trimmed
        .split_once("://")
        .map(|(_, remainder)| remainder)
        .unwrap_or(trimmed);
    let Some((_, path_with_query)) = after_scheme.split_once('/') else {
        return false;
    };
    let raw_path = format!("/{path_with_query}");
    let path = raw_path
        .split(['?', '#'])
        .next()
        .unwrap_or(raw_path.as_str());
    let normalized_path = path.trim_end_matches('/');

    !normalized_path.is_empty()
}

fn normalize_url_validation_value(value: &str) -> String {
    let trimmed = value.trim();
    let trimmed = trimmed.trim_end_matches('/');

    trimmed.to_ascii_lowercase()
}

fn provider_descriptor_aliases(profile: &ProviderProfile) -> Vec<String> {
    let mut aliases = Vec::new();

    for alias in profile.aliases {
        let alias = (*alias).to_owned();
        aliases.push(alias);
    }

    aliases
}

fn provider_descriptor_headers(profile: &ProviderProfile) -> Vec<ProviderDescriptorHeader> {
    let mut headers = Vec::new();

    for (name, value) in profile.default_headers {
        let header = ProviderDescriptorHeader {
            name: (*name).to_owned(),
            value: (*value).to_owned(),
        };
        headers.push(header);
    }

    headers
}

fn provider_descriptor_env_aliases(raw_aliases: &[&str]) -> Vec<String> {
    let mut aliases = Vec::new();

    for raw_alias in raw_aliases {
        let alias = (*raw_alias).to_owned();
        aliases.push(alias);
    }

    aliases
}

fn build_provider_descriptor_feature(
    feature_support: &ProviderFeatureSupportFacts,
) -> ProviderDescriptorFeature {
    let family = feature_support.family.as_str().to_owned();
    let gate_name = feature_support.gate_name.to_owned();
    let enabled_in_build = feature_support.enabled_in_build;
    let disabled_message = feature_support.disabled_message.clone();

    ProviderDescriptorFeature {
        family,
        gate_name,
        enabled_in_build,
        disabled_message,
    }
}

fn provider_descriptor_region_variants(
    region_endpoint_info: Option<ProviderRegionEndpointInfo>,
) -> Vec<ProviderDescriptorRegionVariant> {
    let mut variants = Vec::new();

    let Some(region_endpoint_info) = region_endpoint_info else {
        return variants;
    };

    for variant in region_endpoint_info.variants {
        let descriptor_variant = ProviderDescriptorRegionVariant {
            label: variant.label.to_owned(),
            base_url: variant.base_url.to_owned(),
        };
        variants.push(descriptor_variant);
    }

    variants
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

const PROVIDER_KIND_ORDER: [ProviderKind; 45] = [
    ProviderKind::Anthropic,
    ProviderKind::BailianCoding,
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
    ProviderKind::GithubCopilot,
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
    ProviderKind::OpencodeGo,
    ProviderKind::OpencodeZen,
    ProviderKind::Openrouter,
    ProviderKind::Perplexity,
    ProviderKind::Qianfan,
    ProviderKind::Qwen,
    ProviderKind::Sambanova,
    ProviderKind::Sglang,
    ProviderKind::Siliconflow,
    ProviderKind::Stepfun,
    ProviderKind::StepPlan,
    ProviderKind::Together,
    ProviderKind::Venice,
    ProviderKind::VercelAiGateway,
    ProviderKind::Vllm,
    ProviderKind::Volcengine,
    ProviderKind::VolcengineCoding,
    ProviderKind::Xai,
    ProviderKind::Xiaomi,
    ProviderKind::Zai,
    ProviderKind::Zhipu,
];

const PROVIDER_PROFILES: [ProviderProfile; 45] = [
    ProviderProfile {
        kind: ProviderKind::Anthropic,
        id: "anthropic",
        aliases: &["anthropic_compatible"],
        base_url: "https://api.anthropic.com",
        chat_completions_path: "/v1/messages",
        models_path: Some("/v1/models"),
        protocol_family: ProviderProtocolFamily::AnthropicMessages,
        auth_scheme: ProviderAuthScheme::XApiKey,
        default_headers: &ANTHROPIC_DEFAULT_HEADERS,
        default_api_key_env: Some("ANTHROPIC_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::Anthropic,
    },
    ProviderProfile {
        kind: ProviderKind::BailianCoding,
        id: "bailian_coding",
        aliases: &["bailian_coding_compatible"],
        base_url: "https://coding.dashscope.aliyuncs.com/v1",
        chat_completions_path: "/chat/completions",
        models_path: Some("/models"),
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("BAILIAN_API_KEY"),
        api_key_env_aliases: &[],
        default_user_agent: Some("openclaw"),
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
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
        kind: ProviderKind::GithubCopilot,
        id: "github-copilot",
        aliases: &["github_copilot", "copilot"],
        base_url: "https://api.githubcopilot.com",
        chat_completions_path: "/chat/completions",
        models_path: None,
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &GITHUB_COPILOT_DEFAULT_HEADERS,
        default_api_key_env: None,
        api_key_env_aliases: &[],
        default_user_agent: Some(GITHUB_COPILOT_USER_AGENT),
        default_oauth_access_token_env: Some(GITHUB_COPILOT_OAUTH_TOKEN_ENV),
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
        default_user_agent: Some("KimiCLI/Loong"),
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
        kind: ProviderKind::OpencodeGo,
        id: "opencode_go",
        aliases: &["opencode-go"],
        base_url: OPENCODE_GO_BASE_URL,
        chat_completions_path: "/chat/completions",
        models_path: Some("/models"),
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some(OPENCODE_API_KEY_ENV),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
    },
    ProviderProfile {
        kind: ProviderKind::OpencodeZen,
        id: "opencode_zen",
        aliases: &["opencode", "opencode-zen"],
        base_url: OPENCODE_ZEN_BASE_URL,
        chat_completions_path: "/chat/completions",
        models_path: Some("/models"),
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some(OPENCODE_API_KEY_ENV),
        api_key_env_aliases: &[],
        default_user_agent: None,
        default_oauth_access_token_env: None,
        oauth_access_token_env_aliases: &[],
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
        kind: ProviderKind::StepPlan,
        id: "step_plan",
        aliases: &["stepfun_step_plan"],
        base_url: "https://api.stepfun.com",
        chat_completions_path: "/step_plan/v1/chat/completions",
        models_path: Some("/step_plan/v1/models"),
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
        kind: ProviderKind::Xiaomi,
        id: "xiaomi",
        aliases: &[
            "xiaomi_compatible",
            "xiaomi_mimo",
            "xiaomi-mimo",
            "mimo",
            "mimo_compatible",
        ],
        base_url: "https://api.xiaomimimo.com",
        chat_completions_path: "/v1/chat/completions",
        models_path: Some("/v1/models"),
        protocol_family: ProviderProtocolFamily::OpenAiChatCompletions,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_api_key_env: Some("XIAOMI_API_KEY"),
        api_key_env_aliases: &["MIMO_API_KEY", "XIAOMIMIMO_API_KEY"],
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
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::collections::{BTreeMap, BTreeSet};

    use crate::test_support::ScopedEnv;
    use loong_contracts::SecretRef;

    fn encode_provider_descriptor(descriptor: &ProviderDescriptorDocument) -> Value {
        serde_json::to_value(descriptor).expect("serialize provider descriptor document")
    }

    #[test]
    fn provider_profile_lookup_matches_kind() {
        for kind in ProviderKind::all_sorted() {
            assert_eq!(kind.profile().kind, *kind);
        }
    }

    #[test]
    fn provider_profile_aliases_are_unique_and_do_not_shadow_ids() {
        let mut provider_ids = BTreeSet::new();
        for kind in ProviderKind::all_sorted() {
            let profile = kind.profile();
            let provider_id = profile.id;
            let inserted = provider_ids.insert(provider_id);
            assert!(inserted, "duplicate provider id detected: {provider_id}");
        }

        let mut alias_owners = BTreeMap::new();
        for kind in ProviderKind::all_sorted() {
            let profile = kind.profile();
            let provider_id = profile.id;
            for alias in profile.aliases {
                let normalized_alias = alias.trim();
                assert!(
                    !normalized_alias.is_empty(),
                    "provider `{provider_id}` contains an empty alias"
                );
                assert_ne!(
                    normalized_alias, provider_id,
                    "provider `{provider_id}` repeats its canonical id as an alias"
                );
                assert!(
                    !provider_ids.contains(normalized_alias),
                    "provider alias `{normalized_alias}` collides with a canonical provider id"
                );

                let previous_owner = alias_owners.insert(normalized_alias.to_owned(), provider_id);
                assert!(
                    previous_owner.is_none(),
                    "provider alias `{normalized_alias}` is shared by `{}` and `{provider_id}`",
                    previous_owner.unwrap_or_default()
                );
            }
        }
    }

    #[test]
    fn provider_profile_aliases_round_trip_through_provider_kind_deserialization() {
        for kind in ProviderKind::all_sorted() {
            let profile = kind.profile();
            let canonical_id = format!("\"{}\"", profile.id);
            let parsed_canonical = serde_json::from_str::<ProviderKind>(canonical_id.as_str())
                .expect("canonical provider id should deserialize");
            assert_eq!(
                parsed_canonical, *kind,
                "canonical provider id should deserialize to its matching provider kind"
            );

            for alias in profile.aliases {
                let raw_alias = format!("\"{alias}\"");
                let parsed_alias = serde_json::from_str::<ProviderKind>(raw_alias.as_str())
                    .expect("provider alias should deserialize");
                assert_eq!(
                    parsed_alias, *kind,
                    "provider alias `{alias}` should deserialize to the same provider kind as parse_provider_kind_id"
                );
            }
        }
    }

    #[test]
    fn provider_kind_serializes_github_copilot_using_canonical_hyphenated_id() {
        let raw = serde_json::to_string(&ProviderKind::GithubCopilot)
            .expect("provider kind should serialize");

        assert_eq!(raw, "\"github-copilot\"");
    }

    #[test]
    fn provider_feature_family_gate_messages_are_stable() {
        let anthropic_message = ProviderFeatureFamily::Anthropic.disabled_message();
        let bedrock_message = ProviderFeatureFamily::Bedrock.disabled_message();
        let volcengine_message = ProviderFeatureFamily::Volcengine.disabled_message();
        let openai_message = ProviderFeatureFamily::OpenAiCompatible.disabled_message();

        assert_eq!(
            anthropic_message,
            "anthropic provider family is disabled (enable feature `provider-anthropic`)"
        );
        assert_eq!(
            bedrock_message,
            "bedrock provider family is disabled (enable feature `provider-bedrock`)"
        );
        assert_eq!(
            volcengine_message,
            "volcengine provider family is disabled (enable feature `provider-volcengine`)"
        );
        assert_eq!(
            openai_message,
            "openai-compatible provider family is disabled (enable feature `provider-openai`)"
        );
    }

    #[test]
    fn provider_profile_static_auth_hints_are_stable() {
        let byteplus_hint = ProviderKind::ByteplusCoding
            .profile()
            .auth_guidance_hint()
            .expect("byteplus coding should expose auth guidance");
        let volcengine_hint = ProviderKind::Volcengine
            .profile()
            .auth_guidance_hint()
            .expect("volcengine should expose auth guidance");
        let bedrock_hint = ProviderKind::Bedrock
            .profile()
            .alternative_auth_configuration_hint()
            .expect("bedrock should expose a SigV4 fallback hint");
        let custom_hint = ProviderKind::Custom
            .profile()
            .alternative_auth_configuration_hint()
            .expect("custom provider should expose header guidance");

        assert!(byteplus_hint.contains("BytePlus"));
        assert!(byteplus_hint.contains("BYTEPLUS_API_KEY"));
        assert!(byteplus_hint.contains("Authorization: Bearer <BYTEPLUS_API_KEY>"));

        assert!(volcengine_hint.contains("Volcengine"));
        assert!(volcengine_hint.contains("ARK_API_KEY"));
        assert!(volcengine_hint.contains("AK/SK request signing is not used"));

        assert!(bedrock_hint.contains("AWS_ACCESS_KEY_ID"));
        assert!(bedrock_hint.contains("AWS_SECRET_ACCESS_KEY"));
        assert!(bedrock_hint.contains("BEDROCK_AWS_REGION"));
        assert!(bedrock_hint.contains("AWS_REGION"));

        assert!(custom_hint.contains("Authorization"));
        assert!(custom_hint.contains("X-API-Key"));
        assert!(custom_hint.contains("provider.headers"));
    }

    #[test]
    fn provider_configuration_hint_prefers_canonical_cross_routing_guidance() {
        let cases = [
            (
                ProviderKind::Byteplus,
                "https://ark.cn-beijing.volces.com/api/coding/v3",
                "kind = \"volcengine_coding\"",
            ),
            (
                ProviderKind::ByteplusCoding,
                "https://ark.cn-beijing.volces.com",
                "kind = \"volcengine\"",
            ),
        ];

        for (kind, base_url, expected_kind_hint) in cases {
            let provider = ProviderConfig {
                kind,
                base_url: base_url.to_owned(),
                base_url_explicit: true,
                ..ProviderConfig::default()
            };
            let hint = provider
                .configuration_hint()
                .expect("cross-routed provider configs should surface a hint");

            assert!(
                hint.contains(expected_kind_hint),
                "expected cross-routing hint for {kind:?} with {base_url}: {hint}"
            );
        }
    }

    #[test]
    fn provider_configuration_hint_requires_coding_path_even_for_proxy_hosts() {
        let cases = [ProviderKind::ByteplusCoding, ProviderKind::VolcengineCoding];

        for kind in cases {
            let provider = ProviderConfig {
                kind,
                base_url: "https://proxy.example.com/openai/v1".to_owned(),
                base_url_explicit: true,
                ..ProviderConfig::default()
            };
            let hint = provider
                .configuration_hint()
                .expect("coding providers should reject proxy urls that drop the coding path");

            assert!(
                hint.contains("/api/coding/v3"),
                "expected coding-path guidance for {kind:?}: {hint}"
            );
        }
    }

    #[test]
    fn provider_configuration_hint_rejects_non_coding_profiles_on_coding_proxy_paths() {
        let cases = [
            (ProviderKind::Byteplus, "kind = \"byteplus_coding\""),
            (ProviderKind::Volcengine, "kind = \"volcengine_coding\""),
        ];

        for (kind, expected_kind_hint) in cases {
            let provider = ProviderConfig {
                kind,
                base_url: "https://proxy.example.com/api/coding/v3".to_owned(),
                base_url_explicit: true,
                ..ProviderConfig::default()
            };
            let hint = provider
                .configuration_hint()
                .expect("non-coding profiles should reject coding proxy paths");

            assert!(
                hint.contains(expected_kind_hint),
                "expected non-coding guidance for {kind:?}: {hint}"
            );
        }
    }

    #[test]
    fn provider_configuration_hint_allows_proxy_coding_routes_that_keep_the_required_path() {
        let cases = [ProviderKind::ByteplusCoding, ProviderKind::VolcengineCoding];

        for kind in cases {
            let provider = ProviderConfig {
                kind,
                base_url: "https://proxy.example.com/api/coding/v3".to_owned(),
                base_url_explicit: true,
                ..ProviderConfig::default()
            };
            let hint = provider.configuration_hint();

            assert!(
                hint.is_none(),
                "proxy coding routes should stay valid for {kind:?}: {hint:?}"
            );
        }
    }

    #[test]
    fn provider_configuration_hint_checks_explicit_endpoints_before_current_base_url() {
        let provider = ProviderConfig {
            kind: ProviderKind::ByteplusCoding,
            base_url: "https://ark.ap-southeast.bytepluses.com/api/coding/v3".to_owned(),
            base_url_explicit: true,
            endpoint: Some("https://ark.cn-beijing.volces.com/api/v3/chat/completions".to_owned()),
            endpoint_explicit: true,
            ..ProviderConfig::default()
        };
        let hint = provider
            .configuration_hint()
            .expect("explicit cross-routed endpoints should surface a hint");

        assert!(hint.contains("kind = \"volcengine\""));
    }

    #[test]
    fn provider_configuration_hint_requires_coding_path_for_explicit_endpoint_overrides() {
        let cases = [ProviderKind::ByteplusCoding, ProviderKind::VolcengineCoding];

        for kind in cases {
            let provider = ProviderConfig {
                kind,
                endpoint: Some("https://proxy.example.com/openai/v1/chat/completions".to_owned()),
                endpoint_explicit: true,
                ..ProviderConfig::default()
            };
            let hint = provider
                .configuration_hint()
                .expect("explicit endpoint overrides should keep the coding path");

            assert!(
                hint.contains("/api/coding/v3"),
                "expected coding-path guidance for {kind:?}: {hint}"
            );
        }
    }

    #[test]
    fn provider_configuration_hint_requires_coding_path_for_explicit_models_endpoints() {
        let cases = [ProviderKind::ByteplusCoding, ProviderKind::VolcengineCoding];

        for kind in cases {
            let provider = ProviderConfig {
                kind,
                models_endpoint: Some("https://proxy.example.com/openai/v1/models".to_owned()),
                models_endpoint_explicit: true,
                ..ProviderConfig::default()
            };
            let hint = provider
                .configuration_hint()
                .expect("explicit models endpoints should keep the coding path");

            assert!(
                hint.contains("/api/coding/v3"),
                "expected coding-path guidance for {kind:?}: {hint}"
            );
        }
    }

    #[test]
    fn provider_configuration_hint_ignores_non_canonical_host_only_proxy_paths() {
        let provider = ProviderConfig {
            kind: ProviderKind::Byteplus,
            base_url: "https://ark.cn-beijing.volces.com/custom/proxy".to_owned(),
            base_url_explicit: true,
            ..ProviderConfig::default()
        };
        let hint = provider.configuration_hint();

        assert!(
            hint.is_none(),
            "non-canonical host-only proxy paths should not claim a canonical cross-route: {hint:?}"
        );
    }

    #[test]
    fn provider_feature_support_facts_are_stable() {
        let facts = ProviderFeatureFamily::Volcengine.support_facts();

        assert_eq!(facts.family, ProviderFeatureFamily::Volcengine);
        assert_eq!(facts.gate_name, "provider-volcengine");
        assert_eq!(
            facts.enabled_in_build,
            ProviderFeatureFamily::Volcengine.is_enabled_in_build()
        );
        assert_eq!(
            facts.disabled_message,
            "volcengine provider family is disabled (enable feature `provider-volcengine`)"
        );
    }

    #[test]
    fn provider_support_facts_preserve_auth_guidance_and_missing_auth_message() {
        let provider = ProviderConfig {
            kind: ProviderKind::ByteplusCoding,
            ..ProviderConfig::default()
        };

        let support_facts = provider.support_facts();
        let auth_support = support_facts.auth;
        let guidance_hint = auth_support
            .guidance_hint
            .expect("byteplus coding should expose auth guidance");

        assert!(auth_support.requires_explicit_configuration);
        assert!(
            auth_support
                .hint_env_names
                .contains(&"BYTEPLUS_API_KEY".to_owned())
        );
        assert!(guidance_hint.contains("BytePlus"));
        assert!(guidance_hint.contains("BYTEPLUS_API_KEY"));
        assert!(guidance_hint.contains("Authorization: Bearer <BYTEPLUS_API_KEY>"));
        assert!(
            auth_support
                .missing_configuration_message
                .contains("BYTEPLUS_API_KEY")
        );
        assert!(
            auth_support
                .missing_configuration_message
                .contains("BytePlus")
        );
    }

    #[test]
    fn auth_hint_env_names_respect_explicit_api_key_env_binding_precedence() {
        let provider = ProviderConfig {
            kind: ProviderKind::Openai,
            api_key: Some(SecretRef::Env {
                env: "TEAM_OPENAI_KEY".to_owned(),
            }),
            ..ProviderConfig::default()
        };

        let env_names = provider.auth_hint_env_names();

        assert_eq!(env_names, vec!["TEAM_OPENAI_KEY".to_owned()]);
    }

    #[test]
    fn auth_hint_env_names_keep_api_key_fallback_after_explicit_oauth_env_binding() {
        let provider = ProviderConfig {
            kind: ProviderKind::Openai,
            oauth_access_token: Some(SecretRef::Env {
                env: "TEAM_OPENAI_OAUTH".to_owned(),
            }),
            ..ProviderConfig::default()
        };

        let env_names = provider.auth_hint_env_names();

        assert_eq!(
            env_names,
            vec!["TEAM_OPENAI_OAUTH".to_owned(), "OPENAI_API_KEY".to_owned(),]
        );
    }

    #[test]
    fn provider_support_facts_preserve_region_endpoint_hints() {
        let provider = ProviderConfig {
            kind: ProviderKind::Minimax,
            ..ProviderConfig::default()
        };

        let support_facts = provider.support_facts();
        let region_endpoint_support = support_facts.region_endpoint;
        let note = region_endpoint_support
            .note
            .expect("minimax should expose a region endpoint note");
        let catalog_failure_hint = region_endpoint_support
            .catalog_failure_hint
            .expect("minimax should expose a catalog failure hint");
        let request_failure_hint = region_endpoint_support
            .request_failure_hint
            .expect("minimax should expose a request failure hint");

        assert!(note.contains("MiniMax region endpoint"));
        assert!(note.contains("https://api.minimaxi.com"));
        assert!(note.contains("https://api.minimax.io"));
        assert!(catalog_failure_hint.contains("https://api.minimaxi.com"));
        assert!(catalog_failure_hint.contains("https://api.minimax.io"));
        assert!(request_failure_hint.contains("https://api.minimaxi.com"));
        assert!(request_failure_hint.contains("https://api.minimax.io"));
    }

    #[test]
    fn step_plan_provider_support_facts_expose_region_endpoint_variants() {
        let provider = ProviderConfig {
            kind: ProviderKind::StepPlan,
            ..ProviderConfig::default()
        };

        let support_facts = provider.support_facts();
        let region_endpoint_support = support_facts.region_endpoint;
        let note = region_endpoint_support
            .note
            .expect("step plan should expose a region endpoint note");
        let catalog_failure_hint = region_endpoint_support
            .catalog_failure_hint
            .expect("step plan should expose a catalog failure hint");
        let request_failure_hint = region_endpoint_support
            .request_failure_hint
            .expect("step plan should expose a request failure hint");

        assert!(note.contains("Stepfun"));
        assert!(note.contains("https://api.stepfun.com"));
        assert!(note.contains("https://api.stepfun.ai"));
        assert!(catalog_failure_hint.contains("https://api.stepfun.com"));
        assert!(catalog_failure_hint.contains("https://api.stepfun.ai"));
        assert!(request_failure_hint.contains("https://api.stepfun.com"));
        assert!(request_failure_hint.contains("https://api.stepfun.ai"));
    }

    #[test]
    fn provider_descriptor_document_preserves_x_api_key_contract_facts() {
        let provider = ProviderConfig {
            kind: ProviderKind::Anthropic,
            ..ProviderConfig::default()
        };

        let descriptor = provider.descriptor_document();
        let encoded = encode_provider_descriptor(&descriptor);
        let hint_env_names = encoded["auth"]["hint_env_names"]
            .as_array()
            .expect("hint env names should be an array");

        assert_eq!(
            encoded["schema"]["version"],
            json!(PROVIDER_DESCRIPTOR_SCHEMA_VERSION)
        );
        assert_eq!(encoded["schema"]["surface"], json!("provider_descriptor"));
        assert_eq!(encoded["schema"]["purpose"], json!("internal_sdk_contract"));
        assert_eq!(encoded["kind"], json!("anthropic"));
        assert_eq!(encoded["display_name"], json!("Anthropic"));
        assert_eq!(encoded["protocol_family"], json!("anthropic_messages"));
        assert_eq!(encoded["feature"]["family"], json!("anthropic"));
        assert_eq!(encoded["auth"]["scheme"], json!("x_api_key"));
        assert_eq!(
            encoded["auth"]["default_api_key_env"],
            json!("ANTHROPIC_API_KEY")
        );
        assert_eq!(
            encoded["auth"]["requires_explicit_configuration"],
            json!(true)
        );
        assert!(
            hint_env_names.contains(&json!("ANTHROPIC_API_KEY")),
            "anthropic descriptor should surface the canonical x-api-key env hint"
        );
        assert_eq!(
            encoded["default_headers"][0]["name"],
            json!("anthropic-version")
        );
    }

    #[test]
    fn provider_descriptor_document_marks_auth_optional_profiles_without_required_envs() {
        let provider = ProviderConfig {
            kind: ProviderKind::Ollama,
            ..ProviderConfig::default()
        };

        let descriptor = provider.descriptor_document();
        let encoded = encode_provider_descriptor(&descriptor);

        assert_eq!(encoded["kind"], json!("ollama"));
        assert_eq!(encoded["auth"]["scheme"], json!("bearer"));
        assert_eq!(encoded["auth"]["auth_optional"], json!(true));
        assert_eq!(encoded["auth"]["model_probe_auth_optional"], json!(true));
        assert_eq!(
            encoded["auth"]["requires_explicit_configuration"],
            json!(false)
        );
        assert_eq!(encoded["auth"]["hint_env_names"], json!([]));
        assert_eq!(encoded["region_endpoint"]["variants"], json!([]));
    }

    #[test]
    fn provider_descriptor_document_prefers_dynamic_configuration_hint() {
        let provider = ProviderConfig {
            kind: ProviderKind::Custom,
            ..ProviderConfig::default()
        };

        let descriptor = provider.descriptor_document();
        let encoded = encode_provider_descriptor(&descriptor);
        let configuration_hint = encoded["configuration_hint"]
            .as_str()
            .expect("custom descriptor should expose a configuration hint");

        assert!(configuration_hint.contains("tenant-scoped base_url configuration"));
        assert!(configuration_hint.contains("current template"));
        assert!(configuration_hint.contains("https://<openai-compatible-host>/v1"));
    }

    #[test]
    fn provider_descriptor_document_preserves_region_endpoint_variants_and_hints() {
        let provider = ProviderConfig {
            kind: ProviderKind::Minimax,
            ..ProviderConfig::default()
        };

        let descriptor = provider.descriptor_document();
        let encoded = encode_provider_descriptor(&descriptor);
        let note = encoded["region_endpoint"]["note"]
            .as_str()
            .expect("minimax descriptor should expose a region note");
        let catalog_failure_hint = encoded["region_endpoint"]["catalog_failure_hint"]
            .as_str()
            .expect("minimax descriptor should expose a catalog failure hint");
        let request_failure_hint = encoded["region_endpoint"]["request_failure_hint"]
            .as_str()
            .expect("minimax descriptor should expose a request failure hint");

        assert_eq!(encoded["region_endpoint"]["family_label"], json!("MiniMax"));
        assert_eq!(
            encoded["region_endpoint"]["variants"][0]["label"],
            json!("CN")
        );
        assert_eq!(
            encoded["region_endpoint"]["variants"][0]["base_url"],
            json!("https://api.minimaxi.com")
        );
        assert_eq!(
            encoded["region_endpoint"]["variants"][1]["label"],
            json!("Global")
        );
        assert_eq!(
            encoded["region_endpoint"]["variants"][1]["base_url"],
            json!("https://api.minimax.io")
        );
        assert!(note.contains("MiniMax region endpoint"));
        assert!(catalog_failure_hint.contains("https://api.minimaxi.com"));
        assert!(catalog_failure_hint.contains("https://api.minimax.io"));
        assert!(request_failure_hint.contains("https://api.minimaxi.com"));
        assert!(request_failure_hint.contains("https://api.minimax.io"));
    }

    #[test]
    fn step_plan_descriptor_document_preserves_region_endpoint_variants_and_hints() {
        let provider = ProviderConfig {
            kind: ProviderKind::StepPlan,
            ..ProviderConfig::default()
        };

        let descriptor = provider.descriptor_document();
        let encoded = encode_provider_descriptor(&descriptor);
        let note = encoded["region_endpoint"]["note"]
            .as_str()
            .expect("step plan descriptor should expose a region note");
        let catalog_failure_hint = encoded["region_endpoint"]["catalog_failure_hint"]
            .as_str()
            .expect("step plan descriptor should expose a catalog failure hint");
        let request_failure_hint = encoded["region_endpoint"]["request_failure_hint"]
            .as_str()
            .expect("step plan descriptor should expose a request failure hint");

        assert_eq!(encoded["region_endpoint"]["family_label"], json!("Stepfun"));
        assert_eq!(
            encoded["region_endpoint"]["variants"][0]["label"],
            json!("CN")
        );
        assert_eq!(
            encoded["region_endpoint"]["variants"][0]["base_url"],
            json!("https://api.stepfun.com")
        );
        assert_eq!(
            encoded["region_endpoint"]["variants"][1]["label"],
            json!("Global")
        );
        assert_eq!(
            encoded["region_endpoint"]["variants"][1]["base_url"],
            json!("https://api.stepfun.ai")
        );
        assert!(note.contains("https://api.stepfun.com"));
        assert!(note.contains("https://api.stepfun.ai"));
        assert!(catalog_failure_hint.contains("https://api.stepfun.com"));
        assert!(catalog_failure_hint.contains("https://api.stepfun.ai"));
        assert!(request_failure_hint.contains("https://api.stepfun.com"));
        assert!(request_failure_hint.contains("https://api.stepfun.ai"));
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
            api_key: Some(SecretRef::Inline("${OPENAI_API_KEY}".to_owned())),
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
    fn explicit_api_key_env_field_beats_default_oauth_fallback() {
        let mut env = ScopedEnv::new();
        env.set("OPENAI_API_KEY", "api-key-wins");
        env.set("OPENAI_CODEX_OAUTH_TOKEN", "oauth-fallback-should-not-win");

        let config: ProviderConfig = toml::from_str(
            r#"
kind = "openai"
api_key_env = "OPENAI_API_KEY"
"#,
        )
        .expect("deserialize provider config");

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
    fn normalized_for_persistence_canonicalizes_legacy_api_key_env_binding() {
        let mut config = ProviderConfig::fresh_for_kind(ProviderKind::Openai);
        config.set_api_key_env(Some("OPENAI_API_KEY".to_owned()));

        let normalized = config.normalized_for_persistence();

        assert_eq!(
            normalized.api_key,
            Some(SecretRef::Env {
                env: "OPENAI_API_KEY".to_owned(),
            })
        );
        assert_eq!(normalized.api_key_env, None);
        assert_eq!(normalized.oauth_access_token, None);
        assert_eq!(normalized.oauth_access_token_env, None);
    }

    #[test]
    fn normalized_for_persistence_keeps_secret_ref_env_binding_canonical() {
        let mut config = ProviderConfig::fresh_for_kind(ProviderKind::Openai);
        config.api_key = Some(SecretRef::Env {
            env: "OPENAI_API_KEY".to_owned(),
        });

        let normalized = config.normalized_for_persistence();

        assert_eq!(
            normalized.api_key,
            Some(SecretRef::Env {
                env: "OPENAI_API_KEY".to_owned(),
            })
        );
        assert_eq!(normalized.api_key_env, None);
    }

    #[test]
    fn normalized_for_persistence_keeps_implicit_provider_auth_defaults_unset() {
        let config = ProviderConfig::fresh_for_kind(ProviderKind::Openai);

        let normalized = config.normalized_for_persistence();

        assert_eq!(normalized.api_key, None);
        assert_eq!(normalized.api_key_env, None);
        assert_eq!(normalized.oauth_access_token, None);
        assert_eq!(normalized.oauth_access_token_env, None);
    }

    #[test]
    fn canonicalize_configured_auth_env_bindings_rewrites_inline_env_templates() {
        let mut config = ProviderConfig::fresh_for_kind(ProviderKind::Openai);
        config.set_api_key_env(Some("OPENAI_API_KEY".to_owned()));
        config.api_key = Some(SecretRef::Inline("${OPENAI_API_KEY}".to_owned()));

        config.canonicalize_configured_auth_env_bindings();

        assert_eq!(
            config.api_key,
            Some(SecretRef::Env {
                env: "OPENAI_API_KEY".to_owned(),
            })
        );
        assert_eq!(config.api_key_env, None);
    }

    #[test]
    fn canonicalize_configured_auth_env_bindings_treats_blank_inline_secret_as_absent() {
        let mut config = ProviderConfig::fresh_for_kind(ProviderKind::Openai);
        config.set_api_key_env(Some("OPENAI_API_KEY".to_owned()));
        config.api_key = Some(SecretRef::Inline("   ".to_owned()));

        config.canonicalize_configured_auth_env_bindings();

        assert_eq!(
            config.api_key,
            Some(SecretRef::Env {
                env: "OPENAI_API_KEY".to_owned(),
            })
        );
        assert_eq!(config.api_key_env, None);
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
            Some("MiniMax-M2.7")
        );
        assert_eq!(
            ProviderKind::Xiaomi.recommended_onboarding_model(),
            Some("mimo-v2-pro")
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
