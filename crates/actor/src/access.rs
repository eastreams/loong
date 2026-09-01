#![allow(unsafe_code)]

//! Owned actor/scope access for borrow-free reply futures.
//!
//! `ActorAccess` is the unsafe capsule that lets a reply be a plain [`Future`]
//! while still touching actor state inside synchronous scopes. The runtime
//! creates one handle per reply and polls that reply only on the actor task,
//! serially with every other actor-aware future and mailbox dispatch.

use std::{marker::PhantomData, ptr::NonNull};

use crate::{Actor, ActorScope, runtime::ScopeState};

/// Owned access handle handed to [`ActorScope::cx_reply`] and
/// [`ActorScope::cx_stream`] futures.
///
/// The handle carries a phantom lifetime so safe code cannot store it in a
/// `'static` location (thread locals, detached tasks, globals). It is `Send`
/// because the runtime only polls the owning future on the actor task; the raw
/// pointers are never dereferenced concurrently.
pub struct ActorAccess<'a, A: Actor> {
    actor: NonNull<A>,
    scope: NonNull<ScopeState<A>>,
    _lifetime: PhantomData<&'a mut A>,
}

impl<A: Actor> ActorAccess<'_, A> {
    pub(crate) fn new<'a>(actor: &'a mut A, scope: &'a mut ScopeState<A>) -> ActorAccess<'a, A> {
        ActorAccess {
            actor: NonNull::from(actor),
            scope: NonNull::from(scope),
            _lifetime: PhantomData,
        }
    }

    /// Runs `f` with a temporary `&mut A`.
    ///
    /// The higher-ranked closure signature prevents the mutable borrow from
    /// escaping the call. Do not call this from any task other than the actor
    /// task that owns the reply future.
    pub fn with_actor<R>(&mut self, f: impl for<'a> FnOnce(&'a mut A) -> R) -> R {
        // SAFETY: the reply future is polled only on the actor task, serially
        // with all other actor work, so this reconstructs the unique borrow the
        // runtime has already made available for this poll.
        let actor = unsafe { self.actor.as_mut() };
        f(actor)
    }

    /// Runs `f` with a temporary [`ActorScope`].
    pub fn with_scope<R>(&mut self, f: impl for<'a> FnOnce(&'a mut ActorScope<'a, A>) -> R) -> R {
        // SAFETY: see `with_actor`. The scope state outlives the reply future
        // and is only accessed from the actor task.
        let state = unsafe { self.scope.as_mut() };
        let mut scope = state.actor_scope();
        f(&mut scope)
    }
}

// SAFETY: the runtime polls the owning future on the actor task, and every
// dereference happens inside `with_actor` / `with_scope` while the actor task
// has exclusive access. The phantom lifetime does not correspond to an actual
// borrow that could race with another thread.
unsafe impl<A: Actor> Send for ActorAccess<'_, A> {}
