mod read;
use std::ops::Deref;

pub use read::FsReadError;

use crate::Facade;

/// Filesystem access facade.
///
/// This module is the side-effect boundary for governed filesystem operations.
/// Callers provide raw paths and operation inputs; `FsAccess` resolves paths,
/// builds typed actions, asks policy for grants, and only then touches disk.
pub struct FsAccess<'a> {
    ctx: &'a Facade,
}

impl<'a> FsAccess<'a> {
    #[inline]
    #[must_use]
    pub fn new(ctx: &'a Facade) -> Self {
        Self { ctx }
    }
}

impl<'a> Deref for FsAccess<'a> {
    type Target = Facade;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.ctx
    }
}
