//! Database access. Owns transaction boundaries (design section 2); this crate does
//! not declare `axum` and does not know about HTTP.

mod migrate;
mod pool;

pub use migrate::{run_migrations, Applied, MigrateError, MigrationSettings, MIGRATION_LOCK_ID};
pub use pool::ConnectErrorKind;
