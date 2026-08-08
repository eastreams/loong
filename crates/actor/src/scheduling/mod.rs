//! Built-in actor-aware reply scheduling profiles.
//!
//! [`#[actor(...)]`](macro@crate::actor) selects one profile:
//!
//! - omitting `mailbox` selects [`Disabled`];
//! - `mailbox` without `interleaved` selects [`Serial`];
//! - fixed `interleaved` forms select [`Fixed`];
//! - dynamic `interleaved` forms select [`Dynamic`];
//! - unbounded `interleaved` selects [`Unbounded`].
//!
//! The macro reference documents syntax and defaults.
//! Manual [`crate::MessageConfig`] implementations select a profile directly.
//!
//! A finite limit bounds active interleaved replies.
//! At the limit, dispatch pauses before the next handler.
//! The reply mode becomes known only after handler dispatch.
//! Every queued handler must therefore pass the same gate.
//!
//! Ready replies finish during dispatch.
//! Owned replies run in separate Tokio tasks.
//! Exclusive replies run one at a time in every profile.

mod profile;
mod queue;
mod runtime;
mod state;

use std::{
    panic::{self, AssertUnwindSafe},
    pin::Pin,
};

use crate::{ActorFuture, mailbox::Control};

pub use profile::{
    Disabled, Dynamic, Fixed, InterleavedScheduler, ReplyScheduler, SchedulerProfile, Serial,
    Unbounded,
};
pub(crate) use runtime::{ActorScheduler, RuntimeScheduler, SchedulerTurn, Seal, TurnContext};
pub(crate) use state::{
    DynamicLimit, Exclusive, FixedLimit, InterleavedLane, InterleavedProfile, InterleavedState,
    SerialLane, UnboundedLimit,
};

pub(crate) type ErasedActorFuture<A> = Pin<Box<dyn ActorFuture<A, Output = ()> + Send + 'static>>;

// Automatic frame destruction cannot borrow the actor's lifecycle control.
// Isolate each value so one Drop panic cannot skip sibling cleanup.
fn drop_without_unwind<T>(value: T) {
    if let Err(payload) = panic::catch_unwind(AssertUnwindSafe(|| drop(value))) {
        Control::discard_panic(payload);
    }
}
