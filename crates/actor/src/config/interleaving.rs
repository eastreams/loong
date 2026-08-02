use super::{InterleavingConfig, sealed};

const DEFAULT_CAPACITY: usize = 32;

/// Messaging without interleaved replies.
#[doc(hidden)]
pub struct NoInterleaving;

/// Interleaved replies with one fixed limit.
#[doc(hidden)]
pub struct Interleaving<const N: usize = DEFAULT_CAPACITY>;

/// Interleaved replies configured during actor creation.
#[doc(hidden)]
pub struct DynamicInterleaving<const DEFAULT: usize = DEFAULT_CAPACITY>;

/// Interleaved replies without a finite limit.
#[doc(hidden)]
pub struct UnboundedInterleaving;

impl sealed::Interleaving for NoInterleaving {}
impl InterleavingConfig for NoInterleaving {}

impl<const N: usize> sealed::Interleaving for Interleaving<N> {}
impl<const N: usize> InterleavingConfig for Interleaving<N> {}

impl<const DEFAULT: usize> sealed::Interleaving for DynamicInterleaving<DEFAULT> {}
impl<const DEFAULT: usize> InterleavingConfig for DynamicInterleaving<DEFAULT> {}

impl sealed::Interleaving for UnboundedInterleaving {}
impl InterleavingConfig for UnboundedInterleaving {}
