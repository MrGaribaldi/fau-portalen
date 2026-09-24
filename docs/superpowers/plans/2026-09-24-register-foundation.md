# Register Foundation Implementation Plan (#3441, part 1)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Lay the school register's foundation: migration `0004_school_register.sql` with the
`fau_register` role and its grants, and the pure register rules in `fau-domain` (slugs, slug
resolution, search folding, the scope filter, and the Brreg FAU rules), all tested.

**Architecture:** The schema follows docs/school-register-design.md §4 as decided on 24 September
2026. It is global reference data with no `tenant_id`. `fau_register` alone writes it (D9), and
row-level security on `schools` confines the runtime role to inserting submitted schools. The rules
are pure functions in a new `fau_domain::register` module over strings and small structs. No HTTP
or SQL, which `tests/dependency_boundary.rs` enforces. Sync, source clients, the Brreg matcher, the
submission lookup, search queries and the CLI are later plans, and they consume what this plan
produces.

**Tech Stack:** Rust 1.98.1, sqlx 0.8, PostgreSQL 17.5, `unicode-normalization` 0.1 (new, domain
only).

**Spec:** `docs/school-register-design.md` (§4 schema, §5.4 lookups, §6 slugs, §7 matching, §2.3
scope, §2.5/§4.6 Brreg). Decisions and rulings: `docs/planning-decisions.md`, "School register
decided (#3441) — 24 September 2026". Conventions: `docs/superpowers/plans/2026-09-24-membership-foundation.md`.

## Global Constraints

- All technical content in English. User-facing strings are Bokmål source strings, and this plan
  has none.
- `crates/domain` declares neither `axum`, `sqlx`, `tower`, `hyper` nor `reqwest`.
- Schema conventions from 0002/0003: UUID primary keys, `timestamptz`, `text` plus `check` rather
  than enums, explicit grants, and nothing by default. No column name may contain `key`, `dek`,
  `kek`, `secret`, `private`, `passphrase`, `password`, `cipher`, `nonce` or `wrapped`
  (`schema_review.rs`). No register table carries `tenant_id`.
- Migrations are immutable once applied. 0004 must say in its header that it supersedes 0002's
  comment on `tenants.school_id`, and it must **not** recreate `tenants_one_live_per_school`, which
  0003 created.
- Brreg addresses are never stored: no address column on `registered_faus`, and none in any review
  item (ruling, planning-decisions 24 September).
- Slug rules, copied from §6: NFC, full lowercase; `æ→ae`, `ø→oe`, `å→aa`, `ä→ae`, `ö→oe`; `đ→d`,
  `ŋ→n`, `ŧ→t`, `ß→ss`; NFKD with combining marks dropped; apostrophes removed without a separator;
  every other run of non-`[a-z0-9]` becomes `-`, trimmed; capped at **80** characters at a hyphen
  boundary. The municipality segment is `<4-digit number>-<slug of the Norwegian name>` (D3).
- Scope filter (§2.3, D2): `ErSkole AND ErAktiv AND ErGrunnskole`, no category `10` or `25`,
  primary NACE (Prioritet 1) not `85.593` and not `85.3xx`, municipality not `2599`. On NSR data of
  24 September 2026 it gives 2,677 in scope, 31 adult education, 17 upper secondary and 8 abroad
  (measured).
- Tests: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test --workspace`;
  `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings` stay clean after
  every task. Tests are written first.
- Commits: `GIT_AUTHOR_NAME="Erik W. Bjønnes" GIT_AUTHOR_EMAIL=erik@ewb-solutions.as` (same for
  committer), message ending `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`, on branch
  `school-register-3441`. Never push.

## Decisions this plan makes

| Question | Decision | Reason |
| --- | --- | --- |
| Keeping the runtime role to submitted rows | RLS on `schools`: `fau_app` may select all rows and insert only `origin='submitted'`, `verification='pending'`, no slug, no orgnr | D9 says "insert for submissions only". A grant cannot restrict by value, and RLS costs four policies. |
| What `fau_register` may delete | Only `held` schools, by an RLS delete policy | §5.3: "the only delete is a held row that loses a match review". |
| `tenants.school_id` | `not null` plus `tenants_school_fk ... on delete restrict` | §4.2 recommendation. No production tenants exist, and the submitted-school path creates its row in the same transaction. |
| Register mail templates | 0004 widens `outbox_tenant_scoped_unless_global` with `register.review_item`, `register.seed_summary`, `register.submission_approved` and `register.sync_aborted` | Later plans send these. A migration is cheaper now than a second constraint swap later. |
| Test schools | `common::membership::school` inserts a listed school (fixed test municipality `0301`) through an admin connection to the same database | It was built to be the only fixture that changes (final review M11). |
| `pg_trgm` | Not created here | Only the search plan needs it, and §13 leaves its behaviour on CloudNativePG unverified. |
| Lossy folding of `æ` | `a` (`bærum` becomes `barum`) | The transliterated form already covers `baerum`. The lossy form exists for what people type without Norwegian letters. |
| Brreg name casing | Keep a name that has any lowercase letter. Otherwise title-case the first word, keep `FAU`/`SFO`/`AU` in capitals, take any word's casing from the school's display name, and lowercase the rest | §4.6: "BESTUM FAU" becomes "Bestum FAU". A prefill is a suggestion. |

---

## File Structure

```text
backend/
  db/roles.sql                               + fau_register role, connect, schema usage
  db/zz-compose-init.sh                      + fau_register login from FAU_REGISTER_PASSWORD
  migrations/0004_school_register.sql        new
  crates/domain/Cargo.toml                   + unicode-normalization
  crates/domain/src/lib.rs                   + pub mod register
  crates/domain/src/register/mod.rs          new: module doc, re-exports
  crates/domain/src/register/text.rs         new: latin_fold, collapse (crate-private)
  crates/domain/src/register/slug.rs         new: slugify, municipality_slug, first_free_slug, resolve
  crates/domain/src/register/search.rs       new: search_text, search_query
  crates/domain/src/register/scope.rs        new: NsrScopeFacts, classify, effective_in_scope
  crates/domain/src/register/brreg.rs        new: is_fau_name, address_keys, name_core, suggest_fau_name
  crates/app/tests/common/mod.rs             + fau_register login, TestDb::register_pool
  crates/app/tests/common/membership.rs      school() inserts a real register row
  crates/app/tests/register_schema.rs        new: 0004's constraints, grants and policies
  crates/app/tests/migrations.rs             contract version 3 -> 4
  crates/app/tests/membership_schema.rs      tenants inserted with a real school
compose.yaml, .env.example                   + FAU_REGISTER_PASSWORD
docs/app-foundation-operations.md            one line on the third role
```

---

### Task 1: Migration 0004, the `fau_register` role, and the schema tests

**Files:**
- Create: `backend/migrations/0004_school_register.sql`
- Create: `backend/crates/app/tests/register_schema.rs`
- Modify: `backend/db/roles.sql`, `backend/db/zz-compose-init.sh`, `compose.yaml`, `.env.example`, `docs/app-foundation-operations.md`
- Modify: `backend/crates/app/tests/common/mod.rs`, `backend/crates/app/tests/common/membership.rs`, `backend/crates/app/tests/migrations.rs`, `backend/crates/app/tests/membership_schema.rs`, plus every other test that inserts into `tenants` with a made-up `school_id` (find them with `grep -rn "insert into tenants" backend/crates/app/tests`)

**Interfaces:**
- Produces: tables `municipalities`, `municipality_names`, `municipality_numbers`,
  `municipality_slug_history`, `schools`, `school_orgnr_history`, `school_slug_history`,
  `register_source_records`, `register_sync_runs`, `register_review_items`, `school_submissions`,
  `registered_faus`, `school_fau_links`, `register_lookups`; role `fau_register`;
  `TestDb::register_pool(&self) -> PgPool`; `common::membership::school(pool, label) -> Uuid` now
  backed by a real row; `common::register::test_municipality(pool) -> Uuid`, the fixed `0301`
  municipality.

- [ ] **Step 1: Write the failing schema tests**

Create `backend/crates/app/tests/register_schema.rs`:

```rust
//! Migration 0004: the school register (docs/school-register-design.md §4, D9).
//! Every constraint, grant and policy 0004 relies on, proven in SQL before any Rust
//! depends on it.

mod common;
use common::TestDb;
use uuid::Uuid;

fn sqlstate(err: &sqlx::Error) -> Option<String> {
    err.as_database_error().and_then(|e| e.code()).map(|c| c.into_owned())
}

fn constraint_name(err: &sqlx::Error) -> Option<String> {
    err.as_database_error().and_then(|e| e.constraint()).map(|c| c.to_owned())
}

const REGISTER_TABLES: &[&str] = &[
    "municipalities", "municipality_names", "municipality_numbers", "municipality_slug_history",
    "schools", "school_orgnr_history", "school_slug_history", "register_source_records",
    "register_sync_runs", "register_review_items", "school_submissions", "registered_faus",
    "school_fau_links", "register_lookups",
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

async fn listed_school(pool: &sqlx::PgPool, m: Uuid, orgnr: &str, slug: Option<&str>) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into schools (id, municipality_id, origin, display_name, register_name, slug,
                              verification, orgnr, status, search_text)
         values ($1, $2, 'register', 'Skole', 'Skole', $3, 'listed', $4, 'active', 'skole')",
    )
    .bind(id).bind(m).bind(slug).bind(orgnr)
    .execute(pool).await?;
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
        assert!(!tenant_col, "{table} is global reference data and must not carry tenant_id");
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
        assert!(!c.contains("address") && !c.contains("adresse"), "no address column allowed: {c}");
    }
}

#[tokio::test]
async fn contract_version_is_four() {
    let db = TestDb::migrated().await;
    let v: i32 = sqlx::query_scalar("select max(version) from schema_contract")
        .fetch_one(&db.admin_pool()).await.unwrap();
    assert_eq!(v, 4);
}

#[tokio::test]
async fn a_tenant_needs_a_real_school() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let err = sqlx::query("insert into tenants (id, name, status, school_id) values ($1, 'FAU', 'pending', $2)")
        .bind(Uuid::now_v7()).bind(Uuid::now_v7())
        .execute(&admin).await.unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("23503"));
    assert_eq!(constraint_name(&err).as_deref(), Some("tenants_school_fk"));

    let err = sqlx::query("insert into tenants (id, name, status, school_id) values ($1, 'FAU', 'pending', null)")
        .bind(Uuid::now_v7())
        .execute(&admin).await.unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("23502"), "school_id is not null");
}

#[tokio::test]
async fn a_referenced_school_cannot_be_deleted() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let m = municipality(&admin, "3201", "3201-baerum").await.unwrap();
    let s = listed_school(&admin, m, "974552124", Some("hosle-skole")).await.unwrap();
    sqlx::query("insert into tenants (id, name, status, school_id) values ($1, 'FAU', 'active', $2)")
        .bind(Uuid::now_v7()).bind(s).execute(&admin).await.unwrap();
    let err = sqlx::query("delete from schools where id = $1").bind(s).execute(&admin).await.unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("23503"));
    let err = sqlx::query("delete from municipalities where id = $1").bind(m).execute(&admin).await.unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("23503"));
}

#[tokio::test]
async fn school_slugs_are_unique_per_municipality_only() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let a = municipality(&admin, "1515", "1515-heroey").await.unwrap();
    let b = municipality(&admin, "1818", "1818-heroey").await.unwrap();
    listed_school(&admin, a, "900000001", Some("heroey-skule")).await.unwrap();
    listed_school(&admin, b, "900000002", Some("heroey-skule")).await
        .expect("the same slug in another municipality is fine");
    let err = listed_school(&admin, a, "900000003", Some("heroey-skule")).await.unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("23505"));
    assert_eq!(constraint_name(&err).as_deref(), Some("schools_slug_current"));
}

#[tokio::test]
async fn municipality_slugs_and_current_numbers_are_unique() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    municipality(&admin, "3201", "3201-baerum").await.unwrap();
    let err = municipality(&admin, "3202", "3201-baerum").await.unwrap_err();
    assert_eq!(constraint_name(&err).as_deref(), Some("municipalities_slug_current"));
    let err = municipality(&admin, "3201", "3201-baerum-2").await.unwrap_err();
    assert_eq!(constraint_name(&err).as_deref(), Some("municipality_numbers_current"));
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
    assert_eq!(constraint_name(&err).as_deref(), Some("schools_slug_needs_verification"));
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
    assert_eq!(constraint_name(&err).as_deref(), Some("schools_register_origin_has_orgnr"));
    let err = sqlx::query(
        "insert into schools (id, municipality_id, origin, display_name, verification, orgnr, status, search_text)
         values ($1, $2, 'register', 'X', 'listed', '900000009', 'closed', 'x')",
    )
    .bind(Uuid::now_v7()).bind(m).execute(&admin).await.unwrap_err();
    assert_eq!(constraint_name(&err).as_deref(), Some("schools_closed_has_date"));
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
    assert_eq!(constraint_name(&err).as_deref(), Some("school_fau_links_one_per_school"));
    let err = link(s2, "913706366", "linked").await.unwrap_err();
    assert_eq!(constraint_name(&err).as_deref(), Some("school_fau_links_one_per_fau"));
    link(s2, "913706366", "candidate").await.expect("candidates are not exclusive");
}

#[tokio::test]
async fn the_runtime_role_reads_the_public_register_but_not_the_bookkeeping() {
    let db = TestDb::migrated().await;
    let app = db.app_pool().await;
    for table in ["municipalities", "municipality_names", "municipality_numbers", "municipality_slug_history",
                  "schools", "school_slug_history", "registered_faus", "school_fau_links"] {
        sqlx::query(&format!("select count(*) from {table}")).execute(&app).await
            .unwrap_or_else(|e| panic!("fau_app must read {table}: {e}"));
    }
    for table in ["register_source_records", "register_sync_runs", "register_review_items",
                  "school_submissions", "register_lookups", "school_orgnr_history"] {
        let err = sqlx::query(&format!("select count(*) from {table}")).execute(&app).await.unwrap_err();
        assert_eq!(sqlstate(&err).as_deref(), Some("42501"), "fau_app must not read {table}");
    }
}

#[tokio::test]
async fn the_runtime_role_may_insert_only_a_pending_submitted_school() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let app = db.app_pool().await;
    let m = municipality(&admin, "3201", "3201-baerum").await.unwrap();

    submitted_school(&app, m).await.expect("a pending submitted school is allowed");

    let err = listed_school(&app, m, "900000021", None).await.unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("42501"), "RLS refuses a listed school from fau_app");

    let err = sqlx::query("update schools set display_name = 'x'").execute(&app).await.unwrap_err();
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
    let m = municipality(&reg, "3201", "3201-baerum").await.expect("fau_register writes municipalities");
    let listed = listed_school(&reg, m, "900000031", Some("hosle-skole")).await.unwrap();
    let held = Uuid::now_v7();
    sqlx::query(
        "insert into schools (id, municipality_id, origin, display_name, verification, orgnr, status, search_text)
         values ($1, $2, 'register', 'Hosle', 'held', '900000032', 'active', 'hosle')",
    )
    .bind(held).bind(m).execute(&reg).await.unwrap();

    let gone = sqlx::query("delete from schools where id = $1").bind(listed).execute(&reg).await.unwrap();
    assert_eq!(gone.rows_affected(), 0, "RLS hides a listed school from delete");
    let gone = sqlx::query("delete from schools where id = $1").bind(held).execute(&reg).await.unwrap();
    assert_eq!(gone.rows_affected(), 1, "a held school may be deleted");

    let err = sqlx::query("update tenants set name = 'x'").execute(&reg).await.unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("42501"), "fau_register only reads tenants");
    sqlx::query("select count(*) from tenants").execute(&reg).await.expect("fau_register reads tenants");
}

#[tokio::test]
async fn the_outbox_accepts_the_global_register_templates_only() {
    let db = TestDb::migrated().await;
    let reg = db.register_pool().await;
    for template in ["register.review_item", "register.seed_summary", "register.submission_approved", "register.sync_aborted"] {
        sqlx::query("insert into outbox (id, template, recipient_email, created_at) values ($1, $2, 'fau@ewb-solutions.as', now())")
            .bind(Uuid::now_v7()).bind(template).execute(&reg).await
            .unwrap_or_else(|e| panic!("{template}: {e}"));
    }
    let err = sqlx::query("insert into outbox (id, template, recipient_email, created_at) values ($1, 'invitation.issued', 'x@example.test', now())")
        .bind(Uuid::now_v7()).execute(&reg).await.unwrap_err();
    assert_eq!(constraint_name(&err).as_deref(), Some("outbox_tenant_scoped_unless_global"));
}
```

Also change both `assert_eq!(version, 3 …)` lines in `backend/crates/app/tests/migrations.rs` (lines
23 and 96) to `4`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test -p fau-app --test register_schema`
Expected: compile error, because `register_pool` is not defined. After Step 3's harness change, the
tests fail because the tables do not exist.

- [ ] **Step 3: Extend the harness**

In `backend/crates/app/tests/common/mod.rs`, grant login to the third role. Change
`for role in ["fau_app", "fau_migrate"]` to `for role in ["fau_app", "fau_migrate", "fau_register"]`,
and add beside `app_pool`:

```rust
    /// A pool connected as the register role `fau_register` (D9), which alone writes
    /// the school register.
    pub async fn register_pool(&self) -> PgPool {
        PgPool::connect(&self.role_url("fau_register"))
            .await
            .expect("connect as fau_register")
    }
```

Create `backend/crates/app/tests/common/register.rs`, and add `pub mod register;` to
`common/mod.rs` next to `pub mod membership;`:

```rust
//! Register fixtures. Inserted through a superuser connection to the caller's own
//! database, because neither the runtime role nor the harness's usual pools may
//! write listed schools (D9, RLS in 0004).

use sqlx::PgPool;
use uuid::Uuid;

/// The fixed test municipality, 0301 Oslo, with a stable id.
pub const TEST_MUNICIPALITY: Uuid = Uuid::from_u128(0x0199_0000_0000_7000_8000_0000_0000_0301);

/// A superuser pool on the same database `pool` is connected to.
pub async fn admin_on_same_database(pool: &PgPool) -> PgPool {
    let db: String = sqlx::query_scalar("select current_database()")
        .fetch_one(pool)
        .await
        .expect("read current_database()");
    PgPool::connect(&super::with_database(&super::admin_url(), &db))
        .await
        .expect("connect as superuser to the test database")
}

/// Ensures the test municipality exists; idempotent.
pub async fn test_municipality(admin: &PgPool) -> Uuid {
    sqlx::query(
        "insert into municipalities (id, name, county_number, county_name, slug, status, source, search_text)
         values ($1, 'Oslo', '03', 'Oslo', '0301-oslo', 'active', 'kartverket', 'oslo')
         on conflict (id) do nothing",
    )
    .bind(TEST_MUNICIPALITY)
    .execute(admin)
    .await
    .expect("insert the test municipality");
    sqlx::query(
        "insert into municipality_numbers (municipality_id, number, valid_from)
         values ($1, '0301', '1838-01-01') on conflict do nothing",
    )
    .bind(TEST_MUNICIPALITY)
    .execute(admin)
    .await
    .expect("insert the test municipality's number");
    TEST_MUNICIPALITY
}

/// Ensures a listed school with this id exists in the test municipality; idempotent.
/// No slug, so any number of test schools can coexist.
pub async fn listed_school(admin: &PgPool, id: Uuid, label: &str) -> Uuid {
    let m = test_municipality(admin).await;
    // A nine-digit orgnr derived from the id, unique per id in practice.
    let orgnr = format!("{:09}", id.as_u128() % 1_000_000_000);
    sqlx::query(
        "insert into schools (id, municipality_id, origin, display_name, register_name,
                              verification, orgnr, status, search_text)
         values ($1, $2, 'register', $3, $3, 'listed', $4, 'active', $3)
         on conflict (id) do nothing",
    )
    .bind(id)
    .bind(m)
    .bind(label)
    .bind(orgnr)
    .execute(admin)
    .await
    .expect("insert a listed test school");
    id
}
```

In `backend/crates/app/tests/common/membership.rs`, make `school` insert the row. Replace its doc
comment's "Today a school is a bare uuid … async." with "Backed by a listed register row (#3441),
inserted idempotently.", and after the FNV loop:

```rust
    let id = uuid::Builder::from_custom_bytes(hash.to_be_bytes()).into_uuid();
    let admin = super::register::admin_on_same_database(pool).await;
    super::register::listed_school(&admin, id, label).await;
    admin.close().await;
    id
```

Rename its `_pool` parameter to `pool`.

Every other test that inserts a tenant with `school_id` null or a fresh `Uuid::now_v7()` must use a
real school. In `membership_schema.rs`, change the helper to
`async fn tenant(pool: &PgPool, status: &str, school: Uuid)`, and pass `common::membership::school(pool, "<distinct label>").await`
wherever a caller passed `None` or `Some(Uuid::now_v7())`. The helper's callers must keep their
meaning: two `None`s used to mean two tenants that do not collide, so give them distinct labels.
Apply the same pattern to each hit of `grep -rn "insert into tenants" backend/crates/app/tests`.

- [ ] **Step 4: Add the role**

In `backend/db/roles.sql`, inside the first `do $$` block after `fau_app`:

```sql
  begin
    create role fau_register nologin;
  exception when duplicate_object or unique_violation then
    null;
  end;
```

In the second block: `execute format('grant connect on database %I to fau_register', current_database());`.
After `grant usage on schema public to fau_app;`:

```sql
-- The register role (#3441, D9) alone writes the school register, and runs the
-- weekly sync and the submission lookups. Table privileges come from migration 0004.
grant usage on schema public to fau_register;
revoke create on schema public from fau_register;
```

Also update the header comment "#3424 provisions the same two names" to "the same three names".

In `backend/db/zz-compose-init.sh`, add `-v register_pw="${FAU_REGISTER_PASSWORD:-fau_register}" \`
and `alter role fau_register login password :'register_pw';`. In `compose.yaml` under
`db.environment`, add `FAU_REGISTER_PASSWORD: ${FAU_REGISTER_PASSWORD:-fau_register}`. In
`.env.example`, add `FAU_REGISTER_PASSWORD=fau_register` after `FAU_APP_PASSWORD`. In
`docs/app-foundation-operations.md`, where `FAU_MIGRATE_PASSWORD` / `FAU_APP_PASSWORD` are
explained (line ~79), add `FAU_REGISTER_PASSWORD` for the register role (#3441). Note that a
**new** dev volume is needed for the init script to run again.

- [ ] **Step 5: Write the migration**

Create `backend/migrations/0004_school_register.sql`:

```sql
-- 0004: the school and municipality register (#3441). Implements
-- docs/school-register-design.md sections 4.1-4.6 and 5.4, as decided on
-- 24 September 2026 (docs/planning-decisions.md, "School register decided").
--
-- Global, public reference data: no table here carries tenant_id, and nothing is
-- encrypted -- this is Udir's, Kartverket's, SSB's and Brreg's open data. The one
-- exception, who submitted a school, lives in school_submissions and is personal data
-- whose retention #3426 owns. Brreg FAU addresses are personal data in practice (many
-- are a parent's home address) and are never stored: registered_faus has no address.
--
-- Supersedes 0002's comment on tenants.school_id. That comment predicted a
-- tenant-scoped schools (tenant_id, id) and a composite, deferrable foreign key. The
-- flow spec (section 9) made schools global, so the key below is a plain
-- references schools (id), with no circularity. The one-live-FAU-per-school index
-- already exists (0003, tenants_one_live_per_school) and is not created again.
--
-- Privileges (D9): fau_register alone writes the register. fau_app reads the public
-- tables and may insert only a pending submitted school, its submission and its lookup
-- row -- the insert on schools is confined by row-level security below.

-- 1. Municipalities (4.1).
create table municipalities (
  id                uuid        primary key,
  -- The Norwegian name (Kartverket kommunenavnNorsk), which the slug follows (D3).
  name              text        not null,
  official_name     text,
  county_number     text        not null check (county_number ~ '^[0-9]{2}$'),
  county_name       text        not null,
  slug              text        not null,
  status            text        not null check (status in ('active', 'dissolved')),
  dissolved_on      date,
  -- Svalbard (2100) is 'manual': neither source lists it (D2).
  source            text        not null check (source in ('kartverket', 'manual')),
  source_checked_at timestamptz,
  search_text       text        not null,
  created_at        timestamptz not null default now(),
  updated_at        timestamptz not null default now(),
  constraint municipalities_dissolved_has_date check ((status = 'dissolved') = (dissolved_on is not null))
);
create unique index municipalities_slug_current on municipalities (slug);

create table municipality_names (
  municipality_id uuid    not null references municipalities (id) on delete restrict,
  name            text    not null,
  language        text    not null,
  priority        integer not null,
  primary key (municipality_id, name)
);

-- A number is reused over time (0716 was Våle, then Re), so it is keyed on
-- (number, valid_from) and a lookup by number must say when.
create table municipality_numbers (
  municipality_id uuid not null references municipalities (id) on delete restrict,
  number          text not null check (number ~ '^[0-9]{4}$'),
  valid_from      date not null,
  valid_until     date,
  primary key (number, valid_from),
  check (valid_until is null or valid_from < valid_until)
);
create unique index municipality_numbers_current on municipality_numbers (number) where valid_until is null;
create index municipality_numbers_municipality_idx on municipality_numbers (municipality_id);

create table municipality_slug_history (
  slug            text        not null,
  municipality_id uuid        not null references municipalities (id) on delete restrict,
  valid_from      timestamptz not null,
  valid_until     timestamptz not null,
  primary key (slug, valid_from),
  check (valid_from < valid_until)
);

-- 2. Schools (4.2).
create table schools (
  id                   uuid        primary key,
  municipality_id      uuid        not null references municipalities (id) on delete restrict,
  origin               text        not null check (origin in ('register', 'submitted')),
  display_name         text        not null,
  register_name        text,
  display_name_curated boolean     not null default false,
  slug                 text,
  -- listed: from NSR; pending: submitted, awaiting review; verified: submitted and
  -- accepted; rejected; held: an NSR row held back while a match is reviewed (4.5).
  verification         text        not null
    check (verification in ('listed', 'pending', 'verified', 'rejected', 'held')),
  verified_at          timestamptz,
  orgnr                text        unique check (orgnr ~ '^[0-9A-Z]{9}$'),
  ownership            text        check (ownership in ('public', 'private')),
  grade_from           smallint    check (grade_from between 1 and 13),
  grade_to             smallint    check (grade_to between 1 and 13),
  register_language    text,
  website              text,
  street_address       text,
  postcode             text,
  post_town            text,
  in_scope             boolean     not null default true,
  scope_override       boolean,
  status               text        not null check (status in ('active', 'closed')),
  closed_on            date,
  closure_reason       text
    check (closure_reason in ('closed', 'merged', 'duplicate', 'rejected', 'out_of_scope')),
  -- Many-to-one: several closed schools may name one successor (merging FAU-er, #3498).
  successor_id         uuid        references schools (id) on delete restrict,
  source_changed_at    timestamptz,
  last_seen_in_source_at timestamptz,
  search_text          text        not null,
  created_at           timestamptz not null default now(),
  updated_at           timestamptz not null default now(),
  constraint schools_closed_has_date check ((status = 'closed') = (closed_on is not null)),
  constraint schools_reason_only_when_closed check (status = 'closed' or closure_reason is null),
  constraint schools_register_origin_has_orgnr check (origin = 'submitted' or orgnr is not null),
  constraint schools_slug_needs_verification check (slug is null or verification in ('listed', 'verified')),
  constraint schools_grades_ordered check (grade_from is null or grade_to is null or grade_from <= grade_to),
  constraint schools_not_own_successor check (successor_id is null or successor_id <> id)
);
-- ADR-002 section 4: school slugs are unique within a municipality, among current slugs.
create unique index schools_slug_current on schools (municipality_id, slug) where slug is not null;
create index schools_municipality_idx on schools (municipality_id);

create table school_orgnr_history (
  orgnr       text        primary key,
  school_id   uuid        not null references schools (id) on delete restrict,
  valid_from  timestamptz not null,
  valid_until timestamptz
);
create index school_orgnr_history_school_idx on school_orgnr_history (school_id);

-- Keyed on the municipality at the time, so an old path still resolves after the
-- school moved municipality in a split.
create table school_slug_history (
  municipality_id uuid        not null references municipalities (id) on delete restrict,
  slug            text        not null,
  school_id       uuid        not null references schools (id) on delete restrict,
  valid_from      timestamptz not null,
  valid_until     timestamptz not null,
  primary key (municipality_id, slug, valid_from),
  check (valid_from < valid_until)
);
create index school_slug_history_school_idx on school_slug_history (school_id);

alter table tenants alter column school_id set not null;
alter table tenants
  add constraint tenants_school_fk foreign key (school_id) references schools (id) on delete restrict;

-- 3. Brreg FAU entities and their links (4.6). No address column, on purpose.
create table registered_faus (
  orgnr                  text        primary key check (orgnr ~ '^[0-9]{9}$'),
  registered_name        text        not null,
  organisation_form      text        not null,
  municipality_number    text        check (municipality_number ~ '^[0-9]{4}$'),
  status                 text        not null check (status in ('active', 'deleted')),
  last_seen_in_source_at timestamptz not null,
  created_at             timestamptz not null default now(),
  updated_at             timestamptz not null default now()
);

create table school_fau_links (
  school_id  uuid        not null references schools (id) on delete restrict,
  fau_orgnr  text        not null references registered_faus (orgnr) on delete restrict,
  method     text        not null check (method in ('address', 'name', 'operator')),
  state      text        not null check (state in ('linked', 'candidate', 'rejected')),
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  primary key (school_id, fau_orgnr)
);
create unique index school_fau_links_one_per_school on school_fau_links (school_id) where state = 'linked';
create unique index school_fau_links_one_per_fau on school_fau_links (fau_orgnr) where state = 'linked';
create index school_fau_links_fau_idx on school_fau_links (fau_orgnr);

-- 4. Sync bookkeeping (4.3). NSR payloads only: Brreg payloads carry addresses.
create table register_source_records (
  source            text        not null check (source in ('nsr')),
  external_id       text        not null,
  payload           jsonb       not null,
  payload_sha256    text        not null check (payload_sha256 ~ '^[0-9a-f]{64}$'),
  source_changed_at timestamptz,
  fetched_at        timestamptz not null,
  in_scope          boolean     not null,
  scope_reason      text        not null,
  primary key (source, external_id)
);

create table register_sync_runs (
  id           uuid        primary key,
  kind         text        not null check (kind in ('seed', 'sync', 'dry_run')),
  started_at   timestamptz not null,
  finished_at  timestamptz,
  outcome      text        check (outcome in ('applied', 'no_change', 'aborted', 'failed')),
  counts       jsonb       not null default '{}'::jsonb check (jsonb_typeof(counts) = 'object'),
  abort_reason text,
  constraint register_sync_runs_outcome_when_finished check ((finished_at is null) = (outcome is null))
);

create table register_review_items (
  id              uuid        primary key,
  kind            text        not null check (kind in (
                    'closure_with_fau', 'possible_reregistration', 'possible_submission_match',
                    'municipality_split_or_merge', 'mass_change', 'unknown_municipality_number',
                    'fau_several_at_school', 'fau_several_schools', 'fau_match_conflict',
                    'submission_matches_listed_school')),
  school_id       uuid        references schools (id) on delete restrict,
  other_school_id uuid        references schools (id) on delete restrict,
  municipality_id uuid        references municipalities (id) on delete restrict,
  fau_orgnr       text        references registered_faus (orgnr) on delete restrict,
  -- Ids, codes, names and orgnrs only; never an address (ruling, 24 September 2026).
  details         jsonb       not null default '{}'::jsonb check (jsonb_typeof(details) = 'object'),
  created_at      timestamptz not null default now(),
  resolved_at     timestamptz,
  resolution      text,
  constraint register_review_items_resolved_together check ((resolved_at is null) = (resolution is null))
);
create index register_review_items_open_idx on register_review_items (created_at) where resolved_at is null;

-- 5. Submitted schools (4.4) and their immediate lookup (5.4, D9).
create table school_submissions (
  id               uuid        primary key,
  school_id        uuid        not null unique references schools (id) on delete restrict,
  submitted_by     uuid        not null references accounts (id),
  submitted_name   text        not null,
  decision_url     text        not null,
  decision_kind    text        not null
    check (decision_kind in ('municipal_decision', 'udir_private_school_approval')),
  document_title   text,
  retrieval_status text        not null check (retrieval_status in (
                     'fetched', 'not_allowlisted', 'failed', 'quarantined', 'attached_by_reviewer')),
  retrieved_at     timestamptz,
  review_state     text        not null check (review_state in ('pending', 'verified', 'matched', 'rejected')),
  reviewed_at      timestamptz,
  created_at       timestamptz not null default now()
);

create table register_lookups (
  id            uuid        primary key,
  submission_id uuid        not null unique references school_submissions (id) on delete restrict,
  queued_at     timestamptz not null,
  processed_at  timestamptz,
  outcome       text        check (outcome in ('approved', 'review', 'not_found', 'failed')),
  attempts      integer     not null default 0 check (attempts >= 0),
  constraint register_lookups_outcome_when_processed check ((processed_at is null) = (outcome is null))
);
create index register_lookups_queued_idx on register_lookups (queued_at) where processed_at is null;

-- 6. Register mail is global (no tenant): widen 0003's list deliberately.
alter table outbox drop constraint outbox_tenant_scoped_unless_global;
alter table outbox add constraint outbox_tenant_scoped_unless_global
  check (tenant_id is not null or template in (
    'signup.collision', 'register.review_item', 'register.seed_summary',
    'register.submission_approved', 'register.sync_aborted'));

-- 7. Privileges (D9).
grant select, insert, update on
  municipalities, municipality_names, municipality_numbers, municipality_slug_history,
  schools, school_orgnr_history, school_slug_history, registered_faus, school_fau_links,
  register_source_records, register_sync_runs, register_review_items,
  school_submissions, register_lookups
  to fau_register;
-- Only a held row, which nothing references, may be deleted (5.3); RLS below narrows it.
grant delete on schools to fau_register;
-- Whether a school has an FAU decides between automatic change and review (D8).
grant select on tenants to fau_register;
grant select, insert on audit_events to fau_register;
grant select, insert on outbox to fau_register;
grant select on schema_contract to fau_register;

grant select on
  municipalities, municipality_names, municipality_numbers, municipality_slug_history,
  schools, school_slug_history, registered_faus, school_fau_links
  to fau_app;
grant insert on schools, school_submissions, register_lookups to fau_app;

alter table schools enable row level security;
create policy schools_app_read on schools for select to fau_app using (true);
create policy schools_app_submit on schools for insert to fau_app
  with check (origin = 'submitted' and verification = 'pending' and slug is null
              and orgnr is null and not display_name_curated);
create policy schools_register_read on schools for select to fau_register using (true);
create policy schools_register_insert on schools for insert to fau_register with check (true);
create policy schools_register_update on schools for update to fau_register using (true) with check (true);
create policy schools_register_delete_held on schools for delete to fau_register using (verification = 'held');

insert into schema_contract (version) values (4);
```

- [ ] **Step 6: Run the whole suite**

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test --workspace`
Expected: all tests pass, including the 14 new ones in `register_schema.rs`, plus
`schema_review.rs`'s standing guards over the new tables. If a guard fails on a column name, rename
the column. Do not weaken the guard. If an older test fails with `tenants_school_fk` or 23502, it
still inserts a made-up school: route it through `common::membership::school`.

Then: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings`.

- [ ] **Step 7: Commit**

```bash
git add backend compose.yaml .env.example docs/app-foundation-operations.md
git commit -m "Add migration 0004: the school register, its role and its schema tests (#3441)"
```

---

### Task 2: Domain: text folding and slugs

**Files:**
- Modify: `backend/Cargo.toml` (`[workspace.dependencies]`: `unicode-normalization = "0.1"`), `backend/crates/domain/Cargo.toml` (`unicode-normalization = { workspace = true }`), `backend/crates/domain/src/lib.rs` (`pub mod register;`)
- Create: `backend/crates/domain/src/register/mod.rs`, `text.rs`, `slug.rs`

**Interfaces:**
- Produces: `fau_domain::register::slug::{slugify, municipality_slug, first_free_slug, resolve, Resolution, SlugError, MAX_SLUG_LEN}`;
  crate-private `register::text::{latin_fold, collapse, Letters}`, used by Tasks 3 and 5.

- [ ] **Step 1: Write the module skeleton and failing tests**

`backend/crates/domain/src/register/mod.rs`:

```rust
//! The school and municipality register (#3441, docs/school-register-design.md):
//! pure rules over names and register facts. Fetching, storing and syncing live in
//! other crates.

pub mod slug;
mod text;
```

`backend/crates/domain/src/register/text.rs`:

```rust
//! Shared Latin folding for slugs (section 6) and search (section 7). One table of
//! letters, so a slug and a search form never disagree about a letter.

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// How the Norwegian letters and their neighbours are spelled in ASCII.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Letters {
    /// ADR-002's reversible form: æ→ae, ø→oe, å→aa, ä→ae, ö→oe.
    Transliterate,
    /// What people type without Norwegian letters: æ→a, ø→o, å→a, ä→a, ö→o.
    Lossy,
}

/// NFC and full lowercase, the letter table, then NFKD with combining marks dropped.
/// Letters of other scripts pass through; `collapse` drops them.
pub(crate) fn latin_fold(input: &str, letters: Letters) -> String {
    let lower = input.nfc().collect::<String>().to_lowercase();
    let mut mapped = String::with_capacity(lower.len() + 8);
    for c in lower.chars() {
        match (c, letters) {
            ('æ' | 'ä', Letters::Transliterate) => mapped.push_str("ae"),
            ('ø' | 'ö', Letters::Transliterate) => mapped.push_str("oe"),
            ('å', Letters::Transliterate) => mapped.push_str("aa"),
            ('æ' | 'ä' | 'å', Letters::Lossy) => mapped.push('a'),
            ('ø' | 'ö', Letters::Lossy) => mapped.push('o'),
            ('đ', _) => mapped.push('d'),
            ('ŋ', _) => mapped.push('n'),
            ('ŧ', _) => mapped.push('t'),
            ('ß', _) => mapped.push_str("ss"),
            _ => mapped.push(c),
        }
    }
    mapped.nfkd().filter(|c| !is_combining_mark(*c)).collect()
}

fn is_apostrophe(c: char) -> bool {
    matches!(c, '\'' | '\u{2019}' | '\u{02BC}' | '`')
}

/// Keeps `[a-z0-9]` (ASCII-lowercasing anything NFKD produced in capitals), drops
/// apostrophes without a separator, and turns every other run into one `sep`,
/// trimmed at both ends.
pub(crate) fn collapse(input: &str, sep: char) -> String {
    let mut out = String::with_capacity(input.len());
    let mut gap = false;
    for c in input.chars() {
        if is_apostrophe(c) {
            continue;
        }
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if gap && !out.is_empty() {
                out.push(sep);
            }
            gap = false;
            out.push(c);
        } else {
            gap = true;
        }
    }
    out
}
```

`backend/crates/domain/src/register/slug.rs`, beginning with the tests:

```rust
//! Slug minting and resolution (section 6; ADR-002 sections 3-4, amended by D4).

use super::text::{collapse, latin_fold, Letters};

/// Section 6, step 8.
pub const MAX_SLUG_LEN: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlugError {
    /// Nothing sluggable was left, e.g. a name in a non-Latin script only.
    Empty,
    /// A municipality number that is not four ASCII digits.
    InvalidMunicipalityNumber,
}

impl std::fmt::Display for SlugError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SlugError::Empty => "name has no sluggable characters",
            SlugError::InvalidMunicipalityNumber => "municipality number is not four digits",
        })
    }
}

impl std::error::Error for SlugError {}

/// What a path segment resolves to (ADR-002's resolution rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution<Id> {
    /// The current holder: serve it.
    Current(Id),
    /// A single former holder: 301 to its canonical path.
    Redirect(Id),
    /// Nobody, or several former holders: 404, never a guess.
    NotFound,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(name: &str) -> String {
        slugify(name).unwrap()
    }

    #[test]
    fn section_six_examples_from_the_real_data() {
        assert_eq!(municipality_slug("3911", "Færder").unwrap(), "3911-faerder");
        assert_eq!(municipality_slug("1515", "Herøy").unwrap(), "1515-heroey");
        assert_eq!(municipality_slug("1818", "Herøy").unwrap(), "1818-heroey");
        assert_eq!(municipality_slug("5540", "Kåfjord").unwrap(), "5540-kaafjord");
        assert_eq!(municipality_slug("5610", "Karasjok").unwrap(), "5610-karasjok");
        assert_eq!(municipality_slug("3201", "Bærum").unwrap(), "3201-baerum");
        assert_eq!(municipality_slug("0301", "Oslo").unwrap(), "0301-oslo");
        assert_eq!(s("Grünerløkka skole"), "grunerloekka-skole");
        assert_eq!(s("St. Svithun skole"), "st-svithun-skole");
        assert_eq!(s("Deanu Sàmeskuvla"), "deanu-sameskuvla");
        assert_eq!(s("Máze Skuvla/Masi skole Máze skole"), "maze-skuvla-masi-skole-maze-skole");
        assert_eq!(s("Hosle skole"), "hosle-skole");
    }

    #[test]
    fn same_number_different_era() {
        assert_eq!(municipality_slug("0716", "Våle").unwrap(), "0716-vaale");
        assert_eq!(municipality_slug("0716", "Re").unwrap(), "0716-re");
    }

    #[test]
    fn sami_names_fold_to_ascii() {
        assert_eq!(s("Gáivuotna"), "gaivuotna");
        assert_eq!(s("Kárášjohka"), "karasjohka");
        assert_eq!(s("Guovdageaidnu"), "guovdageaidnu");
        assert_eq!(s("Aarborte"), "aarborte");
        assert_eq!(s("Unjárga"), "unjarga");
        assert_eq!(s("Deatnu đŋŧ"), "deatnu-dnt");
        assert_eq!(s("Stïentje"), "stientje");
    }

    #[test]
    fn d4_letters_beyond_ae_oe_aa() {
        assert_eq!(s("Märta Östgård"), "maerta-oestgaard");
        assert_eq!(s("Straße"), "strasse");
        assert_eq!(s("Čáhcesuolu"), "cahcesuolu");
    }

    #[test]
    fn punctuation() {
        assert_eq!(s("Children's International School"), "childrens-international-school");
        assert_eq!(s("Children’s"), "childrens");
        assert_eq!(s("Fossen skole 1.-4. skole"), "fossen-skole-1-4-skole");
        assert_eq!(s("Viti skole avd Nordbyhagen Sone 1, 2 & 3"), "viti-skole-avd-nordbyhagen-sone-1-2-3");
        assert_eq!(s("  --Elverum kommune - Ydalir skole--  "), "elverum-kommune-ydalir-skole");
    }

    #[test]
    fn decomposed_input_is_composed_first() {
        // "å" as a + U+030A, as some sources send it.
        assert_eq!(s("Ka\u{030A}fjord"), "kaafjord");
    }

    #[test]
    fn the_cap_cuts_at_a_hyphen() {
        let long = "abcdefghij ".repeat(10); // 10 words of 10 letters
        let slug = s(&long);
        assert!(slug.len() <= MAX_SLUG_LEN, "{} > {MAX_SLUG_LEN}", slug.len());
        assert_eq!(slug, ["abcdefghij"; 7].join("-"), "76 characters, cut before the eighth word");
        assert!(!slug.ends_with('-'));
        let one_word = "a".repeat(100);
        assert_eq!(s(&one_word).len(), MAX_SLUG_LEN, "no hyphen to cut at: hard cut");
    }

    #[test]
    fn slugging_a_slug_changes_nothing() {
        for name in ["Grünerløkka skole", "Máze Skuvla/Masi skole Máze skole", "Children's", &"abcdefghij ".repeat(10)] {
            let once = s(name);
            assert_eq!(s(&once), once, "{name}");
        }
    }

    #[test]
    fn case_only_changes_give_the_same_slug() {
        assert_eq!(s("HOSLE SKOLE"), s("Hosle skole"));
    }

    #[test]
    fn nothing_sluggable_is_an_error() {
        assert_eq!(slugify("Школа"), Err(SlugError::Empty));
        assert_eq!(slugify(" - "), Err(SlugError::Empty));
        assert_eq!(municipality_slug("301", "Oslo"), Err(SlugError::InvalidMunicipalityNumber));
        assert_eq!(municipality_slug("03O1", "Oslo"), Err(SlugError::InvalidMunicipalityNumber));
    }

    #[test]
    fn municipality_segments_always_start_with_four_digits_and_a_hyphen() {
        // ADR-002: no register segment can collide with a root path or locale code.
        for (n, name) in [("0301", "Oslo"), ("2100", "Svalbard"), ("5501", "Tromsø")] {
            let seg = municipality_slug(n, name).unwrap();
            assert!(seg.len() > 5 && seg.as_bytes()[..4].iter().all(u8::is_ascii_digit) && seg.as_bytes()[4] == b'-');
        }
    }

    #[test]
    fn collisions_take_the_post_town_then_a_number() {
        let taken = ["hosle-skole", "hosle-skole-hosle", "hosle-skole-hosle-2"];
        let is_taken = |c: &str| taken.contains(&c);
        assert_eq!(first_free_slug("bekkestua-skole", Some("Hosle"), is_taken), "bekkestua-skole");
        assert_eq!(first_free_slug("hosle-skole", Some("HOSLE"), is_taken), "hosle-skole-hosle-3");
        let only_base = |c: &str| c == "hosle-skole";
        assert_eq!(first_free_slug("hosle-skole", Some("Hosle"), only_base), "hosle-skole-hosle");
        assert_eq!(first_free_slug("hosle-skole", None, only_base), "hosle-skole-2");
        assert_eq!(first_free_slug("hosle-skole", Some("Школа"), only_base), "hosle-skole-2");
    }

    #[test]
    fn resolution_follows_adr_002() {
        assert_eq!(resolve(Some(1), &[2, 3]), Resolution::Current(1));
        assert_eq!(resolve(None, &[2]), Resolution::Redirect(2));
        assert_eq!(resolve(None, &[2, 2]), Resolution::Redirect(2), "one holder, two history rows");
        assert_eq!(resolve(None, &[2, 3]), Resolution::NotFound, "never a guess");
        assert_eq!(resolve::<u8>(None, &[]), Resolution::NotFound);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd /workspace/backend && cargo test -p fau-domain register::slug`
Expected: compile errors, because `slugify`, `municipality_slug`, `first_free_slug` and `resolve`
are not defined.

- [ ] **Step 3: Implement**

Add to `slug.rs`, above the tests:

```rust
/// Section 6, steps 2-8, for a school's display name or a municipality's Norwegian name.
pub fn slugify(name: &str) -> Result<String, SlugError> {
    let slug = cap(collapse(&latin_fold(name, Letters::Transliterate), '-'));
    if slug.is_empty() {
        Err(SlugError::Empty)
    } else {
        Ok(slug)
    }
}

/// Cuts at the last hyphen within the cap, or hard at the cap if there is none. The
/// input is ASCII, so byte indices are character indices.
fn cap(slug: String) -> String {
    if slug.len() <= MAX_SLUG_LEN {
        return slug;
    }
    let head = &slug[..MAX_SLUG_LEN];
    if slug.as_bytes()[MAX_SLUG_LEN] == b'-' {
        return head.to_owned();
    }
    match head.rfind('-') {
        Some(i) => head[..i].to_owned(),
        None => head.to_owned(),
    }
}

/// `<kommunenr>-<navn>` (ADR-002 section 3), from the Norwegian name (D3).
pub fn municipality_slug(number: &str, norwegian_name: &str) -> Result<String, SlugError> {
    if number.len() != 4 || !number.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SlugError::InvalidMunicipalityNumber);
    }
    Ok(format!("{number}-{}", slugify(norwegian_name)?))
}

/// Section 6's collision rule within one municipality: the base slug, then with the
/// post town appended, then numbered from 2. `is_taken` must count a slug held in
/// history by a different, non-closed school as taken.
pub fn first_free_slug(base: &str, post_town: Option<&str>, is_taken: impl Fn(&str) -> bool) -> String {
    if !is_taken(base) {
        return base.to_owned();
    }
    let stem = match post_town.and_then(|t| slugify(t).ok()) {
        Some(town) => {
            let with_town = format!("{base}-{town}");
            if !is_taken(&with_town) {
                return with_town;
            }
            with_town
        }
        None => base.to_owned(),
    };
    (2u32..)
        .map(|n| format!("{stem}-{n}"))
        .find(|candidate| !is_taken(candidate))
        .expect("some numbered slug is free")
}

/// ADR-002's resolution: the current holder; else a single former holder; else none.
pub fn resolve<Id: Copy + PartialEq>(current: Option<Id>, historical: &[Id]) -> Resolution<Id> {
    if let Some(id) = current {
        return Resolution::Current(id);
    }
    match historical.split_first() {
        Some((first, rest)) if rest.iter().all(|h| h == first) => Resolution::Redirect(*first),
        _ => Resolution::NotFound,
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd /workspace/backend && cargo test -p fau-domain` (and the domain's `dependency_boundary` test).
Expected: PASS. Then `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings`.

- [ ] **Step 5: Commit**

```bash
git add backend/Cargo.toml backend/Cargo.lock backend/crates/domain
git commit -m "Add slug minting and resolution to the domain (#3441)"
```

---

### Task 3: Domain: search folding

**Files:**
- Create: `backend/crates/domain/src/register/search.rs`
- Modify: `backend/crates/domain/src/register/mod.rs` (`pub mod search;`)

**Interfaces:**
- Consumes: `text::{latin_fold, collapse, Letters}` from Task 2.
- Produces: `search_text<'a>(names: impl IntoIterator<Item = &'a str>) -> String` and
  `search_query(q: &str) -> String`. The persistence plan stores `search_text` and matches
  `(' ' || search_text) like '% ' || search_query(q) || '%'`.

- [ ] **Step 1: Write the failing tests**

`search.rs`:

```rust
//! Search normalisation (section 7). Done in Rust at write time and at query time,
//! never in the database, so matching does not depend on its collation or LC_CTYPE.

use unicode_normalization::UnicodeNormalization;

use super::text::{collapse, latin_fold, Letters};

#[cfg(test)]
mod tests {
    use super::*;

    /// The SQL the persistence plan runs, in Rust: a word-prefix match.
    fn finds(names: &[&str], q: &str) -> bool {
        format!(" {}", search_text(names.iter().copied())).contains(&format!(" {}", search_query(q)))
    }

    #[test]
    fn tromsoe_every_way_people_type_it() {
        for q in ["tromso", "tromsoe", "Tromsø", "TROMSØ", "troms"] {
            assert!(finds(&["Tromsø"], q), "{q}");
        }
    }

    #[test]
    fn every_official_name_is_searchable() {
        let names = ["Karasjok", "Kárášjohka"];
        assert!(finds(&names, "karasjok"));
        assert!(finds(&names, "kárášjohka"));
        assert!(finds(&names, "karasjohka"));
    }

    #[test]
    fn prefixes_of_any_word() {
        assert!(finds(&["Ålesund"], "aal"));
        assert!(finds(&["Ålesund"], "ale"));
        assert!(finds(&["Nordre Follo"], "follo"));
        assert!(!finds(&["Nordre Follo"], "ollo"), "word prefixes only, not infixes");
    }

    #[test]
    fn baerum_three_ways() {
        for q in ["bærum", "baerum", "barum", "Bærum"] {
            assert!(finds(&["Bærum"], q), "{q}");
        }
    }

    #[test]
    fn forms_are_deduplicated_and_single_spaced() {
        assert_eq!(search_text(["Oslo"]), "oslo");
        assert_eq!(search_text(["Tromsø"]), "tromsø tromsoe tromso");
        let t = search_text(["Nordre  Follo", "Nordre-Follo"]);
        assert!(!t.contains("  "));
        assert_eq!(t, "nordre follo");
    }

    #[test]
    fn a_query_is_lossy_and_single_spaced() {
        assert_eq!(search_query("  Nordre--Follo "), "nordre follo");
        assert_eq!(search_query("Åsane"), "asane");
        assert_eq!(search_query("?!"), "");
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd /workspace/backend && cargo test -p fau-domain register::search`
Expected: compile errors, because `search_text` and `search_query` are not defined.

- [ ] **Step 3: Implement**

Above the tests in `search.rs`:

```rust
/// The space-joined matching forms of every name, for a `search_text` column: each
/// name folded (NFC, lowercase, letters kept), transliterated (`tromsoe`) and lossy
/// (`tromso`). Duplicate forms are kept once, in first-seen order.
pub fn search_text<'a>(names: impl IntoIterator<Item = &'a str>) -> String {
    let mut forms: Vec<String> = Vec::new();
    for name in names {
        let candidates = [
            folded(name),
            collapse(&latin_fold(name, Letters::Transliterate), ' '),
            collapse(&latin_fold(name, Letters::Lossy), ' '),
        ];
        for form in candidates {
            if !form.is_empty() && !forms.contains(&form) {
                forms.push(form);
            }
        }
    }
    forms.join(" ")
}

/// The query form: lossy, because that is what people type, and it matches all three
/// stored forms' lossy member.
pub fn search_query(q: &str) -> String {
    collapse(&latin_fold(q, Letters::Lossy), ' ')
}

fn folded(name: &str) -> String {
    let lower = name.nfc().collect::<String>().to_lowercase();
    lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cd /workspace/backend && cargo test -p fau-domain`, then fmt and clippy.
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/domain
git commit -m "Add register search folding to the domain (#3441)"
```

---

### Task 4: Domain: the scope filter

**Files:**
- Create: `backend/crates/domain/src/register/scope.rs`
- Modify: `backend/crates/domain/src/register/mod.rs` (`pub mod scope;`)

**Interfaces:**
- Produces: `NsrScopeFacts { is_school: bool, is_active: bool, is_primary_school: bool, municipality_number: String, category_ids: Vec<String>, primary_nace: Option<String> }`;
  `classify(&NsrScopeFacts) -> ScopeDecision`; `ScopeDecision::{InScope, OutOfScope(OutOfScopeReason)}`;
  `OutOfScopeReason::{NotASchool, Inactive, NotPrimarySchool, Abroad, AdultEducation, UpperSecondary}` with `code(self) -> &'static str`;
  `effective_in_scope(classified_in_scope: bool, operator_override: Option<bool>) -> bool`.
  The sync plan fills `NsrScopeFacts` from NSR's `ErSkole`, `ErAktiv`, `ErGrunnskole`,
  `Kommune.Kommunenummer`, `Skolekategorier[].Id` and the `Naeringskoder` entry with `Prioritet == 1`.

- [ ] **Step 1: Write the failing tests**

```rust
//! Which NSR units can have an FAU (section 2.3, decision D2).

#[cfg(test)]
mod tests {
    use super::*;

    /// Hosle skole, 974552124, as NSR returned it on 24 September 2026.
    fn hosle() -> NsrScopeFacts {
        NsrScopeFacts {
            is_school: true,
            is_active: true,
            is_primary_school: true,
            municipality_number: "3201".into(),
            category_ids: vec!["1".into(), "3".into(), "5".into(), "32".into()],
            primary_nace: Some("85.201".into()),
        }
    }

    #[test]
    fn an_ordinary_grunnskole_is_in_scope() {
        assert_eq!(classify(&hosle()), ScopeDecision::InScope);
    }

    #[test]
    fn combined_and_special_schools_are_in() {
        // A combined school's secondary NACE is 85.310; only the primary code counts.
        let combined = hosle();
        assert_eq!(classify(&combined), ScopeDecision::InScope);
        let special = NsrScopeFacts { primary_nace: Some("85.202".into()), ..hosle() };
        assert_eq!(classify(&special), ScopeDecision::InScope);
    }

    #[test]
    fn exclusions_carry_their_reason() {
        use OutOfScopeReason::*;
        let cases = [
            (NsrScopeFacts { is_school: false, ..hosle() }, NotASchool),
            (NsrScopeFacts { is_active: false, ..hosle() }, Inactive),
            (NsrScopeFacts { is_primary_school: false, ..hosle() }, NotPrimarySchool),
            (NsrScopeFacts { municipality_number: "2599".into(), ..hosle() }, Abroad),
            (NsrScopeFacts { category_ids: vec!["10".into()], ..hosle() }, AdultEducation),
            (NsrScopeFacts { category_ids: vec!["25".into()], ..hosle() }, AdultEducation),
            (NsrScopeFacts { primary_nace: Some("85.593".into()), ..hosle() }, AdultEducation),
            (NsrScopeFacts { primary_nace: Some("85.320".into()), ..hosle() }, UpperSecondary),
        ];
        for (facts, reason) in cases {
            assert_eq!(classify(&facts), ScopeDecision::OutOfScope(reason), "{reason:?}");
        }
    }

    #[test]
    fn reason_codes_are_stable() {
        use OutOfScopeReason::*;
        let codes: Vec<_> = [NotASchool, Inactive, NotPrimarySchool, Abroad, AdultEducation, UpperSecondary]
            .map(OutOfScopeReason::code)
            .into();
        assert_eq!(codes, ["not_a_school", "inactive", "not_grunnskole", "abroad", "adult_education", "upper_secondary"]);
        assert_eq!(ScopeDecision::InScope.code(), "in_scope");
    }

    #[test]
    fn the_operator_override_wins() {
        assert!(effective_in_scope(true, None));
        assert!(!effective_in_scope(true, Some(false)), "Porsgrunn kommune Vikarer, marked out");
        assert!(effective_in_scope(false, Some(true)));
        assert!(!effective_in_scope(false, None));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd /workspace/backend && cargo test -p fau-domain register::scope`
Expected: compile errors, because the types are not defined.

- [ ] **Step 3: Implement**

```rust
/// The NSR fields the filter reads. `primary_nace` is the `Naeringskoder` entry with
/// `Prioritet` 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NsrScopeFacts {
    pub is_school: bool,
    pub is_active: bool,
    pub is_primary_school: bool,
    pub municipality_number: String,
    pub category_ids: Vec<String>,
    pub primary_nace: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutOfScopeReason {
    NotASchool,
    Inactive,
    NotPrimarySchool,
    /// Norwegian schools abroad, pseudo-municipality 2599.
    Abroad,
    /// Category 10 or 25, or primary NACE 85.593.
    AdultEducation,
    /// Primary NACE 85.3xx, e.g. hospital schools such as Viti skole.
    UpperSecondary,
}

impl OutOfScopeReason {
    /// Stored in `register_source_records.scope_reason`.
    pub fn code(self) -> &'static str {
        match self {
            OutOfScopeReason::NotASchool => "not_a_school",
            OutOfScopeReason::Inactive => "inactive",
            OutOfScopeReason::NotPrimarySchool => "not_grunnskole",
            OutOfScopeReason::Abroad => "abroad",
            OutOfScopeReason::AdultEducation => "adult_education",
            OutOfScopeReason::UpperSecondary => "upper_secondary",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeDecision {
    InScope,
    OutOfScope(OutOfScopeReason),
}

impl ScopeDecision {
    pub fn code(self) -> &'static str {
        match self {
            ScopeDecision::InScope => "in_scope",
            ScopeDecision::OutOfScope(reason) => reason.code(),
        }
    }
}

/// Section 2.3's filter, checked in a fixed order so each unit gets one reason.
pub fn classify(facts: &NsrScopeFacts) -> ScopeDecision {
    use OutOfScopeReason::*;
    let out = ScopeDecision::OutOfScope;
    if !facts.is_school {
        return out(NotASchool);
    }
    if !facts.is_active {
        return out(Inactive);
    }
    if !facts.is_primary_school {
        return out(NotPrimarySchool);
    }
    if facts.municipality_number == "2599" {
        return out(Abroad);
    }
    if facts.category_ids.iter().any(|c| c == "10" || c == "25") {
        return out(AdultEducation);
    }
    match facts.primary_nace.as_deref() {
        Some("85.593") => out(AdultEducation),
        Some(code) if code.starts_with("85.3") => out(UpperSecondary),
        _ => ScopeDecision::InScope,
    }
}

/// `coalesce(scope_override, in_scope)`: an operator's decision wins (D2).
pub fn effective_in_scope(classified_in_scope: bool, operator_override: Option<bool>) -> bool {
    operator_override.unwrap_or(classified_in_scope)
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cd /workspace/backend && cargo test -p fau-domain`, then fmt and clippy.
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/domain
git commit -m "Add the register scope filter to the domain (#3441)"
```

---

### Task 5: Domain: Brreg FAU rules

**Files:**
- Create: `backend/crates/domain/src/register/brreg.rs`
- Modify: `backend/crates/domain/src/register/mod.rs` (`pub mod brreg;`)

**Interfaces:**
- Consumes: `search::search_query` from Task 3 (the lossy, space-separated form).
- Produces: `is_fau_name(name: &str) -> bool`; `AddressKey { street: String, postcode: String }` (`Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord`);
  `address_keys<'a>(lines: impl IntoIterator<Item = &'a str>, postcode: Option<&str>) -> Vec<AddressKey>` (sorted, deduplicated);
  `name_core(name: &str) -> String`; `suggest_fau_name(registered: &str, school_display_name: &str) -> String`.
  The matcher plan calls `address_keys` on Brreg `forretningsadresse`/`postadresse` and NSR
  `Beliggenhetsadresse`/`Postadresse`, in memory only, and never persists an `AddressKey`.

- [ ] **Step 1: Write the failing tests**

```rust
//! Brreg FAU entities (sections 2.5 and 4.6, D1). Addresses pass through these
//! functions in memory only; nothing here is ever stored (ruling, 24 September 2026).

use super::search::search_query;

#[cfg(test)]
mod tests {
    use super::*;

    fn key(street: &str, postcode: &str) -> AddressKey {
        AddressKey { street: street.into(), postcode: postcode.into() }
    }

    #[test]
    fn fau_names_are_recognised_on_word_boundaries() {
        for name in ["BESTUM FAU", "FAU FAUSKANGER BARNE OG UNGDOMSKOLE", "FORELDRENES ARBEIDSUTVALG VED HOSLE SKOLE",
                     "FORELDRERÅDETS ARBEIDSUTVALG TORSHOV SKOLE", "FORELDRERÅDET VED ÅSANE SKULE", "ARBEIDSUTVALET VED X"] {
            assert!(is_fau_name(name), "{name}");
        }
        for name in ["FAUSKE IDRETTSLAG", "ASKERIS FAUNA", "SAMFUNNSHUS AS"] {
            assert!(!is_fau_name(name), "{name}");
        }
    }

    #[test]
    fn address_keys_normalise_street_names() {
        assert_eq!(address_keys(["Bispeveien 73"], Some("1362")), vec![key("bispevei 73", "1362")]);
        assert_eq!(address_keys(["Bispevegen 73"], Some("1362")), address_keys(["BISPEVEIEN 73"], Some("1362")));
        assert_eq!(address_keys(["Storgata 61"], Some("1890")), address_keys(["Storgaten 61"], Some("1890")));
        assert_eq!(address_keys(["Holgerslystveien 18 A"], Some("0280")), address_keys(["Holgerslystveien 18a"], Some("0280")));
    }

    #[test]
    fn personal_and_postbox_lines_are_ignored() {
        // Section 2.5: 913 of 2,477 carry such a line, usually a parent's home address.
        assert_eq!(address_keys(["c/o Kari Nordmann", "Abbedissevegen 67"], Some("5314")), vec![key("abbedissevei 67", "5314")]);
        assert!(address_keys(["C/O Ola Nordmann 12"], Some("5314")).is_empty());
        assert!(address_keys(["v/ Ola Nordmann 12"], Some("5314")).is_empty());
        assert!(address_keys(["Postboks 264"], Some("1891")).is_empty());
        assert!(address_keys(["Pb 264"], Some("1891")).is_empty());
        assert!(address_keys(["FAU v/ Bergenhus skole", "c/o Rakkestad kommune", "Postboks 264"], Some("1891")).is_empty());
    }

    #[test]
    fn a_line_needs_a_number_and_a_valid_postcode() {
        assert!(address_keys(["Skoleveien"], Some("1362")).is_empty());
        assert!(address_keys(["Bispeveien 73"], None).is_empty());
        assert!(address_keys(["Bispeveien 73"], Some("136")).is_empty());
    }

    #[test]
    fn keys_are_sorted_and_deduplicated() {
        let keys = address_keys(["Bispeveien 73", "Bispevegen 73", "Aveien 1"], Some("1362"));
        assert_eq!(keys, vec![key("avei 1", "1362"), key("bispevei 73", "1362")]);
    }

    #[test]
    fn name_cores_compare_an_fau_with_its_school() {
        assert_eq!(name_core("BESTUM FAU"), name_core("Bestum skole"));
        assert_eq!(name_core("FORELDRERÅDETS ARBEIDSUTVALG VED HOSLE SKOLE"), "hosle");
        assert_eq!(name_core("FAU FAUSKANGER BARNE OG UNGDOMSKOLE"), name_core("Fauskanger barne- og ungdomsskule"));
        assert_ne!(name_core("Bekkestua skole"), name_core("Hosle skole"));
        assert_eq!(name_core("FAU"), "");
    }

    #[test]
    fn fau_names_are_suggested_in_readable_case() {
        assert_eq!(suggest_fau_name("BESTUM FAU", "Bestum skole"), "Bestum FAU");
        assert_eq!(
            suggest_fau_name("FAU FAUSKANGER BARNE OG UNGDOMSKOLE", "Fauskanger barne- og ungdomsskule"),
            "FAU Fauskanger barne og ungdomskole"
        );
        assert_eq!(
            suggest_fau_name("FORELDRERÅDETS ARBEIDSUTVALG VED HOSLE SKOLE", "Hosle skole"),
            "Foreldrerådets arbeidsutvalg ved Hosle skole"
        );
        assert_eq!(suggest_fau_name("FAU V/ ST. SVITHUN SKOLE", "St. Svithun skole"), "FAU v/ St. Svithun skole");
        assert_eq!(suggest_fau_name("Bestum FAU", "Bestum skole"), "Bestum FAU", "mixed case is kept as registered");
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd /workspace/backend && cargo test -p fau-domain register::brreg`
Expected: compile errors, because the functions are not defined.

- [ ] **Step 3: Implement**

```rust
/// Lossy-folded words that mark a Brreg entity as an FAU, matched as whole words.
const FAU_WORDS: &[&str] = &[
    "fau", "arbeidsutvalg", "arbeidsutval", "arbeidsutvalet", "arbeidsutvalget",
    "foreldrerad", "foreldreradet", "foreldreutval", "foreldreutvalg", "samarbeidsutvalg",
];

/// Words that say what kind of body or school it is, not which one (section 2.5).
const GENERIC_WORDS: &[&str] = &[
    "fau", "foreldrenes", "foreldreradets", "foreldreradet", "foreldrerad", "arbeidsutvalg",
    "arbeidsutval", "arbeidsutvalet", "arbeidsutvalget", "ved", "pa", "for", "i", "og", "v",
    "skole", "skolen", "skoles", "skule", "skulen", "skules", "barneskole", "barneskule",
    "ungdomsskole", "ungdomsskule", "ungdomskole", "barne", "oppvekstsenter",
];

/// Acronyms kept in capitals when a registered name is re-cased.
const ACRONYMS: &[&str] = &["FAU", "SFO", "AU"];

/// Whether a registered name looks like an FAU (the caller also requires form FLI).
pub fn is_fau_name(name: &str) -> bool {
    search_query(name).split(' ').any(|w| FAU_WORDS.contains(&w))
}

/// A normalised street line and its postcode. Compared in memory; never stored.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AddressKey {
    pub street: String,
    pub postcode: String,
}

/// Every usable address line as a key: lines naming a person (`c/o`, `v/`) or a post
/// box are skipped, a line must carry a number, and the postcode must be four digits.
pub fn address_keys<'a>(lines: impl IntoIterator<Item = &'a str>, postcode: Option<&str>) -> Vec<AddressKey> {
    let Some(postcode) = postcode.filter(|p| p.len() == 4 && p.bytes().all(|b| b.is_ascii_digit())) else {
        return Vec::new();
    };
    let mut keys: Vec<AddressKey> = lines
        .into_iter()
        .filter_map(street_line)
        .map(|street| AddressKey { street, postcode: postcode.to_owned() })
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

fn street_line(line: &str) -> Option<String> {
    let words: Vec<String> = search_query(line).split(' ').filter(|w| !w.is_empty()).map(str::to_owned).collect();
    let personal = matches!(words.as_slice(), [c, o, ..] if c == "c" && o == "o") || words.first().is_some_and(|w| w == "v");
    let postbox = words.iter().any(|w| w == "postboks" || w == "pb" || w == "boks");
    if personal || postbox || !words.iter().any(|w| w.bytes().any(|b| b.is_ascii_digit())) {
        return None;
    }
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    for word in words {
        // "18 a" and "18a" are the same house.
        if word.len() == 1 && word.as_bytes()[0].is_ascii_lowercase() {
            if let Some(prev) = out.last_mut() {
                if prev.bytes().all(|b| b.is_ascii_digit()) {
                    prev.push_str(&word);
                    continue;
                }
            }
        }
        out.push(street_word(&word));
    }
    Some(out.join(" "))
}

/// Street-type spellings that differ between registrations of the same address.
fn street_word(word: &str) -> String {
    for (suffix, canonical) in [("veien", "vei"), ("vegen", "vei"), ("veg", "vei"), ("gaten", "gate"), ("gata", "gate")] {
        if let Some(stem) = word.strip_suffix(suffix) {
            return format!("{stem}{canonical}");
        }
    }
    match word {
        "vn" | "v" => "vei".to_owned(),
        "gt" => "gate".to_owned(),
        _ => word.to_owned(),
    }
}

/// The identifying part of an FAU's or a school's name: lossy-folded, with the words
/// that name the kind of body or school removed.
pub fn name_core(name: &str) -> String {
    search_query(name)
        .split(' ')
        .filter(|w| !w.is_empty() && !GENERIC_WORDS.contains(w))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The signup form's FAU-name suggestion from a Brreg name, which Brreg stores in
/// capitals. A name with any lowercase letter is returned as registered.
pub fn suggest_fau_name(registered: &str, school_display_name: &str) -> String {
    let registered = registered.trim();
    if registered.chars().any(char::is_lowercase) {
        return registered.to_owned();
    }
    let school_words: Vec<&str> = school_display_name
        .split(|c: char| c.is_whitespace() || c == '-' || c == '/')
        .filter(|w| !w.is_empty())
        .collect();
    registered
        .split_whitespace()
        .enumerate()
        .map(|(i, word)| recase(word, i == 0, &school_words))
        .collect::<Vec<_>>()
        .join(" ")
}

fn recase(word: &str, first: bool, school_words: &[&str]) -> String {
    let core: &str = word.trim_matches(|c: char| !c.is_alphanumeric());
    if core.is_empty() {
        return word.to_lowercase();
    }
    let start = word.find(core).expect("core is a substring of word");
    let (prefix, suffix) = (&word[..start], &word[start + core.len()..]);
    let core_lower = core.to_lowercase();
    let cased = if ACRONYMS.contains(&core) {
        core.to_owned()
    } else if let Some(school) = school_words
        .iter()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .find(|w| w.to_lowercase() == core_lower)
    {
        school.to_owned()
    } else if first {
        let mut chars = core_lower.chars();
        chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
    } else {
        core_lower
    };
    format!("{}{cased}{}", prefix.to_lowercase(), suffix.to_lowercase())
}
```

Check against the tests. "FAU V/ ST. SVITHUN SKOLE": `V/` has core `V`, is not first, is not an
acronym and is no school word, so it becomes `v/`. `ST.` has core `ST`, which matches the school
word `St`, and its suffix `.` is kept. The result is `FAU v/ St. Svithun skole`.

- [ ] **Step 4: Run to verify it passes**

Run: `cd /workspace/backend && cargo test -p fau-domain`, then fmt and clippy.
Expected: PASS. If a street-normalisation test fails, fix the normaliser, not the test: the tests
are the §2.5 cases.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/domain
git commit -m "Add Brreg FAU recognition, address keys and name suggestions (#3441)"
```

---

## Self-Review

- **Spec coverage.** §4.1–4.4 → Task 1. §4.6 schema → Task 1. §5.4's `register_lookups` → Task 1.
  §6 algorithm, collisions, reserved segments and resolution → Task 2. §7 normalisation → Task 3
  (queries, `pg_trgm` and ICU ordering belong to the picker plan). §2.3/D2 → Task 4. §2.5/§4.6
  rules → Task 5. D9 role → Task 1. §4.2 FK and `not null` → Task 1. Brreg address ruling →
  Task 1's column test and Task 5's doc comment.
- **Later plans, not this one:** source clients and recorded fixtures, `fau register sync|export|review|lookups`,
  the apply logic (renames, renumbers, closures, re-registrations, circuit breaker, advisory lock),
  the Brreg matcher, search queries and ordering, outbox mail for review items and the seed summary,
  and the CronJob manifests (#3424).
- **Types.** `search_query` (Task 3) is the only cross-task function Task 5 uses.
  `text::{latin_fold, collapse, Letters}` is used by Tasks 2 and 3.
