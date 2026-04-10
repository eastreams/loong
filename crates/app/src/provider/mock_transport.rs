use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_util::stream;
use serde_json::Value;

use super::transport_trait::{
    ProviderTransport, TransportError, TransportRequest, TransportResponse, TransportStream,
};

enum MockStreamResponse {
    Events(Vec<Result<Value, TransportError>>),
    Response(TransportResponse),
}

#[derive(Default)]
struct MockTransportState {
    execute_responses: VecDeque<Result<TransportResponse, TransportError>>,
    stream_responses: VecDeque<Result<MockStreamResponse, TransportError>>,
    requests: Vec<TransportRequest>,
}

#[derive(Clone, Default)]
pub(super) struct MockTransport {
    state: Arc<Mutex<MockTransportState>>,
}

impl MockTransport {
    pub(super) fn with_execute_responses<I>(responses: I) -> Self
    where
        I: IntoIterator<Item = Result<TransportResponse, TransportError>>,
    {
        let mut state = MockTransportState::default();
        state.execute_responses.extend(responses);
        Self {
            state: Arc::new(Mutex::new(state)),
        }
    }

    pub(super) fn with_stream_events<I>(responses: I) -> Self
    where
        I: IntoIterator<Item = Result<Vec<Result<Value, TransportError>>, TransportError>>,
    {
        let mut state = MockTransportState::default();
        state.stream_responses.extend(
            responses
                .into_iter()
                .map(|response| response.map(MockStreamResponse::Events)),
        );
        Self {
            state: Arc::new(Mutex::new(state)),
        }
    }

    pub(super) fn with_stream_response(response: TransportResponse) -> Self {
        let mut state = MockTransportState::default();
        state
            .stream_responses
            .push_back(Ok(MockStreamResponse::Response(response)));
        Self {
            state: Arc::new(Mutex::new(state)),
        }
    }

    pub(super) fn requests(&self) -> Vec<TransportRequest> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .requests
            .clone()
    }
}

#[async_trait]
impl ProviderTransport for MockTransport {
    async fn execute(
        &self,
        request: TransportRequest,
    ) -> Result<TransportResponse, TransportError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.requests.push(request);
        state
            .execute_responses
            .pop_front()
            .unwrap_or_else(|| Err(TransportError::other("mock execute response missing")))
    }

    async fn stream(&self, request: TransportRequest) -> Result<TransportStream, TransportError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.requests.push(request);
        match state
            .stream_responses
            .pop_front()
            .unwrap_or_else(|| Err(TransportError::other("mock stream response missing")))
        {
            Ok(MockStreamResponse::Events(events)) => Ok(TransportStream::Events {
                events: Box::pin(stream::iter(events)),
            }),
            Ok(MockStreamResponse::Response(response)) => Ok(TransportStream::Response(response)),
            Err(error) => Err(error),
        }
    }
}
