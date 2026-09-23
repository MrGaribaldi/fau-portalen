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

/// Session-level advisory lock id guarding `apply_roles`. `CREATE ROLE` and `ALTER
/// ROLE` touch the shared, cluster-wide `pg_authid` catalog, and PostgreSQL's update
/// path for shared catalogs can raise "tuple concurrently updated" when two sessions
/// modify the same row at the same moment -- observed running the migration tests
/// with several `TestDb::fresh()` databases in parallel, each calling `apply_roles`.
///
/// Unlike `pg_authid` itself, **advisory locks are scoped to the session's current
/// database, not the cluster** (confirmed empirically: two sessions connected to
/// different databases do not contend for the same key). So this only serialises
/// callers that acquire it while connected to the *same* database -- which is why
/// [`apply_roles_sql`] always takes it on a connection to [`admin_url`]'s database,
/// never on a connection to the per-test database it is about to modify.
const ROLES_LOCK: i64 = 0x0FA0_0002;

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

/// Returns `base` pointed at `db_name`, with `role` as both username and password --
/// the harness-wide convention that `apply_roles` establishes on the database side.
fn role_url(base: &str, db_name: &str, role: &str) -> String {
    let mut url = url::Url::parse(base).expect("valid postgres URL");
    let _ = url.set_username(role);
    let _ = url.set_password(Some(role));
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

        // `create database ... template ...` copies the template's schema-level
        // ACLs (pg_namespace/pg_class -- per-database catalogs) but *not* its
        // database-level ACL (pg_database.datacl, a shared/cluster catalog): the
        // clone gets a fresh, default ACL no matter what roles.sql set on the
        // template. Without reapplying it here, a cloned database would silently
        // regain PUBLIC's default TEMPORARY privilege that roles.sql revokes --
        // found by a schema-review test that expected `fau_app` to lack it and
        // didn't.
        apply_roles_sql(&with_database(&admin_url(), &name)).await;

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
        role_url(&self.admin_url, &self.name, role)
    }

    /// A superuser pool for this database, for assertions and direct SQL. Lazy: no
    /// connection is opened until the pool is first used, so building it cannot fail
    /// on its own.
    pub fn admin_pool(&self) -> PgPool {
        let url = with_database(&self.admin_url, &self.name);
        PgPool::connect_lazy(&url).expect("build lazy admin pool")
    }

    /// A pool connected as the runtime role `fau_app` to this database.
    /// `apply_roles` -- called directly, or already applied while the migrated
    /// template was built -- must have run first, or the role has no password to
    /// connect with.
    pub async fn app_pool(&self) -> PgPool {
        PgPool::connect(&self.url())
            .await
            .expect("connect as fau_app")
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

    apply_roles_and_migrations(&admin_url(), &building).await;

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

/// Prepares the migrated template: applies `roles.sql` and grants login as
/// superuser, then runs pending migrations as `fau_migrate`. Closes its own pool
/// before returning -- `create database ... template ...` fails while any session is
/// still connected to the template, so this cannot rely on drop order.
async fn apply_roles_and_migrations(admin_base: &str, db_name: &str) {
    apply_roles_sql(&with_database(admin_base, db_name)).await;

    let migration_url = role_url(admin_base, db_name, "fau_migrate");
    run_pending_migrations(&migration_url).await;
}

/// Runs `roles.sql` (idempotent under a race -- see the file itself) then grants
/// both roles `login` with a password equal to the role name, the convention
/// `role_url` assumes. A test-only convenience: production grants login out of
/// band, never through this file.
///
/// Holds [`ROLES_LOCK`] on a pinned connection to [`admin_url`]'s database for the
/// whole critical section, while the actual `roles.sql` and `alter role` statements
/// run on a second connection to `url` -- the database whose privileges are being
/// set up. Two connections, not one, because the lock must be acquired on a
/// database every caller shares, and `url` is a different database every time.
async fn apply_roles_sql(url: &str) {
    let mut lock_conn = PgConnection::connect(&admin_url())
        .await
        .expect("connect to hold the roles advisory lock");

    sqlx::query("select pg_advisory_lock($1)")
        .bind(ROLES_LOCK)
        .execute(&mut lock_conn)
        .await
        .expect("acquire the roles advisory lock");

    let mut conn = PgConnection::connect(url)
        .await
        .expect("connect to apply roles.sql");

    sqlx::raw_sql(ROLES_SQL)
        .execute(&mut conn)
        .await
        .expect("apply roles.sql");

    for role in ["fau_app", "fau_migrate"] {
        sqlx::query(&format!("alter role {role} login password '{role}'"))
            .execute(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("grant login to role {role}: {e}"));
    }

    // Closed before releasing the lock, not after: the whole point is that no other
    // session may run these statements while this one still could be.
    conn.close()
        .await
        .expect("close connection after applying roles.sql");

    let unlocked: bool = sqlx::query_scalar("select pg_advisory_unlock($1)")
        .bind(ROLES_LOCK)
        .fetch_one(&mut lock_conn)
        .await
        .expect("release the roles advisory lock");
    assert!(
        unlocked,
        "roles advisory lock was not held by this session -- something is wrong with \
         lock accounting, not with the roles themselves"
    );
    lock_conn
        .close()
        .await
        .expect("close the roles-lock connection");
}

/// Applies `roles.sql` to `db`'s own database as superuser, then grants both roles
/// `login`. Needed before running `fau migrate` (or connecting as `fau_app`) against
/// a database that was not built from the migrated template, e.g. `TestDb::fresh()`.
/// Idempotent, so calling it again on an already-prepared database is harmless.
pub async fn apply_roles(db: &TestDb) {
    apply_roles_sql(&with_database(&db.admin_url, &db.name)).await;
}

/// Runs `fau migrate` against `migration_url` (the `fau_migrate` DSN for the
/// database being prepared) and panics on failure. Used only while building the
/// migrated template, where a failure means the template itself is broken and every
/// test that would use it must not silently proceed.
async fn run_pending_migrations(migration_url: &str) {
    let out = fau_command("migrate", &[("MIGRATION_DATABASE_URL", migration_url)])
        .output()
        .await
        .expect("run fau migrate while building the template");
    assert!(
        out.status.success(),
        "fau migrate failed while building the template: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A `Command` for the `fau` binary under test, with a clean environment (`PATH`
/// only) plus `env`, so no ambient variable from the harness's own process leaks
/// into what is meant to be an isolated run.
fn fau_command(subcommand: &str, env: &[(&str, &str)]) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fau"));
    cmd.arg(subcommand)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default());
    for (key, value) in env {
        cmd.env(key, value);
    }
    cmd
}

/// Runs `fau migrate` against `db`'s `fau_migrate` URL and returns the process
/// output. Requires `apply_roles(db)` (or an already-migrated template) to have run
/// first, since the role must exist and accept the harness's test password.
pub async fn run_fau_migrate(db: &TestDb) -> std::process::Output {
    run_fau_migrate_with(db, &[]).await
}

/// As [`run_fau_migrate`], with extra environment variables layered on top of the
/// runner's defaults -- e.g. a short `MIGRATION_LOCK_WAIT_MS` for the bounded-lock
/// test.
pub async fn run_fau_migrate_with(db: &TestDb, extra_env: &[(&str, &str)]) -> std::process::Output {
    let migration_url = db.migration_url();
    let mut cmd = fau_command(
        "migrate",
        &[("MIGRATION_DATABASE_URL", migration_url.as_str())],
    );
    for (key, value) in extra_env {
        cmd.env(key, value);
    }
    cmd.output().await.expect("run fau migrate")
}

/// Runs `fau serve` against `db`'s `fau_app` URL and waits for it to exit, bounded by
/// a timeout. `serve` currently ends in `todo!()` right after loading its
/// configuration, so this observes a failure today either way -- the point of the
/// test that uses it is only that no schema object appears, not the exit code.
pub async fn start_serve_expecting_failure(db: &TestDb) -> std::process::Output {
    let database_url = db.url();
    let mut cmd = fau_command(
        "serve",
        &[
            ("APP_ENV", "test"),
            ("HTTP_BIND", "127.0.0.1:0"),
            ("PUBLIC_BASE_URL", "http://localhost:8000"),
            ("DATABASE_URL", database_url.as_str()),
            ("DB_POOL_MAX_CONNECTIONS", "5"),
            ("LOG_LEVEL", "info"),
        ],
    );
    // The `output()` future is wrapped in a timeout below and may be dropped before
    // the child exits; without this, dropping it would orphan the process instead
    // of killing it.
    cmd.kill_on_drop(true);
    tokio::time::timeout(std::time::Duration::from_secs(10), cmd.output())
        .await
        .expect("fau serve did not exit within the timeout")
        .expect("run fau serve")
}
