use std::sync::Arc;

use async_trait::async_trait;
use loac::Writer;
use loong_provider::{Failover, Provider, StreamError};
use tokio::sync::mpsc;

/// Intentionally not `Clone`: failover threads the request through providers
/// instead of cloning it.
#[derive(Debug, PartialEq)]
struct Request;

struct Refusing;

#[async_trait]
impl Provider<Request, u8, mpsc::Sender<u8>> for Refusing {
    async fn stream(
        &self,
        req: Request,
        _out: &mut mpsc::Sender<u8>,
    ) -> Result<(), StreamError<Request>> {
        Err(StreamError::rejected("simulated refusal", req))
    }
}

struct Sequence(Vec<u8>);

#[async_trait]
impl Provider<Request, u8, mpsc::Sender<u8>> for Sequence {
    async fn stream(
        &self,
        _req: Request,
        out: &mut mpsc::Sender<u8>,
    ) -> Result<(), StreamError<Request>> {
        for &item in &self.0 {
            if out.write(item).await.is_err() {
                break;
            }
        }
        Ok(())
    }
}

struct DisconnectsAfterTwo;

#[async_trait]
impl Provider<Request, u8, mpsc::Sender<u8>> for DisconnectsAfterTwo {
    async fn stream(
        &self,
        _req: Request,
        out: &mut mpsc::Sender<u8>,
    ) -> Result<(), StreamError<Request>> {
        for item in [1u8, 2] {
            if out.write(item).await.is_err() {
                return Ok(());
            }
        }
        Err(StreamError::disconnected("simulated disconnect"))
    }
}

#[tokio::test]
async fn commits_to_first_provider_that_starts() {
    let failover: Failover<Request, u8, mpsc::Sender<u8>> =
        Failover::new(vec![Arc::new(Refusing), Arc::new(Sequence(vec![1, 2, 3]))]);

    let (mut tx, mut rx) = mpsc::channel(4);
    let request = Request;
    let task = tokio::spawn(async move { failover.stream(request, &mut tx).await });

    assert_eq!(rx.recv().await, Some(1));
    assert_eq!(rx.recv().await, Some(2));
    assert_eq!(rx.recv().await, Some(3));
    assert_eq!(rx.recv().await, None);
    assert_eq!(task.await.unwrap(), Ok(()));
}

#[tokio::test]
async fn reports_rejected_when_every_provider_refuses() {
    let failover: Failover<Request, u8, mpsc::Sender<u8>> =
        Failover::new(vec![Arc::new(Refusing), Arc::new(Refusing)]);

    let (mut tx, _rx) = mpsc::channel(4);
    let request = Request;
    let result = failover.stream(request, &mut tx).await;

    assert_eq!(
        result,
        Err(StreamError::Rejected {
            reason: "all providers rejected the request".to_string(),
            req: Request,
        })
    );
}

#[tokio::test]
async fn reports_rejected_with_no_providers() {
    let failover: Failover<Request, u8, mpsc::Sender<u8>> = Failover::new(Vec::new());

    let (mut tx, _rx) = mpsc::channel(4);
    let request = Request;
    let result = failover.stream(request, &mut tx).await;

    assert_eq!(
        result,
        Err(StreamError::Rejected {
            reason: "all providers rejected the request".to_string(),
            req: Request,
        })
    );
}

#[tokio::test]
async fn reports_disconnect_without_retrying() {
    let failover: Failover<Request, u8, mpsc::Sender<u8>> =
        Failover::new(vec![Arc::new(DisconnectsAfterTwo), Arc::new(Refusing)]);

    let (mut tx, mut rx) = mpsc::channel(4);
    let request = Request;
    let task = tokio::spawn(async move { failover.stream(request, &mut tx).await });

    assert_eq!(rx.recv().await, Some(1));
    assert_eq!(rx.recv().await, Some(2));
    assert_eq!(rx.recv().await, None);
    assert_eq!(
        task.await.unwrap(),
        Err(StreamError::Disconnected {
            reason: "simulated disconnect".to_string()
        })
    );
}
