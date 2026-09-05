#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Procedural macros for `loac`.

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;

mod actor;
mod message;
mod sync_handler;

/// Configures one `impl Actor for Type` block.
///
/// The attribute generates these runtime configuration implementations:
///
/// - [`ActorConfig`][actor-config];
/// - [`MessageConfig`][message-config];
/// - [`SupervisionConfig`][supervision-config].
///
/// The generated `MessageConfig` opens the actor's `Sender`, `Inbox`, and
/// `Scheduler` state.
/// Do not implement those traits again for the same actor.
/// A bare attribute enables no optional capability.
///
/// ```
/// # use actor_api as loac;
/// use loac::{Actor, ActorScope, actor};
///
/// struct Worker;
///
/// #[actor(mailbox = dynamic, interleaved = dynamic, children = 4)]
/// impl Actor for Worker {
///     type SpawnArgs = ();
///
///     async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
///         Self
///     }
/// }
/// ```
///
/// # Options
///
/// | Option | Purpose | Requires |
/// | --- | --- | --- |
/// | `mailbox` | Enables typed public messaging | Nothing |
/// | `mailbox_budget = E` | Limits consecutive message dispatch | `mailbox` |
/// | `interleaved` | Enables `Handler`/`StreamHandler` dispatch and interleaved actor-aware replies | `mailbox` |
/// | `children` | Enables direct child actor ownership | Nothing |
///
/// `mailbox`, `interleaved`, and `children` share five forms:
///
/// | Form | Selected profile |
/// | --- | --- |
/// | bare option | Fixed limit of `32` |
/// | `option = N` | Fixed limit of `N` |
/// | `option = dynamic` | Per-spawn limit defaulting to `32` |
/// | `option = dynamic(N)` | Per-spawn limit defaulting to `N` |
/// | `option = unbounded` | No finite limit |
///
/// Every finite expression must produce a nonzero `usize` constant.
/// Dynamic forms expose one method on [`SpawnOptions`][spawn-options]:
///
/// - [`with_mailbox_capacity`][mailbox-builder] for `mailbox`;
/// - [`with_max_in_flight`][interleaving-builder] for `interleaved`;
/// - [`with_max_children`][children-builder] for `children`.
///
/// # Mailbox
///
/// Mailbox capacity bounds accepted messages awaiting dispatch.
/// It does not bound active replies.
/// [`ActorRef::call`][call] and [`ActorRef::send`][send] wait when full.
/// [`ActorRef::try_call`][try-call] and [`ActorRef::try_send`][try-send] return immediately.
/// A mailbox without `interleaved` uses [`Serial`][serial].
/// Omitting `mailbox` removes public messaging methods.
/// It selects zero-sized [`Disabled`][disabled].
///
/// # Mailbox dispatch budget
///
/// `mailbox_budget = E` accepts a nonzero `usize` const expression.
/// It defaults to `16` when omitted.
/// At most `E` messages dispatch before checking other actor work.
/// This check does not force a Tokio task yield.
/// The option requires `mailbox`.
///
/// # Interleaved replies
///
/// `Handler` and `StreamHandler` dispatch requires this capability.
/// The limit counts active interleaved replies.
/// At the limit, queued messages pause before handler dispatch.
/// Their reply modes are not known yet.
/// All queued messages therefore wait behind the same limit.
/// Omitting `interleaved` removes that capability and its queue.
/// Exclusive replies remain available.
/// The option requires `mailbox`.
///
/// # Child actors
///
/// The limit counts direct child actors retained by the parent.
/// An exited child actor remains counted until its slot is released.
/// That release occurs before [`Actor::on_child_exit`][on-child-exit] starts.
/// A finite rejection returns the original spawn input.
/// Unbounded spawning uses [`Infallible`](std::convert::Infallible) as its error.
/// Omitting `children` removes child actor spawning methods.
/// This option does not require `mailbox`.
///
/// [actor-config]: https://docs.rs/loac/latest/loac/trait.ActorConfig.html
/// [disabled]: https://docs.rs/loac/latest/loac/scheduling/struct.Disabled.html
/// [message-config]: https://docs.rs/loac/latest/loac/trait.MessageConfig.html
/// [serial]: https://docs.rs/loac/latest/loac/scheduling/struct.Serial.html
/// [supervision-config]: https://docs.rs/loac/latest/loac/trait.SupervisionConfig.html
/// [spawn-options]: https://docs.rs/loac/latest/loac/type.SpawnOptions.html
/// [mailbox-builder]: https://docs.rs/loac/latest/loac/trait.DynamicMailboxOptions.html#tymethod.with_mailbox_capacity
/// [interleaving-builder]: https://docs.rs/loac/latest/loac/trait.DynamicInterleavingOptions.html#tymethod.with_max_in_flight
/// [children-builder]: https://docs.rs/loac/latest/loac/trait.DynamicChildrenOptions.html#tymethod.with_max_children
/// [call]: https://docs.rs/loac/latest/loac/struct.ActorRef.html#method.call
/// [send]: https://docs.rs/loac/latest/loac/struct.ActorRef.html#method.send
/// [try-call]: https://docs.rs/loac/latest/loac/struct.ActorRef.html#method.try_call
/// [try-send]: https://docs.rs/loac/latest/loac/struct.ActorRef.html#method.try_send
/// [on-child-exit]: https://docs.rs/loac/latest/loac/trait.Actor.html#method.on_child_exit
#[proc_macro_attribute]
pub fn actor(args: TokenStream, input: TokenStream) -> TokenStream {
    actor::expand(args, input)
}

/// Derives `loac::Message` for a struct, enum, or union.
///
/// A message derived without a `#[message(...)]` attribute is send-only with
/// unit output. Selecting either `reply` or `stream` makes the message
/// callable through `loac::HasReply`. The reply type defaults to `()` when
/// `reply` is omitted.
///
/// Prefer `#[message(reply = Type)]` and `loac::Handler` for ordinary
/// asynchronous request handling; the handler returns a future and `loac`
/// schedules it on the interleaved lane.
///
/// For a reply that is already complete during dispatch, implement
/// `loac::SyncHandler` with `#[message(reply = Type)]` and attach
/// `#[loac::sync_handler]` to the impl.
///
/// Use `#[message(stream = Item, reply = Final)]` for a streamed reply:
/// `loac::call` then returns `loac::StreamReply<Item, Final>`, and the message
/// is handled by implementing `loac::StreamHandler`. The final reply type
/// defaults to `()` when only `stream` is present.
///
/// Explicit reply scheduling remains available by implementing
/// `loac::DispatchHandler` directly.
///
/// Generic parameters and existing `where` predicates are preserved. The derive
/// adds `Send + 'static` bounds to the message type and every selected reply
/// or stream item type.
#[proc_macro_derive(Message, attributes(message))]
pub fn derive_message(input: TokenStream) -> TokenStream {
    message::expand(input)
}

/// Enables dispatch for one `SyncHandler` impl.
///
/// Attach this attribute to an `impl SyncHandler<M> for Actor` block whose
/// message uses `#[message(reply = Type)]`. It emits a ready-scheduled
/// `DispatchHandler` impl for that concrete actor and message pair, so the
/// actor does not need an interleaving lane.
///
/// ```
/// # use actor_api as loac;
/// use loac::{Actor, ActorScope, Message, SyncHandler, actor};
///
/// struct Counter(u64);
///
/// #[actor(mailbox)]
/// impl Actor for Counter {
///     type SpawnArgs = u64;
///
///     async fn init(value: u64, _scope: &mut ActorScope<'_, Self>) -> Self {
///         Self(value)
///     }
/// }
///
/// #[derive(Message)]
/// #[message(reply = u64)]
/// struct Add(u64);
///
/// #[loac::sync_handler]
/// impl SyncHandler<Add> for Counter {
///     fn handle(&mut self, message: Add, _scope: &mut ActorScope<Self>) -> u64 {
///         self.0 += message.0;
///         self.0
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn sync_handler(args: TokenStream, input: TokenStream) -> TokenStream {
    sync_handler::expand(args, input)
}

fn actor_crate_path() -> syn::Result<TokenStream2> {
    // Proc macros lack `$crate`.
    // Resolve the package after dependency renaming.
    match crate_name("loac").map_err(|error| {
        syn::Error::new(
            Span::call_site(),
            format!("could not resolve the `loac` crate: {error}"),
        )
    })? {
        // `crate` may name a package binary or example.
        // The runtime exports one stable self alias.
        FoundCrate::Itself => Ok(quote!(::loac)),
        FoundCrate::Name(name) => {
            let ident = syn::Ident::new(&name, Span::call_site());
            Ok(quote!(::#ident))
        }
    }
}
