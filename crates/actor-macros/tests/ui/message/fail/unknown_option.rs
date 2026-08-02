// Unknown keys must not fall back to unit replies.
#[derive(actor_api::Message)]
#[message(result = u8)]
struct UnknownOption;

fn main() {}
