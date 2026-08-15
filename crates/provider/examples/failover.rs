use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use loac::Writer;
use loong_provider::{Failover, Provider, StreamError};
use tokio::{sync::mpsc, time::Instant};

struct Refusing;

#[async_trait]
impl Provider<String, u8, mpsc::Sender<u8>> for Refusing {
    async fn stream(
        &self,
        req: String,
        _out: &mut mpsc::Sender<u8>,
    ) -> Result<(), StreamError<String>> {
        Err(StreamError::rejected("simulated refusal", req))
    }
}

struct Ticks(Vec<u8>);

#[async_trait]
impl Provider<String, u8, mpsc::Sender<u8>> for Ticks {
    async fn stream(
        &self,
        _req: String,
        out: &mut mpsc::Sender<u8>,
    ) -> Result<(), StreamError<String>> {
        for &item in &self.0 {
            if out.write(item).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Ok(())
    }
}

struct DisconnectsAfterTwo;

#[async_trait]
impl Provider<String, u8, mpsc::Sender<u8>> for DisconnectsAfterTwo {
    async fn stream(
        &self,
        _req: String,
        out: &mut mpsc::Sender<u8>,
    ) -> Result<(), StreamError<String>> {
        for item in [1u8, 2] {
            if out.write(item).await.is_err() {
                return Ok(());
            }
        }
        Err(StreamError::disconnected("simulated disconnect"))
    }
}

/// Runs one stream and shows how the subscriber handles each outcome.
async fn run(prompt: &str, failover: Failover<String, u8, mpsc::Sender<u8>>) {
    let (mut tx, mut rx) = mpsc::channel(4);
    let prompt = prompt.to_string();
    let t0 = Instant::now();

    // This demo runs the production in a plain Tokio task. Inside an actor,
    // call `failover.stream(...)` from the handler's owned reply future so the
    // actor runtime tracks it for Kill/Drain.
    let outcome = tokio::spawn(async move { failover.stream(prompt, &mut tx).await });

    while let Some(item) = rx.recv().await {
        println!("{:.3}s: {item}", t0.elapsed().as_secs_f32());
    }

    match outcome.await.unwrap() {
        Ok(()) => println!("stream completed"),
        Err(StreamError::Rejected { reason, .. }) => println!("stream rejected: {reason}"),
        Err(StreamError::Disconnected { reason }) => {
            println!("stream disconnected: {reason}")
        }
    }
}

#[tokio::main]
async fn main() {
    run(
        "prompt",
        Failover::new(vec![Arc::new(Refusing), Arc::new(Ticks(vec![1, 2, 3, 4]))]),
    )
    .await;

    run(
        "prompt",
        Failover::new(vec![Arc::new(Refusing), Arc::new(Refusing)]),
    )
    .await;

    run("prompt", Failover::new(vec![Arc::new(DisconnectsAfterTwo)])).await;
}
