//! The school register's public sources (docs/school-register-design.md §2): fetching, and
//! turning each API's JSON into `fau_domain::register::source` values. No test calls the
//! network; `tests/fixtures/` holds recorded responses.

pub mod brreg;
pub mod client;
mod error;
pub mod kartverket;
pub mod nsr;
pub mod ssb;

pub use error::{Source, SourceError, SourceErrorKind};
