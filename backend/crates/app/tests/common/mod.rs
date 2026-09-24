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
//! Each test binary only exercises a subset of these helpers, so unused items are
//! expected here, not a defect.
#![allow(dead_code)]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use sqlx::{Connection, PgConnection, PgPool};
use tokio::io::AsyncBufReadExt;
use tokio::sync::Mutex as TokioMutex;
use uuid::Uuid;

pub mod membership;

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
        // regain PUBLIC's default TEMPORARY privilege that roles.sql revokes, which
        // `tests/schema_review.rs` checks `fau_app` lacks.
        apply_roles_sql(&with_database(&admin_url(), &name)).await;

        Self {
            name,
            admin_url: admin_url(),
        }
    }

    /// A `DATABASE_URL` for the runtime role `fau_app`, pointed at this database.
    ///
    /// `fau_app` does not exist until `db/roles.sql` runs, and does not accept a
    /// password until `apply_roles` grants it login -- this only builds the string.
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

    /// Simulates a database outage for readiness tests: sets the
    /// connection limit to zero, revokes `connect` from the runtime role and from
    /// `public`, then terminates every other backend already connected to this
    /// database. Superuser admin connections (used by [`TestDb::admin_pool`] and by
    /// this method itself) are unaffected -- PostgreSQL exempts superusers from both
    /// the connection limit and `connect` privilege checks.
    pub async fn sever_connections(&self) {
        let pool = self.admin_pool();
        sqlx::query(&format!(
            r#"alter database "{}" connection limit 0"#,
            self.name
        ))
        .execute(&pool)
        .await
        .expect("set connection limit to 0");
        sqlx::query(&format!(
            r#"revoke connect on database "{}" from fau_app"#,
            self.name
        ))
        .execute(&pool)
        .await
        .expect("revoke connect from fau_app");
        sqlx::query(&format!(
            r#"revoke connect on database "{}" from public"#,
            self.name
        ))
        .execute(&pool)
        .await
        .expect("revoke connect from public");
        sqlx::query(
            "select pg_terminate_backend(pid) from pg_stat_activity \
             where datname = $1 and pid <> pg_backend_pid()",
        )
        .bind(&self.name)
        .execute(&pool)
        .await
        .expect("terminate other backends on this database");
    }

    /// Reverses [`TestDb::sever_connections`].
    pub async fn restore_connections(&self) {
        let pool = self.admin_pool();
        sqlx::query(&format!(
            r#"alter database "{}" connection limit -1"#,
            self.name
        ))
        .execute(&pool)
        .await
        .expect("restore connection limit");
        sqlx::query(&format!(
            r#"grant connect on database "{}" to fau_app"#,
            self.name
        ))
        .execute(&pool)
        .await
        .expect("restore connect for fau_app");
        sqlx::query(&format!(
            r#"grant connect on database "{}" to public"#,
            self.name
        ))
        .execute(&pool)
        .await
        .expect("restore connect for public");
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

    assert_migrations_are_current(&building).await;

    sqlx::query(&format!(
        r#"alter database "{building}" rename to "{name}""#
    ))
    .execute(&mut *conn)
    .await
    .expect("rename the completed template into place");
}

/// Defence in depth against `sqlx::migrate!`'s inability to notice a *new*
/// migration file on stable Rust (see `persistence/build.rs`'s doc comment for the
/// full mechanism): before a freshly built template is renamed into place, confirms
/// the `fau` binary that just ran actually applied one row per `.sql` file present
/// in `migrations/` right now. Without `build.rs` forcing a rebuild, a binary built
/// before a migration file was added would compile as `Fresh`, run against the new
/// template-build database, and apply only the migrations it was actually compiled
/// with -- silently producing a template missing the newest migration's schema, and
/// every test built from it would then fail downstream with a confusing "relation
/// does not exist" instead of pointing at the real, stale-binary cause. This assert
/// makes that impossible to miss: a stale template can never be renamed into place.
async fn assert_migrations_are_current(db_name: &str) {
    let url = with_database(&admin_url(), db_name);
    let mut conn = PgConnection::connect(&url)
        .await
        .expect("connect to the freshly built template to verify its migrations");

    let applied: i64 = sqlx::query_scalar("select count(*) from _sqlx_migrations")
        .fetch_one(&mut conn)
        .await
        .expect("count applied migrations on the freshly built template");

    let on_disk = std::fs::read_dir(migrations_dir())
        .expect("read the migrations directory")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("sql"))
        .count() as i64;

    assert_eq!(
        applied, on_disk,
        "the fau binary's embedded migrations are stale: it applied {applied} \
         migration(s) but {on_disk} *.sql file(s) exist in migrations/ right now. \
         `sqlx::migrate!` embeds each file via `include_str!` at compile time and \
         does not notice a newly added file on stable Rust, so a binary built before \
         this file appeared silently keeps running the old, smaller migration set. \
         Rebuild the fau binary (`cargo build -p fau-app`) and re-run -- \
         persistence/build.rs's `cargo:rerun-if-changed` on migrations/ should force \
         this automatically; if this assertion still fires, that mechanism itself is \
         broken."
    );
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

/// Binds a fresh, unused TCP port: bind to `127.0.0.1:0`, read back the port the
/// kernel picked, then drop the listener. `HTTP_BIND=127.0.0.1:0` is not usable
/// directly for a test that needs to know the port up front (the bound address is
/// not observable after `serve` binds it), so the harness picks the port itself
/// instead.
fn free_port() -> u16 {
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port for fau serve");
    listener
        .local_addr()
        .expect("read the ephemeral port back")
        .port()
}

/// The default environment `spawn_serve` starts `fau serve` with, keyed on `db` and
/// a pre-chosen port: `APP_ENV=development`, `DATABASE_URL` for the
/// runtime role `fau_app`, `PUBLIC_BASE_URL` pointed at the chosen port.
fn default_serve_env(db: &TestDb, port: u16) -> Vec<(&'static str, String)> {
    vec![
        ("APP_ENV", "development".to_owned()),
        ("HTTP_BIND", format!("127.0.0.1:{port}")),
        ("PUBLIC_BASE_URL", format!("http://localhost:{port}")),
        ("DATABASE_URL", db.url()),
        ("DB_POOL_MAX_CONNECTIONS", "5".to_owned()),
        ("LOG_LEVEL", "info".to_owned()),
    ]
}

/// Polls `make_request` until it returns a response satisfying `predicate`, or
/// `timeout` elapses. A failed request (e.g. connection refused while the server is
/// still starting) counts as "not yet", not as an error, so this can be used from
/// the moment a process is spawned.
pub async fn poll_until<F, Fut>(
    mut make_request: F,
    predicate: impl Fn(&reqwest::Response) -> bool,
    timeout: Duration,
) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = reqwest::Result<reqwest::Response>>,
{
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Ok(response) = make_request().await {
            if predicate(&response) {
                return true;
            }
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The fixed text `crate::http::test_routes::slow` logs, synchronously, as the very
/// first thing it does -- before its 2-second sleep. Cannot be a `pub const` shared
/// with `src/http/test_routes.rs`: `fau-app` is a binary-only crate (no `lib.rs`),
/// so a test binary can only observe it as a subprocess's captured stdout, never by
/// linking against its source. Kept identical to the literal in that handler by
/// hand; if the two ever drift, [`wait_for_log`] simply never finds it and the test
/// using it times out loudly rather than passing on a stale assumption.
pub const SLOW_STARTED_MARKER: &str = "test_routes: /test/slow started";

/// As [`SLOW_STARTED_MARKER`], for `crate::http::test_routes::slow_write` -- logged
/// before it opens its transaction.
pub const SLOW_WRITE_STARTED_MARKER: &str = "test_routes: /test/slow-write started";

/// Polls `app`'s captured stdout (`ServeHandle::captured_stdout`) until some line
/// contains `needle`, or `timeout` elapses.
///
/// A bare `tokio::time::sleep` before acting on "the request must have reached the
/// server by now" is a guess, not a proof -- `get_async`'s returned future does not
/// touch the wire until it is actually polled, so nothing is guaranteed to have
/// started merely because some wall-clock time passed while an unrelated future sat
/// unpolled. Waiting for the handler's own started-marker log line instead proves
/// the request was received and dispatched, which is what a test claiming to
/// exercise an *in-flight* request needs.
pub async fn wait_for_log(app: &ServeHandle, needle: &str, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if app
            .captured_stdout()
            .iter()
            .any(|line| line.contains(needle))
        {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Reads `reader` line by line, appending each line to `sink`, until the pipe
/// closes (the child exited). Spawned rather than awaited -- [`ServeHandle`] must
/// be usable while its child is still running -- but the returned [`JoinHandle`]
/// is what lets [`ServeHandle::wait`] prove the capture has actually reached EOF
/// rather than merely read whatever happened to arrive before `child.wait()`
/// returned.
fn spawn_line_capture(
    reader: impl tokio::io::AsyncRead + Unpin + Send + 'static,
    sink: Arc<StdMutex<Vec<String>>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut lines = tokio::io::BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            sink.lock().expect("captured-output lock").push(line);
        }
    })
}

/// A running `fau serve` process, bound to an ephemeral port and pointed at a
/// [`TestDb`]. Kills the child on drop (`kill_on_drop`), so a panicking test never
/// leaves a `fau` process behind.
pub struct ServeHandle {
    port: u16,
    client: reqwest::Client,
    child: TokioMutex<tokio::process::Child>,
    stdout_lines: Arc<StdMutex<Vec<String>>>,
    stderr_lines: Arc<StdMutex<Vec<String>>>,
    /// The two [`spawn_line_capture`] tasks, taken and joined exactly once by
    /// [`ServeHandle::wait`] -- see that method's doc comment for why this matters.
    stdout_task: StdMutex<Option<tokio::task::JoinHandle<()>>>,
    stderr_task: StdMutex<Option<tokio::task::JoinHandle<()>>>,
}

impl ServeHandle {
    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    /// A bare `GET` against this server.
    pub async fn get(&self, path: &str) -> reqwest::Response {
        self.client
            .get(self.url(path))
            .send()
            .await
            .expect("request to fau serve")
    }

    /// A `GET` with extra request headers.
    pub async fn get_with_headers(
        &self,
        path: &str,
        headers: &[(&str, &str)],
    ) -> reqwest::Response {
        let mut req = self.client.get(self.url(path));
        for (name, value) in headers {
            req = req.header(*name, *value);
        }
        req.send().await.expect("request to fau serve")
    }

    /// A `GET` as a bare, un-awaited future -- for [`poll_until`], which needs to
    /// issue a fresh request on every iteration.
    pub fn get_async(
        &self,
        path: &str,
    ) -> impl std::future::Future<Output = reqwest::Result<reqwest::Response>> + '_ {
        self.client.get(self.url(path)).send()
    }

    /// Whether the child process is still running.
    pub async fn is_running(&self) -> bool {
        let mut child = self.child.lock().await;
        matches!(child.try_wait(), Ok(None))
    }

    /// Sends `SIGTERM` to the child, the same signal a container runtime sends on
    /// shutdown (design section 12).
    pub async fn send_sigterm(&self) {
        let child = self.child.lock().await;
        let pid = child.id().expect("child has not already been waited on");
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid as i32),
            nix::sys::signal::Signal::SIGTERM,
        )
        .expect("send SIGTERM to fau serve");
    }

    /// Waits for the child to exit and returns its status. Also joins both
    /// stdout/stderr capture tasks before returning: they end at EOF, which the
    /// pipes only reach once the child (the write end) has actually exited, so by
    /// the time this returns, [`ServeHandle::captured_stdout`] and
    /// [`ServeHandle::captured_stderr`] are guaranteed complete -- not racing
    /// whatever had been read so far, which the logging and shutdown tests depend
    /// on.
    pub async fn wait(&self) -> std::process::ExitStatus {
        let status = {
            let mut child = self.child.lock().await;
            child.wait().await.expect("wait for fau serve to exit")
        };

        // The lock is taken, `take()`n and dropped in the `let` itself -- never held
        // across the `.await` below, which a `std::sync::MutexGuard` cannot survive
        // in an async fn without making the whole future non-`Send`.
        let stdout_task = self.stdout_task.lock().expect("stdout-task lock").take();
        if let Some(task) = stdout_task {
            let _ = task.await;
        }
        let stderr_task = self.stderr_task.lock().expect("stderr-task lock").take();
        if let Some(task) = stderr_task {
            let _ = task.await;
        }

        status
    }

    /// Every stdout line captured so far, in order. Logging tests read this rather
    /// than re-spawning the process.
    pub fn captured_stdout(&self) -> Vec<String> {
        self.stdout_lines
            .lock()
            .expect("captured-stdout lock")
            .clone()
    }

    /// As [`ServeHandle::captured_stdout`], for stderr.
    pub fn captured_stderr(&self) -> Vec<String> {
        self.stderr_lines
            .lock()
            .expect("captured-stderr lock")
            .clone()
    }
}

/// Starts `fau serve` against `db` with the default environment and
/// waits for `/health/live` to answer, bounded to about ten seconds.
pub async fn spawn_serve(db: &TestDb) -> ServeHandle {
    spawn_serve_with_env(db, &[]).await
}

/// An alias for [`spawn_serve`], used by tests that exercise a `test-routes`-only
/// endpoint (e.g. `/test/slow`, `/test/panic`). There is nothing different to set up:
/// a Cargo feature applies to the whole compiled unit, so `cargo test --features
/// test-routes ...` already builds the `fau` binary under test (`CARGO_BIN_EXE_fau`,
/// spawned by [`fau_command`]) with the feature on, the same as the test binary
/// itself. This name exists only to document that at the call site, not because the
/// spawning differs.
pub async fn spawn_serve_with_test_routes(db: &TestDb) -> ServeHandle {
    spawn_serve(db).await
}

/// As [`spawn_serve`], with `extra_env` layered on top of the defaults -- a later
/// duplicate key overrides the default, matching `std::process::Command::env`'s
/// last-one-wins behaviour.
pub async fn spawn_serve_with_env(db: &TestDb, extra_env: &[(&str, &str)]) -> ServeHandle {
    let port = free_port();
    let default_env = default_serve_env(db, port);
    let mut env: Vec<(&str, &str)> = default_env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    env.extend_from_slice(extra_env);

    let mut cmd = fau_command("serve", &env);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.kill_on_drop(true);

    let mut child = cmd.spawn().expect("spawn fau serve");
    let stdout = child.stdout.take().expect("fau serve stdout is piped");
    let stderr = child.stderr.take().expect("fau serve stderr is piped");

    let stdout_lines = Arc::new(StdMutex::new(Vec::new()));
    let stderr_lines = Arc::new(StdMutex::new(Vec::new()));
    let stdout_task = spawn_line_capture(stdout, stdout_lines.clone());
    let stderr_task = spawn_line_capture(stderr, stderr_lines.clone());

    let client = reqwest::Client::builder()
        .build()
        .expect("build reqwest client");

    let handle = ServeHandle {
        port,
        client,
        child: TokioMutex::new(child),
        stdout_lines,
        stderr_lines,
        stdout_task: StdMutex::new(Some(stdout_task)),
        stderr_task: StdMutex::new(Some(stderr_task)),
    };

    wait_for_live(&handle, Duration::from_secs(10)).await;

    handle
}

/// As [`spawn_serve`], but with the runtime `DATABASE_URL`'s password replaced by
/// `SENTINEL_DB_PASSWORD` -- a wrong password against the real, migrated test
/// database, so a connection attempt genuinely fails (SQLSTATE 28P01) rather than
/// merely going untried. The sentinel-secrets logging tests use this to prove a DSN's
/// password never reaches log output even on the classic path a leak has
/// historically come from: a failed connection's own error message. Hits
/// `/health/ready` once before returning -- forcing that failure to actually happen
/// here, rather than depending on the caller to trigger it.
pub async fn spawn_serve_with_sentinels(db: &TestDb) -> ServeHandle {
    let app = spawn_serve_with_sentinels_and_env(db, &[]).await;
    app.get("/health/ready").await;
    app
}

/// As [`spawn_serve_with_sentinels`], with `extra_env` layered on top -- e.g.
/// `LOG_LEVEL=warn` -- and, unlike it, **no** internal `/health/ready` call: a
/// caller that needs to know exactly which request produced a given log line (the
/// test proving a readiness `WARN` carries the *calling* request's own id) needs its own call to be the only one, since `readiness`'s own warning is
/// itself rate-limited to one line per ready-to-not-ready transition -- a second
/// probe against the same still-failing database would not log again at all.
pub async fn spawn_serve_with_sentinels_and_env(
    db: &TestDb,
    extra_env: &[(&str, &str)],
) -> ServeHandle {
    let mut dsn = url::Url::parse(&db.url()).expect("TestDb::url is a valid postgres URL");
    dsn.set_password(Some("SENTINEL_DB_PASSWORD"))
        .expect("set a sentinel password on the DSN");
    let sentinel_dsn = dsn.to_string();

    let mut env: Vec<(&str, &str)> = vec![("DATABASE_URL", sentinel_dsn.as_str())];
    env.extend_from_slice(extra_env);

    spawn_serve_with_env(db, &env).await
}

/// Waits for `/health/live` to answer, bounded by `timeout`. Checks
/// [`ServeHandle::is_running`] on every iteration and once more immediately after a
/// successful response, panicking with the captured stderr rather than either
/// stalling for the full timeout or -- the more dangerous failure -- silently
/// accepting a `200` from some *other* process that happens to already be listening
/// on the same ephemeral port (a real risk: the port is chosen by binding, reading
/// it back and dropping the listener, which leaves a window for a raced bind by
/// another concurrently-starting `fau serve`).
async fn wait_for_live(handle: &ServeHandle, timeout: Duration) {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if !handle.is_running().await {
            panic!(
                "fau serve exited before answering /health/live; captured stderr:\n{}",
                handle.captured_stderr().join("\n")
            );
        }
        if let Ok(res) = handle.get_async("/health/live").await {
            if res.status().is_success() {
                break;
            }
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "fau serve did not answer /health/live within {timeout:?}; captured stderr:\n{}",
                handle.captured_stderr().join("\n")
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    if !handle.is_running().await {
        panic!(
            "fau serve exited immediately after answering /health/live; captured stderr:\n{}",
            handle.captured_stderr().join("\n")
        );
    }
}

/// Runs `fau serve` against `db` and waits for it to exit on its own, bounded by a
/// timeout that also kills the process if it does not (`kill_on_drop`, same
/// mechanism as [`run_fau_migrate`]). For a scenario where `serve` is expected to
/// refuse to start (e.g. a schema contract below its minimum) rather than bind and
/// run -- unlike [`spawn_serve`], this never waits for `/health/live`.
pub async fn run_fau_serve_until_exit(db: &TestDb) -> std::process::Output {
    let port = free_port();
    let default_env = default_serve_env(db, port);
    let env: Vec<(&str, &str)> = default_env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let mut cmd = fau_command("serve", &env);
    cmd.kill_on_drop(true);
    tokio::time::timeout(Duration::from_secs(10), cmd.output())
        .await
        .expect("fau serve did not exit within the timeout")
        .expect("run fau serve")
}
