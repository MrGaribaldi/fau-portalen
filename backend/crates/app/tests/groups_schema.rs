//! Migration 0007: groups, their members and the guest class, proven in SQL before any Rust
//! depends on them (groups design §3.1).

mod common;
use common::TestDb;
use sqlx::PgPool;
use uuid::Uuid;

fn sqlstate(err: &sqlx::Error) -> Option<String> {
    err.as_database_error()
        .and_then(|e| e.code())
        .map(|c| c.into_owned())
}

fn constraint_name(err: &sqlx::Error) -> Option<String> {
    err.as_database_error()
        .and_then(|e| e.constraint())
        .map(|c| c.to_owned())
}

/// Shaped like fau-crypto's envelope: version byte 1, then `len - 1` bytes.
fn envelope(len: usize) -> Vec<u8> {
    let mut v = vec![0u8; len];
    v[0] = 1;
    v
}

struct Seed {
    tenant: Uuid,
    membership: Uuid,
    unit: Uuid,
    cohort: Uuid,
}

async fn seed(pool: &PgPool) -> Seed {
    let school = common::membership::school(pool, &format!("groups-{}", Uuid::now_v7())).await;
    let (tenant, account, membership) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    let (year, unit, cohort) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    for (sql, binds) in [
        (
            "insert into tenants (id, name, status, school_id) values ($1, 'FAU', 'active', $2)",
            vec![tenant, school],
        ),
        (
            "insert into accounts (id, email) values ($1, $1::text || '@example.test')",
            vec![account],
        ),
        (
            "insert into memberships (tenant_id, id, account_id) values ($1, $2, $3)",
            vec![tenant, membership, account],
        ),
        (
            "insert into school_years (tenant_id, id, name, starts_on, ends_on_exclusive)
             values ($1, $2, '2026/27', '2026-08-01', '2027-08-01')",
            vec![tenant, year],
        ),
        (
            "insert into organization_units (tenant_id, id, school_year_id, kind, name)
             values ($1, $2, $3, 'grade', '7. trinn')",
            vec![tenant, unit, year],
        ),
        (
            "insert into cohorts (tenant_id, id, name) values ($1, $2, 'K2019')",
            vec![tenant, cohort],
        ),
    ] {
        let mut q = sqlx::query(sql);
        for b in binds {
            q = q.bind(b);
        }
        q.execute(pool).await.unwrap();
    }
    Seed {
        tenant,
        membership,
        unit,
        cohort,
    }
}

async fn group(
    pool: &PgPool,
    s: &Seed,
    name: Vec<u8>,
    unit: Option<Uuid>,
    cohort: Option<Uuid>,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into groups (tenant_id, id, encrypted_name, unit_id, cohort_id, created_by, created_at)
         values ($1, $2, $3, $4, $5, $6, '2026-09-23T10:00:00Z')",
    )
    .bind(s.tenant)
    .bind(id)
    .bind(name)
    .bind(unit)
    .bind(cohort)
    .bind(s.membership)
    .execute(pool)
    .await
    .map(|_| id)
}

async fn role(
    pool: &PgPool,
    s: &Seed,
    class: &str,
    group: Option<Uuid>,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into roles (tenant_id, id, name, capability_class, group_id) values ($1, $2, 'Gjest', $3, $4)",
    )
    .bind(s.tenant)
    .bind(id)
    .bind(class)
    .bind(group)
    .execute(pool)
    .await
    .map(|_| id)
}

#[tokio::test]
async fn a_guest_role_names_a_group_and_only_a_guest_role_does() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let s = seed(&pool).await;
    let g = group(&pool, &s, envelope(42), None, None).await.unwrap();

    role(&pool, &s, "guest", Some(g))
        .await
        .expect("a guest role naming a group");
    role(&pool, &s, "member", None)
        .await
        .expect("a member role naming none");
    for (class, group) in [("guest", None), ("member", Some(g)), ("admin", Some(g))] {
        let err = role(&pool, &s, class, group).await.unwrap_err();
        assert_eq!(sqlstate(&err).as_deref(), Some("23514"), "{class}");
        assert_eq!(
            constraint_name(&err).as_deref(),
            Some("roles_guest_names_a_group")
        );
    }
    let err = role(&pool, &s, "owner", None).await.unwrap_err();
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("roles_capability_class_check")
    );
}

#[tokio::test]
async fn a_group_name_must_be_shaped_like_an_envelope() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let s = seed(&pool).await;
    group(&pool, &s, envelope(42), None, None)
        .await
        .expect("the shortest envelope");
    group(&pool, &s, envelope(512), None, None)
        .await
        .expect("the longest envelope");
    for bad in [
        b"Juleballkomiteen".to_vec(),
        envelope(41),
        envelope(513),
        vec![2u8; 60],
    ] {
        let err = group(&pool, &s, bad, None, None).await.unwrap_err();
        assert_eq!(sqlstate(&err).as_deref(), Some("23514"));
        assert_eq!(
            constraint_name(&err).as_deref(),
            Some("groups_name_is_an_envelope")
        );
    }
}

#[tokio::test]
async fn a_group_binds_to_at_most_one_unit_or_cohort_of_its_own_fau() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let s = seed(&pool).await;
    group(&pool, &s, envelope(42), Some(s.unit), None)
        .await
        .unwrap();
    group(&pool, &s, envelope(42), None, Some(s.cohort))
        .await
        .unwrap();
    let err = group(&pool, &s, envelope(42), Some(s.unit), Some(s.cohort))
        .await
        .unwrap_err();
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("groups_bound_to_at_most_one")
    );

    let other = seed(&pool).await;
    let err = group(&pool, &s, envelope(42), Some(other.unit), None)
        .await
        .unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("23503"));
}

async fn add(pool: &PgPool, s: &Seed, g: Uuid, added_at: &str) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into group_members (tenant_id, id, group_id, membership_id, added_by, added_at)
         values ($1, $2, $3, $4, $4, $5::timestamptz)",
    )
    .bind(s.tenant)
    .bind(id)
    .bind(g)
    .bind(s.membership)
    .bind(added_at)
    .execute(pool)
    .await
    .map(|_| id)
}

#[tokio::test]
async fn a_membership_is_in_a_group_at_most_once_at_a_time() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let s = seed(&pool).await;
    let g = group(&pool, &s, envelope(42), None, None).await.unwrap();
    let first = add(&pool, &s, g, "2026-09-23T10:00:00Z").await.unwrap();
    let err = add(&pool, &s, g, "2026-09-23T11:00:00Z").await.unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("23505"));
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("group_members_one_current")
    );

    // Removal is soft, and history keeps both rows.
    sqlx::query("update group_members set removed_at = '2026-09-24T10:00:00Z' where id = $1")
        .bind(first)
        .execute(&pool)
        .await
        .unwrap();
    add(&pool, &s, g, "2026-09-25T10:00:00Z")
        .await
        .expect("re-added after removal");
}

#[tokio::test]
async fn removal_cannot_precede_addition() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let s = seed(&pool).await;
    let g = group(&pool, &s, envelope(42), None, None).await.unwrap();
    let row = add(&pool, &s, g, "2026-09-23T10:00:00Z").await.unwrap();
    let err =
        sqlx::query("update group_members set removed_at = '2026-09-22T10:00:00Z' where id = $1")
            .bind(row)
            .execute(&pool)
            .await
            .unwrap_err();
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("group_member_removed_after_added")
    );
}

#[tokio::test]
async fn a_guest_role_carries_no_unit_or_cohort() {
    // Controller ruling P7: a guest reaches only its own groups. A guest role that also
    // named a unit or a cohort would put its holder into every group bound to that unit
    // or cohort through ROLE_FOLLOWS_GROUP, which is broader than that.
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let s = seed(&pool).await;
    let g = group(&pool, &s, envelope(42), None, None).await.unwrap();

    let err = sqlx::query(
        "insert into roles (tenant_id, id, name, capability_class, group_id, unit_id)
         values ($1, $2, 'Gjest', 'guest', $3, $4)",
    )
    .bind(s.tenant)
    .bind(Uuid::now_v7())
    .bind(g)
    .bind(s.unit)
    .execute(&pool)
    .await
    .unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("23514"));
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("roles_guest_has_no_unit_or_cohort")
    );

    let err = sqlx::query(
        "insert into roles (tenant_id, id, name, capability_class, group_id, cohort_id)
         values ($1, $2, 'Gjest', 'guest', $3, $4)",
    )
    .bind(s.tenant)
    .bind(Uuid::now_v7())
    .bind(g)
    .bind(s.cohort)
    .execute(&pool)
    .await
    .unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("23514"));
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("roles_guest_has_no_unit_or_cohort")
    );
}

#[tokio::test]
async fn the_runtime_role_cannot_delete_groups_or_their_history() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let s = seed(&admin).await;
    let app = db.app_pool().await;
    let g = group(&app, &s, envelope(42), None, None)
        .await
        .expect("fau_app may insert");
    add(&app, &s, g, "2026-09-23T10:00:00Z")
        .await
        .expect("fau_app may insert members");
    for sql in [
        "delete from group_members where group_id = $1",
        "delete from groups where id = $1",
    ] {
        let err = sqlx::query(sql).bind(g).execute(&app).await.unwrap_err();
        assert_eq!(sqlstate(&err).as_deref(), Some("42501"), "{sql}");
    }
}
