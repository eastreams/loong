//! Minimal push-based writer contract shared by the actor runtime and
//! higher-level streaming crates.

use std::future::Future;

use tokio::sync::mpsc;

/// A minimal item writer.
pub trait Writer<Item>
where
    Item: Send,
{
    /// Writes one item, returning it if the writer is closed.
    fn write(&mut self, item: Item) -> impl Future<Output = Result<(), Item>> + Send + '_;
}

impl<T: Send> Writer<T> for mpsc::Sender<T> {
    async fn write(&mut self, item: T) -> Result<(), T> {
        self.send(item).await.map_err(|sent| sent.0)
    }
}
