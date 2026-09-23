//! Entities, role periods and capability rules (design section 2). No HTTP, no SQL --
//! `tests/dependency_boundary.rs` enforces that against this crate's own manifest.

pub mod error_code;

pub use error_code::{ErrorCode, ParamValue};
