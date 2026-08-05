use std::{mem::size_of, num::NonZeroUsize};

use crate::{
    Actor, ActorScope, HasInterleaving, MessageConfig, actor,
    transport::{MessageSender, TryReserveError},
};

use super::{
    ActorConfig, ActorOptions, DEFAULT_MAILBOX_CAPACITY, DEFAULT_MAX_IN_FLIGHT,
    DynamicInterleaving, DynamicInterleavingOptions, DynamicMailbox, DynamicMailboxOptions,
    FixedInterleaving, FixedMailbox, NoInterleaving, NoMailbox, ReplySchedulingConfig,
    UnboundedInterleaving, UnboundedMailbox,
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

struct CustomDynamic;

#[actor(mailbox = 1, interleaved = dynamic(3))]
impl Actor for CustomDynamic {
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
    A: ActorConfig + MessageConfig + ReplySchedulingConfig,
{
}

fn assert_send_sync_static<T: Send + Sync + 'static>() {}
fn assert_has_interleaving<A: HasInterleaving>() {}

#[test]
fn generated_configs_use_actor_specific_options() {
    assert_config::<Bare>();
    assert_config::<DefaultMailbox>();
    assert_config::<Fixed>();
    assert_config::<Dynamic>();
    assert_config::<CustomDynamic>();
    assert_config::<Unbounded>();

    let _: ActorOptions<Bare, NoMailbox, NoInterleaving> = Default::default();
    let _: ActorOptions<DefaultMailbox, FixedMailbox, NoInterleaving> = Default::default();
    let _: ActorOptions<Fixed, FixedMailbox, FixedInterleaving> = Default::default();
    let _: ActorOptions<Dynamic, DynamicMailbox, DynamicInterleaving> = Default::default();
    let _: ActorOptions<CustomDynamic, FixedMailbox, DynamicInterleaving<3>> = Default::default();
    let _: ActorOptions<Unbounded, UnboundedMailbox, UnboundedInterleaving> = Default::default();

    assert_has_interleaving::<Fixed>();
    assert_has_interleaving::<Dynamic>();
    assert_has_interleaving::<CustomDynamic>();
    assert_has_interleaving::<Unbounded>();

    assert_send_sync_static::<<Bare as ActorConfig>::Options>();
    assert_send_sync_static::<<Fixed as ActorConfig>::Options>();
    assert_send_sync_static::<<Dynamic as ActorConfig>::Options>();
    assert_send_sync_static::<<CustomDynamic as ActorConfig>::Options>();
    assert_send_sync_static::<<Unbounded as ActorConfig>::Options>();
}

#[test]
fn omitted_interleaving_uses_zero_sized_spawn_state() {
    assert_eq!(size_of::<NoInterleaving>(), 0);
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
fn dynamic_interleaving_resolves_default_and_override_limit() {
    let options = <Dynamic as ActorConfig>::Options::default();
    assert_eq!(
        options.max_in_flight(),
        NonZeroUsize::new(DEFAULT_MAX_IN_FLIGHT).unwrap()
    );

    let max_in_flight = NonZeroUsize::new(5).unwrap();
    let options = DynamicInterleavingOptions::with_max_in_flight(options, max_in_flight);
    assert_eq!(options.max_in_flight(), max_in_flight);

    let options = <CustomDynamic as ActorConfig>::Options::default();
    assert_eq!(options.max_in_flight(), NonZeroUsize::new(3).unwrap());
    let options = DynamicInterleavingOptions::with_max_in_flight(options, max_in_flight);
    assert_eq!(options.max_in_flight(), max_in_flight);
}

#[test]
fn generated_config_uses_default_mailbox_budget() {
    assert_eq!(
        <Fixed as MessageConfig>::MAILBOX_DISPATCH_BUDGET,
        NonZeroUsize::new(16).unwrap()
    );
}
