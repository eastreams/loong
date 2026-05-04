use std::collections::BTreeMap;
use std::fmt::{self, Debug};
use std::io::{self, IsTerminal, Write};

use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_log::NormalizeEvent;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::field::RecordFields;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::fmt::format::{FormatFields, Writer};
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormattedFields};
use tracing_subscriber::registry::{LookupSpan, SpanRef};
use tracing_subscriber::util::SubscriberInitExt;

const DEFAULT_LOG_FILTER: &str = "warn";
const MAX_ERROR_CHARS: usize = 240;
const PAYLOAD_KEYS_FIELD: &str = "payload_keys";
const PAYLOAD_KEYS_JSON_FIELD: &str = "payload_keys_json";

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

fn resolved_log_directive(loong_log: Option<&str>, rust_log: Option<&str>) -> String {
    loong_log
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| rust_log.map(str::trim).filter(|value| !value.is_empty()))
        .unwrap_or(DEFAULT_LOG_FILTER)
        .to_owned()
}

fn build_env_filter(raw: &str) -> EnvFilter {
    EnvFilter::try_new(raw).unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_FILTER))
}

pub fn summarize_error(error: &str) -> String {
    let compact = error.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= MAX_ERROR_CHARS {
        return compact;
    }

    let visible_chars = MAX_ERROR_CHARS.saturating_sub(3);
    let truncated = compact.chars().take(visible_chars).collect::<String>();
    format!("{truncated}...")
}

pub fn debug_variant_name(value: &impl Debug) -> String {
    let rendered = format!("{value:?}");
    let variant_end = rendered
        .find(|character: char| character.is_ascii_whitespace() || character == '{')
        .or_else(|| rendered.find('('))
        .unwrap_or(rendered.len());
    rendered[..variant_end].to_owned()
}

pub fn init_tracing() {
    let log_format = LogFormat::parse(std::env::var("LOONG_LOG_FORMAT").ok().as_deref());
    let directive = resolved_log_directive(
        std::env::var("LOONG_LOG").ok().as_deref(),
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

    let init_result = match log_format {
        LogFormat::Compact => base.compact().finish().try_init(),
        LogFormat::Pretty => base.pretty().finish().try_init(),
        LogFormat::Json => base
            .event_format(LoongJsonEventFormat)
            .fmt_fields(LoongJsonFields)
            .finish()
            .try_init(),
    };

    if let Err(error) = init_result {
        let mut stderr = io::stderr();
        let _ = writeln!(stderr, "loong.daemon tracing init failed: {error}");
    }
}

#[derive(Debug, Default)]
struct LoongJsonFields;

impl<'writer> FormatFields<'writer> for LoongJsonFields {
    fn format_fields<R: RecordFields>(
        &self,
        mut writer: Writer<'writer>,
        fields: R,
    ) -> fmt::Result {
        let mut recorder = LoongJsonRecorder::default();
        fields.record(&mut recorder);
        write_json_map(&mut writer, recorder.into_values())
    }

    fn add_fields(
        &self,
        current: &'writer mut FormattedFields<Self>,
        fields: &tracing::span::Record<'_>,
    ) -> fmt::Result {
        if current.is_empty() {
            let mut writer = current.as_writer();
            let mut recorder = LoongJsonRecorder::default();
            fields.record(&mut recorder);
            write_json_map(&mut writer, recorder.into_values())?;
            return Ok(());
        }

        let mut recorder = LoongJsonRecorder::with_values(
            serde_json::from_str(&current.fields).map_err(|_error| fmt::Error)?,
        );
        fields.record(&mut recorder);
        current.fields = encode_json_map(recorder.into_values())?;

        Ok(())
    }
}

#[derive(Debug, Default)]
struct LoongJsonEventFormat;

impl<S, N> FormatEvent<S, N> for LoongJsonEventFormat
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let normalized_meta = event.normalized_metadata();
        let meta = normalized_meta.as_ref().unwrap_or_else(|| event.metadata());
        let mut object = Map::new();
        object.insert("timestamp".to_owned(), Value::String(utc_timestamp()));
        object.insert("level".to_owned(), Value::String(meta.level().to_string()));

        let mut recorder = LoongJsonRecorder::default();
        event.record(&mut recorder);
        extend_json_object(&mut object, recorder.into_values());

        object.insert("target".to_owned(), Value::String(meta.target().to_owned()));
        record_current_span_context(ctx, event, &mut object);

        let encoded = serde_json::to_string(&Value::Object(object)).map_err(|_error| fmt::Error)?;
        writer.write_str(&encoded)?;
        writer.write_char('\n')
    }
}

#[derive(Debug, Default)]
struct LoongJsonRecorder {
    values: BTreeMap<String, Value>,
}

impl LoongJsonRecorder {
    fn with_values(values: BTreeMap<String, Value>) -> Self {
        Self { values }
    }

    fn into_values(self) -> BTreeMap<String, Value> {
        self.values
    }

    fn insert_value(&mut self, field: &Field, value: Value) {
        self.values
            .insert(json_field_name(field.name()).to_owned(), value);
    }

    fn insert_string_or_structured_json(&mut self, field: &Field, value: &str) {
        let field_name = json_field_name(field.name());
        match structured_json_field_value(field_name, value) {
            Some((output_field, value)) => {
                self.values.insert(output_field.to_owned(), value);
            }
            None => {
                self.values
                    .insert(field_name.to_owned(), Value::String(value.to_owned()));
            }
        }
    }
}

impl Visit for LoongJsonRecorder {
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.insert_value(field, Value::from(value));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.insert_value(field, Value::from(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.insert_value(field, Value::from(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.insert_value(field, Value::from(value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.insert_string_or_structured_json(field, value);
    }

    fn record_bytes(&mut self, field: &Field, value: &[u8]) {
        let bytes = value
            .iter()
            .map(|byte| Value::from(u64::from(*byte)))
            .collect::<Vec<_>>();
        self.insert_value(field, Value::Array(bytes));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name().starts_with("log.") {
            return;
        }
        self.insert_string_or_structured_json(field, &format!("{value:?}"));
    }
}

fn write_json_map(writer: &mut dyn fmt::Write, values: BTreeMap<String, Value>) -> fmt::Result {
    let encoded = encode_json_map(values)?;
    writer.write_str(&encoded)
}

fn encode_json_map(values: BTreeMap<String, Value>) -> Result<String, fmt::Error> {
    serde_json::to_string(&values).map_err(|_error| fmt::Error)
}

fn extend_json_object(object: &mut Map<String, Value>, values: BTreeMap<String, Value>) {
    for (field, value) in values {
        object.insert(field, value);
    }
}

fn record_current_span_context<S, N>(
    ctx: &FmtContext<'_, S, N>,
    event: &Event<'_>,
    object: &mut Map<String, Value>,
) where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    let Some(current_span) = event
        .parent()
        .and_then(|id| ctx.span(id))
        .or_else(|| ctx.lookup_current())
    else {
        return;
    };

    let spans = current_span
        .scope()
        .from_root()
        .map(span_json::<S, N>)
        .collect::<Vec<_>>();

    if let Some(current) = spans.last() {
        object.insert("span".to_owned(), current.clone());
    }
    object.insert("spans".to_owned(), Value::Array(spans));
}

fn span_json<S, N>(span: SpanRef<'_, S>) -> Value
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    let mut object = Map::new();
    let extensions = span.extensions();
    if let Some(fields) = extensions.get::<FormattedFields<N>>()
        && let Ok(Value::Object(recorded)) = serde_json::from_str::<Value>(&fields.fields)
    {
        for (field, value) in recorded {
            object.insert(field, value);
        }
    }
    object.insert(
        "name".to_owned(),
        Value::String(span.metadata().name().to_owned()),
    );
    Value::Object(object)
}

fn utc_timestamp() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_error| "1970-01-01T00:00:00Z".to_owned())
}

fn json_field_name(name: &str) -> &str {
    match name.strip_prefix("r#") {
        Some(stripped) => stripped,
        None => name,
    }
}

fn structured_json_field_value(field_name: &str, value: &str) -> Option<(&'static str, Value)> {
    if field_name != PAYLOAD_KEYS_JSON_FIELD {
        return None;
    }

    let parsed = serde_json::from_str::<Value>(value).ok()?;
    parsed.is_array().then_some((PAYLOAD_KEYS_FIELD, parsed))
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::{Arc, Mutex};

    use serde_json::{Value, json};
    use tracing_subscriber::fmt::MakeWriter;
    use tracing_subscriber::fmt::format::FmtSpan;

    use super::{
        LogFormat, LoongJsonEventFormat, LoongJsonFields, build_env_filter, debug_variant_name,
        resolved_log_directive, summarize_error,
    };
    use crate::Commands;

    #[test]
    fn resolved_log_directive_prefers_loong_log() {
        assert_eq!(
            resolved_log_directive(Some("loong_app=debug"), Some("warn")),
            "loong_app=debug"
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
    fn json_logs_emit_payload_keys_array_and_duration_number() {
        let event = capture_json_event("loong.tools=debug", || {
            let payload_keys =
                serde_json::to_string(&["path".to_owned()]).expect("keys should encode");
            tracing::debug!(
                target: "loong.tools",
                payload_keys_json = payload_keys.as_str(),
                duration_ms = 7_u64,
                "tool execution completed"
            );
        });

        assert_eq!(event.get("payload_keys"), Some(&json!(["path"])));
        assert!(event.get("payload_keys_json").is_none());
        assert_eq!(event.get("duration_ms"), Some(&json!(7)));
    }

    #[test]
    fn json_logs_do_not_coerce_public_payload_keys_strings() {
        let event = capture_json_event("info", || {
            tracing::info!(
                target: "loong.test",
                payload_keys = "[\"literal\"]",
                "ordinary event"
            );
        });

        assert_eq!(event.get("payload_keys"), Some(&json!("[\"literal\"]")));
    }

    #[test]
    fn json_logs_preserve_log_backed_target_metadata() {
        let event = capture_json_event("info", || {
            let record = tracing_log::log::Record::builder()
                .args(format_args!("from log facade"))
                .level(tracing_log::log::Level::Info)
                .target("dependency.crate")
                .file(Some("dependency.rs"))
                .line(Some(42))
                .module_path(Some("dependency::module"))
                .build();
            tracing_log::format_trace(&record).expect("log record should dispatch");
        });

        assert_eq!(event.get("target"), Some(&json!("dependency.crate")));
        assert_eq!(event.get("message"), Some(&json!("from log facade")));
    }

    #[test]
    fn json_logs_preserve_span_context() {
        let event = capture_json_event("loong.test=info", || {
            let span = tracing::info_span!(target: "loong.test", "operation", request_id = 42_u64);
            let _guard = span.enter();
            tracing::info!(target: "loong.test", "inside span");
        });

        assert_eq!(event.pointer("/span/name"), Some(&json!("operation")));
        assert_eq!(event.pointer("/span/request_id"), Some(&json!(42)));
        assert_eq!(event.pointer("/spans/0/name"), Some(&json!("operation")));
        assert_eq!(event.pointer("/spans/0/request_id"), Some(&json!(42)));
    }

    #[test]
    fn json_logs_emit_close_span_events_as_json() {
        let events = capture_json_events("loong.test=info", || {
            let span = tracing::info_span!(target: "loong.test", "operation", request_id = 42_u64);
            let _guard = span.enter();
        });

        assert!(
            events
                .iter()
                .any(|event| event.get("message") == Some(&json!("close")))
        );
    }

    #[test]
    fn build_env_filter_falls_back_on_invalid_directive() {
        let filter = build_env_filter("[broken");
        let rendered = filter.to_string();
        assert_eq!(rendered, "warn");
    }

    #[test]
    fn summarize_error_collapses_whitespace_and_truncates() {
        let repeated = "detail ".repeat(64);
        let summary = summarize_error(&format!("line one\nline two\t{repeated}"));

        assert!(!summary.contains('\n'));
        assert!(!summary.contains('\t'));
        assert!(summary.ends_with("..."));
        assert!(summary.chars().count() <= 240);
    }

    #[test]
    fn debug_variant_name_keeps_only_variant_identity() {
        let welcome = debug_variant_name(&Commands::Welcome);
        let turn_run = debug_variant_name(&Commands::Turn {
            command: crate::TurnCommands::Run {
                config: None,
                session: None,
                message: "ship".to_owned(),
                acp: false,
                acp_event_stream: false,
                acp_bootstrap_mcp_server: Vec::new(),
                acp_cwd: None,
            },
        });

        assert_eq!(welcome, "Welcome");
        assert_eq!(turn_run, "Turn");
    }

    fn capture_json_event(filter: &str, emit: impl FnOnce()) -> Value {
        capture_json_events(filter, emit)
            .into_iter()
            .next()
            .expect("captured log line")
    }

    fn capture_json_events(filter: &str, emit: impl FnOnce()) -> Vec<Value> {
        let logs = CapturedLogs::default();
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(logs.clone())
            .with_target(true)
            .with_span_events(FmtSpan::CLOSE)
            .with_ansi(false)
            .event_format(LoongJsonEventFormat)
            .fmt_fields(LoongJsonFields)
            .finish();

        tracing::subscriber::with_default(subscriber, emit);

        let output = logs.output();
        output
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("log line should be JSON"))
            .collect()
    }

    #[derive(Clone, Default)]
    struct CapturedLogs {
        output: Arc<Mutex<Vec<u8>>>,
    }

    impl CapturedLogs {
        fn output(&self) -> String {
            let output = self.output.lock().expect("captured logs lock").clone();
            String::from_utf8(output).expect("captured logs should be UTF-8")
        }
    }

    impl<'writer> MakeWriter<'writer> for CapturedLogs {
        type Writer = CapturedLogWriter;

        fn make_writer(&'writer self) -> Self::Writer {
            CapturedLogWriter {
                output: Arc::clone(&self.output),
            }
        }
    }

    struct CapturedLogWriter {
        output: Arc<Mutex<Vec<u8>>>,
    }

    impl io::Write for CapturedLogWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.output.lock().expect("captured logs lock").extend(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}
