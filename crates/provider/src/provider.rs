//! The provider trait and its stream error.

use std::sync::Arc;

use async_trait::async_trait;

/// A stateless upstream that streams items into a caller-owned writer.
///
/// `stream` either commits and resolves when the stream ends, or fails as
/// described by [`StreamError`], which splits failures at the commit boundary.
///
/// `Req`, `Item`, and `Out` are trait parameters, not method parameters, so
/// the trait stays dyn-compatible. `Out` is any [`loac::Writer`] of `Item`.
///
/// None of the parameters needs `'static`: `req` is owned by the future and
/// `out` is borrowed only for the stream's duration.
#[async_trait]
pub trait Provider<Req, Item, Out>: Send + Sync
where
    Req: Send,
    Item: Send,
    Out: loac::Writer<Item> + Send,
{
    /// Streams the request into `out` and resolves when the stream ends.
    async fn stream(&self, req: Req, out: &mut Out) -> Result<(), StreamError<Req>>
    where
        Req: 'async_trait;
}

/// Why a stream failed, split by the commit boundary.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum StreamError<Req> {
    /// The provider could not open or commit the stream. The request is
    /// returned so ordered failover can retry the next provider.
    #[error("provider rejected the request: {reason}")]
    Rejected {
        /// Why the provider declined, before committing.
        reason: String,
        /// The original request, returned with this rejection.
        req: Req,
    },

    /// The provider committed, then disconnected mid-stream. Items may already
    /// have been emitted, so this stream cannot be retried.
    #[error("provider disconnected mid-stream: {reason}")]
    Disconnected {
        /// Why the upstream ended after committing.
        reason: String,
    },
}

impl<Req> StreamError<Req> {
    pub fn rejected(reason: impl Into<String>, req: Req) -> Self {
        Self::Rejected {
            reason: reason.into(),
            req,
        }
    }

    pub fn disconnected(reason: impl Into<String>) -> Self {
        Self::Disconnected {
            reason: reason.into(),
        }
    }
}

/// A shared provider is itself a provider.
///
/// This makes `Arc<NonCloneProvider>` a [`Provider`] without requiring the
/// provider type to implement `Clone`.
#[async_trait]
impl<Req, Item, Out, P> Provider<Req, Item, Out> for Arc<P>
where
    Req: Send,
    Item: Send,
    Out: loac::Writer<Item> + Send,
    P: Provider<Req, Item, Out> + ?Sized,
{
    async fn stream(&self, req: Req, out: &mut Out) -> Result<(), StreamError<Req>>
    where
        Req: 'async_trait,
    {
        self.as_ref().stream(req, out).await
    }
}
