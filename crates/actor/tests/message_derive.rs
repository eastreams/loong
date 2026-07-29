use loong_actor::prelude::*;

// The prelude exports the trait and derive together.
// Their shared name occupies separate Rust namespaces.
#[derive(Message)]
struct Notify;

// Rust type tokens keep the reply statically visible.
#[derive(Message)]
#[message(reply = Result<u64, &'static str>)]
struct Query;

// This helper makes reply mismatches fail compilation.
fn assert_reply<M, R>()
where
    M: Message<Reply = R>,
    R: Send + 'static,
{
}

// Integration targets resolve this package as `Name("loong_actor")`.
// This catches expansions that incorrectly use `crate`.
#[test]
fn derives_messages_inside_the_actor_package() {
    assert_reply::<Notify, ()>();
    assert_reply::<Query, Result<u64, &'static str>>();
}
