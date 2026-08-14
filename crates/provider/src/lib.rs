//! Stateless provider clients and ordered failover.
//!
//! A [`Provider`] is a plain upstream client, not an actor. It owns no
//! mailbox and no lifecycle; the actor that initiates a stream hosts the
//! production work (as an owned reply future in `loac`). This crate only
//! standardizes the sink contract, the client shape, and the failover loop.

use std::{future::Future, sync::Arc};

use async_trait::async_trait;
use tokio::sync::mpsc;

/// A minimal item sink a provider streams into.
///
/// `write` returns the item back on `Err` when the sink is closed. The closed
/// signal is ordinary flow control (the subscriber stopped), not an error to
/// surface.
pub trait Writer<Item>
where
    Item: Send,
{
    /// Writes one item. `Err` returns the item: the sink is closed.
    fn write(&mut self, item: Item) -> impl Future<Output = Result<(), Item>> + Send + '_;
}

impl<T: Send> Writer<T> for mpsc::Sender<T> {
    async fn write(&mut self, item: T) -> Result<(), T> {
        self.send(item).await.map_err(|sent| sent.0)
    }
}

/// A stateless upstream that streams items into a caller-owned sink.
///
/// `stream` either commits and resolves when the stream ends, or fails in one
/// of two phases described by [`StreamError`]:
///
/// - [`StreamError::Rejected`] before committing, which failover can retry;
/// - [`StreamError::Disconnected`] after committing, which is terminal
///   because items may already have been emitted and cannot be replayed.
///
/// `Req`, `Item`, and `Out` are trait parameters, not method parameters, so
/// the trait stays dyn-compatible. `Out` is any [`Writer`] of `Item`.
///
/// None of the parameters needs `'static`: `req` is owned by the future and
/// `out` is borrowed only for the stream's duration.
#[async_trait]
pub trait Provider<Req, Item, Out>: Send + Sync
where
    Req: Send,
    Item: Send,
    Out: Writer<Item> + Send,
{
    /// Streams the request into `out` and resolves when the stream ends.
    async fn stream(&self, req: Req, out: &mut Out) -> Result<(), StreamError<Req>>;
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
        /// The request, returned for retry by the next provider.
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

/// Ordered failover across a fixed set of providers.
///
/// `Failover` is itself a [`Provider`], so it composes: one failover can be an
/// entry in another failover. The first provider that commits wins; later
/// providers are not consulted for this stream. A committed provider that
/// disconnects ends the failover immediately.
///
/// The `'static` bounds come from storing `Arc<dyn Provider<...>>`, not from
/// the `Provider` contract.
pub struct Failover<Req, Item, Out>
where
    Req: Send + 'static,
    Item: Send + 'static,
    Out: Writer<Item> + Send + 'static,
{
    providers: Vec<Arc<dyn Provider<Req, Item, Out>>>,
}

impl<Req, Item, Out> Failover<Req, Item, Out>
where
    Req: Send + 'static,
    Item: Send + 'static,
    Out: Writer<Item> + Send + 'static,
{
    pub fn new(providers: Vec<Arc<dyn Provider<Req, Item, Out>>>) -> Self {
        Self { providers }
    }
}

#[async_trait]
impl<Req, Item, Out> Provider<Req, Item, Out> for Failover<Req, Item, Out>
where
    Req: Send + 'static,
    Item: Send + 'static,
    Out: Writer<Item> + Send + 'static,
{
    async fn stream(&self, req: Req, out: &mut Out) -> Result<(), StreamError<Req>> {
        let mut req = req;
        for provider in &self.providers {
            match provider.stream(req, &mut *out).await {
                Ok(()) => return Ok(()),
                Err(StreamError::Rejected { req: returned, .. }) => {
                    // Per-provider reason is deferred to tracing.
                    req = returned;
                }
                Err(StreamError::Disconnected { reason }) => {
                    return Err(StreamError::Disconnected { reason });
                }
            }
        }
        Err(StreamError::Rejected {
            reason: "all providers rejected the request".to_string(),
            req,
        })
    }
}
