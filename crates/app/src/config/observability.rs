use serde::{Deserialize, Serialize};

pub const OTEL_CAPTURE_CONTENT_ENV: &str = "LOONG_OTEL_CAPTURE_CONTENT";

/// Runtime observability knobs for tracing and diagnostics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ObservabilityConfig {
    /// Include raw provider message content in OpenTelemetry spans.
    ///
    /// This can expose prompts, user messages, tool results, and other sensitive
    /// payloads to the configured trace exporter/storage. Keep it disabled unless
    /// that pipeline is trusted.
    #[serde(default)]
    pub capture_content: bool,
}

impl ObservabilityConfig {
    /// Resolve the effective content-capture setting.
    ///
    /// `LOONG_OTEL_CAPTURE_CONTENT=1|true` remains supported for compatibility
    /// with the previous environment-only switch.
    pub fn capture_content_enabled(&self) -> bool {
        self.capture_content
            || std::env::var(OTEL_CAPTURE_CONTENT_ENV).is_ok_and(|value| {
                matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true")
            })
    }
}

pub(crate) fn template_usage_comment() -> &'static str {
    "# Observability notes:\n\
# - `[observability].capture_content = true` records raw provider messages on OpenTelemetry spans.\n\
# - Captured content can include prompts, user messages, tool results, and other sensitive payloads.\n\
# - Enable only for trusted local/debug trace pipelines; `LOONG_OTEL_CAPTURE_CONTENT=1|true` still enables the same behavior.\n\
\n"
}
