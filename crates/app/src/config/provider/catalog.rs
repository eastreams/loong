use super::*;

impl ProviderUrlValidationProfile {
    pub(super) fn matching_canonical_fingerprint(
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

    pub(super) fn matches_required_path_fragment(self, value: &str) -> bool {
        for fragment in self.required_path_fragments {
            let fragment_match = url_contains_validation_fragment(value, fragment);
            if fragment_match {
                return true;
            }
        }

        false
    }

    pub(super) fn matches_forbidden_path_fragment(self, value: &str) -> bool {
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

    pub(super) fn url_validation_profile(self) -> Option<&'static ProviderUrlValidationProfile> {
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

    pub(super) fn region_endpoint_guide(self) -> Option<ProviderRegionEndpointGuide> {
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

pub(super) fn find_cross_routed_validation_profile(
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

pub(super) fn provider_descriptor_aliases(profile: &ProviderProfile) -> Vec<String> {
    let mut aliases = Vec::new();

    for alias in profile.aliases {
        let alias = (*alias).to_owned();
        aliases.push(alias);
    }

    aliases
}

pub(super) fn provider_descriptor_headers(
    profile: &ProviderProfile,
) -> Vec<ProviderDescriptorHeader> {
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

pub(super) fn provider_descriptor_env_aliases(raw_aliases: &[&str]) -> Vec<String> {
    let mut aliases = Vec::new();

    for raw_alias in raw_aliases {
        let alias = (*raw_alias).to_owned();
        aliases.push(alias);
    }

    aliases
}

pub(super) fn build_provider_descriptor_feature(
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

pub(super) fn provider_descriptor_region_variants(
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

pub(super) const PROVIDER_PROFILES: [ProviderProfile; 45] = [
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
