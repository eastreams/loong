use std::{
    panic::{self, AssertUnwindSafe},
    sync::{Arc, Weak},
    task::{Context, Poll},
};

use tokio::sync::{mpsc, oneshot};

use crate::{
    Actor, ActorScope, CallError, Handler, Message, owned::OwnedTasks, reply::sealed::HandleReply,
    scheduler::ReplyScheduler,
};

mod control;

use control::DispatchPermit;
pub(crate) use control::{Control, HookEntryPermit, Mode, poll_with_panic_safe_waker};

pub(crate) type ReplyReceiver<R> = oneshot::Receiver<Result<R, CallError>>;

/// A type-erased mailbox entry for one statically checked message.
///
/// Addresses construct a concrete call or one-way envelope only when the actor
/// implements the corresponding `Handler<M>`. Erasure lets one bounded inbox
/// hold every message type handled by that actor. Dynamic dispatch ends at
/// [`Envelope::dispatch`]; the selected handler and reply strategy stay
/// statically dispatched.
pub(crate) type DynEnvelope<A> = Box<dyn Envelope<A>>;

/// Capacity and the concrete envelope remain recoverable when admission loses.
pub(crate) type RejectedAdmission<'a, A, E> = (mpsc::Permit<'a, DynEnvelope<A>>, Box<E>);

/// Notifies one response observer without blaming its Waker on the actor.
///
/// A rejected value remains actor-owned. Its destructor may still fail the
/// actor through the separate user Drop boundary.
fn notify_response<T>(control: &Control, reply: oneshot::Sender<T>, value: T) {
    match panic::catch_unwind(AssertUnwindSafe(|| reply.send(value))) {
        Ok(Ok(())) => {}
        Ok(Err(value)) => control.drop_user_value(value),
        Err(payload) => Control::discard_panic(payload),
    }
}

/// Shared state behind typed and erased actor handles.
///
/// The mailbox sender and lifecycle state share one allocation.
/// Queued envelopes must not strong-own this value.
pub(crate) struct ActorInner<A: Actor> {
    pub(crate) sender: mpsc::Sender<DynEnvelope<A>>,
    pub(crate) control: Control,
}

// `admit` lives in control.rs, keeping raw gate access private.
impl<A: Actor> ActorInner<A> {
    pub(crate) fn channel(capacity: usize) -> (Arc<Self>, ActorInbox<A>) {
        let (sender, receiver) = mpsc::channel(capacity);
        let control = Control::new();
        let actor = Arc::new(Self { sender, control });
        let inbox = ActorInbox {
            receiver,
            inner: Arc::clone(&actor),
        };
        (actor, inbox)
    }
}

/// Owns accepted messages and their lifecycle cleanup context.
///
/// Tokio drops a receiver's queue under one unwind boundary.
/// This wrapper contains each envelope destructor independently.
pub(crate) struct ActorInbox<A: Actor> {
    receiver: mpsc::Receiver<DynEnvelope<A>>,
    inner: Arc<ActorInner<A>>,
}

impl<A: Actor> ActorInbox<A> {
    /// Closes the receiver without exposing capacity-waiter wake panics.
    pub(crate) fn close(&mut self) {
        if let Err(payload) = panic::catch_unwind(AssertUnwindSafe(|| self.receiver.close())) {
            Control::discard_panic(payload);
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.receiver.is_empty()
    }

    pub(crate) fn poll_recv(&mut self, task: &mut Context<'_>) -> Poll<Option<DynEnvelope<A>>> {
        self.receiver.poll_recv(task)
    }

    /// Discards one accepted envelope through its lifecycle boundary.
    pub(crate) fn try_discard(&mut self) -> bool {
        let Ok(envelope) = self.receiver.try_recv() else {
            return false;
        };
        self.inner.control.drop_user_value(envelope);
        true
    }
}

impl<A: Actor> Drop for ActorInbox<A> {
    fn drop(&mut self) {
        // Runtime teardown closes lifecycle admission first.
        // Outstanding permits therefore cannot refill this queue.
        self.close();
        while self.try_discard() {}
    }
}

/// Runtime behavior of a concrete mailbox entry after its message type is
/// erased for storage.
///
/// An admitted entry remains queued until the actor task either discards it or
/// consumes it exactly once for dispatch. Concrete envelope types own their
/// queued cleanup behavior, including whether a waiting caller must be notified.
pub(crate) trait Envelope<A: Actor>: Send {
    /// Attempts to move one accepted entry from the queue into actor execution.
    ///
    /// Abandoned calls return before lifecycle dispatch. Other entries first
    /// commit dispatch against lifecycle shutdown.
    /// If dispatch wins, it invokes the statically selected handler and hands
    /// reply completion to the actor runtime; otherwise it performs queued
    /// rejection without invoking user handler code.
    fn dispatch(
        self: Box<Self>,
        actor: &mut A,
        scope: &mut ActorScope<'_, A>,
        owned: &OwnedTasks<A>,
        scheduler: &mut ReplyScheduler<A>,
        inner: &Arc<ActorInner<A>>,
    );
}

/// Coupled ownership of a two-way call before it leaves the queued phase.
struct QueuedCall<A: Actor, M: Message> {
    message: M,
    reply: oneshot::Sender<Result<M::Reply, CallError>>,
    actor: Weak<ActorInner<A>>,
}

enum CallEnvelopeState<A: Actor, M: Message> {
    Queued(QueuedCall<A, M>),
    /// Tombstone installed after queued ownership leaves the envelope, making
    /// its subsequent `Drop` a no-op.
    Consumed,
}

/// A request-response mailbox entry awaiting dispatch.
///
/// While queued, dropping the caller's response marks the entry abandoned, and
/// dropping the entry reports a phase-aware queued failure to a remaining
/// caller. Dispatch consumes the queued state and transfers reply ownership to
/// [`DispatchReply`].
pub(crate) struct CallEnvelope<A: Actor, M: Message> {
    state: CallEnvelopeState<A, M>,
}

impl<A: Actor, M: Message> CallEnvelope<A, M> {
    /// Creates the queued entry and the response endpoint retained by its caller.
    pub(crate) fn new(message: M, actor: &Arc<ActorInner<A>>) -> (Self, ReplyReceiver<M::Reply>) {
        let (reply, response) = oneshot::channel();
        (
            Self {
                state: CallEnvelopeState::Queued(QueuedCall {
                    message,
                    reply,
                    actor: Arc::downgrade(actor),
                }),
            },
            response,
        )
    }

    /// Recovers a message whose envelope lost admission before dispatch.
    pub(crate) fn into_message(mut self) -> M {
        let QueuedCall {
            message,
            reply,
            actor,
        } = self.take_queued();
        if let Err(payload) = panic::catch_unwind(AssertUnwindSafe(|| drop(reply))) {
            Control::discard_panic(payload);
        }
        drop(actor);
        message
    }

    /// Moves the coupled queued state out while disarming queued-failure Drop.
    fn take_queued(&mut self) -> QueuedCall<A, M> {
        match std::mem::replace(&mut self.state, CallEnvelopeState::Consumed) {
            CallEnvelopeState::Queued(queued) => queued,
            CallEnvelopeState::Consumed => panic!("a call envelope is consumed at most once"),
        }
    }
}

impl<A, M> Envelope<A> for CallEnvelope<A, M>
where
    A: Handler<M>,
    M: Message,
{
    fn dispatch(
        mut self: Box<Self>,
        actor: &mut A,
        scope: &mut ActorScope<'_, A>,
        owned: &OwnedTasks<A>,
        scheduler: &mut ReplyScheduler<A>,
        inner: &Arc<ActorInner<A>>,
    ) {
        // Only calls can be abandoned; one-way envelopes have no receiver.
        match &self.state {
            CallEnvelopeState::Queued(queued) if queued.reply.is_closed() => return,
            CallEnvelopeState::Queued(_) => {}
            CallEnvelopeState::Consumed => {
                panic!("a consumed call envelope cannot remain in the mailbox")
            }
        }

        let QueuedCall {
            message,
            reply,
            actor: queued_actor,
        } = self.take_queued();
        drop(queued_actor);

        let permit = match inner.begin_dispatch() {
            Ok(permit) => permit,
            Err(error) => {
                notify_response(&inner.control, reply, Err(error));
                inner.control.drop_user_value(message);
                return;
            }
        };

        // The permit commits DuringDispatch before any user code runs,
        // including synchronous reply construction.
        let reply = DispatchReply::new(reply, permit);
        HandleReply::handle(actor.handle(message, scope), owned, scheduler, reply);
    }
}

impl<A: Actor, M: Message> Drop for CallEnvelope<A, M> {
    fn drop(&mut self) {
        let CallEnvelopeState::Queued(QueuedCall {
            message,
            reply,
            actor,
        }) = std::mem::replace(&mut self.state, CallEnvelopeState::Consumed)
        else {
            return;
        };

        let Some(actor) = actor.upgrade() else {
            // ExitGuard retains the actor through receiver cleanup.
            // This fallback handles malformed internal fixtures.
            if let Err(payload) = panic::catch_unwind(AssertUnwindSafe(|| drop(reply))) {
                Control::discard_panic(payload);
            }
            if let Err(payload) = panic::catch_unwind(AssertUnwindSafe(|| drop(message))) {
                Control::discard_panic(payload);
            }
            return;
        };
        let error = actor.control.queued_failure();
        notify_response(&actor.control, reply, Err(error));
        actor.control.drop_user_value(message);
    }
}

/// A queued message whose sender observes admission but not completion.
///
/// Keeping this envelope distinct from `CallEnvelope` makes the absence of a
/// response receiver structural: queued one-way work is never mistaken for an
/// abandoned call and does not allocate a dummy channel.
pub(crate) struct SendEnvelope<M: Message<Reply = ()>> {
    message: M,
}

impl<M: Message<Reply = ()>> SendEnvelope<M> {
    pub(crate) fn new(message: M) -> Self {
        Self { message }
    }

    /// Recovers a message whose envelope lost admission before dispatch.
    pub(crate) fn into_message(self) -> M {
        self.message
    }
}

impl<A, M> Envelope<A> for SendEnvelope<M>
where
    A: Handler<M>,
    M: Message<Reply = ()>,
{
    fn dispatch(
        self: Box<Self>,
        actor: &mut A,
        scope: &mut ActorScope<'_, A>,
        owned: &OwnedTasks<A>,
        scheduler: &mut ReplyScheduler<A>,
        inner: &Arc<ActorInner<A>>,
    ) {
        let Self { message } = *self;
        let permit = match inner.begin_dispatch() {
            Ok(permit) => permit,
            Err(_) => return,
        };

        // One-way completion still owns a dispatch permit, so panic and Kill
        // use the same state transition as a call even though no result is sent.
        let reply = DispatchReply::one_way(permit);
        HandleReply::handle(actor.handle(message, scope), owned, scheduler, reply);
    }
}

enum DispatchReplyState<'a, A: Actor, R> {
    Caller {
        reply: oneshot::Sender<Result<R, CallError>>,
        permit: DispatchPermit<'a, A>,
    },
    OneWay {
        permit: DispatchPermit<'a, A>,
    },
    Completed,
}

pub(crate) struct DispatchReply<'a, A: Actor, R> {
    state: DispatchReplyState<'a, A, R>,
}

impl<'a, A: Actor, R> DispatchReply<'a, A, R> {
    fn new(reply: oneshot::Sender<Result<R, CallError>>, permit: DispatchPermit<'a, A>) -> Self {
        Self {
            state: DispatchReplyState::Caller { reply, permit },
        }
    }

    /// Promotes stack-bound dispatch only when reply work escapes this call.
    pub(crate) fn into_owned(mut self) -> DispatchReply<'static, A, R> {
        let state = std::mem::replace(&mut self.state, DispatchReplyState::Completed);
        let state = match state {
            DispatchReplyState::Caller { reply, permit } => DispatchReplyState::Caller {
                reply,
                permit: permit.into_owned(),
            },
            DispatchReplyState::OneWay { permit } => DispatchReplyState::OneWay {
                permit: permit.into_owned(),
            },
            DispatchReplyState::Completed => {
                panic!("a completed dispatch reply cannot escape dispatch")
            }
        };
        DispatchReply { state }
    }

    /// Completes dispatched work only if completion commits before Kill,
    /// failure, abort, or terminal publication.
    ///
    /// The gate decides the outcome; caller notification and destruction of a
    /// rejected reply value happen after the watch transaction is released.
    /// One-way work follows the same gate without creating a response channel.
    pub(crate) fn complete(mut self, response: R) {
        let state = std::mem::replace(&mut self.state, DispatchReplyState::Completed);

        match state {
            DispatchReplyState::Caller { reply, permit } => match permit.begin_completion() {
                Ok(_) => notify_response(permit.control(), reply, Ok(response)),
                Err(error) => {
                    notify_response(permit.control(), reply, Err(error));
                    permit.control().drop_user_value(response);
                }
            },
            DispatchReplyState::OneWay { permit } => {
                let _ = permit.begin_completion();
                permit.control().drop_user_value(response);
            }
            DispatchReplyState::Completed => {
                panic!("a dispatch reply completes at most once")
            }
        }
    }
}

impl<'a, A: Actor> DispatchReply<'a, A, ()> {
    fn one_way(permit: DispatchPermit<'a, A>) -> Self {
        Self {
            state: DispatchReplyState::OneWay { permit },
        }
    }
}

impl<A: Actor, R> Drop for DispatchReply<'_, A, R> {
    fn drop(&mut self) {
        let state = std::mem::replace(&mut self.state, DispatchReplyState::Completed);
        match state {
            DispatchReplyState::Caller { reply, permit } => {
                let error = permit.fail();
                notify_response(permit.control(), reply, Err(error));
            }
            DispatchReplyState::OneWay { permit } => {
                let _ = permit.fail();
            }
            DispatchReplyState::Completed => {}
        }
    }
}

#[cfg(test)]
mod tests;
