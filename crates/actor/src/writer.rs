//! Minimal push-based writer contract shared by the actor runtime and
//! higher-level streaming crates.

use std::{future::Future, marker::PhantomData};

use tokio::sync::mpsc;

/// A writer that returns the item when the writer is closed.
pub trait Writer<Item>
where
    Item: Send,
{
    /// Writes one item.
    fn write(&mut self, item: Item) -> impl Future<Output = Result<(), Item>> + Send + '_;
}

impl<T: Send> Writer<T> for mpsc::Sender<T> {
    async fn write(&mut self, item: T) -> Result<(), T> {
        self.send(item).await.map_err(|sent| sent.0)
    }
}

/// A lifetime-bound item writer passed to [`StreamHandler`](crate::StreamHandler).
///
/// The wrapper owns the runtime-provided writer and ties it to the handler
/// future's borrow, so the writer cannot be moved into a `'static` task. It
/// closes the item stream when the handler future finishes or is dropped.
pub struct StreamOut<'a, W> {
    inner: W,
    _borrow: PhantomData<&'a mut ()>,
}

impl<'a, W> StreamOut<'a, W> {
    pub(crate) fn new(inner: W) -> Self {
        Self {
            inner,
            _borrow: PhantomData,
        }
    }
}

impl<Item, W> Writer<Item> for StreamOut<'_, W>
where
    Item: Send,
    W: Writer<Item>,
{
    fn write(&mut self, item: Item) -> impl Future<Output = Result<(), Item>> + Send + '_ {
        self.inner.write(item)
    }
}
