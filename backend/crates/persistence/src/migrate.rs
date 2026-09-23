//! The migration runner: a bounded outer lock, then sqlx's own migrator.
//!
//! `_sqlx_migrations` answers *which files have run* (design section 5); sqlx's
//! migrator itself takes a PostgreSQL advisory lock for the whole run via the
//! blocking `pg_advisory_lock`. That call *is* bounded by the connection's
//! `lock_timeout` -- confirmed against a live server: under `lock_timeout = 500ms`
//! a contended `pg_advisory_lock` is cancelled after ~0.52s with "canceling
//! statement due to lock timeout", the same as any other lock wait.
//!
//! That is exactly the problem, not the fix: `lock_timeout` is one setting per
//! connection, but a migration run needs two *different* bounds --
//!
//! - `MIGRATION_LOCK_WAIT_MS` (default 30s): how long it is normal to queue behind
//!   another FAU migrator already running, e.g. during a rolling deploy.
//! - `MIGRATION_LOCK_TIMEOUT_MS` (default 10s): how long a single DDL statement
//!   inside a migration may wait on a table lock held by live traffic, protecting
//!   production queries from a migration that would otherwise queue behind them for
//!   as long as the run-level wait allows.
//!
//! Reusing `lock_timeout` for both would force them to the same number, which is
//! wrong either way: 10s is too impatient for a routine rolling deploy, and 30s is
//! too tolerant of a migration stuck behind live traffic. So this runner takes its
//! own outer lock first, with `pg_try_advisory_lock` -- non-blocking, so it is never
//! subject to `lock_timeout` at all -- polled against a deadline we control from
//! `MIGRATION_LOCK_WAIT_MS`. Only then does it hand off to sqlx, whose internal
//! `pg_advisory_lock` is left bounded by `lock_timeout` (`MIGRATION_LOCK_TIMEOUT_MS`)
//! purely as a defence-in-depth backstop: by the time sqlx reaches it, the outer
//! lock has already guaranteed no other FAU migrator holds it, so it should never
//! actually have to wait. The outer lock also gives a named [`MigrateError::LockUnavailable`]
//! instead of whatever sqlx's own timeout error looks like.

use std::time::{Duration, Instant};

use sqlx::postgres::PgConnectOptions;
use sqlx::{Connection, PgConnection};

use crate::pool::{self, ConnectErrorKind};

/// Outer lock for the whole migration run, shared by every FAU migration process --
/// exported so the test suite can contend on the same key to prove the bounded
/// wait. Arbitrary but fixed: "FAU0" packed into the high bytes, then a sequence.
pub const MIGRATION_LOCK_ID: i64 = 0x4641_5530_0000_0001;

/// Bounds the initial TCP/TLS connection attempt itself, before any lock is even in
/// play -- independent of `lock_wait_ms`/`lock_timeout_ms`, both of which only apply
/// once a connection already exists. Without this, an address that silently drops
/// packets (no RST, no ICMP) could hang past every other configured bound.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Settings for one migration run. Deliberately not `MigrateConfig` itself:
/// `persistence` does not depend on `app`, so this keeps the runner testable with
/// values that never touch the environment.
#[derive(Clone)]
pub struct MigrationSettings {
    pub url: String,
    pub lock_timeout_ms: u64,
    pub lock_wait_ms: u64,
}

/// Hand-written so `url` -- which carries a password -- is never printed. A derived
/// `Debug` would print the plain `String` in full, defeating the point of every
/// other redaction in this crate.
impl std::fmt::Debug for MigrationSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MigrationSettings")
            .field("url", &"[redacted]")
            .field("lock_timeout_ms", &self.lock_timeout_ms)
            .field("lock_wait_ms", &self.lock_wait_ms)
            .finish()
    }
}

/// What a successful run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Applied {
    pub applied: usize,
    pub already_current: bool,
}

/// Every reason a run can fail. `Display` never repeats a DSN or a password, a
/// query, or any other value the caller or the database supplied -- `Connect`
/// carries only a fixed classification (see [`pool::ConnectErrorKind`]), and `Sql`
/// carries the SQLSTATE code where the database gave one (a fixed enumeration, safe
/// to surface) or a fixed classification otherwise, plus the migration version
/// where sqlx's own error identifies one -- also just a number from our own
/// filenames, never anything a caller supplied.
#[derive(Debug, thiserror::Error)]
pub enum MigrateError {
    #[error("migration lock unavailable")]
    LockUnavailable,
    #[error("migration {version} has a changed checksum")]
    ChecksumMismatch { version: i64 },
    #[error("could not connect to the migration database ({0})")]
    Connect(ConnectErrorKind),
    #[error("migration failed ({0})")]
    Sql(String),
}

fn sql_error(e: sqlx::Error) -> MigrateError {
    MigrateError::Sql(sql_error_kind(&e))
}

/// A diagnosable but safe description of a `sqlx::Error`: the SQLSTATE when the
/// database gave one, otherwise a fixed word for the kind of failure. Never the
/// error's own `Display`, which can quote the failing statement or a bound value.
fn sql_error_kind(e: &sqlx::Error) -> String {
    match e {
        sqlx::Error::Database(db_err) => match db_err.code() {
            Some(code) => format!("sqlstate {code}"),
            None => "database error".to_owned(),
        },
        sqlx::Error::Io(_) => "io error".to_owned(),
        sqlx::Error::PoolTimedOut => "pool timed out".to_owned(),
        sqlx::Error::PoolClosed => "pool closed".to_owned(),
        _ => "query failed".to_owned(),
    }
}

fn map_migrate_error(e: sqlx::migrate::MigrateError) -> MigrateError {
    match e {
        sqlx::migrate::MigrateError::VersionMismatch(version) => {
            MigrateError::ChecksumMismatch { version }
        }
        sqlx::migrate::MigrateError::Execute(inner) => sql_error(inner),
        other => MigrateError::Sql(migrate_error_kind(&other)),
    }
}

/// As [`sql_error_kind`], for the migrator's own errors: a fixed description that
/// includes the migration version wherever sqlx's error identifies one -- a number
/// from our own filenames, not a value from the database or the caller.
fn migrate_error_kind(e: &sqlx::migrate::MigrateError) -> String {
    match e {
        sqlx::migrate::MigrateError::VersionMissing(v) => {
            format!("applied migration {v} missing on disk")
        }
        sqlx::migrate::MigrateError::VersionNotPresent(v) => {
            format!("migration {v} not in source")
        }
        sqlx::migrate::MigrateError::VersionTooOld(v, latest) => {
            format!("migration {v} older than latest applied {latest}")
        }
        sqlx::migrate::MigrateError::VersionTooNew(v, latest) => {
            format!("migration {v} newer than latest applied {latest}")
        }
        sqlx::migrate::MigrateError::Dirty(v) => format!("migration {v} partially applied"),
        _ => "migration failed".to_owned(),
    }
}

/// Polls `pg_try_advisory_lock` against a deadline we control (`wait`, from
/// `MIGRATION_LOCK_WAIT_MS`). `pg_try_advisory_lock` never blocks -- it returns
/// immediately whether or not the lock was granted -- so, unlike the blocking
/// `pg_advisory_lock` sqlx's own migrator takes internally once we hand off, it is
/// never subject to the connection's `lock_timeout`. That is what lets the run-level
/// queueing wait here and the DDL `lock_timeout` bound be different numbers instead
/// of one shared setting; see this module's docs for why they must differ.
async fn acquire_outer_lock(conn: &mut PgConnection, wait: Duration) -> Result<(), MigrateError> {
    let deadline = Instant::now() + wait;
    loop {
        let got: bool = sqlx::query_scalar("select pg_try_advisory_lock($1)")
            .bind(MIGRATION_LOCK_ID)
            .fetch_one(&mut *conn)
            .await
            .map_err(sql_error)?;
        if got {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(MigrateError::LockUnavailable);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Best effort: the session ending also releases the lock (a crashed process
/// releases it automatically), but a pooled connection would otherwise keep
/// holding it after a successful run, so this releases it explicitly on every
/// path. A failure here must never mask the run's actual result.
async fn release_outer_lock(conn: &mut PgConnection) {
    let _ = sqlx::query("select pg_advisory_unlock($1)")
        .bind(MIGRATION_LOCK_ID)
        .execute(conn)
        .await;
}

/// Whether `_sqlx_migrations` exists yet, and its row count if so -- 0 on a
/// database that has never been migrated, since the table itself does not exist
/// before the first successful run.
async fn count_applied(conn: &mut PgConnection) -> Result<i64, MigrateError> {
    let exists: bool = sqlx::query_scalar("select to_regclass('_sqlx_migrations') is not null")
        .fetch_one(&mut *conn)
        .await
        .map_err(sql_error)?;
    if !exists {
        return Ok(0);
    }
    sqlx::query_scalar("select count(*) from _sqlx_migrations")
        .fetch_one(conn)
        .await
        .map_err(sql_error)
}

/// Opens the migration connection under [`CONNECT_TIMEOUT`], separate from and
/// tighter than `lock_wait_ms`/`lock_timeout_ms` -- both apply only once a
/// connection exists, so neither would catch an address that never answers at all.
async fn connect(opts: &PgConnectOptions) -> Result<PgConnection, MigrateError> {
    match tokio::time::timeout(CONNECT_TIMEOUT, PgConnection::connect_with(opts)).await {
        Ok(Ok(conn)) => Ok(conn),
        Ok(Err(e)) => Err(MigrateError::Connect(pool::classify_connect_error(&e))),
        Err(_elapsed) => Err(MigrateError::Connect(ConnectErrorKind::Timeout)),
    }
}

/// Applies every pending migration in `../../migrations` (resolved from this
/// crate's manifest directory, i.e. `backend/migrations`), under the bounded outer
/// lock. Idempotent: running it again against an up-to-date database succeeds and
/// reports `already_current`.
pub async fn run_migrations(cfg: &MigrationSettings) -> Result<Applied, MigrateError> {
    let opts = pool::connect_options(
        &cfg.url,
        [("lock_timeout", cfg.lock_timeout_ms.to_string())],
    )
    .map_err(MigrateError::Connect)?;

    let mut conn = connect(&opts).await?;

    acquire_outer_lock(&mut conn, Duration::from_millis(cfg.lock_wait_ms)).await?;

    // Split out so the outer lock is released on every path below, including a
    // migration failure -- a `?` inline here would skip that and hold the lock
    // until the pooled connection eventually drops.
    let result = run_pending(&mut conn).await;

    release_outer_lock(&mut conn).await;

    result
}

/// Runs the embedded migrations and reports how many were newly applied.
async fn run_pending(conn: &mut PgConnection) -> Result<Applied, MigrateError> {
    let before = count_applied(conn).await?;

    sqlx::migrate!("../../migrations")
        .run(&mut *conn)
        .await
        .map_err(map_migrate_error)?;

    let after = count_applied(conn).await?;
    let applied = after.saturating_sub(before) as usize;

    Ok(Applied {
        applied,
        already_current: applied == 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_settings_debug_redacts_the_url() {
        let cfg = MigrationSettings {
            url: "postgres://u:hunter2@127.0.0.1:5432/db".to_owned(),
            lock_timeout_ms: 10_000,
            lock_wait_ms: 30_000,
        };
        let debug = format!("{cfg:?}");
        assert!(!debug.contains("hunter2"), "password leaked: {debug}");
        assert!(!debug.contains("127.0.0.1"), "host leaked: {debug}");
        assert!(debug.contains("[redacted]"), "unexpected output: {debug}");
        // The non-secret fields are still useful for diagnostics.
        assert!(debug.contains("10000"));
        assert!(debug.contains("30000"));
    }
}
