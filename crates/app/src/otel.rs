//! OpenTelemetry adapter used by provider, tool, and chat execution paths.
//!
//! This module is the only place in `loong-app` that imports the `opentelemetry`
//! crate. Callers always use the same helper types, and the helper type documents
//! which execution path owns each shape:
//!
//! - `OtelSpanHandle`: provider streaming request execution. The provider dispatch
//!   layer resolves `[observability].capture_content` before entering the executor;
//!   the executor keeps this local span handle and adds request/response attributes
//!   as streaming state accumulates.
//! - `OtelAttachedSpanHandle`: tool dispatch. The app layer passes
//!   `LoongConfig.observability` explicitly into the tool dispatch span boundary,
//!   and this attached handle keeps nested synchronous tool work under the current
//!   OTel context.
//! - `OtelContextSpanHandle` plus `spawn_with_context`: chat pending-turn execution.
//!   The TUI creates the parent span before `tokio::spawn`, clones the context
//!   handle into the task, and `spawn_with_context` propagates that context when
//!   OTel support is compiled in.
//!
//! With `observability-otel` enabled these helpers create real spans. Without it
//! they compile to no-op types with the same API, so call sites do not need local
//! feature gates and `opentelemetry` remains an optional dependency.

#[cfg(feature = "observability-otel")]
mod imp {
    use opentelemetry::context::FutureExt;
    use opentelemetry::global::{self, BoxedSpan};
    use opentelemetry::trace::{Span, SpanKind, TraceContextExt, Tracer, TracerProvider};
    use opentelemetry::{Context, KeyValue};

    /// Attribute values accepted by the OTel helper API.
    ///
    /// The wrapper keeps `opentelemetry::KeyValue` out of provider/tool/chat code
    /// while still preserving the concrete value types used by semantic attributes.
    #[derive(Debug, Clone)]
    pub(crate) enum OtelAttributeValue {
        Bool(bool),
        I64(i64),
        String(String),
        StaticStr(&'static str),
    }

    impl From<bool> for OtelAttributeValue {
        fn from(value: bool) -> Self {
            Self::Bool(value)
        }
    }

    impl From<i64> for OtelAttributeValue {
        fn from(value: i64) -> Self {
            Self::I64(value)
        }
    }

    impl From<String> for OtelAttributeValue {
        fn from(value: String) -> Self {
            Self::String(value)
        }
    }

    impl From<&'static str> for OtelAttributeValue {
        fn from(value: &'static str) -> Self {
            Self::StaticStr(value)
        }
    }

    impl OtelAttributeValue {
        fn into_key_value(self, key: &'static str) -> KeyValue {
            match self {
                Self::Bool(value) => KeyValue::new(key, value),
                Self::I64(value) => KeyValue::new(key, value),
                Self::String(value) => KeyValue::new(key, value),
                Self::StaticStr(value) => KeyValue::new(key, value),
            }
        }
    }

    /// Span kinds used by current Loong spans.
    ///
    /// The helper exposes only the kinds the app uses so callers do not import
    /// `opentelemetry::trace::SpanKind` directly.
    #[derive(Debug, Clone, Copy)]
    pub(crate) enum OtelSpanKind {
        Internal,
        Client,
    }

    impl From<OtelSpanKind> for SpanKind {
        fn from(value: OtelSpanKind) -> Self {
            match value {
                OtelSpanKind::Internal => Self::Internal,
                OtelSpanKind::Client => Self::Client,
            }
        }
    }

    /// Build a typed attribute pair for `Otel*SpanHandle::start`.
    ///
    /// Callers pass arrays of these pairs when creating spans. In OTel builds the
    /// values become real `KeyValue`s; in no-OTel builds the values are discarded.
    pub(crate) fn attr(
        key: &'static str,
        value: impl Into<OtelAttributeValue>,
    ) -> (&'static str, OtelAttributeValue) {
        (key, value.into())
    }

    /// Plain span handle used when context attachment/propagation is not needed.
    ///
    /// Provider request execution uses this form because the request executor keeps
    /// the span handle locally and sets attributes as streaming state accumulates.
    pub(crate) struct OtelSpanHandle {
        span: BoxedSpan,
    }

    impl OtelSpanHandle {
        pub(crate) fn start(
            name: impl Into<String>,
            kind: OtelSpanKind,
            attributes: impl IntoIterator<Item = (&'static str, OtelAttributeValue)>,
        ) -> Self {
            let tracer = global::tracer_provider().tracer("loong");
            let attributes = attributes
                .into_iter()
                .map(|(key, value)| value.into_key_value(key))
                .collect::<Vec<_>>();
            let span = tracer
                .span_builder(name.into())
                .with_kind(kind.into())
                .with_attributes(attributes)
                .start(&tracer);
            Self { span }
        }

        pub(crate) fn set_attribute(
            &mut self,
            key: &'static str,
            value: impl Into<OtelAttributeValue>,
        ) {
            self.span.set_attribute(value.into().into_key_value(key));
        }

        pub(crate) fn end(&mut self) {
            self.span.end();
        }
    }

    /// Context-carrying span used for async task propagation.
    ///
    /// Chat pending-turn execution clones this handle into the spawned task and
    /// passes it to `spawn_with_context`, preserving the parent span context across
    /// the `tokio::spawn` boundary when OTel is compiled in.
    #[derive(Clone)]
    pub(crate) struct OtelContextSpanHandle {
        context: Context,
    }

    impl OtelContextSpanHandle {
        pub(crate) fn start(
            name: impl Into<String>,
            kind: OtelSpanKind,
            attributes: impl IntoIterator<Item = (&'static str, OtelAttributeValue)>,
        ) -> Self {
            let tracer = global::tracer_provider().tracer("loong");
            let attributes = attributes
                .into_iter()
                .map(|(key, value)| value.into_key_value(key))
                .collect::<Vec<_>>();
            let span = tracer
                .span_builder(name.into())
                .with_kind(kind.into())
                .with_attributes(attributes)
                .start(&tracer);
            Self {
                context: Context::current().with_span(span),
            }
        }

        pub(crate) fn set_attribute(
            &self,
            key: &'static str,
            value: impl Into<OtelAttributeValue>,
        ) {
            self.context
                .span()
                .set_attribute(value.into().into_key_value(key));
        }

        pub(crate) fn end(&self) {
            self.context.span().end();
        }
    }

    /// Span handle that is attached to the current thread while it is alive.
    ///
    /// Tool dispatch uses this form so nested tool execution can observe the
    /// current span through OpenTelemetry context APIs. In no-OTel builds the same
    /// type exists but carries no state.
    pub(crate) struct OtelAttachedSpanHandle {
        context: Context,
        _guard: opentelemetry::context::ContextGuard,
    }

    impl OtelAttachedSpanHandle {
        pub(crate) fn start(
            name: impl Into<String>,
            kind: OtelSpanKind,
            attributes: impl IntoIterator<Item = (&'static str, OtelAttributeValue)>,
        ) -> Self {
            let span = OtelContextSpanHandle::start(name, kind, attributes);
            let guard = span.context.clone().attach();
            Self {
                context: span.context,
                _guard: guard,
            }
        }

        pub(crate) fn set_attribute(
            &self,
            key: &'static str,
            value: impl Into<OtelAttributeValue>,
        ) {
            self.context
                .span()
                .set_attribute(value.into().into_key_value(key));
        }

        pub(crate) fn end(&self) {
            self.context.span().end();
        }
    }

    /// Spawn a future with the provided OTel context when OTel is enabled.
    ///
    /// The no-OTel implementation falls back to plain `tokio::spawn`, preserving
    /// behavior while removing the optional dependency from that build.
    pub(crate) fn spawn_with_context<F>(
        span: &OtelContextSpanHandle,
        future: F,
    ) -> tokio::task::JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        tokio::spawn(future.with_context(span.context.clone()))
    }
}

#[cfg(not(feature = "observability-otel"))]
mod imp {
    //! No-op implementation used when `observability-otel` is disabled.
    //!
    //! The types mirror the real implementation so provider/tool/chat code compiles
    //! through the same calls, but all values are discarded and no OTel dependency is
    //! linked. Config loading also forces `ObservabilityConfig.capture_content` to
    //! `false` in this build, so payload serialization for content capture is skipped
    //! before reaching these no-op methods.

    #[derive(Debug, Clone)]
    pub(crate) enum OtelAttributeValue {
        Bool,
        I64,
        String,
        StaticStr,
    }

    impl From<bool> for OtelAttributeValue {
        fn from(_value: bool) -> Self {
            Self::Bool
        }
    }

    impl From<i64> for OtelAttributeValue {
        fn from(_value: i64) -> Self {
            Self::I64
        }
    }

    impl From<String> for OtelAttributeValue {
        fn from(_value: String) -> Self {
            Self::String
        }
    }

    impl From<&'static str> for OtelAttributeValue {
        fn from(_value: &'static str) -> Self {
            Self::StaticStr
        }
    }

    #[derive(Debug, Clone, Copy)]
    pub(crate) enum OtelSpanKind {
        Internal,
        Client,
    }

    pub(crate) fn attr(
        key: &'static str,
        value: impl Into<OtelAttributeValue>,
    ) -> (&'static str, OtelAttributeValue) {
        let _ = value.into();
        (key, OtelAttributeValue::StaticStr)
    }

    pub(crate) struct OtelSpanHandle;

    impl OtelSpanHandle {
        pub(crate) fn start(
            _name: impl Into<String>,
            _kind: OtelSpanKind,
            _attributes: impl IntoIterator<Item = (&'static str, OtelAttributeValue)>,
        ) -> Self {
            Self
        }

        pub(crate) fn set_attribute(
            &mut self,
            _key: &'static str,
            _value: impl Into<OtelAttributeValue>,
        ) {
        }

        pub(crate) fn end(&mut self) {}
    }

    #[derive(Clone)]
    pub(crate) struct OtelContextSpanHandle;

    impl OtelContextSpanHandle {
        pub(crate) fn start(
            _name: impl Into<String>,
            _kind: OtelSpanKind,
            _attributes: impl IntoIterator<Item = (&'static str, OtelAttributeValue)>,
        ) -> Self {
            Self
        }

        pub(crate) fn set_attribute(
            &self,
            _key: &'static str,
            _value: impl Into<OtelAttributeValue>,
        ) {
        }

        pub(crate) fn end(&self) {}
    }

    pub(crate) struct OtelAttachedSpanHandle;

    impl OtelAttachedSpanHandle {
        pub(crate) fn start(
            _name: impl Into<String>,
            _kind: OtelSpanKind,
            _attributes: impl IntoIterator<Item = (&'static str, OtelAttributeValue)>,
        ) -> Self {
            Self
        }

        pub(crate) fn set_attribute(
            &self,
            _key: &'static str,
            _value: impl Into<OtelAttributeValue>,
        ) {
        }

        pub(crate) fn end(&self) {}
    }

    pub(crate) fn spawn_with_context<F>(
        _span: &OtelContextSpanHandle,
        future: F,
    ) -> tokio::task::JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        tokio::spawn(future)
    }
}

pub(crate) use imp::{
    OtelAttachedSpanHandle, OtelContextSpanHandle, OtelSpanHandle, OtelSpanKind, attr,
    spawn_with_context,
};
