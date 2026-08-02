// The runtime dependency uses a Cargo alias.
// This catches package-name-bound expansions.
use actor_api::prelude::*;

#[derive(Message)]
enum Wake {
    Now,
}

// A union checks that data shape remains irrelevant.
#[derive(actor_api::Message)]
union Reset {
    value: u8,
}

// These generics exercise preserved parameters and predicates.
// The nested reply checks Rust type-token parsing.
#[derive(actor_api::Message)]
#[message(reply = Result<[T; N], &'static str>)]
struct Query<'a, T, const N: usize>(&'a [T; N])
where
    T: Copy;

// This helper makes reply mismatches fail compilation.
fn assert_reply<M, R>()
where
    M: Message<Reply = R>,
    R: Send + 'static,
{
}

fn main() {
    assert_reply::<Wake, ()>();
    assert_reply::<Reset, ()>();
    assert_reply::<Query<'static, u8, 4>, Result<[u8; 4], &'static str>>();
    let _ = Wake::Now;
    let _ = Reset { value: 0 };
}
