//! The migration runner and its bounded lock, from the design's section 5 and the
//! test plan in section 15.

mod common;
use common::TestDb;

#[tokio::test]
async fn migrate_applies_and_is_idempotent() {
    let db = TestDb::fresh().await;
    common::apply_roles(&db).await;

    let first = common::run_fau_migrate(&db).await;
    assert!(first.status.success(), "{:?}", first);

    let second = common::run_fau_migrate(&db).await;
    assert!(second.status.success(), "re-running migrate must succeed");

    let version: i32 = sqlx::query_scalar("select max(version) from schema_contract")
        .fetch_one(&db.admin_pool())
        .await
        .unwrap();
    assert_eq!(
        version,
        common::migration_file_count(),
        "contract version after all migrations"
    );
}

#[tokio::test]
async fn altered_checksum_on_an_applied_migration_fails() {
    let db = TestDb::migrated().await;
    // Corrupt the recorded checksum, which is how sqlx detects an edited file.
    sqlx::query("update _sqlx_migrations set checksum = decode('00', 'hex') where version = 1")
        .execute(&db.admin_pool())
        .await
        .unwrap();

    let out = common::run_fau_migrate(&db).await;
    assert!(
        !out.status.success(),
        "migrate must refuse a changed checksum"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("changed checksum"),
        "expected a checksum-mismatch message, got: {stderr}"
    );
}

/// A database already at version 1 -- migrated by an older binary that shipped only
/// 0001 -- is brought forward by applying 0002 and 0003, leaving 0001's record alone.
#[tokio::test]
async fn migrate_completes_a_partly_migrated_database() {
    let db = TestDb::fresh().await;
    common::apply_roles(&db).await;

    // A migration source holding a byte-for-byte copy of 0001 only, so its checksum
    // matches the one the binary embeds.
    let dir = std::env::temp_dir().join(format!("fau-partial-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../migrations/0001_schema_contract.sql"
    );
    std::fs::copy(source, dir.join("0001_schema_contract.sql")).unwrap();

    let migrator = sqlx::migrate::Migrator::new(dir.as_path())
        .await
        .expect("read the one-file migration source");
    let migrate_pool = sqlx::PgPool::connect(&db.migration_url())
        .await
        .expect("connect as fau_migrate");
    migrator.run(&migrate_pool).await.expect("apply 0001 alone");
    migrate_pool.close().await;
    std::fs::remove_dir_all(&dir).unwrap();

    let admin = db.admin_pool();
    let rows_before: i64 = sqlx::query_scalar("select count(*) from _sqlx_migrations")
        .fetch_one(&admin)
        .await
        .unwrap();
    assert_eq!(rows_before, 1, "the starting point must be version 1 only");
    let row_before: (String, String) = sqlx::query_as(
        "select c.applied_at::text, m.installed_on::text
           from schema_contract c, _sqlx_migrations m
          where c.version = 1 and m.version = 1",
    )
    .fetch_one(&admin)
    .await
    .unwrap();

    let out = common::run_fau_migrate(&db).await;
    assert!(out.status.success(), "{:?}", out);

    let version: i32 = sqlx::query_scalar("select max(version) from schema_contract")
        .fetch_one(&admin)
        .await
        .unwrap();
    assert_eq!(version, common::migration_file_count());
    let rows: i64 = sqlx::query_scalar("select count(*) from _sqlx_migrations")
        .fetch_one(&admin)
        .await
        .unwrap();
    assert_eq!(rows, i64::from(common::migration_file_count()));
    let row_after: (String, String) = sqlx::query_as(
        "select c.applied_at::text, m.installed_on::text
           from schema_contract c, _sqlx_migrations m
          where c.version = 1 and m.version = 1",
    )
    .fetch_one(&admin)
    .await
    .unwrap();
    assert_eq!(row_after, row_before, "version 1 must not be re-applied");
}

#[tokio::test]
async fn concurrent_migrate_processes_serialise() {
    let db = TestDb::fresh().await;
    common::apply_roles(&db).await;

    let (a, b) = tokio::join!(common::run_fau_migrate(&db), common::run_fau_migrate(&db));
    assert!(
        a.status.success() && b.status.success(),
        "both must succeed: one applies, the other finds nothing to do"
    );

    // Exactly one row per migration proves neither applied the same file twice.
    let rows: i64 = sqlx::query_scalar("select count(*) from _sqlx_migrations")
        .fetch_one(&db.admin_pool())
        .await
        .unwrap();
    assert_eq!(rows, i64::from(common::migration_file_count()));
}

#[tokio::test]
async fn migrate_fails_rather_than_hanging_when_the_lock_is_held() {
    let db = TestDb::fresh().await;
    common::apply_roles(&db).await;

    // Hold the outer lock from an independent session.
    let holder = db.admin_pool();
    sqlx::query("select pg_advisory_lock($1)")
        .bind(fau_persistence::MIGRATION_LOCK_ID)
        .execute(&holder)
        .await
        .unwrap();

    let started = std::time::Instant::now();
    let out = common::run_fau_migrate_with(&db, &[("MIGRATION_LOCK_WAIT_MS", "500")]).await;
    let elapsed = started.elapsed();

    assert!(!out.status.success(), "must fail, not hang");
    // Generous relative to the 500ms wait, but tight enough to prove this failed
    // fast rather than falling back to some other, much longer timeout.
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "took {elapsed:?}, expected well under 5s for a 500ms lock wait"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("migration lock unavailable"),
        "expected the lock-unavailable message, got: {stderr}"
    );
}

#[tokio::test]
async fn serve_performs_no_ddl() {
    // Spec section 3. The runtime role has no DDL rights (db/roles.sql), so this is
    // belt and braces: an empty database must stay empty when serve is started.
    //
    // The schema-contract gate makes serve refuse an unmigrated database
    // outright -- the missing `schema_contract` table (SQLSTATE 42P01) is treated as
    // contract version 0, below the minimum -- rather than binding and answering
    // /health/live, so this proves "no DDL" via the process's exit instead of by
    // sending it SIGTERM.
    let db = TestDb::fresh().await;
    common::apply_roles(&db).await;

    let out = common::run_fau_serve_until_exit(&db).await;
    assert!(
        !out.status.success(),
        "serve must refuse an unmigrated database"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    // The refusal text itself, not just the shared "schema contract" substring the
    // unreachable-database warning also carries: the missing table is folded into
    // contract version 0.
    assert!(
        stderr.contains("version 0"),
        "expected the schema-contract refusal message, got: {stderr}"
    );
    let dsn = db.url();
    assert!(
        !stderr.contains(&dsn),
        "DSN (with password) leaked: {stderr}"
    );

    let tables: i64 = sqlx::query_scalar(
        "select count(*) from information_schema.tables where table_schema = 'public'",
    )
    .fetch_one(&db.admin_pool())
    .await
    .unwrap();
    assert_eq!(tables, 0, "serve created schema objects");
}
