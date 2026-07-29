// Reply types use Rust tokens, never Actix-style strings.
#[derive(actor_api::Message)]
#[message(reply = "u8")]
struct QuotedReply;

fn main() {}
