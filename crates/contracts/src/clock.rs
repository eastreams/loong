use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub trait Clock: Send + Sync {
    fn now_epoch_s(&self) -> u64;
}

#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_epoch_s(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs())
    }
}

#[derive(Debug)]
pub struct FixedClock {
    now_epoch_s: AtomicU64,
}

impl FixedClock {
    #[must_use]
    pub fn new(now_epoch_s: u64) -> Self {
        Self {
            now_epoch_s: AtomicU64::new(now_epoch_s),
        }
    }

    pub fn set(&self, now_epoch_s: u64) {
        self.now_epoch_s.store(now_epoch_s, Ordering::Relaxed);
    }

    pub fn advance_by(&self, delta_s: u64) {
        self.now_epoch_s.fetch_add(delta_s, Ordering::Relaxed);
    }
}

impl Clock for FixedClock {
    fn now_epoch_s(&self) -> u64 {
        self.now_epoch_s.load(Ordering::Relaxed)
    }
}
