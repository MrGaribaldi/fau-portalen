//! Ephemeral PostgreSQL databases for integration tests, from the design's section 15.
//!
//! Each test gets its own database, created fresh or from a migrated template, and
//! dropped when the [`TestDb`] goes out of scope. A wrapping transaction was rejected
//! by the design: isolation breaks as soon as the code under test manages its own
//! transactions, which persistence does.
//!
//! Every integration-test binary (one per `tests/*.rs` file) links its own copy of
//! this module and runs in its own process, so template creation is guarded by a
//! PostgreSQL advisory lock on the maintenance database rather than by anything
//! process-local.
//!
//! Each test binary only exercises a subset of these helpers -- `harness_selftest`
//! today, more from Task 4 onward -- so unused items are expected here, not a defect.
#![allow(dead_code)]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use sqlx::{Connection, PgConnection, PgPool};
use uuid::Uuid;

/// The database the harness talks to before any per-test database exists: superuser,
/// used only for `create database` / `drop database` and to hold the template-build
/// advisory lock.
///
/// Port 5433, not 5432: this points at the Compose `db` service published on the
/// host, deliberately distinct from any PostgreSQL an operator already runs there.
const DEFAULT_ADMIN_URL: &str = "postgres://postgres:postgres@127.0.0.1:5433/postgres";

/// Prefix for the migrated template's name. The rest is a content hash (see
/// [`template_name`]), so a changed `roles.sql` or migration set builds a fresh
/// template instead of silently reusing a stale one left in the persistent `db`
/// volume from an earlier state of the repository.
const TEMPLATE_PREFIX: &str = "fau_test_tpl_";

/// Session-level advisory lock id guarding template creation. Arbitrary but fixed:
/// every process across every test binary must pick the same key to contend on.
const TEMPLATE_LOCK: i64 = 0x0FA0_0001;

/// `roles.sql`'s contents, embedded at compile time -- both applied to the template
/// and folded into [`template_name`]'s content hash.
const ROLES_SQL: &str = include_str!("../../../../db/roles.sql");

/// The admin (superuser) connection string, from `TEST_DATABASE_URL` or the default
/// above.
pub fn admin_url() -> String {
    std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| DEFAULT_ADMIN_URL.to_owned())
}

/// A pool connected to the maintenance database as superuser -- for creating and
/// dropping per-test databases.
pub async fn admin_pool() -> PgPool {
    PgPool::connect(&admin_url())
        .await
        .expect("connect to the test-database superuser account")
}

/// Returns `base` with its path replaced by `/db_name`, keeping scheme, credentials,
/// host and port.
fn with_database(base: &str, db_name: &str) -> String {
    let mut url = url::Url::parse(base).expect("TEST_DATABASE_URL must be a valid postgres URL");
    url.set_path(&format!("/{db_name}"));
    url.to_string()
}

/// A fresh, unique database name. UUIDv7 so names sort by creation time and a leaked
/// database is easy to spot among `fau_test_*`.
fn fresh_name() -> String {
    format!("fau_test_{}", Uuid::now_v7().simple())
}

/// `backend/migrations`, resolved from the crate's own manifest directory so it
/// works regardless of the process's current directory.
fn migrations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

/// The migrated template's name: [`TEMPLATE_PREFIX`] plus the first 12 hex digits of
/// a hash over `roles.sql` and every file in `backend/migrations/` (sorted by name,
/// name and contents both hashed). Two processes that would build the same template
/// agree on its name without communicating; a process with different content on disk
/// gets a different name and builds its own rather than reusing a mismatched one.
///
/// `DefaultHasher` is not guaranteed stable across Rust versions, but every test
/// binary in one `cargo test` invocation is built by the same toolchain, which is all
/// this needs -- a hash that changed because of a different compiler only means an
/// unnecessary rebuild of the template, never a wrong (stale) one being reused.
fn template_name() -> String {
    let mut hasher = DefaultHasher::new();
    ROLES_SQL.hash(&mut hasher);

    let dir = migrations_dir();
    let mut file_names: Vec<String> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    file_names.sort();

    for file_name in file_names {
        file_name.hash(&mut hasher);
        let contents = std::fs::read_to_string(dir.join(&file_name)).unwrap_or_default();
        contents.hash(&mut hasher);
    }

    let full = format!("{:016x}", hasher.finish());
    format!("{TEMPLATE_PREFIX}{}", &full[..12])
}

/// One database per test, dropped on scope exit.
pub struct TestDb {
    pub name: String,
    admin_url: String,
}

impl TestDb {
    /// Creates a new, empty database. No roles and no migrations applied.
    pub async fn fresh() -> Self {
        let pool = admin_pool().await;
        let name = fresh_name();
        sqlx::query(&format!(r#"create database "{name}""#))
            .execute(&pool)
            .await
            .expect("create fresh test database");
        Self {
            name,
            admin_url: admin_url(),
        }
    }

    /// Creates a new database from the migrated template, building the template first
    /// if no other process has already done so.
    pub async fn migrated() -> Self {
        let template = ensure_template().await;
        let pool = admin_pool().await;
        let name = fresh_name();
        sqlx::query(&format!(
            r#"create database "{name}" template "{template}""#
        ))
        .execute(&pool)
        .await
        .expect("create test database from template");
        Self {
            name,
            admin_url: admin_url(),
        }
    }

    /// A `DATABASE_URL` for the runtime role `fau_app`, pointed at this database.
    ///
    /// `fau_app` does not exist until Task 5's `roles.sql` runs, and does not accept a
    /// password until `apply_roles` grants it login -- this only builds the string,
    /// present now so later tasks have the interface to build on.
    pub fn url(&self) -> String {
        self.role_url("fau_app")
    }

    /// A `MIGRATION_DATABASE_URL` for the migration role `fau_migrate`, pointed at
    /// this database. Same caveat as [`TestDb::url`].
    pub fn migration_url(&self) -> String {
        self.role_url("fau_migrate")
    }

    fn role_url(&self, role: &str) -> String {
        let mut url = url::Url::parse(&self.admin_url).expect("valid postgres URL");
        let _ = url.set_username(role);
        let _ = url.set_password(Some(role));
        url.set_path(&format!("/{}", self.name));
        url.to_string()
    }

    /// A superuser pool for this database, for assertions and direct SQL. Lazy: no
    /// connection is opened until the pool is first used, so building it cannot fail
    /// on its own.
    pub fn admin_pool(&self) -> PgPool {
        let url = with_database(&self.admin_url, &self.name);
        PgPool::connect_lazy(&url).expect("build lazy admin pool")
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let (url, name) = (self.admin_url.clone(), self.name.clone());
        // A dedicated thread: we may be inside a tokio runtime that is shutting down,
        // and `Drop` cannot await.
        let joined = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("drop runtime");
            rt.block_on(async {
                let pool = match sqlx::PgPool::connect(&url).await {
                    Ok(pool) => pool,
                    Err(_) => {
                        // Never the URL -- it carries a password. Never a panic
                        // either: a leaked database is a nuisance the next
                        // `pg_database` audit catches, not a reason to abort
                        // whatever test triggered this drop.
                        eprintln!(
                            "TestDb: could not connect to drop database {name} -- it may have leaked"
                        );
                        return;
                    }
                };
                let dropped = sqlx::query(&format!(
                    r#"drop database if exists "{name}" with (force)"#
                ))
                .execute(&pool)
                .await;
                if dropped.is_err() {
                    eprintln!("TestDb: failed to drop database {name} -- it may have leaked");
                }
            });
        })
        .join();
        if joined.is_err() {
            eprintln!(
                "TestDb: the drop thread for database {} panicked",
                self.name
            );
        }
    }
}

/// Builds the migrated template exactly once across every concurrently running test
/// binary, using a session-level advisory lock on the maintenance database as the
/// cross-process mutex, held on one pinned connection for the whole critical section
/// (acquire, check, build, unlock) so nothing else can interleave. Returns the
/// template's name for the caller to build from.
async fn ensure_template() -> String {
    let name = template_name();
    let mut conn = PgConnection::connect(&admin_url())
        .await
        .expect("connect to the test-database superuser account");

    sqlx::query("select pg_advisory_lock($1)")
        .bind(TEMPLATE_LOCK)
        .execute(&mut conn)
        .await
        .expect("acquire the template-build advisory lock");

    let exists: bool =
        sqlx::query_scalar("select exists(select 1 from pg_database where datname = $1)")
            .bind(&name)
            .fetch_one(&mut conn)
            .await
            .expect("check whether the template already exists");

    if !exists {
        build_template(&mut conn, &name).await;
        cleanup_old_templates(&mut conn, &name).await;
    }

    let unlocked: bool = sqlx::query_scalar("select pg_advisory_unlock($1)")
        .bind(TEMPLATE_LOCK)
        .fetch_one(&mut conn)
        .await
        .expect("release the template-build advisory lock");
    assert!(
        unlocked,
        "template-build advisory lock was not held by this session -- something is wrong \
         with lock accounting, not with the template itself"
    );

    name
}

/// Builds `name` under a temporary name and only makes it visible under its real name
/// once fully built, via `alter database ... rename to`. A process that crashes
/// mid-build leaves a `..._building` database behind rather than a half-built
/// template masquerading as a complete one -- the next `ensure_template` call (which
/// only runs this while holding the advisory lock, so it cannot race the crashed
/// attempt) clears that leftover before starting its own.
async fn build_template(conn: &mut PgConnection, name: &str) {
    let building = format!("{name}__building");

    let _ = sqlx::query(&format!(
        r#"drop database if exists "{building}" with (force)"#
    ))
    .execute(&mut *conn)
    .await;

    sqlx::query(&format!(r#"create database "{building}""#))
        .execute(&mut *conn)
        .await
        .expect("create template-build database");

    apply_roles_and_migrations(&with_database(&admin_url(), &building)).await;

    sqlx::query(&format!(
        r#"alter database "{building}" rename to "{name}""#
    ))
    .execute(&mut *conn)
    .await
    .expect("rename the completed template into place");
}

/// Opportunistically drops older `fau_test_tpl_*` databases (a previous content hash,
/// or a `..._building` leftover from a crashed build) other than the one just built.
/// Best effort only: a plain `drop database` (no `force`) fails harmlessly if another
/// process still has a session open against one, which is fine -- an old template
/// still in use gets cleaned up on some later run instead. This must never fail the
/// test run that triggered it.
async fn cleanup_old_templates(conn: &mut PgConnection, keep: &str) {
    let stale: Vec<String> = sqlx::query_scalar(
        "select datname from pg_database where datname like $1 and datname <> $2",
    )
    .bind(format!("{TEMPLATE_PREFIX}%"))
    .bind(keep)
    .fetch_all(&mut *conn)
    .await
    .unwrap_or_default();

    for old_name in stale {
        let _ = sqlx::query(&format!(r#"drop database if exists "{old_name}""#))
            .execute(&mut *conn)
            .await;
    }
}

/// Prepares the migrated template: applies `roles.sql` as superuser, then runs
/// pending migrations. Closes its own pool before returning -- `create database ...
/// template ...` fails while any session is still connected to the template, so this
/// cannot rely on drop order.
async fn apply_roles_and_migrations(url: &str) {
    let pool = PgPool::connect(url)
        .await
        .expect("connect to template database");

    sqlx::raw_sql(ROLES_SQL)
        .execute(&pool)
        .await
        .expect("apply roles.sql");

    run_pending_migrations(url).await;

    pool.close().await;
}

/// Applies pending migrations to `migration_url`. Not implemented yet: `fau migrate`
/// currently ends in `todo!()` (see `crates/app/src/main.rs`), so invoking the binary
/// here would only panic. Task 4, which builds the migration runner, replaces this
/// body with a `Command::new(env!("CARGO_BIN_EXE_fau"))` run of `migrate` against
/// `MIGRATION_DATABASE_URL = migration_url`. Nothing exercises `TestDb::migrated()`
/// before then.
async fn run_pending_migrations(_migration_url: &str) {}
