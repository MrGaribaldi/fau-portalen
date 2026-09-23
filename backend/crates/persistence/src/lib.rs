//! Database access. Owns transaction boundaries (design section 2); this crate does
//! not declare `axum` and does not know about HTTP.

mod contract;
mod migrate;
mod pool;

pub use contract::{is_retryable, is_undefined_table, read_contract_version};
pub use migrate::{run_migrations, Applied, MigrateError, MigrationSettings, MIGRATION_LOCK_ID};
pub use pool::{lazy_pool, safe_error_kind, ConnectErrorKind};
