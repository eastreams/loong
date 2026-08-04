use std::num::NonZeroUsize;

use crate::{
    Actor, ActorScope, MessageConfig, actor,
    transport::{MessageSender, TryReserveError},
};

use super::{
    ActorConfig, ActorOptions, DEFAULT_MAILBOX_CAPACITY, DynamicMailbox, DynamicMailboxOptions,
    FixedMailbox, InterleavingConfig, NoMailbox, UnboundedMailbox,
};

struct Bare;

#[actor]
impl Actor for Bare {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct DefaultMailbox;

#[actor(mailbox)]
impl Actor for DefaultMailbox {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct Fixed;

#[actor(mailbox = 11, interleaved = 7)]
impl Actor for Fixed {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct Dynamic;

#[actor(mailbox = dynamic, interleaved = dynamic)]
impl Actor for Dynamic {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct Unbounded;

#[actor(mailbox = unbounded, interleaved = unbounded)]
impl Actor for Unbounded {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn assert_config<A>()
where
    A: ActorConfig + MessageConfig + InterleavingConfig,
{
}

fn assert_send_sync_static<T: Send + Sync + 'static>() {}

#[test]
fn generated_configs_use_actor_specific_options() {
    assert_config::<Bare>();
    assert_config::<DefaultMailbox>();
    assert_config::<Fixed>();
    assert_config::<Dynamic>();
    assert_config::<Unbounded>();

    let _: ActorOptions<Bare, NoMailbox> = Default::default();
    let _: ActorOptions<DefaultMailbox, FixedMailbox> = Default::default();
    let _: ActorOptions<Fixed, FixedMailbox> = Default::default();
    let _: ActorOptions<Dynamic, DynamicMailbox> = Default::default();
    let _: ActorOptions<Unbounded, UnboundedMailbox> = Default::default();

    assert_send_sync_static::<<Bare as ActorConfig>::Options>();
    assert_send_sync_static::<<Fixed as ActorConfig>::Options>();
    assert_send_sync_static::<<Dynamic as ActorConfig>::Options>();
    assert_send_sync_static::<<Unbounded as ActorConfig>::Options>();
}

#[test]
fn dynamic_options_resolve_default_and_override_capacity() {
    let options = <Dynamic as ActorConfig>::Options::default();
    assert_eq!(
        options.mailbox_capacity(),
        NonZeroUsize::new(DEFAULT_MAILBOX_CAPACITY).unwrap()
    );

    let capacity = NonZeroUsize::new(7).unwrap();
    let options = DynamicMailboxOptions::with_mailbox_capacity(options, capacity);
    assert_eq!(options.mailbox_capacity(), capacity);
}

#[test]
fn default_mailbox_uses_the_shared_capacity() {
    let options = <DefaultMailbox as ActorConfig>::Options::default();
    let (sender, _inbox) = <DefaultMailbox as MessageConfig>::open(&options);
    let reservations = (0..DEFAULT_MAILBOX_CAPACITY)
        .map(|_| sender.try_reserve().unwrap())
        .collect::<Vec<_>>();

    assert!(matches!(sender.try_reserve(), Err(TryReserveError::Full)));
    drop(reservations);
}

#[test]
fn generated_configs_resolve_runtime_limits() {
    let max_in_flight = NonZeroUsize::new(5).unwrap();
    let options = <Fixed as ActorConfig>::Options::default().with_max_in_flight(max_in_flight);

    assert_eq!(Fixed::max_in_flight(&options), max_in_flight);
    assert_eq!(
        <Fixed as MessageConfig>::MAILBOX_DISPATCH_BUDGET,
        NonZeroUsize::new(16).unwrap()
    );
}
