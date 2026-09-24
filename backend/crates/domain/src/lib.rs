//! Entities, role periods and capability rules (design section 2). No HTTP, no SQL --
//! `tests/dependency_boundary.rs` enforces that against this crate's own manifest.

pub mod email;
pub mod error_code;
pub mod membership;
pub mod register;
pub mod schema_contract;
pub mod time;

pub use error_code::{ErrorCode, ParamValue};
pub use schema_contract::{is_compatible, MINIMUM_CONTRACT_VERSION};
