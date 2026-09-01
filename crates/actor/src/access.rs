#![allow(unsafe_code)]

//! Owned actor/scope access for borrow-free reply futures.
//!
//! `Cx` is the unsafe capsule that lets a reply be a plain [`Future`]
//! while still touching actor state inside synchronous scopes. The runtime
//! creates one handle per reply and polls that reply only on the actor task,
//! serially with every other actor-aware future and mailbox dispatch.

use std::{marker::PhantomData, ptr::NonNull};

use crate::{Actor, ActorScope, runtime::ScopeState};

/// Owned access handle used by [`Handler`](crate::Handler) and
/// [`StreamHandler`](crate::StreamHandler) futures, and by the explicit
/// [`ActorScope::cx_reply`] / [`ActorScope::cx_stream`] constructors.
///
/// The handle carries a phantom lifetime so safe code cannot store it in a
/// `'static` location (thread locals, detached tasks, globals). It is `Send`
/// because the runtime only polls the owning future on the actor task; the raw
/// pointers are never dereferenced concurrently.
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

    /// Runs `f` with temporary `&mut A` and [`ActorScope`] borrows together.
    ///
    /// The higher-ranked closure signature prevents either borrow from
    /// escaping the call. Do not call this from any task other than the actor
    /// task that owns the reply future.
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

    /// Runs `f` with a temporary `&mut A`.
    ///
    /// The higher-ranked closure signature prevents the mutable borrow from
    /// escaping the call. Do not call this from any task other than the actor
    /// task that owns the reply future.
    pub fn with_actor<R>(&mut self, f: impl for<'a> FnOnce(&'a mut A) -> R) -> R {
        self.with(|actor, _| f(actor))
    }

    /// Runs `f` with a temporary [`ActorScope`].
    pub fn with_scope<R>(&mut self, f: impl for<'a> FnOnce(&'a mut ActorScope<'a, A>) -> R) -> R {
        self.with(|_, scope| f(scope))
    }
}

// SAFETY: the runtime polls the owning future on the actor task, and every
// dereference happens inside `with` / `with_actor` / `with_scope` while the
// actor task has exclusive access. The phantom lifetime does not correspond to
// an actual borrow that could race with another thread.
unsafe impl<A: Actor> Send for Cx<'_, A> {}
