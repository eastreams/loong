//! Minimal push-based writer contract shared by the actor runtime and
//! higher-level streaming crates.

use std::future::Future;

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
