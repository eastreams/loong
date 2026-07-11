#![forbid(unsafe_code)]

//! Concrete builtin tool implementations.
//!
//! Use this crate from the app runtime by registering concrete tool values into
//! `loong-app::tools::plane`, for example
//! `register(path, file::ReadTool::new("read"))` when the `file` feature exposes
//! that module. The registry path, audit,
//! policy grant, and access facade are owned by app/kernel/access layers. This
//! crate only supplies the concrete implementation type plus its payload
//! parsing and response shaping.
//!
//! Do not put `ToolPlane`, registry, policy, access facade, or broad tool
//! abstraction code here. Keeping those out prevents the builtin implementation
//! crate from becoming the next abstraction owner by accident.

#[cfg(feature = "file")]
pub mod file;
