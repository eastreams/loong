use std::collections::VecDeque;
use std::pin::Pin;
use std::str::from_utf8;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::Stream;
use serde_json::Value;

use super::transport_trait::TransportError;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub(super) enum SseLine {
    EventType { name: String },
    Data { content: String },
    Retry { timeout_ms: u64 },
    Comment,
    Empty,
}

pub(super) fn parse_sse_line(line: &str) -> SseLine {
    if line.is_empty() {
        return SseLine::Empty;
    }
    if line.starts_with(':') {
        return SseLine::Comment;
    }
    if let Some(rest) = line.strip_prefix("event:") {
        return SseLine::EventType {
            name: rest.trim().to_owned(),
        };
    }
    if let Some(rest) = line.strip_prefix("retry:") {
        return SseLine::Retry {
            timeout_ms: rest.trim().parse().unwrap_or(3000),
        };
    }
    if let Some(rest) = line.strip_prefix("data:") {
        return SseLine::Data {
            content: rest.trim().to_owned(),
        };
    }
    SseLine::Empty
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum SseStreamEvent {
    Message {
        data: Value,
        event_type: Option<String>,
    },
}

impl SseStreamEvent {
    pub(super) fn from_sse_lines(
        event_type: Option<String>,
        data_lines: &[String],
    ) -> Result<Option<Self>, serde_json::Error> {
        if data_lines.is_empty() {
            return Ok(None);
        }
        let combined = data_lines.join("\n");
        if combined.is_empty() {
            return Ok(None);
        }
        let parsed: Value = serde_json::from_str(&combined)?;
        Ok(Some(SseStreamEvent::Message {
            data: parsed,
            event_type,
        }))
    }
}

#[derive(Default)]
pub(super) struct SseDecoder {
    line_buffer: Vec<u8>,
    event_type: Option<String>,
    data_lines: Vec<String>,
}

impl SseDecoder {
    pub(super) fn push_chunk(&mut self, chunk: &[u8]) -> Result<Vec<Value>, TransportError> {
        let mut messages = Vec::new();
        for byte in chunk {
            if *byte == b'\n' {
                let line = std::mem::take(&mut self.line_buffer);
                self.push_line(line.as_slice(), &mut messages)?;
                continue;
            }
            if *byte != b'\r' {
                self.line_buffer.push(*byte);
            }
        }
        Ok(messages)
    }

    pub(super) fn finish(&mut self) -> Result<Vec<Value>, TransportError> {
        let mut messages = Vec::new();
        if !self.line_buffer.is_empty() {
            let line = std::mem::take(&mut self.line_buffer);
            self.push_line(line.as_slice(), &mut messages)?;
        }
        self.flush_event(&mut messages)?;
        Ok(messages)
    }

    fn push_line(&mut self, line: &[u8], messages: &mut Vec<Value>) -> Result<(), TransportError> {
        let line = from_utf8(line).map_err(|error| {
            TransportError::response_shape_invalid(format!("invalid UTF-8 in SSE stream: {error}"))
        })?;
        match parse_sse_line(line) {
            SseLine::EventType { name } => {
                self.event_type = Some(name);
            }
            SseLine::Data { content } => {
                self.data_lines.push(content);
            }
            SseLine::Retry { .. } | SseLine::Comment => {}
            SseLine::Empty => {
                self.flush_event(messages)?;
            }
        }
        Ok(())
    }

    fn flush_event(&mut self, messages: &mut Vec<Value>) -> Result<(), TransportError> {
        let event_type = self.event_type.take();
        let data_lines = std::mem::take(&mut self.data_lines);
        let Some(event) =
            SseStreamEvent::from_sse_lines(event_type, &data_lines).map_err(|error| {
                TransportError::response_shape_invalid(format!(
                    "streaming event parse failed: {error}"
                ))
            })?
        else {
            return Ok(());
        };
        match event {
            SseStreamEvent::Message { data, .. } => {
                messages.push(data);
            }
        }
        Ok(())
    }
}

pub(super) struct SseEventStream {
    byte_stream: Pin<Box<dyn Stream<Item = Result<Bytes, TransportError>> + Send>>,
    decoder: SseDecoder,
    pending: VecDeque<Result<Value, TransportError>>,
}

impl SseEventStream {
    pub(super) fn new(
        byte_stream: Pin<Box<dyn Stream<Item = Result<Bytes, TransportError>> + Send>>,
    ) -> Self {
        Self {
            byte_stream,
            decoder: SseDecoder::default(),
            pending: VecDeque::new(),
        }
    }
}

impl Stream for SseEventStream {
    type Item = Result<Value, TransportError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if let Some(item) = this.pending.pop_front() {
                return Poll::Ready(Some(item));
            }
            match this.byte_stream.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(bytes))) => match this.decoder.push_chunk(bytes.as_ref()) {
                    Ok(messages) => {
                        this.pending.extend(messages.into_iter().map(Ok));
                    }
                    Err(error) => {
                        this.pending.push_back(Err(error));
                    }
                },
                Poll::Ready(Some(Err(error))) => {
                    return Poll::Ready(Some(Err(error)));
                }
                Poll::Ready(None) => match this.decoder.finish() {
                    Ok(messages) => {
                        if messages.is_empty() {
                            return Poll::Ready(None);
                        }
                        this.pending.extend(messages.into_iter().map(Ok));
                    }
                    Err(error) => {
                        return Poll::Ready(Some(Err(error)));
                    }
                },
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::wildcard_enum_match_arm)]
    #[test]
    fn sse_line_parser_extracts_data_field() {
        let line = "data: {\"type\":\"content_block_delta\",\"text\":\"Hello\"}";
        let parsed = parse_sse_line(line);
        match parsed {
            SseLine::Data { content } => {
                assert_eq!(
                    content,
                    "{\"type\":\"content_block_delta\",\"text\":\"Hello\"}"
                );
            }
            other => {
                panic!("expected SseLine::Data, got {:?}", other)
            }
        }
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    #[test]
    fn sse_line_parser_extracts_event_type() {
        let line = "event: content_block_delta";
        let parsed = parse_sse_line(line);
        match parsed {
            SseLine::EventType { name } => {
                assert_eq!(name.as_str(), "content_block_delta");
            }
            other => {
                panic!("expected SseLine::EventType, got {:?}", other)
            }
        }
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    #[test]
    fn sse_line_parser_extracts_retry_field() {
        let line = "retry: 1000";
        let parsed = parse_sse_line(line);
        match parsed {
            SseLine::Retry { timeout_ms } => {
                assert_eq!(timeout_ms, 1000);
            }
            other => {
                panic!("expected SseLine::Retry, got {:?}", other)
            }
        }
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    #[test]
    fn sse_line_parser_handles_empty_line() {
        let parsed = parse_sse_line("");
        match parsed {
            SseLine::Empty => {}
            other => {
                panic!("expected SseLine::Empty, got {:?}", other)
            }
        }
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    #[test]
    fn sse_line_parser_handles_comment_line() {
        let parsed = parse_sse_line(": this is a comment");
        match parsed {
            SseLine::Comment => {}
            other => {
                panic!("expected SseLine::Comment, got {:?}", other)
            }
        }
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    #[test]
    fn sse_line_parser_data_field_without_json_value() {
        let line = "data:";
        let parsed = parse_sse_line(line);
        match parsed {
            SseLine::Data { content } => {
                assert_eq!(content, "");
            }
            other => {
                panic!("expected SseLine::Data, got {:?}", other)
            }
        }
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    #[test]
    fn sse_lines_accumulate_into_complete_event() {
        let event_type_line = parse_sse_line("event: content_block_delta");
        let data_line = parse_sse_line("data: {\"type\":\"text_delta\",\"text\":\"Hello\"}");

        let (event_type, data) = match (&event_type_line, &data_line) {
            (SseLine::EventType { name: event_type }, SseLine::Data { content }) => {
                (event_type.clone(), content.clone())
            }
            _ => panic!("expected EventType and Data"),
        };

        assert_eq!(event_type.as_str(), "content_block_delta");
        assert_eq!(data, "{\"type\":\"text_delta\",\"text\":\"Hello\"}");
    }

    #[test]
    fn sse_stream_event_from_lines_parses_json() {
        let event_type = Some("content_block_delta".to_owned());
        let data_lines = vec!["{\"type\":\"text_delta\",\"text\":\"Hello\"}".to_owned()];
        let event = SseStreamEvent::from_sse_lines(event_type, &data_lines);

        match event {
            Ok(Some(SseStreamEvent::Message { data, event_type })) => {
                assert_eq!(event_type.as_deref(), Some("content_block_delta"));
                assert_eq!(
                    data.get("type").and_then(|value| value.as_str()),
                    Some("text_delta")
                );
                assert_eq!(
                    data.get("text").and_then(|value| value.as_str()),
                    Some("Hello")
                );
            }
            Err(_) | Ok(None) => panic!("expected SseStreamEvent::Message, got {:?}", event),
        }
    }

    #[test]
    fn sse_stream_event_from_lines_returns_none_for_empty_data() {
        let event_type = Some("content_block_delta".to_owned());
        let data_lines: Vec<String> = vec![];
        let event = SseStreamEvent::from_sse_lines(event_type, &data_lines);
        assert!(event.unwrap().is_none());
    }

    #[test]
    fn sse_stream_event_from_lines_returns_err_for_invalid_json() {
        let event_type = Some("content_block_delta".to_owned());
        let data_lines = vec!["not valid json".to_owned()];
        let event = SseStreamEvent::from_sse_lines(event_type, &data_lines);
        assert!(event.is_err());
    }

    #[test]
    fn sse_decoder_buffers_partial_chunks_until_event_is_complete() {
        let mut decoder = SseDecoder::default();

        let first = decoder
            .push_chunk(b"event: content_block_delta\ndata: {\"type\":\"text_delta\"")
            .expect("first chunk");
        assert!(first.is_empty());

        let second = decoder
            .push_chunk(b",\"text\":\"hello\"}\n\n")
            .expect("second chunk");
        assert_eq!(second.len(), 1);
        assert_eq!(second[0]["type"], "text_delta");
        assert_eq!(second[0]["text"], "hello");
    }
}
