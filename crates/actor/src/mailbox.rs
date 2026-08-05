use std::{
    panic::{self, AssertUnwindSafe},
    sync::Arc,
    task::{Context, Poll},
};

use tokio::sync::oneshot;

use crate::{
    Actor, ActorScope, CallError, Handler, Message,
    owned::OwnedTasks,
    reply::sealed::HandleReply,
    scheduling::ActorScheduler,
    transport::{ErasedEnvelope, RuntimeInbox},
};

mod control;

use control::DispatchPermit;
pub(crate) use control::{Control, HookEntryPermit, Mode, poll_with_panic_safe_waker};

pub(crate) type ReplyReceiver<R> = oneshot::Receiver<Result<R, CallError>>;

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
    pub(crate) sender: A::Sender,
    pub(crate) control: Control,
}

// `admit` lives in control.rs, keeping raw gate access private.
impl<A: Actor> ActorInner<A> {
    pub(crate) fn open(options: &A::Options) -> (Arc<Self>, ActorInbox<A>) {
        let (sender, receiver) = A::open(options);
        let control = Control::new();
        let actor = Arc::new(Self { sender, control });
        let inbox = ActorInbox {
            receiver,
            inner: Arc::clone(&actor),
        };
        (actor, inbox)
    }
}

/// Owns receiving storage and its lifecycle cleanup context.
///
/// The runtime discards each accepted entry explicitly.
pub(crate) struct ActorInbox<A: Actor> {
    receiver: A::Inbox,
    inner: Arc<ActorInner<A>>,
}

impl<A: Actor> ActorInbox<A> {
    /// Closes the receiver without exposing capacity-waiter wake panics.
    pub(crate) fn close(&mut self) {
        if let Err(payload) = panic::catch_unwind(AssertUnwindSafe(|| {
            self.receiver.close();
        })) {
            Control::discard_panic(payload);
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.receiver.is_empty()
    }

    pub(crate) fn poll_recv(&mut self, task: &mut Context<'_>) -> Poll<Option<ErasedEnvelope<A>>> {
        self.receiver.poll_recv(task)
    }

    /// Discards one accepted envelope through its lifecycle boundary.
    pub(crate) fn try_discard(&mut self) -> bool {
        let Some(envelope) = self.receiver.try_recv() else {
            return false;
        };
        envelope.discard(&self.inner.control);
        true
    }
}

impl<A: Actor> Drop for ActorInbox<A> {
    fn drop(&mut self) {
        // Runtime teardown closes lifecycle admission first.
        // Bounded reservations rely on this ordering.
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
        scheduler: &mut ActorScheduler<A>,
        inner: &Arc<ActorInner<A>>,
    );

    /// Rejects one accepted entry without invoking its handler.
    fn discard(self: Box<Self>, control: &Control);
}

impl<A: Actor> ErasedEnvelope<A> {
    pub(crate) fn dispatch(
        self,
        actor: &mut A,
        scope: &mut ActorScope<'_, A>,
        owned: &OwnedTasks<A>,
        scheduler: &mut ActorScheduler<A>,
        inner: &Arc<ActorInner<A>>,
    ) {
        self.into_envelope()
            .dispatch(actor, scope, owned, scheduler, inner);
    }

    pub(crate) fn discard(self, control: &Control) {
        self.into_envelope().discard(control);
    }
}

/// A request-response mailbox entry awaiting dispatch.
///
/// The runtime explicitly dispatches or discards accepted entries.
/// Raw transport Drop has no lifecycle guarantee.
pub(crate) struct CallEnvelope<M: Message> {
    message: M,
    reply: oneshot::Sender<Result<M::Reply, CallError>>,
}

impl<M: Message> CallEnvelope<M> {
    /// Creates the queued entry and the response endpoint retained by its caller.
    pub(crate) fn new(message: M) -> (Self, ReplyReceiver<M::Reply>) {
        let (reply, response) = oneshot::channel();
        (Self { message, reply }, response)
    }

    /// Recovers a message whose envelope lost admission before dispatch.
    pub(crate) fn into_message(self) -> M {
        let Self { message, reply } = self;
        if let Err(payload) = panic::catch_unwind(AssertUnwindSafe(|| drop(reply))) {
            Control::discard_panic(payload);
        }
        message
    }

    fn reject(self, control: &Control, error: CallError) {
        let Self { message, reply } = self;
        notify_response(control, reply, Err(error));
        control.drop_user_value(message);
    }
}

impl<A, M> Envelope<A> for CallEnvelope<M>
where
    A: Handler<M>,
    M: Message,
{
    fn dispatch(
        self: Box<Self>,
        actor: &mut A,
        scope: &mut ActorScope<'_, A>,
        owned: &OwnedTasks<A>,
        scheduler: &mut ActorScheduler<A>,
        inner: &Arc<ActorInner<A>>,
    ) {
        // Only calls can be abandoned; one-way envelopes have no receiver.
        if self.reply.is_closed() {
            (*self).reject(&inner.control, inner.control.queued_failure());
            return;
        }

        let Self { message, reply } = *self;

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

    fn discard(self: Box<Self>, control: &Control) {
        (*self).reject(control, control.queued_failure());
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
        scheduler: &mut ActorScheduler<A>,
        inner: &Arc<ActorInner<A>>,
    ) {
        let Self { message } = *self;
        let permit = match inner.begin_dispatch() {
            Ok(permit) => permit,
            Err(_) => {
                inner.control.drop_user_value(message);
                return;
            }
        };

        // One-way completion still owns a dispatch permit, so panic and Kill
        // use the same state transition as a call even though no result is sent.
        let reply = DispatchReply::one_way(permit);
        HandleReply::handle(actor.handle(message, scope), owned, scheduler, reply);
    }

    fn discard(self: Box<Self>, control: &Control) {
        let Self { message } = *self;
        control.drop_user_value(message);
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
