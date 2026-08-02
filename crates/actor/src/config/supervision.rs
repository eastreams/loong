use super::{SupervisionConfig, sealed};

const DEFAULT_CAPACITY: usize = 32;

/// An actor without direct children.
#[doc(hidden)]
pub struct NoChildren;

/// Direct-child ownership with one fixed limit.
#[doc(hidden)]
pub struct Children<const N: usize = DEFAULT_CAPACITY>;

/// Direct-child ownership configured during actor creation.
#[doc(hidden)]
pub struct DynamicChildren<const DEFAULT: usize = DEFAULT_CAPACITY>;

/// Direct-child ownership without a finite limit.
#[doc(hidden)]
pub struct UnboundedChildren;

impl sealed::Supervision for NoChildren {}
impl SupervisionConfig for NoChildren {}

impl<const N: usize> sealed::Supervision for Children<N> {}
impl<const N: usize> SupervisionConfig for Children<N> {}

impl<const DEFAULT: usize> sealed::Supervision for DynamicChildren<DEFAULT> {}
impl<const DEFAULT: usize> SupervisionConfig for DynamicChildren<DEFAULT> {}

impl sealed::Supervision for UnboundedChildren {}
impl SupervisionConfig for UnboundedChildren {}
