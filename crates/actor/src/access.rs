#![allow(unsafe_code)]

//! Owned actor/scope access for borrow-free reply futures.
//!
//! `Cx` is the unsafe capsule that lets a reply be a plain [`Future`]
//! while still touching actor state inside synchronous scopes. The runtime
//! creates one handle per reply and polls that reply only on the actor task,
//! serially with every other actor-aware future and mailbox dispatch.

use std::{marker::PhantomData, ops::Deref, ptr::NonNull};

use crate::{Actor, ActorRef, ActorScope, runtime::ScopeState};

/// Owned access handle used by [`Handler`](crate::Handler) and
/// [`StreamHandler`](crate::StreamHandler) futures, and by the explicit
/// [`ActorScope::cx_reply`] / [`ActorScope::cx_stream`] interleaved and
/// [`ActorScope::cx_exclusive`] / [`ActorScope::cx_stream_exclusive`]
/// exclusive constructors.
///
/// The handle carries a phantom lifetime so safe code cannot store it in a
/// `'static` location (thread locals, detached tasks, globals). It is `Send`
/// because the runtime only polls the owning future on the actor task; the raw
/// pointers are never dereferenced concurrently. For address-only access, `Cx`
/// derefs to [`ActorRef`] and exposes [`myself`](Self::myself).
pub struct Cx<'a, A: Actor + 'a> {
    actor: NonNull<A>,
    scope: NonNull<ScopeState<A>>,
    _lifetime: PhantomData<&'a mut A>,
}

impl<A: Actor> Cx<'_, A> {
    pub(crate) fn new<'a>(actor: &'a mut A, scope: &'a mut ScopeState<A>) -> Cx<'a, A> {
        Cx {
            actor: NonNull::from(actor),
            scope: NonNull::from(scope),
            _lifetime: PhantomData,
        }
    }

    /// Runs `f` with temporary `&mut A` and [`ActorScope`] borrows.
    ///
    /// Use `_` for the borrow you do not need. The higher-ranked closure
    /// signature prevents either borrow from escaping the call. Do not call
    /// this from any task other than the actor task that owns the reply
    /// future.
    pub fn with<R>(
        &mut self,
        f: impl for<'a> FnOnce(&'a mut A, &'a mut ActorScope<'a, A>) -> R,
    ) -> R {
        // SAFETY: the reply future is polled only on the actor task, serially
        // with all other actor work. The two pointers were created from two
        // non-overlapping mutable borrows (`&mut A` and `&mut ScopeState<A>`),
        // so reconstructing them together preserves uniqueness.
        let actor = unsafe { self.actor.as_mut() };
        let state = unsafe { self.scope.as_mut() };
        let mut scope = state.actor_scope();
        f(actor, &mut scope)
    }

    /// Returns this actor's non-owning address.
    ///
    /// Unlike [`with`](Self::with), this does not open an actor or scope
    /// borrow, so it is available whenever the `Cx` handle is. The returned
    /// address is the same one [`ActorScope::myself`] would return inside
    /// `with`.
    #[must_use]
    pub fn myself(&self) -> &ActorRef<A> {
        // SAFETY: `scope` points to the actor task's scope state, which the
        // runtime keeps alive for as long as this `Cx` exists. The address
        // field is immutable and `ActorRef` is a thread-safe handle, so a
        // shared borrow of it cannot race with the exclusive actor borrows
        // created by `with`.
        let state = unsafe { self.scope.as_ref() };
        &state.actor_ref
    }
}

impl<A: Actor> Deref for Cx<'_, A> {
    type Target = ActorRef<A>;

    fn deref(&self) -> &Self::Target {
        self.myself()
    }
}

// SAFETY: the runtime polls the owning future on the actor task. Actor and
// scope mutations happen inside `with` while the actor task has exclusive
// access; `myself` only reads the immutable address field. The phantom
// lifetime does not correspond to an actual borrow that could race with
// another thread.
unsafe impl<A: Actor> Send for Cx<'_, A> {}
