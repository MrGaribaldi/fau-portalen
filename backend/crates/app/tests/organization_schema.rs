//! Migration 0006: #3412's school structure -- school years, cohorts, organization units
//! and the cohorts a unit spans -- and the foreign keys 0002 left open on `roles`
//! (groups design §3.1, §8). Plain school structure, not pupils: no table here holds a
//! child's name.

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

async fn tenant(pool: &PgPool) -> Uuid {
    let school = common::membership::school(pool, &format!("org-{}", Uuid::now_v7())).await;
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into tenants (id, name, status, school_id) values ($1, 'FAU', 'active', $2)",
    )
    .bind(id)
    .bind(school)
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn school_year(pool: &PgPool, tenant: Uuid, name: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into school_years (tenant_id, id, name, starts_on, ends_on_exclusive)
         values ($1, $2, $3, '2026-08-01', '2027-08-01')",
    )
    .bind(tenant)
    .bind(id)
    .bind(name)
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn unit(
    pool: &PgPool,
    tenant: Uuid,
    year: Uuid,
    kind: &str,
    name: &str,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into organization_units (tenant_id, id, school_year_id, kind, name)
         values ($1, $2, $3, $4, $5)",
    )
    .bind(tenant)
    .bind(id)
    .bind(year)
    .bind(kind)
    .bind(name)
    .execute(pool)
    .await
    .map(|_| id)
}

async fn cohort(pool: &PgPool, tenant: Uuid, name: &str) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query("insert into cohorts (tenant_id, id, name) values ($1, $2, $3)")
        .bind(tenant)
        .bind(id)
        .bind(name)
        .execute(pool)
        .await
        .map(|_| id)
}

async fn link(
    pool: &PgPool,
    tenant: Uuid,
    unit: Uuid,
    cohort: Uuid,
    grade: i16,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "insert into unit_cohorts (tenant_id, unit_id, cohort_id, grade_level) values ($1, $2, $3, $4)",
    )
    .bind(tenant)
    .bind(unit)
    .bind(cohort)
    .bind(grade)
    .execute(pool)
    .await
    .map(|_| ())
}

async fn role(
    pool: &PgPool,
    tenant: Uuid,
    unit: Option<Uuid>,
    cohort: Option<Uuid>,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into roles (tenant_id, id, name, capability_class, unit_id, cohort_id)
         values ($1, $2, 'Kontaktforelder', 'member', $3, $4)",
    )
    .bind(tenant)
    .bind(id)
    .bind(unit)
    .bind(cohort)
    .execute(pool)
    .await
    .map(|_| id)
}

#[tokio::test]
async fn a_role_cannot_name_another_fau_s_unit_or_cohort() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (a, b) = (tenant(&pool).await, tenant(&pool).await);
    let year_b = school_year(&pool, b, "2026/27").await;
    let unit_b = unit(&pool, b, year_b, "grade", "7. trinn").await.unwrap();
    let cohort_b = cohort(&pool, b, "K2019").await.unwrap();

    // Positive control: FAU B's own role may name them.
    role(&pool, b, Some(unit_b), None).await.unwrap();
    role(&pool, b, None, Some(cohort_b)).await.unwrap();

    let err = role(&pool, a, Some(unit_b), None).await.unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("23503"));
    assert_eq!(constraint_name(&err).as_deref(), Some("roles_unit_fk"));
    let err = role(&pool, a, None, Some(cohort_b)).await.unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("23503"));
    assert_eq!(constraint_name(&err).as_deref(), Some("roles_cohort_fk"));
}

#[tokio::test]
async fn a_base_can_span_two_cohorts_at_different_grades() {
    // #3412's own example: base "Blå" spans K2018 at grade 3 and K2019 at grade 2.
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let t = tenant(&pool).await;
    let year = school_year(&pool, t, "2026/27").await;
    let blue = unit(&pool, t, year, "base", "Blå").await.unwrap();
    let k2018 = cohort(&pool, t, "K2018").await.unwrap();
    let k2019 = cohort(&pool, t, "K2019").await.unwrap();
    link(&pool, t, blue, k2018, 3).await.unwrap();
    link(&pool, t, blue, k2019, 2).await.unwrap();

    // Another FAU's cohort cannot be linked.
    let other = tenant(&pool).await;
    let foreign = cohort(&pool, other, "K2018").await.unwrap();
    let err = link(&pool, t, blue, foreign, 3).await.unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("23503"));
}

/// The quoted values in a check constraint's definition, as PostgreSQL prints them.
async fn allowed_values(pool: &PgPool, table: &str, constraint: &str) -> Vec<String> {
    let def: String = sqlx::query_scalar(
        "select pg_get_constraintdef(c.oid) from pg_constraint c
          where c.conrelid = $1::regclass and c.conname = $2 and c.contype = 'c'",
    )
    .bind(table)
    .bind(constraint)
    .fetch_one(pool)
    .await
    .unwrap();
    def.split('\'')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

#[tokio::test]
async fn unit_kinds_are_exactly_four_english_codes() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    assert_eq!(
        allowed_values(&pool, "organization_units", "organization_units_kind_check").await,
        ["grade", "class", "base", "teaching_group"]
    );
    let t = tenant(&pool).await;
    let year = school_year(&pool, t, "2026/27").await;
    let err = unit(&pool, t, year, "gruppe", "Gruppe 1")
        .await
        .unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("23514"));
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("organization_units_kind_check")
    );
}

#[tokio::test]
async fn grade_levels_are_those_of_grunnskole() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let t = tenant(&pool).await;
    let year = school_year(&pool, t, "2026/27").await;
    let class = unit(&pool, t, year, "class", "1A").await.unwrap();
    let tenth = unit(&pool, t, year, "class", "10A").await.unwrap();
    let k = cohort(&pool, t, "K2020").await.unwrap();
    let k10 = cohort(&pool, t, "K2011").await.unwrap();
    link(&pool, t, class, k, 1).await.unwrap();
    link(&pool, t, tenth, k10, 10).await.unwrap();
    for bad in [0_i16, 11] {
        let u = unit(&pool, t, year, "class", &format!("X{bad}"))
            .await
            .unwrap();
        let err = link(&pool, t, u, k, bad).await.unwrap_err();
        assert_eq!(sqlstate(&err).as_deref(), Some("23514"), "grade {bad}");
        assert_eq!(
            constraint_name(&err).as_deref(),
            Some("unit_cohorts_grade_level_check")
        );
    }
}

#[tokio::test]
async fn names_are_unique_per_fau_and_per_school_year() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let t = tenant(&pool).await;
    cohort(&pool, t, "K2019").await.unwrap();
    let err = cohort(&pool, t, "K2019").await.unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("23505"));
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("cohorts_name_unique")
    );

    let this_year = school_year(&pool, t, "2026/27").await;
    let next_year = school_year(&pool, t, "2027/28").await;
    unit(&pool, t, this_year, "class", "3A").await.unwrap();
    // Historical rows are kept, so next year's 3A is a new unit, not a rename.
    unit(&pool, t, next_year, "class", "3A").await.unwrap();
    let err = unit(&pool, t, this_year, "class", "3A").await.unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("23505"));
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("organization_units_name_unique")
    );
}

#[tokio::test]
async fn a_school_year_is_a_non_empty_half_open_period() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let t = tenant(&pool).await;
    let err = sqlx::query(
        "insert into school_years (tenant_id, id, name, starts_on, ends_on_exclusive)
         values ($1, $2, '2026/27', '2026-08-01', '2026-08-01')",
    )
    .bind(t)
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await
    .unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("23514"));
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("school_year_period_is_non_empty")
    );
}

#[tokio::test]
async fn the_runtime_role_may_add_structure_but_not_change_or_delete_it() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let t = tenant(&admin).await;
    let app = db.app_pool().await;
    let k = cohort(&app, t, "K2019").await.expect("fau_app may insert");
    for sql in [
        "update cohorts set name = 'K2020' where id = $1",
        "delete from cohorts where id = $1",
    ] {
        let err = sqlx::query(sql).bind(k).execute(&app).await.unwrap_err();
        assert_eq!(sqlstate(&err).as_deref(), Some("42501"), "{sql}");
    }
}
