use std::collections::BTreeSet;

use loong_contracts::Capability;

pub trait PolicyContext: Send + Sync {
    fn capabilities(&self) -> BTreeSet<Capability>;
}

pub trait PolicyContextFactory: 'static {
    type Context<'a>: PolicyContext;
}
