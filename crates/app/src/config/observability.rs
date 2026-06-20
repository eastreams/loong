use serde::{Deserialize, Serialize};

const OTEL_CAPTURE_CONTENT_ENV: &str = "LOONG_OTEL_CAPTURE_CONTENT";

/// Runtime observability knobs for tracing and diagnostics.
///
/// Call path for content capture:
///
/// - `LoongConfig.observability` is deserialized from `[observability]` in `loong.toml`.
/// - Provider streaming turns copy `capture_content` into
///   `StreamingModelRequestRuntime.capture_content` before entering the request executor.
/// - Tool execution receives this config as an explicit app-layer parameter at
///   the span construction boundary.
/// - The actual span creation goes through `crate::otel`, whose implementation is
///   feature-gated by `observability-otel`.
///
/// Keeping the config struct available even when `observability-otel` is disabled
/// keeps the TOML schema stable; requested content capture is warned about and
/// forced off in that build.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct ObservabilityConfig {
    /// Include raw provider/tool content in OpenTelemetry spans.
    ///
    /// Provider request/response messages use this value directly through
    /// `StreamingModelRequestRuntime.capture_content`; tool payload capture receives
    /// the same top-level config as an explicit app-layer parameter. This can expose
    /// prompts, user messages, tool results, and other sensitive payloads to the
    /// configured trace exporter/storage. Keep it disabled unless that pipeline is
    /// trusted. In builds without the `observability-otel` cargo feature, parsed
    /// runtime config forces this field to `false` after warning.
    #[serde(default)]
    pub capture_content: bool,
}

impl<'de> Deserialize<'de> for ObservabilityConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize, Default)]
        struct RawObservabilityConfig {
            #[serde(default)]
            capture_content: bool,
        }

        let raw = RawObservabilityConfig::deserialize(deserializer)?;
        Ok(Self::from_configured_capture_content(raw.capture_content))
    }
}

impl ObservabilityConfig {
    /// Runtime default used when config parsing omits the `[observability]` table.
    ///
    /// `Default` remains the schema default and does not read process environment;
    /// runtime construction does, preserving the legacy `LOONG_OTEL_CAPTURE_CONTENT`
    /// fallback without leaking env state into config templates.
    pub(crate) fn runtime_default() -> Self {
        Self::from_configured_capture_content(false)
    }

    /// Build a runtime-effective observability config from the configured value.
    ///
    /// After this constructor or TOML deserialization, callers can read
    /// `capture_content` directly. OTel-disabled builds force the value to `false`
    /// and emit a warning when TOML or the legacy environment variable requested
    /// content capture.
    pub(crate) fn from_configured_capture_content(configured: bool) -> Self {
        let env_requested = legacy_capture_content_env_enabled();
        #[cfg(feature = "observability-otel")]
        {
            Self {
                capture_content: configured || env_requested,
            }
        }
        #[cfg(not(feature = "observability-otel"))]
        {
            if configured || env_requested {
                tracing::warn!(
                    target: "loong.config",
                    configured,
                    env_requested,
                    "observability content capture was requested, but the observability-otel cargo feature is disabled; capture_content is forced to false"
                );
            }
            Self {
                capture_content: false,
            }
        }
    }
}

fn legacy_capture_content_env_enabled() -> bool {
    std::env::var(OTEL_CAPTURE_CONTENT_ENV)
        .is_ok_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true"))
}

pub(crate) fn template_usage_comment() -> &'static str {
    "# Observability notes:\n\
# - `[observability].capture_content = true` records raw provider messages and tool payloads on OpenTelemetry spans.\n\
# - Captured content can include prompts, user messages, tool results, and other sensitive payloads.\n\
# - Enable only for trusted local/debug trace pipelines; requires the `observability-otel` cargo feature.\n\
# - The legacy `LOONG_OTEL_CAPTURE_CONTENT=1|true` env var also enables capture when OpenTelemetry support is compiled in.\n\
\n"
}
