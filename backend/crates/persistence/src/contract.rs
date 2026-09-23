//! Reads the schema contract version (design section 5): what version this
//! database's `schema_contract` table currently claims. Interpreting that against
//! this binary's own minimum is `fau-domain::schema_contract::is_compatible`; acting
//! on the result -- refusing to serve, or treating the database as merely
//! unreachable -- is `fau serve`'s startup gate.

use sqlx::PgPool;

use crate::pool::{SQLSTATE_INVALID_AUTHORIZATION, SQLSTATE_INVALID_PASSWORD};

/// PostgreSQL's SQLSTATE for "relation does not exist" -- a fixed identifier from
/// PostgreSQL's own error-codes table, so matching on it (via [`is_undefined_table`])
/// is safe the same way the connection-error SQLSTATEs in `pool.rs` are.
const SQLSTATE_UNDEFINED_TABLE: &str = "42P01";

/// PostgreSQL's SQLSTATE for "invalid catalog name" -- the shape a query takes when
/// the target database itself has not been created yet, as distinct from
/// `schema_contract` not existing within an otherwise-reachable database (see
/// [`is_undefined_table`]).
const SQLSTATE_INVALID_CATALOG_NAME: &str = "3D000";

/// The highest version this database's `schema_contract` table claims, or 0 if the
/// table is empty. Fails with a `sqlx::Error` carrying SQLSTATE `42P01` on a database
/// that has never been migrated -- [`is_undefined_table`] recognises that case for a
/// caller that wants to treat it as contract version 0.
pub async fn read_contract_version(pool: &PgPool) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar("select coalesce(max(version), 0) from schema_contract")
        .fetch_one(pool)
        .await
}

/// Whether `e` is PostgreSQL's "relation does not exist" -- the shape a query
/// against `schema_contract` takes on a database that has never been migrated.
pub fn is_undefined_table(e: &sqlx::Error) -> bool {
    matches!(
        e.as_database_error().and_then(|db_err| db_err.code()),
        Some(code) if code == SQLSTATE_UNDEFINED_TABLE
    )
}

/// Whether a failure to read the schema contract (other than a missing table --
/// handled separately by [`is_undefined_table`], which the caller folds into
/// contract version 0) is worth retrying rather than refusing to start. The
/// classification:
///
/// - Retryable, so `fau serve` warns and keeps running: network/TLS trouble
///   (`Io`/`Tls`), a pool timeout, and three SQLSTATEs that are plausibly
///   transient -- `28P01`/`28000` (auth, which may be mid credential rotation) and
///   `3D000` (the database itself does not exist yet).
/// - Not retryable, so `fau serve` refuses to start: everything else, including
///   `42501` (permission denied -- a privilege problem no retry fixes) and a
///   decode failure (the query succeeded but its result was not the shape
///   expected).
///
/// Reuses `pool`'s own SQLSTATE constants rather than repeating the literal
/// strings a second time.
pub fn is_retryable(e: &sqlx::Error) -> bool {
    match e {
        sqlx::Error::Io(_) | sqlx::Error::Tls(_) | sqlx::Error::PoolTimedOut => true,
        sqlx::Error::Database(db_err) => matches!(
            db_err.code().as_deref(),
            Some(SQLSTATE_INVALID_PASSWORD)
                | Some(SQLSTATE_INVALID_AUTHORIZATION)
                | Some(SQLSTATE_INVALID_CATALOG_NAME)
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;

    /// A minimal `DatabaseError` double, same shape as `pool.rs`'s, so
    /// `is_undefined_table`'s SQLSTATE branch can be exercised without a live server.
    #[derive(Debug)]
    struct FakeDbError {
        code: &'static str,
    }

    impl std::fmt::Display for FakeDbError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "fake database error {}", self.code)
        }
    }

    impl std::error::Error for FakeDbError {}

    impl sqlx::error::DatabaseError for FakeDbError {
        fn message(&self) -> &str {
            "fake"
        }

        fn code(&self) -> Option<Cow<'_, str>> {
            Some(Cow::Borrowed(self.code))
        }

        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }

        fn kind(&self) -> sqlx::error::ErrorKind {
            sqlx::error::ErrorKind::Other
        }
    }

    fn database_error(code: &'static str) -> sqlx::Error {
        sqlx::Error::Database(Box::new(FakeDbError { code }))
    }

    #[test]
    fn undefined_table_sqlstate_is_recognised() {
        assert!(is_undefined_table(&database_error("42P01")));
    }

    #[test]
    fn an_unrelated_sqlstate_is_not_undefined_table() {
        assert!(!is_undefined_table(&database_error("42601")));
    }

    #[test]
    fn a_non_database_error_is_not_undefined_table() {
        assert!(!is_undefined_table(&sqlx::Error::PoolTimedOut));
    }

    #[test]
    fn io_is_retryable() {
        let err = sqlx::Error::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "connection refused",
        ));
        assert!(is_retryable(&err));
    }

    #[test]
    fn tls_is_retryable() {
        let err = sqlx::Error::Tls(Box::new(std::io::Error::other("tls handshake failed")));
        assert!(is_retryable(&err));
    }

    #[test]
    fn pool_timed_out_is_retryable() {
        assert!(is_retryable(&sqlx::Error::PoolTimedOut));
    }

    #[test]
    fn invalid_password_sqlstate_is_retryable() {
        // A credential rotation in flight is plausibly transient.
        assert!(is_retryable(&database_error("28P01")));
    }

    #[test]
    fn invalid_authorization_sqlstate_is_retryable() {
        assert!(is_retryable(&database_error("28000")));
    }

    #[test]
    fn invalid_catalog_name_sqlstate_is_retryable() {
        // 3D000: the database itself has not been created yet.
        assert!(is_retryable(&database_error("3D000")));
    }

    #[test]
    fn insufficient_privilege_sqlstate_is_not_retryable() {
        // 42501: a privilege problem, not something a restart loop fixes.
        assert!(!is_retryable(&database_error("42501")));
    }

    #[test]
    fn a_decode_error_is_not_retryable() {
        let err = sqlx::Error::Decode(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "unexpected shape",
        )));
        assert!(!is_retryable(&err));
    }
}
