//! Proves the ephemeral-database harness itself, from the design's section 15:
//! each test gets its own database, and nothing is left behind when it drops.

mod common;
use common::TestDb;

#[tokio::test]
async fn fresh_database_is_empty_and_isolated() {
    let a = TestDb::fresh().await;
    let b = TestDb::fresh().await;
    assert_ne!(a.name, b.name);

    sqlx::query("create table probe (id int primary key)")
        .execute(&a.admin_pool())
        .await
        .unwrap();

    let in_b: Option<String> = sqlx::query_scalar("select to_regclass('probe')::text")
        .fetch_one(&b.admin_pool())
        .await
        .unwrap();
    assert!(in_b.is_none(), "databases are not isolated");
}

#[tokio::test]
async fn dropped_database_is_gone() {
    let name = {
        let db = TestDb::fresh().await;
        db.name.clone()
    };
    // Drop ran at end of scope.
    let pool = common::admin_pool().await;
    let exists: bool =
        sqlx::query_scalar("select exists(select 1 from pg_database where datname = $1)")
            .bind(&name)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!exists, "database {name} was left behind");
}
