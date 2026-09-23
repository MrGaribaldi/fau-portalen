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
    // 0001 and 0002 both exist as of Task 6.
    assert_eq!(version, 2, "contract version after all migrations");
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
    // 0001 and 0002 both exist as of Task 6.
    let rows: i64 = sqlx::query_scalar("select count(*) from _sqlx_migrations")
        .fetch_one(&db.admin_pool())
        .await
        .unwrap();
    assert_eq!(rows, 2);
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
    // Spec section 3. The runtime role has no DDL rights (Task 5), so this is
    // belt and braces: an empty database must stay empty when serve is started.
    let db = TestDb::fresh().await;
    common::apply_roles(&db).await;

    let app = common::spawn_serve(&db).await; // waits for /health/live
    app.send_sigterm().await;
    app.wait().await;

    let tables: i64 = sqlx::query_scalar(
        "select count(*) from information_schema.tables where table_schema = 'public'",
    )
    .fetch_one(&db.admin_pool())
    .await
    .unwrap();
    assert_eq!(tables, 0, "serve created schema objects");
}
