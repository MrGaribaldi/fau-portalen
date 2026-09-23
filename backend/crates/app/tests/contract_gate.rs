//! The schema contract gate, from the design's section 5 and section 4's rule that a
//! merely unreachable database is not a startup failure.

mod common;
use common::TestDb;

#[tokio::test]
async fn refuses_to_serve_below_the_minimum() {
    let db = TestDb::migrated().await;
    sqlx::query("delete from schema_contract where version = 2")
        .execute(&db.admin_pool())
        .await
        .unwrap();

    let out = common::run_fau_serve_until_exit(&db).await;
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    // The refusal text itself, not just the shared "schema contract" substring the
    // unreachable-database warning also carries: version 1 (0002's row deleted,
    // 0001's remains) is below this binary's minimum of 2.
    assert!(stderr.contains("version 1"), "message was: {stderr}");
    assert!(stderr.contains("at least 2"), "message was: {stderr}");
    let dsn = db.url();
    assert!(
        !stderr.contains(&dsn),
        "DSN (with password) leaked: {stderr}"
    );

    // The refusal is also a JSON ERROR event on stdout, so log collection sees it,
    // in addition to (not instead of) the plain stderr line above.
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .any(|v| v["level"] == "ERROR"),
        "no JSON ERROR refusal event found on stdout: {stdout}"
    );
}

#[tokio::test]
async fn refuses_to_serve_when_the_runtime_role_cannot_read_the_contract() {
    // A privilege problem (SQLSTATE 42501) is not something a restart loop fixes,
    // so it takes the refusal path -- distinct from a merely unreachable database,
    // which warns and keeps running.
    let db = TestDb::migrated().await;
    sqlx::query("revoke select on schema_contract from fau_app")
        .execute(&db.admin_pool())
        .await
        .unwrap();

    let out = common::run_fau_serve_until_exit(&db).await;
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("42501") || stderr.contains("permission"),
        "expected the message to name the privilege problem, got: {stderr}"
    );
    assert!(
        !stderr.contains("unreachable"),
        "a privilege problem must not be reported as an unreachable database: {stderr}"
    );
}

#[tokio::test]
async fn serves_above_the_minimum() {
    let db = TestDb::migrated().await;
    sqlx::query("insert into schema_contract (version) values (99)")
        .execute(&db.admin_pool())
        .await
        .unwrap();

    let app = common::spawn_serve(&db).await;
    let status = app.get("/health/live").await.status();
    assert_eq!(
        status, 200,
        "a database ahead of the binary must still be served"
    );
    // Readiness must tolerate a higher version too, or a rollback to an older
    // binary would never take traffic.
    let ready = app.get("/health/ready").await.status();
    assert_eq!(
        ready, 200,
        "readiness must accept a database ahead of the binary"
    );
}

#[tokio::test]
async fn stays_running_when_the_database_is_unreachable_at_startup() {
    // Design section 4: a merely unreachable database is not a contract
    // failure. Port 1 refuses the underlying TCP connection immediately, but
    // sqlx's pool retries a failed `Io` connection attempt internally before
    // giving up on it, so this check actually runs close to its full 5s bound here
    // rather than returning instantly -- `spawn_serve`'s own 10s wait for
    // /health/live comfortably covers that.
    let db = TestDb::migrated().await;
    let app = common::spawn_serve_with_env(
        &db,
        &[("DATABASE_URL", "postgres://fau_app:fau_app@127.0.0.1:1/fau")],
    )
    .await;

    assert_eq!(app.get("/health/live").await.status(), 200);
    assert!(
        app.is_running().await,
        "serve must stay up when the database is unreachable at startup"
    );
}
