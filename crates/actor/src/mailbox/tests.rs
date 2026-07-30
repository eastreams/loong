use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::Wake,
};

mod admission;
mod dispatch_reply;

struct WakeCounter(AtomicUsize);

impl Wake for WakeCounter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct PanicWake(Arc<AtomicUsize>);

impl Wake for PanicWake {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
        panic!("intentional notification panic");
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
        panic!("intentional notification panic");
    }
}
