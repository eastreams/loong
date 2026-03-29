use std::io::{self, IsTerminal};

use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::util::SubscriberInitExt;

const DEFAULT_LOG_FILTER: &str = "warn";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogFormat {
    Compact,
    Pretty,
    Json,
}

impl LogFormat {
    fn parse(raw: Option<&str>) -> Self {
        match raw
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("compact")
            .to_ascii_lowercase()
            .as_str()
        {
            "pretty" => Self::Pretty,
            "json" => Self::Json,
            _ => Self::Compact,
        }
    }
}

fn resolved_log_directive(loongclaw_log: Option<&str>, rust_log: Option<&str>) -> String {
    loongclaw_log
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| rust_log.map(str::trim).filter(|value| !value.is_empty()))
        .unwrap_or(DEFAULT_LOG_FILTER)
        .to_owned()
}

fn build_env_filter(raw: &str) -> EnvFilter {
    EnvFilter::try_new(raw).unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_FILTER))
}

pub fn init_tracing() {
    let log_format = LogFormat::parse(std::env::var("LOONGCLAW_LOG_FORMAT").ok().as_deref());
    let directive = resolved_log_directive(
        std::env::var("LOONGCLAW_LOG").ok().as_deref(),
        std::env::var("RUST_LOG").ok().as_deref(),
    );
    let env_filter = build_env_filter(directive.as_str());
    let use_ansi = log_format != LogFormat::Json && io::stderr().is_terminal();
    let base = tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_writer(io::stderr)
        .with_target(true)
        .with_span_events(FmtSpan::CLOSE)
        .with_ansi(use_ansi);

    let _ = match log_format {
        LogFormat::Compact => base.compact().finish().try_init(),
        LogFormat::Pretty => base.pretty().finish().try_init(),
        LogFormat::Json => base.json().flatten_event(true).finish().try_init(),
    };
}

#[cfg(test)]
mod tests {
    use super::{LogFormat, build_env_filter, resolved_log_directive};

    #[test]
    fn resolved_log_directive_prefers_loongclaw_log() {
        assert_eq!(
            resolved_log_directive(Some("loongclaw_app=debug"), Some("warn")),
            "loongclaw_app=debug"
        );
    }

    #[test]
    fn resolved_log_directive_falls_back_to_rust_log_then_default() {
        assert_eq!(resolved_log_directive(None, Some("info")), "info");
        assert_eq!(resolved_log_directive(None, None), "warn");
    }

    #[test]
    fn parse_log_format_accepts_known_variants() {
        assert_eq!(LogFormat::parse(Some("pretty")), LogFormat::Pretty);
        assert_eq!(LogFormat::parse(Some("json")), LogFormat::Json);
        assert_eq!(LogFormat::parse(Some("compact")), LogFormat::Compact);
        assert_eq!(LogFormat::parse(Some("unknown")), LogFormat::Compact);
    }

    #[test]
    fn build_env_filter_falls_back_on_invalid_directive() {
        let filter = build_env_filter("[broken");
        let rendered = filter.to_string();
        assert_eq!(rendered, "warn");
    }
}
