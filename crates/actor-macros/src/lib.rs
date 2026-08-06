#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Procedural macros for `loong-actor`.

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;

mod actor;
mod message;

/// Configures one `impl Actor for Type` block.
///
/// The attribute generates these runtime configuration implementations:
///
/// - [`ActorConfig`][actor-config];
/// - [`MessageConfig`][message-config];
/// - [`SupervisionConfig`][supervision-config].
///
/// The generated `MessageConfig` opens matched runtime state.
/// Do not implement those traits again for the same actor.
/// A bare attribute enables no optional capability.
///
/// ```
/// # use actor_api as loong_actor;
/// use loong_actor::{Actor, ActorScope, actor};
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
/// | `interleaved` | Enables interleaved actor-aware replies | `mailbox` |
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
/// [actor-config]: https://docs.rs/loong-actor/latest/loong_actor/trait.ActorConfig.html
/// [disabled]: https://docs.rs/loong-actor/latest/loong_actor/scheduling/struct.Disabled.html
/// [message-config]: https://docs.rs/loong-actor/latest/loong_actor/trait.MessageConfig.html
/// [serial]: https://docs.rs/loong-actor/latest/loong_actor/scheduling/struct.Serial.html
/// [supervision-config]: https://docs.rs/loong-actor/latest/loong_actor/trait.SupervisionConfig.html
/// [spawn-options]: https://docs.rs/loong-actor/latest/loong_actor/type.SpawnOptions.html
/// [mailbox-builder]: https://docs.rs/loong-actor/latest/loong_actor/trait.DynamicMailboxOptions.html#tymethod.with_mailbox_capacity
/// [interleaving-builder]: https://docs.rs/loong-actor/latest/loong_actor/trait.DynamicInterleavingOptions.html#tymethod.with_max_in_flight
/// [children-builder]: https://docs.rs/loong-actor/latest/loong_actor/trait.DynamicChildrenOptions.html#tymethod.with_max_children
/// [call]: https://docs.rs/loong-actor/latest/loong_actor/struct.ActorRef.html#method.call
/// [send]: https://docs.rs/loong-actor/latest/loong_actor/struct.ActorRef.html#method.send
/// [try-call]: https://docs.rs/loong-actor/latest/loong_actor/struct.ActorRef.html#method.try_call
/// [try-send]: https://docs.rs/loong-actor/latest/loong_actor/struct.ActorRef.html#method.try_send
/// [on-child-exit]: https://docs.rs/loong-actor/latest/loong_actor/trait.Actor.html#method.on_child_exit
#[proc_macro_attribute]
pub fn actor(args: TokenStream, input: TokenStream) -> TokenStream {
    actor::expand(args, input)
}

/// Derives `loong_actor::Message` for a struct, enum, or union.
///
/// The reply type defaults to `()`. Use `#[message(reply = Type)]` to select
/// another owned `Send + 'static` Rust type. Generic parameters and existing
/// `where` predicates are preserved.
#[proc_macro_derive(Message, attributes(message))]
pub fn derive_message(input: TokenStream) -> TokenStream {
    message::expand(input)
}

fn actor_crate_path() -> syn::Result<TokenStream2> {
    // Proc macros lack `$crate`.
    // Resolve the package after dependency renaming.
    match crate_name("loong-actor").map_err(|error| {
        syn::Error::new(
            Span::call_site(),
            format!("could not resolve the `loong-actor` crate: {error}"),
        )
    })? {
        // `crate` may name a package binary or example.
        // The runtime exports one stable self alias.
        FoundCrate::Itself => Ok(quote!(::loong_actor)),
        FoundCrate::Name(name) => {
            let ident = syn::Ident::new(&name, Span::call_site());
            Ok(quote!(::#ident))
        }
    }
}
