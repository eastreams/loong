use serde_json::Value;

use crate::config::{
    ProviderConfig, ProviderKind, ProviderProfileHealthModeConfig, ProviderProtocolFamily,
    ProviderReasoningExtraBodyModeConfig, ProviderToolSchemaModeConfig, ProviderWireApi,
};

use super::ProviderProfileHealthMode;

const DEFAULT_UNSUPPORTED_PARAMETER_MESSAGE_FRAGMENTS: &[&str] = &[
    "unknown parameter",
    "unsupported parameter",
    "not supported",
];
const DEFAULT_TOOL_SCHEMA_ERROR_PARAMETERS: &[&str] = &["tools", "tool_choice"];
const DEFAULT_TOOL_SCHEMA_ERROR_MESSAGE_FRAGMENTS: &[&str] = &[
    "tools are not supported",
    "tool use is not supported",
    "function calling is not supported",
];
const DEFAULT_MODEL_NOT_FOUND_CODES: &[&str] = &[
    "model_not_found",
    "unsupported_model",
    "invalid_model",
    "not_found_error",
];
const DEFAULT_MODEL_ERROR_PARAMETERS: &[&str] = &["model"];
const DEFAULT_MODEL_MISMATCH_MESSAGE_FRAGMENTS: &[&str] = &[
    "only supports",
    "only available",
    "this endpoint",
    "/v1/responses",
    "model does not exist",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProviderTransportMode {
    OpenAiChatCompletions,
    AnthropicMessages,
    BedrockConverse,
    GoogleGenerateContent,
    Responses,
    KimiApi,
}

impl ProviderTransportMode {
    pub(super) fn for_provider(provider: &ProviderConfig) -> Self {
        match provider.kind.protocol_family() {
            ProviderProtocolFamily::AnthropicMessages => Self::AnthropicMessages,
            ProviderProtocolFamily::BedrockConverse => Self::BedrockConverse,
            ProviderProtocolFamily::OpenAiChatCompletions => {
                if matches!(provider.kind, ProviderKind::KimiCoding) {
                    Self::KimiApi
                } else {
                    match provider.wire_api {
                        ProviderWireApi::ChatCompletions => Self::OpenAiChatCompletions,
                        ProviderWireApi::Responses => Self::Responses,
                    }
                }
            }
        }
    }

    pub(super) fn supports_turn_streaming_events(self) -> bool {
        matches!(self, Self::AnthropicMessages)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProviderFeatureFamily {
    OpenAiCompatible,
    Anthropic,
    Bedrock,
    Google,
    VolcengineCompatible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ProviderValidationContract {
    pub(super) forbid_kimi_coding_endpoint: bool,
    pub(super) require_kimi_cli_user_agent_prefix: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ProviderErrorClassificationContract {
    pub(super) unsupported_parameter_message_fragments: &'static [&'static str],
    pub(super) tool_schema_error_parameters: &'static [&'static str],
    pub(super) tool_schema_error_message_fragments: &'static [&'static str],
    pub(super) model_not_found_codes: &'static [&'static str],
    pub(super) model_error_parameters: &'static [&'static str],
    pub(super) model_mismatch_message_fragments: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ProviderCapabilityContract {
    pub(super) tool_schema_mode: ProviderToolSchemaMode,
    pub(super) reasoning_extra_body_mode: ProviderReasoningExtraBodyMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProviderToolSchemaMode {
    Disabled,
    EnabledStrict,
    EnabledWithDowngradeOnUnsupported,
}

impl ProviderToolSchemaMode {
    pub(super) fn is_enabled(self) -> bool {
        !matches!(self, Self::Disabled)
    }

    pub(super) fn allows_downgrade_on_unsupported(self) -> bool {
        matches!(self, Self::EnabledWithDowngradeOnUnsupported)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProviderReasoningExtraBodyMode {
    Omit,
    KimiThinking,
}

impl ProviderCapabilityContract {
    pub(super) fn turn_tool_schema_enabled(self) -> bool {
        self.tool_schema_mode.is_enabled()
    }

    pub(super) fn tool_schema_downgrade_on_unsupported(self) -> bool {
        self.tool_schema_mode.allows_downgrade_on_unsupported()
    }

    pub(super) fn include_reasoning_extra_body(self) -> bool {
        !matches!(
            self.reasoning_extra_body_mode,
            ProviderReasoningExtraBodyMode::Omit
        )
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ProviderCapabilityContractOverrides {
    tool_schema_mode: Option<ProviderToolSchemaMode>,
    reasoning_extra_body_mode: Option<ProviderReasoningExtraBodyMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ProviderRuntimeContract {
    pub(super) feature_family: ProviderFeatureFamily,
    pub(super) default_token_field: TokenLimitField,
    pub(super) default_reasoning_field: ReasoningField,
    pub(super) default_temperature_field: TemperatureField,
    pub(super) payload_adaptation: ProviderPayloadAdaptationContract,
    pub(super) transport_mode: ProviderTransportMode,
    pub(super) profile_health_mode: ProviderProfileHealthMode,
    pub(super) validation: ProviderValidationContract,
    pub(super) error_classification: ProviderErrorClassificationContract,
    pub(super) capability: ProviderCapabilityContract,
}

impl ProviderRuntimeContract {
    pub(super) fn supports_turn_streaming_events(self) -> bool {
        self.transport_mode.supports_turn_streaming_events()
    }
}

pub(super) fn provider_runtime_contract(provider: &ProviderConfig) -> ProviderRuntimeContract {
    let transport_mode = ProviderTransportMode::for_provider(provider);
    let feature_family = match provider.kind.feature_family() {
        crate::config::ProviderFeatureFamily::OpenAiCompatible => {
            ProviderFeatureFamily::OpenAiCompatible
        }
        crate::config::ProviderFeatureFamily::Anthropic => ProviderFeatureFamily::Anthropic,
        crate::config::ProviderFeatureFamily::Bedrock => ProviderFeatureFamily::Bedrock,
        crate::config::ProviderFeatureFamily::Volcengine => {
            ProviderFeatureFamily::VolcengineCompatible
        }
    };
    build_provider_runtime_contract(provider, transport_mode, feature_family)
}

pub(super) fn provider_runtime_contract_for_route(
    provider: &ProviderConfig,
    transport_mode: ProviderTransportMode,
    feature_family: ProviderFeatureFamily,
) -> ProviderRuntimeContract {
    build_provider_runtime_contract(provider, transport_mode, feature_family)
}

fn build_provider_runtime_contract(
    provider: &ProviderConfig,
    transport_mode: ProviderTransportMode,
    feature_family: ProviderFeatureFamily,
) -> ProviderRuntimeContract {
    let default_token_field = match transport_mode {
        ProviderTransportMode::Responses | ProviderTransportMode::GoogleGenerateContent => {
            TokenLimitField::MaxOutputTokens
        }
        ProviderTransportMode::OpenAiChatCompletions
            if matches!(provider.kind, ProviderKind::Openai) =>
        {
            TokenLimitField::MaxCompletionTokens
        }
        ProviderTransportMode::OpenAiChatCompletions
        | ProviderTransportMode::AnthropicMessages
        | ProviderTransportMode::BedrockConverse
        | ProviderTransportMode::KimiApi => TokenLimitField::MaxTokens,
    };
    let validation = if matches!(provider.kind, ProviderKind::Kimi) {
        ProviderValidationContract {
            forbid_kimi_coding_endpoint: true,
            require_kimi_cli_user_agent_prefix: false,
        }
    } else if matches!(provider.kind, ProviderKind::KimiCoding) {
        ProviderValidationContract {
            forbid_kimi_coding_endpoint: false,
            require_kimi_cli_user_agent_prefix: true,
        }
    } else {
        ProviderValidationContract {
            forbid_kimi_coding_endpoint: false,
            require_kimi_cli_user_agent_prefix: false,
        }
    };
    let profile_health_mode = match provider.resolved_profile_health_mode_config() {
        ProviderProfileHealthModeConfig::ProviderDefault => {
            if matches!(provider.kind, ProviderKind::Openrouter) {
                ProviderProfileHealthMode::ObserveOnly
            } else {
                ProviderProfileHealthMode::EnforceUnusableWindows
            }
        }
        ProviderProfileHealthModeConfig::Enforce => {
            ProviderProfileHealthMode::EnforceUnusableWindows
        }
        ProviderProfileHealthModeConfig::ObserveOnly => ProviderProfileHealthMode::ObserveOnly,
    };
    let default_reasoning_field = match transport_mode {
        ProviderTransportMode::Responses => ReasoningField::ReasoningObject,
        ProviderTransportMode::OpenAiChatCompletions | ProviderTransportMode::KimiApi => {
            ReasoningField::ReasoningEffort
        }
        ProviderTransportMode::AnthropicMessages
        | ProviderTransportMode::BedrockConverse
        | ProviderTransportMode::GoogleGenerateContent => ReasoningField::Omit,
    };
    let default_temperature_field = TemperatureField::Include;
    let payload_adaptation = provider_payload_adaptation_contract(
        feature_family,
        default_token_field,
        default_reasoning_field,
        default_temperature_field,
    );
    let error_classification = provider_error_classification_contract(feature_family);
    let capability = provider_capability_contract(feature_family, provider);

    ProviderRuntimeContract {
        feature_family,
        default_token_field,
        default_reasoning_field,
        default_temperature_field,
        payload_adaptation,
        transport_mode,
        profile_health_mode,
        validation,
        error_classification,
        capability,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ProviderPayloadAdaptationContract {
    pub(super) token_field_progression: [TokenLimitField; 4],
    pub(super) reasoning_field_progression: [ReasoningField; 3],
    pub(super) temperature_field_progression: [TemperatureField; 2],
    pub(super) unsupported_parameter_message_fragments: &'static [&'static str],
    pub(super) token_error_parameters: &'static [&'static str],
    pub(super) reasoning_error_parameters: &'static [&'static str],
    pub(super) temperature_error_parameters: &'static [&'static str],
    pub(super) temperature_default_only_fragments: &'static [&'static str],
}

fn provider_payload_adaptation_contract(
    feature_family: ProviderFeatureFamily,
    default_token_field: TokenLimitField,
    default_reasoning_field: ReasoningField,
    default_temperature_field: TemperatureField,
) -> ProviderPayloadAdaptationContract {
    match feature_family {
        ProviderFeatureFamily::OpenAiCompatible | ProviderFeatureFamily::VolcengineCompatible => {
            ProviderPayloadAdaptationContract {
                token_field_progression: token_field_progression(default_token_field),
                reasoning_field_progression: reasoning_field_progression(default_reasoning_field),
                temperature_field_progression: temperature_field_progression(
                    default_temperature_field,
                ),
                unsupported_parameter_message_fragments:
                    DEFAULT_UNSUPPORTED_PARAMETER_MESSAGE_FRAGMENTS,
                token_error_parameters: &[
                    "max_output_tokens",
                    "max_tokens",
                    "max_completion_tokens",
                ],
                reasoning_error_parameters: &["reasoning_effort", "reasoning"],
                temperature_error_parameters: &["temperature"],
                temperature_default_only_fragments: &["only the default"],
            }
        }
        ProviderFeatureFamily::Anthropic | ProviderFeatureFamily::Bedrock => {
            ProviderPayloadAdaptationContract {
                token_field_progression: token_field_progression(default_token_field),
                reasoning_field_progression: reasoning_field_progression(default_reasoning_field),
                temperature_field_progression: temperature_field_progression(
                    default_temperature_field,
                ),
                unsupported_parameter_message_fragments:
                    DEFAULT_UNSUPPORTED_PARAMETER_MESSAGE_FRAGMENTS,
                token_error_parameters: &["max_tokens", "max_completion_tokens"],
                reasoning_error_parameters: &["reasoning_effort", "reasoning"],
                temperature_error_parameters: &["temperature"],
                temperature_default_only_fragments: &["only the default"],
            }
        }
        ProviderFeatureFamily::Google => ProviderPayloadAdaptationContract {
            token_field_progression: google_token_field_progression(),
            reasoning_field_progression: reasoning_field_progression(default_reasoning_field),
            temperature_field_progression: temperature_field_progression(default_temperature_field),
            unsupported_parameter_message_fragments:
                DEFAULT_UNSUPPORTED_PARAMETER_MESSAGE_FRAGMENTS,
            token_error_parameters: &["maxOutputTokens", "generationConfig.maxOutputTokens"],
            reasoning_error_parameters: &["reasoning_effort", "reasoning"],
            temperature_error_parameters: &["temperature", "generationConfig.temperature"],
            temperature_default_only_fragments: &["only the default"],
        },
    }
}

fn provider_error_classification_contract(
    feature_family: ProviderFeatureFamily,
) -> ProviderErrorClassificationContract {
    match feature_family {
        ProviderFeatureFamily::OpenAiCompatible
        | ProviderFeatureFamily::Anthropic
        | ProviderFeatureFamily::Bedrock
        | ProviderFeatureFamily::Google
        | ProviderFeatureFamily::VolcengineCompatible => ProviderErrorClassificationContract {
            unsupported_parameter_message_fragments:
                DEFAULT_UNSUPPORTED_PARAMETER_MESSAGE_FRAGMENTS,
            tool_schema_error_parameters: DEFAULT_TOOL_SCHEMA_ERROR_PARAMETERS,
            tool_schema_error_message_fragments: DEFAULT_TOOL_SCHEMA_ERROR_MESSAGE_FRAGMENTS,
            model_not_found_codes: DEFAULT_MODEL_NOT_FOUND_CODES,
            model_error_parameters: DEFAULT_MODEL_ERROR_PARAMETERS,
            model_mismatch_message_fragments: DEFAULT_MODEL_MISMATCH_MESSAGE_FRAGMENTS,
        },
    }
}

fn provider_capability_contract(
    feature_family: ProviderFeatureFamily,
    provider: &ProviderConfig,
) -> ProviderCapabilityContract {
    let mut capability = match feature_family {
        ProviderFeatureFamily::OpenAiCompatible
        | ProviderFeatureFamily::Anthropic
        | ProviderFeatureFamily::Bedrock
        | ProviderFeatureFamily::Google
        | ProviderFeatureFamily::VolcengineCompatible => ProviderCapabilityContract {
            tool_schema_mode: ProviderToolSchemaMode::EnabledWithDowngradeOnUnsupported,
            reasoning_extra_body_mode: ProviderReasoningExtraBodyMode::Omit,
        },
    };

    let overrides = provider_capability_overrides(provider.kind);
    if let Some(mode) = overrides.tool_schema_mode {
        capability.tool_schema_mode = mode;
    }
    if let Some(mode) = overrides.reasoning_extra_body_mode {
        capability.reasoning_extra_body_mode = mode;
    }
    apply_provider_capability_config_overrides(provider, &mut capability);

    capability
}

fn provider_capability_overrides(
    provider_kind: ProviderKind,
) -> ProviderCapabilityContractOverrides {
    if matches!(provider_kind, ProviderKind::KimiCoding) {
        ProviderCapabilityContractOverrides {
            reasoning_extra_body_mode: Some(ProviderReasoningExtraBodyMode::KimiThinking),
            ..ProviderCapabilityContractOverrides::default()
        }
    } else {
        ProviderCapabilityContractOverrides::default()
    }
}

fn apply_provider_capability_config_overrides(
    provider: &ProviderConfig,
    capability: &mut ProviderCapabilityContract,
) {
    capability.tool_schema_mode = match provider.resolved_tool_schema_mode_config() {
        ProviderToolSchemaModeConfig::ProviderDefault => capability.tool_schema_mode,
        ProviderToolSchemaModeConfig::Disabled => ProviderToolSchemaMode::Disabled,
        ProviderToolSchemaModeConfig::EnabledStrict => ProviderToolSchemaMode::EnabledStrict,
        ProviderToolSchemaModeConfig::EnabledWithDowngrade => {
            ProviderToolSchemaMode::EnabledWithDowngradeOnUnsupported
        }
    };
    capability.reasoning_extra_body_mode =
        match provider.resolved_reasoning_extra_body_mode_config() {
            ProviderReasoningExtraBodyModeConfig::ProviderDefault => {
                capability.reasoning_extra_body_mode
            }
            ProviderReasoningExtraBodyModeConfig::Omit => ProviderReasoningExtraBodyMode::Omit,
            ProviderReasoningExtraBodyModeConfig::KimiThinking => {
                ProviderReasoningExtraBodyMode::KimiThinking
            }
        };
}

fn token_field_progression(default_field: TokenLimitField) -> [TokenLimitField; 4] {
    match default_field {
        TokenLimitField::MaxOutputTokens => [
            TokenLimitField::MaxOutputTokens,
            TokenLimitField::MaxTokens,
            TokenLimitField::MaxCompletionTokens,
            TokenLimitField::Omit,
        ],
        TokenLimitField::MaxCompletionTokens => [
            TokenLimitField::MaxCompletionTokens,
            TokenLimitField::MaxTokens,
            TokenLimitField::Omit,
            TokenLimitField::Omit,
        ],
        TokenLimitField::MaxTokens => [
            TokenLimitField::MaxTokens,
            TokenLimitField::MaxCompletionTokens,
            TokenLimitField::Omit,
            TokenLimitField::Omit,
        ],
        TokenLimitField::Omit => [
            TokenLimitField::Omit,
            TokenLimitField::Omit,
            TokenLimitField::Omit,
            TokenLimitField::Omit,
        ],
    }
}

fn google_token_field_progression() -> [TokenLimitField; 4] {
    [
        TokenLimitField::MaxOutputTokens,
        TokenLimitField::Omit,
        TokenLimitField::Omit,
        TokenLimitField::Omit,
    ]
}

fn reasoning_field_progression(default_field: ReasoningField) -> [ReasoningField; 3] {
    match default_field {
        ReasoningField::ReasoningEffort => [
            ReasoningField::ReasoningEffort,
            ReasoningField::ReasoningObject,
            ReasoningField::Omit,
        ],
        ReasoningField::ReasoningObject => [
            ReasoningField::ReasoningObject,
            ReasoningField::ReasoningEffort,
            ReasoningField::Omit,
        ],
        ReasoningField::Omit => [
            ReasoningField::Omit,
            ReasoningField::ReasoningEffort,
            ReasoningField::ReasoningObject,
        ],
    }
}

fn temperature_field_progression(default_field: TemperatureField) -> [TemperatureField; 2] {
    match default_field {
        TemperatureField::Include => [TemperatureField::Include, TemperatureField::Omit],
        TemperatureField::Omit => [TemperatureField::Omit, TemperatureField::Omit],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TokenLimitField {
    MaxOutputTokens,
    MaxCompletionTokens,
    MaxTokens,
    Omit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReasoningField {
    ReasoningEffort,
    ReasoningObject,
    Omit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TemperatureField {
    Include,
    Omit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CompletionPayloadMode {
    pub(super) token_field: TokenLimitField,
    pub(super) reasoning_field: ReasoningField,
    pub(super) temperature_field: TemperatureField,
}

impl CompletionPayloadMode {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn default_for(provider: &ProviderConfig) -> Self {
        let runtime_contract = provider_runtime_contract(provider);
        Self::default_for_contract(provider, runtime_contract)
    }

    pub(super) fn default_for_contract(
        provider: &ProviderConfig,
        runtime_contract: ProviderRuntimeContract,
    ) -> Self {
        let token_field = if provider.max_tokens.is_some() {
            runtime_contract.default_token_field
        } else {
            TokenLimitField::Omit
        };

        let reasoning_field = if provider.reasoning_effort.is_some() {
            runtime_contract.default_reasoning_field
        } else {
            ReasoningField::Omit
        };

        Self {
            token_field,
            reasoning_field,
            temperature_field: runtime_contract.default_temperature_field,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub(super) struct ProviderApiError {
    pub(super) code: Option<String>,
    pub(super) param: Option<String>,
    pub(super) message: Option<String>,
}

pub(super) fn parse_provider_api_error(body: &Value) -> ProviderApiError {
    let error = provider_error_value(body);
    ProviderApiError {
        code: extract_error_string(error, &["code", "Code", "type", "Type", "CodeN"])
            .map(|value| normalize_error_token(value.as_str())),
        param: extract_error_string(
            error,
            &["param", "Param", "parameter", "Parameter", "field", "Field"],
        )
        .map(|value| normalize_error_token(value.as_str())),
        message: extract_error_message(error).map(|value| value.to_ascii_lowercase()),
    }
}

fn provider_error_value(body: &Value) -> &Value {
    body.get("error")
        .or_else(|| body.get("Error"))
        .or_else(|| {
            body.get("ResponseMetadata")
                .and_then(|metadata| metadata.get("Error"))
        })
        .or_else(|| {
            body.get("response_metadata")
                .and_then(|metadata| metadata.get("error"))
        })
        .unwrap_or(body)
}

fn extract_error_string(error: &Value, field_names: &[&str]) -> Option<String> {
    field_names.iter().find_map(|field| {
        error
            .get(*field)
            .and_then(value_to_trimmed_string)
            .or_else(|| {
                error
                    .get("details")
                    .and_then(Value::as_array)
                    .and_then(|details| {
                        details
                            .iter()
                            .find_map(|detail| detail.get(*field).and_then(value_to_trimmed_string))
                    })
            })
    })
}

fn extract_error_message(error: &Value) -> Option<String> {
    extract_error_string(error, &["message", "Message", "detail", "Detail"])
        .or_else(|| error.get("raw_body").and_then(value_to_trimmed_string))
        .or_else(|| {
            error
                .get("details")
                .and_then(Value::as_array)
                .and_then(|details| {
                    details
                        .iter()
                        .filter_map(|detail| {
                            detail.get("message").and_then(value_to_trimmed_string)
                        })
                        .find(|message| !message.is_empty())
                })
        })
}

fn value_to_trimmed_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        }
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(boolean) => Some(boolean.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

fn normalize_error_token(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace([' ', '-'], "_")
}

fn is_parameter_unsupported_with_fragments(
    error: &ProviderApiError,
    parameter: &str,
    unsupported_fragments: &[&str],
) -> bool {
    let param = parameter.to_lowercase();
    if error.param.as_deref() == Some(param.as_str()) {
        return true;
    }

    let message = error.message.as_deref().unwrap_or_default();
    if !message.contains_any_fragment(unsupported_fragments) {
        return false;
    }
    message.contains(param.as_str())
        || message.contains(param.replace('_', "-").as_str())
        || message.contains(param.replace('_', " ").as_str())
}

pub(super) fn should_disable_tool_schema_for_error(
    error: &ProviderApiError,
    runtime_contract: ProviderRuntimeContract,
) -> bool {
    let error_contract = runtime_contract.error_classification;
    if error_contract
        .tool_schema_error_parameters
        .iter()
        .any(|parameter| {
            is_parameter_unsupported_with_fragments(
                error,
                parameter,
                error_contract.unsupported_parameter_message_fragments,
            )
        })
    {
        return true;
    }

    let message = error.message.as_deref().unwrap_or_default();
    message.contains_any_fragment(error_contract.tool_schema_error_message_fragments)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub(super) enum PayloadAdaptationAxis {
    TokenField,
    ReasoningField,
    TemperatureField,
}

pub(super) fn classify_payload_adaptation_axis(
    error: &ProviderApiError,
    contract: &ProviderPayloadAdaptationContract,
) -> Option<PayloadAdaptationAxis> {
    if contract.token_error_parameters.iter().any(|parameter| {
        is_parameter_unsupported_with_fragments(
            error,
            parameter,
            contract.unsupported_parameter_message_fragments,
        )
    }) {
        return Some(PayloadAdaptationAxis::TokenField);
    }

    if contract.reasoning_error_parameters.iter().any(|parameter| {
        is_parameter_unsupported_with_fragments(
            error,
            parameter,
            contract.unsupported_parameter_message_fragments,
        )
    }) {
        return Some(PayloadAdaptationAxis::ReasoningField);
    }

    let temperature_rejected = contract
        .temperature_error_parameters
        .iter()
        .any(|parameter| {
            is_parameter_unsupported_with_fragments(
                error,
                parameter,
                contract.unsupported_parameter_message_fragments,
            )
        })
        || (error
            .param
            .as_deref()
            .is_some_and(|param| contract.temperature_error_parameters.contains(&param))
            && error
                .message
                .as_deref()
                .unwrap_or_default()
                .contains_any_fragment(contract.temperature_default_only_fragments));
    if temperature_rejected {
        return Some(PayloadAdaptationAxis::TemperatureField);
    }

    None
}

fn next_progressive_value<T: Copy + Eq>(progression: &[T], current: T) -> Option<T> {
    let current_index = progression.iter().position(|value| *value == current)?;
    progression
        .iter()
        .skip(current_index + 1)
        .copied()
        .find(|candidate| *candidate != current)
}

trait ContainsAnyFragment {
    fn contains_any_fragment(&self, fragments: &[&str]) -> bool;
}

impl ContainsAnyFragment for str {
    fn contains_any_fragment(&self, fragments: &[&str]) -> bool {
        fragments.iter().any(|fragment| self.contains(fragment))
    }
}

pub(super) fn should_try_next_model_on_error(
    error: &ProviderApiError,
    runtime_contract: ProviderRuntimeContract,
) -> bool {
    let error_contract = runtime_contract.error_classification;
    if let Some(code) = error.code.as_deref()
        && error_contract.model_not_found_codes.contains(&code)
    {
        return true;
    }
    if error
        .param
        .as_deref()
        .is_some_and(|param| error_contract.model_error_parameters.contains(&param))
    {
        return true;
    }

    let message = error.message.as_deref().unwrap_or_default();
    message.contains_any_fragment(error_contract.model_mismatch_message_fragments)
}

pub(super) fn adapt_payload_mode_for_error(
    current: CompletionPayloadMode,
    provider: &ProviderConfig,
    runtime_contract: ProviderRuntimeContract,
    error: &ProviderApiError,
) -> Option<CompletionPayloadMode> {
    let axis = classify_payload_adaptation_axis(error, &runtime_contract.payload_adaptation)?;
    if matches!(axis, PayloadAdaptationAxis::TokenField) && provider.max_tokens.is_some() {
        next_progressive_value(
            &runtime_contract.payload_adaptation.token_field_progression,
            current.token_field,
        )
        .map(|token_field| CompletionPayloadMode {
            token_field,
            ..current
        })
    } else if matches!(axis, PayloadAdaptationAxis::ReasoningField)
        && provider.reasoning_effort.is_some()
    {
        next_progressive_value(
            &runtime_contract
                .payload_adaptation
                .reasoning_field_progression,
            current.reasoning_field,
        )
        .map(|reasoning_field| CompletionPayloadMode {
            reasoning_field,
            ..current
        })
    } else if matches!(axis, PayloadAdaptationAxis::TemperatureField) {
        next_progressive_value(
            &runtime_contract
                .payload_adaptation
                .temperature_field_progression,
            current.temperature_field,
        )
        .map(|temperature_field| CompletionPayloadMode {
            temperature_field,
            ..current
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        ProviderConfig, ProviderKind, ProviderReasoningExtraBodyModeConfig,
        ProviderToolSchemaModeConfig,
    };
    use serde_json::json;

    #[test]
    fn capability_contract_matrix_is_provider_scoped() {
        let mut providers_with_reasoning_extra_body = Vec::new();
        for provider_kind in ProviderKind::all_sorted() {
            let provider = ProviderConfig {
                kind: *provider_kind,
                ..ProviderConfig::default()
            };
            let contract = provider_runtime_contract(&provider);
            assert!(
                contract.capability.turn_tool_schema_enabled(),
                "tool schema should be enabled by default for provider kind `{}`",
                provider_kind.as_str()
            );
            assert!(
                contract.capability.tool_schema_downgrade_on_unsupported(),
                "tool schema downgrade should stay enabled for provider kind `{}`",
                provider_kind.as_str()
            );
            if contract.capability.include_reasoning_extra_body() {
                providers_with_reasoning_extra_body.push(provider_kind.as_str());
            }
        }

        assert_eq!(providers_with_reasoning_extra_body, ["kimi_coding"]);
    }

    #[test]
    fn capability_contract_honors_explicit_config_overrides() {
        let disabled_tool_schema = provider_runtime_contract(&ProviderConfig {
            kind: ProviderKind::Openai,
            tool_schema_mode: ProviderToolSchemaModeConfig::Disabled,
            ..ProviderConfig::default()
        });
        assert_eq!(
            disabled_tool_schema.capability.tool_schema_mode,
            ProviderToolSchemaMode::Disabled
        );
        assert!(!disabled_tool_schema.capability.turn_tool_schema_enabled());
        assert!(
            !disabled_tool_schema
                .capability
                .tool_schema_downgrade_on_unsupported()
        );

        let strict_tool_schema = provider_runtime_contract(&ProviderConfig {
            kind: ProviderKind::Openai,
            tool_schema_mode: ProviderToolSchemaModeConfig::EnabledStrict,
            ..ProviderConfig::default()
        });
        assert_eq!(
            strict_tool_schema.capability.tool_schema_mode,
            ProviderToolSchemaMode::EnabledStrict
        );
        assert!(strict_tool_schema.capability.turn_tool_schema_enabled());
        assert!(
            !strict_tool_schema
                .capability
                .tool_schema_downgrade_on_unsupported()
        );

        let forced_kimi_reasoning = provider_runtime_contract(&ProviderConfig {
            kind: ProviderKind::Openai,
            reasoning_extra_body_mode: ProviderReasoningExtraBodyModeConfig::KimiThinking,
            ..ProviderConfig::default()
        });
        assert_eq!(
            forced_kimi_reasoning.capability.reasoning_extra_body_mode,
            ProviderReasoningExtraBodyMode::KimiThinking
        );
        assert!(
            forced_kimi_reasoning
                .capability
                .include_reasoning_extra_body()
        );
    }

    #[test]
    fn tool_schema_error_classifier_handles_mixed_signals() {
        let runtime_contract = provider_runtime_contract(&ProviderConfig::default());

        let cases = vec![
            (
                "parameter only",
                ProviderApiError {
                    param: Some("tools".to_owned()),
                    ..ProviderApiError::default()
                },
                true,
            ),
            (
                "unsupported message for unrelated parameter",
                ProviderApiError {
                    param: Some("temperature".to_owned()),
                    message: Some("unsupported parameter: temperature".to_owned()),
                    ..ProviderApiError::default()
                },
                false,
            ),
            (
                "message fragment",
                ProviderApiError {
                    message: Some("function calling is not supported for this model".to_owned()),
                    ..ProviderApiError::default()
                },
                true,
            ),
            (
                "negative control",
                ProviderApiError {
                    message: Some("tooling is available and enabled".to_owned()),
                    ..ProviderApiError::default()
                },
                false,
            ),
        ];

        for (name, error, expected) in cases {
            assert_eq!(
                should_disable_tool_schema_for_error(&error, runtime_contract),
                expected,
                "unexpected tool-schema classification for case `{name}` with error={error:?}",
            );
        }
    }

    #[test]
    fn model_switch_classifier_handles_mixed_signals() {
        let runtime_contract = provider_runtime_contract(&ProviderConfig::default());

        let cases = vec![
            (
                "model parameter",
                ProviderApiError {
                    param: Some("model".to_owned()),
                    ..ProviderApiError::default()
                },
                true,
            ),
            (
                "model code",
                ProviderApiError {
                    code: Some("model_not_found".to_owned()),
                    ..ProviderApiError::default()
                },
                true,
            ),
            (
                "message fragment",
                ProviderApiError {
                    message: Some("this endpoint only supports /v1/responses".to_owned()),
                    ..ProviderApiError::default()
                },
                true,
            ),
            (
                "negative control",
                ProviderApiError {
                    code: Some("invalid_request".to_owned()),
                    param: Some("temperature".to_owned()),
                    message: Some("temporary upstream issue".to_owned()),
                },
                false,
            ),
        ];

        for (name, error, expected) in cases {
            assert_eq!(
                should_try_next_model_on_error(&error, runtime_contract),
                expected,
                "unexpected model-switch classification for case `{name}` with error={error:?}",
            );
        }
    }

    #[test]
    fn payload_adaptation_axis_classifier_handles_mixed_signal_matrix() {
        let runtime_contract = provider_runtime_contract(&ProviderConfig::default());
        let contract = runtime_contract.payload_adaptation;
        let cases = vec![
            (
                "token parameter",
                ProviderApiError {
                    param: Some("max_tokens".to_owned()),
                    ..ProviderApiError::default()
                },
                Some(PayloadAdaptationAxis::TokenField),
            ),
            (
                "reasoning via unsupported message",
                ProviderApiError {
                    message: Some("unsupported parameter: reasoning".to_owned()),
                    ..ProviderApiError::default()
                },
                Some(PayloadAdaptationAxis::ReasoningField),
            ),
            (
                "temperature default-only message",
                ProviderApiError {
                    param: Some("temperature".to_owned()),
                    message: Some("only the default value is supported".to_owned()),
                    ..ProviderApiError::default()
                },
                Some(PayloadAdaptationAxis::TemperatureField),
            ),
            (
                "unrelated unsupported parameter",
                ProviderApiError {
                    param: Some("tools".to_owned()),
                    message: Some("unsupported parameter: tools".to_owned()),
                    ..ProviderApiError::default()
                },
                None,
            ),
        ];

        for (name, error, expected) in cases {
            assert_eq!(
                classify_payload_adaptation_axis(&error, &contract),
                expected,
                "unexpected payload adaptation axis for case `{name}` with error={error:?}",
            );
        }
    }

    #[test]
    fn parse_provider_api_error_uses_raw_body_when_structured_message_is_absent() {
        let parsed = parse_provider_api_error(&json!({
            "raw_body": "error code: 502"
        }));

        assert_eq!(parsed.message.as_deref(), Some("error code: 502"));
        assert_eq!(parsed.code, None);
        assert_eq!(parsed.param, None);
    }
}
