//! Narrow APIs for requesting domain operations.
//!
//! Give each caller only the access it needs. An access API must not expose the
//! kernel, a session, a runtime, a backend, or a global context. Application
//! setup belongs elsewhere.

pub mod fs;
