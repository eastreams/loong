// Multiple message attributes cannot select one reply unambiguously.
#[derive(actor_api::Message)]
#[message(reply = u8)]
#[message(reply = u16)]
struct DuplicateAttribute;

fn main() {}
