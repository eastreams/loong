use std::{
    mem::{needs_drop, size_of},
    num::NonZeroUsize,
};

use crate::{
    Actor, ActorScope, HasChildren, HasInterleaving, MessageConfig, actor, scheduling, supervision,
    transport::{MessageSender, TryReserveError},
};

use super::{
    ActorConfig, ActorOptions, DEFAULT_MAILBOX_CAPACITY, DEFAULT_MAX_CHILDREN,
    DEFAULT_MAX_IN_FLIGHT, DynamicChildren, DynamicChildrenOptions, DynamicInterleaving,
    DynamicInterleavingOptions, DynamicMailbox, DynamicMailboxOptions, FixedChildren,
    FixedInterleaving, FixedMailbox, NoChildren, NoInterleaving, NoMailbox, SupervisionConfig,
    UnboundedChildren, UnboundedInterleaving, UnboundedMailbox,
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

struct DefaultChildren;

#[actor(children)]
impl Actor for DefaultChildren {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct Fixed;

#[actor(mailbox = 11, interleaved = 7, children = 5)]
impl Actor for Fixed {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct Dynamic;

#[actor(mailbox = dynamic, interleaved = dynamic, children = dynamic)]
impl Actor for Dynamic {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct CustomDynamic;

#[actor(mailbox = 1, interleaved = dynamic(3), children = dynamic(4))]
impl Actor for CustomDynamic {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct Unbounded;

#[actor(mailbox = unbounded, interleaved = unbounded, children = unbounded)]
impl Actor for Unbounded {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn assert_config<A>()
where
    A: ActorConfig + MessageConfig + SupervisionConfig,
{
}

fn assert_send_sync_static<T: Send + Sync + 'static>() {}
fn assert_has_interleaving<A: HasInterleaving>() {}
fn assert_has_children<A: HasChildren>() {}

trait Same<T> {}

impl<T> Same<T> for T {}

fn assert_same<T, U>()
where
    T: Same<U>,
{
}

#[test]
fn generated_configs_use_actor_specific_options() {
    assert_config::<Bare>();
    assert_config::<DefaultMailbox>();
    assert_config::<DefaultChildren>();
    assert_config::<Fixed>();
    assert_config::<Dynamic>();
    assert_config::<CustomDynamic>();
    assert_config::<Unbounded>();

    let _: ActorOptions<Bare, NoMailbox, NoInterleaving, NoChildren> = Default::default();
    let _: ActorOptions<DefaultMailbox, FixedMailbox, NoInterleaving, NoChildren> =
        Default::default();
    let _: ActorOptions<DefaultChildren, NoMailbox, NoInterleaving, FixedChildren> =
        Default::default();
    let _: ActorOptions<Fixed, FixedMailbox, FixedInterleaving, FixedChildren> = Default::default();
    let _: ActorOptions<Dynamic, DynamicMailbox, DynamicInterleaving, DynamicChildren> =
        Default::default();
    let _: ActorOptions<CustomDynamic, FixedMailbox, DynamicInterleaving<3>, DynamicChildren<4>> =
        Default::default();
    let _: ActorOptions<Unbounded, UnboundedMailbox, UnboundedInterleaving, UnboundedChildren> =
        Default::default();

    assert_has_interleaving::<Fixed>();
    assert_has_interleaving::<Dynamic>();
    assert_has_interleaving::<CustomDynamic>();
    assert_has_interleaving::<Unbounded>();
    assert_has_children::<Fixed>();
    assert_has_children::<DefaultChildren>();
    assert_has_children::<Dynamic>();
    assert_has_children::<CustomDynamic>();
    assert_has_children::<Unbounded>();

    assert_send_sync_static::<<Bare as ActorConfig>::Options>();
    assert_send_sync_static::<<Fixed as ActorConfig>::Options>();
    assert_send_sync_static::<<Dynamic as ActorConfig>::Options>();
    assert_send_sync_static::<<CustomDynamic as ActorConfig>::Options>();
    assert_send_sync_static::<<Unbounded as ActorConfig>::Options>();

    assert_same::<<Bare as SupervisionConfig>::Children, supervision::Disabled>();
    assert_same::<<Bare as MessageConfig>::Scheduler, scheduling::Disabled>();
    assert_same::<<DefaultMailbox as MessageConfig>::Scheduler, scheduling::Serial<DefaultMailbox>>(
    );
    assert_same::<<DefaultChildren as MessageConfig>::Scheduler, scheduling::Disabled>();
    assert_same::<<Fixed as MessageConfig>::Scheduler, scheduling::Fixed<Fixed, 7>>();
    assert_same::<<Dynamic as MessageConfig>::Scheduler, scheduling::Dynamic<Dynamic>>();
    assert_same::<<CustomDynamic as MessageConfig>::Scheduler, scheduling::Dynamic<CustomDynamic>>(
    );
    assert_same::<<Unbounded as MessageConfig>::Scheduler, scheduling::Unbounded<Unbounded>>();
    assert_same::<
        <DefaultChildren as SupervisionConfig>::Children,
        supervision::Fixed<DEFAULT_MAX_CHILDREN>,
    >();
    assert_same::<<Fixed as SupervisionConfig>::Children, supervision::Fixed<5>>();
    assert_same::<<Dynamic as SupervisionConfig>::Children, supervision::Dynamic>();
    assert_same::<<CustomDynamic as SupervisionConfig>::Children, supervision::Dynamic>();
    assert_same::<<Unbounded as SupervisionConfig>::Children, supervision::Unbounded>();
}

#[test]
fn omitted_interleaving_uses_zero_sized_spawn_state() {
    assert_eq!(size_of::<NoInterleaving>(), 0);
}

#[test]
fn omitted_mailbox_uses_no_scheduler_state() {
    assert_eq!(size_of::<<Bare as MessageConfig>::Scheduler>(), 0);
    assert_eq!(
        size_of::<<DefaultChildren as MessageConfig>::Scheduler>(),
        0
    );
    assert!(!needs_drop::<<Bare as MessageConfig>::Scheduler>());
    assert!(!needs_drop::<<DefaultChildren as MessageConfig>::Scheduler>());
}

#[test]
fn omitted_children_use_zero_sized_state() {
    let options = <Bare as ActorConfig>::Options::default();
    let children: supervision::Disabled = Bare::open_children(&options);

    assert_eq!(size_of::<NoChildren>(), 0);
    assert_eq!(size_of::<supervision::Disabled>(), 0);
    let _ = children;
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
    let (sender, _inbox, _scheduler) = <DefaultMailbox as MessageConfig>::open(&options);
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
fn dynamic_children_resolve_default_and_override_limit() {
    let options = <Dynamic as ActorConfig>::Options::default();
    assert_eq!(
        options.max_children(),
        NonZeroUsize::new(DEFAULT_MAX_CHILDREN).unwrap()
    );

    let max_children = NonZeroUsize::new(6).unwrap();
    let options = DynamicChildrenOptions::with_max_children(options, max_children);
    assert_eq!(options.max_children(), max_children);

    let options = <CustomDynamic as ActorConfig>::Options::default();
    assert_eq!(options.max_children(), NonZeroUsize::new(4).unwrap());
    let options = DynamicChildrenOptions::with_max_children(options, max_children);
    assert_eq!(options.max_children(), max_children);
}

#[test]
fn generated_config_uses_default_mailbox_budget() {
    assert_eq!(
        <Fixed as MessageConfig>::MAILBOX_DISPATCH_BUDGET,
        NonZeroUsize::new(16).unwrap()
    );
}
