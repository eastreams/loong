use std::{
    mem::size_of,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};

use crate::{
    Actor, ActorConfig, ActorFuture, ActorScope, IntoActorFuture, ReplySchedulingConfig,
    mailbox::{ActorInner, Mode},
    scheduling::{Exclusive, Fixed, InterleavedProfile, Serial},
};

use super::{RuntimeInterleavedScheduler, RuntimeScheduler};

struct SerialActor;

#[crate::actor(mailbox = 1)]
impl Actor for SerialActor {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct FixedActor;

#[crate::actor(mailbox = 1, interleaved = 2)]
impl Actor for FixedActor {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct DynamicActor;

#[crate::actor(mailbox = 1, interleaved = dynamic(2))]
impl Actor for DynamicActor {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct UnboundedActor;

#[crate::actor(mailbox = 1, interleaved = unbounded)]
impl Actor for UnboundedActor {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[test]
fn serial_profile_has_smaller_runtime_state() {
    assert!(size_of::<Serial<SerialActor>>() < size_of::<Fixed<FixedActor, 2>>());
}

#[test]
fn fixed_limit_stops_dispatch_at_capacity() {
    let options = <FixedActor as ActorConfig>::Options::default();
    let mut scheduler = FixedActor::open_scheduler(&options);

    assert!(scheduler.state().has_dispatch_capacity());
    RuntimeInterleavedScheduler::push_interleaved(
        &mut scheduler,
        std::future::pending::<()>().into_actor(),
    );
    assert!(scheduler.state().has_dispatch_capacity());
    RuntimeInterleavedScheduler::push_interleaved(
        &mut scheduler,
        std::future::pending::<()>().into_actor(),
    );
    assert!(!scheduler.state().has_dispatch_capacity());
}

#[test]
fn dynamic_limit_uses_each_spawn_option() {
    let options = <DynamicActor as ActorConfig>::Options::default();
    let mut scheduler = DynamicActor::open_scheduler(&options);
    for _ in 0..2 {
        RuntimeInterleavedScheduler::push_interleaved(
            &mut scheduler,
            std::future::pending::<()>().into_actor(),
        );
    }
    assert!(!scheduler.state().has_dispatch_capacity());

    let options = options.with_max_in_flight(std::num::NonZeroUsize::new(3).unwrap());
    let mut scheduler = DynamicActor::open_scheduler(&options);
    for _ in 0..2 {
        RuntimeInterleavedScheduler::push_interleaved(
            &mut scheduler,
            std::future::pending::<()>().into_actor(),
        );
    }
    assert!(scheduler.state().has_dispatch_capacity());
}

#[test]
fn unbounded_profile_never_closes_capacity() {
    let options = <UnboundedActor as ActorConfig>::Options::default();
    let mut scheduler = UnboundedActor::open_scheduler(&options);

    for _ in 0..128 {
        assert!(scheduler.state().has_dispatch_capacity());
        RuntimeInterleavedScheduler::push_interleaved(
            &mut scheduler,
            std::future::pending::<()>().into_actor(),
        );
    }
    assert!(scheduler.state().has_dispatch_capacity());
}

struct DropProbe {
    dropped: Arc<AtomicBool>,
    dropped_while_unwinding: Arc<AtomicBool>,
    panic: bool,
}

impl ActorFuture<DynamicActor> for DropProbe {
    type Output = ();

    fn poll(
        self: Pin<&mut Self>,
        _: &mut DynamicActor,
        _: &mut ActorScope<'_, DynamicActor>,
        _: &mut Context<'_>,
    ) -> Poll<()> {
        Poll::Pending
    }
}

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
        self.dropped_while_unwinding
            .store(std::thread::panicking(), Ordering::SeqCst);
        assert!(!self.panic, "intentional scheduler future drop panic");
    }
}

fn actor_inner() -> Arc<ActorInner<DynamicActor>> {
    let options = <DynamicActor as ActorConfig>::Options::default();
    ActorInner::open(&options).0
}

fn dynamic_scheduler() -> <DynamicActor as ReplySchedulingConfig>::Scheduler {
    DynamicActor::open_scheduler(&<DynamicActor as ActorConfig>::Options::default())
}

#[test]
fn clear_contains_each_reply_drop() {
    let actor = actor_inner();
    let exclusive_dropped = Arc::new(AtomicBool::new(false));
    let interleaved_dropped = Arc::new(AtomicBool::new(false));
    let dropped_while_unwinding = Arc::new(AtomicBool::new(false));
    let mut scheduler = dynamic_scheduler();

    RuntimeInterleavedScheduler::push_interleaved(
        &mut scheduler,
        DropProbe {
            dropped: Arc::clone(&interleaved_dropped),
            dropped_while_unwinding: Arc::clone(&dropped_while_unwinding),
            panic: false,
        },
    );
    RuntimeScheduler::push_exclusive(
        &mut scheduler,
        DropProbe {
            dropped: Arc::clone(&exclusive_dropped),
            dropped_while_unwinding: Arc::clone(&dropped_while_unwinding),
            panic: true,
        },
    );
    RuntimeScheduler::clear(&mut scheduler, &actor.control);

    assert!(exclusive_dropped.load(Ordering::SeqCst));
    assert!(interleaved_dropped.load(Ordering::SeqCst));
    assert!(RuntimeScheduler::is_idle(&mut scheduler));
    assert!(!dropped_while_unwinding.load(Ordering::SeqCst));
    assert_eq!(actor.control.mode(), Mode::Failing);
}

#[test]
fn queue_clear_continues_after_one_drop_panics() {
    let actor = actor_inner();
    let first_dropped = Arc::new(AtomicBool::new(false));
    let second_dropped = Arc::new(AtomicBool::new(false));
    let dropped_while_unwinding = Arc::new(AtomicBool::new(false));
    let mut scheduler = dynamic_scheduler();

    for (dropped, panic) in [
        (Arc::clone(&first_dropped), true),
        (Arc::clone(&second_dropped), false),
    ] {
        RuntimeInterleavedScheduler::push_interleaved(
            &mut scheduler,
            DropProbe {
                dropped,
                dropped_while_unwinding: Arc::clone(&dropped_while_unwinding),
                panic,
            },
        );
    }
    RuntimeScheduler::clear(&mut scheduler, &actor.control);

    assert!(first_dropped.load(Ordering::SeqCst));
    assert!(second_dropped.load(Ordering::SeqCst));
    assert!(!dropped_while_unwinding.load(Ordering::SeqCst));
    assert!(RuntimeScheduler::is_idle(&mut scheduler));
    assert_eq!(actor.control.mode(), Mode::Failing);
}

#[test]
fn exclusive_drop_contains_its_future_panic() {
    let dropped = Arc::new(AtomicBool::new(false));
    let dropped_while_unwinding = Arc::new(AtomicBool::new(false));
    let mut exclusive = Exclusive::<DynamicActor>::new();
    exclusive.push(DropProbe {
        dropped: Arc::clone(&dropped),
        dropped_while_unwinding: Arc::clone(&dropped_while_unwinding),
        panic: true,
    });

    drop(exclusive);

    assert!(dropped.load(Ordering::SeqCst));
    assert!(!dropped_while_unwinding.load(Ordering::SeqCst));
}
