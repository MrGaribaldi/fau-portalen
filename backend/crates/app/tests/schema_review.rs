//! The database privilege split, from the design's section 6.

mod common;
use common::TestDb;

#[tokio::test]
async fn runtime_role_cannot_perform_ddl() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await; // connects as fau_app

    let err = sqlx::query("create table sneaky (id int)")
        .execute(&pool)
        .await;
    assert!(err.is_err(), "runtime role must not have DDL rights");
}

#[tokio::test]
async fn runtime_role_can_read_and_write_the_spine() {
    // Expected to fail until Task 6 creates `accounts` -- that is this task's exit
    // condition for Task 6, not a defect here.
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let n: i64 = sqlx::query_scalar("select count(*) from accounts")
        .fetch_one(&pool)
        .await
        .expect("runtime role must read accounts");
    assert_eq!(n, 0);
}

#[tokio::test]
async fn runtime_role_cannot_create_temp_tables() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;

    let err = sqlx::query("create temporary table sneaky_temp (id int)")
        .execute(&pool)
        .await;
    assert!(
        err.is_err(),
        "runtime role must not have TEMP rights on the database"
    );
}

#[tokio::test]
async fn runtime_role_is_not_a_superuser_and_cannot_bypass_rls() {
    let db = TestDb::migrated().await;
    let row: (bool, bool) =
        sqlx::query_as("select rolsuper, rolbypassrls from pg_roles where rolname = 'fau_app'")
            .fetch_one(&db.admin_pool())
            .await
            .unwrap();
    assert_eq!(
        row,
        (false, false),
        "row-level security lands in #3418 and must not already be bypassed"
    );
}
