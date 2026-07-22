#![forbid(unsafe_code)]

//! Concrete builtin tool implementations.
//!
//! The app bootstrap registers these values into the runtime-owned tool plane,
//! for example `register(path, file::ReadTool)` when the `file` feature exposes
//! that module. Runtime owns registry identity and dispatch, Kernel owns policy
//! grants and audit, and Access owns physical side effects. This crate supplies
//! only concrete implementation types, payload parsing, and response shaping.
//!
//! Do not put `ToolPlane`, registry, policy, access facade, or broad tool
//! abstraction code here. Keeping those out prevents the builtin implementation
//! crate from becoming the next abstraction owner by accident.

#[cfg(feature = "file")]
pub mod file;
