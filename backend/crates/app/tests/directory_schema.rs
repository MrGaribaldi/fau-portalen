//! Migration 0008: the member directory's fields on `memberships` (groups design §4.1,
//! §4.2), proven in SQL before any Rust depends on them. Under Erik's D3 (28 September
//! 2026) neither field outlives the membership: a revoked membership holds no name and no
//! contact address.

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

#[tokio::test]
async fn both_fields_hold_only_a_bounded_envelope() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (_, m) = seed(&pool).await;
    for (column, constraint) in [
        (
            "encrypted_display_name",
            "memberships_display_name_is_an_envelope",
        ),
        (
            "encrypted_contact_email",
            "memberships_contact_email_is_an_envelope",
        ),
    ] {
        // Positive controls at both bounds, and null.
        set(&pool, m, column, Some(envelope(1, 42))).await.unwrap();
        set(&pool, m, column, Some(envelope(1, 512))).await.unwrap();
        set(&pool, m, column, None).await.unwrap();
        for bad in [
            envelope(1, 41),
            envelope(1, 513),
            envelope(0, 100),
            envelope(2, 100),
        ] {
            let err = set(&pool, m, column, Some(bad)).await.unwrap_err();
            assert_eq!(
                constraint_name(&err).as_deref(),
                Some(constraint),
                "{column}"
            );
        }
    }
}

#[tokio::test]
async fn a_revoked_membership_holds_neither_a_name_nor_a_contact_email() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (_, m) = seed(&pool).await;
    let checks = [
        (
            "encrypted_display_name",
            "memberships_display_name_only_while_current",
        ),
        (
            "encrypted_contact_email",
            "memberships_contact_email_only_while_current",
        ),
    ];
    let revoke = |clear: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query(&format!(
                "update memberships set revoked_at = '2026-09-23T10:00:00Z'{clear} where id = $1"
            ))
            .bind(m)
            .execute(&pool)
            .await
            .map(|_| ())
        }
    };

    // Each field on its own blocks the revocation, with its own check.
    for (column, constraint) in checks {
        set(&pool, m, column, Some(envelope(1, 60))).await.unwrap();
        let err = revoke("").await.unwrap_err();
        assert_eq!(
            constraint_name(&err).as_deref(),
            Some(constraint),
            "{column}"
        );
        set(&pool, m, column, None).await.unwrap();
    }

    // Clearing both in the same statement is accepted: under D3 the name goes too.
    for (column, _) in checks {
        set(&pool, m, column, Some(envelope(1, 60))).await.unwrap();
    }
    revoke(", encrypted_display_name = null, encrypted_contact_email = null")
        .await
        .unwrap();
    for (column, constraint) in checks {
        let err = set(&pool, m, column, Some(envelope(1, 60)))
            .await
            .unwrap_err();
        assert_eq!(
            constraint_name(&err).as_deref(),
            Some(constraint),
            "{column}"
        );
    }

    // Positive control, the re-invitation path: reopened, the row takes a new name.
    sqlx::query("update memberships set revoked_at = null where id = $1")
        .bind(m)
        .execute(&pool)
        .await
        .unwrap();
    set(&pool, m, "encrypted_display_name", Some(envelope(1, 60)))
        .await
        .unwrap();
}

#[tokio::test]
async fn an_erased_membership_holds_neither_field() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (_, m) = seed(&pool).await;
    for column in ["encrypted_display_name", "encrypted_contact_email"] {
        set(&pool, m, column, Some(envelope(1, 60))).await.unwrap();
        let err = sqlx::query(
            "update memberships set name_erased_at = '2026-09-23T10:00:00Z' where id = $1",
        )
        .bind(m)
        .execute(&pool)
        .await
        .unwrap_err();
        assert_eq!(
            constraint_name(&err).as_deref(),
            Some("memberships_erasure_leaves_nothing"),
            "{column}"
        );
        set(&pool, m, column, None).await.unwrap();
    }
    // Positive control: with both cleared, the erasure marker is accepted.
    sqlx::query("update memberships set name_erased_at = '2026-09-23T10:00:00Z' where id = $1")
        .bind(m)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn the_runtime_role_can_write_both_fields() {
    let db = TestDb::migrated().await;
    let (_, m) = seed(&db.admin_pool()).await;
    let app = db.app_pool().await;
    set(&app, m, "encrypted_display_name", Some(envelope(1, 60)))
        .await
        .unwrap();
    set(&app, m, "encrypted_contact_email", Some(envelope(1, 60)))
        .await
        .unwrap();
}
