//! `/health/ready`, from the design's section 9 and ADR-001's evidence table: local
//! initialisation, a bounded database check, and the schema contract, cached briefly
//! on success and never on failure.

mod common;
use common::TestDb;

/// Reads `/health/ready`'s `reason` field, which is present only on a `not_ready`
/// body -- panics if the response was `ready` or was not valid JSON, since every
/// caller here already knows which case it expects.
async fn ready_reason(response: reqwest::Response) -> String {
    let body: serde_json::Value = response.json().await.expect("a JSON readiness body");
    body["reason"]
        .as_str()
        .unwrap_or_else(|| panic!("no string reason in readiness body: {body}"))
        .to_owned()
}

#[tokio::test]
async fn database_down_then_back_does_not_restart_the_process() {
    // Spec section 15, the third ADR-001 evidence row.
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    assert_eq!(app.get("/health/ready").await.status(), 200);

    db.sever_connections().await;
    let ready = common::poll_until(
        || app.get_async("/health/ready"),
        |r| r.status() == 503,
        std::time::Duration::from_secs(10),
    )
    .await;
    assert!(ready, "readiness never went false");
    let response = app.get("/health/ready").await;
    assert_eq!(response.status(), 503);
    assert_eq!(
        ready_reason(response).await,
        "database",
        "an outage must be reported as a database problem, not a schema-contract one"
    );
    assert_eq!(
        app.get("/health/live").await.status(),
        200,
        "liveness must not follow"
    );
    assert!(app.is_running().await, "the process restarted or exited");

    db.restore_connections().await;
    let back = common::poll_until(
        || app.get_async("/health/ready"),
        |r| r.status() == 200,
        std::time::Duration::from_secs(15),
    )
    .await;
    assert!(back, "readiness never recovered");
    assert!(app.is_running().await);
}

#[tokio::test]
async fn readiness_is_503_when_the_contract_is_below_the_minimum() {
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    sqlx::query("delete from schema_contract where version = 2")
        .execute(&db.admin_pool())
        .await
        .unwrap();
    let dropped = common::poll_until(
        || app.get_async("/health/ready"),
        |r| r.status() == 503,
        std::time::Duration::from_secs(5),
    )
    .await;
    assert!(dropped);
    let response = app.get("/health/ready").await;
    assert_eq!(response.status(), 503);
    assert_eq!(
        ready_reason(response).await,
        "schema_contract",
        "a below-minimum contract must be reported as its own reason, not a generic database one"
    );
}

/// A migration (or anything else) holding an exclusive lock on `schema_contract`
/// must not turn a `/health/ready` scrape into a hang: `db_check`'s `select 1` never
/// touches that table, so only `read_contract_version` would ever wait on this lock,
/// and `readiness::check`'s outer one-second timeout wraps both queries together.
#[tokio::test]
async fn readiness_does_not_hang_when_schema_contract_is_locked() {
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;

    let admin_pool = db.admin_pool();
    let mut lock_conn = admin_pool
        .acquire()
        .await
        .expect("acquire a dedicated admin connection");
    sqlx::query("begin")
        .execute(&mut *lock_conn)
        .await
        .expect("begin a transaction to hold the lock in");
    sqlx::query("lock table schema_contract in access exclusive mode")
        .execute(&mut *lock_conn)
        .await
        .expect("acquire the exclusive lock");

    let started = std::time::Instant::now();
    let response = app.get("/health/ready").await;
    let elapsed = started.elapsed();

    // Release the lock before any assertion below can panic and skip this.
    let _ = sqlx::query("rollback").execute(&mut *lock_conn).await;

    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "readiness took {elapsed:?} while schema_contract was exclusively locked -- \
         it must stay bounded to about one second, not hang on the lock"
    );
    assert_eq!(response.status(), 503);
    assert_eq!(ready_reason(response).await, "database");
}

#[tokio::test]
async fn readiness_body_names_no_internal_address() {
    // ADR-001: minimal responses without internal addresses -- checked on both the
    // ready body and the not-ready one, since the two are built differently
    // (`ReadyBody::Ready` vs `ReadyBody::NotReady`) and either could regress
    // independently.
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    let leaks = ["postgres://", "127.0.0.1", "5432", "password"];

    let ready_body = app.get("/health/ready").await.text().await.unwrap();
    for leak in leaks {
        assert!(
            !ready_body.contains(leak),
            "ready body leaked {leak}: {ready_body}"
        );
    }

    db.sever_connections().await;
    common::poll_until(
        || app.get_async("/health/ready"),
        |r| r.status() == 503,
        std::time::Duration::from_secs(10),
    )
    .await;
    let not_ready_body = app.get("/health/ready").await.text().await.unwrap();
    for leak in leaks {
        assert!(
            !not_ready_body.contains(leak),
            "not-ready body leaked {leak}: {not_ready_body}"
        );
    }
}

#[tokio::test]
async fn a_failing_probe_is_not_cached() {
    // This integration test can only honestly show that recovery becomes visible
    // again once the outage ends -- polling on a wall-clock window cannot itself
    // distinguish "never cached" from "cached for less than the window observed".
    // The exact property (a failure is never cached; a success is cached for
    // exactly `CACHE_TTL`) is proven deterministically, with no timing dependency,
    // by `fau_app`'s own unit tests on `ReadinessState::probe_with`
    // (`a_failure_is_never_cached_so_the_next_call_rechecks` and
    // `a_success_is_cached_for_the_ttl`).
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    db.sever_connections().await;
    let dropped = common::poll_until(
        || app.get_async("/health/ready"),
        |r| r.status() == 503,
        std::time::Duration::from_secs(10),
    )
    .await;
    assert!(dropped, "readiness never went false after the outage");

    db.restore_connections().await;
    // Recovery must be visible well inside the cache window of a *successful* probe.
    let back = common::poll_until(
        || app.get_async("/health/ready"),
        |r| r.status() == 200,
        std::time::Duration::from_secs(5),
    )
    .await;
    assert!(back);
}
