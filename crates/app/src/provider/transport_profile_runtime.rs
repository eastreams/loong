use crate::CliResult;
use crate::config::{
    ANTHROPIC_DEFAULT_HEADERS, ProviderAuthScheme, ProviderConfig,
    ProviderFeatureFamily as ConfigProviderFeatureFamily, ProviderKind,
};

use super::contracts::{ProviderFeatureFamily, ProviderTransportMode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProviderRequestTransportProfile {
    pub(super) endpoint: String,
    pub(super) request_model: String,
    pub(super) transport_mode: ProviderTransportMode,
    pub(super) feature_family: ProviderFeatureFamily,
    pub(super) auth_scheme: ProviderAuthScheme,
    pub(super) default_headers: &'static [(&'static str, &'static str)],
    pub(super) default_user_agent: Option<&'static str>,
}

pub(super) fn resolve_provider_request_transport_profile(
    provider: &ProviderConfig,
    model: &str,
) -> CliResult<ProviderRequestTransportProfile> {
    if provider.kind == ProviderKind::OpencodeZen {
        return resolve_opencode_zen_transport_profile(provider, model);
    }

    if provider.kind == ProviderKind::OpencodeGo {
        return resolve_opencode_go_transport_profile(provider, model);
    }

    Ok(default_provider_request_transport_profile(provider, model))
}

fn default_provider_request_transport_profile(
    provider: &ProviderConfig,
    model: &str,
) -> ProviderRequestTransportProfile {
    let endpoint = provider.endpoint();
    let request_model = model.trim().to_owned();
    let transport_mode = ProviderTransportMode::for_provider(provider);
    let feature_family = runtime_feature_family(provider);
    let auth_scheme = provider.kind.auth_scheme();
    let default_headers = provider.kind.default_headers();
    let default_user_agent = provider.kind.default_user_agent();

    ProviderRequestTransportProfile {
        endpoint,
        request_model,
        transport_mode,
        feature_family,
        auth_scheme,
        default_headers,
        default_user_agent,
    }
}

fn resolve_opencode_zen_transport_profile(
    provider: &ProviderConfig,
    raw_model: &str,
) -> CliResult<ProviderRequestTransportProfile> {
    let request_model = normalize_opencode_model(provider.kind, raw_model)?;
    let normalized_model = request_model.to_ascii_lowercase();
    let resolved_base_url = provider.resolved_base_url();

    if normalized_model.starts_with("gpt-") {
        return Ok(ProviderRequestTransportProfile {
            endpoint: join_opencode_base_url(resolved_base_url.as_str(), "/responses"),
            request_model,
            transport_mode: ProviderTransportMode::Responses,
            feature_family: ProviderFeatureFamily::OpenAiCompatible,
            auth_scheme: ProviderAuthScheme::Bearer,
            default_headers: &[],
            default_user_agent: None,
        });
    }

    if normalized_model.starts_with("claude-") {
        return Ok(ProviderRequestTransportProfile {
            endpoint: join_opencode_base_url(resolved_base_url.as_str(), "/messages"),
            request_model,
            transport_mode: ProviderTransportMode::AnthropicMessages,
            feature_family: ProviderFeatureFamily::Anthropic,
            auth_scheme: ProviderAuthScheme::XApiKey,
            default_headers: &ANTHROPIC_DEFAULT_HEADERS,
            default_user_agent: None,
        });
    }

    if normalized_model.starts_with("gemini-") {
        return Ok(ProviderRequestTransportProfile {
            endpoint: join_opencode_base_url(
                resolved_base_url.as_str(),
                format!("/models/{request_model}").as_str(),
            ),
            request_model,
            transport_mode: ProviderTransportMode::GoogleGenerateContent,
            feature_family: ProviderFeatureFamily::Google,
            auth_scheme: ProviderAuthScheme::XGoogApiKey,
            default_headers: &[],
            default_user_agent: None,
        });
    }

    Ok(ProviderRequestTransportProfile {
        endpoint: join_opencode_base_url(resolved_base_url.as_str(), "/chat/completions"),
        request_model,
        transport_mode: ProviderTransportMode::OpenAiChatCompletions,
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_user_agent: None,
    })
}

fn resolve_opencode_go_transport_profile(
    provider: &ProviderConfig,
    raw_model: &str,
) -> CliResult<ProviderRequestTransportProfile> {
    let request_model = normalize_opencode_model(provider.kind, raw_model)?;
    let normalized_model = request_model.to_ascii_lowercase();
    let resolved_base_url = provider.resolved_base_url();

    if normalized_model.starts_with("minimax-") {
        return Ok(ProviderRequestTransportProfile {
            endpoint: join_opencode_base_url(resolved_base_url.as_str(), "/messages"),
            request_model,
            transport_mode: ProviderTransportMode::AnthropicMessages,
            feature_family: ProviderFeatureFamily::Anthropic,
            auth_scheme: ProviderAuthScheme::XApiKey,
            default_headers: &ANTHROPIC_DEFAULT_HEADERS,
            default_user_agent: None,
        });
    }

    Ok(ProviderRequestTransportProfile {
        endpoint: join_opencode_base_url(resolved_base_url.as_str(), "/chat/completions"),
        request_model,
        transport_mode: ProviderTransportMode::OpenAiChatCompletions,
        feature_family: ProviderFeatureFamily::OpenAiCompatible,
        auth_scheme: ProviderAuthScheme::Bearer,
        default_headers: &[],
        default_user_agent: None,
    })
}

fn normalize_opencode_model(kind: ProviderKind, raw_model: &str) -> CliResult<String> {
    let trimmed_model = raw_model.trim();
    if trimmed_model.is_empty() {
        return Err(format!(
            "{} model id is empty; set `provider.model` explicitly or fetch the model catalog first",
            kind.as_str()
        ));
    }

    let other_prefix = if kind == ProviderKind::OpencodeZen {
        "opencode-go/"
    } else {
        "opencode/"
    };
    let other_kind = if kind == ProviderKind::OpencodeZen {
        "opencode_go"
    } else {
        "opencode_zen"
    };
    let lowered_model = trimmed_model.to_ascii_lowercase();
    if lowered_model.starts_with(other_prefix) {
        return Err(format!(
            "{} uses the `{}` model namespace; switch to `kind = \"{}\"` or remove the copied prefix",
            kind.as_str(),
            other_prefix.trim_end_matches('/'),
            other_kind
        ));
    }

    let own_prefix = if kind == ProviderKind::OpencodeZen {
        "opencode/"
    } else {
        "opencode-go/"
    };
    let without_prefix = if lowered_model.starts_with(own_prefix) {
        let prefix_len = own_prefix.len();
        trimmed_model.get(prefix_len..).unwrap_or_default()
    } else {
        trimmed_model
    };
    let normalized_model = without_prefix.trim().to_owned();
    if normalized_model.is_empty() {
        return Err(format!(
            "{} model id is empty after removing the copied `{}` prefix",
            kind.as_str(),
            own_prefix.trim_end_matches('/')
        ));
    }

    Ok(normalized_model)
}

fn join_opencode_base_url(base_url: &str, path: &str) -> String {
    let normalized_base_url = base_url.trim_end_matches('/');
    format!("{normalized_base_url}{path}")
}

fn runtime_feature_family(provider: &ProviderConfig) -> ProviderFeatureFamily {
    match provider.kind.feature_family() {
        ConfigProviderFeatureFamily::OpenAiCompatible => ProviderFeatureFamily::OpenAiCompatible,
        ConfigProviderFeatureFamily::Anthropic => ProviderFeatureFamily::Anthropic,
        ConfigProviderFeatureFamily::Bedrock => ProviderFeatureFamily::Bedrock,
        ConfigProviderFeatureFamily::Volcengine => ProviderFeatureFamily::VolcengineCompatible,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencode_zen_routes_gpt_models_to_responses() {
        let provider = ProviderConfig::fresh_for_kind(ProviderKind::OpencodeZen);
        let profile = resolve_provider_request_transport_profile(&provider, "opencode/gpt-5.4")
            .expect("transport profile");

        assert_eq!(profile.request_model, "gpt-5.4");
        assert_eq!(profile.endpoint, "https://opencode.ai/zen/v1/responses");
        assert_eq!(profile.transport_mode, ProviderTransportMode::Responses);
        assert_eq!(profile.auth_scheme, ProviderAuthScheme::Bearer);
    }

    #[test]
    fn opencode_zen_routes_claude_models_to_messages() {
        let provider = ProviderConfig::fresh_for_kind(ProviderKind::OpencodeZen);
        let profile = resolve_provider_request_transport_profile(&provider, "claude-sonnet-4-6")
            .expect("transport profile");

        assert_eq!(profile.endpoint, "https://opencode.ai/zen/v1/messages");
        assert_eq!(
            profile.transport_mode,
            ProviderTransportMode::AnthropicMessages
        );
        assert_eq!(profile.auth_scheme, ProviderAuthScheme::XApiKey);
        assert_eq!(profile.default_headers, &ANTHROPIC_DEFAULT_HEADERS);
    }

    #[test]
    fn opencode_zen_routes_gemini_models_to_google_transport() {
        let provider = ProviderConfig::fresh_for_kind(ProviderKind::OpencodeZen);
        let profile = resolve_provider_request_transport_profile(&provider, "gemini-3.1-pro")
            .expect("transport profile");

        assert_eq!(
            profile.endpoint,
            "https://opencode.ai/zen/v1/models/gemini-3.1-pro"
        );
        assert_eq!(
            profile.transport_mode,
            ProviderTransportMode::GoogleGenerateContent
        );
        assert_eq!(profile.auth_scheme, ProviderAuthScheme::XGoogApiKey);
    }

    #[test]
    fn opencode_go_routes_minimax_models_to_messages() {
        let provider = ProviderConfig::fresh_for_kind(ProviderKind::OpencodeGo);
        let profile =
            resolve_provider_request_transport_profile(&provider, "opencode-go/minimax-m2.7")
                .expect("transport profile");

        assert_eq!(profile.request_model, "minimax-m2.7");
        assert_eq!(profile.endpoint, "https://opencode.ai/zen/go/v1/messages");
        assert_eq!(
            profile.transport_mode,
            ProviderTransportMode::AnthropicMessages
        );
    }

    #[test]
    fn opencode_go_routes_other_models_to_chat_completions() {
        let provider = ProviderConfig::fresh_for_kind(ProviderKind::OpencodeGo);
        let profile = resolve_provider_request_transport_profile(&provider, "glm-5.1")
            .expect("transport profile");

        assert_eq!(
            profile.endpoint,
            "https://opencode.ai/zen/go/v1/chat/completions"
        );
        assert_eq!(
            profile.transport_mode,
            ProviderTransportMode::OpenAiChatCompletions
        );
    }

    #[test]
    fn opencode_model_normalization_rejects_cross_provider_prefixes() {
        let provider = ProviderConfig::fresh_for_kind(ProviderKind::OpencodeZen);
        let error = resolve_provider_request_transport_profile(&provider, "opencode-go/glm-5.1")
            .expect_err("cross-provider model prefix should fail");

        assert!(error.contains("kind = \"opencode_go\""));
    }

    #[test]
    fn opencode_model_normalization_preserves_original_model_casing() {
        let provider = ProviderConfig::fresh_for_kind(ProviderKind::OpencodeZen);
        let profile = resolve_provider_request_transport_profile(&provider, "opencode/GPT-5.4")
            .expect("transport profile");

        assert_eq!(profile.request_model, "GPT-5.4");
    }
}
