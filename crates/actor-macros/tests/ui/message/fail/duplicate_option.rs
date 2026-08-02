// Repeated reply options must not silently select one.
#[derive(actor_api::Message)]
#[message(reply = u8, reply = u16)]
struct DuplicateOption;

fn main() {}
