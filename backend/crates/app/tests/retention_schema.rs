//! Migration 0009: a former member's fields are kept for their chosen period (#3511,
//! docs/member-retention-design.md §4).

mod common;
use common::TestDb;
use sqlx::PgPool;
use uuid::Uuid;

fn constraint_name(err: &sqlx::Error) -> Option<String> {
    err.as_database_error()
        .and_then(|e| e.constraint())
        .map(|c| c.to_owned())
}

/// Shaped like fau-crypto's envelope: version byte `version`, then `len - 1` bytes.
fn envelope(version: u8, len: usize) -> Vec<u8> {
    let mut v = vec![0u8; len];
    v[0] = version;
    v
}

/// One active FAU with one membership, straight in SQL.
async fn seed(pool: &PgPool) -> (Uuid, Uuid) {
    let school = common::membership::school(pool, &format!("dir-{}", Uuid::now_v7())).await;
    let (tenant, account, membership) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    sqlx::query(
        "insert into tenants (id, name, status, school_id) values ($1, 'FAU', 'active', $2)",
    )
    .bind(tenant)
    .bind(school)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("insert into accounts (id, email) values ($1, $1::text || '@example.test')")
        .bind(account)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("insert into memberships (tenant_id, id, account_id) values ($1, $2, $3)")
        .bind(tenant)
        .bind(membership)
        .bind(account)
        .execute(pool)
        .await
        .unwrap();
    (tenant, membership)
}

async fn set(
    pool: &PgPool,
    id: Uuid,
    column: &str,
    value: Option<Vec<u8>>,
) -> Result<(), sqlx::Error> {
    sqlx::query(&format!(
        "update memberships set {column} = $2 where id = $1"
    ))
    .bind(id)
    .bind(value)
    .execute(pool)
    .await
    .map(|_| ())
}

async fn exec(pool: &PgPool, sql: &str, m: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(sql).bind(m).execute(pool).await.map(|_| ())
}

/// Mutation check: keep 0008's `*_only_while_current` checks and the retained revocation
/// is refused. Only `encrypted_display_name` is set here: PostgreSQL checks a row's CHECK
/// constraints in constraint-name order, so with both fields set the two checks race and
/// only one name is deterministic to assert on — each field gets its own test instead
/// (`encrypted_contact_email`'s is `a_contact_address_alone_is_held_by_its_own_check`).
#[tokio::test]
async fn a_revoked_row_keeps_its_fields_only_while_a_retention_date_is_set() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (_, m) = seed(&pool).await;
    set(&pool, m, "encrypted_display_name", Some(envelope(1, 60)))
        .await
        .unwrap();

    let err = exec(
        &pool,
        "update memberships set revoked_at = now() where id = $1",
        m,
    )
    .await
    .unwrap_err();
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("memberships_display_name_only_while_current_or_retained")
    );
    exec(
        &pool,
        "update memberships set revoked_at = now(), profile_retained_until = '2027-01-01'
          where id = $1",
        m,
    )
    .await
    .unwrap();
    // Dropping the date without the fields is refused; with them it is accepted.
    assert!(exec(
        &pool,
        "update memberships set profile_retained_until = null where id = $1",
        m
    )
    .await
    .is_err());
    exec(
        &pool,
        "update memberships set profile_retained_until = null,
                encrypted_display_name = null, encrypted_contact_email = null where id = $1",
        m,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_contact_address_alone_is_held_by_its_own_check() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (_, m) = seed(&pool).await;
    set(&pool, m, "encrypted_contact_email", Some(envelope(1, 60)))
        .await
        .unwrap();
    let err = exec(
        &pool,
        "update memberships set revoked_at = now() where id = $1",
        m,
    )
    .await
    .unwrap_err();
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("memberships_contact_email_only_while_current_or_retained")
    );
}

#[tokio::test]
async fn a_retention_date_needs_something_to_retain_and_an_erasure_clears_it() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (_, m) = seed(&pool).await;
    let err = exec(
        &pool,
        "update memberships set profile_retained_until = '2027-01-01' where id = $1",
        m,
    )
    .await
    .unwrap_err();
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("memberships_retention_needs_a_field")
    );

    set(&pool, m, "encrypted_display_name", Some(envelope(1, 60)))
        .await
        .unwrap();
    exec(
        &pool,
        "update memberships set profile_retained_until = '2027-01-01' where id = $1",
        m,
    )
    .await
    .unwrap();
    let err = exec(
        &pool,
        "update memberships set name_erased_at = now(),
                encrypted_display_name = null, encrypted_contact_email = null where id = $1",
        m,
    )
    .await
    .unwrap_err();
    // The field check fires first once the fields are gone; either way it is refused.
    assert!(matches!(
        constraint_name(&err).as_deref(),
        Some("memberships_erasure_clears_retention" | "memberships_retention_needs_a_field")
    ));
}

#[tokio::test]
async fn retention_months_takes_only_the_five_allowed_values() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let acc = Uuid::now_v7();
    sqlx::query("insert into accounts (id, email) values ($1, 'r@example.test')")
        .bind(acc)
        .execute(&pool)
        .await
        .unwrap();
    for ok in [0, 3, 6, 12, 24] {
        sqlx::query("update accounts set retention_months = $2 where id = $1")
            .bind(acc)
            .bind(ok)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("{ok}: {e}"));
    }
    for bad in [-1, 1, 5, 18, 25, 36] {
        let err = sqlx::query("update accounts set retention_months = $2 where id = $1")
            .bind(acc)
            .bind(bad)
            .execute(&pool)
            .await
            .unwrap_err();
        assert_eq!(
            constraint_name(&err).as_deref(),
            Some("accounts_retention_months_is_allowed"),
            "{bad}"
        );
    }
}

#[tokio::test]
async fn the_contract_is_version_nine() {
    let db = TestDb::migrated().await;
    let v: i32 = sqlx::query_scalar("select max(version) from schema_contract")
        .fetch_one(&db.admin_pool())
        .await
        .unwrap();
    assert_eq!(v, 9);
}
