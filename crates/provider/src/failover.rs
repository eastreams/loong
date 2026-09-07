//! Ordered failover across a fixed set of providers.

use std::sync::Arc;

use async_trait::async_trait;

use crate::provider::{Provider, StreamError};

/// Ordered failover across a fixed set of providers.
///
/// `Failover` is itself a [`Provider`], so it composes: one failover can be an
/// entry in another failover. The first provider that succeeds wins; later
/// providers are not consulted for this stream. A committed provider that
/// disconnects ends the failover immediately.
///
/// `Failover` owns each request and is intentionally `'static`. For borrowed
/// requests, see [`RefFailover`].
pub struct Failover<Req, Item, Out>
where
    Req: Send + 'static,
    Item: Send + 'static,
    Out: loac::Writer<Item> + Send + 'static,
{
    providers: Vec<Arc<dyn Provider<Req, Item, Out>>>,
}

impl<Req, Item, Out> Failover<Req, Item, Out>
where
    Req: Send + 'static,
    Item: Send + 'static,
    Out: loac::Writer<Item> + Send + 'static,
{
    #[must_use]
    pub fn new(providers: Vec<Arc<dyn Provider<Req, Item, Out>>>) -> Self {
        Self { providers }
    }
}

impl<Req, Item, Out> Clone for Failover<Req, Item, Out>
where
    Req: Send + 'static,
    Item: Send + 'static,
    Out: loac::Writer<Item> + Send + 'static,
{
    fn clone(&self) -> Self {
        Self {
            providers: self.providers.clone(),
        }
    }
}

#[async_trait]
impl<Req, Item, Out> Provider<Req, Item, Out> for Failover<Req, Item, Out>
where
    Req: Send + 'static,
    Item: Send + 'static,
    Out: loac::Writer<Item> + Send + 'static,
{
    async fn stream(&self, req: Req, out: &mut Out) -> Result<(), StreamError<Req>> {
        let mut req = req;
        for provider in &self.providers {
            match provider.stream(req, &mut *out).await {
                Ok(()) => return Ok(()),
                Err(StreamError::Rejected { req: returned, .. }) => {
                    // Individual rejection reasons are dropped; failover
                    // aggregates one final rejection.
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

/// A provider that accepts any borrow of `Req`.
pub type BorrowedProvider<Req, Item, Out> = dyn for<'a> Provider<&'a Req, Item, Out>;

/// Ordered failover for borrowed requests.
///
/// `RefFailover` stores [`BorrowedProvider`]s. The higher-ranked lifetime
/// keeps this failover `'static` while each call borrows its request only for
/// the stream's duration.
///
/// Like [`Failover`], `RefFailover` is itself a [`Provider`] for borrowed
/// requests, so it composes: one borrowed failover can be an entry in another.
/// Use it when callers hold `&Req` rather than an owned `Req`.
pub struct RefFailover<Req, Item, Out>
where
    Req: ?Sized + Sync,
    Item: Send,
    Out: loac::Writer<Item> + Send,
{
    providers: Vec<Arc<BorrowedProvider<Req, Item, Out>>>,
}

impl<Req, Item, Out> RefFailover<Req, Item, Out>
where
    Req: ?Sized + Sync,
    Item: Send,
    Out: loac::Writer<Item> + Send,
{
    #[must_use]
    pub fn new(providers: Vec<Arc<BorrowedProvider<Req, Item, Out>>>) -> Self {
        Self { providers }
    }

    /// Streams a borrowed request through the providers.
    pub async fn stream<'a>(
        &self,
        req: &'a Req,
        out: &mut Out,
    ) -> Result<(), StreamError<&'a Req>> {
        let mut req = req;
        for provider in &self.providers {
            match provider.stream(req, &mut *out).await {
                Ok(()) => return Ok(()),
                Err(StreamError::Rejected { req: returned, .. }) => {
                    // Individual rejection reasons are dropped; failover
                    // aggregates one final rejection.
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

impl<Req, Item, Out> Clone for RefFailover<Req, Item, Out>
where
    Req: ?Sized + Sync,
    Item: Send,
    Out: loac::Writer<Item> + Send,
{
    fn clone(&self) -> Self {
        Self {
            providers: self.providers.clone(),
        }
    }
}

#[async_trait]
impl<'a, Req, Item, Out> Provider<&'a Req, Item, Out> for RefFailover<Req, Item, Out>
where
    Req: ?Sized + Sync,
    Item: Send,
    Out: loac::Writer<Item> + Send,
{
    async fn stream(&self, req: &'a Req, out: &mut Out) -> Result<(), StreamError<&'a Req>> {
        RefFailover::stream(self, req, out).await
    }
}
