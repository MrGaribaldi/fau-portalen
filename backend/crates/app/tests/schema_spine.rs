//! Migration 0002: the identity and tenancy spine, from the design's section 5.

mod common;
use common::TestDb;
use uuid::Uuid;

async fn seed_two_tenants(db: &TestDb) -> (Uuid, Uuid, Uuid) {
    let pool = db.admin_pool();
    let (t1, t2, acc) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    sqlx::query("insert into tenants (id, name, status) values ($1,'FAU A','active'), ($2,'FAU B','active')")
        .bind(t1).bind(t2).execute(&pool).await.unwrap();
    sqlx::query("insert into accounts (id, email) values ($1, 'a@example.test')")
        .bind(acc)
        .execute(&pool)
        .await
        .unwrap();
    (t1, t2, acc)
}

/// The Postgres SQLSTATE code, if the error is a database error at all. `is_err()`
/// alone cannot distinguish the constraint under test from any other failure
/// (e.g. `tenant_status_is_constrained` would "pass" against a table that did not
/// exist, for the wrong reason entirely) -- the code pins down which rule actually
/// fired.
fn sqlstate(err: &sqlx::Error) -> Option<String> {
    err.as_database_error()
        .and_then(|e| e.code())
        .map(|c| c.into_owned())
}

/// The violated constraint's name, where Postgres reports one (it does for a
/// foreign key, a unique index or a named check; not for every error kind).
fn constraint_name(err: &sqlx::Error) -> Option<String> {
    err.as_database_error()
        .and_then(|e| e.constraint())
        .map(|c| c.to_owned())
}

#[tokio::test]
async fn cross_tenant_reference_is_rejected_by_the_database() {
    // Spec section 5, property 1 -- #3412's central integrity rule.
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t1, t2, acc) = seed_two_tenants(&db).await;

    let m1 = Uuid::now_v7();
    sqlx::query("insert into memberships (tenant_id, id, account_id) values ($1,$2,$3)")
        .bind(t1)
        .bind(m1)
        .bind(acc)
        .execute(&pool)
        .await
        .unwrap();

    let r1 = Uuid::now_v7();
    sqlx::query(
        "insert into roles (tenant_id, id, name, capability_class) values ($1,$2,'Medlem','member')",
    )
    .bind(t1)
    .bind(r1)
    .execute(&pool)
    .await
    .unwrap();

    let r2 = Uuid::now_v7();
    sqlx::query(
        "insert into roles (tenant_id, id, name, capability_class) values ($1,$2,'Leder','admin')",
    )
    .bind(t2)
    .bind(r2)
    .execute(&pool)
    .await
    .unwrap();

    // Positive control: tenant A's own role assigned to tenant A's membership must
    // succeed. Without this, a schema that rejected every insert (or every
    // role_assignments insert regardless of tenant) would still pass the negative
    // assertion below for the wrong reason.
    let same_tenant = sqlx::query(
        "insert into role_assignments
           (tenant_id, id, membership_id, role_id, starts_on, ends_on_exclusive)
         values ($1, $2, $3, $4, date '2026-08-01', date '2027-08-01')",
    )
    .bind(t1)
    .bind(Uuid::now_v7())
    .bind(m1)
    .bind(r1)
    .execute(&pool)
    .await;
    assert!(
        same_tenant.is_ok(),
        "a same-tenant role assignment must succeed: {same_tenant:?}"
    );

    // Tenant B's role assigned to tenant A's membership, written directly in SQL.
    let err = sqlx::query(
        "insert into role_assignments
           (tenant_id, id, membership_id, role_id, starts_on, ends_on_exclusive)
         values ($1, $2, $3, $4, date '2026-08-01', date '2027-08-01')",
    )
    .bind(t1)
    .bind(Uuid::now_v7())
    .bind(m1)
    .bind(r2)
    .execute(&pool)
    .await
    .expect_err("composite foreign key did not reject a cross-tenant reference");

    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("23503"),
        "expected a foreign-key violation, got {err:?}"
    );
}

#[tokio::test]
async fn granted_by_is_also_rejected_across_tenants() {
    // The third composite foreign key on role_assignments (granted_by ->
    // memberships) is easy to miss if only membership_id and role_id are tested --
    // cheap to cover directly.
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t1, t2, acc) = seed_two_tenants(&db).await;

    let m1 = Uuid::now_v7();
    sqlx::query("insert into memberships (tenant_id, id, account_id) values ($1,$2,$3)")
        .bind(t1)
        .bind(m1)
        .bind(acc)
        .execute(&pool)
        .await
        .unwrap();
    let m2 = Uuid::now_v7();
    sqlx::query("insert into memberships (tenant_id, id, account_id) values ($1,$2,$3)")
        .bind(t2)
        .bind(m2)
        .bind(acc)
        .execute(&pool)
        .await
        .unwrap();
    let r1 = Uuid::now_v7();
    sqlx::query(
        "insert into roles (tenant_id, id, name, capability_class) values ($1,$2,'Medlem','member')",
    )
    .bind(t1)
    .bind(r1)
    .execute(&pool)
    .await
    .unwrap();

    // Tenant B's membership recorded as the granter of a tenant A role assignment.
    let err = sqlx::query(
        "insert into role_assignments
           (tenant_id, id, membership_id, role_id, granted_by, starts_on, ends_on_exclusive)
         values ($1, $2, $3, $4, $5, date '2026-08-01', date '2027-08-01')",
    )
    .bind(t1)
    .bind(Uuid::now_v7())
    .bind(m1)
    .bind(r1)
    .bind(m2)
    .execute(&pool)
    .await
    .expect_err("composite foreign key did not reject a cross-tenant granted_by");

    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("23503"),
        "expected a foreign-key violation, got {err:?}"
    );
}

#[tokio::test]
async fn role_period_must_be_a_non_empty_half_open_interval() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t1, _t2, acc) = seed_two_tenants(&db).await;
    let (m, r) = (Uuid::now_v7(), Uuid::now_v7());
    sqlx::query("insert into memberships (tenant_id, id, account_id) values ($1,$2,$3)")
        .bind(t1)
        .bind(m)
        .bind(acc)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("insert into roles (tenant_id, id, name, capability_class) values ($1,$2,'Medlem','member')")
        .bind(t1).bind(r).execute(&pool).await.unwrap();

    // Positive control: a genuinely non-empty period must succeed, so the negative
    // cases below are known to fail on the period itself, not on some other cause.
    let valid = sqlx::query(
        "insert into role_assignments
           (tenant_id, id, membership_id, role_id, starts_on, ends_on_exclusive)
         values ($1,$2,$3,$4,$5::date,$6::date)",
    )
    .bind(t1)
    .bind(Uuid::now_v7())
    .bind(m)
    .bind(r)
    .bind("2026-08-01")
    .bind("2027-08-01")
    .execute(&pool)
    .await;
    assert!(valid.is_ok(), "a valid period must succeed: {valid:?}");

    for (starts, ends) in [("2027-08-01", "2026-08-01"), ("2026-08-01", "2026-08-01")] {
        let err = sqlx::query(
            "insert into role_assignments
               (tenant_id, id, membership_id, role_id, starts_on, ends_on_exclusive)
             values ($1,$2,$3,$4,$5::date,$6::date)",
        )
        .bind(t1)
        .bind(Uuid::now_v7())
        .bind(m)
        .bind(r)
        .bind(starts)
        .bind(ends)
        .execute(&pool)
        .await
        .expect_err(&format!("accepted {starts}..{ends}"));

        assert_eq!(
            sqlstate(&err).as_deref(),
            Some("23514"),
            "expected a check violation for {starts}..{ends}, got {err:?}"
        );
        assert_eq!(
            constraint_name(&err).as_deref(),
            Some("role_period_is_non_empty"),
            "expected the role_period_is_non_empty check for {starts}..{ends}, got {err:?}"
        );
    }
}

#[tokio::test]
async fn ends_on_exclusive_is_mandatory() {
    let db = TestDb::migrated().await;
    let null_allowed: bool = sqlx::query_scalar(
        "select is_nullable = 'YES' from information_schema.columns
         where table_name = 'role_assignments' and column_name = 'ends_on_exclusive'",
    )
    .fetch_one(&db.admin_pool())
    .await
    .unwrap();
    assert!(
        !null_allowed,
        "an open-ended role period must be impossible"
    );
}

#[tokio::test]
async fn an_account_may_hold_several_identity_mappings() {
    // ADR-003 decision 3: the rule most expensive to add later.
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let acc = Uuid::now_v7();
    sqlx::query("insert into accounts (id, email) values ($1,'m@example.test')")
        .bind(acc)
        .execute(&pool)
        .await
        .unwrap();

    for issuer in ["https://a.hanko.example", "https://b.other.example"] {
        sqlx::query(
            "insert into identity_mappings (issuer, subject, account_id) values ($1,'sub-1',$2)",
        )
        .bind(issuer)
        .bind(acc)
        .execute(&pool)
        .await
        .expect("two issuers must map to one account");
    }
}

#[tokio::test]
async fn one_membership_per_account_per_tenant() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t1, _t2, acc) = seed_two_tenants(&db).await;
    sqlx::query("insert into memberships (tenant_id, id, account_id) values ($1,$2,$3)")
        .bind(t1)
        .bind(Uuid::now_v7())
        .bind(acc)
        .execute(&pool)
        .await
        .unwrap();
    let dup = sqlx::query("insert into memberships (tenant_id, id, account_id) values ($1,$2,$3)")
        .bind(t1)
        .bind(Uuid::now_v7())
        .bind(acc)
        .execute(&pool)
        .await
        .expect_err("duplicate membership accepted");

    assert_eq!(
        sqlstate(&dup).as_deref(),
        Some("23505"),
        "expected a unique-constraint violation, got {dup:?}"
    );
}

#[tokio::test]
async fn locale_columns_exist_with_the_decided_defaults() {
    // #3439. Nothing may hard-code two locales; these are free-text BCP 47 tags.
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();

    let account_locale_nullable: bool = sqlx::query_scalar(
        "select is_nullable = 'YES' from information_schema.columns
         where table_name = 'accounts' and column_name = 'locale'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(account_locale_nullable);

    let t = Uuid::now_v7();
    sqlx::query("insert into tenants (id, name, status) values ($1,'FAU','pending')")
        .bind(t)
        .execute(&pool)
        .await
        .unwrap();
    let default_locale: String =
        sqlx::query_scalar("select default_locale from tenants where id = $1")
            .bind(t)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(default_locale, "nb-NO");
}

#[tokio::test]
async fn tenant_status_is_constrained() {
    let db = TestDb::migrated().await;
    let bad = sqlx::query("insert into tenants (id, name, status) values ($1,'X','deleted')")
        .bind(Uuid::now_v7())
        .execute(&db.admin_pool())
        .await
        .expect_err("status must be pending/active/closed");

    assert_eq!(
        sqlstate(&bad).as_deref(),
        Some("23514"),
        "expected a check violation, got {bad:?}"
    );
    assert_eq!(
        constraint_name(&bad).as_deref(),
        Some("tenants_status_check"),
        "expected the tenants status check, got {bad:?}"
    );
}

#[tokio::test]
async fn retention_months_defaults_to_three() {
    // ADR-003 decision 6a: member-elected retention is designed for, not built.
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let acc = Uuid::now_v7();
    sqlx::query("insert into accounts (id, email) values ($1,'r@example.test')")
        .bind(acc)
        .execute(&pool)
        .await
        .unwrap();
    let months: i32 = sqlx::query_scalar("select retention_months from accounts where id = $1")
        .bind(acc)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(months, 3);
}
