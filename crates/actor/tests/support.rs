#![allow(dead_code)]

use std::{
    future::Future,
    sync::{Mutex, MutexGuard},
    time::Duration,
};

pub async fn watchdog<F: Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(2), future)
        .await
        .expect("runtime operation exceeded the deadlock watchdog")
}

pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
