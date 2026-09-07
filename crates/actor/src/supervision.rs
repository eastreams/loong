//! Built-in direct-child supervision profiles.
//!
//! [`#[actor(...)]`](macro@crate::actor) selects one profile:
//!
//! - omitting `children` selects [`Disabled`];
//! - fixed `children` forms select [`Fixed`];
//! - dynamic `children` forms select [`Dynamic`];
//! - unbounded `children` selects [`Unbounded`].
//!
//! The macro reference documents syntax and defaults.
//! Manual configurations may select a profile directly.
//!
//! A finite limit bounds child actors retained by the parent.
//! At the limit, child spawning returns [`Full`] with its input.
//! An exited child actor still occupies its slot.
//! The slot is released before [`Actor::on_child_exit`](crate::Actor::on_child_exit).

pub(crate) mod runtime;

use std::{convert::Infallible, fmt, num::NonZeroUsize};

/// A sealed direct-child supervision profile.
///
/// Custom actor configurations select one built-in profile.
/// They do not implement this trait directly.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot supervise child actors",
    label = "select a built-in supervision profile"
)]
#[allow(
    private_bounds,
    private_interfaces,
    reason = "a private runtime bridge seals supervision profiles"
)]
pub trait ChildSupervisor: Send + 'static {
    /// Exposes this profile only to the actor runtime.
    #[doc(hidden)]
    fn __runtime(&mut self, _: runtime::Seal) -> &mut impl runtime::RuntimeChildren;
}

/// A sealed supervision profile supporting child actor spawning.
///
/// [`Disabled`] intentionally lacks this capability.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot spawn child actors",
    label = "enable `children` in this actor's configuration"
)]
#[allow(
    private_bounds,
    private_interfaces,
    reason = "a private runtime bridge reserves child spawning"
)]
pub trait ChildSpawner: ChildSupervisor {
    /// The error returned with rejected spawn input.
    type Error<T>;

    /// Exposes child spawning only to the actor runtime.
    #[doc(hidden)]
    fn __runtime_spawner(
        &mut self,
        _: runtime::Seal,
    ) -> &mut impl runtime::RuntimeChildSpawner<Self>;
}

/// Disables direct child actor supervision.
///
/// This profile is zero-sized.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Disabled;

impl Disabled {
    /// Creates a disabled profile.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// Supervises at most `N` retained direct child actors.
pub struct Fixed<const N: usize> {
    state: runtime::ChildSet,
}

impl<const N: usize> Fixed<N> {
    /// Creates an empty fixed profile.
    ///
    /// Compilation fails when `N` is zero.
    #[must_use]
    pub fn new() -> Self {
        const { assert!(N > 0, "child limit must be greater than zero") };
        Self {
            state: runtime::ChildSet::new(),
        }
    }
}

impl<const N: usize> Default for Fixed<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Supervises direct child actors with a per-spawn limit.
pub struct Dynamic {
    state: runtime::ChildSet,
    limit: NonZeroUsize,
}

impl Dynamic {
    /// Creates an empty profile with one resolved limit.
    #[must_use]
    pub fn new(limit: NonZeroUsize) -> Self {
        Self {
            state: runtime::ChildSet::new(),
            limit,
        }
    }
}

/// Supervises direct child actors without a finite limit.
pub struct Unbounded {
    state: runtime::ChildSet,
}

impl Unbounded {
    /// Creates an empty unbounded profile.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: runtime::ChildSet::new(),
        }
    }
}

impl Default for Unbounded {
    fn default() -> Self {
        Self::new()
    }
}

impl ChildSupervisor for Disabled {
    fn __runtime(&mut self, _: runtime::Seal) -> &mut impl runtime::RuntimeChildren {
        self
    }
}

impl<const N: usize> ChildSupervisor for Fixed<N> {
    fn __runtime(&mut self, _: runtime::Seal) -> &mut impl runtime::RuntimeChildren {
        self
    }
}

impl ChildSupervisor for Dynamic {
    fn __runtime(&mut self, _: runtime::Seal) -> &mut impl runtime::RuntimeChildren {
        self
    }
}

impl ChildSupervisor for Unbounded {
    fn __runtime(&mut self, _: runtime::Seal) -> &mut impl runtime::RuntimeChildren {
        self
    }
}

impl<const N: usize> ChildSpawner for Fixed<N> {
    type Error<T> = Full<T>;

    fn __runtime_spawner(
        &mut self,
        _: runtime::Seal,
    ) -> &mut impl runtime::RuntimeChildSpawner<Self> {
        self
    }
}

impl ChildSpawner for Dynamic {
    type Error<T> = Full<T>;

    fn __runtime_spawner(
        &mut self,
        _: runtime::Seal,
    ) -> &mut impl runtime::RuntimeChildSpawner<Self> {
        self
    }
}

impl ChildSpawner for Unbounded {
    type Error<T> = Infallible;

    fn __runtime_spawner(
        &mut self,
        _: runtime::Seal,
    ) -> &mut impl runtime::RuntimeChildSpawner<Self> {
        self
    }
}

/// A spawn input rejected at the direct child actor limit.
#[derive(thiserror::Error)]
#[error("direct child actor limit reached")]
pub struct Full<T> {
    value: T,
}

impl<T> Full<T> {
    pub(crate) const fn new(value: T) -> Self {
        Self { value }
    }

    /// Returns the rejected value without dropping it.
    pub fn into_inner(self) -> T {
        self.value
    }
}

impl<T> fmt::Debug for Full<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Full")
            .field("value", &"<value>")
            .finish()
    }
}
