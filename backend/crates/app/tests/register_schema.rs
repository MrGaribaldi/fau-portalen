//! Migration 0004: the school register (docs/school-register-design.md §4, D9).
//! Every constraint, grant and policy 0004 relies on, proven in SQL before any Rust
//! depends on it.

mod common;
use common::TestDb;
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

const REGISTER_TABLES: &[&str] = &[
    "municipalities",
    "municipality_names",
    "municipality_numbers",
    "municipality_slug_history",
    "schools",
    "school_orgnr_history",
    "school_slug_history",
    "register_source_records",
    "register_sync_runs",
    "register_review_items",
    "school_submissions",
    "registered_faus",
    "school_fau_links",
    "register_lookups",
];

async fn municipality(pool: &sqlx::PgPool, number: &str, slug: &str) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into municipalities (id, name, county_number, county_name, slug, status, source, search_text)
         values ($1, $2, '03', 'Oslo', $3, 'active', 'kartverket', $2)",
    )
    .bind(id).bind(slug).bind(slug)
    .execute(pool).await?;
    sqlx::query("insert into municipality_numbers (municipality_id, number, valid_from) values ($1, $2, '2020-01-01')")
        .bind(id).bind(number)
        .execute(pool).await?;
    Ok(id)
}

async fn listed_school(
    pool: &sqlx::PgPool,
    m: Uuid,
    orgnr: &str,
    slug: Option<&str>,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into schools (id, municipality_id, origin, display_name, register_name, slug,
                              verification, orgnr, status, search_text)
         values ($1, $2, 'register', 'Skole', 'Skole', $3, 'listed', $4, 'active', 'skole')",
    )
    .bind(id)
    .bind(m)
    .bind(slug)
    .bind(orgnr)
    .execute(pool)
    .await?;
    Ok(id)
}

async fn submitted_school(pool: &sqlx::PgPool, m: Uuid) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into schools (id, municipality_id, origin, display_name, verification, status, search_text)
         values ($1, $2, 'submitted', 'Ny skole', 'pending', 'active', 'ny skole')",
    )
    .bind(id).bind(m)
    .execute(pool).await?;
    Ok(id)
}

#[tokio::test]
async fn register_tables_exist_and_carry_no_tenant_id() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    for table in REGISTER_TABLES {
        let exists: bool = sqlx::query_scalar(
            "select exists (select 1 from information_schema.tables where table_schema = 'public' and table_name = $1)",
        )
        .bind(table).fetch_one(&admin).await.unwrap();
        assert!(exists, "{table} must exist");
        let tenant_col: bool = sqlx::query_scalar(
            "select exists (select 1 from information_schema.columns
                            where table_schema = 'public' and table_name = $1 and column_name = 'tenant_id')",
        )
        .bind(table).fetch_one(&admin).await.unwrap();
        assert!(
            !tenant_col,
            "{table} is global reference data and must not carry tenant_id"
        );
    }
}

#[tokio::test]
async fn registered_faus_has_no_address_column() {
    // Ruling of 24 September 2026: Brreg addresses are personal data in practice.
    let db = TestDb::migrated().await;
    let cols: Vec<String> = sqlx::query_scalar(
        "select column_name from information_schema.columns
         where table_schema = 'public' and table_name in ('registered_faus', 'register_review_items')",
    )
    .fetch_all(&db.admin_pool()).await.unwrap();
    for c in cols {
        assert!(
            !c.contains("address") && !c.contains("adresse"),
            "no address column allowed: {c}"
        );
    }
}

#[tokio::test]
async fn contract_version_is_four() {
    let db = TestDb::migrated().await;
    let v: i32 = sqlx::query_scalar("select max(version) from schema_contract")
        .fetch_one(&db.admin_pool())
        .await
        .unwrap();
    assert_eq!(v, 4);
}

#[tokio::test]
async fn a_tenant_needs_a_real_school() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let err = sqlx::query(
        "insert into tenants (id, name, status, school_id) values ($1, 'FAU', 'pending', $2)",
    )
    .bind(Uuid::now_v7())
    .bind(Uuid::now_v7())
    .execute(&admin)
    .await
    .unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("23503"));
    assert_eq!(constraint_name(&err).as_deref(), Some("tenants_school_fk"));

    let err = sqlx::query(
        "insert into tenants (id, name, status, school_id) values ($1, 'FAU', 'pending', null)",
    )
    .bind(Uuid::now_v7())
    .execute(&admin)
    .await
    .unwrap_err();
    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("23502"),
        "school_id is not null"
    );
}

#[tokio::test]
async fn a_referenced_school_cannot_be_deleted() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let m = municipality(&admin, "3201", "3201-baerum").await.unwrap();
    let s = listed_school(&admin, m, "974552124", Some("hosle-skole"))
        .await
        .unwrap();
    sqlx::query(
        "insert into tenants (id, name, status, school_id) values ($1, 'FAU', 'active', $2)",
    )
    .bind(Uuid::now_v7())
    .bind(s)
    .execute(&admin)
    .await
    .unwrap();
    let err = sqlx::query("delete from schools where id = $1")
        .bind(s)
        .execute(&admin)
        .await
        .unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("23503"));
    let err = sqlx::query("delete from municipalities where id = $1")
        .bind(m)
        .execute(&admin)
        .await
        .unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("23503"));
}

#[tokio::test]
async fn school_slugs_are_unique_per_municipality_only() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let a = municipality(&admin, "1515", "1515-heroey").await.unwrap();
    let b = municipality(&admin, "1818", "1818-heroey").await.unwrap();
    listed_school(&admin, a, "900000001", Some("heroey-skule"))
        .await
        .unwrap();
    listed_school(&admin, b, "900000002", Some("heroey-skule"))
        .await
        .expect("the same slug in another municipality is fine");
    let err = listed_school(&admin, a, "900000003", Some("heroey-skule"))
        .await
        .unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("23505"));
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("schools_slug_current")
    );
}

#[tokio::test]
async fn municipality_slugs_and_current_numbers_are_unique() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    municipality(&admin, "3201", "3201-baerum").await.unwrap();
    let err = municipality(&admin, "3202", "3201-baerum")
        .await
        .unwrap_err();
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("municipalities_slug_current")
    );

    // A second municipality naming the same *current* number 3201, from a later
    // valid_from than the first's -- so this cannot also collide with
    // municipality_numbers' own (number, valid_from) primary key, and isolates the
    // partial unique index that forbids two currently-valid rows for one number.
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into municipalities (id, name, county_number, county_name, slug, status, source, search_text)
         values ($1, 'Bærum', '03', 'Oslo', '3201-baerum-2', 'active', 'kartverket', 'baerum')",
    )
    .bind(id)
    .execute(&admin)
    .await
    .unwrap();
    let err = sqlx::query(
        "insert into municipality_numbers (municipality_id, number, valid_from) values ($1, '3201', '2020-01-02')",
    )
    .bind(id)
    .execute(&admin)
    .await
    .unwrap_err();
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("municipality_numbers_current")
    );
}

#[tokio::test]
async fn an_unverified_school_cannot_hold_a_slug() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let m = municipality(&admin, "3201", "3201-baerum").await.unwrap();
    let err = sqlx::query(
        "insert into schools (id, municipality_id, origin, display_name, slug, verification, status, search_text)
         values ($1, $2, 'submitted', 'Ny', 'ny', 'pending', 'active', 'ny')",
    )
    .bind(Uuid::now_v7()).bind(m).execute(&admin).await.unwrap_err();
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("schools_slug_needs_verification")
    );
}

#[tokio::test]
async fn a_register_school_needs_an_orgnr_and_closure_needs_a_date() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let m = municipality(&admin, "3201", "3201-baerum").await.unwrap();
    let err = sqlx::query(
        "insert into schools (id, municipality_id, origin, display_name, verification, status, search_text)
         values ($1, $2, 'register', 'X', 'listed', 'active', 'x')",
    )
    .bind(Uuid::now_v7()).bind(m).execute(&admin).await.unwrap_err();
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("schools_register_origin_has_orgnr")
    );
    let err = sqlx::query(
        "insert into schools (id, municipality_id, origin, display_name, verification, orgnr, status, search_text)
         values ($1, $2, 'register', 'X', 'listed', '900000009', 'closed', 'x')",
    )
    .bind(Uuid::now_v7()).bind(m).execute(&admin).await.unwrap_err();
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("schools_closed_has_date")
    );
}

#[tokio::test]
async fn one_linked_fau_per_school_and_one_school_per_fau() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let m = municipality(&admin, "0301", "0301-oslo").await.unwrap();
    let s1 = listed_school(&admin, m, "900000011", None).await.unwrap();
    let s2 = listed_school(&admin, m, "900000012", None).await.unwrap();
    for orgnr in ["913706366", "913706367"] {
        sqlx::query(
            "insert into registered_faus (orgnr, registered_name, organisation_form, status, last_seen_in_source_at)
             values ($1, 'BESTUM FAU', 'FLI', 'active', now())",
        )
        .bind(orgnr).execute(&admin).await.unwrap();
    }
    let link = |s: Uuid, f: &'static str, state: &'static str| {
        let admin = admin.clone();
        async move {
            sqlx::query("insert into school_fau_links (school_id, fau_orgnr, method, state) values ($1, $2, 'address', $3)")
                .bind(s).bind(f).bind(state).execute(&admin).await
        }
    };
    link(s1, "913706366", "linked").await.unwrap();
    let err = link(s1, "913706367", "linked").await.unwrap_err();
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("school_fau_links_one_per_school")
    );
    let err = link(s2, "913706366", "linked").await.unwrap_err();
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("school_fau_links_one_per_fau")
    );
    link(s2, "913706366", "candidate")
        .await
        .expect("candidates are not exclusive");
}

#[tokio::test]
async fn the_runtime_role_reads_the_public_register_but_not_the_bookkeeping() {
    let db = TestDb::migrated().await;
    let app = db.app_pool().await;
    for table in [
        "municipalities",
        "municipality_names",
        "municipality_numbers",
        "municipality_slug_history",
        "schools",
        "school_slug_history",
        "registered_faus",
        "school_fau_links",
    ] {
        sqlx::query(&format!("select count(*) from {table}"))
            .execute(&app)
            .await
            .unwrap_or_else(|e| panic!("fau_app must read {table}: {e}"));
    }
    for table in [
        "register_source_records",
        "register_sync_runs",
        "register_review_items",
        "school_submissions",
        "register_lookups",
        "school_orgnr_history",
    ] {
        let err = sqlx::query(&format!("select count(*) from {table}"))
            .execute(&app)
            .await
            .unwrap_err();
        assert_eq!(
            sqlstate(&err).as_deref(),
            Some("42501"),
            "fau_app must not read {table}"
        );
    }
}

#[tokio::test]
async fn the_runtime_role_may_insert_only_a_pending_submitted_school() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let app = db.app_pool().await;
    let m = municipality(&admin, "3201", "3201-baerum").await.unwrap();

    submitted_school(&app, m)
        .await
        .expect("a pending submitted school is allowed");

    let err = listed_school(&app, m, "900000021", None).await.unwrap_err();
    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("42501"),
        "RLS refuses a listed school from fau_app"
    );

    let err = sqlx::query("update schools set display_name = 'x'")
        .execute(&app)
        .await
        .unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("42501"));
    let err = sqlx::query("insert into municipalities (id, name, county_number, county_name, slug, status, source, search_text)
                           values ($1, 'x', '03', 'x', '0000-x', 'active', 'manual', 'x')")
        .bind(Uuid::now_v7()).execute(&app).await.unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("42501"));
}

#[tokio::test]
async fn the_register_role_writes_the_register_and_deletes_only_held_schools() {
    let db = TestDb::migrated().await;
    let reg = db.register_pool().await;
    let m = municipality(&reg, "3201", "3201-baerum")
        .await
        .expect("fau_register writes municipalities");
    let listed = listed_school(&reg, m, "900000031", Some("hosle-skole"))
        .await
        .unwrap();
    let held = Uuid::now_v7();
    sqlx::query(
        "insert into schools (id, municipality_id, origin, display_name, verification, orgnr, status, search_text)
         values ($1, $2, 'register', 'Hosle', 'held', '900000032', 'active', 'hosle')",
    )
    .bind(held).bind(m).execute(&reg).await.unwrap();

    let gone = sqlx::query("delete from schools where id = $1")
        .bind(listed)
        .execute(&reg)
        .await
        .unwrap();
    assert_eq!(
        gone.rows_affected(),
        0,
        "RLS hides a listed school from delete"
    );
    let gone = sqlx::query("delete from schools where id = $1")
        .bind(held)
        .execute(&reg)
        .await
        .unwrap();
    assert_eq!(gone.rows_affected(), 1, "a held school may be deleted");

    let err = sqlx::query("update tenants set name = 'x'")
        .execute(&reg)
        .await
        .unwrap_err();
    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("42501"),
        "fau_register only reads tenants"
    );
    sqlx::query("select count(*) from tenants")
        .execute(&reg)
        .await
        .expect("fau_register reads tenants");
}

#[tokio::test]
async fn the_outbox_accepts_the_global_register_templates_only() {
    let db = TestDb::migrated().await;
    let reg = db.register_pool().await;
    for template in [
        "register.review_item",
        "register.seed_summary",
        "register.submission_approved",
        "register.sync_aborted",
    ] {
        sqlx::query("insert into outbox (id, template, recipient_email, created_at) values ($1, $2, 'fau@ewb-solutions.as', now())")
            .bind(Uuid::now_v7()).bind(template).execute(&reg).await
            .unwrap_or_else(|e| panic!("{template}: {e}"));
    }
    let err = sqlx::query("insert into outbox (id, template, recipient_email, created_at) values ($1, 'invitation.issued', 'x@example.test', now())")
        .bind(Uuid::now_v7()).execute(&reg).await.unwrap_err();
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("outbox_tenant_scoped_unless_global")
    );
}
