use loac::supervision::ChildSupervisor;

struct ForeignChildren;

// Manual configurations must select a built-in supervision profile.
impl ChildSupervisor for ForeignChildren {}

fn main() {}
