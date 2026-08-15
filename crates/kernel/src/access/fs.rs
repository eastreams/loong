mod read;
use std::ops::Deref;

pub use read::FsReadError;

use crate::Facade;

/// Filesystem access facade.
///
/// This module is the side-effect boundary for governed filesystem operations.
///
/// The planned flow resolves raw caller paths into proof-bearing values,
/// builds typed actions, asks policy for grants, and only then touches disk.
/// Path resolution is not implemented yet: the current `read` scaffold still
/// carries a raw `PathBuf`, so it is not a resolved-path proof.
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
        self.ctx
    }
}
