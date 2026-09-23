//! The runtime readiness database check (design section 9): `select 1` bounded to
//! one second, called by `fau-app`'s readiness probe on every `/health/ready`
//! scrape as the first of two sequential queries. **This function's own timeout
//! does not, by itself, bound the probe as a whole** -- the second query,
//! [`crate::contract::read_contract_version`], answers a different question (schema
//! compatibility) and carries no timeout of its own here; called from `fau serve`'s
//! *startup* gate it sits under `main.rs`'s separate five-second
//! `SCHEMA_CONTRACT_CHECK_TIMEOUT`, but called from the readiness probe on every
//! scrape it has nothing bounding it in isolation. It is `fau_app::readiness`'s
//! `probe` that wraps *both* queries, run sequentially, in one outer one-second
//! `tokio::time::timeout` -- that is what keeps a scrape bounded to about one
//! second overall even when, for example, a migration holds an exclusive lock on
//! `schema_contract` that only the second query would ever wait on.

use std::time::Duration;

use sqlx::PgPool;

use crate::pool::safe_error_kind;

/// The per-probe bound the design names (section 9): a database check under one
/// second. The pool's own `acquire_timeout` (900ms, `pool::lazy_pool`) is shorter
/// than this, so a saturated pool surfaces as a clean [`DbCheckError::Failed`]
/// (`"pool timed out"`) well inside this bound, rather than this timeout racing an
/// acquire that was already failing on its own terms.
pub const DB_CHECK_TIMEOUT: Duration = Duration::from_secs(1);

/// Why [`db_check`] failed. Deliberately coarse -- readiness folds either variant
/// into the same `NotReadyReason::Database`, and neither must ever carry a DSN or
/// the underlying error's `Display`, which can echo back a connection string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbCheckError {
    /// The query did not complete within [`DB_CHECK_TIMEOUT`].
    Timeout,
    /// The query failed. Carries `pool::safe_error_kind`'s fixed, safe description,
    /// never the underlying `sqlx::Error`'s own `Display`.
    Failed(String),
}

impl std::fmt::Display for DbCheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => f.write_str("timed out waiting for a response"),
            Self::Failed(kind) => f.write_str(kind),
        }
    }
}

/// `select 1` bounded to one second (design section 9). Never performs DDL and
/// never returns anything derived from the query's own result -- the caller only
/// needs to know whether the database answered in time.
pub async fn db_check(pool: &PgPool) -> Result<(), DbCheckError> {
    match tokio::time::timeout(DB_CHECK_TIMEOUT, sqlx::query("select 1").execute(pool)).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(DbCheckError::Failed(safe_error_kind(&e))),
        Err(_elapsed) => Err(DbCheckError::Timeout),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_display_is_fixed_and_carries_no_detail() {
        assert_eq!(
            DbCheckError::Timeout.to_string(),
            "timed out waiting for a response"
        );
    }

    #[test]
    fn failed_display_reuses_the_safe_kind_string_verbatim() {
        let err = DbCheckError::Failed("io error".to_owned());
        assert_eq!(err.to_string(), "io error");
    }

    #[tokio::test]
    async fn db_check_fails_without_hanging_against_an_unreachable_database() {
        // Port 1 refuses the connection immediately, so this proves `db_check`
        // reports a failure -- and never panics or hangs -- without needing a real
        // database or `TEST_DATABASE_URL`. The one-second bound itself is exercised
        // by the app crate's `readiness.rs` integration tests, which sever
        // connections to a real, migrated database.
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://u:p@127.0.0.1:1/none")
            .expect("build a lazy pool");

        let err = db_check(&pool)
            .await
            .expect_err("port 1 refuses connections");
        let text = err.to_string();
        assert!(!text.contains("127.0.0.1"), "address leaked: {text}");
        assert!(!text.is_empty());
    }
}
