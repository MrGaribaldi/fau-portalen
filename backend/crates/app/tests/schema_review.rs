//! The database privilege split, from the design's section 6.

mod common;
use common::TestDb;
use uuid::Uuid;

/// The Postgres SQLSTATE code, if the error is a database error at all -- pins down
/// *which* rule refused the statement (42501, insufficient privilege) rather than
/// accepting any failure, including a syntax error, as proof of the guard (minor
/// finding 4).
fn sqlstate(err: &sqlx::Error) -> Option<String> {
    err.as_database_error()
        .and_then(|e| e.code())
        .map(|c| c.into_owned())
}

#[tokio::test]
async fn runtime_role_cannot_perform_ddl() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await; // connects as fau_app

    let err = sqlx::query("create table sneaky (id int)")
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("42501"),
        "runtime role must not have DDL rights: {err:?}"
    );
}

#[tokio::test]
async fn runtime_role_can_read_and_write_the_spine() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let n: i64 = sqlx::query_scalar("select count(*) from accounts")
        .fetch_one(&pool)
        .await
        .expect("runtime role must read accounts");
    assert_eq!(n, 0);

    // The name promises both read and write (minor finding 5); the read alone was
    // proven above, so an insert through the same runtime pool is what actually
    // pins "write" -- a name that would otherwise pass on a read-only grant.
    let id = Uuid::now_v7();
    sqlx::query("insert into accounts (id, email) values ($1, $2)")
        .bind(id)
        .bind(format!("{id}@example.test"))
        .execute(&pool)
        .await
        .expect("runtime role must write accounts");
    let n: i64 = sqlx::query_scalar("select count(*) from accounts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn runtime_role_cannot_create_temp_tables() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;

    let err = sqlx::query("create temporary table sneaky_temp (id int)")
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("42501"),
        "runtime role must not have TEMP rights on the database: {err:?}"
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

#[tokio::test]
async fn no_application_table_contains_key_material() {
    // Spec section 5 and ADR-003 decision 5: keys live only in the key service.
    // A wrapped key on a row is a defect, and this is the test that says so.
    let db = TestDb::migrated().await;
    let columns: Vec<(String, String)> = sqlx::query_as(
        "select table_name, column_name from information_schema.columns
         where table_schema = 'public'",
    )
    .fetch_all(&db.admin_pool())
    .await
    .unwrap();

    const FORBIDDEN: &[&str] = &[
        "key",
        "dek",
        "kek",
        "secret",
        "private",
        "passphrase",
        "password",
        "cipher",
        "nonce",
        "wrapped",
    ];
    let mut offenders = Vec::new();
    for (table, column) in &columns {
        let c = column.to_ascii_lowercase();
        for needle in FORBIDDEN {
            // Substring, not equality: key_id, wrapped_dek and encryption_key all fail.
            if c.contains(needle) {
                offenders.push(format!("{table}.{column}"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "possible key material in application tables: {offenders:?}. \
         Keys belong in the key service (ADR-003 decision 5). If a column name is a \
         false positive, rename it -- this test is deliberately blunt."
    );
}

#[tokio::test]
async fn the_providers_subject_appears_only_in_identity_mappings() {
    // ADR-003 decision 3.
    let db = TestDb::migrated().await;
    let tables: Vec<String> = sqlx::query_scalar(
        "select distinct table_name from information_schema.columns
         where table_schema = 'public' and column_name in ('subject', 'sub', 'issuer')",
    )
    .fetch_all(&db.admin_pool())
    .await
    .unwrap();
    assert_eq!(tables, vec!["identity_mappings".to_string()]);
}

/// What [`tenant_fk_pairing_check`] found: every foreign key it judged to be a
/// reference *between* two tenant-scoped tables (both sides carry their own
/// `tenant_id` column), and the subset of those whose `tenant_id` is not paired
/// with the referenced table's `tenant_id` at the same position in the key.
struct FkPairingResult {
    /// Constraint names judged to be tenant-to-tenant and therefore checked.
    checked: Vec<String>,
    /// Constraint names among `checked` that fail the pairing requirement.
    violations: Vec<String>,
}

/// Spec section 5, property 1 -- #3412's central integrity rule. A reference
/// between two tables that each carry their own `tenant_id` must pair that
/// `tenant_id` with the referenced table's `tenant_id` **at the same position in
/// the key**, or a cross-tenant row could still satisfy the foreign key: a swapped
/// composite key such as `foreign key (role_id, tenant_id) references roles
/// (tenant_id, id)` has `tenant_id` present on both sides, but pairs it with
/// `roles.id` (via `role_id`) rather than with `roles.tenant_id`, so it does not
/// actually enforce that the referenced role belongs to the same tenant.
///
/// This is why the check does not use `information_schema.key_column_usage` /
/// `constraint_column_usage`: those two views report the referencing and
/// referenced columns as two separate, independently-ordered lists, with nothing
/// that reliably pairs a referencing column with the specific referenced column it
/// is checked against. `pg_constraint.conkey`/`confkey` are themselves the pairing
/// -- position *i* of `conkey` is checked against position *i* of `confkey` at
/// the database level -- so `unnest(conkey, confkey) with ordinality` walks them
/// in lockstep and `pg_attribute` turns each position's two attnums into the two
/// column names actually paired together.
///
/// `tenants` itself is deliberately excluded from "tenant-scoped": it does not
/// carry a `tenant_id` column (it *is* the tenant), so every table's own
/// `tenant_id -> tenants (id)` reference is correctly not judged tenant-to-tenant
/// by this check.
async fn tenant_fk_pairing_check(pool: &sqlx::PgPool) -> FkPairingResult {
    let tenant_tables: std::collections::HashSet<String> = sqlx::query_scalar(
        "select table_name from information_schema.columns
         where table_schema = 'public' and column_name = 'tenant_id'",
    )
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .collect();

    // One row per (constraint, position): the referencing column and the
    // referenced column paired at that exact position in the key, straight from
    // pg_constraint's own conkey/confkey arrays rather than an information_schema
    // view that loses the pairing.
    let pairs: Vec<(String, String, String, String, String)> = sqlx::query_as(
        "select
           con.conname,
           cl.relname as table_name,
           fcl.relname as ref_table_name,
           ref_att.attname as referencing_column,
           fref_att.attname as referenced_column
         from pg_constraint con
         join pg_class cl on cl.oid = con.conrelid
         join pg_class fcl on fcl.oid = con.confrelid
         cross join lateral unnest(con.conkey, con.confkey)
           with ordinality as u(referencing_attnum, referenced_attnum, ord)
         join pg_attribute ref_att
           on ref_att.attrelid = con.conrelid and ref_att.attnum = u.referencing_attnum
         join pg_attribute fref_att
           on fref_att.attrelid = con.confrelid and fref_att.attnum = u.referenced_attnum
         where con.contype = 'f' and con.connamespace = 'public'::regnamespace",
    )
    .fetch_all(pool)
    .await
    .unwrap();

    // (referencing table, referenced table, [(referencing column, referenced
    // column), ...] -- one pair per position in the key).
    type ConstraintColumns = (String, String, Vec<(String, String)>);
    let mut by_constraint: std::collections::HashMap<String, ConstraintColumns> =
        std::collections::HashMap::new();
    for (conname, table_name, ref_table, referencing_column, referenced_column) in pairs {
        by_constraint
            .entry(conname)
            .or_insert_with(|| (table_name, ref_table, Vec::new()))
            .2
            .push((referencing_column, referenced_column));
    }

    let mut checked = Vec::new();
    let mut violations = Vec::new();
    for (conname, (table_name, ref_table, column_pairs)) in by_constraint {
        if !tenant_tables.contains(&table_name) || !tenant_tables.contains(&ref_table) {
            // Not a reference between two tenant-scoped tables -- e.g. every
            // table's own tenant_id -> tenants(id), or memberships.account_id ->
            // accounts(id), where accounts is deliberately global.
            continue;
        }
        checked.push(conname.clone());

        let paired = column_pairs.iter().any(|(referencing, referenced)| {
            referencing == "tenant_id" && referenced == "tenant_id"
        });
        if !paired {
            violations.push(conname);
        }
    }
    checked.sort();
    violations.sort();

    FkPairingResult {
        checked,
        violations,
    }
}

#[tokio::test]
async fn every_foreign_key_between_tenant_tables_includes_tenant_id() {
    // Generalises property 1 into a standing guard rather than a fact checked once
    // for 0002's own tables: a future migration that reintroduced a single-column,
    // or a swapped-composite, tenant-to-tenant foreign key would fail this test.
    let db = TestDb::migrated().await;
    let result = tenant_fk_pairing_check(&db.admin_pool()).await;

    assert!(
        result.violations.is_empty(),
        "foreign key(s) between tenant-scoped tables do not pair tenant_id with \
         tenant_id at the same position: {:?}",
        result.violations
    );

    // Guard against a vacuous pass: if a filtering bug ever made `checked` empty
    // (or dropped role_assignments' own foreign keys out of it), the assertion
    // above would trivially "pass" having verified nothing. 0002 has exactly three
    // tenant-to-tenant foreign keys, all on role_assignments -- to memberships (via
    // membership_id), to roles (via role_id), and to memberships again (via
    // granted_by) -- so requiring at least three, and requiring these three exact
    // names to be present, catches that failure mode.
    assert!(
        result.checked.len() >= 3,
        "expected at least 3 tenant-to-tenant foreign keys to be checked, found {}: {:?}",
        result.checked.len(),
        result.checked
    );
    let checked: std::collections::HashSet<&str> =
        result.checked.iter().map(String::as_str).collect();
    for expected in [
        "role_assignments_tenant_id_membership_id_fkey",
        "role_assignments_tenant_id_role_id_fkey",
        "role_assignments_tenant_id_granted_by_fkey",
    ] {
        assert!(
            checked.contains(expected),
            "expected {expected} to be among the checked tenant-to-tenant foreign \
             keys, got {:?}",
            result.checked
        );
    }
}

#[tokio::test]
async fn foreign_key_pairing_check_detects_a_swapped_composite_key() {
    // Proves tenant_fk_pairing_check actually verifies position-wise pairing and
    // is not satisfied by tenant_id merely appearing somewhere on each side. Builds
    // two tenant-scoped tables on a fresh, unmigrated database and a deliberately
    // wrong composite foreign key -- columns swapped, so tenant_id lines up with
    // the referenced table's id instead of its tenant_id -- and confirms the same
    // helper the standing test above uses reports it.
    let db = TestDb::fresh().await;
    let pool = db.admin_pool();

    sqlx::query(
        "create table probe_parents (
           tenant_id uuid not null,
           id        uuid not null,
           primary key (tenant_id, id)
         )",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "create table probe_children (
           tenant_id uuid not null,
           id        uuid not null,
           parent_id uuid not null,
           primary key (tenant_id, id),
           -- Deliberately swapped: pairs (parent_id -> tenant_id) and
           -- (tenant_id -> id) instead of (tenant_id -> tenant_id) and
           -- (parent_id -> id). tenant_id is present on both sides, so a check
           -- that only asked \"does tenant_id appear anywhere on each side\" would
           -- wrongly accept this.
           foreign key (parent_id, tenant_id) references probe_parents (tenant_id, id)
         )",
    )
    .execute(&pool)
    .await
    .unwrap();

    let result = tenant_fk_pairing_check(&pool).await;

    assert!(
        result
            .checked
            .iter()
            .any(|c| c.starts_with("probe_children_") && c.ends_with("_fkey")),
        "expected the probe foreign key to be judged tenant-to-tenant and checked, \
         got {:?}",
        result.checked
    );
    assert!(
        !result.violations.is_empty(),
        "expected the swapped composite key to be reported as a violation, but \
         none were found among checked constraints {:?}",
        result.checked
    );
}

#[tokio::test]
async fn membership_foreign_keys_pair_tenant_id() {
    // 0003's own tenant-to-tenant references, named so a filtering bug cannot make the
    // standing guard above pass vacuously for the new tables.
    let db = TestDb::migrated().await;
    let result = tenant_fk_pairing_check(&db.admin_pool()).await;
    assert!(result.violations.is_empty(), "{:?}", result.violations);

    let checked: std::collections::HashSet<&str> =
        result.checked.iter().map(String::as_str).collect();
    for expected in [
        "recovery_contacts_tenant_id_nominated_by_fkey",
        "handover_grants_tenant_id_source_assignment_id_fkey",
        "access_requests_tenant_id_requester_membership_id_fkey",
        "access_requests_tenant_id_replaced_assignment_id_fkey",
        "access_requests_tenant_id_decided_by_fkey",
        "invitations_tenant_id_issued_by_fkey",
        "invitations_tenant_id_handover_grant_id_fkey",
        "invitations_tenant_id_access_request_id_fkey",
        "invitations_tenant_id_accepted_membership_id_fkey",
        "invitation_roles_tenant_id_invitation_id_fkey",
        "invitation_roles_tenant_id_role_id_fkey",
    ] {
        assert!(
            checked.contains(expected),
            "expected {expected} among the checked foreign keys, got {:?}",
            result.checked
        );
    }
}

#[tokio::test]
async fn audit_events_are_append_only_for_the_runtime_role() {
    // Flow spec §9: the runtime role may insert and select audit, never update or
    // delete it. 42501 is insufficient_privilege: the grant, not a trigger or a
    // constraint, is what refuses.
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let id = uuid::Uuid::now_v7();
    sqlx::query(
        "insert into audit_events (id, actor_kind, action, subject_type, subject_id, occurred_at)
         values ($1, 'system', 'tenant.signup_created', 'tenant', $1, now())",
    )
    .bind(id)
    .execute(&pool)
    .await
    .expect("the runtime role appends audit");
    let n: i64 = sqlx::query_scalar("select count(*) from audit_events")
        .fetch_one(&pool)
        .await
        .expect("the runtime role reads audit");
    assert_eq!(n, 1);

    for statement in [
        "update audit_events set action = 'tenant.changed'",
        "delete from audit_events",
        "truncate audit_events",
    ] {
        let err = sqlx::query(statement)
            .execute(&pool)
            .await
            .expect_err(statement);
        assert_eq!(
            err.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("42501"),
            "{statement}: {err:?}"
        );
    }
}

#[tokio::test]
async fn the_runtime_role_cannot_delete_from_the_outbox() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let err = sqlx::query("delete from outbox")
        .execute(&pool)
        .await
        .expect_err("the runtime role deleted outbox rows");
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("42501")
    );
}

#[tokio::test]
async fn no_column_stores_a_raw_token() {
    // Invitation tokens are stored only as SHA-256 hashes (flow spec §5.1). Any column
    // whose name mentions a token must be a `bytea` named `..._hash`.
    let db = TestDb::migrated().await;
    let columns: Vec<(String, String, String)> = sqlx::query_as(
        "select table_name, column_name, data_type from information_schema.columns
         where table_schema = 'public' and column_name like '%token%'",
    )
    .fetch_all(&db.admin_pool())
    .await
    .unwrap();
    assert!(
        !columns.is_empty(),
        "expected invitations.token_hash to exist; the guard would pass vacuously"
    );
    for (table, column, data_type) in &columns {
        assert!(
            column.ends_with("_hash") && data_type == "bytea",
            "{table}.{column} ({data_type}) looks like a stored token"
        );
    }
}
