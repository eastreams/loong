use std::sync::Arc;

use crate::{
    Actor, ActorConfig, ActorRef, ActorScope, ChildExit, ChildId, ExitReason, ExitStatus,
    SubtreeStatus,
    mailbox::{ActorInbox, ActorInner, Mode},
};

use super::*;

/// Injects child state without widening the production runtime traits.
pub(crate) trait ChildrenFixture {
    fn len(&self) -> usize;

    fn insert_ref<A: Actor>(&mut self, actor_ref: &ActorRef<A>) -> ChildId;

    fn publish(&self, event: ChildExit);
}

impl<P: ChildProfile> ChildrenFixture for P {
    fn len(&self) -> usize {
        self.state().len()
    }

    fn insert_ref<A: Actor>(&mut self, actor_ref: &ActorRef<A>) -> ChildId {
        self.state_mut().insert(ErasedActorOwner::new(actor_ref))
    }

    fn publish(&self, event: ChildExit) {
        assert!(self.state().event_tx.send(event).is_ok());
    }
}

struct FirstActor;

#[crate::actor]
impl Actor for FirstActor {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct SecondActor;

#[crate::actor]
impl Actor for SecondActor {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct RegistrationProbe;

#[crate::actor]
impl Actor for RegistrationProbe {
    type SpawnArgs = ();

    async fn init(_: (), scope: &mut ActorScope<'_, Self>) -> Self {
        scope.request_shutdown(crate::Shutdown::Stop);
        Self
    }
}

fn actor_inner<A: Actor>() -> (Arc<ActorInner<A>>, ActorInbox<A>) {
    let (inner, inbox, _scheduler) = ActorInner::open(&<A as ActorConfig>::Options::default());
    (inner, inbox)
}

// The only child start token names an existing registration.
#[tokio::test(flavor = "current_thread")]
async fn registration_precedes_task_start() {
    let prepared = crate::runtime::PreparedActor::<RegistrationProbe>::new(
        (),
        <RegistrationProbe as ActorConfig>::Options::default(),
    );
    let mut children = ChildSet::new();

    let registered = children.register(prepared);
    let id = registered.parent.id;
    assert!(children.actors.contains_key(id.key()));

    let child = crate::runtime::start_child(registered);
    assert_eq!(child.id(), &id);
    let event = std::future::poll_fn(|task| children.poll_exit(task)).await;
    assert_eq!(event.child(), &id);
    assert!(children.reap(&event));
}

// Dequeue observes completion. Reaping alone releases registration capacity.
#[tokio::test]
async fn dequeued_exit_keeps_capacity_until_reaped() {
    let (inner, _inbox) = actor_inner::<FirstActor>();
    let actor_ref = ActorRef::new(Arc::clone(&inner));
    let mut children = Fixed::<1>::new();
    let child = children.insert_ref(&actor_ref);
    let status = inner.control.finish(ExitStatus::new(
        ExitReason::Stopped,
        SubtreeStatus::Terminated,
    ));
    children.publish(ChildExit::new(child, status));

    let event = std::future::poll_fn(|task| children.poll_exit(task)).await;
    let full = children.admit("next").unwrap_err();

    assert_eq!(full.into_inner(), "next");
    assert!(children.reap(&event));
    assert!(matches!(children.admit("next"), Ok("next")));
}

// Slot generations prevent stale exits from removing reused registrations.
#[test]
fn stale_exit_cannot_reap_a_reused_slot() {
    let mut children = ChildSet::new();
    let (first, _first_inbox) = actor_inner::<FirstActor>();
    let first_ref = ActorRef::new(Arc::clone(&first));
    let first_id = children.insert(ErasedActorOwner::new(&first_ref));
    assert!(children.reap(&ChildExit::new(
        first_id,
        ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Terminated),
    )));

    let (second, _second_inbox) = actor_inner::<FirstActor>();
    let second_ref = ActorRef::new(Arc::clone(&second));
    let second_id = children.insert(ErasedActorOwner::new(&second_ref));

    assert_ne!(first_id, second_id);
    assert!(!children.reap(&ChildExit::new(
        first_id,
        ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Terminated),
    )));
    assert_eq!(children.len(), 1);
}

// Typed messaging and erased ownership share one actor allocation.
#[test]
fn typed_and_erased_handles_share_one_allocation() {
    let (inner, _inbox) = actor_inner::<FirstActor>();
    let actor_ref = ActorRef::new(inner);
    let erased = ErasedActorOwner::new(&actor_ref);

    assert_eq!(
        Arc::as_ptr(&actor_ref.0).cast::<()>(),
        Arc::as_ptr(&erased.0).cast::<()>(),
    );
}

// Heterogeneous ownership keeps RAII Kill for every actor type.
#[test]
fn heterogeneous_owner_drop_requests_kill() {
    let (first, _first_inbox) = actor_inner::<FirstActor>();
    let (second, _second_inbox) = actor_inner::<SecondActor>();
    let first_ref = ActorRef::new(Arc::clone(&first));
    let second_ref = ActorRef::new(Arc::clone(&second));
    let mut children = ChildSet::new();
    children.insert(ErasedActorOwner::new(&first_ref));
    children.insert(ErasedActorOwner::new(&second_ref));

    drop(children);

    assert_eq!(first.control.mode(), Mode::Killing);
    assert_eq!(second.control.mode(), Mode::Killing);
}

// Waiting retains uncertainty before completed owners are cleared.
#[tokio::test]
async fn wait_all_retains_unconfirmed_subtree() {
    let (uncertain, _uncertain_inbox) = actor_inner::<FirstActor>();
    uncertain.control.finish(ExitStatus::new(
        ExitReason::Aborted,
        SubtreeStatus::Unconfirmed,
    ));
    let (terminated, _terminated_inbox) = actor_inner::<SecondActor>();
    terminated.control.finish(ExitStatus::new(
        ExitReason::Stopped,
        SubtreeStatus::Terminated,
    ));
    let uncertain_ref = ActorRef::new(uncertain);
    let terminated_ref = ActorRef::new(terminated);
    let mut children = ChildSet::new();
    children.insert(ErasedActorOwner::new(&uncertain_ref));
    children.insert(ErasedActorOwner::new(&terminated_ref));

    children.wait_all().await;

    assert_eq!(children.len(), 0);
    assert_eq!(
        children.terminal_status(ExitReason::Stopped),
        ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Unconfirmed),
    );
}

// Owning both endpoints makes receiver closure an invariant failure.
#[tokio::test]
#[should_panic(expected = "child-exit receiver closed while parent runtime was alive")]
async fn closed_exit_receiver_panics() {
    let mut children = ChildSet::new();
    children.event_rx.close();

    std::future::poll_fn(|task| children.poll_exit(task)).await;
}
