# Membership Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the register-independent membership foundation for FAU: migration `0003_membership.sql`,
the pure membership rules in `fau-domain`, and one-transaction persistence functions for signup,
activation, invitations, requests, roles with the last-admin safeguard, handover, recovery and the
per-request access check, all tested against real PostgreSQL.

**Architecture:** The rules live in `crates/domain` as pure functions over values (no sqlx, no axum,
enforced by `tests/dependency_boundary.rs`). `crates/persistence` gains a `membership` module in which
every public function is one transaction: lock the tenant row, read a snapshot, call the domain rule,
write the state change with its audit entry and, where the spec says so, its outbox row. Every
function takes a `Moment` (an instant plus its Europe/Oslo date), so rule-deciding time never comes
from the database clock and tests can move time.

**Tech Stack:** Rust 1.98.1, sqlx 0.8 (postgres, runtime queries only), jiff 0.2 (bundled tzdb),
getrandom 0.4, sha2 0.10, serde_json 1, uuid 1 (v7), PostgreSQL 17.5.

**Spec:** `docs/fau-creation-and-membership-flow.md` (#3413) is the binding authority. Supporting:
`docs/tenant-role-history-design.md` (#3412: entities and date rules),
`docs/identity-and-encryption.md` (ADR-003 decisions 6a, 7a, 8, 9, 10),
`docs/app-foundation-design.md` (conventions), `docs/school-register-design.md` (#3441, for what
0003 must stay compatible with). Style and the foundation's own constraints:
`docs/superpowers/plans/2026-09-22-app-foundation.md`.

---

## Decisions this plan makes

The brief left these open, or the spec was silent or ambiguous. Each is cheap to reverse.

| Question | Decision | Reason |
| --- | --- | --- |
| Pending-signup data: columns on `tenants` or a table | Separate `tenant_signups` (pk `tenant_id`, `on delete cascade`) | The registrant address, leader address and chosen end date exist only while pending, two are personal data, and activation deletes the row once the account, membership and invitation replace them. Expiry deletes the tenant and the row goes with it. |
| `invitation_roles`: reference a `roles` row or carry a definition | References `roles (tenant_id, id)` | #3412's `invitation_role` has `role_id`. A role is a position successive people hold, so the leader invitation, a replacement proposal and a handover invitation all offer the previous holder's role, and the capability class can only come from the role row. Issuers may create a new role with the invitation (`RoleChoice::New`); a handover issuer may not (that edits the organisation). |
| `audit_events` keys and foreign keys | `id` primary key, nullable `tenant_id`, **no foreign keys** | Global events (a signup collision) have no tenant. Audit must outlive the rows it describes: an expired pending FAU is deleted, its audit stays. An append-only table cannot take part in a cascade. |
| Column names | `issued_by`, `subject_type`/`subject_id` | `schema_review.rs` forbids any column named `issuer`, `subject` or `sub` outside `identity_mappings`, and any name containing `key`, `secret`, `private` etc. |
| How the invitation mail gets its link | The outbox row carries `invitation_id`, never the token; the raw token is returned to the caller | "Never stored" covers the outbox too: an outbox row lives in backups. Delivery is out of scope; the sender (#3410) will need to mint a link at send time, most simply by rotating the token exactly as `resend_invitation` does. Recorded here so #3410 does not store tokens to get around it. |
| Token generator | `getrandom` rather than `rand` | A token needs raw OS randomness, not a userspace PRNG. `getrandom 0.4` and `sha2 0.10` are already in `Cargo.lock` (via `uuid` and `sqlx`), so only `jiff` adds crates. |
| Dates and timestamps in SQL | Bound as text with `::date` / `::timestamptz` casts; read with `to_char` and epoch microseconds | sqlx 0.8 has no jiff support; adding `chrono` or `time` would give the workspace two date types. |
| The admin end date's meaning | The chosen date is stored as the **exclusive** end, and the default is the first 1 October at least three months away | §6.2's example treats "a role ending 2027-10-01" as the exclusive end, and §3.1.5's default is 1 October; the UI (#3422) converts to and from "last day" per #3412. |
| The 1–24 month range | Both bounds inclusive; months added with truncation to month end | Same arithmetic as the handover boundary, one rule everywhere. |
| "Frozen" | `tenants.frozen_at timestamptz` is added; issuance, requests, activation and acceptance refuse a frozen FAU; reads continue | §5.1 requires acceptance to check "not frozen" and §10 tests it, but no column existed. The freeze flow itself (ADR-003 7a) is out of scope. |
| Which invitations need issuer authority at acceptance | `normal`: issuer admin today. `handover`: the linked grant valid today and still the issuer's. `recovery`: the FAU still has no admin. `activation`: none | §5.1's three authorities map to three modes; the activation invitation "completes the signup form" (decision 11). |
| `issued_by` for activation and recovery | Null | Activation's issuer is the signup, not an admin action; the recovery contact holds no membership. A check constraint ties `issued_by` to the mode. |
| Last-admin safeguard: what "would leave no admin" means | Any day from today through the removed rows' remaining term without admin coverage the removed rows would have provided | "No admin today" alone can never catch §7's "an admin revoking the only other admin": the revoker is valid today. See Task 8. |
| Who may withdraw and re-send | Any admin today: both. Handover issuer: both, while the grant is valid. Normal issuer who is no longer admin: withdraw only | §5.1 "the issuer or any admin can withdraw"; §6.3 lets an outgoing admin re-send their own replacement invitations. |
| Replacement proposals: the successor's address | `invitee_email` column; `requester_email` is the proposer's own address | §5.4: "a replacement proposal counts against its proposer", so the limit counts `requester_email`. |
| Recovery notice recipients | Current members; if none, everyone who held a role in the past 24 months; **always** plus `fau@ewb-solutions.as` | ADR-003 decision 8: EWB is "the notified second party for every recovery on every FAU". |
| Recovery contact outside the no-admin state | Refused (`NotInNoAdminState`); plain member addition by the recovery contact is not built | The brief scopes `recovery_grant_admin` only; §10 tests only the admin grant. |
| Re-inviting a revoked member | The same membership row is reopened | #3412: one membership per account per FAU. The revocation stays in audit. |
| `MINIMUM_CONTRACT_VERSION` | Stays 2; the database reports 3 | `fau serve` calls none of these functions yet, and the constant's own doc says it is raised when the binary needs the schema. #3417 raises it with its first route that calls them. |
| The one-live-FAU-per-school index vs #3441 | 0003 creates `tenants_one_live_per_school` on the bare `school_id` | `docs/school-register-design.md` makes `schools` global and keyed on our UUIDs, so its FK is a plain `references schools (id)`. The register migration must add only the FK, not its own copy of the index (its draft names one `tenants_one_live_fau_per_school`). |
| Warnings (§6.1) and the no-admin flag notification (§6.4.1–2) | Not in this plan | Both are scheduled notifications that need a scheduler; the brief's function list does not include them. They build on `current_member_emails` and `recovery_notice_recipients` here. |

---

## Global Constraints

Every task's requirements implicitly include this section. The foundation plan's Global Constraints
still apply; the ones this plan leans on are repeated with the new ones.

**Language and copy**
- All technical content in English: code, comments, SQL, docs, commit messages. (`CLAUDE.md`.)
- No display text in errors. Every error is a typed enum variant; `Display` is a fixed English phrase
  that never carries an address, a name, a token or free text. The `ErrorCode` mapping comes later with HTTP (#3417).
- Audit stores **action codes**, never rendered sentences, and parameters are ids, codes, dates and
  flags only: "no token, document text or raw email payload" (#3412). Outbox parameters are ids and codes.
- The only stored user-facing string is the role name "Administrator" (spec §3.4.2), which is data.

**Architecture**
- `crates/domain` declares neither `axum` nor `sqlx` (or `tower`, `hyper`, `reqwest`); enforced by `tests/dependency_boundary.rs`.
- `persistence` owns transaction boundaries. "Every state change is one transaction with its audit entry and its outbox message" (spec §2.6).
- "Today" means "the server's calendar date in Europe/Oslo" (spec §2.1). All rule-deciding time comes from a `Moment`, never from SQL `now()`.
- Rights "are re-checked on every request, never cached from login" (spec §2.1).
- The #3414 gate (second factor plus session freshness) is the HTTP layer's (#3417). Persistence functions that need it say so in a `**Gate:**` doc comment and verify only authority in the database.

**Schema** (from 0002, checked by the standing `schema_review.rs` guards)
- Every reference between tenant data uses the composite key `(tenant_id, id)`.
- Explicit grants to `fau_app`; nothing by default. Audit is `select, insert` only.
- No key material in any table; tokens only as SHA-256 hashes (`bytea`, 32 bytes).
- Role periods are half-open `[starts_on, ends_on_exclusive)` with a mandatory end.
- Migrations are immutable once applied.

**Values from the spec** (copied)
- Pending FAU expiry: **7 days**; at most **three** pending FAU-er per address (§3.3).
- Admin end date: default "the first 1 October at least three months after today"; range "from one month to 24 months after today" (§3.1.5).
- Invitations: "valid for **14 days**. The token is single-use and stored only as a hash" (§5.1).
- Handover: "six calendar months from the exclusive end date, truncated to the last valid day of the month, and the boundary is computed and stored explicitly" (§6.2). 2027-08-31 gives 2028-02-29 (#3412).
- Requests: message "plain text, maximum 500 characters"; "one open request per address per FAU; five requests per FAU per day; a replacement proposal counts against its proposer"; lapse "after 30 days" (§5.2, §5.4).
- Recovery: notify "every current member, both when the invitation is created and when it is accepted"; "when no members remain, notices go to recent role-holders and to the second party" (§6.4.4); past-holder window 24 months (§6.4.2).
- Collisions and activations are copied to `fau@ewb-solutions.as` (§3.2, §3.4.5).

**Testing**
- `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test --workspace`.
  Each test gets its own database from the migrated template; the template name is a content hash of
  `migrations/` and `db/roles.sql`, so 0003 rebuilds it automatically.
- Persistence tests connect as `fau_app` (`db.app_pool()`), which proves the grants; `db.admin_pool()` (superuser) is used only to arrange states the runtime cannot (a frozen FAU, a disabled account, sabotage).
- Tests are written before the code; `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings` stay clean after every task.
- `tests/image.rs::dockerfile_pins_base_images_by_digest` reads `/workspace/Dockerfile` and fails when the backend is copied elsewhere; in the real repository it passes.

**Out of scope — do not build**
- HTTP endpoints, Hanko, sessions, the #3414 gate itself (#3417).
- Mail delivery (#3410); only outbox rows are written.
- `schools`, `municipalities` and the `tenants.school_id` foreign key (#3441).
- Units and cohorts (#3418's school setup).
- The UI (#3422), including the 14-day recovery banner (derived later from `recovery.*` audit entries).
- The freeze flow (ADR-003 7a), school-representative nomination screens, lifecycle warnings (§6.1), the no-admin notification sweep (§6.4.1–2), request withdrawal by the requester, row-level security.

---

## File Structure

```text
backend/
  Cargo.toml                          + jiff, getrandom, sha2 in [workspace.dependencies]
  migrations/0003_membership.sql      new: all tables in spec §9 except the register
  crates/domain/
    Cargo.toml                        + jiff
    src/lib.rs                        + pub mod email, membership, time
    src/time.rs                       Moment, oslo_today
    src/email.rs                      Email, VerifiedEmail (redacted Debug)
    src/membership/mod.rs
    src/membership/period.rs          Period: half-open dates
    src/membership/rules.rs           numbers and date arithmetic (handover boundary, admin end, lifetimes)
    src/membership/vocabulary.rs      closed sets with their database codes; RoleName, FauName
    src/membership/access.rs          evaluate_access, tenant_has_admin, removal_leaves_no_admin
    src/membership/acceptance.rs      check_acceptance over a snapshot
    src/membership/requests.rs        request limits, message, lapse, replacement dates
  crates/persistence/
    Cargo.toml                        + getrandom, jiff, serde_json, sha2, uuid
    src/lib.rs                        + pub mod membership
    src/membership/mod.rs             public surface
    src/membership/error.rs           MembershipError
    src/membership/token.rs           InvitationToken, hashing
    src/membership/sql.rs             shared: binding, tenant lock, authority, AdminState, recipients, audit, outbox
    src/membership/signup.rs          create_pending_tenant, activate_tenant, expire_pending_tenants
    src/membership/invitations.rs     issue, resend, withdraw, accept; insert_invitation, resolve_roles
    src/membership/requests.rs        access requests and replacement proposals
    src/membership/roles.rs           grant, revoke assignment, revoke membership
    src/membership/handover.rs        create_handover_grants, recovery_grant_admin
    src/membership/access.rs          effective_access
  crates/app/
    Cargo.toml                        + jiff, sha2 as dev-dependencies
    tests/common/mod.rs               + pub mod membership
    tests/common/membership.rs        fixtures built through the persistence functions
    tests/membership_schema.rs        0003 constraints
    tests/schema_review.rs            + 0003 FK names, audit append-only, outbox no-delete, no raw token column
    tests/migrations.rs, contract_gate.rs, readiness.rs   contract version 3
    tests/signup.rs, invitations.rs, requests.rs, roles.rs, handover_recovery.rs, access.rs
```

`sql.rs` grows task by task (5 → 9) so that each commit has no dead code under `clippy -D warnings`;
every edit to it is shown as an exact replace or append.

---

### Task 1: Migration 0003 and its schema tests

Adds the membership tables in SQL, proven by tests before any Rust depends on them. The standing `schema_review.rs` guards (key material, composite FK pairing, runtime role) run over the new tables automatically; this task adds three guards of its own and names 0003's foreign keys so the pairing guard cannot pass vacuously. The migrated template rebuilds itself because its name is a content hash of `migrations/`. Existing tests that assumed contract version 2 as the newest are updated: `migrations.rs` counts three migrations, and the two tests that simulate a below-minimum contract delete every row from version 2 upward instead of only version 2 (otherwise version 3 would remain and the database would still be served).

**Files:**
- Create: `backend/migrations/0003_membership.sql`
- Create: `backend/crates/app/tests/membership_schema.rs`
- Modify: `backend/crates/app/tests/schema_review.rs`
- Modify: `backend/crates/app/tests/migrations.rs`
- Modify: `backend/crates/app/tests/contract_gate.rs`
- Modify: `backend/crates/app/tests/readiness.rs`

**Interfaces:**
- Consumes: 0002's tables; `common::TestDb` (`migrated()`, `admin_pool()`, `app_pool()`); `tenant_fk_pairing_check` in `schema_review.rs`.
- Produces: tables `tenant_signups`, `audit_events`, `outbox`, `recovery_contacts`, `handover_grants`, `access_requests`, `invitations`, `invitation_roles`; column `tenants.frozen_at`; index `tenants_one_live_per_school`; contract version 3. Constraint names used by later tests: `invitations_token_hash_unique`, `access_requests_one_open_per_address`, `handover_grants_one_per_source` (target of `on conflict (tenant_id, source_assignment_id)`).

- [ ] **Step 1: Write the failing tests**

Create (or replace in full) `backend/crates/app/tests/membership_schema.rs`:

```rust
//! Migration 0003: the membership foundation's constraints, proven in SQL before any
//! Rust depends on them (flow spec §9).

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

async fn tenant(
    pool: &sqlx::PgPool,
    status: &str,
    school: Option<Uuid>,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query("insert into tenants (id, name, status, school_id) values ($1, 'FAU', $2, $3)")
        .bind(id)
        .bind(status)
        .bind(school)
        .execute(pool)
        .await
        .map(|_| id)
}

/// An active tenant with one membership and one role, for tables that reference them.
async fn seeded(pool: &sqlx::PgPool) -> (Uuid, Uuid, Uuid) {
    let t = tenant(pool, "active", None).await.unwrap();
    let (acc, m, r) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    sqlx::query("insert into accounts (id, email) values ($1, $2)")
        .bind(acc)
        .bind(format!("{acc}@example.test"))
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("insert into memberships (tenant_id, id, account_id) values ($1, $2, $3)")
        .bind(t)
        .bind(m)
        .bind(acc)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "insert into roles (tenant_id, id, name, capability_class) values ($1, $2, 'Leder', 'admin')",
    )
    .bind(t)
    .bind(r)
    .execute(pool)
    .await
    .unwrap();
    (t, m, r)
}

async fn invitation(
    pool: &sqlx::PgPool,
    t: Uuid,
    mode: &str,
    issued_by: Option<Uuid>,
    hash: Vec<u8>,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into invitations
           (tenant_id, id, token_hash, mode, recipient_email, issued_by, expires_at, created_at)
         values ($1, $2, $3, $4, 'ny@example.test', $5, now() + interval '14 days', now())",
    )
    .bind(t)
    .bind(id)
    .bind(hash)
    .bind(mode)
    .bind(issued_by)
    .execute(pool)
    .await
    .map(|_| id)
}

#[tokio::test]
async fn one_live_fau_per_school() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let school = Uuid::now_v7();

    tenant(&pool, "pending", Some(school)).await.unwrap();
    for status in ["pending", "active"] {
        let err = tenant(&pool, status, Some(school))
            .await
            .expect_err("a second live FAU for one school was accepted");
        assert_eq!(sqlstate(&err).as_deref(), Some("23505"), "{err:?}");
        assert_eq!(
            constraint_name(&err).as_deref(),
            Some("tenants_one_live_per_school")
        );
    }
    // A closed FAU does not block, and FAU-er without a school yet never collide.
    tenant(&pool, "closed", Some(school)).await.unwrap();
    tenant(&pool, "active", None).await.unwrap();
    tenant(&pool, "active", None).await.unwrap();
}

#[tokio::test]
async fn deleting_a_pending_tenant_takes_its_signup_row() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t = tenant(&pool, "pending", Some(Uuid::now_v7()))
        .await
        .unwrap();
    sqlx::query(
        "insert into tenant_signups
           (tenant_id, registrant_email, leader_email, admin_ends_on_exclusive, expires_at)
         values ($1, 'r@example.test', 'l@example.test', date '2027-10-01', now())",
    )
    .bind(t)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query("delete from tenants where id = $1")
        .bind(t)
        .execute(&pool)
        .await
        .expect("the runtime role deletes an expired pending tenant");
    let left: i64 = sqlx::query_scalar("select count(*) from tenant_signups")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn invitation_token_hashes_are_unique_sha256_digests() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t, m, _) = seeded(&pool).await;

    invitation(&pool, t, "normal", Some(m), vec![7; 32])
        .await
        .unwrap();
    let dup = invitation(&pool, t, "normal", Some(m), vec![7; 32])
        .await
        .expect_err("duplicate token hash accepted");
    assert_eq!(
        constraint_name(&dup).as_deref(),
        Some("invitations_token_hash_unique")
    );
    let short = invitation(&pool, t, "normal", Some(m), vec![1; 16])
        .await
        .expect_err("a 16-byte hash accepted");
    assert_eq!(
        constraint_name(&short).as_deref(),
        Some("invitation_token_hash_is_sha256")
    );
}

#[tokio::test]
async fn invitation_issuer_must_match_its_mode() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t, m, _) = seeded(&pool).await;

    invitation(&pool, t, "activation", None, vec![1; 32])
        .await
        .unwrap();
    let normal_without_issuer = invitation(&pool, t, "normal", None, vec![2; 32])
        .await
        .expect_err("a normal invitation without an issuer was accepted");
    assert_eq!(
        constraint_name(&normal_without_issuer).as_deref(),
        Some("invitation_issuer_matches_mode")
    );
    let activation_with_issuer = invitation(&pool, t, "activation", Some(m), vec![3; 32])
        .await
        .expect_err("an activation invitation with an issuer was accepted");
    assert_eq!(
        constraint_name(&activation_with_issuer).as_deref(),
        Some("invitation_issuer_matches_mode")
    );
    let handover_without_grant = invitation(&pool, t, "handover", Some(m), vec![4; 32])
        .await
        .expect_err("a handover invitation without a grant was accepted");
    assert_eq!(
        constraint_name(&handover_without_grant).as_deref(),
        Some("invitation_handover_has_grant")
    );
}

#[tokio::test]
async fn an_invitation_role_cannot_point_at_another_tenants_role() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t1, m1, r1) = seeded(&pool).await;
    let (_t2, _m2, r2) = seeded(&pool).await;
    let inv = invitation(&pool, t1, "normal", Some(m1), vec![5; 32])
        .await
        .unwrap();

    let insert = |role: Uuid| {
        sqlx::query(
            "insert into invitation_roles (tenant_id, invitation_id, role_id, starts_on, ends_on_exclusive)
             values ($1, $2, $3, date '2026-10-01', date '2027-10-01')",
        )
        .bind(t1)
        .bind(inv)
        .bind(role)
        .execute(&pool)
    };
    insert(r1).await.expect("a same-tenant role is accepted");
    let err = insert(r2)
        .await
        .expect_err("a cross-tenant role was accepted");
    assert_eq!(sqlstate(&err).as_deref(), Some("23503"), "{err:?}");
}

#[tokio::test]
async fn access_request_constraints() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t, _, _) = seeded(&pool).await;

    let insert = |email: &'static str, message: String| {
        sqlx::query(
            "insert into access_requests
               (tenant_id, id, kind, requester_email, invitee_email, message, created_on, created_at)
             values ($1, $2, 'access', $3, $3, $4, current_date, now())",
        )
        .bind(t)
        .bind(Uuid::now_v7())
        .bind(email)
        .bind(message)
        .execute(&pool)
    };
    insert("a@example.test", "ø".repeat(500)).await.unwrap();
    let dup = insert("a@example.test", "igjen".into())
        .await
        .expect_err("a second open request from one address was accepted");
    assert_eq!(
        constraint_name(&dup).as_deref(),
        Some("access_requests_one_open_per_address")
    );
    let long = insert("b@example.test", "ø".repeat(501))
        .await
        .expect_err("a 501-character message was accepted");
    assert_eq!(
        constraint_name(&long).as_deref(),
        Some("access_request_message_is_short")
    );

    let replacement_without_role = sqlx::query(
        "insert into access_requests
           (tenant_id, id, kind, requester_email, invitee_email, created_on, created_at)
         values ($1, $2, 'replacement', 'c@example.test', 'd@example.test', current_date, now())",
    )
    .bind(t)
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await
    .expect_err("a replacement without proposer and role was accepted");
    assert_eq!(
        constraint_name(&replacement_without_role).as_deref(),
        Some("replacement_names_proposer_and_role")
    );
}

#[tokio::test]
async fn one_handover_grant_per_source_assignment() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t, m, r) = seeded(&pool).await;
    let ra = Uuid::now_v7();
    sqlx::query(
        "insert into role_assignments (tenant_id, id, membership_id, role_id, starts_on, ends_on_exclusive)
         values ($1, $2, $3, $4, date '2026-10-01', date '2027-10-01')",
    )
    .bind(t)
    .bind(ra)
    .bind(m)
    .bind(r)
    .execute(&pool)
    .await
    .unwrap();

    let grant = || {
        sqlx::query(
            "insert into handover_grants (tenant_id, id, source_assignment_id, starts_on, ends_on_exclusive)
             values ($1, $2, $3, date '2027-10-01', date '2028-04-01')",
        )
        .bind(t)
        .bind(Uuid::now_v7())
        .bind(ra)
        .execute(&pool)
    };
    grant().await.unwrap();
    let err = grant()
        .await
        .expect_err("a second grant from one assignment was accepted");
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("handover_grants_one_per_source")
    );
}

#[tokio::test]
async fn a_school_rep_holds_the_seat_only_once_confirmed() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t, _, _) = seeded(&pool).await;

    let err = sqlx::query(
        "insert into recovery_contacts (tenant_id, holder, nomination_status, nominee_email)
         values ($1, 'school_rep', 'nominated', 'rektor@skole.example.test')",
    )
    .bind(t)
    .execute(&pool)
    .await
    .expect_err("an unconfirmed nominee took the seat");
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("recovery_school_rep_is_confirmed")
    );

    let unrecorded = sqlx::query(
        "insert into recovery_contacts (tenant_id, holder, nomination_status, nominee_email)
         values ($1, 'school_rep', 'confirmed', 'rektor@skole.example.test')",
    )
    .bind(t)
    .execute(&pool)
    .await
    .expect_err("a confirmation without its verification record was accepted");
    assert_eq!(
        constraint_name(&unrecorded).as_deref(),
        Some("recovery_confirmation_is_recorded")
    );

    sqlx::query("insert into recovery_contacts (tenant_id, holder) values ($1, 'ewb')")
        .bind(t)
        .execute(&pool)
        .await
        .expect("EWB in the seat with no nomination is the activation default");
}

#[tokio::test]
async fn audit_and_outbox_codes_are_constrained() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;

    let bad_action = sqlx::query(
        "insert into audit_events (id, actor_kind, action, subject_type, subject_id, occurred_at)
         values ($1, 'system', 'Kari ble lagt til', 'tenant', $1, now())",
    )
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await
    .expect_err("a sentence was accepted as an action code");
    assert_eq!(
        constraint_name(&bad_action).as_deref(),
        Some("audit_action_is_a_code")
    );

    let big_params = sqlx::query(
        "insert into outbox (id, template, recipient_email, params, created_at)
         values ($1, 'invitation.issued', 'x@example.test', jsonb_build_object('pad', repeat('x', 3000)), now())",
    )
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await
    .expect_err("an oversized parameter object was accepted");
    assert_eq!(
        constraint_name(&big_params).as_deref(),
        Some("outbox_params_are_small")
    );
}

#[tokio::test]
async fn the_contract_version_is_three() {
    let db = TestDb::migrated().await;
    let version: i32 = sqlx::query_scalar("select max(version) from schema_contract")
        .fetch_one(&db.app_pool().await)
        .await
        .unwrap();
    assert_eq!(version, 3);
}
```

Append to the end of `backend/crates/app/tests/schema_review.rs`:

```rust

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
```

In `backend/crates/app/tests/migrations.rs`, replace:

```rust
    // 0001 and 0002 are the migrations that exist today.
    assert_eq!(version, 2, "contract version after all migrations");
```

with:

```rust
    // 0001, 0002 and 0003 are the migrations that exist today.
    assert_eq!(version, 3, "contract version after all migrations");
```

In `backend/crates/app/tests/migrations.rs`, replace:

```rust
/// 0001 -- is brought forward by applying just 0002, leaving 0001's record alone.
```

with:

```rust
/// 0001 -- is brought forward by applying 0002 and 0003, leaving 0001's record alone.
```

In `backend/crates/app/tests/migrations.rs`, replace:

```rust
    assert_eq!(version, 2);
```

with:

```rust
    assert_eq!(version, 3);
```

In `backend/crates/app/tests/migrations.rs`, replace (every occurrence):

```rust
    assert_eq!(rows, 2);
```

with:

```rust
    assert_eq!(rows, 3);
```

In `backend/crates/app/tests/migrations.rs`, replace:

```rust
    // 0001 and 0002 are the migrations that exist today.
```

with:

```rust
    // 0001, 0002 and 0003 are the migrations that exist today.
```

In `backend/crates/app/tests/contract_gate.rs`, replace:

```rust
"delete from schema_contract where version = 2"
```

with:

```rust
"delete from schema_contract where version >= 2"
```

In `backend/crates/app/tests/contract_gate.rs`, replace:

```rust
    // unreachable-database warning also carries: version 1 (0002's row deleted,
    // 0001's remains) is below this binary's minimum of 2.
```

with:

```rust
    // unreachable-database warning also carries: version 1 (every row from 0002 on
    // deleted, 0001's remains) is below this binary's minimum of 2.
```

In `backend/crates/app/tests/readiness.rs`, replace:

```rust
"delete from schema_contract where version = 2"
```

with:

```rust
"delete from schema_contract where version >= 2"
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test -p fau-app --test membership_schema --test schema_review --test migrations --test contract_gate --test readiness`
Expected: FAIL: `relation "tenant_signups" does not exist` and similar in `membership_schema` and the new `schema_review` tests; `migrations` expects version 3 and finds 2.

- [ ] **Step 3: Implement**

Create (or replace in full) `backend/migrations/0003_membership.sql`:

```sql
-- 0003: the membership foundation. Implements the data model in section 9 of
-- docs/fau-creation-and-membership-flow.md (#3413), on the rules 0002 established:
--   * every reference between tenant data uses the composite key (tenant_id, id);
--   * explicit grants to the runtime role, nothing by default;
--   * no key material of any kind -- invitation tokens are stored as SHA-256 hashes;
--   * half-open date ranges [starts_on, ends_on_exclusive) with a mandatory end.
--
-- The schools register and tenants.school_id's foreign key are #3441's and are
-- deliberately absent. docs/school-register-design.md makes schools a global table keyed
-- on our own UUIDs, so the FK will be a plain `references schools (id)`, not the
-- tenant-scoped composite form 0002's comment anticipated. The register migration adds
-- only that FK: the one-live-FAU-per-school index below already exists and must not be
-- created a second time under the register design's name.

-- 1. Tenants: one live FAU per school, and a freeze marker.

-- A pending FAU counts (spec 3.2); a closed one does not block a new one.
create unique index tenants_one_live_per_school on tenants (school_id)
  where status in ('pending', 'active') and school_id is not null;

-- ADR-003 decision 7a freezes an FAU on a delete request. The freeze flow itself is not
-- built here; the column exists so invitation issuance and acceptance can refuse a
-- frozen FAU, as spec 5.1 requires.
alter table tenants add column frozen_at timestamptz;

-- 2. Pending-signup data (spec 3.1, 3.3). A separate table rather than columns on
-- tenants: these values exist only while the FAU is pending, two of them are personal
-- data, and activation deletes the row, so nothing about the registrant or the leader
-- lingers on the tenant once the account, membership and invitation that replace them
-- exist. Expiry deletes the tenant and the row goes with it.
create table tenant_signups (
  tenant_id               uuid        primary key references tenants (id) on delete cascade,
  registrant_email        text        not null,
  leader_email            text        not null,
  -- The admin end date the registrant chose, as an exclusive end.
  admin_ends_on_exclusive date        not null,
  expires_at              timestamptz not null,
  created_at              timestamptz not null default now()
);
create index tenant_signups_registrant_email_idx on tenant_signups (registrant_email);
create index tenant_signups_expires_at_idx on tenant_signups (expires_at);

-- 3. Audit (spec 9, decision 14; #3412's audit_event). Append-only: the runtime role
-- is granted insert and select, never update or delete. No foreign keys, on purpose:
-- audit outlives the rows it describes (an expired pending FAU is deleted, its audit
-- stays), and an append-only table cannot take part in a cascade.
create table audit_events (
  id                  uuid        primary key,
  -- Null for a global event, such as a signup collision on a school.
  tenant_id           uuid,
  actor_kind          text        not null check (actor_kind in (
                        'member', 'system', 'registrant', 'requester',
                        'recovery_ewb', 'recovery_school_rep')),
  actor_account_id    uuid,
  actor_membership_id uuid,
  -- An action code such as 'invitation.accepted', never a rendered sentence (#3439).
  action              text        not null
    constraint audit_action_is_a_code check (action ~ '^[a-z][a-z_]*(\.[a-z][a-z_]*)+$'),
  subject_type        text        not null
    constraint audit_subject_type_is_a_code check (subject_type ~ '^[a-z][a-z_]*$'),
  subject_id          uuid        not null,
  occurred_at         timestamptz not null,
  -- Bounded parameters: ids, codes, dates, flags. Never personal free text, never a
  -- token (#3412).
  params              jsonb       not null default '{}'::jsonb
    constraint audit_params_are_small
      check (jsonb_typeof(params) = 'object' and octet_length(params::text) <= 2048)
);
create index audit_events_tenant_idx on audit_events (tenant_id, occurred_at);

-- 4. Outbox (spec 2.6, 9). Written in the same transaction as the state change it
-- announces; a sender (#3410) delivers it later. Never carries a token.
create table outbox (
  id              uuid        primary key,
  template        text        not null
    constraint outbox_template_is_a_code check (template ~ '^[a-z][a-z_]*(\.[a-z][a-z_]*)+$'),
  recipient_email text        not null,
  params          jsonb       not null default '{}'::jsonb
    constraint outbox_params_are_small
      check (jsonb_typeof(params) = 'object' and octet_length(params::text) <= 2048),
  created_at      timestamptz not null,
  sent_at         timestamptz,
  attempts        integer     not null default 0 check (attempts >= 0),
  -- A fixed classification of the last delivery failure, never the provider's message.
  last_error_kind text
);
create index outbox_unsent_idx on outbox (created_at) where sent_at is null;

-- 5. The recovery-contact seat (spec 6.5, ADR-003 decisions 8 and 9). One row per
-- tenant, written at activation with EWB in the seat.
create table recovery_contacts (
  tenant_id          uuid        primary key references tenants (id),
  holder             text        not null check (holder in ('ewb', 'school_rep')),
  nomination_status  text        not null default 'none'
                       check (nomination_status in ('none', 'nominated', 'confirmed')),
  nominee_email      text,
  nominated_by       uuid,
  nominated_at       timestamptz,
  -- ADR-003 decision 9, step 2: the nominee verified an address on the school's or
  -- municipality's domain.
  domain_verified_at timestamptz,
  -- Step 3: the recorded manual title check -- who checked, when, and how.
  title_checked_by   text,
  title_checked_at   timestamptz,
  title_check_method text        check (title_check_method in ('staff_listing', 'telephone')),
  updated_at         timestamptz not null default now(),
  foreign key (tenant_id, nominated_by) references memberships (tenant_id, id),
  constraint recovery_nominee_matches_status
    check ((nomination_status = 'none') = (nominee_email is null)),
  constraint recovery_confirmation_is_recorded
    check ((nomination_status = 'confirmed') = (domain_verified_at is not null
                                                and title_checked_by is not null
                                                and title_checked_at is not null
                                                and title_check_method is not null)),
  -- EWB keeps the seat until the nominee is confirmed.
  constraint recovery_school_rep_is_confirmed
    check ((holder = 'school_rep') = (nomination_status = 'confirmed'))
);

-- 6. Handover grants (#3412, spec 6.2). The boundary is computed in Rust by
-- fau_domain::membership::rules::handover_boundary and stored, never recomputed in SQL.
create table handover_grants (
  tenant_id            uuid        not null references tenants (id),
  id                   uuid        not null,
  source_assignment_id uuid        not null,
  starts_on            date        not null,
  ends_on_exclusive    date        not null,
  revoked_at           timestamptz,
  created_at           timestamptz not null default now(),
  primary key (tenant_id, id),
  constraint handover_grants_one_per_source unique (tenant_id, source_assignment_id),
  foreign key (tenant_id, source_assignment_id) references role_assignments (tenant_id, id),
  constraint handover_period_is_non_empty check (starts_on < ends_on_exclusive)
);

-- 7. Access requests and replacement proposals: one model (spec 5.4).
create table access_requests (
  tenant_id                  uuid        not null references tenants (id),
  id                         uuid        not null,
  kind                       text        not null check (kind in ('access', 'replacement')),
  -- Who asked: the verified requester, or the proposing member's own address. The
  -- one-open-request limit counts this address.
  requester_email            text        not null,
  -- Who an approval invites: the requester, or the proposed successor.
  invitee_email              text        not null,
  requester_membership_id    uuid,
  replaced_assignment_id     uuid,
  proposed_starts_on         date,
  proposed_ends_on_exclusive date,
  message                    text
    constraint access_request_message_is_short check (char_length(message) <= 500),
  status                     text        not null default 'pending'
    check (status in ('pending', 'approved', 'declined', 'withdrawn', 'lapsed')),
  -- The Europe/Oslo date of creation, for the per-day limit and the 30-day lapse.
  created_on                 date        not null,
  created_at                 timestamptz not null,
  decided_by                 uuid,
  closed_at                  timestamptz,
  primary key (tenant_id, id),
  foreign key (tenant_id, requester_membership_id) references memberships (tenant_id, id),
  foreign key (tenant_id, replaced_assignment_id)  references role_assignments (tenant_id, id),
  foreign key (tenant_id, decided_by)              references memberships (tenant_id, id),
  constraint replacement_names_proposer_and_role
    check ((kind = 'replacement') = (requester_membership_id is not null
                                     and replaced_assignment_id is not null
                                     and proposed_starts_on is not null
                                     and proposed_ends_on_exclusive is not null)),
  constraint access_request_proposed_period_is_non_empty
    check (proposed_starts_on < proposed_ends_on_exclusive),
  constraint access_request_closure_matches_status
    check ((status = 'pending') = (closed_at is null)),
  constraint access_request_decider_matches_status
    check ((status in ('approved', 'declined')) = (decided_by is not null))
);
-- The one-open-request limit, enforced by the database as well as by the code.
create unique index access_requests_one_open_per_address
  on access_requests (tenant_id, requester_email) where status = 'pending';
create index access_requests_created_on_idx on access_requests (tenant_id, created_on);

-- 8. Invitations (spec 5.1).
create table invitations (
  tenant_id              uuid        not null references tenants (id),
  id                     uuid        not null,
  -- SHA-256 of the token. The token itself is returned once and never stored.
  token_hash             bytea       not null,
  mode                   text        not null
    check (mode in ('normal', 'activation', 'handover', 'recovery')),
  recipient_email        text        not null,
  -- The issuing membership. Null for activation (the signup issues it) and recovery
  -- (the recovery contact holds no membership).
  issued_by              uuid,
  handover_grant_id      uuid,
  access_request_id      uuid,
  -- Which seat issued a recovery invitation.
  recovery_holder        text        check (recovery_holder in ('ewb', 'school_rep')),
  expires_at             timestamptz not null,
  accepted_at            timestamptz,
  accepted_membership_id uuid,
  revoked_at             timestamptz,
  created_at             timestamptz not null,
  primary key (tenant_id, id),
  constraint invitations_token_hash_unique unique (token_hash),
  constraint invitation_token_hash_is_sha256 check (octet_length(token_hash) = 32),
  foreign key (tenant_id, issued_by)              references memberships (tenant_id, id),
  foreign key (tenant_id, handover_grant_id)      references handover_grants (tenant_id, id),
  foreign key (tenant_id, access_request_id)      references access_requests (tenant_id, id),
  foreign key (tenant_id, accepted_membership_id) references memberships (tenant_id, id),
  constraint invitation_issuer_matches_mode
    check ((mode in ('normal', 'handover')) = (issued_by is not null)),
  constraint invitation_handover_has_grant
    check ((mode = 'handover') = (handover_grant_id is not null)),
  constraint invitation_recovery_names_seat
    check ((mode = 'recovery') = (recovery_holder is not null)),
  constraint invitation_acceptance_is_complete
    check ((accepted_at is null) = (accepted_membership_id is null)),
  constraint invitation_is_not_accepted_and_revoked
    check (accepted_at is null or revoked_at is null)
);
create index invitations_recipient_idx on invitations (tenant_id, recipient_email);

-- The roles an invitation offers. Each row references a roles row, as #3412's
-- invitation_role does, rather than carrying a copy of a name and class: a role is a
-- position that successive people hold, so the leader invitation, a replacement
-- proposal and a handover invitation all offer the same role the previous holder had,
-- and the capability class can only come from the role itself.
create table invitation_roles (
  tenant_id         uuid not null references tenants (id),
  invitation_id     uuid not null,
  role_id           uuid not null,
  starts_on         date not null,
  ends_on_exclusive date not null,
  primary key (tenant_id, invitation_id, role_id),
  foreign key (tenant_id, invitation_id) references invitations (tenant_id, id),
  foreign key (tenant_id, role_id)       references roles (tenant_id, id),
  constraint invitation_role_period_is_non_empty check (starts_on < ends_on_exclusive)
);

-- 9. Grants. Audit is insert and select only; the outbox is never deleted from by the
-- runtime (purging sent rows is #3410's decision); tenant_signups is deleted on
-- activation, and cascades from a deleted pending tenant.
grant select, insert on audit_events to fau_app;
grant select, insert, update on outbox to fau_app;
grant select, insert, update, delete on tenant_signups to fau_app;
grant select, insert, update on
  recovery_contacts, handover_grants, access_requests, invitations
  to fau_app;
grant select, insert on invitation_roles to fau_app;

insert into schema_contract (version) values (3);
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test -p fau-app --test membership_schema --test schema_review --test migrations --test contract_gate --test readiness`
Expected: PASS, and `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings` are clean.

- [ ] **Step 5: Commit** (the controller commits; executors do not)

```bash
git add backend/migrations/0003_membership.sql backend/crates/app/tests/membership_schema.rs backend/crates/app/tests/schema_review.rs backend/crates/app/tests/migrations.rs backend/crates/app/tests/contract_gate.rs backend/crates/app/tests/readiness.rs
git commit -m "Add migration 0003: the membership foundation tables (#3413)"
```

---

### Task 2: Domain: jiff, Moment, email, periods and date rules

Brings `jiff` back (removed from the foundation as unused) with the tz database bundled into the binary, so "today in Europe/Oslo" never depends on the runtime image's tzdata. Adds the `Moment` every rule takes, the email types with redacted `Debug`, half-open `Period`, and the spec's numbers and date arithmetic: the handover boundary with month-end truncation, the admin end-date default and range, and the 7- and 14-day lifetimes. The tests are inline `#[cfg(test)]` modules in each new file, as in the existing domain crate; Step 1 writes the files with their tests, Step 3 adds the module wiring that makes them compile. The domain manifest still declares no HTTP or SQL crate, which `tests/dependency_boundary.rs` keeps checking.

**Files:**
- Modify: `backend/Cargo.toml`
- Modify: `backend/crates/domain/Cargo.toml`
- Create: `backend/crates/domain/src/time.rs`
- Create: `backend/crates/domain/src/email.rs`
- Create: `backend/crates/domain/src/membership/period.rs`
- Create: `backend/crates/domain/src/membership/rules.rs`
- Create: `backend/crates/domain/src/membership/mod.rs`
- Replace in full: `backend/crates/domain/src/lib.rs`

**Interfaces:**
- Produces (`fau_domain`):
  - `time::{Moment, oslo_today, OSLO}`: `Moment::at(Timestamp) -> Moment`, `.now() -> Timestamp`, `.today() -> Date`.
  - `email::{Email, EmailError, VerifiedEmail}`: `Email::parse(&str) -> Result<Email, EmailError>`, `.as_str()`; `VerifiedEmail::from_provider(Email)`, `.email() -> &Email`.
  - `membership::period::{Period, EmptyPeriod}`: `Period::new(Date, Date) -> Result<Period, EmptyPeriod>`, `.starts_on()`, `.ends_on_exclusive()`, `.contains(Date)`, `.has_ended_by(Date)`.
  - `membership::rules`: `handover_boundary(Date) -> Date`, `handover_period(Date) -> Option<Period>`, `default_admin_end(Date) -> Date`, `validate_admin_end(Date, Date) -> Result<(), AdminEndError>` (`AdminEndError::{TooSoon, TooLate}`), `pending_signup_expiry(Timestamp)`, `invitation_expiry(Timestamp)`, `recent_holder_window_start(Date)`, and constants `MAX_PENDING_SIGNUPS_PER_ADDRESS`, `BOOTSTRAP_ADMIN_ROLE_NAME`, `EWB_OVERSIGHT_ADDRESS`.

- [ ] **Step 1: Write the failing tests**

Create (or replace in full) `backend/crates/domain/src/time.rs`:

```rust
//! "Today" is the server's calendar date in Europe/Oslo (flow spec §2.1, #3412). Every
//! rule that depends on the date takes a [`Moment`], so one operation sees one instant and
//! one date, and a test can pin both.

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::Timestamp;

/// The IANA zone that decides "today". Bundled into the binary (jiff's
/// `tzdb-bundle-always`), so the answer never depends on the runtime image's tzdata.
pub const OSLO: &str = "Europe/Oslo";

/// One instant and the Europe/Oslo calendar date it falls on. Constructed only through
/// [`Moment::at`], so the two can never disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Moment {
    now: Timestamp,
    today: Date,
}

impl Moment {
    pub fn at(now: Timestamp) -> Self {
        Self {
            now,
            today: oslo_today(now),
        }
    }

    pub fn now(&self) -> Timestamp {
        self.now
    }

    pub fn today(&self) -> Date {
        self.today
    }
}

/// The Europe/Oslo calendar date of `now`.
pub fn oslo_today(now: Timestamp) -> Date {
    // Cannot fail: the zone is compiled into the binary by `tzdb-bundle-always`, and the
    // unit tests below prove the lookup works.
    let tz = TimeZone::get(OSLO).expect("Europe/Oslo is in the bundled tzdb");
    now.to_zoned(tz).date()
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn winter_midnight_is_utc_plus_one() {
        assert_eq!(oslo_today(ts("2026-12-31T22:59:59Z")), date(2026, 12, 31));
        assert_eq!(oslo_today(ts("2026-12-31T23:00:00Z")), date(2027, 1, 1));
    }

    #[test]
    fn summer_midnight_is_utc_plus_two() {
        assert_eq!(oslo_today(ts("2026-07-31T21:59:59Z")), date(2026, 7, 31));
        assert_eq!(oslo_today(ts("2026-07-31T22:00:00Z")), date(2026, 8, 1));
    }

    #[test]
    fn the_night_daylight_saving_starts() {
        // 29 March 2026: clocks go from 02:00 to 03:00. At 23:30 UTC on the 28th Oslo is
        // still on UTC+1, so it is already 00:30 on the 29th.
        assert_eq!(oslo_today(ts("2026-03-28T22:59:59Z")), date(2026, 3, 28));
        assert_eq!(oslo_today(ts("2026-03-28T23:30:00Z")), date(2026, 3, 29));
    }

    #[test]
    fn a_moment_carries_its_own_date() {
        let m = Moment::at(ts("2026-07-31T22:30:00Z"));
        assert_eq!(m.now(), ts("2026-07-31T22:30:00Z"));
        assert_eq!(m.today(), date(2026, 8, 1));
    }
}
```

Create (or replace in full) `backend/crates/domain/src/email.rs`:

```rust
//! Email addresses. An address is a login address, not an identity: the account ID is
//! the identity (flow spec §1, #3437), so nothing here assumes an address is permanent.
//!
//! `Debug` is redacted on both types. Errors and log lines in this workspace never carry
//! personal data (app-foundation design §10), and a derived `Debug` on any struct that
//! holds an address would otherwise print it.

use std::fmt;

/// A syntactically plausible, normalised address: trimmed and lower-cased. Deliverability
/// is proven only by the provider's passcode, which is what [`VerifiedEmail`] records.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Email(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailError {
    Empty,
    TooLong,
    Malformed,
}

impl Email {
    /// RFC 5321's practical ceiling on a forward path.
    pub const MAX_LEN: usize = 254;

    pub fn parse(raw: &str) -> Result<Self, EmailError> {
        let s = raw.trim();
        if s.is_empty() {
            return Err(EmailError::Empty);
        }
        if s.len() > Self::MAX_LEN {
            return Err(EmailError::TooLong);
        }
        if s.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(EmailError::Malformed);
        }
        let (local, domain) = s.rsplit_once('@').ok_or(EmailError::Malformed)?;
        if local.is_empty()
            || local.contains('@')
            || !domain.contains('.')
            || domain.starts_with('.')
            || domain.ends_with('.')
        {
            return Err(EmailError::Malformed);
        }
        Ok(Self(s.to_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Email {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Email([redacted])")
    }
}

/// An address the identity provider has just verified with a passcode. The type is the
/// proof: persistence functions that require a verified address take this, so an
/// unverified one cannot reach them.
#[derive(Clone, PartialEq, Eq)]
pub struct VerifiedEmail(Email);

impl VerifiedEmail {
    /// Construct only from the session the HTTP layer established with Hanko (#3417),
    /// never from request input.
    pub fn from_provider(email: Email) -> Self {
        Self(email)
    }

    pub fn email(&self) -> &Email {
        &self.0
    }
}

impl fmt::Debug for VerifiedEmail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VerifiedEmail([redacted])")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_trims_and_lowercases() {
        let e = Email::parse("  Kari.Nordmann@Example.TEST ").unwrap();
        assert_eq!(e.as_str(), "kari.nordmann@example.test");
    }

    #[test]
    fn parse_rejects_empty_long_and_malformed() {
        assert_eq!(Email::parse("   "), Err(EmailError::Empty));
        let long = format!("{}@example.test", "a".repeat(250));
        assert_eq!(Email::parse(&long), Err(EmailError::TooLong));
        for bad in [
            "no-at-sign",
            "@example.test",
            "a@b@example.test",
            "a@localhost",
            "a@.example.test",
            "a@example.test.",
            "a b@example.test",
        ] {
            assert_eq!(Email::parse(bad), Err(EmailError::Malformed), "{bad}");
        }
    }

    #[test]
    fn debug_never_prints_the_address() {
        let e = Email::parse("kari@example.test").unwrap();
        assert_eq!(format!("{e:?}"), "Email([redacted])");
        let v = VerifiedEmail::from_provider(e);
        assert_eq!(format!("{v:?}"), "VerifiedEmail([redacted])");
        assert_eq!(v.email().as_str(), "kari@example.test");
    }
}
```

Create (or replace in full) `backend/crates/domain/src/membership/period.rs`:

```rust
//! Half-open date ranges `[starts_on, ends_on_exclusive)` with a mandatory end (#3412;
//! the same rule 0002's `role_period_is_non_empty` check enforces in the database).

use jiff::civil::Date;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Period {
    starts_on: Date,
    ends_on_exclusive: Date,
}

/// `starts_on` is not before `ends_on_exclusive`: the period would contain no day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptyPeriod;

impl Period {
    pub fn new(starts_on: Date, ends_on_exclusive: Date) -> Result<Self, EmptyPeriod> {
        if starts_on < ends_on_exclusive {
            Ok(Self {
                starts_on,
                ends_on_exclusive,
            })
        } else {
            Err(EmptyPeriod)
        }
    }

    pub fn starts_on(&self) -> Date {
        self.starts_on
    }

    pub fn ends_on_exclusive(&self) -> Date {
        self.ends_on_exclusive
    }

    /// Whether `day` falls inside the period. The end date itself is outside it: a role
    /// ending 2027-08-01 grants nothing on 2027-08-01.
    pub fn contains(&self, day: Date) -> bool {
        self.starts_on <= day && day < self.ends_on_exclusive
    }

    /// Whether the period is over by `day`, so offering it would grant nothing ever.
    pub fn has_ended_by(&self, day: Date) -> bool {
        self.ends_on_exclusive <= day
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn contains_the_start_and_excludes_the_end() {
        let p = Period::new(date(2026, 8, 1), date(2027, 8, 1)).unwrap();
        assert!(!p.contains(date(2026, 7, 31)));
        assert!(p.contains(date(2026, 8, 1)));
        assert!(p.contains(date(2027, 7, 31)));
        assert!(!p.contains(date(2027, 8, 1)));
    }

    #[test]
    fn rejects_empty_and_reversed_periods() {
        assert_eq!(
            Period::new(date(2026, 8, 1), date(2026, 8, 1)),
            Err(EmptyPeriod)
        );
        assert_eq!(
            Period::new(date(2027, 8, 1), date(2026, 8, 1)),
            Err(EmptyPeriod)
        );
    }

    #[test]
    fn has_ended_by_the_exclusive_end() {
        let p = Period::new(date(2026, 8, 1), date(2027, 8, 1)).unwrap();
        assert!(!p.has_ended_by(date(2027, 7, 31)));
        assert!(p.has_ended_by(date(2027, 8, 1)));
    }
}
```

Create (or replace in full) `backend/crates/domain/src/membership/rules.rs`:

```rust
//! The flow spec's numbers and date arithmetic, in one place so every caller computes
//! them the same way (#3412: "compute and store the boundary explicitly, so the same rule
//! is used everywhere").

use jiff::civil::Date;
use jiff::{SignedDuration, Timestamp, ToSpan};

use super::period::Period;

/// Calendar months a handover grant runs from the admin role's exclusive end (§6.2).
pub const HANDOVER_MONTHS: i32 = 6;
/// A new admin's end date must lie between these many months from today (§3.1.5).
pub const ADMIN_END_MIN_MONTHS: i32 = 1;
pub const ADMIN_END_MAX_MONTHS: i32 = 24;
/// The default end date is the first 1 October at least this many months away (§3.1.5).
pub const ADMIN_END_DEFAULT_LEAD_MONTHS: i32 = 3;
/// A pending FAU not verified within this many days expires (§3.3).
pub const PENDING_SIGNUP_DAYS: i64 = 7;
/// One address may hold at most this many pending FAU-er at a time (§3.3).
pub const MAX_PENDING_SIGNUPS_PER_ADDRESS: i64 = 3;
/// Every invitation is valid for this many days (§5.1).
pub const INVITATION_DAYS: i64 = 14;
/// Recovery notices reach everyone who held a role in this many past months when no
/// member remains (§6.4, ADR-003 decision 10).
pub const RECENT_HOLDER_MONTHS: i32 = 24;

/// The name the registrant's first role carries (§3.4.2). Stored as data, like any role
/// name an admin types; it is identical in Bokmål and English.
pub const BOOTSTRAP_ADMIN_ROLE_NAME: &str = "Administrator";
/// Erik's oversight address: signup collisions (§3.2), activations (§3.4.5) and every
/// recovery (ADR-003 decisions 8 and 10) are copied here.
pub const EWB_OVERSIGHT_ADDRESS: &str = "fau@ewb-solutions.as";

/// Adds calendar months, truncating to the last valid day of the target month (jiff's
/// behaviour for civil dates), saturating at the end of the representable range.
fn add_months(day: Date, months: i32) -> Date {
    day.checked_add(months.months()).unwrap_or(Date::MAX)
}

/// The exclusive end of the handover window for an admin role ending (exclusively) on
/// `admin_ends_on_exclusive`: six calendar months later, truncated to the last valid day
/// of the month. 2027-08-31 gives 2028-02-29.
pub fn handover_boundary(admin_ends_on_exclusive: Date) -> Date {
    add_months(admin_ends_on_exclusive, HANDOVER_MONTHS)
}

/// The handover window itself, `[admin_ends_on_exclusive, boundary)`. `None` only at the
/// very end of the calendar, where there is no later day to end on.
pub fn handover_period(admin_ends_on_exclusive: Date) -> Option<Period> {
    Period::new(
        admin_ends_on_exclusive,
        handover_boundary(admin_ends_on_exclusive),
    )
    .ok()
}

/// The default admin end date offered at signup: the first 1 October at least three
/// months after `today`. The value is an exclusive end, like every stored period end.
pub fn default_admin_end(today: Date) -> Date {
    let earliest = add_months(today, ADMIN_END_DEFAULT_LEAD_MONTHS);
    let this_year = Date::new(earliest.year(), 10, 1).unwrap_or(Date::MAX);
    if this_year >= earliest {
        this_year
    } else {
        Date::new(earliest.year() + 1, 10, 1).unwrap_or(Date::MAX)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminEndError {
    /// Earlier than one month after today.
    TooSoon,
    /// Later than 24 months after today.
    TooLate,
}

/// Validates a chosen admin end date (exclusive) against the 1–24 month range, both
/// bounds inclusive.
pub fn validate_admin_end(today: Date, ends_on_exclusive: Date) -> Result<(), AdminEndError> {
    if ends_on_exclusive < add_months(today, ADMIN_END_MIN_MONTHS) {
        return Err(AdminEndError::TooSoon);
    }
    if ends_on_exclusive > add_months(today, ADMIN_END_MAX_MONTHS) {
        return Err(AdminEndError::TooLate);
    }
    Ok(())
}

/// When a pending signup created at `now` expires. An exact duration, not calendar days:
/// a daylight-saving change inside the week does not move the deadline by an hour.
pub fn pending_signup_expiry(now: Timestamp) -> Timestamp {
    now.checked_add(SignedDuration::from_hours(24 * PENDING_SIGNUP_DAYS))
        .unwrap_or(Timestamp::MAX)
}

/// When an invitation issued (or re-sent) at `now` expires.
pub fn invitation_expiry(now: Timestamp) -> Timestamp {
    now.checked_add(SignedDuration::from_hours(24 * INVITATION_DAYS))
        .unwrap_or(Timestamp::MAX)
}

/// The first day of the "held a role in the past 24 months" window.
pub fn recent_holder_window_start(today: Date) -> Date {
    today
        .checked_sub(RECENT_HOLDER_MONTHS.months())
        .unwrap_or(Date::MIN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn handover_boundary_is_six_calendar_months_later() {
        assert_eq!(handover_boundary(date(2027, 8, 1)), date(2028, 2, 1));
        assert_eq!(handover_boundary(date(2027, 10, 1)), date(2028, 4, 1));
    }

    #[test]
    fn handover_boundary_truncates_to_month_end_in_a_leap_year() {
        // The worked example in #3412.
        assert_eq!(handover_boundary(date(2027, 8, 31)), date(2028, 2, 29));
    }

    #[test]
    fn handover_boundary_truncates_to_month_end_outside_a_leap_year() {
        assert_eq!(handover_boundary(date(2026, 8, 31)), date(2027, 2, 28));
        assert_eq!(handover_boundary(date(2027, 12, 31)), date(2028, 6, 30));
        assert_eq!(handover_boundary(date(2028, 2, 29)), date(2028, 8, 29));
    }

    #[test]
    fn handover_period_starts_on_the_exclusive_end() {
        let p = handover_period(date(2027, 10, 1)).unwrap();
        assert_eq!(p.starts_on(), date(2027, 10, 1));
        assert_eq!(p.ends_on_exclusive(), date(2028, 4, 1));
        assert!(handover_period(Date::MAX).is_none());
    }

    #[test]
    fn default_admin_end_is_the_first_october_at_least_three_months_away() {
        assert_eq!(default_admin_end(date(2026, 9, 23)), date(2027, 10, 1));
        assert_eq!(default_admin_end(date(2026, 1, 15)), date(2026, 10, 1));
        assert_eq!(default_admin_end(date(2026, 6, 30)), date(2026, 10, 1));
        // Exactly three months away still counts as "at least three months".
        assert_eq!(default_admin_end(date(2026, 7, 1)), date(2026, 10, 1));
        assert_eq!(default_admin_end(date(2026, 7, 2)), date(2027, 10, 1));
        assert_eq!(default_admin_end(date(2026, 10, 1)), date(2027, 10, 1));
    }

    #[test]
    fn the_default_always_passes_validation() {
        let mut day = date(2026, 1, 1);
        while day < date(2029, 1, 1) {
            assert_eq!(
                validate_admin_end(day, default_admin_end(day)),
                Ok(()),
                "{day}"
            );
            day = day.tomorrow().unwrap();
        }
    }

    #[test]
    fn admin_end_range_is_one_to_twenty_four_months_inclusive() {
        let today = date(2026, 9, 23);
        assert_eq!(
            validate_admin_end(today, date(2026, 10, 22)),
            Err(AdminEndError::TooSoon)
        );
        assert_eq!(validate_admin_end(today, date(2026, 10, 23)), Ok(()));
        assert_eq!(validate_admin_end(today, date(2028, 9, 23)), Ok(()));
        assert_eq!(
            validate_admin_end(today, date(2028, 9, 24)),
            Err(AdminEndError::TooLate)
        );
    }

    #[test]
    fn admin_end_range_truncates_at_month_end() {
        // One month after 31 January is 28 February, not 3 March.
        let today = date(2026, 1, 31);
        assert_eq!(
            validate_admin_end(today, date(2026, 2, 27)),
            Err(AdminEndError::TooSoon)
        );
        assert_eq!(validate_admin_end(today, date(2026, 2, 28)), Ok(()));
        // 24 months after 29 February 2028 is 28 February 2030.
        let leap = date(2028, 2, 29);
        assert_eq!(validate_admin_end(leap, date(2030, 2, 28)), Ok(()));
        assert_eq!(
            validate_admin_end(leap, date(2030, 3, 1)),
            Err(AdminEndError::TooLate)
        );
    }

    #[test]
    fn pending_signups_expire_after_seven_days() {
        assert_eq!(
            pending_signup_expiry(ts("2026-09-23T10:00:00Z")),
            ts("2026-09-30T10:00:00Z")
        );
    }

    #[test]
    fn invitations_expire_after_fourteen_days_even_across_a_clock_change() {
        assert_eq!(
            invitation_expiry(ts("2026-09-23T10:00:00Z")),
            ts("2026-10-07T10:00:00Z")
        );
        // Oslo leaves daylight saving on 25 October 2026; the lifetime is exact.
        assert_eq!(
            invitation_expiry(ts("2026-10-20T10:00:00Z")),
            ts("2026-11-03T10:00:00Z")
        );
    }

    #[test]
    fn recent_holder_window_is_twenty_four_months() {
        assert_eq!(
            recent_holder_window_start(date(2026, 9, 23)),
            date(2024, 9, 23)
        );
        assert_eq!(
            recent_holder_window_start(date(2028, 2, 29)),
            date(2026, 2, 28)
        );
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd /workspace/backend && cargo test -p fau-domain`
Expected: the new files are not yet modules, so the crate builds without them and none of their tests run (the count stays at the foundation's). This is the "red" state for inline tests.

- [ ] **Step 3: Implement**

In `backend/Cargo.toml`, replace:

```toml
clap = { version = "4", features = ["derive"] }
```

with:

```toml
clap = { version = "4", features = ["derive"] }
jiff = { version = "0.2", default-features = false, features = ["std", "tzdb-bundle-always"] }
```

In `backend/crates/domain/Cargo.toml`, replace:

```toml
[dependencies]
serde = { workspace = true }
```

with:

```toml
[dependencies]
jiff = { workspace = true }
serde = { workspace = true }
```

Create (or replace in full) `backend/crates/domain/src/membership/mod.rs`:

```rust
//! The membership rules from the #3413 flow spec, as pure functions over values. No I/O:
//! persistence loads a snapshot, calls these, and writes the outcome in one transaction.

pub mod period;
pub mod rules;
```

Create (or replace in full) `backend/crates/domain/src/lib.rs`:

```rust
//! Entities, role periods and capability rules (design section 2). No HTTP, no SQL --
//! `tests/dependency_boundary.rs` enforces that against this crate's own manifest.

pub mod email;
pub mod error_code;
pub mod membership;
pub mod schema_contract;
pub mod time;

pub use error_code::{ErrorCode, ParamValue};
pub use schema_contract::{is_compatible, MINIMUM_CONTRACT_VERSION};
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd /workspace/backend && cargo test -p fau-domain`
Expected: PASS, and `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings` are clean.

- [ ] **Step 5: Commit** (the controller commits; executors do not)

```bash
git add backend/Cargo.toml backend/Cargo.lock backend/crates/domain
git commit -m "Add the membership date rules and email types to the domain (#3413)"
```

---

### Task 3: Domain: vocabulary, access evaluation and the no-admin predicate

The closed sets (capability class, tenant status, invitation mode, request kind and status, recovery holder) with the exact codes the 0002/0003 check constraints accept, validated names, and the access rule: rights are the union of roles valid today, a handover grant is reported separately and never gives capability, and each standing precondition (FAU active, account usable, membership active) removes all access. `tenant_has_admin` is the negation of spec §6.4's no-admin state.

**Files:**
- Create: `backend/crates/domain/src/membership/vocabulary.rs`
- Create: `backend/crates/domain/src/membership/access.rs`
- Replace in full: `backend/crates/domain/src/membership/mod.rs`

**Interfaces:**
- Consumes: `Period` (Task 2).
- Produces (`fau_domain::membership`):
  - `vocabulary::{CapabilityClass, TenantStatus, InvitationMode, RequestKind, RequestStatus, RecoveryHolder}`, each with `ALL`, `.code() -> &'static str`, `from_code(&str) -> Option<Self>`; `RoleName`, `FauName` (`parse(&str) -> Result<_, NameError>`, `.as_str()`).
  - `access::{Capability, AssignmentView, GrantView, Standing, Access, evaluate_access, tenant_has_admin}`: `evaluate_access(Standing, &[AssignmentView], &[GrantView], Date) -> Access`; `tenant_has_admin(&[AssignmentView], &[GrantView], Date) -> bool`; `Access::NONE`.
  - Task 8 adds `access::removal_leaves_no_admin` to this file.

- [ ] **Step 1: Write the failing tests**

Create (or replace in full) `backend/crates/domain/src/membership/vocabulary.rs`:

```rust
//! The closed sets the membership model is built from, with the exact codes the database
//! check constraints in migrations 0002 and 0003 accept. `code()` and `from_code()` are
//! the only translation between the two.

/// The privilege a role grants. The class decides, never the role's name (§2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityClass {
    Member,
    Admin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TenantStatus {
    Pending,
    Active,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvitationMode {
    /// Issued by an admin valid today.
    Normal,
    /// The leader invitation written by activation (§3.4.3); no issuing membership.
    Activation,
    /// A replacement invitation from an outgoing admin's handover grant (§6.3).
    Handover,
    /// Issued by the recovery contact in the no-admin state (§6.4); no issuing membership.
    Recovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    Access,
    Replacement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestStatus {
    Pending,
    Approved,
    Declined,
    Withdrawn,
    Lapsed,
}

/// Who holds an FAU's recovery-contact seat (§6.5, ADR-003 decision 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryHolder {
    Ewb,
    SchoolRep,
}

macro_rules! codes {
    ($ty:ty { $($variant:ident => $code:literal),+ $(,)? }) => {
        impl $ty {
            pub const ALL: &'static [$ty] = &[$(<$ty>::$variant),+];

            pub fn code(self) -> &'static str {
                match self { $(<$ty>::$variant => $code),+ }
            }

            pub fn from_code(code: &str) -> Option<Self> {
                match code { $($code => Some(<$ty>::$variant),)+ _ => None }
            }
        }
    };
}

codes!(CapabilityClass { Member => "member", Admin => "admin" });
codes!(TenantStatus { Pending => "pending", Active => "active", Closed => "closed" });
codes!(InvitationMode {
    Normal => "normal",
    Activation => "activation",
    Handover => "handover",
    Recovery => "recovery",
});
codes!(RequestKind { Access => "access", Replacement => "replacement" });
codes!(RequestStatus {
    Pending => "pending",
    Approved => "approved",
    Declined => "declined",
    Withdrawn => "withdrawn",
    Lapsed => "lapsed",
});
codes!(RecoveryHolder { Ewb => "ewb", SchoolRep => "school_rep" });

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameError {
    Empty,
    TooLong,
    ControlCharacter,
}

fn validate_name(raw: &str, max_chars: usize) -> Result<String, NameError> {
    let s = raw.trim();
    if s.is_empty() {
        return Err(NameError::Empty);
    }
    if s.chars().count() > max_chars {
        return Err(NameError::TooLong);
    }
    if s.chars().any(char::is_control) {
        return Err(NameError::ControlCharacter);
    }
    Ok(s.to_owned())
}

/// A role's display name, as an admin typed it ("Leder", "Kasserer"). Free text, so it
/// is never written to audit parameters; audit carries the role's id instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleName(String);

impl RoleName {
    pub const MAX_CHARS: usize = 100;

    pub fn parse(raw: &str) -> Result<Self, NameError> {
        validate_name(raw, Self::MAX_CHARS).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An FAU's name. Pre-filled from the school name and editable (§3.1.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FauName(String);

impl FauName {
    pub const MAX_CHARS: usize = 200;

    pub fn parse(raw: &str) -> Result<Self, NameError> {
        validate_name(raw, Self::MAX_CHARS).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! round_trips {
        ($name:ident, $ty:ty) => {
            #[test]
            fn $name() {
                for v in <$ty>::ALL {
                    assert_eq!(<$ty>::from_code(v.code()), Some(*v));
                }
                assert_eq!(<$ty>::from_code("unknown"), None);
            }
        };
    }

    round_trips!(capability_class_round_trips, CapabilityClass);
    round_trips!(tenant_status_round_trips, TenantStatus);
    round_trips!(invitation_mode_round_trips, InvitationMode);
    round_trips!(request_kind_round_trips, RequestKind);
    round_trips!(request_status_round_trips, RequestStatus);
    round_trips!(recovery_holder_round_trips, RecoveryHolder);

    #[test]
    fn codes_match_the_database_check_constraints() {
        // The literal sets from 0002 and 0003; changing either side must change both.
        assert_eq!(
            CapabilityClass::ALL
                .iter()
                .map(|v| v.code())
                .collect::<Vec<_>>(),
            ["member", "admin"]
        );
        assert_eq!(
            TenantStatus::ALL
                .iter()
                .map(|v| v.code())
                .collect::<Vec<_>>(),
            ["pending", "active", "closed"]
        );
        assert_eq!(
            InvitationMode::ALL
                .iter()
                .map(|v| v.code())
                .collect::<Vec<_>>(),
            ["normal", "activation", "handover", "recovery"]
        );
        assert_eq!(
            RequestKind::ALL
                .iter()
                .map(|v| v.code())
                .collect::<Vec<_>>(),
            ["access", "replacement"]
        );
        assert_eq!(
            RequestStatus::ALL
                .iter()
                .map(|v| v.code())
                .collect::<Vec<_>>(),
            ["pending", "approved", "declined", "withdrawn", "lapsed"]
        );
        assert_eq!(
            RecoveryHolder::ALL
                .iter()
                .map(|v| v.code())
                .collect::<Vec<_>>(),
            ["ewb", "school_rep"]
        );
    }

    #[test]
    fn names_are_trimmed_and_bounded() {
        assert_eq!(RoleName::parse("  Leder ").unwrap().as_str(), "Leder");
        assert_eq!(RoleName::parse("   "), Err(NameError::Empty));
        assert_eq!(
            RoleName::parse(&"æ".repeat(101)),
            Err(NameError::TooLong),
            "the bound counts characters, not bytes"
        );
        assert!(RoleName::parse(&"æ".repeat(100)).is_ok());
        assert_eq!(RoleName::parse("Le\nder"), Err(NameError::ControlCharacter));
        assert_eq!(
            FauName::parse("Nordre Skole FAU").unwrap().as_str(),
            "Nordre Skole FAU"
        );
        assert_eq!(FauName::parse(&"x".repeat(201)), Err(NameError::TooLong));
    }
}
```

Create (or replace in full) `backend/crates/domain/src/membership/access.rs`:

```rust
//! Access evaluation (§2.1, §4, §6.3, §6.4). Rights are the union of roles valid today,
//! re-evaluated on every request from the database, never cached from login.

use jiff::civil::Date;

use super::period::Period;
use super::vocabulary::CapabilityClass;

/// What a person may do in one FAU today. Ordered: `Admin` includes every `Member` right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capability {
    None,
    Member,
    Admin,
}

/// One role assignment as the rules need it. `capability` comes from the role row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssignmentView {
    pub capability: CapabilityClass,
    pub period: Period,
    pub revoked: bool,
}

impl AssignmentView {
    pub fn valid_on(&self, day: Date) -> bool {
        !self.revoked && self.period.contains(day)
    }
}

/// One handover grant (§6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrantView {
    pub period: Period,
    pub revoked: bool,
}

impl GrantView {
    pub fn valid_on(&self, day: Date) -> bool {
        !self.revoked && self.period.contains(day)
    }
}

/// The preconditions that come before any role is looked at. Each one alone removes all
/// access: an FAU that is not active, a disabled or unverified account, or a revoked
/// membership (#3412: "a revoked right gives no access through the revoked right").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Standing {
    pub tenant_active: bool,
    pub account_usable: bool,
    pub membership_active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Access {
    pub capability: Capability,
    /// A handover grant is valid today. It gives no document access (§6.3); the HTTP
    /// layer lets it reach only the handover operations.
    pub handover: bool,
    /// When `capability` is `None`, the earliest future start of a role, so the page can
    /// say when access begins (§4).
    pub next_start: Option<Date>,
}

impl Access {
    pub const NONE: Access = Access {
        capability: Capability::None,
        handover: false,
        next_start: None,
    };
}

pub fn evaluate_access(
    standing: Standing,
    assignments: &[AssignmentView],
    grants: &[GrantView],
    today: Date,
) -> Access {
    if !(standing.tenant_active && standing.account_usable && standing.membership_active) {
        return Access::NONE;
    }
    let capability = assignments
        .iter()
        .filter(|a| a.valid_on(today))
        .map(|a| match a.capability {
            CapabilityClass::Member => Capability::Member,
            CapabilityClass::Admin => Capability::Admin,
        })
        .max()
        .unwrap_or(Capability::None);
    let handover = grants.iter().any(|g| g.valid_on(today));
    let next_start = if capability == Capability::None {
        assignments
            .iter()
            .filter(|a| !a.revoked && a.period.starts_on() > today)
            .map(|a| a.period.starts_on())
            .min()
    } else {
        None
    };
    Access {
        capability,
        handover,
        next_start,
    }
}

/// The negation of §6.4's no-admin state: an `admin`-class role valid today or a handover
/// grant valid today. The caller passes only rows whose membership is active and whose
/// account is usable; a revoked person's rows do not count.
pub fn tenant_has_admin(assignments: &[AssignmentView], grants: &[GrantView], today: Date) -> bool {
    assignments
        .iter()
        .any(|a| a.capability == CapabilityClass::Admin && a.valid_on(today))
        || grants.iter().any(|g| g.valid_on(today))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    const OK: Standing = Standing {
        tenant_active: true,
        account_usable: true,
        membership_active: true,
    };

    fn role(capability: CapabilityClass, from: Date, to: Date) -> AssignmentView {
        AssignmentView {
            capability,
            period: Period::new(from, to).unwrap(),
            revoked: false,
        }
    }

    fn grant(from: Date, to: Date) -> GrantView {
        GrantView {
            period: Period::new(from, to).unwrap(),
            revoked: false,
        }
    }

    #[test]
    fn a_role_valid_tomorrow_grants_nothing_today() {
        let today = date(2026, 9, 23);
        let a = [role(
            CapabilityClass::Member,
            date(2026, 9, 24),
            date(2027, 9, 24),
        )];
        let access = evaluate_access(OK, &a, &[], today);
        assert_eq!(access.capability, Capability::None);
        assert_eq!(access.next_start, Some(date(2026, 9, 24)));
    }

    #[test]
    fn the_end_date_itself_grants_nothing() {
        let a = [role(
            CapabilityClass::Member,
            date(2026, 8, 1),
            date(2027, 8, 1),
        )];
        assert_eq!(
            evaluate_access(OK, &a, &[], date(2027, 7, 31)).capability,
            Capability::Member
        );
        assert_eq!(
            evaluate_access(OK, &a, &[], date(2027, 8, 1)).capability,
            Capability::None
        );
    }

    #[test]
    fn rights_are_the_union_of_valid_roles() {
        // #3412's example: admin ends, a member role continues.
        let a = [
            role(CapabilityClass::Admin, date(2026, 8, 1), date(2027, 8, 1)),
            role(CapabilityClass::Member, date(2026, 8, 1), date(2028, 1, 1)),
        ];
        assert_eq!(
            evaluate_access(OK, &a, &[], date(2027, 7, 31)).capability,
            Capability::Admin
        );
        assert_eq!(
            evaluate_access(OK, &a, &[], date(2027, 8, 1)).capability,
            Capability::Member
        );
    }

    #[test]
    fn a_revoked_role_grants_nothing() {
        let mut a = role(CapabilityClass::Admin, date(2026, 8, 1), date(2027, 8, 1));
        a.revoked = true;
        assert_eq!(
            evaluate_access(OK, &[a], &[], date(2026, 9, 23)),
            Access::NONE
        );
    }

    #[test]
    fn each_standing_precondition_removes_all_access() {
        let a = [role(
            CapabilityClass::Admin,
            date(2026, 8, 1),
            date(2027, 8, 1),
        )];
        let g = [grant(date(2026, 8, 1), date(2027, 2, 1))];
        for standing in [
            Standing {
                tenant_active: false,
                ..OK
            },
            Standing {
                account_usable: false,
                ..OK
            },
            Standing {
                membership_active: false,
                ..OK
            },
        ] {
            assert_eq!(
                evaluate_access(standing, &a, &g, date(2026, 9, 23)),
                Access::NONE
            );
        }
    }

    #[test]
    fn a_handover_grant_alone_gives_no_capability() {
        let a = [role(
            CapabilityClass::Admin,
            date(2026, 10, 1),
            date(2027, 10, 1),
        )];
        let g = [grant(date(2027, 10, 1), date(2028, 4, 1))];
        let access = evaluate_access(OK, &a, &g, date(2027, 11, 1));
        assert_eq!(access.capability, Capability::None);
        assert!(access.handover);
        assert_eq!(access.next_start, None);
    }

    #[test]
    fn a_handover_grant_ends_at_its_boundary_or_on_revocation() {
        let g = grant(date(2027, 10, 1), date(2028, 4, 1));
        assert!(evaluate_access(OK, &[], &[g], date(2028, 3, 31)).handover);
        assert!(!evaluate_access(OK, &[], &[g], date(2028, 4, 1)).handover);
        let revoked = GrantView { revoked: true, ..g };
        assert!(!evaluate_access(OK, &[], &[revoked], date(2027, 11, 1)).handover);
    }

    #[test]
    fn the_no_admin_predicate() {
        let today = date(2026, 9, 23);
        let member = role(CapabilityClass::Member, date(2026, 1, 1), date(2027, 1, 1));
        let admin = role(CapabilityClass::Admin, date(2026, 1, 1), date(2027, 1, 1));
        let future_admin = role(CapabilityClass::Admin, date(2026, 10, 1), date(2027, 10, 1));
        let g = grant(date(2026, 8, 1), date(2027, 2, 1));

        assert!(!tenant_has_admin(&[], &[], today));
        assert!(
            !tenant_has_admin(&[member], &[], today),
            "the class decides, not the name"
        );
        assert!(!tenant_has_admin(&[future_admin], &[], today));
        assert!(tenant_has_admin(&[admin], &[], today));
        assert!(
            tenant_has_admin(&[member], &[g], today),
            "a valid handover grant counts"
        );
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd /workspace/backend && cargo test -p fau-domain membership::`
Expected: the new tests do not run yet (the files are not modules).

- [ ] **Step 3: Implement**

Create (or replace in full) `backend/crates/domain/src/membership/mod.rs`:

```rust
//! The membership rules from the #3413 flow spec, as pure functions over values. No I/O:
//! persistence loads a snapshot, calls these, and writes the outcome in one transaction.

pub mod access;
pub mod period;
pub mod rules;
pub mod vocabulary;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd /workspace/backend && cargo test -p fau-domain`
Expected: PASS, and `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings` are clean.

- [ ] **Step 5: Commit** (the controller commits; executors do not)

```bash
git add backend/crates/domain/src/membership
git commit -m "Add membership vocabulary and access evaluation to the domain (#3413)"
```

---

### Task 4: Domain: invitation acceptance and request limits

Spec §5.1's acceptance checks as one pure function over a snapshot, returning a typed refusal per failure, in a fixed order (token state, person, FAU, issuer authority). "Email is verified" is carried by the `VerifiedEmail` type rather than a runtime flag. Plus §5.2–5.4's limits, message rule, 30-day lapse cutoff and replacement-date check.

**Files:**
- Create: `backend/crates/domain/src/membership/acceptance.rs`
- Create: `backend/crates/domain/src/membership/requests.rs`
- Replace in full: `backend/crates/domain/src/membership/mod.rs`

**Interfaces:**
- Consumes: `Email`, `VerifiedEmail`, `InvitationMode`, `TenantStatus`, `Period`.
- Produces (`fau_domain::membership`):
  - `acceptance::{AcceptanceSnapshot, AcceptanceRefusal, check_acceptance}`: `check_acceptance(&AcceptanceSnapshot, Timestamp) -> Result<(), AcceptanceRefusal>`; refusals `Revoked, AlreadyAccepted, Expired, EmailMismatch, AccountDisabled, TenantNotActive, TenantFrozen, IssuerLacksAuthority`.
  - `requests::{check_request_limits, RequestLimit, normalise_message, MessageTooLong, lapse_cutoff, check_replacement_dates, ReplacementDateError}` and the constants `MAX_OPEN_REQUESTS_PER_ADDRESS`, `MAX_REQUESTS_PER_TENANT_PER_DAY`, `REQUEST_LAPSE_DAYS`, `MESSAGE_MAX_CHARS`.

- [ ] **Step 1: Write the failing tests**

Create (or replace in full) `backend/crates/domain/src/membership/acceptance.rs`:

```rust
//! Invitation acceptance (§5.1). Persistence locks the invitation, reads everything the
//! rule needs into an [`AcceptanceSnapshot`], and calls [`check_acceptance`]; the rule
//! itself never touches the database, so every failure case is a unit test here.

use jiff::Timestamp;

use super::vocabulary::{InvitationMode, TenantStatus};
use crate::email::{Email, VerifiedEmail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptanceSnapshot {
    pub mode: InvitationMode,
    pub expires_at: Timestamp,
    pub accepted: bool,
    pub revoked: bool,
    pub recipient: Email,
    /// The address the provider verified for the person clicking "Bli med". The type
    /// carries the verification, so "email is verified" is not a runtime check.
    pub acceptor: VerifiedEmail,
    pub acceptor_account_disabled: bool,
    pub tenant_status: TenantStatus,
    pub tenant_frozen: bool,
    /// `normal` mode: the issuing membership holds an admin role valid today.
    pub issuer_admin_today: bool,
    /// `handover` mode: the linked grant is valid today and still belongs to the issuer.
    pub handover_grant_valid_today: bool,
    /// `recovery` mode: the FAU has an admin today, which ends the recovery contact's
    /// authority (§6.4).
    pub tenant_has_admin_today: bool,
}

/// Why an acceptance was refused. Each maps to its own message in #3422, and none of
/// them names another person (§5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptanceRefusal {
    Revoked,
    AlreadyAccepted,
    Expired,
    EmailMismatch,
    AccountDisabled,
    TenantNotActive,
    TenantFrozen,
    IssuerLacksAuthority,
}

/// Every check in §5.1, in a fixed order: the token's own state first, then the person,
/// then the FAU, then the issuer's authority.
pub fn check_acceptance(s: &AcceptanceSnapshot, now: Timestamp) -> Result<(), AcceptanceRefusal> {
    if s.revoked {
        return Err(AcceptanceRefusal::Revoked);
    }
    if s.accepted {
        return Err(AcceptanceRefusal::AlreadyAccepted);
    }
    if now >= s.expires_at {
        return Err(AcceptanceRefusal::Expired);
    }
    if &s.recipient != s.acceptor.email() {
        return Err(AcceptanceRefusal::EmailMismatch);
    }
    if s.acceptor_account_disabled {
        return Err(AcceptanceRefusal::AccountDisabled);
    }
    if s.tenant_status != TenantStatus::Active {
        return Err(AcceptanceRefusal::TenantNotActive);
    }
    if s.tenant_frozen {
        return Err(AcceptanceRefusal::TenantFrozen);
    }
    let authorised = match s.mode {
        InvitationMode::Normal => s.issuer_admin_today,
        InvitationMode::Handover => s.handover_grant_valid_today,
        InvitationMode::Recovery => !s.tenant_has_admin_today,
        // Completes the signup form (decision 11): the registrant's own authority is not
        // re-checked, only the token, the address and the FAU.
        InvitationMode::Activation => true,
    };
    if !authorised {
        return Err(AcceptanceRefusal::IssuerLacksAuthority);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    fn email(s: &str) -> Email {
        Email::parse(s).unwrap()
    }

    fn valid(mode: InvitationMode) -> AcceptanceSnapshot {
        AcceptanceSnapshot {
            mode,
            expires_at: ts("2026-10-07T10:00:00Z"),
            accepted: false,
            revoked: false,
            recipient: email("ny@example.test"),
            acceptor: VerifiedEmail::from_provider(email("ny@example.test")),
            acceptor_account_disabled: false,
            tenant_status: TenantStatus::Active,
            tenant_frozen: false,
            issuer_admin_today: true,
            handover_grant_valid_today: true,
            tenant_has_admin_today: false,
        }
    }

    const NOW: &str = "2026-09-30T10:00:00Z";

    #[test]
    fn a_valid_invitation_is_accepted_in_every_mode() {
        for mode in InvitationMode::ALL {
            assert_eq!(check_acceptance(&valid(*mode), ts(NOW)), Ok(()), "{mode:?}");
        }
    }

    #[test]
    fn expired_used_and_revoked_tokens_are_refused() {
        let s = valid(InvitationMode::Normal);
        assert_eq!(
            check_acceptance(&s, ts("2026-10-07T10:00:00Z")),
            Err(AcceptanceRefusal::Expired),
            "the expiry instant itself is already expired"
        );
        let used = AcceptanceSnapshot {
            accepted: true,
            ..s.clone()
        };
        assert_eq!(
            check_acceptance(&used, ts(NOW)),
            Err(AcceptanceRefusal::AlreadyAccepted)
        );
        let revoked = AcceptanceSnapshot { revoked: true, ..s };
        assert_eq!(
            check_acceptance(&revoked, ts(NOW)),
            Err(AcceptanceRefusal::Revoked)
        );
    }

    #[test]
    fn revocation_is_reported_before_expiry() {
        let s = AcceptanceSnapshot {
            revoked: true,
            ..valid(InvitationMode::Normal)
        };
        assert_eq!(
            check_acceptance(&s, ts("2027-01-01T00:00:00Z")),
            Err(AcceptanceRefusal::Revoked)
        );
    }

    #[test]
    fn a_different_address_is_refused() {
        let s = AcceptanceSnapshot {
            acceptor: VerifiedEmail::from_provider(email("annen@example.test")),
            ..valid(InvitationMode::Normal)
        };
        assert_eq!(
            check_acceptance(&s, ts(NOW)),
            Err(AcceptanceRefusal::EmailMismatch)
        );
    }

    #[test]
    fn a_disabled_account_is_refused() {
        let s = AcceptanceSnapshot {
            acceptor_account_disabled: true,
            ..valid(InvitationMode::Normal)
        };
        assert_eq!(
            check_acceptance(&s, ts(NOW)),
            Err(AcceptanceRefusal::AccountDisabled)
        );
    }

    #[test]
    fn an_fau_that_is_not_active_or_is_frozen_is_refused() {
        for status in [TenantStatus::Pending, TenantStatus::Closed] {
            let s = AcceptanceSnapshot {
                tenant_status: status,
                ..valid(InvitationMode::Normal)
            };
            assert_eq!(
                check_acceptance(&s, ts(NOW)),
                Err(AcceptanceRefusal::TenantNotActive)
            );
        }
        let frozen = AcceptanceSnapshot {
            tenant_frozen: true,
            ..valid(InvitationMode::Normal)
        };
        assert_eq!(
            check_acceptance(&frozen, ts(NOW)),
            Err(AcceptanceRefusal::TenantFrozen)
        );
    }

    #[test]
    fn the_issuer_must_still_have_authority() {
        let normal = AcceptanceSnapshot {
            issuer_admin_today: false,
            ..valid(InvitationMode::Normal)
        };
        assert_eq!(
            check_acceptance(&normal, ts(NOW)),
            Err(AcceptanceRefusal::IssuerLacksAuthority)
        );
        let handover = AcceptanceSnapshot {
            handover_grant_valid_today: false,
            ..valid(InvitationMode::Handover)
        };
        assert_eq!(
            check_acceptance(&handover, ts(NOW)),
            Err(AcceptanceRefusal::IssuerLacksAuthority)
        );
        let recovery = AcceptanceSnapshot {
            tenant_has_admin_today: true,
            ..valid(InvitationMode::Recovery)
        };
        assert_eq!(
            check_acceptance(&recovery, ts(NOW)),
            Err(AcceptanceRefusal::IssuerLacksAuthority)
        );
    }

    #[test]
    fn an_activation_invitation_needs_no_issuer_authority() {
        let s = AcceptanceSnapshot {
            issuer_admin_today: false,
            handover_grant_valid_today: false,
            tenant_has_admin_today: true,
            ..valid(InvitationMode::Activation)
        };
        assert_eq!(check_acceptance(&s, ts(NOW)), Ok(()));
    }
}
```

Create (or replace in full) `backend/crates/domain/src/membership/requests.rs`:

```rust
//! Access requests and replacement proposals (§5.2–5.4): one model, one set of limits.
//! The limits are the spec's starting values for #3417 and #3418 to tune (§12).

use jiff::civil::Date;
use jiff::ToSpan;

use super::period::Period;

/// One open request per address per FAU (§5.4).
pub const MAX_OPEN_REQUESTS_PER_ADDRESS: i64 = 1;
/// Five requests per FAU per day (§5.4).
pub const MAX_REQUESTS_PER_TENANT_PER_DAY: i64 = 5;
/// An unhandled request lapses after this many days (§5.2.4).
pub const REQUEST_LAPSE_DAYS: i32 = 30;
/// The optional message is plain text of at most this many characters (§5.2).
pub const MESSAGE_MAX_CHARS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestLimit {
    /// This address already has an open request in this FAU.
    OpenRequestExists,
    /// The FAU has received its daily maximum.
    TenantDailyLimit,
}

/// `open_by_address`: pending requests from this address in this FAU, where a
/// replacement proposal counts against its proposer. `created_today`: requests created in
/// this FAU on today's Europe/Oslo date, any status.
pub fn check_request_limits(open_by_address: i64, created_today: i64) -> Result<(), RequestLimit> {
    if open_by_address >= MAX_OPEN_REQUESTS_PER_ADDRESS {
        return Err(RequestLimit::OpenRequestExists);
    }
    if created_today >= MAX_REQUESTS_PER_TENANT_PER_DAY {
        return Err(RequestLimit::TenantDailyLimit);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageTooLong;

/// Trims the optional message; blank becomes `None`. Escaping is the renderer's job
/// ("always escaped", §5.2), so the text is stored as typed.
pub fn normalise_message(raw: Option<&str>) -> Result<Option<String>, MessageTooLong> {
    match raw.map(str::trim) {
        None | Some("") => Ok(None),
        Some(s) if s.chars().count() > MESSAGE_MAX_CHARS => Err(MessageTooLong),
        Some(s) => Ok(Some(s.to_owned())),
    }
}

/// Requests created on or before this date have lapsed by `today`: a request created on
/// 1 September lapses on 1 October.
pub fn lapse_cutoff(today: Date) -> Date {
    today
        .checked_sub(REQUEST_LAPSE_DAYS.days())
        .unwrap_or(Date::MIN)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplacementDateError {
    /// The successor's role would start before today (§5.3).
    StartsInPast,
    Empty,
}

/// Validates a replacement proposal's dates: starting no earlier than today, non-empty.
pub fn check_replacement_dates(
    today: Date,
    starts_on: Date,
    ends_on_exclusive: Date,
) -> Result<Period, ReplacementDateError> {
    if starts_on < today {
        return Err(ReplacementDateError::StartsInPast);
    }
    Period::new(starts_on, ends_on_exclusive).map_err(|_| ReplacementDateError::Empty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn one_open_request_per_address() {
        assert_eq!(check_request_limits(0, 0), Ok(()));
        assert_eq!(
            check_request_limits(1, 0),
            Err(RequestLimit::OpenRequestExists)
        );
    }

    #[test]
    fn five_requests_per_fau_per_day() {
        assert_eq!(check_request_limits(0, 4), Ok(()));
        assert_eq!(
            check_request_limits(0, 5),
            Err(RequestLimit::TenantDailyLimit)
        );
    }

    #[test]
    fn messages_are_trimmed_optional_and_bounded_in_characters() {
        assert_eq!(normalise_message(None), Ok(None));
        assert_eq!(normalise_message(Some("   ")), Ok(None));
        assert_eq!(
            normalise_message(Some(" Hei! ")),
            Ok(Some("Hei!".to_owned()))
        );
        assert!(normalise_message(Some(&"ø".repeat(500))).is_ok());
        assert_eq!(
            normalise_message(Some(&"ø".repeat(501))),
            Err(MessageTooLong)
        );
    }

    #[test]
    fn requests_lapse_after_thirty_days() {
        assert_eq!(lapse_cutoff(date(2026, 10, 1)), date(2026, 9, 1));
        assert_eq!(lapse_cutoff(date(2026, 3, 1)), date(2026, 1, 30));
    }

    #[test]
    fn replacement_dates_start_today_or_later() {
        let today = date(2026, 9, 23);
        assert_eq!(
            check_replacement_dates(today, date(2026, 9, 22), date(2027, 10, 1)),
            Err(ReplacementDateError::StartsInPast)
        );
        assert_eq!(
            check_replacement_dates(today, date(2027, 10, 1), date(2027, 10, 1)),
            Err(ReplacementDateError::Empty)
        );
        let p = check_replacement_dates(today, today, date(2027, 10, 1)).unwrap();
        assert_eq!(p.starts_on(), today);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd /workspace/backend && cargo test -p fau-domain membership::`
Expected: the new tests do not run yet (the files are not modules).

- [ ] **Step 3: Implement**

Create (or replace in full) `backend/crates/domain/src/membership/mod.rs`:

```rust
//! The membership rules from the #3413 flow spec, as pure functions over values. No I/O:
//! persistence loads a snapshot, calls these, and writes the outcome in one transaction.

pub mod acceptance;
pub mod access;
pub mod period;
pub mod requests;
pub mod rules;
pub mod vocabulary;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd /workspace/backend && cargo test -p fau-domain`
Expected: PASS, and `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings` are clean.

- [ ] **Step 5: Commit** (the controller commits; executors do not)

```bash
git add backend/crates/domain/src/membership
git commit -m "Add invitation acceptance and request limits to the domain (#3413)"
```

---

### Task 5: Persistence: plumbing, signup, activation and pending expiry

Adds the `fau_persistence::membership` module: the typed error, invitation tokens (32 OS-random bytes, SHA-256 stored, raw token returned once with redacted `Debug`), the text binding for dates and timestamps, the tenant lock, the audit and outbox writers, and the first three transactions. `insert_invitation` is introduced here because activation issues the leader invitation; Task 6 grows `invitations.rs`. `sql.rs` starts with only what this task uses and grows in Tasks 6–9, so every intermediate state passes `clippy -D warnings` with no dead code.

**Dependencies added, each justified:** `getrandom 0.4` (OS CSPRNG for tokens; chosen over `rand` because a token needs raw OS randomness, not a userspace PRNG, and it is already in `Cargo.lock` via `uuid`); `sha2 0.10` (token hashing; already in `Cargo.lock` via `sqlx`); `jiff`, `serde_json` and `uuid` in persistence (all already workspace dependencies). So `Cargo.lock` gains no crate in this task.

The test fixtures in `tests/common/membership.rs` build FAU-er through these functions, so a working fixture is evidence too. All persistence tests run as `fau_app` (`db.app_pool()`), which proves 0003's grants suffice.

**Files:**
- Modify: `backend/Cargo.toml`
- Modify: `backend/crates/persistence/Cargo.toml`
- Modify: `backend/crates/app/Cargo.toml`
- Create: `backend/crates/persistence/src/membership/error.rs`
- Create: `backend/crates/persistence/src/membership/token.rs`
- Create: `backend/crates/persistence/src/membership/sql.rs`
- Create: `backend/crates/persistence/src/membership/invitations.rs`
- Create: `backend/crates/persistence/src/membership/signup.rs`
- Create: `backend/crates/persistence/src/membership/mod.rs`
- Modify: `backend/crates/persistence/src/lib.rs`
- Modify: `backend/crates/app/tests/common/mod.rs`
- Create: `backend/crates/app/tests/common/membership.rs`
- Create: `backend/crates/app/tests/signup.rs`

**Interfaces:**
- Consumes: everything from Tasks 1–4; `crate::pool::safe_error_kind`.
- Produces (`fau_persistence::membership`):
  - `MembershipError` (every variant used by later tasks is declared here), `ExistingFau::{Pending, Active}`.
  - `InvitationToken` (`.expose() -> &str`), `IssuedInvitation { invitation_id, token, expires_at }`.
  - `create_pending_tenant(&PgPool, PendingSignup, Moment) -> Result<PendingTenant, MembershipError>`; `PendingSignup { school_id, fau_name: FauName, registrant_email: Email, leader_email: Email, admin_ends_on_exclusive: Date }`; `PendingTenant { tenant_id, expires_at }`.
  - `activate_tenant(&PgPool, Activation, Moment) -> Result<Activated, MembershipError>`; `Activation { tenant_id, registrant: VerifiedEmail }`; `Activated { account_id, membership_id, admin_role_id, admin_assignment_id, leader_invitation: Option<IssuedInvitation> }`.
  - `expire_pending_tenants(&PgPool, Moment) -> Result<u64, MembershipError>`.
  - Crate-private, used by later tasks: `sql::{date_param, ts_param, parse_date, from_micros, lock_tenant, write_audit, Audit, ActorKind, enqueue, upsert_verified_account, ensure_membership, insert_assignment}`, `invitations::{insert_invitation, NewInvitation}`, `token::hash_token`.
  - Test fixtures (`tests/common/membership.rs`): `T0`, `at`, `email`, `verified`, `day`, `period`, `signup`, `Fau`, `active_fau`, `count`, `outbox_count`, `audit_count`.

- [ ] **Step 1: Write the failing tests**

In `backend/crates/app/tests/common/mod.rs`, replace:

```rust
use uuid::Uuid;
```

with:

```rust
use uuid::Uuid;

pub mod membership;
```

Create (or replace in full) `backend/crates/app/tests/common/membership.rs`:

```rust
//! Builders for the membership integration tests. Every fixture goes through the
//! persistence functions themselves, so a fixture that works is evidence too.

use fau_domain::email::{Email, VerifiedEmail};
use fau_domain::membership::period::Period;
use fau_domain::membership::rules::default_admin_end;
use fau_domain::membership::vocabulary::FauName;
use fau_domain::time::Moment;
use fau_persistence::membership::{
    activate_tenant, create_pending_tenant, Activation, PendingSignup,
};
use jiff::civil::Date;
use sqlx::PgPool;
use uuid::Uuid;

/// The instant most tests start at: 23 September 2026, 12:00 in Oslo. The default admin
/// end date from here is 2027-10-01.
pub const T0: &str = "2026-09-23T10:00:00Z";

pub fn at(rfc3339: &str) -> Moment {
    Moment::at(rfc3339.parse().expect("an RFC 3339 timestamp"))
}

pub fn email(s: &str) -> Email {
    Email::parse(s).expect("a valid test address")
}

pub fn verified(s: &str) -> VerifiedEmail {
    VerifiedEmail::from_provider(email(s))
}

pub fn day(year: i16, month: i8, d: i8) -> Date {
    jiff::civil::date(year, month, d)
}

pub fn period(from: Date, to: Date) -> Period {
    Period::new(from, to).expect("a non-empty test period")
}

pub fn signup(school_id: Uuid, registrant: &str, leader: &str, at: Moment) -> PendingSignup {
    PendingSignup {
        school_id,
        fau_name: FauName::parse("Nordre skole FAU").unwrap(),
        registrant_email: email(registrant),
        leader_email: email(leader),
        admin_ends_on_exclusive: default_admin_end(at.today()),
    }
}

/// An active FAU whose registrant is its only admin.
pub struct Fau {
    pub tenant_id: Uuid,
    pub admin_account_id: Uuid,
    pub admin_membership_id: Uuid,
    pub admin_role_id: Uuid,
    pub admin_assignment_id: Uuid,
}

pub async fn active_fau(pool: &PgPool, registrant: &str, at: Moment) -> Fau {
    let pending =
        create_pending_tenant(pool, signup(Uuid::now_v7(), registrant, registrant, at), at)
            .await
            .expect("signup");
    let activated = activate_tenant(
        pool,
        Activation {
            tenant_id: pending.tenant_id,
            registrant: verified(registrant),
        },
        at,
    )
    .await
    .expect("activation");
    Fau {
        tenant_id: pending.tenant_id,
        admin_account_id: activated.account_id,
        admin_membership_id: activated.membership_id,
        admin_role_id: activated.admin_role_id,
        admin_assignment_id: activated.admin_assignment_id,
    }
}

pub async fn count(pool: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.expect(sql)
}

pub async fn outbox_count(pool: &PgPool, template: &str, recipient: &str) -> i64 {
    sqlx::query_scalar("select count(*) from outbox where template = $1 and recipient_email = $2")
        .bind(template)
        .bind(recipient)
        .fetch_one(pool)
        .await
        .unwrap()
}

pub async fn audit_count(pool: &PgPool, action: &str) -> i64 {
    sqlx::query_scalar("select count(*) from audit_events where action = $1")
        .bind(action)
        .fetch_one(pool)
        .await
        .unwrap()
}
```

Create (or replace in full) `backend/crates/app/tests/signup.rs`:

```rust
//! Signup, activation and pending expiry (flow spec §3, §10 "Signup and activation").

mod common;
use common::membership::*;
use common::TestDb;
use fau_domain::membership::rules::AdminEndError;
use fau_persistence::membership::{
    activate_tenant, create_pending_tenant, expire_pending_tenants, Activation, ExistingFau,
    MembershipError,
};
use uuid::Uuid;

#[tokio::test]
async fn signup_creates_a_pending_fau() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);

    let pending = create_pending_tenant(
        &pool,
        signup(Uuid::now_v7(), "reg@example.test", "leder@example.test", t0),
        t0,
    )
    .await
    .unwrap();

    assert_eq!(pending.expires_at, "2026-09-30T10:00:00Z".parse().unwrap());
    let status: String = sqlx::query_scalar("select status from tenants where id = $1")
        .bind(pending.tenant_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "pending");
    assert_eq!(count(&pool, "select count(*) from tenant_signups").await, 1);
    assert_eq!(audit_count(&pool, "tenant.signup_created").await, 1);
    assert_eq!(
        count(&pool, "select count(*) from memberships").await,
        0,
        "the registrant has no data rights before verifying (spec 3.3)"
    );
}

#[tokio::test]
async fn a_second_signup_for_a_school_is_refused_and_copied_to_ewb() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let school = Uuid::now_v7();

    let first = create_pending_tenant(
        &pool,
        signup(school, "a@example.test", "a@example.test", t0),
        t0,
    )
    .await
    .unwrap();
    let err = create_pending_tenant(
        &pool,
        signup(school, "b@example.test", "b@example.test", t0),
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::SchoolTaken(ExistingFau::Pending));

    activate_tenant(
        &pool,
        Activation {
            tenant_id: first.tenant_id,
            registrant: verified("a@example.test"),
        },
        t0,
    )
    .await
    .unwrap();
    let err = create_pending_tenant(
        &pool,
        signup(school, "c@example.test", "c@example.test", t0),
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::SchoolTaken(ExistingFau::Active));

    // The refusal names nobody: the error carries only the state, and its text is fixed.
    assert_eq!(err.to_string(), "the school already has an FAU");
    // Both collisions reached Erik, and both are audited, although neither created a row.
    assert_eq!(
        outbox_count(&pool, "signup.collision", "fau@ewb-solutions.as").await,
        2
    );
    assert_eq!(audit_count(&pool, "tenant.signup_collision").await, 2);
    assert_eq!(count(&pool, "select count(*) from tenants").await, 1);
}

#[tokio::test]
async fn one_address_holds_at_most_three_pending_faus() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    for _ in 0..3 {
        create_pending_tenant(
            &pool,
            signup(Uuid::now_v7(), "r@example.test", "r@example.test", t0),
            t0,
        )
        .await
        .unwrap();
    }
    let err = create_pending_tenant(
        &pool,
        signup(Uuid::now_v7(), "r@example.test", "r@example.test", t0),
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::TooManyPendingSignups);
    // Another address is unaffected.
    create_pending_tenant(
        &pool,
        signup(Uuid::now_v7(), "s@example.test", "s@example.test", t0),
        t0,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn the_admin_end_date_must_be_one_to_twenty_four_months_away() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let mut s = signup(Uuid::now_v7(), "r@example.test", "r@example.test", t0);
    s.admin_ends_on_exclusive = day(2026, 10, 22);
    assert_eq!(
        create_pending_tenant(&pool, s.clone(), t0)
            .await
            .unwrap_err(),
        MembershipError::AdminEnd(AdminEndError::TooSoon)
    );
    s.admin_ends_on_exclusive = day(2028, 9, 24);
    assert_eq!(
        create_pending_tenant(&pool, s, t0).await.unwrap_err(),
        MembershipError::AdminEnd(AdminEndError::TooLate)
    );
}

#[tokio::test]
async fn a_pending_fau_expires_after_seven_days_and_frees_the_school() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let school = Uuid::now_v7();
    let pending = create_pending_tenant(
        &pool,
        signup(school, "r@example.test", "r@example.test", t0),
        t0,
    )
    .await
    .unwrap();

    assert_eq!(
        expire_pending_tenants(&pool, at("2026-09-30T09:59:59Z"))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        expire_pending_tenants(&pool, at("2026-09-30T10:00:00Z"))
            .await
            .unwrap(),
        1
    );

    assert_eq!(count(&pool, "select count(*) from tenants").await, 0);
    assert_eq!(count(&pool, "select count(*) from tenant_signups").await, 0);
    assert_eq!(audit_count(&pool, "tenant.signup_expired").await, 1);
    let later = at("2026-10-01T10:00:00Z");
    create_pending_tenant(
        &pool,
        signup(school, "ny@example.test", "ny@example.test", later),
        later,
    )
    .await
    .expect("the school is free again");
    assert_eq!(
        activate_tenant(
            &pool,
            Activation {
                tenant_id: pending.tenant_id,
                registrant: verified("r@example.test"),
            },
            later,
        )
        .await
        .unwrap_err(),
        MembershipError::UnknownTenant
    );
}

#[tokio::test]
async fn an_expired_pending_fau_does_not_hold_its_school_before_the_sweep() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let school = Uuid::now_v7();
    let pending = create_pending_tenant(
        &pool,
        signup(school, "r@example.test", "r@example.test", t0),
        t0,
    )
    .await
    .unwrap();

    let late = at("2026-09-30T10:00:00Z");
    assert_eq!(
        activate_tenant(
            &pool,
            Activation {
                tenant_id: pending.tenant_id,
                registrant: verified("r@example.test"),
            },
            late,
        )
        .await
        .unwrap_err(),
        MembershipError::SignupExpired
    );
    create_pending_tenant(
        &pool,
        signup(school, "ny@example.test", "ny@example.test", late),
        late,
    )
    .await
    .expect("the stale pending FAU is expired inline");
}

#[tokio::test]
async fn activation_writes_everything_together() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let pending = create_pending_tenant(
        &pool,
        signup(Uuid::now_v7(), "reg@example.test", "leder@example.test", t0),
        t0,
    )
    .await
    .unwrap();

    let activated = activate_tenant(
        &pool,
        Activation {
            tenant_id: pending.tenant_id,
            registrant: verified("reg@example.test"),
        },
        t0,
    )
    .await
    .unwrap();

    let status: String = sqlx::query_scalar("select status from tenants where id = $1")
        .bind(pending.tenant_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "active");
    let verified_at_set: bool =
        sqlx::query_scalar("select verified_at is not null from accounts where id = $1")
            .bind(activated.account_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(verified_at_set);

    let (name, class, starts, ends): (String, String, String, String) = sqlx::query_as(
        "select r.name, r.capability_class, ra.starts_on::text, ra.ends_on_exclusive::text
           from role_assignments ra join roles r on r.tenant_id = ra.tenant_id and r.id = ra.role_id
          where ra.id = $1",
    )
    .bind(activated.admin_assignment_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (
            name.as_str(),
            class.as_str(),
            starts.as_str(),
            ends.as_str()
        ),
        ("Administrator", "admin", "2026-09-23", "2027-10-01")
    );

    let leader = activated
        .leader_invitation
        .expect("the leader is a different address");
    let (mode, issued_by_null, recipient): (String, bool, String) = sqlx::query_as(
        "select mode, issued_by is null, recipient_email from invitations where id = $1",
    )
    .bind(leader.invitation_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((mode.as_str(), issued_by_null), ("activation", true));
    assert_eq!(recipient, "leder@example.test");
    let (offered_role, offered_end): (uuid::Uuid, String) = sqlx::query_as(
        "select role_id, ends_on_exclusive::text from invitation_roles where invitation_id = $1",
    )
    .bind(leader.invitation_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(offered_role, activated.admin_role_id);
    assert_eq!(
        offered_end, "2027-10-01",
        "the leader is offered the same end date"
    );

    let holder: String =
        sqlx::query_scalar("select holder from recovery_contacts where tenant_id = $1")
            .bind(pending.tenant_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(holder, "ewb");
    assert_eq!(count(&pool, "select count(*) from tenant_signups").await, 0);
    assert_eq!(audit_count(&pool, "tenant.activated").await, 1);
    assert_eq!(
        outbox_count(&pool, "tenant.activated", "fau@ewb-solutions.as").await,
        1
    );
    assert_eq!(
        outbox_count(&pool, "invitation.issued", "leder@example.test").await,
        1
    );
    // Verifying the registrant verified nobody else.
    assert_eq!(
        count(
            &pool,
            "select count(*) from accounts where email = 'leder@example.test'"
        )
        .await,
        0
    );
}

#[tokio::test]
async fn activation_skips_the_leader_invitation_when_the_leader_is_the_registrant() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    active_fau(&pool, "solo@example.test", at(T0)).await;
    assert_eq!(count(&pool, "select count(*) from invitations").await, 0);
    assert_eq!(count(&pool, "select count(*) from memberships").await, 1);
}

#[tokio::test]
async fn activation_is_all_or_nothing() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let pending = create_pending_tenant(
        &pool,
        signup(Uuid::now_v7(), "reg@example.test", "leder@example.test", t0),
        t0,
    )
    .await
    .unwrap();
    // Sabotage the fourth step: a recovery seat already exists, so activation's own
    // insert fails after the status, account, membership, role and invitation writes.
    sqlx::query("insert into recovery_contacts (tenant_id, holder) values ($1, 'ewb')")
        .bind(pending.tenant_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();

    let err = activate_tenant(
        &pool,
        Activation {
            tenant_id: pending.tenant_id,
            registrant: verified("reg@example.test"),
        },
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::Database("sqlstate 23505".to_owned()));

    let status: String = sqlx::query_scalar("select status from tenants where id = $1")
        .bind(pending.tenant_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "pending");
    for table in [
        "accounts",
        "memberships",
        "roles",
        "role_assignments",
        "invitations",
        "outbox",
    ] {
        assert_eq!(
            count(&pool, &format!("select count(*) from {table}")).await,
            0,
            "{table} kept a row from a failed activation"
        );
    }
    assert_eq!(audit_count(&pool, "tenant.activated").await, 0);
    assert_eq!(count(&pool, "select count(*) from tenant_signups").await, 1);
}

#[tokio::test]
async fn activation_requires_the_registrants_own_address() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let pending = create_pending_tenant(
        &pool,
        signup(Uuid::now_v7(), "reg@example.test", "reg@example.test", t0),
        t0,
    )
    .await
    .unwrap();
    let err = activate_tenant(
        &pool,
        Activation {
            tenant_id: pending.tenant_id,
            registrant: verified("someone-else@example.test"),
        },
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::RegistrantMismatch);
}

#[tokio::test]
async fn a_registrant_with_an_account_elsewhere_keeps_one_account() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let first = active_fau(&pool, "same@example.test", at(T0)).await;
    let second = active_fau(&pool, "same@example.test", at(T0)).await;
    assert_eq!(first.admin_account_id, second.admin_account_id);
    assert_eq!(count(&pool, "select count(*) from accounts").await, 1);
    assert_eq!(count(&pool, "select count(*) from memberships").await, 2);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test -p fau-app --test signup`
Expected: compile error: `could not find membership in fau_persistence`.

- [ ] **Step 3: Implement**

In `backend/Cargo.toml`, replace:

```toml
clap = { version = "4", features = ["derive"] }
jiff = { version = "0.2", default-features = false, features = ["std", "tzdb-bundle-always"] }
```

with:

```toml
clap = { version = "4", features = ["derive"] }
getrandom = "0.4"
jiff = { version = "0.2", default-features = false, features = ["std", "tzdb-bundle-always"] }
```

In `backend/Cargo.toml`, replace:

```toml
serde_json = "1"
```

with:

```toml
serde_json = "1"
sha2 = "0.10"
```

In `backend/crates/persistence/Cargo.toml`, replace:

```toml
[dependencies]
fau-domain = { path = "../domain" }
sqlx = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
```

with:

```toml
[dependencies]
fau-domain = { path = "../domain" }
# Invitation tokens: 32 bytes straight from the operating system's CSPRNG. Already in
# Cargo.lock through `uuid`, so it adds no crate.
getrandom = { workspace = true }
# The date and timestamp types the domain rules take; bound into SQL as text (see
# src/membership/sql.rs), because sqlx 0.8 has no jiff support.
jiff = { workspace = true }
# Audit and outbox parameters, bound as text and cast to jsonb.
serde_json = { workspace = true }
# SHA-256 of invitation tokens. Already in Cargo.lock through `sqlx`.
sha2 = { workspace = true }
sqlx = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
# Row ids (UUIDv7, ADR-002), already a workspace dependency.
uuid = { workspace = true }
```

In `backend/crates/app/Cargo.toml`, replace:

```toml
[dev-dependencies]
```

with:

```toml
[dev-dependencies]
# Test-only: dates for the membership fixtures.
jiff = { workspace = true }
```

Create (or replace in full) `backend/crates/persistence/src/membership/error.rs`:

```rust
//! The one error type every membership transaction returns. Typed, so #3417's HTTP layer
//! can map each variant to an `ErrorCode` without parsing text; and its `Display` is a
//! fixed phrase per variant that never carries an address, a name, a token or a message.

use fau_domain::membership::acceptance::AcceptanceRefusal;
use fau_domain::membership::requests::{ReplacementDateError, RequestLimit};
use fau_domain::membership::rules::AdminEndError;

use crate::pool::safe_error_kind;

/// Which state the FAU that already holds a school is in (spec 3.2). The only thing a
/// colliding registrant learns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExistingFau {
    Pending,
    Active,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MembershipError {
    // Input that only makes sense against today's date or the database.
    #[error("admin end date out of range")]
    AdminEnd(AdminEndError),
    #[error("period is empty")]
    EmptyPeriod,
    #[error("period has already ended")]
    PeriodAlreadyEnded,
    #[error("no roles offered")]
    NoRolesOffered,
    #[error("a role is offered twice")]
    DuplicateRole,

    // Signup and activation.
    #[error("the school already has an FAU")]
    SchoolTaken(ExistingFau),
    #[error("too many pending signups for this address")]
    TooManyPendingSignups,
    #[error("the signup has expired")]
    SignupExpired,
    #[error("the verified address does not match the signup")]
    RegistrantMismatch,

    // The FAU.
    #[error("FAU not found")]
    UnknownTenant,
    #[error("the FAU is not pending")]
    TenantNotPending,
    #[error("the FAU is not active")]
    TenantNotActive,
    #[error("the FAU is frozen")]
    TenantFrozen,

    // Authority.
    #[error("the actor lacks authority for this action")]
    NotAuthorized,
    #[error("an invitation to oneself is refused")]
    SelfInvitation,
    #[error("the account is disabled")]
    AccountDisabled,

    // Invitations.
    #[error("invitation not found")]
    UnknownInvitation,
    #[error("the invitation is no longer pending")]
    InvitationNotPending,
    #[error("the invitation was refused")]
    Acceptance(AcceptanceRefusal),
    #[error("an end-date change is not allowed for this invitation")]
    OverrideNotAllowed,

    // Roles and memberships.
    #[error("role not found")]
    UnknownRole,
    #[error("the role is not an admin role")]
    NotAdminRole,
    #[error("membership not found")]
    UnknownMembership,
    #[error("the membership is revoked")]
    MembershipRevoked,
    #[error("role assignment not found")]
    UnknownAssignment,
    #[error("the role assignment is already revoked")]
    AssignmentAlreadyRevoked,
    #[error("the role is not held today")]
    RoleNotHeldToday,
    #[error("the action would leave the FAU without an administrator")]
    WouldLeaveNoAdmin,
    #[error("the FAU has an administrator")]
    NotInNoAdminState,

    // Requests.
    #[error("replacement dates are invalid")]
    ReplacementDates(ReplacementDateError),
    #[error("request limit reached")]
    RequestLimit(RequestLimit),
    #[error("the message is too long")]
    MessageTooLong,
    #[error("request not found")]
    UnknownRequest,
    #[error("the request is no longer pending")]
    RequestNotPending,

    // Infrastructure.
    #[error("the operating system's random source failed")]
    Randomness,
    /// `pool::safe_error_kind`'s fixed description: a SQLSTATE or a fixed word, never the
    /// driver's own message, which can quote bound values.
    #[error("database error ({0})")]
    Database(String),
}

impl From<sqlx::Error> for MembershipError {
    fn from(e: sqlx::Error) -> Self {
        MembershipError::Database(safe_error_kind(&e))
    }
}

impl MembershipError {
    /// A value read back from the database did not decode: a bug or a schema drift, not
    /// user input. Fixed text, like every other variant.
    pub(crate) fn decode() -> Self {
        MembershipError::Database("decode error".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_fixed_text() {
        assert_eq!(
            MembershipError::SchoolTaken(ExistingFau::Active).to_string(),
            "the school already has an FAU"
        );
        assert_eq!(
            MembershipError::Acceptance(AcceptanceRefusal::EmailMismatch).to_string(),
            "the invitation was refused"
        );
    }

    #[test]
    fn a_database_error_carries_only_the_sqlstate() {
        let e: MembershipError = crate::pool::test_support::database_error("23505").into();
        assert_eq!(e, MembershipError::Database("sqlstate 23505".to_owned()));
        assert_eq!(e.to_string(), "database error (sqlstate 23505)");
    }
}
```

Create (or replace in full) `backend/crates/persistence/src/membership/token.rs`:

```rust
//! Invitation tokens (spec 5.1): 32 bytes from the operating system's CSPRNG, shown as
//! 64 lower-case hex characters, stored only as the SHA-256 of that text. The raw token
//! is returned to the caller once, is never written to the database or the outbox, and
//! has a redacted `Debug` so it cannot reach a log line by accident.

use std::fmt;

use sha2::{Digest, Sha256};

use super::error::MembershipError;

/// A freshly generated invitation token. Deliberately neither `Clone` nor `Display`:
/// the only way to read it is [`InvitationToken::expose`], which is easy to grep for.
pub struct InvitationToken(String);

impl InvitationToken {
    pub(crate) fn generate() -> Result<Self, MembershipError> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|_| MembershipError::Randomness)?;
        Ok(Self(bytes.iter().map(|b| format!("{b:02x}")).collect()))
    }

    /// The raw token, for the link in the invitation email.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub(crate) fn hash(&self) -> Vec<u8> {
        hash_token(&self.0)
    }
}

impl fmt::Debug for InvitationToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("InvitationToken([redacted])")
    }
}

/// SHA-256 of the token's text, as stored in `invitations.token_hash`.
pub(crate) fn hash_token(raw: &str) -> Vec<u8> {
    Sha256::digest(raw.as_bytes()).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_64_lowercase_hex_characters_and_unique() {
        let a = InvitationToken::generate().unwrap();
        let b = InvitationToken::generate().unwrap();
        let hex = a.expose();
        assert_eq!(hex.len(), 64);
        assert!(hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        assert_ne!(a.expose(), b.expose());
    }

    #[test]
    fn the_hash_is_sha256_of_the_text() {
        // FIPS 180-2's "abc" test vector.
        let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let hex: String = hash_token("abc")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(hex, expected);
        let t = InvitationToken::generate().unwrap();
        assert_eq!(t.hash(), hash_token(t.expose()));
        assert_eq!(t.hash().len(), 32);
    }

    #[test]
    fn debug_is_redacted() {
        let t = InvitationToken::generate().unwrap();
        let debug = format!("{t:?}");
        assert_eq!(debug, "InvitationToken([redacted])");
        assert!(!debug.contains(t.expose()));
    }
}
```

Create (or replace in full) `backend/crates/persistence/src/membership/sql.rs`:

```rust
//! Shared pieces of every membership transaction: date and timestamp binding, the tenant
//! lock, authority checks, the admin state behind the last-admin safeguard, recipient
//! lists, and the audit and outbox writers.
//!
//! **Dates and timestamps cross the boundary as text.** sqlx 0.8 has no jiff support,
//! and adding `chrono` or `time` next to jiff would give the workspace two date types.
//! So a `Date` is bound as `'2027-10-01'` with a `::date` cast and read back with
//! `to_char(col, 'YYYY-MM-DD')`, and a `Timestamp` is bound as RFC 3339 with a
//! `::timestamptz` cast and read back as microseconds since the epoch. Rule-deciding
//! timestamps always come from the caller's `Moment`, never from SQL `now()`, so a test
//! can move time.

use fau_domain::membership::period::Period;
use fau_domain::membership::vocabulary::TenantStatus;
use fau_domain::time::Moment;
use jiff::civil::Date;
use jiff::Timestamp;
use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use super::error::MembershipError;

pub(crate) fn date_param(d: Date) -> String {
    d.to_string()
}

pub(crate) fn ts_param(t: Timestamp) -> String {
    t.to_string()
}

pub(crate) fn parse_date(s: &str) -> Result<Date, MembershipError> {
    s.parse().map_err(|_| MembershipError::decode())
}

pub(crate) fn from_micros(us: i64) -> Result<Timestamp, MembershipError> {
    Timestamp::from_microsecond(us).map_err(|_| MembershipError::decode())
}

pub(crate) struct TenantState {
    pub(crate) status: TenantStatus,
    pub(crate) frozen: bool,
}

/// Locks the tenant row for the rest of the transaction and returns its state. Every
/// membership mutation takes this lock first, which serialises role changes within one
/// FAU: two admins revoking each other at the same moment cannot both pass the
/// last-admin safeguard. `for no key update` does not block foreign-key checks from
/// other transactions inserting rows that reference the tenant.
pub(crate) async fn lock_tenant(
    conn: &mut PgConnection,
    tenant_id: Uuid,
) -> Result<TenantState, MembershipError> {
    let row: Option<(String, bool)> = sqlx::query_as(
        "select status, frozen_at is not null from tenants where id = $1 for no key update",
    )
    .bind(tenant_id)
    .fetch_optional(&mut *conn)
    .await?;
    let (status, frozen) = row.ok_or(MembershipError::UnknownTenant)?;
    let status = TenantStatus::from_code(&status).ok_or_else(MembershipError::decode)?;
    Ok(TenantState { status, frozen })
}

/// Who performed an audited action. Matches `audit_events.actor_kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActorKind {
    System,
    Registrant,
}

impl ActorKind {
    fn code(self) -> &'static str {
        match self {
            ActorKind::System => "system",
            ActorKind::Registrant => "registrant",
        }
    }
}

/// One audit entry. `params` holds ids, codes, dates and flags only (checked by
/// `audit_params_are_small` in size; by review in content).
pub(crate) struct Audit {
    pub(crate) tenant_id: Option<Uuid>,
    pub(crate) actor_kind: ActorKind,
    pub(crate) actor_account_id: Option<Uuid>,
    pub(crate) actor_membership_id: Option<Uuid>,
    pub(crate) action: &'static str,
    pub(crate) subject_type: &'static str,
    pub(crate) subject_id: Uuid,
    pub(crate) params: Value,
}

impl Audit {
    /// An entry written by a scheduled sweep rather than a person.
    pub(crate) fn system(
        tenant_id: Uuid,
        action: &'static str,
        subject_type: &'static str,
        subject_id: Uuid,
        params: Value,
    ) -> Self {
        Self {
            tenant_id: Some(tenant_id),
            actor_kind: ActorKind::System,
            actor_account_id: None,
            actor_membership_id: None,
            action,
            subject_type,
            subject_id,
            params,
        }
    }
}

pub(crate) async fn write_audit(
    conn: &mut PgConnection,
    at: Moment,
    e: Audit,
) -> Result<(), MembershipError> {
    sqlx::query(
        "insert into audit_events
           (id, tenant_id, actor_kind, actor_account_id, actor_membership_id,
            action, subject_type, subject_id, occurred_at, params)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9::timestamptz, $10::jsonb)",
    )
    .bind(Uuid::now_v7())
    .bind(e.tenant_id)
    .bind(e.actor_kind.code())
    .bind(e.actor_account_id)
    .bind(e.actor_membership_id)
    .bind(e.action)
    .bind(e.subject_type)
    .bind(e.subject_id)
    .bind(ts_param(at.now()))
    .bind(e.params.to_string())
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Queues one notification. `params` carries ids and codes for the renderer to look up;
/// never a token and never free text.
pub(crate) async fn enqueue(
    conn: &mut PgConnection,
    at: Moment,
    template: &'static str,
    recipient_email: &str,
    params: Value,
) -> Result<(), MembershipError> {
    sqlx::query(
        "insert into outbox (id, template, recipient_email, params, created_at)
         values ($1, $2, $3, $4::jsonb, $5::timestamptz)",
    )
    .bind(Uuid::now_v7())
    .bind(template)
    .bind(recipient_email)
    .bind(params.to_string())
    .bind(ts_param(at.now()))
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Finds or creates the account for a verified address, marking it verified if it was
/// not. Returns the account id and whether it is disabled.
pub(crate) async fn upsert_verified_account(
    conn: &mut PgConnection,
    email: &str,
    at: Moment,
) -> Result<(Uuid, bool), MembershipError> {
    Ok(sqlx::query_as(
        "insert into accounts (id, email, verified_at) values ($1, $2, $3::timestamptz)
         on conflict (email) do update
           set verified_at = coalesce(accounts.verified_at, excluded.verified_at)
         returning id, disabled_at is not null",
    )
    .bind(Uuid::now_v7())
    .bind(email)
    .bind(ts_param(at.now()))
    .fetch_one(&mut *conn)
    .await?)
}

/// Finds or creates the account's membership in the tenant. A revoked membership is
/// reopened rather than duplicated (one membership per account per FAU, #3412); the
/// revocation stays in the audit log. Returns the id and whether it already existed.
pub(crate) async fn ensure_membership(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    account_id: Uuid,
) -> Result<(Uuid, bool), MembershipError> {
    let existing: Option<Uuid> = sqlx::query_scalar(
        "select id from memberships where tenant_id = $1 and account_id = $2 for update",
    )
    .bind(tenant_id)
    .bind(account_id)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(id) = existing {
        sqlx::query("update memberships set revoked_at = null where tenant_id = $1 and id = $2")
            .bind(tenant_id)
            .bind(id)
            .execute(&mut *conn)
            .await?;
        return Ok((id, true));
    }
    let id = Uuid::now_v7();
    sqlx::query("insert into memberships (tenant_id, id, account_id) values ($1, $2, $3)")
        .bind(tenant_id)
        .bind(id)
        .bind(account_id)
        .execute(&mut *conn)
        .await?;
    Ok((id, false))
}

/// Inserts a role assignment and returns its id.
pub(crate) async fn insert_assignment(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
    role_id: Uuid,
    period: Period,
    granted_by: Option<Uuid>,
) -> Result<Uuid, MembershipError> {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into role_assignments
           (tenant_id, id, membership_id, role_id, starts_on, ends_on_exclusive, granted_by)
         values ($1, $2, $3, $4, $5::date, $6::date, $7)",
    )
    .bind(tenant_id)
    .bind(id)
    .bind(membership_id)
    .bind(role_id)
    .bind(date_param(period.starts_on()))
    .bind(date_param(period.ends_on_exclusive()))
    .bind(granted_by)
    .execute(&mut *conn)
    .await?;
    Ok(id)
}
```

Create (or replace in full) `backend/crates/persistence/src/membership/invitations.rs`:

```rust
//! Invitations (spec 5.1): issue, re-send, withdraw and accept.

use fau_domain::email::Email;
use fau_domain::membership::period::Period;
use fau_domain::membership::rules::invitation_expiry;
use fau_domain::membership::vocabulary::{InvitationMode, RecoveryHolder};
use fau_domain::time::Moment;
use jiff::Timestamp;
use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

use super::error::MembershipError;
use super::sql::{date_param, enqueue, ts_param, write_audit, ActorKind, Audit};
use super::token::InvitationToken;

/// A newly issued (or re-sent) invitation. `token` is the only copy of the raw token.
#[derive(Debug)]
pub struct IssuedInvitation {
    pub invitation_id: Uuid,
    pub token: InvitationToken,
    pub expires_at: Timestamp,
}

pub(crate) struct NewInvitation {
    pub(crate) tenant_id: Uuid,
    pub(crate) mode: InvitationMode,
    pub(crate) recipient: Email,
    pub(crate) issued_by: Option<Uuid>,
    pub(crate) handover_grant_id: Option<Uuid>,
    pub(crate) access_request_id: Option<Uuid>,
    pub(crate) recovery_holder: Option<RecoveryHolder>,
    pub(crate) roles: Vec<(Uuid, Period)>,
    pub(crate) actor_kind: ActorKind,
    pub(crate) actor_membership_id: Option<Uuid>,
}

/// Writes an invitation, its roles, its audit entry and its outbox message. Every path
/// that issues an invitation -- activation, an admin, a handover grant, an approved
/// request, the recovery contact -- goes through here, inside the caller's transaction.
pub(crate) async fn insert_invitation(
    conn: &mut PgConnection,
    at: Moment,
    new: NewInvitation,
) -> Result<IssuedInvitation, MembershipError> {
    let token = InvitationToken::generate()?;
    let invitation_id = Uuid::now_v7();
    let expires_at = invitation_expiry(at.now());
    sqlx::query(
        "insert into invitations
           (tenant_id, id, token_hash, mode, recipient_email, issued_by, handover_grant_id,
            access_request_id, recovery_holder, expires_at, created_at)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::timestamptz, $11::timestamptz)",
    )
    .bind(new.tenant_id)
    .bind(invitation_id)
    .bind(token.hash())
    .bind(new.mode.code())
    .bind(new.recipient.as_str())
    .bind(new.issued_by)
    .bind(new.handover_grant_id)
    .bind(new.access_request_id)
    .bind(new.recovery_holder.map(RecoveryHolder::code))
    .bind(ts_param(expires_at))
    .bind(ts_param(at.now()))
    .execute(&mut *conn)
    .await?;
    for (role_id, period) in &new.roles {
        sqlx::query(
            "insert into invitation_roles
               (tenant_id, invitation_id, role_id, starts_on, ends_on_exclusive)
             values ($1, $2, $3, $4::date, $5::date)",
        )
        .bind(new.tenant_id)
        .bind(invitation_id)
        .bind(role_id)
        .bind(date_param(period.starts_on()))
        .bind(date_param(period.ends_on_exclusive()))
        .execute(&mut *conn)
        .await?;
    }
    write_audit(
        conn,
        at,
        Audit {
            tenant_id: Some(new.tenant_id),
            actor_kind: new.actor_kind,
            actor_account_id: None,
            actor_membership_id: new.actor_membership_id,
            action: "invitation.issued",
            subject_type: "invitation",
            subject_id: invitation_id,
            params: json!({ "mode": new.mode.code(), "role_count": new.roles.len() }),
        },
    )
    .await?;
    enqueue(
        conn,
        at,
        "invitation.issued",
        new.recipient.as_str(),
        json!({
            "tenant_id": new.tenant_id,
            "invitation_id": invitation_id,
            "mode": new.mode.code(),
        }),
    )
    .await?;
    Ok(IssuedInvitation {
        invitation_id,
        token,
        expires_at,
    })
}
```

Create (or replace in full) `backend/crates/persistence/src/membership/signup.rs`:

```rust
//! Signup, activation and pending expiry (spec 3.2–3.4).

use fau_domain::email::{Email, VerifiedEmail};
use fau_domain::membership::period::Period;
use fau_domain::membership::rules::{
    pending_signup_expiry, validate_admin_end, BOOTSTRAP_ADMIN_ROLE_NAME, EWB_OVERSIGHT_ADDRESS,
    MAX_PENDING_SIGNUPS_PER_ADDRESS,
};
use fau_domain::membership::vocabulary::{
    CapabilityClass, FauName, InvitationMode, RecoveryHolder, TenantStatus,
};
use fau_domain::time::Moment;
use jiff::civil::Date;
use jiff::Timestamp;
use serde_json::json;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::error::{ExistingFau, MembershipError};
use super::invitations::{insert_invitation, IssuedInvitation, NewInvitation};
use super::sql::{
    date_param, enqueue, ensure_membership, from_micros, insert_assignment, lock_tenant,
    parse_date, ts_param, upsert_verified_account, write_audit, ActorKind, Audit,
};

/// The signup form (spec 3.1), already parsed by the caller. `school_id` is a bare uuid
/// until #3441's register gives it a foreign key.
#[derive(Debug, Clone)]
pub struct PendingSignup {
    pub school_id: Uuid,
    pub fau_name: FauName,
    pub registrant_email: Email,
    pub leader_email: Email,
    /// "Til hvilken dato er du valgt?", as an exclusive end.
    pub admin_ends_on_exclusive: Date,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingTenant {
    pub tenant_id: Uuid,
    pub expires_at: Timestamp,
}

/// Creates an FAU in `pending` (spec 3.3).
///
/// A school that already has a pending or active FAU is refused with
/// [`MembershipError::SchoolTaken`], which says which of the two and nothing else; the
/// refusal still commits a copy to EWB's outbox and a global audit entry (spec 3.2).
/// One address may hold at most three pending FAU-er.
pub async fn create_pending_tenant(
    pool: &PgPool,
    signup: PendingSignup,
    at: Moment,
) -> Result<PendingTenant, MembershipError> {
    validate_admin_end(at.today(), signup.admin_ends_on_exclusive)
        .map_err(MembershipError::AdminEnd)?;

    let mut tx = pool.begin().await?;
    // Serialise signups from one address, so the three-pending limit holds when two
    // forms are submitted at once. Released at commit or rollback.
    sqlx::query("select pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(signup.registrant_email.as_str())
        .execute(&mut *tx)
        .await?;
    // An expired pending FAU the sweep has not reached yet must not hold the school.
    expire_due(&mut tx, at, Some(signup.school_id)).await?;

    if let Some(existing) = live_tenant_for_school(&mut tx, signup.school_id).await? {
        return record_collision(tx, at, signup.school_id, existing).await;
    }

    let pending: i64 = sqlx::query_scalar(
        "select count(*) from tenant_signups s join tenants t on t.id = s.tenant_id
          where t.status = 'pending' and s.registrant_email = $1",
    )
    .bind(signup.registrant_email.as_str())
    .fetch_one(&mut *tx)
    .await?;
    if pending >= MAX_PENDING_SIGNUPS_PER_ADDRESS {
        return Err(MembershipError::TooManyPendingSignups);
    }

    let tenant_id = Uuid::now_v7();
    let inserted: Option<Uuid> = sqlx::query_scalar(
        "insert into tenants (id, name, status, school_id) values ($1, $2, 'pending', $3)
         on conflict (school_id) where status in ('pending', 'active') and school_id is not null
         do nothing
         returning id",
    )
    .bind(tenant_id)
    .bind(signup.fau_name.as_str())
    .bind(signup.school_id)
    .fetch_optional(&mut *tx)
    .await?;
    if inserted.is_none() {
        // Lost a race with a concurrent signup for the same school since the check above.
        let existing = live_tenant_for_school(&mut tx, signup.school_id)
            .await?
            .ok_or_else(MembershipError::decode)?;
        return record_collision(tx, at, signup.school_id, existing).await;
    }

    let expires_at = pending_signup_expiry(at.now());
    sqlx::query(
        "insert into tenant_signups
           (tenant_id, registrant_email, leader_email, admin_ends_on_exclusive, expires_at)
         values ($1, $2, $3, $4::date, $5::timestamptz)",
    )
    .bind(tenant_id)
    .bind(signup.registrant_email.as_str())
    .bind(signup.leader_email.as_str())
    .bind(date_param(signup.admin_ends_on_exclusive))
    .bind(ts_param(expires_at))
    .execute(&mut *tx)
    .await?;

    write_audit(
        &mut tx,
        at,
        Audit {
            tenant_id: Some(tenant_id),
            actor_kind: ActorKind::Registrant,
            actor_account_id: None,
            actor_membership_id: None,
            action: "tenant.signup_created",
            subject_type: "tenant",
            subject_id: tenant_id,
            params: json!({ "school_id": signup.school_id }),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(PendingTenant {
        tenant_id,
        expires_at,
    })
}

async fn live_tenant_for_school(
    conn: &mut PgConnection,
    school_id: Uuid,
) -> Result<Option<ExistingFau>, MembershipError> {
    let status: Option<String> = sqlx::query_scalar(
        "select status from tenants where school_id = $1 and status in ('pending', 'active')",
    )
    .bind(school_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(match status.as_deref().and_then(TenantStatus::from_code) {
        Some(TenantStatus::Active) => Some(ExistingFau::Active),
        Some(TenantStatus::Pending) => Some(ExistingFau::Pending),
        _ => None,
    })
}

/// Commits the collision copy to Erik and its audit entry, then returns the refusal.
async fn record_collision(
    mut tx: sqlx::Transaction<'_, sqlx::Postgres>,
    at: Moment,
    school_id: Uuid,
    existing: ExistingFau,
) -> Result<PendingTenant, MembershipError> {
    let status = match existing {
        ExistingFau::Active => "active",
        ExistingFau::Pending => "pending",
    };
    enqueue(
        &mut tx,
        at,
        "signup.collision",
        EWB_OVERSIGHT_ADDRESS,
        json!({ "school_id": school_id, "existing_status": status }),
    )
    .await?;
    write_audit(
        &mut tx,
        at,
        Audit {
            tenant_id: None,
            actor_kind: ActorKind::Registrant,
            actor_account_id: None,
            actor_membership_id: None,
            action: "tenant.signup_collision",
            subject_type: "school",
            subject_id: school_id,
            params: json!({ "existing_status": status }),
        },
    )
    .await?;
    tx.commit().await?;
    Err(MembershipError::SchoolTaken(existing))
}

/// Deletes pending FAU-er whose signup has expired (spec 3.3: deleted, not closed, since
/// they never became FAU-er), optionally only for one school. Returns how many.
async fn expire_due(
    conn: &mut PgConnection,
    at: Moment,
    school_id: Option<Uuid>,
) -> Result<u64, MembershipError> {
    let expired: Vec<Uuid> = sqlx::query_scalar(
        "delete from tenants
          where status = 'pending'
            and id in (select tenant_id from tenant_signups where expires_at <= $1::timestamptz)
            and ($2::uuid is null or school_id = $2)
         returning id",
    )
    .bind(ts_param(at.now()))
    .bind(school_id)
    .fetch_all(&mut *conn)
    .await?;
    for tenant_id in &expired {
        write_audit(
            conn,
            at,
            Audit::system(
                *tenant_id,
                "tenant.signup_expired",
                "tenant",
                *tenant_id,
                json!({}),
            ),
        )
        .await?;
    }
    Ok(expired.len() as u64)
}

/// The scheduled sweep: deletes every expired pending FAU, freeing its school.
pub async fn expire_pending_tenants(pool: &PgPool, at: Moment) -> Result<u64, MembershipError> {
    let mut tx = pool.begin().await?;
    let n = expire_due(&mut tx, at, None).await?;
    tx.commit().await?;
    Ok(n)
}

#[derive(Debug, Clone)]
pub struct Activation {
    pub tenant_id: Uuid,
    /// The registrant's address, just verified with a Hanko passcode (#3417).
    pub registrant: VerifiedEmail,
}

#[derive(Debug)]
pub struct Activated {
    pub account_id: Uuid,
    pub membership_id: Uuid,
    pub admin_role_id: Uuid,
    pub admin_assignment_id: Uuid,
    /// `None` when the leader address is the registrant's own (spec 3.4.3).
    pub leader_invitation: Option<IssuedInvitation>,
}

/// Activates a pending FAU in one transaction (spec 3.4): status, the registrant's
/// account, membership and admin role, the leader invitation, the EWB recovery seat,
/// audit, and the outbox copy to EWB -- all or nothing.
///
/// The leader invitation is exempt from #3414's gate (decision 11); so is activation.
pub async fn activate_tenant(
    pool: &PgPool,
    activation: Activation,
    at: Moment,
) -> Result<Activated, MembershipError> {
    let tenant_id = activation.tenant_id;
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, tenant_id).await?;
    if state.status != TenantStatus::Pending {
        return Err(MembershipError::TenantNotPending);
    }
    // A frozen FAU takes no writes and issues no invitations (ADR-003 decision 7a), and
    // activation does both.
    if state.frozen {
        return Err(MembershipError::TenantFrozen);
    }
    let (registrant_email, leader_email, admin_end, expires_us): (String, String, String, i64) =
        sqlx::query_as(
            "select registrant_email, leader_email,
                    to_char(admin_ends_on_exclusive, 'YYYY-MM-DD'),
                    (extract(epoch from expires_at) * 1000000)::bigint
               from tenant_signups where tenant_id = $1",
        )
        .bind(tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(MembershipError::TenantNotPending)?;
    if at.now() >= from_micros(expires_us)? {
        return Err(MembershipError::SignupExpired);
    }
    if registrant_email != activation.registrant.email().as_str() {
        return Err(MembershipError::RegistrantMismatch);
    }
    let period = Period::new(at.today(), parse_date(&admin_end)?)
        .map_err(|_| MembershipError::EmptyPeriod)?;

    sqlx::query("update tenants set status = 'active' where id = $1")
        .bind(tenant_id)
        .execute(&mut *tx)
        .await?;

    let (account_id, disabled) = upsert_verified_account(&mut tx, &registrant_email, at).await?;
    if disabled {
        return Err(MembershipError::AccountDisabled);
    }
    let (membership_id, _) = ensure_membership(&mut tx, tenant_id, account_id).await?;

    let admin_role_id = Uuid::now_v7();
    sqlx::query(
        "insert into roles (tenant_id, id, name, capability_class) values ($1, $2, $3, $4)",
    )
    .bind(tenant_id)
    .bind(admin_role_id)
    .bind(BOOTSTRAP_ADMIN_ROLE_NAME)
    .bind(CapabilityClass::Admin.code())
    .execute(&mut *tx)
    .await?;
    let admin_assignment_id = insert_assignment(
        &mut tx,
        tenant_id,
        membership_id,
        admin_role_id,
        period,
        None,
    )
    .await?;

    let leader_invitation = if leader_email != registrant_email {
        let leader = Email::parse(&leader_email).map_err(|_| MembershipError::decode())?;
        Some(
            insert_invitation(
                &mut tx,
                at,
                NewInvitation {
                    tenant_id,
                    mode: InvitationMode::Activation,
                    recipient: leader,
                    issued_by: None,
                    handover_grant_id: None,
                    access_request_id: None,
                    recovery_holder: None,
                    roles: vec![(admin_role_id, period)],
                    actor_kind: ActorKind::Registrant,
                    actor_membership_id: Some(membership_id),
                },
            )
            .await?,
        )
    } else {
        None
    };

    sqlx::query("insert into recovery_contacts (tenant_id, holder) values ($1, $2)")
        .bind(tenant_id)
        .bind(RecoveryHolder::Ewb.code())
        .execute(&mut *tx)
        .await?;
    // Pending-only data: the registrant is now an account, the leader an invitation.
    sqlx::query("delete from tenant_signups where tenant_id = $1")
        .bind(tenant_id)
        .execute(&mut *tx)
        .await?;

    write_audit(
        &mut tx,
        at,
        Audit {
            tenant_id: Some(tenant_id),
            actor_kind: ActorKind::Registrant,
            actor_account_id: Some(account_id),
            actor_membership_id: Some(membership_id),
            action: "tenant.activated",
            subject_type: "tenant",
            subject_id: tenant_id,
            params: json!({
                "admin_assignment_id": admin_assignment_id,
                "leader_invited": leader_invitation.is_some(),
            }),
        },
    )
    .await?;
    enqueue(
        &mut tx,
        at,
        "tenant.activated",
        EWB_OVERSIGHT_ADDRESS,
        json!({ "tenant_id": tenant_id }),
    )
    .await?;
    tx.commit().await?;

    Ok(Activated {
        account_id,
        membership_id,
        admin_role_id,
        admin_assignment_id,
        leader_invitation,
    })
}
```

Create (or replace in full) `backend/crates/persistence/src/membership/mod.rs`:

```rust
//! The membership foundation's transactions (#3413 flow spec; #3417, #3418). Each public
//! function is one transaction that writes its state change together with its audit
//! entry and, where the spec says so, its outbox message (spec 2.6).
//!
//! Every function takes a [`fau_domain::time::Moment`]: rule-deciding timestamps and
//! "today" come from the caller, never from the database clock.

mod error;
mod invitations;
mod signup;
mod sql;
mod token;

pub use error::{ExistingFau, MembershipError};
pub use invitations::IssuedInvitation;
pub use signup::{
    activate_tenant, create_pending_tenant, expire_pending_tenants, Activated, Activation,
    PendingSignup, PendingTenant,
};
pub use token::InvitationToken;
```

In `backend/crates/persistence/src/lib.rs`, replace:

```rust
mod health;
mod migrate;
```

with:

```rust
mod health;
pub mod membership;
mod migrate;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test --workspace`
Expected: PASS, and `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings` are clean.

- [ ] **Step 5: Commit** (the controller commits; executors do not)

```bash
git add backend/Cargo.toml backend/Cargo.lock backend/crates/persistence backend/crates/app/Cargo.toml backend/crates/app/tests/common backend/crates/app/tests/signup.rs
git commit -m "Add signup, activation and pending expiry transactions (#3413, #3417)"
```

---

### Task 6: Persistence: issue, re-send, withdraw and accept invitations

Every §5.1 check at acceptance, via Task 4's pure function over a snapshot read under lock (lock order: tenant, then invitation; the token hash is re-checked under the lock in case a re-send replaced it). Acceptance creates or reuses the account and the membership (a revoked membership is reopened, never duplicated) and lets the leader adjust the admin end date within 1–24 months on an `activation` invitation only. Re-send replaces the token on the same row, so the old token stops working, and restarts the 14 days. Handover issuance is here too (grant-scoped authority, existing roles only, no self-invitation); its end-to-end tests come in Task 9 once grants can be created. `sql.rs` gains the authority helpers, a first `AdminState` (the no-admin predicate's inputs, needed by recovery-mode acceptance) and the recipient queries.

**Files:**
- Modify: `backend/crates/app/Cargo.toml`
- Replace in full: `backend/crates/persistence/src/membership/token.rs`
- Modify: `backend/crates/persistence/src/membership/sql.rs`
- Replace in full: `backend/crates/persistence/src/membership/invitations.rs`
- Replace in full: `backend/crates/persistence/src/membership/mod.rs`
- Replace in full: `backend/crates/app/tests/common/membership.rs`
- Create: `backend/crates/app/tests/invitations.rs`

**Interfaces:**
- Consumes: Task 5's plumbing; `check_acceptance`, `validate_admin_end`, `invitation_expiry`.
- Produces:
  - `issue_invitation(&PgPool, IssueInvitation, Moment) -> Result<IssuedInvitation, MembershipError>`; `IssueInvitation { tenant_id, actor_membership_id, recipient: Email, roles: Vec<OfferedRole>, handover_grant_id: Option<Uuid> }`.
  - `resend_invitation(&PgPool, InvitationChange, Moment) -> Result<IssuedInvitation, _>`, `withdraw_invitation(&PgPool, InvitationChange, Moment) -> Result<(), _>`; `InvitationChange { tenant_id, actor_membership_id, invitation_id }`.
  - `accept_invitation(&PgPool, AcceptInvitation, Moment) -> Result<Accepted, _>`; `AcceptInvitation { token: String, acceptor: VerifiedEmail, admin_end_override: Option<Date> }`; `Accepted { tenant_id, account_id, membership_id, assignment_ids: Vec<Uuid> }`.
  - `RoleChoice::{Existing(Uuid), New { name: RoleName, capability: CapabilityClass }}`, `OfferedRole { role, period }`.
  - Crate-private: `invitations::resolve_roles(conn, Moment, tenant_id, Option<Uuid>, &[OfferedRole]) -> Result<Vec<(Uuid, CapabilityClass, Period)>, _>`; `sql::{period_from, require_open, is_admin_today, require_admin, membership_usable, usable_member_email, handover_grant_valid, AdminState { load, has_admin }, current_member_emails, recovery_notice_recipients}`, `Audit::member`.
  - Fixtures: `new_role(&str, CapabilityClass) -> RoleChoice`, `add_member(pool, &Fau, address, RoleChoice, Period, Moment) -> Accepted`.

- [ ] **Step 1: Write the failing tests**

In `backend/crates/app/Cargo.toml`, replace:

```toml
# Test-only: dates for the membership fixtures.
jiff = { workspace = true }
```

with:

```toml
# Test-only: dates for the membership fixtures, and an independent SHA-256 to check
# that invitations store the token's hash and never the token.
jiff = { workspace = true }
sha2 = { workspace = true }
```

Create (or replace in full) `backend/crates/app/tests/common/membership.rs`:

```rust
//! Builders for the membership integration tests. Every fixture goes through the
//! persistence functions themselves, so a fixture that works is evidence too.

use fau_domain::email::{Email, VerifiedEmail};
use fau_domain::membership::period::Period;
use fau_domain::membership::rules::default_admin_end;
use fau_domain::membership::vocabulary::{CapabilityClass, FauName, RoleName};
use fau_domain::time::Moment;
use fau_persistence::membership::{
    accept_invitation, activate_tenant, create_pending_tenant, issue_invitation, AcceptInvitation,
    Accepted, Activation, IssueInvitation, OfferedRole, PendingSignup, RoleChoice,
};
use jiff::civil::Date;
use sqlx::PgPool;
use uuid::Uuid;

/// The instant most tests start at: 23 September 2026, 12:00 in Oslo. The default admin
/// end date from here is 2027-10-01.
pub const T0: &str = "2026-09-23T10:00:00Z";

pub fn at(rfc3339: &str) -> Moment {
    Moment::at(rfc3339.parse().expect("an RFC 3339 timestamp"))
}

pub fn email(s: &str) -> Email {
    Email::parse(s).expect("a valid test address")
}

pub fn verified(s: &str) -> VerifiedEmail {
    VerifiedEmail::from_provider(email(s))
}

pub fn day(year: i16, month: i8, d: i8) -> Date {
    jiff::civil::date(year, month, d)
}

pub fn period(from: Date, to: Date) -> Period {
    Period::new(from, to).expect("a non-empty test period")
}

pub fn signup(school_id: Uuid, registrant: &str, leader: &str, at: Moment) -> PendingSignup {
    PendingSignup {
        school_id,
        fau_name: FauName::parse("Nordre skole FAU").unwrap(),
        registrant_email: email(registrant),
        leader_email: email(leader),
        admin_ends_on_exclusive: default_admin_end(at.today()),
    }
}

/// An active FAU whose registrant is its only admin.
pub struct Fau {
    pub tenant_id: Uuid,
    pub admin_account_id: Uuid,
    pub admin_membership_id: Uuid,
    pub admin_role_id: Uuid,
    pub admin_assignment_id: Uuid,
}

pub async fn active_fau(pool: &PgPool, registrant: &str, at: Moment) -> Fau {
    let pending =
        create_pending_tenant(pool, signup(Uuid::now_v7(), registrant, registrant, at), at)
            .await
            .expect("signup");
    let activated = activate_tenant(
        pool,
        Activation {
            tenant_id: pending.tenant_id,
            registrant: verified(registrant),
        },
        at,
    )
    .await
    .expect("activation");
    Fau {
        tenant_id: pending.tenant_id,
        admin_account_id: activated.account_id,
        admin_membership_id: activated.membership_id,
        admin_role_id: activated.admin_role_id,
        admin_assignment_id: activated.admin_assignment_id,
    }
}

pub fn new_role(name: &str, capability: CapabilityClass) -> RoleChoice {
    RoleChoice::New {
        name: RoleName::parse(name).unwrap(),
        capability,
    }
}

/// The FAU's admin invites `address` to one role, and `address` accepts.
pub async fn add_member(
    pool: &PgPool,
    fau: &Fau,
    address: &str,
    role: RoleChoice,
    period: Period,
    at: Moment,
) -> Accepted {
    let issued = issue_invitation(
        pool,
        IssueInvitation {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            recipient: email(address),
            roles: vec![OfferedRole { role, period }],
            handover_grant_id: None,
        },
        at,
    )
    .await
    .expect("issue");
    accept_invitation(
        pool,
        AcceptInvitation {
            token: issued.token.expose().to_owned(),
            acceptor: verified(address),
            admin_end_override: None,
        },
        at,
    )
    .await
    .expect("accept")
}

pub async fn count(pool: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.expect(sql)
}

pub async fn outbox_count(pool: &PgPool, template: &str, recipient: &str) -> i64 {
    sqlx::query_scalar("select count(*) from outbox where template = $1 and recipient_email = $2")
        .bind(template)
        .bind(recipient)
        .fetch_one(pool)
        .await
        .unwrap()
}

pub async fn audit_count(pool: &PgPool, action: &str) -> i64 {
    sqlx::query_scalar("select count(*) from audit_events where action = $1")
        .bind(action)
        .fetch_one(pool)
        .await
        .unwrap()
}
```

Create (or replace in full) `backend/crates/app/tests/invitations.rs`:

```rust
//! Invitations: issue, re-send, withdraw, accept (flow spec §5.1, §3.5; §10
//! "Invitations and requests").

mod common;
use common::membership::*;
use common::TestDb;
use fau_domain::membership::acceptance::AcceptanceRefusal;
use fau_domain::membership::rules::AdminEndError;
use fau_domain::membership::vocabulary::CapabilityClass;
use fau_persistence::membership::{
    accept_invitation, activate_tenant, create_pending_tenant, issue_invitation, resend_invitation,
    withdraw_invitation, AcceptInvitation, Activation, InvitationChange, IssueInvitation,
    IssuedInvitation, MembershipError, OfferedRole, RoleChoice,
};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

async fn invite(
    pool: &PgPool,
    fau: &Fau,
    actor: Uuid,
    recipient: &str,
    role: RoleChoice,
    t: fau_domain::time::Moment,
) -> Result<IssuedInvitation, MembershipError> {
    issue_invitation(
        pool,
        IssueInvitation {
            tenant_id: fau.tenant_id,
            actor_membership_id: actor,
            recipient: email(recipient),
            roles: vec![OfferedRole {
                role,
                period: period(day(2026, 9, 23), day(2027, 9, 1)),
            }],
            handover_grant_id: None,
        },
        t,
    )
    .await
}

fn accept(token: &str, acceptor: &str) -> AcceptInvitation {
    AcceptInvitation {
        token: token.to_owned(),
        acceptor: verified(acceptor),
        admin_end_override: None,
    }
}

#[tokio::test]
async fn an_admin_invites_and_the_recipient_accepts() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;

    let issued = invite(
        &pool,
        &fau,
        fau.admin_membership_id,
        "ny@example.test",
        new_role("Kasserer", CapabilityClass::Member),
        t0,
    )
    .await
    .unwrap();
    assert_eq!(issued.expires_at, "2026-10-07T10:00:00Z".parse().unwrap());

    // Only the hash is stored, and neither the outbox nor the audit log holds the token.
    let digest = Sha256::digest(issued.token.expose().as_bytes()).to_vec();
    assert_eq!(count(&pool, "select count(*) from invitations").await, 1);
    let stored: Vec<u8> = sqlx::query_scalar("select token_hash from invitations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, digest);
    let leaked: i64 = sqlx::query_scalar(
        "select (select count(*) from outbox where params::text like '%' || $1 || '%')
              + (select count(*) from audit_events where params::text like '%' || $1 || '%')",
    )
    .bind(issued.token.expose())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(leaked, 0);
    assert_eq!(
        outbox_count(&pool, "invitation.issued", "ny@example.test").await,
        1
    );
    assert_eq!(
        count(&pool, "select count(*) from memberships").await,
        1,
        "issuing grants nothing until the recipient accepts"
    );

    let accepted = accept_invitation(&pool, accept(issued.token.expose(), "ny@example.test"), t0)
        .await
        .unwrap();
    assert_eq!(accepted.tenant_id, fau.tenant_id);
    assert_eq!(accepted.assignment_ids.len(), 1);
    let granted_by: Option<Uuid> =
        sqlx::query_scalar("select granted_by from role_assignments where id = $1")
            .bind(accepted.assignment_ids[0])
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(granted_by, Some(fau.admin_membership_id));
    assert_eq!(audit_count(&pool, "invitation.accepted").await, 1);
}

#[tokio::test]
async fn only_an_admin_valid_today_may_invite() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let member = add_member(
        &pool,
        &fau,
        "medlem@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;

    let err = invite(
        &pool,
        &fau,
        member.membership_id,
        "ny@example.test",
        new_role("Medlem", CapabilityClass::Member),
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::NotAuthorized);
}

#[tokio::test]
async fn acceptance_refuses_an_expired_used_or_withdrawn_token() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let role = || new_role("Medlem", CapabilityClass::Member);

    let expired = invite(
        &pool,
        &fau,
        fau.admin_membership_id,
        "a@example.test",
        role(),
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        accept_invitation(
            &pool,
            accept(expired.token.expose(), "a@example.test"),
            at("2026-10-07T10:00:00Z")
        )
        .await
        .unwrap_err(),
        MembershipError::Acceptance(AcceptanceRefusal::Expired)
    );

    let used = invite(
        &pool,
        &fau,
        fau.admin_membership_id,
        "b@example.test",
        role(),
        t0,
    )
    .await
    .unwrap();
    accept_invitation(&pool, accept(used.token.expose(), "b@example.test"), t0)
        .await
        .unwrap();
    assert_eq!(
        accept_invitation(&pool, accept(used.token.expose(), "b@example.test"), t0)
            .await
            .unwrap_err(),
        MembershipError::Acceptance(AcceptanceRefusal::AlreadyAccepted)
    );

    let withdrawn = invite(
        &pool,
        &fau,
        fau.admin_membership_id,
        "c@example.test",
        role(),
        t0,
    )
    .await
    .unwrap();
    withdraw_invitation(
        &pool,
        InvitationChange {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            invitation_id: withdrawn.invitation_id,
        },
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        accept_invitation(
            &pool,
            accept(withdrawn.token.expose(), "c@example.test"),
            t0
        )
        .await
        .unwrap_err(),
        MembershipError::Acceptance(AcceptanceRefusal::Revoked)
    );
    assert_eq!(
        accept_invitation(&pool, accept(&"0".repeat(64), "c@example.test"), t0)
            .await
            .unwrap_err(),
        MembershipError::UnknownInvitation
    );
    assert_eq!(
        accept_invitation(&pool, accept("not-a-token", "c@example.test"), t0)
            .await
            .unwrap_err(),
        MembershipError::UnknownInvitation
    );
}

#[tokio::test]
async fn acceptance_refuses_a_different_address() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let issued = invite(
        &pool,
        &fau,
        fau.admin_membership_id,
        "ny@example.test",
        new_role("Medlem", CapabilityClass::Member),
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        accept_invitation(
            &pool,
            accept(issued.token.expose(), "annen@example.test"),
            t0
        )
        .await
        .unwrap_err(),
        MembershipError::Acceptance(AcceptanceRefusal::EmailMismatch)
    );
}

#[tokio::test]
async fn acceptance_refuses_a_frozen_or_closed_fau() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let issued = invite(
        &pool,
        &fau,
        fau.admin_membership_id,
        "ny@example.test",
        new_role("Medlem", CapabilityClass::Member),
        t0,
    )
    .await
    .unwrap();

    sqlx::query("update tenants set frozen_at = now() where id = $1")
        .bind(fau.tenant_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();
    assert_eq!(
        accept_invitation(&pool, accept(issued.token.expose(), "ny@example.test"), t0)
            .await
            .unwrap_err(),
        MembershipError::Acceptance(AcceptanceRefusal::TenantFrozen)
    );
    assert_eq!(
        invite(
            &pool,
            &fau,
            fau.admin_membership_id,
            "to@example.test",
            new_role("Medlem", CapabilityClass::Member),
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::TenantFrozen,
        "a frozen FAU issues no invitations (ADR-003 decision 7a)"
    );

    sqlx::query("update tenants set frozen_at = null, status = 'closed' where id = $1")
        .bind(fau.tenant_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();
    assert_eq!(
        accept_invitation(&pool, accept(issued.token.expose(), "ny@example.test"), t0)
            .await
            .unwrap_err(),
        MembershipError::Acceptance(AcceptanceRefusal::TenantNotActive)
    );
}

#[tokio::test]
async fn acceptance_refuses_when_the_issuer_has_lost_authority() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let issued = invite(
        &pool,
        &fau,
        fau.admin_membership_id,
        "ny@example.test",
        new_role("Medlem", CapabilityClass::Member),
        t0,
    )
    .await
    .unwrap();

    sqlx::query("update role_assignments set revoked_at = now() where id = $1")
        .bind(fau.admin_assignment_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();
    assert_eq!(
        accept_invitation(&pool, accept(issued.token.expose(), "ny@example.test"), t0)
            .await
            .unwrap_err(),
        MembershipError::Acceptance(AcceptanceRefusal::IssuerLacksAuthority)
    );
}

#[tokio::test]
async fn resending_replaces_the_token_and_restarts_the_fourteen_days() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let first = invite(
        &pool,
        &fau,
        fau.admin_membership_id,
        "ny@example.test",
        new_role("Medlem", CapabilityClass::Member),
        t0,
    )
    .await
    .unwrap();

    let t10 = at("2026-10-03T10:00:00Z");
    let second = resend_invitation(
        &pool,
        InvitationChange {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            invitation_id: first.invitation_id,
        },
        t10,
    )
    .await
    .unwrap();
    assert_eq!(second.invitation_id, first.invitation_id);
    assert_eq!(second.expires_at, "2026-10-17T10:00:00Z".parse().unwrap());
    assert_eq!(
        outbox_count(&pool, "invitation.issued", "ny@example.test").await,
        2
    );

    let later = at("2026-10-10T10:00:00Z");
    assert_eq!(
        accept_invitation(
            &pool,
            accept(first.token.expose(), "ny@example.test"),
            later
        )
        .await
        .unwrap_err(),
        MembershipError::UnknownInvitation,
        "the old token stops working"
    );
    accept_invitation(
        &pool,
        accept(second.token.expose(), "ny@example.test"),
        later,
    )
    .await
    .expect("the new token works past the old expiry");
}

#[tokio::test]
async fn the_issuer_or_any_admin_may_withdraw_and_nobody_else() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let member = add_member(
        &pool,
        &fau,
        "medlem@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    let issued = invite(
        &pool,
        &fau,
        fau.admin_membership_id,
        "ny@example.test",
        new_role("Medlem", CapabilityClass::Member),
        t0,
    )
    .await
    .unwrap();

    let change = |actor| InvitationChange {
        tenant_id: fau.tenant_id,
        actor_membership_id: actor,
        invitation_id: issued.invitation_id,
    };
    assert_eq!(
        withdraw_invitation(&pool, change(member.membership_id), t0)
            .await
            .unwrap_err(),
        MembershipError::NotAuthorized
    );
    withdraw_invitation(&pool, change(fau.admin_membership_id), t0)
        .await
        .unwrap();
    assert_eq!(
        withdraw_invitation(&pool, change(fau.admin_membership_id), t0)
            .await
            .unwrap_err(),
        MembershipError::InvitationNotPending
    );
    assert_eq!(audit_count(&pool, "invitation.withdrawn").await, 1);
}

#[tokio::test]
async fn the_leader_accepts_and_may_adjust_the_end_date_within_range() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let pending = create_pending_tenant(
        &pool,
        signup(Uuid::now_v7(), "reg@example.test", "leder@example.test", t0),
        t0,
    )
    .await
    .unwrap();
    let activated = activate_tenant(
        &pool,
        Activation {
            tenant_id: pending.tenant_id,
            registrant: verified("reg@example.test"),
        },
        t0,
    )
    .await
    .unwrap();
    let token = activated.leader_invitation.unwrap().token;
    let with_end = |end| AcceptInvitation {
        token: token.expose().to_owned(),
        acceptor: verified("leder@example.test"),
        admin_end_override: Some(end),
    };

    let t2 = at("2026-09-25T10:00:00Z");
    assert_eq!(
        accept_invitation(&pool, with_end(day(2028, 9, 26)), t2)
            .await
            .unwrap_err(),
        MembershipError::AdminEnd(AdminEndError::TooLate)
    );
    let accepted = accept_invitation(&pool, with_end(day(2028, 6, 1)), t2)
        .await
        .unwrap();
    let (starts, ends, granted_by_null): (String, String, bool) = sqlx::query_as(
        "select starts_on::text, ends_on_exclusive::text, granted_by is null
           from role_assignments where id = $1",
    )
    .bind(accepted.assignment_ids[0])
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (starts.as_str(), ends.as_str(), granted_by_null),
        ("2026-09-23", "2028-06-01", true)
    );
}

#[tokio::test]
async fn only_an_activation_invitation_accepts_an_end_date_change() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let issued = invite(
        &pool,
        &fau,
        fau.admin_membership_id,
        "ny@example.test",
        RoleChoice::Existing(fau.admin_role_id),
        t0,
    )
    .await
    .unwrap();
    let err = accept_invitation(
        &pool,
        AcceptInvitation {
            token: issued.token.expose().to_owned(),
            acceptor: verified("ny@example.test"),
            admin_end_override: Some(day(2027, 12, 1)),
        },
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::OverrideNotAllowed);
}

#[tokio::test]
async fn an_account_from_another_fau_is_reused() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let first = active_fau(&pool, "leder@example.test", t0).await;
    let second = active_fau(&pool, "admin@example.test", t0).await;
    let accepted = add_member(
        &pool,
        &second,
        "leder@example.test",
        RoleChoice::Existing(second.admin_role_id),
        period(day(2026, 9, 23), day(2027, 10, 1)),
        t0,
    )
    .await;
    assert_eq!(accepted.account_id, first.admin_account_id);
    assert_eq!(count(&pool, "select count(*) from accounts").await, 2);
}

#[tokio::test]
async fn a_revoked_member_who_is_invited_again_gets_the_same_membership_back() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let first = add_member(
        &pool,
        &fau,
        "tilbake@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    sqlx::query("update memberships set revoked_at = now() where id = $1")
        .bind(first.membership_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();

    let again = add_member(
        &pool,
        &fau,
        "tilbake@example.test",
        new_role("Sekretær", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    assert_eq!(again.membership_id, first.membership_id);
    let revoked: bool =
        sqlx::query_scalar("select revoked_at is not null from memberships where id = $1")
            .bind(again.membership_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!revoked);
}

#[tokio::test]
async fn offered_roles_are_validated() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let issue = |roles: Vec<OfferedRole>| {
        issue_invitation(
            &pool,
            IssueInvitation {
                tenant_id: fau.tenant_id,
                actor_membership_id: fau.admin_membership_id,
                recipient: email("ny@example.test"),
                roles,
                handover_grant_id: None,
            },
            t0,
        )
    };
    let current = period(day(2026, 9, 23), day(2027, 9, 1));

    assert_eq!(
        issue(vec![]).await.unwrap_err(),
        MembershipError::NoRolesOffered
    );
    assert_eq!(
        issue(vec![OfferedRole {
            role: RoleChoice::Existing(Uuid::now_v7()),
            period: current,
        }])
        .await
        .unwrap_err(),
        MembershipError::UnknownRole
    );
    assert_eq!(
        issue(vec![
            OfferedRole {
                role: RoleChoice::Existing(fau.admin_role_id),
                period: current,
            },
            OfferedRole {
                role: RoleChoice::Existing(fau.admin_role_id),
                period: current,
            },
        ])
        .await
        .unwrap_err(),
        MembershipError::DuplicateRole
    );
    assert_eq!(
        issue(vec![OfferedRole {
            role: RoleChoice::Existing(fau.admin_role_id),
            period: period(day(2025, 9, 1), day(2026, 9, 23)),
        }])
        .await
        .unwrap_err(),
        MembershipError::PeriodAlreadyEnded
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test -p fau-app --test invitations`
Expected: compile error: `issue_invitation`, `accept_invitation` and friends are not found in `fau_persistence::membership`.

- [ ] **Step 3: Implement**

Create (or replace in full) `backend/crates/persistence/src/membership/token.rs`:

```rust
//! Invitation tokens (spec 5.1): 32 bytes from the operating system's CSPRNG, shown as
//! 64 lower-case hex characters, stored only as the SHA-256 of that text. The raw token
//! is returned to the caller once, is never written to the database or the outbox, and
//! has a redacted `Debug` so it cannot reach a log line by accident.

use std::fmt;

use sha2::{Digest, Sha256};

use super::error::MembershipError;

/// A freshly generated invitation token. Deliberately neither `Clone` nor `Display`:
/// the only way to read it is [`InvitationToken::expose`], which is easy to grep for.
pub struct InvitationToken(String);

impl InvitationToken {
    pub(crate) fn generate() -> Result<Self, MembershipError> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|_| MembershipError::Randomness)?;
        Ok(Self(bytes.iter().map(|b| format!("{b:02x}")).collect()))
    }

    /// The raw token, for the link in the invitation email.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub(crate) fn hash(&self) -> Vec<u8> {
        hash_token(&self.0)
    }
}

impl fmt::Debug for InvitationToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("InvitationToken([redacted])")
    }
}

/// SHA-256 of the token's text, as stored in `invitations.token_hash`.
pub(crate) fn hash_token(raw: &str) -> Vec<u8> {
    Sha256::digest(raw.as_bytes()).to_vec()
}

/// Whether `raw` has the shape of a token this module generates. Anything else is
/// rejected before a database lookup.
pub(crate) fn looks_like_token(raw: &str) -> bool {
    raw.len() == 64
        && raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_64_lowercase_hex_characters_and_unique() {
        let a = InvitationToken::generate().unwrap();
        let b = InvitationToken::generate().unwrap();
        assert!(looks_like_token(a.expose()), "{}", a.expose().len());
        assert_ne!(a.expose(), b.expose());
    }

    #[test]
    fn the_hash_is_sha256_of_the_text() {
        // FIPS 180-2's "abc" test vector.
        let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let hex: String = hash_token("abc")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(hex, expected);
        let t = InvitationToken::generate().unwrap();
        assert_eq!(t.hash(), hash_token(t.expose()));
        assert_eq!(t.hash().len(), 32);
    }

    #[test]
    fn debug_is_redacted() {
        let t = InvitationToken::generate().unwrap();
        let debug = format!("{t:?}");
        assert_eq!(debug, "InvitationToken([redacted])");
        assert!(!debug.contains(t.expose()));
    }

    #[test]
    fn malformed_tokens_are_recognised() {
        assert!(!looks_like_token(""));
        assert!(!looks_like_token(&"A".repeat(64)), "upper case is not ours");
        assert!(!looks_like_token(&"a".repeat(63)));
        assert!(looks_like_token(&"a".repeat(64)));
    }
}
```

In `backend/crates/persistence/src/membership/sql.rs`, replace:

```rust
use fau_domain::membership::period::Period;
use fau_domain::membership::vocabulary::TenantStatus;
use fau_domain::time::Moment;
use jiff::civil::Date;
use jiff::Timestamp;
use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use super::error::MembershipError;
```

with:

```rust
use fau_domain::membership::access::{tenant_has_admin, AssignmentView, GrantView};
use fau_domain::membership::period::Period;
use fau_domain::membership::rules::{recent_holder_window_start, EWB_OVERSIGHT_ADDRESS};
use fau_domain::membership::vocabulary::{CapabilityClass, TenantStatus};
use fau_domain::time::Moment;
use jiff::civil::Date;
use jiff::Timestamp;
use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use super::error::MembershipError;
```

In `backend/crates/persistence/src/membership/sql.rs`, replace:

```rust
/// Who performed an audited action. Matches `audit_events.actor_kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActorKind {
    System,
    Registrant,
}

impl ActorKind {
    fn code(self) -> &'static str {
        match self {
            ActorKind::System => "system",
            ActorKind::Registrant => "registrant",
        }
    }
}
```

with:

```rust
/// Who performed an audited action. Matches `audit_events.actor_kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActorKind {
    Member,
    System,
    Registrant,
}

impl ActorKind {
    fn code(self) -> &'static str {
        match self {
            ActorKind::Member => "member",
            ActorKind::System => "system",
            ActorKind::Registrant => "registrant",
        }
    }
}
```

In `backend/crates/persistence/src/membership/sql.rs`, replace:

```rust
impl Audit {
```

with:

```rust
impl Audit {
    /// An entry by a member acting through their membership.
    pub(crate) fn member(
        tenant_id: Uuid,
        membership_id: Uuid,
        action: &'static str,
        subject_type: &'static str,
        subject_id: Uuid,
        params: Value,
    ) -> Self {
        Self {
            tenant_id: Some(tenant_id),
            actor_kind: ActorKind::Member,
            actor_account_id: None,
            actor_membership_id: Some(membership_id),
            action,
            subject_type,
            subject_id,
            params,
        }
    }
```

Append to the end of `backend/crates/persistence/src/membership/sql.rs`:

```rust

pub(crate) fn period_from(starts: &str, ends: &str) -> Result<Period, MembershipError> {
    Period::new(parse_date(starts)?, parse_date(ends)?).map_err(|_| MembershipError::decode())
}

/// Active and not frozen: the state in which invitations and requests may be created.
pub(crate) fn require_open(state: &TenantState) -> Result<(), MembershipError> {
    if state.status != TenantStatus::Active {
        return Err(MembershipError::TenantNotActive);
    }
    if state.frozen {
        return Err(MembershipError::TenantFrozen);
    }
    Ok(())
}

/// Whether `membership_id` holds an admin-class role valid today, through a membership
/// that is not revoked and an account that is verified and not disabled.
pub(crate) async fn is_admin_today(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
    today: Date,
) -> Result<bool, MembershipError> {
    Ok(sqlx::query_scalar(
        "select exists (
           select 1
             from memberships m
             join accounts a          on a.id = m.account_id
             join role_assignments ra on ra.tenant_id = m.tenant_id and ra.membership_id = m.id
             join roles r             on r.tenant_id = ra.tenant_id and r.id = ra.role_id
            where m.tenant_id = $1 and m.id = $2
              and m.revoked_at is null and a.disabled_at is null and a.verified_at is not null
              and ra.revoked_at is null and r.capability_class = 'admin'
              and ra.starts_on <= $3::date and ra.ends_on_exclusive > $3::date)",
    )
    .bind(tenant_id)
    .bind(membership_id)
    .bind(date_param(today))
    .fetch_one(&mut *conn)
    .await?)
}

pub(crate) async fn require_admin(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
    today: Date,
) -> Result<(), MembershipError> {
    if is_admin_today(conn, tenant_id, membership_id, today).await? {
        Ok(())
    } else {
        Err(MembershipError::NotAuthorized)
    }
}

/// Whether the membership is not revoked and its account is verified and not disabled.
pub(crate) async fn membership_usable(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
) -> Result<bool, MembershipError> {
    Ok(sqlx::query_scalar(
        "select exists (
           select 1 from memberships m join accounts a on a.id = m.account_id
            where m.tenant_id = $1 and m.id = $2
              and m.revoked_at is null and a.disabled_at is null and a.verified_at is not null)",
    )
    .bind(tenant_id)
    .bind(membership_id)
    .fetch_one(&mut *conn)
    .await?)
}

/// The account address behind a usable membership, or `None`.
pub(crate) async fn usable_member_email(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
) -> Result<Option<String>, MembershipError> {
    Ok(sqlx::query_scalar(
        "select a.email from memberships m join accounts a on a.id = m.account_id
          where m.tenant_id = $1 and m.id = $2
            and m.revoked_at is null and a.disabled_at is null and a.verified_at is not null",
    )
    .bind(tenant_id)
    .bind(membership_id)
    .fetch_optional(&mut *conn)
    .await?)
}

/// Whether `grant_id` is valid today and still belongs to `membership_id`, through a
/// membership that is not revoked and an account that is not disabled (spec 6.3).
pub(crate) async fn handover_grant_valid(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    grant_id: Uuid,
    membership_id: Uuid,
    today: Date,
) -> Result<bool, MembershipError> {
    Ok(sqlx::query_scalar(
        "select exists (
           select 1
             from handover_grants g
             join role_assignments ra on ra.tenant_id = g.tenant_id and ra.id = g.source_assignment_id
             join memberships m       on m.tenant_id = ra.tenant_id and m.id = ra.membership_id
             join accounts a          on a.id = m.account_id
            where g.tenant_id = $1 and g.id = $2 and ra.membership_id = $3
              and g.revoked_at is null
              and g.starts_on <= $4::date and g.ends_on_exclusive > $4::date
              and m.revoked_at is null and a.disabled_at is null)",
    )
    .bind(tenant_id)
    .bind(grant_id)
    .bind(membership_id)
    .bind(date_param(today))
    .fetch_one(&mut *conn)
    .await?)
}

/// Every admin-class assignment and handover grant held through a usable membership:
/// the inputs to the no-admin predicate (spec 6.4).
pub(crate) struct AdminState {
    assignments: Vec<AssignmentView>,
    grants: Vec<GrantView>,
}

impl AdminState {
    pub(crate) async fn load(
        conn: &mut PgConnection,
        tenant_id: Uuid,
    ) -> Result<Self, MembershipError> {
        let rows: Vec<(String, String, bool)> = sqlx::query_as(
            "select to_char(ra.starts_on, 'YYYY-MM-DD'), to_char(ra.ends_on_exclusive, 'YYYY-MM-DD'),
                    ra.revoked_at is not null
               from role_assignments ra
               join roles r       on r.tenant_id = ra.tenant_id and r.id = ra.role_id
               join memberships m on m.tenant_id = ra.tenant_id and m.id = ra.membership_id
               join accounts a    on a.id = m.account_id
              where ra.tenant_id = $1 and r.capability_class = 'admin'
                and m.revoked_at is null and a.disabled_at is null and a.verified_at is not null",
        )
        .bind(tenant_id)
        .fetch_all(&mut *conn)
        .await?;
        let mut assignments = Vec::with_capacity(rows.len());
        for (starts, ends, revoked) in rows {
            assignments.push(AssignmentView {
                capability: CapabilityClass::Admin,
                period: period_from(&starts, &ends)?,
                revoked,
            });
        }

        let rows: Vec<(String, String, bool)> = sqlx::query_as(
            "select to_char(g.starts_on, 'YYYY-MM-DD'), to_char(g.ends_on_exclusive, 'YYYY-MM-DD'),
                    g.revoked_at is not null
               from handover_grants g
               join role_assignments ra on ra.tenant_id = g.tenant_id and ra.id = g.source_assignment_id
               join memberships m       on m.tenant_id = ra.tenant_id and m.id = ra.membership_id
               join accounts a          on a.id = m.account_id
              where g.tenant_id = $1 and m.revoked_at is null and a.disabled_at is null",
        )
        .bind(tenant_id)
        .fetch_all(&mut *conn)
        .await?;
        let mut grants = Vec::with_capacity(rows.len());
        for (starts, ends, revoked) in rows {
            grants.push(GrantView {
                period: period_from(&starts, &ends)?,
                revoked,
            });
        }
        Ok(Self {
            assignments,
            grants,
        })
    }

    pub(crate) fn has_admin(&self, today: Date) -> bool {
        tenant_has_admin(&self.assignments, &self.grants, today)
    }
}

/// Addresses of every current member: a usable membership with any role valid today.
pub(crate) async fn current_member_emails(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    today: Date,
) -> Result<Vec<String>, MembershipError> {
    member_emails(conn, tenant_id, today, false).await
}

async fn member_emails(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    today: Date,
    admins_only: bool,
) -> Result<Vec<String>, MembershipError> {
    Ok(sqlx::query_scalar(
        "select distinct a.email
           from memberships m
           join accounts a          on a.id = m.account_id
           join role_assignments ra on ra.tenant_id = m.tenant_id and ra.membership_id = m.id
           join roles r             on r.tenant_id = ra.tenant_id and r.id = ra.role_id
          where m.tenant_id = $1
            and m.revoked_at is null and a.disabled_at is null and a.verified_at is not null
            and ra.revoked_at is null
            and ra.starts_on <= $2::date and ra.ends_on_exclusive > $2::date
            and ($3 = false or r.capability_class = 'admin')
          order by a.email",
    )
    .bind(tenant_id)
    .bind(date_param(today))
    .bind(admins_only)
    .fetch_all(&mut *conn)
    .await?)
}

/// Recovery notices (spec 6.4.4, ADR-003 decision 10): every current member; when none
/// remain, everyone who held a role in the past 24 months. EWB is always added, as the
/// notified second party for every recovery on every FAU (ADR-003 decision 8).
pub(crate) async fn recovery_notice_recipients(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    today: Date,
) -> Result<Vec<String>, MembershipError> {
    let mut recipients = current_member_emails(conn, tenant_id, today).await?;
    if recipients.is_empty() {
        recipients = sqlx::query_scalar(
            "select distinct a.email
               from role_assignments ra
               join memberships m on m.tenant_id = ra.tenant_id and m.id = ra.membership_id
               join accounts a    on a.id = m.account_id
              where ra.tenant_id = $1 and a.disabled_at is null
                and ra.starts_on <= $2::date and ra.ends_on_exclusive > $3::date
              order by a.email",
        )
        .bind(tenant_id)
        .bind(date_param(today))
        .bind(date_param(recent_holder_window_start(today)))
        .fetch_all(&mut *conn)
        .await?;
    }
    if !recipients.iter().any(|e| e == EWB_OVERSIGHT_ADDRESS) {
        recipients.push(EWB_OVERSIGHT_ADDRESS.to_owned());
    }
    Ok(recipients)
}
```

Create (or replace in full) `backend/crates/persistence/src/membership/invitations.rs`:

```rust
//! Invitations (spec 5.1): issue, re-send, withdraw and accept.

use std::collections::HashSet;

use fau_domain::email::{Email, VerifiedEmail};
use fau_domain::membership::acceptance::{check_acceptance, AcceptanceSnapshot};
use fau_domain::membership::period::Period;
use fau_domain::membership::rules::{invitation_expiry, validate_admin_end};
use fau_domain::membership::vocabulary::{
    CapabilityClass, InvitationMode, RecoveryHolder, RoleName,
};
use fau_domain::time::Moment;
use jiff::civil::Date;
use jiff::Timestamp;
use serde_json::json;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::error::MembershipError;
use super::sql::{
    date_param, enqueue, ensure_membership, from_micros, handover_grant_valid, insert_assignment,
    is_admin_today, lock_tenant, membership_usable, period_from, recovery_notice_recipients,
    require_admin, require_open, ts_param, upsert_verified_account, usable_member_email,
    write_audit, ActorKind, AdminState, Audit,
};
use super::token::{hash_token, looks_like_token, InvitationToken};

/// What an issuer offers: an existing role, or a new one created with the invitation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoleChoice {
    Existing(Uuid),
    New {
        name: RoleName,
        capability: CapabilityClass,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferedRole {
    pub role: RoleChoice,
    pub period: Period,
}

/// A newly issued (or re-sent) invitation. `token` is the only copy of the raw token.
#[derive(Debug)]
pub struct IssuedInvitation {
    pub invitation_id: Uuid,
    pub token: InvitationToken,
    pub expires_at: Timestamp,
}

pub(crate) struct NewInvitation {
    pub(crate) tenant_id: Uuid,
    pub(crate) mode: InvitationMode,
    pub(crate) recipient: Email,
    pub(crate) issued_by: Option<Uuid>,
    pub(crate) handover_grant_id: Option<Uuid>,
    pub(crate) access_request_id: Option<Uuid>,
    pub(crate) recovery_holder: Option<RecoveryHolder>,
    pub(crate) roles: Vec<(Uuid, Period)>,
    pub(crate) actor_kind: ActorKind,
    pub(crate) actor_membership_id: Option<Uuid>,
}

/// Writes an invitation, its roles, its audit entry and its outbox message. Every path
/// that issues an invitation -- activation, an admin, a handover grant, an approved
/// request, the recovery contact -- goes through here, inside the caller's transaction.
pub(crate) async fn insert_invitation(
    conn: &mut PgConnection,
    at: Moment,
    new: NewInvitation,
) -> Result<IssuedInvitation, MembershipError> {
    let token = InvitationToken::generate()?;
    let invitation_id = Uuid::now_v7();
    let expires_at = invitation_expiry(at.now());
    sqlx::query(
        "insert into invitations
           (tenant_id, id, token_hash, mode, recipient_email, issued_by, handover_grant_id,
            access_request_id, recovery_holder, expires_at, created_at)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::timestamptz, $11::timestamptz)",
    )
    .bind(new.tenant_id)
    .bind(invitation_id)
    .bind(token.hash())
    .bind(new.mode.code())
    .bind(new.recipient.as_str())
    .bind(new.issued_by)
    .bind(new.handover_grant_id)
    .bind(new.access_request_id)
    .bind(new.recovery_holder.map(RecoveryHolder::code))
    .bind(ts_param(expires_at))
    .bind(ts_param(at.now()))
    .execute(&mut *conn)
    .await?;
    for (role_id, period) in &new.roles {
        sqlx::query(
            "insert into invitation_roles
               (tenant_id, invitation_id, role_id, starts_on, ends_on_exclusive)
             values ($1, $2, $3, $4::date, $5::date)",
        )
        .bind(new.tenant_id)
        .bind(invitation_id)
        .bind(role_id)
        .bind(date_param(period.starts_on()))
        .bind(date_param(period.ends_on_exclusive()))
        .execute(&mut *conn)
        .await?;
    }
    write_audit(
        conn,
        at,
        Audit {
            tenant_id: Some(new.tenant_id),
            actor_kind: new.actor_kind,
            actor_account_id: None,
            actor_membership_id: new.actor_membership_id,
            action: "invitation.issued",
            subject_type: "invitation",
            subject_id: invitation_id,
            params: json!({ "mode": new.mode.code(), "role_count": new.roles.len() }),
        },
    )
    .await?;
    enqueue(
        conn,
        at,
        "invitation.issued",
        new.recipient.as_str(),
        json!({
            "tenant_id": new.tenant_id,
            "invitation_id": invitation_id,
            "mode": new.mode.code(),
        }),
    )
    .await?;
    Ok(IssuedInvitation {
        invitation_id,
        token,
        expires_at,
    })
}

/// Resolves offered roles to role ids, creating new ones, and rejects an empty list, a
/// duplicate, an unknown role or a period that has already ended.
pub(crate) async fn resolve_roles(
    conn: &mut PgConnection,
    at: Moment,
    tenant_id: Uuid,
    actor_membership_id: Option<Uuid>,
    offered: &[OfferedRole],
) -> Result<Vec<(Uuid, CapabilityClass, Period)>, MembershipError> {
    if offered.is_empty() {
        return Err(MembershipError::NoRolesOffered);
    }
    let mut seen = HashSet::new();
    let mut resolved = Vec::with_capacity(offered.len());
    for o in offered {
        if o.period.has_ended_by(at.today()) {
            return Err(MembershipError::PeriodAlreadyEnded);
        }
        let (role_id, capability) = match &o.role {
            RoleChoice::Existing(id) => {
                let class: Option<String> = sqlx::query_scalar(
                    "select capability_class from roles where tenant_id = $1 and id = $2",
                )
                .bind(tenant_id)
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?;
                let class = class.ok_or(MembershipError::UnknownRole)?;
                (
                    *id,
                    CapabilityClass::from_code(&class).ok_or_else(MembershipError::decode)?,
                )
            }
            RoleChoice::New { name, capability } => {
                let id = Uuid::now_v7();
                sqlx::query(
                    "insert into roles (tenant_id, id, name, capability_class) values ($1, $2, $3, $4)",
                )
                .bind(tenant_id)
                .bind(id)
                .bind(name.as_str())
                .bind(capability.code())
                .execute(&mut *conn)
                .await?;
                write_audit(
                    conn,
                    at,
                    Audit {
                        tenant_id: Some(tenant_id),
                        actor_kind: if actor_membership_id.is_some() {
                            ActorKind::Member
                        } else {
                            ActorKind::System
                        },
                        actor_account_id: None,
                        actor_membership_id,
                        action: "role.created",
                        subject_type: "role",
                        subject_id: id,
                        params: json!({ "capability_class": capability.code() }),
                    },
                )
                .await?;
                (id, *capability)
            }
        };
        if !seen.insert(role_id) {
            return Err(MembershipError::DuplicateRole);
        }
        resolved.push((role_id, capability, o.period));
    }
    Ok(resolved)
}

#[derive(Debug, Clone)]
pub struct IssueInvitation {
    pub tenant_id: Uuid,
    pub actor_membership_id: Uuid,
    pub recipient: Email,
    pub roles: Vec<OfferedRole>,
    /// `Some` issues a `handover` invitation under the actor's own handover grant
    /// (spec 6.3); `None` issues a `normal` one and requires an admin role valid today.
    pub handover_grant_id: Option<Uuid>,
}

/// Issues an invitation (spec 5.1, 6.3).
///
/// **Gate:** the caller must already have passed #3414's second-factor and freshness
/// gate; the HTTP layer enforces it (#3417). This function verifies authority in the
/// database -- an admin role valid today, or the actor's own handover grant valid today
/// -- and nothing about the session.
///
/// Under a handover grant the actor may offer only existing roles (creating a role
/// edits the organisation, which handover does not allow) and may not invite their own
/// address (no extending one's own role).
pub async fn issue_invitation(
    pool: &PgPool,
    req: IssueInvitation,
    at: Moment,
) -> Result<IssuedInvitation, MembershipError> {
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, req.tenant_id).await?;
    require_open(&state)?;

    let mode = match req.handover_grant_id {
        None => {
            require_admin(&mut tx, req.tenant_id, req.actor_membership_id, at.today()).await?;
            InvitationMode::Normal
        }
        Some(grant_id) => {
            let valid = handover_grant_valid(
                &mut tx,
                req.tenant_id,
                grant_id,
                req.actor_membership_id,
                at.today(),
            )
            .await?;
            if !valid {
                return Err(MembershipError::NotAuthorized);
            }
            if req
                .roles
                .iter()
                .any(|r| matches!(r.role, RoleChoice::New { .. }))
            {
                return Err(MembershipError::NotAuthorized);
            }
            let own = usable_member_email(&mut tx, req.tenant_id, req.actor_membership_id)
                .await?
                .ok_or(MembershipError::NotAuthorized)?;
            if own == req.recipient.as_str() {
                return Err(MembershipError::SelfInvitation);
            }
            InvitationMode::Handover
        }
    };

    let roles = resolve_roles(
        &mut tx,
        at,
        req.tenant_id,
        Some(req.actor_membership_id),
        &req.roles,
    )
    .await?;
    let issued = insert_invitation(
        &mut tx,
        at,
        NewInvitation {
            tenant_id: req.tenant_id,
            mode,
            recipient: req.recipient,
            issued_by: Some(req.actor_membership_id),
            handover_grant_id: req.handover_grant_id,
            access_request_id: None,
            recovery_holder: None,
            roles: roles.into_iter().map(|(id, _, p)| (id, p)).collect(),
            actor_kind: ActorKind::Member,
            actor_membership_id: Some(req.actor_membership_id),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(issued)
}

struct InvitationRow {
    mode: InvitationMode,
    recipient_email: String,
    issued_by: Option<Uuid>,
    handover_grant_id: Option<Uuid>,
    expires_at: Timestamp,
    accepted: bool,
    revoked: bool,
}

async fn lock_invitation(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    invitation_id: Uuid,
    token_hash: Option<&[u8]>,
) -> Result<Option<InvitationRow>, MembershipError> {
    type Row = (String, String, Option<Uuid>, Option<Uuid>, i64, bool, bool);
    let row: Option<Row> = sqlx::query_as(
        "select mode, recipient_email, issued_by, handover_grant_id,
                (extract(epoch from expires_at) * 1000000)::bigint,
                accepted_at is not null, revoked_at is not null
           from invitations
          where tenant_id = $1 and id = $2 and ($3::bytea is null or token_hash = $3)
          for update",
    )
    .bind(tenant_id)
    .bind(invitation_id)
    .bind(token_hash)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((mode, recipient_email, issued_by, handover_grant_id, expires_us, accepted, revoked)) =
        row
    else {
        return Ok(None);
    };
    Ok(Some(InvitationRow {
        mode: InvitationMode::from_code(&mode).ok_or_else(MembershipError::decode)?,
        recipient_email,
        issued_by,
        handover_grant_id,
        expires_at: from_micros(expires_us)?,
        accepted,
        revoked,
    }))
}

/// Re-sending or withdrawing a pending invitation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvitationChange {
    pub tenant_id: Uuid,
    pub actor_membership_id: Uuid,
    pub invitation_id: Uuid,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChangeKind {
    Resend,
    Withdraw,
}

/// Any admin valid today may re-send or withdraw any pending invitation. Otherwise the
/// issuer may: a handover issuer while their grant is valid (spec 6.3), a normal issuer
/// only to withdraw (spec 5.1: "the issuer or any admin can withdraw").
async fn authorize_change(
    conn: &mut PgConnection,
    change: &InvitationChange,
    inv: &InvitationRow,
    today: Date,
    kind: ChangeKind,
) -> Result<(), MembershipError> {
    let (tenant_id, actor) = (change.tenant_id, change.actor_membership_id);
    if is_admin_today(conn, tenant_id, actor, today).await? {
        return Ok(());
    }
    if inv.issued_by != Some(actor) {
        return Err(MembershipError::NotAuthorized);
    }
    let allowed = match (inv.mode, inv.handover_grant_id) {
        (InvitationMode::Handover, Some(grant_id)) => {
            handover_grant_valid(conn, tenant_id, grant_id, actor, today).await?
        }
        (InvitationMode::Normal, _) => {
            kind == ChangeKind::Withdraw && membership_usable(conn, tenant_id, actor).await?
        }
        _ => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(MembershipError::NotAuthorized)
    }
}

/// Re-sends a pending invitation: a new token replaces the old one, which stops
/// working at once, and the 14 days restart (spec 5.1).
///
/// **Gate:** as [`issue_invitation`].
pub async fn resend_invitation(
    pool: &PgPool,
    change: InvitationChange,
    at: Moment,
) -> Result<IssuedInvitation, MembershipError> {
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, change.tenant_id).await?;
    require_open(&state)?;
    let inv = lock_invitation(&mut tx, change.tenant_id, change.invitation_id, None)
        .await?
        .ok_or(MembershipError::UnknownInvitation)?;
    if inv.accepted || inv.revoked {
        return Err(MembershipError::InvitationNotPending);
    }
    authorize_change(&mut tx, &change, &inv, at.today(), ChangeKind::Resend).await?;

    let token = InvitationToken::generate()?;
    let expires_at = invitation_expiry(at.now());
    sqlx::query(
        "update invitations set token_hash = $3, expires_at = $4::timestamptz
          where tenant_id = $1 and id = $2",
    )
    .bind(change.tenant_id)
    .bind(change.invitation_id)
    .bind(token.hash())
    .bind(ts_param(expires_at))
    .execute(&mut *tx)
    .await?;
    write_audit(
        &mut tx,
        at,
        Audit::member(
            change.tenant_id,
            change.actor_membership_id,
            "invitation.resent",
            "invitation",
            change.invitation_id,
            json!({ "mode": inv.mode.code() }),
        ),
    )
    .await?;
    enqueue(
        &mut tx,
        at,
        "invitation.issued",
        &inv.recipient_email,
        json!({
            "tenant_id": change.tenant_id,
            "invitation_id": change.invitation_id,
            "mode": inv.mode.code(),
        }),
    )
    .await?;
    tx.commit().await?;
    Ok(IssuedInvitation {
        invitation_id: change.invitation_id,
        token,
        expires_at,
    })
}

/// Withdraws a pending invitation (spec 5.1). Allowed while the FAU is frozen: it only
/// ever removes a way in.
///
/// **Gate:** as [`issue_invitation`].
pub async fn withdraw_invitation(
    pool: &PgPool,
    change: InvitationChange,
    at: Moment,
) -> Result<(), MembershipError> {
    let mut tx = pool.begin().await?;
    lock_tenant(&mut tx, change.tenant_id).await?;
    let inv = lock_invitation(&mut tx, change.tenant_id, change.invitation_id, None)
        .await?
        .ok_or(MembershipError::UnknownInvitation)?;
    if inv.accepted || inv.revoked {
        return Err(MembershipError::InvitationNotPending);
    }
    authorize_change(&mut tx, &change, &inv, at.today(), ChangeKind::Withdraw).await?;

    sqlx::query(
        "update invitations set revoked_at = $3::timestamptz where tenant_id = $1 and id = $2",
    )
    .bind(change.tenant_id)
    .bind(change.invitation_id)
    .bind(ts_param(at.now()))
    .execute(&mut *tx)
    .await?;
    write_audit(
        &mut tx,
        at,
        Audit::member(
            change.tenant_id,
            change.actor_membership_id,
            "invitation.withdrawn",
            "invitation",
            change.invitation_id,
            json!({ "mode": inv.mode.code() }),
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct AcceptInvitation {
    /// The raw token from the link.
    pub token: String,
    /// The address the person just verified with a passcode (#3417).
    pub acceptor: VerifiedEmail,
    /// The leader's own choice of end date for the admin role an `activation`
    /// invitation offers (spec 3.4.3, 3.5), within the same 1–24 month range. Refused on
    /// any other mode.
    pub admin_end_override: Option<Date>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accepted {
    pub tenant_id: Uuid,
    pub account_id: Uuid,
    pub membership_id: Uuid,
    pub assignment_ids: Vec<Uuid>,
}

/// Accepts an invitation in one transaction (spec 5.1): every check in
/// `fau_domain::membership::acceptance`, then the account and membership (created or
/// reused), the role assignments, the acceptance mark and the audit entry. A recovery
/// invitation also notifies every current member (spec 6.4.4).
///
/// Opening the link calls nothing here; only the explicit "Bli med" does.
pub async fn accept_invitation(
    pool: &PgPool,
    req: AcceptInvitation,
    at: Moment,
) -> Result<Accepted, MembershipError> {
    if !looks_like_token(&req.token) {
        return Err(MembershipError::UnknownInvitation);
    }
    let hash = hash_token(&req.token);
    let mut tx = pool.begin().await?;

    let found: Option<(Uuid, Uuid)> =
        sqlx::query_as("select tenant_id, id from invitations where token_hash = $1")
            .bind(&hash)
            .fetch_optional(&mut *tx)
            .await?;
    let (tenant_id, invitation_id) = found.ok_or(MembershipError::UnknownInvitation)?;
    // Lock order everywhere is tenant, then invitation. The hash is re-checked under the
    // lock, in case a re-send replaced it between the two reads.
    let state = lock_tenant(&mut tx, tenant_id).await?;
    let inv = lock_invitation(&mut tx, tenant_id, invitation_id, Some(&hash))
        .await?
        .ok_or(MembershipError::UnknownInvitation)?;

    let acceptor_email = req.acceptor.email().as_str().to_owned();
    let account_disabled: Option<bool> =
        sqlx::query_scalar("select disabled_at is not null from accounts where email = $1")
            .bind(&acceptor_email)
            .fetch_optional(&mut *tx)
            .await?;

    let issuer_admin_today = match (inv.mode, inv.issued_by) {
        (InvitationMode::Normal, Some(issuer)) => {
            is_admin_today(&mut tx, tenant_id, issuer, at.today()).await?
        }
        _ => false,
    };
    let handover_grant_valid_today = match (inv.mode, inv.handover_grant_id, inv.issued_by) {
        (InvitationMode::Handover, Some(grant_id), Some(issuer)) => {
            handover_grant_valid(&mut tx, tenant_id, grant_id, issuer, at.today()).await?
        }
        _ => false,
    };
    let tenant_has_admin_today = if inv.mode == InvitationMode::Recovery {
        AdminState::load(&mut tx, tenant_id)
            .await?
            .has_admin(at.today())
    } else {
        true
    };

    let snapshot = AcceptanceSnapshot {
        mode: inv.mode,
        expires_at: inv.expires_at,
        accepted: inv.accepted,
        revoked: inv.revoked,
        recipient: Email::parse(&inv.recipient_email).map_err(|_| MembershipError::decode())?,
        acceptor: req.acceptor.clone(),
        acceptor_account_disabled: account_disabled.unwrap_or(false),
        tenant_status: state.status,
        tenant_frozen: state.frozen,
        issuer_admin_today,
        handover_grant_valid_today,
        tenant_has_admin_today,
    };
    check_acceptance(&snapshot, at.now()).map_err(MembershipError::Acceptance)?;

    let rows: Vec<(Uuid, String, String, String)> = sqlx::query_as(
        "select ir.role_id, r.capability_class,
                to_char(ir.starts_on, 'YYYY-MM-DD'), to_char(ir.ends_on_exclusive, 'YYYY-MM-DD')
           from invitation_roles ir
           join roles r on r.tenant_id = ir.tenant_id and r.id = ir.role_id
          where ir.tenant_id = $1 and ir.invitation_id = $2
          order by ir.role_id",
    )
    .bind(tenant_id)
    .bind(invitation_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut roles = Vec::with_capacity(rows.len());
    for (role_id, class, starts, ends) in rows {
        let class = CapabilityClass::from_code(&class).ok_or_else(MembershipError::decode)?;
        roles.push((role_id, class, period_from(&starts, &ends)?));
    }

    if let Some(end) = req.admin_end_override {
        if inv.mode != InvitationMode::Activation {
            return Err(MembershipError::OverrideNotAllowed);
        }
        validate_admin_end(at.today(), end).map_err(MembershipError::AdminEnd)?;
        for (_, class, period) in roles.iter_mut() {
            if *class == CapabilityClass::Admin {
                *period = Period::new(period.starts_on(), end)
                    .map_err(|_| MembershipError::EmptyPeriod)?;
            }
        }
    }

    // Recovery notices go to the members as they stood before this acceptance.
    let recovery_recipients = if inv.mode == InvitationMode::Recovery {
        recovery_notice_recipients(&mut tx, tenant_id, at.today()).await?
    } else {
        Vec::new()
    };

    let (account_id, _) = upsert_verified_account(&mut tx, &acceptor_email, at).await?;
    let (membership_id, reused) = ensure_membership(&mut tx, tenant_id, account_id).await?;
    let mut assignment_ids = Vec::with_capacity(roles.len());
    for (role_id, _, period) in &roles {
        assignment_ids.push(
            insert_assignment(
                &mut tx,
                tenant_id,
                membership_id,
                *role_id,
                *period,
                inv.issued_by,
            )
            .await?,
        );
    }
    sqlx::query(
        "update invitations set accepted_at = $3::timestamptz, accepted_membership_id = $4
          where tenant_id = $1 and id = $2",
    )
    .bind(tenant_id)
    .bind(invitation_id)
    .bind(ts_param(at.now()))
    .bind(membership_id)
    .execute(&mut *tx)
    .await?;
    write_audit(
        &mut tx,
        at,
        Audit {
            tenant_id: Some(tenant_id),
            actor_kind: ActorKind::Member,
            actor_account_id: Some(account_id),
            actor_membership_id: Some(membership_id),
            action: "invitation.accepted",
            subject_type: "invitation",
            subject_id: invitation_id,
            params: json!({
                "mode": inv.mode.code(),
                "membership_reused": reused,
                "assignment_count": assignment_ids.len(),
            }),
        },
    )
    .await?;
    for recipient in &recovery_recipients {
        enqueue(
            &mut tx,
            at,
            "recovery.invitation_accepted",
            recipient,
            json!({ "tenant_id": tenant_id, "invitation_id": invitation_id }),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Accepted {
        tenant_id,
        account_id,
        membership_id,
        assignment_ids,
    })
}
```

Create (or replace in full) `backend/crates/persistence/src/membership/mod.rs`:

```rust
//! The membership foundation's transactions (#3413 flow spec; #3417, #3418). Each public
//! function is one transaction that writes its state change together with its audit
//! entry and, where the spec says so, its outbox message (spec 2.6).
//!
//! Every function takes a [`fau_domain::time::Moment`]: rule-deciding timestamps and
//! "today" come from the caller, never from the database clock.

mod error;
mod invitations;
mod signup;
mod sql;
mod token;

pub use error::{ExistingFau, MembershipError};
pub use invitations::{
    accept_invitation, issue_invitation, resend_invitation, withdraw_invitation, AcceptInvitation,
    Accepted, InvitationChange, IssueInvitation, IssuedInvitation, OfferedRole, RoleChoice,
};
pub use signup::{
    activate_tenant, create_pending_tenant, expire_pending_tenants, Activated, Activation,
    PendingSignup, PendingTenant,
};
pub use token::InvitationToken;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test --workspace`
Expected: PASS, and `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings` are clean.

- [ ] **Step 5: Commit** (the controller commits; executors do not)

```bash
git add backend/crates/persistence/src/membership backend/crates/app/Cargo.toml backend/crates/app/tests/common/membership.rs backend/crates/app/tests/invitations.rs
git commit -m "Add invitation issue, re-send, withdrawal and acceptance (#3413, #3418)"
```

---

### Task 7: Persistence: access requests and replacement proposals

One table, one approval path (spec §5.4). An access request is created by a passcode-verified address and emails every current admin without the requester learning who they are; a replacement proposal names one of the proposer's own roles held today and counts against the proposer's address. Approval issues a normal invitation linked to the request, with the roles the admin chooses; decline and the 30-day lapse tell the requester only that it was not approved. The tenant row lock serialises the limit checks, and the partial unique index from Task 1 backs the one-open-request limit.

**Files:**
- Modify: `backend/crates/persistence/src/membership/sql.rs`
- Create: `backend/crates/persistence/src/membership/requests.rs`
- Replace in full: `backend/crates/persistence/src/membership/mod.rs`
- Create: `backend/crates/app/tests/requests.rs`

**Interfaces:**
- Consumes: `resolve_roles`, `insert_invitation`, `require_admin`, `usable_member_email`; domain request rules.
- Produces:
  - `create_access_request(&PgPool, CreateAccessRequest, Moment) -> Result<Uuid, _>`; `CreateAccessRequest { tenant_id, requester: VerifiedEmail, message: Option<String> }`.
  - `create_replacement_proposal(&PgPool, CreateReplacementProposal, Moment) -> Result<Uuid, _>`; `CreateReplacementProposal { tenant_id, proposer_membership_id, replaced_assignment_id, successor: Email, starts_on, ends_on_exclusive, message }`.
  - `approve_request(&PgPool, RequestDecision, Vec<OfferedRole>, Moment) -> Result<IssuedInvitation, _>`, `decline_request(&PgPool, RequestDecision, Moment) -> Result<(), _>`; `RequestDecision { tenant_id, actor_membership_id, request_id }`.
  - `lapse_requests(&PgPool, Moment) -> Result<u64, _>`.
  - Crate-private: `sql::admin_emails`; `ActorKind::Requester`.

- [ ] **Step 1: Write the failing tests**

Create (or replace in full) `backend/crates/app/tests/requests.rs`:

```rust
//! Access requests and replacement proposals (flow spec §5.2–5.4).

mod common;
use common::membership::*;
use common::TestDb;
use fau_domain::membership::requests::{ReplacementDateError, RequestLimit};
use fau_domain::membership::vocabulary::CapabilityClass;
use fau_persistence::membership::{
    accept_invitation, approve_request, create_access_request, create_replacement_proposal,
    decline_request, lapse_requests, AcceptInvitation, CreateAccessRequest,
    CreateReplacementProposal, MembershipError, OfferedRole, RequestDecision, RoleChoice,
};
use sqlx::PgPool;
use uuid::Uuid;

fn access(fau: &Fau, requester: &str, message: Option<&str>) -> CreateAccessRequest {
    CreateAccessRequest {
        tenant_id: fau.tenant_id,
        requester: verified(requester),
        message: message.map(str::to_owned),
    }
}

async fn status(pool: &PgPool, request_id: Uuid) -> String {
    sqlx::query_scalar("select status from access_requests where id = $1")
        .bind(request_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn an_access_request_reaches_the_admins_and_names_nobody() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;

    let id = create_access_request(&pool, access(&fau, "ny@example.test", Some(" Hei! ")), t0)
        .await
        .unwrap();

    assert_eq!(status(&pool, id).await, "pending");
    assert_eq!(
        outbox_count(&pool, "request.received", "admin@example.test").await,
        1
    );
    let message: Option<String> =
        sqlx::query_scalar("select message from access_requests where id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(message.as_deref(), Some("Hei!"));
    let in_audit: i64 =
        sqlx::query_scalar("select count(*) from audit_events where params::text like '%Hei%'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(in_audit, 0, "free text never reaches audit parameters");
}

#[tokio::test]
async fn one_open_request_per_address_and_five_per_fau_per_day() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;

    create_access_request(&pool, access(&fau, "a@example.test", None), t0)
        .await
        .unwrap();
    assert_eq!(
        create_access_request(&pool, access(&fau, "a@example.test", None), t0)
            .await
            .unwrap_err(),
        MembershipError::RequestLimit(RequestLimit::OpenRequestExists)
    );
    for n in 0..4 {
        create_access_request(&pool, access(&fau, &format!("b{n}@example.test"), None), t0)
            .await
            .unwrap();
    }
    assert_eq!(
        create_access_request(&pool, access(&fau, "c@example.test", None), t0)
            .await
            .unwrap_err(),
        MembershipError::RequestLimit(RequestLimit::TenantDailyLimit)
    );
    let tomorrow = at("2026-09-24T10:00:00Z");
    create_access_request(&pool, access(&fau, "c@example.test", None), tomorrow)
        .await
        .expect("the daily limit resets on the next Oslo date");
}

#[tokio::test]
async fn the_message_is_at_most_five_hundred_characters() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let long = "ø".repeat(501);
    assert_eq!(
        create_access_request(&pool, access(&fau, "a@example.test", Some(&long)), t0)
            .await
            .unwrap_err(),
        MembershipError::MessageTooLong
    );
}

#[tokio::test]
async fn approving_issues_a_normal_invitation_linked_to_the_request() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let request_id = create_access_request(&pool, access(&fau, "ny@example.test", None), t0)
        .await
        .unwrap();

    let issued = approve_request(
        &pool,
        RequestDecision {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            request_id,
        },
        vec![OfferedRole {
            role: new_role("Medlem", CapabilityClass::Member),
            period: period(day(2026, 9, 23), day(2027, 9, 1)),
        }],
        t0,
    )
    .await
    .unwrap();

    assert_eq!(status(&pool, request_id).await, "approved");
    let (mode, link): (String, Option<Uuid>) =
        sqlx::query_as("select mode, access_request_id from invitations where id = $1")
            .bind(issued.invitation_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((mode.as_str(), link), ("normal", Some(request_id)));
    accept_invitation(
        &pool,
        AcceptInvitation {
            token: issued.token.expose().to_owned(),
            acceptor: verified("ny@example.test"),
            admin_end_override: None,
        },
        t0,
    )
    .await
    .unwrap();
    assert_eq!(audit_count(&pool, "request.approved").await, 1);
}

#[tokio::test]
async fn declining_tells_the_requester_and_nothing_more() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let request_id = create_access_request(&pool, access(&fau, "ny@example.test", None), t0)
        .await
        .unwrap();
    let decision = RequestDecision {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        request_id,
    };
    decline_request(&pool, decision.clone(), t0).await.unwrap();

    assert_eq!(status(&pool, request_id).await, "declined");
    assert_eq!(
        outbox_count(&pool, "request.declined", "ny@example.test").await,
        1
    );
    let params: String =
        sqlx::query_scalar("select params::text from outbox where template = 'request.declined'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!params.contains(&fau.admin_membership_id.to_string()));
    assert_eq!(
        decline_request(&pool, decision, t0).await.unwrap_err(),
        MembershipError::RequestNotPending
    );
}

#[tokio::test]
async fn only_an_admin_decides() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let member = add_member(
        &pool,
        &fau,
        "medlem@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    let request_id = create_access_request(&pool, access(&fau, "ny@example.test", None), t0)
        .await
        .unwrap();
    let err = decline_request(
        &pool,
        RequestDecision {
            tenant_id: fau.tenant_id,
            actor_membership_id: member.membership_id,
            request_id,
        },
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::NotAuthorized);
}

#[tokio::test]
async fn an_unhandled_request_lapses_after_thirty_days() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let request_id = create_access_request(&pool, access(&fau, "ny@example.test", None), t0)
        .await
        .unwrap();

    assert_eq!(
        lapse_requests(&pool, at("2026-10-22T10:00:00Z"))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        lapse_requests(&pool, at("2026-10-23T10:00:00Z"))
            .await
            .unwrap(),
        1
    );
    assert_eq!(status(&pool, request_id).await, "lapsed");
    assert_eq!(
        outbox_count(&pool, "request.lapsed", "ny@example.test").await,
        1
    );
    assert_eq!(audit_count(&pool, "request.lapsed").await, 1);
}

#[tokio::test]
async fn a_member_proposes_a_successor_for_their_own_role() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let kasserer = add_member(
        &pool,
        &fau,
        "kasserer@example.test",
        new_role("Kasserer", CapabilityClass::Member),
        period(day(2026, 9, 1), day(2027, 9, 1)),
        t0,
    )
    .await;
    let proposal = |starts, ends| CreateReplacementProposal {
        tenant_id: fau.tenant_id,
        proposer_membership_id: kasserer.membership_id,
        replaced_assignment_id: kasserer.assignment_ids[0],
        successor: email("etterfolger@example.test"),
        starts_on: starts,
        ends_on_exclusive: ends,
        message: None,
    };

    assert_eq!(
        create_replacement_proposal(&pool, proposal(day(2026, 9, 22), day(2028, 9, 1)), t0)
            .await
            .unwrap_err(),
        MembershipError::ReplacementDates(ReplacementDateError::StartsInPast)
    );
    let request_id =
        create_replacement_proposal(&pool, proposal(day(2027, 9, 1), day(2028, 9, 1)), t0)
            .await
            .unwrap();
    assert_eq!(
        create_replacement_proposal(&pool, proposal(day(2027, 9, 1), day(2028, 9, 1)), t0)
            .await
            .unwrap_err(),
        MembershipError::RequestLimit(RequestLimit::OpenRequestExists),
        "a proposal counts against its proposer"
    );
    let still_held: bool =
        sqlx::query_scalar("select revoked_at is null from role_assignments where id = $1")
            .bind(kasserer.assignment_ids[0])
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(still_held, "proposing does not end the proposer's role");

    // The role is the proposer's own, so the successor holds the same position.
    let role_id: Uuid = sqlx::query_scalar("select role_id from role_assignments where id = $1")
        .bind(kasserer.assignment_ids[0])
        .fetch_one(&pool)
        .await
        .unwrap();
    let issued = approve_request(
        &pool,
        RequestDecision {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            request_id,
        },
        vec![OfferedRole {
            role: RoleChoice::Existing(role_id),
            period: period(day(2027, 9, 1), day(2028, 9, 1)),
        }],
        t0,
    )
    .await
    .unwrap();
    let recipient: String =
        sqlx::query_scalar("select recipient_email from invitations where id = $1")
            .bind(issued.invitation_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(recipient, "etterfolger@example.test");
}

#[tokio::test]
async fn a_member_may_propose_only_for_a_role_they_hold_today() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let future = add_member(
        &pool,
        &fau,
        "snart@example.test",
        new_role("Sekretær", CapabilityClass::Member),
        period(day(2026, 10, 1), day(2027, 10, 1)),
        t0,
    )
    .await;
    let other = add_member(
        &pool,
        &fau,
        "annen@example.test",
        new_role("Kasserer", CapabilityClass::Member),
        period(day(2026, 9, 1), day(2027, 9, 1)),
        t0,
    )
    .await;
    let proposal = |proposer: Uuid, assignment: Uuid| CreateReplacementProposal {
        tenant_id: fau.tenant_id,
        proposer_membership_id: proposer,
        replaced_assignment_id: assignment,
        successor: email("etterfolger@example.test"),
        starts_on: day(2027, 10, 1),
        ends_on_exclusive: day(2028, 10, 1),
        message: None,
    };

    assert_eq!(
        create_replacement_proposal(
            &pool,
            proposal(future.membership_id, future.assignment_ids[0]),
            t0
        )
        .await
        .unwrap_err(),
        MembershipError::RoleNotHeldToday
    );
    assert_eq!(
        create_replacement_proposal(
            &pool,
            proposal(future.membership_id, other.assignment_ids[0]),
            t0
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized
    );
}

#[tokio::test]
async fn a_frozen_fau_takes_no_requests() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    sqlx::query("update tenants set frozen_at = now() where id = $1")
        .bind(fau.tenant_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();
    assert_eq!(
        create_access_request(&pool, access(&fau, "ny@example.test", None), t0)
            .await
            .unwrap_err(),
        MembershipError::TenantFrozen
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test -p fau-app --test requests`
Expected: compile error: `create_access_request` and friends are not found.

- [ ] **Step 3: Implement**

In `backend/crates/persistence/src/membership/sql.rs`, replace:

```rust
/// Who performed an audited action. Matches `audit_events.actor_kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActorKind {
    Member,
    System,
    Registrant,
}

impl ActorKind {
    fn code(self) -> &'static str {
        match self {
            ActorKind::Member => "member",
            ActorKind::System => "system",
            ActorKind::Registrant => "registrant",
        }
    }
}
```

with:

```rust
/// Who performed an audited action. Matches `audit_events.actor_kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActorKind {
    Member,
    System,
    Registrant,
    Requester,
}

impl ActorKind {
    fn code(self) -> &'static str {
        match self {
            ActorKind::Member => "member",
            ActorKind::System => "system",
            ActorKind::Registrant => "registrant",
            ActorKind::Requester => "requester",
        }
    }
}
```

Append to the end of `backend/crates/persistence/src/membership/sql.rs`:

```rust

/// Addresses of everyone holding an admin-class role valid today.
pub(crate) async fn admin_emails(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    today: Date,
) -> Result<Vec<String>, MembershipError> {
    member_emails(conn, tenant_id, today, true).await
}
```

Create (or replace in full) `backend/crates/persistence/src/membership/requests.rs`:

```rust
//! Access requests and replacement proposals (spec 5.2–5.4): one table, one approval
//! path, the same statuses. The requester never learns who the admins are.

use fau_domain::email::{Email, VerifiedEmail};
use fau_domain::membership::period::Period;
use fau_domain::membership::requests::{
    check_replacement_dates, check_request_limits, lapse_cutoff, normalise_message,
};
use fau_domain::membership::vocabulary::{InvitationMode, RequestKind};
use fau_domain::time::Moment;
use jiff::civil::Date;
use serde_json::json;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::error::MembershipError;
use super::invitations::{
    insert_invitation, resolve_roles, IssuedInvitation, NewInvitation, OfferedRole,
};
use super::sql::{
    admin_emails, date_param, enqueue, lock_tenant, parse_date, require_admin, require_open,
    ts_param, usable_member_email, write_audit, ActorKind, Audit,
};

#[derive(Debug, Clone)]
pub struct CreateAccessRequest {
    pub tenant_id: Uuid,
    /// Confirmed with a Hanko passcode first (spec 3.2), so nobody can make the portal
    /// email an FAU's admins from an address they do not control.
    pub requester: VerifiedEmail,
    pub message: Option<String>,
}

/// Creates an access request (spec 5.2) and emails every current admin.
pub async fn create_access_request(
    pool: &PgPool,
    req: CreateAccessRequest,
    at: Moment,
) -> Result<Uuid, MembershipError> {
    let message =
        normalise_message(req.message.as_deref()).map_err(|_| MembershipError::MessageTooLong)?;
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, req.tenant_id).await?;
    require_open(&state)?;
    let requester = req.requester.email().as_str();
    check_limits(&mut tx, req.tenant_id, requester, at.today()).await?;

    let request_id = insert_request(
        &mut tx,
        at,
        NewRequest {
            tenant_id: req.tenant_id,
            kind: RequestKind::Access,
            requester_email: requester,
            invitee_email: requester,
            requester_membership_id: None,
            replaced_assignment_id: None,
            proposed: None,
            message: message.as_deref(),
        },
    )
    .await?;
    notify_admins(&mut tx, at, req.tenant_id, request_id, RequestKind::Access).await?;
    write_audit(
        &mut tx,
        at,
        Audit {
            tenant_id: Some(req.tenant_id),
            actor_kind: ActorKind::Requester,
            actor_account_id: None,
            actor_membership_id: None,
            action: "request.created",
            subject_type: "access_request",
            subject_id: request_id,
            params: json!({ "kind": RequestKind::Access.code() }),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(request_id)
}

#[derive(Debug, Clone)]
pub struct CreateReplacementProposal {
    pub tenant_id: Uuid,
    pub proposer_membership_id: Uuid,
    /// One of the proposer's own role assignments, valid today (spec 5.3).
    pub replaced_assignment_id: Uuid,
    pub successor: Email,
    pub starts_on: Date,
    pub ends_on_exclusive: Date,
    pub message: Option<String>,
}

/// Creates a replacement proposal (spec 5.3). The role's name and class are the
/// proposer's own role's and cannot be changed here; an admin may change them on
/// approval. Proposing does not end the proposer's role. Counts against the proposer's
/// address for the one-open-request limit.
pub async fn create_replacement_proposal(
    pool: &PgPool,
    req: CreateReplacementProposal,
    at: Moment,
) -> Result<Uuid, MembershipError> {
    let message =
        normalise_message(req.message.as_deref()).map_err(|_| MembershipError::MessageTooLong)?;
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, req.tenant_id).await?;
    require_open(&state)?;
    let proposer_email = usable_member_email(&mut tx, req.tenant_id, req.proposer_membership_id)
        .await?
        .ok_or(MembershipError::NotAuthorized)?;

    let assignment: Option<(Uuid, String, String, bool)> = sqlx::query_as(
        "select membership_id,
                to_char(starts_on, 'YYYY-MM-DD'), to_char(ends_on_exclusive, 'YYYY-MM-DD'),
                revoked_at is not null
           from role_assignments where tenant_id = $1 and id = $2",
    )
    .bind(req.tenant_id)
    .bind(req.replaced_assignment_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (holder, starts, ends, revoked) = assignment.ok_or(MembershipError::UnknownAssignment)?;
    if holder != req.proposer_membership_id {
        return Err(MembershipError::NotAuthorized);
    }
    let held = Period::new(parse_date(&starts)?, parse_date(&ends)?)
        .map_err(|_| MembershipError::decode())?;
    if revoked || !held.contains(at.today()) {
        return Err(MembershipError::RoleNotHeldToday);
    }
    let proposed = check_replacement_dates(at.today(), req.starts_on, req.ends_on_exclusive)
        .map_err(MembershipError::ReplacementDates)?;
    check_limits(&mut tx, req.tenant_id, &proposer_email, at.today()).await?;

    let request_id = insert_request(
        &mut tx,
        at,
        NewRequest {
            tenant_id: req.tenant_id,
            kind: RequestKind::Replacement,
            requester_email: &proposer_email,
            invitee_email: req.successor.as_str(),
            requester_membership_id: Some(req.proposer_membership_id),
            replaced_assignment_id: Some(req.replaced_assignment_id),
            proposed: Some(proposed),
            message: message.as_deref(),
        },
    )
    .await?;
    notify_admins(
        &mut tx,
        at,
        req.tenant_id,
        request_id,
        RequestKind::Replacement,
    )
    .await?;
    write_audit(
        &mut tx,
        at,
        Audit::member(
            req.tenant_id,
            req.proposer_membership_id,
            "request.created",
            "access_request",
            request_id,
            json!({
                "kind": RequestKind::Replacement.code(),
                "replaced_assignment_id": req.replaced_assignment_id,
            }),
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(request_id)
}

async fn check_limits(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    requester_email: &str,
    today: Date,
) -> Result<(), MembershipError> {
    let open: i64 = sqlx::query_scalar(
        "select count(*) from access_requests
          where tenant_id = $1 and requester_email = $2 and status = 'pending'",
    )
    .bind(tenant_id)
    .bind(requester_email)
    .fetch_one(&mut *conn)
    .await?;
    let created_today: i64 = sqlx::query_scalar(
        "select count(*) from access_requests where tenant_id = $1 and created_on = $2::date",
    )
    .bind(tenant_id)
    .bind(date_param(today))
    .fetch_one(&mut *conn)
    .await?;
    check_request_limits(open, created_today).map_err(MembershipError::RequestLimit)
}

struct NewRequest<'a> {
    tenant_id: Uuid,
    kind: RequestKind,
    requester_email: &'a str,
    invitee_email: &'a str,
    requester_membership_id: Option<Uuid>,
    replaced_assignment_id: Option<Uuid>,
    proposed: Option<Period>,
    message: Option<&'a str>,
}

async fn insert_request(
    conn: &mut PgConnection,
    at: Moment,
    r: NewRequest<'_>,
) -> Result<Uuid, MembershipError> {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into access_requests
           (tenant_id, id, kind, requester_email, invitee_email, requester_membership_id,
            replaced_assignment_id, proposed_starts_on, proposed_ends_on_exclusive, message,
            created_on, created_at)
         values ($1, $2, $3, $4, $5, $6, $7, $8::date, $9::date, $10, $11::date, $12::timestamptz)",
    )
    .bind(r.tenant_id)
    .bind(id)
    .bind(r.kind.code())
    .bind(r.requester_email)
    .bind(r.invitee_email)
    .bind(r.requester_membership_id)
    .bind(r.replaced_assignment_id)
    .bind(r.proposed.map(|p| date_param(p.starts_on())))
    .bind(r.proposed.map(|p| date_param(p.ends_on_exclusive())))
    .bind(r.message)
    .bind(date_param(at.today()))
    .bind(ts_param(at.now()))
    .execute(&mut *conn)
    .await?;
    Ok(id)
}

/// One `request.received` message per current admin. An FAU with no admin gets none;
/// the request waits, and lapses if nobody arrives to handle it.
async fn notify_admins(
    conn: &mut PgConnection,
    at: Moment,
    tenant_id: Uuid,
    request_id: Uuid,
    kind: RequestKind,
) -> Result<(), MembershipError> {
    for admin in admin_emails(conn, tenant_id, at.today()).await? {
        enqueue(
            conn,
            at,
            "request.received",
            &admin,
            json!({ "tenant_id": tenant_id, "request_id": request_id, "kind": kind.code() }),
        )
        .await?;
    }
    Ok(())
}

/// A decision on a pending request.
#[derive(Debug, Clone)]
pub struct RequestDecision {
    pub tenant_id: Uuid,
    pub actor_membership_id: Uuid,
    pub request_id: Uuid,
}

struct RequestRow {
    requester_email: String,
    invitee_email: String,
}

async fn lock_pending_request(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    request_id: Uuid,
) -> Result<RequestRow, MembershipError> {
    let row: Option<(String, String, String)> = sqlx::query_as(
        "select status, requester_email, invitee_email from access_requests
          where tenant_id = $1 and id = $2 for update",
    )
    .bind(tenant_id)
    .bind(request_id)
    .fetch_optional(&mut *conn)
    .await?;
    let (status, requester_email, invitee_email) = row.ok_or(MembershipError::UnknownRequest)?;
    if status != "pending" {
        return Err(MembershipError::RequestNotPending);
    }
    Ok(RequestRow {
        requester_email,
        invitee_email,
    })
}

/// Approves a pending request by issuing a normal invitation to its invitee with the
/// roles the admin chose (spec 5.2.2, 5.3), linked to the request.
///
/// **Gate:** as `issue_invitation`. Only an admin valid today may approve; a handover
/// grant does not suffice (spec 6.3).
pub async fn approve_request(
    pool: &PgPool,
    decision: RequestDecision,
    roles: Vec<OfferedRole>,
    at: Moment,
) -> Result<IssuedInvitation, MembershipError> {
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, decision.tenant_id).await?;
    require_open(&state)?;
    require_admin(
        &mut tx,
        decision.tenant_id,
        decision.actor_membership_id,
        at.today(),
    )
    .await?;
    let request = lock_pending_request(&mut tx, decision.tenant_id, decision.request_id).await?;
    let invitee = Email::parse(&request.invitee_email).map_err(|_| MembershipError::decode())?;

    let roles = resolve_roles(
        &mut tx,
        at,
        decision.tenant_id,
        Some(decision.actor_membership_id),
        &roles,
    )
    .await?;
    let issued = insert_invitation(
        &mut tx,
        at,
        NewInvitation {
            tenant_id: decision.tenant_id,
            mode: InvitationMode::Normal,
            recipient: invitee,
            issued_by: Some(decision.actor_membership_id),
            handover_grant_id: None,
            access_request_id: Some(decision.request_id),
            recovery_holder: None,
            roles: roles.into_iter().map(|(id, _, p)| (id, p)).collect(),
            actor_kind: ActorKind::Member,
            actor_membership_id: Some(decision.actor_membership_id),
        },
    )
    .await?;
    close_request(&mut tx, at, &decision, "approved").await?;
    tx.commit().await?;
    Ok(issued)
}

/// Declines a pending request. The requester is told only that it was not approved
/// (spec 5.2.3).
///
/// **Gate:** as `issue_invitation`.
pub async fn decline_request(
    pool: &PgPool,
    decision: RequestDecision,
    at: Moment,
) -> Result<(), MembershipError> {
    let mut tx = pool.begin().await?;
    lock_tenant(&mut tx, decision.tenant_id).await?;
    require_admin(
        &mut tx,
        decision.tenant_id,
        decision.actor_membership_id,
        at.today(),
    )
    .await?;
    let request = lock_pending_request(&mut tx, decision.tenant_id, decision.request_id).await?;
    close_request(&mut tx, at, &decision, "declined").await?;
    enqueue(
        &mut tx,
        at,
        "request.declined",
        &request.requester_email,
        json!({ "request_id": decision.request_id }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn close_request(
    conn: &mut PgConnection,
    at: Moment,
    decision: &RequestDecision,
    status: &'static str,
) -> Result<(), MembershipError> {
    sqlx::query(
        "update access_requests set status = $3, decided_by = $4, closed_at = $5::timestamptz
          where tenant_id = $1 and id = $2",
    )
    .bind(decision.tenant_id)
    .bind(decision.request_id)
    .bind(status)
    .bind(decision.actor_membership_id)
    .bind(ts_param(at.now()))
    .execute(&mut *conn)
    .await?;
    let action = if status == "approved" {
        "request.approved"
    } else {
        "request.declined"
    };
    write_audit(
        conn,
        at,
        Audit::member(
            decision.tenant_id,
            decision.actor_membership_id,
            action,
            "access_request",
            decision.request_id,
            json!({}),
        ),
    )
    .await
}

/// The scheduled sweep: every request pending for 30 days lapses, and its requester is
/// told the same as for a decline (spec 5.2.4). Returns how many.
pub async fn lapse_requests(pool: &PgPool, at: Moment) -> Result<u64, MembershipError> {
    let mut tx = pool.begin().await?;
    let lapsed: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
        "update access_requests set status = 'lapsed', closed_at = $1::timestamptz
          where status = 'pending' and created_on <= $2::date
         returning tenant_id, id, requester_email",
    )
    .bind(ts_param(at.now()))
    .bind(date_param(lapse_cutoff(at.today())))
    .fetch_all(&mut *tx)
    .await?;
    for (tenant_id, request_id, requester_email) in &lapsed {
        enqueue(
            &mut tx,
            at,
            "request.lapsed",
            requester_email,
            json!({ "request_id": request_id }),
        )
        .await?;
        write_audit(
            &mut tx,
            at,
            Audit::system(
                *tenant_id,
                "request.lapsed",
                "access_request",
                *request_id,
                json!({}),
            ),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(lapsed.len() as u64)
}
```

Create (or replace in full) `backend/crates/persistence/src/membership/mod.rs`:

```rust
//! The membership foundation's transactions (#3413 flow spec; #3417, #3418). Each public
//! function is one transaction that writes its state change together with its audit
//! entry and, where the spec says so, its outbox message (spec 2.6).
//!
//! Every function takes a [`fau_domain::time::Moment`]: rule-deciding timestamps and
//! "today" come from the caller, never from the database clock.

mod error;
mod invitations;
mod requests;
mod signup;
mod sql;
mod token;

pub use error::{ExistingFau, MembershipError};
pub use invitations::{
    accept_invitation, issue_invitation, resend_invitation, withdraw_invitation, AcceptInvitation,
    Accepted, InvitationChange, IssueInvitation, IssuedInvitation, OfferedRole, RoleChoice,
};
pub use requests::{
    approve_request, create_access_request, create_replacement_proposal, decline_request,
    lapse_requests, CreateAccessRequest, CreateReplacementProposal, RequestDecision,
};
pub use signup::{
    activate_tenant, create_pending_tenant, expire_pending_tenants, Activated, Activation,
    PendingSignup, PendingTenant,
};
pub use token::InvitationToken;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test --workspace`
Expected: PASS, and `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings` are clean.

- [ ] **Step 5: Commit** (the controller commits; executors do not)

```bash
git add backend/crates/persistence/src/membership backend/crates/app/tests/requests.rs
git commit -m "Add access requests and replacement proposals (#3413, #3418)"
```

---

### Task 8: Domain and persistence: roles, revocation and the last-admin safeguard

Grant, revoke one assignment, revoke a whole membership (also "leave"). Revocation is immediate, revokes any handover grant derived from the revoked role, and creates none. The safeguard needs a rule that "no admin today" alone cannot give: when an admin revokes the only other admin, the revoker is valid today by definition, so the FAU still has an admin *today*. Spec §7 still demands confirmation, because the FAU loses its admin as soon as the revoker's own term ends. So `removal_leaves_no_admin` asks whether removing the rows would create any day, from today through the removed rows' remaining term, without an admin that the removed rows would have covered. Coverage can only drop where a remaining row ends, so checking the first remaining day and every remaining end inside the removed term is exhaustive. It is added to `access.rs` in the domain with its own unit tests; `AdminState` in `sql.rs` is replaced by a version that keeps row ids, so an action can say which rows it removes.

**Files:**
- Replace in full: `backend/crates/domain/src/membership/access.rs`
- Modify: `backend/crates/persistence/src/membership/sql.rs`
- Create: `backend/crates/persistence/src/membership/roles.rs`
- Replace in full: `backend/crates/persistence/src/membership/mod.rs`
- Create: `backend/crates/app/tests/roles.rs`

**Interfaces:**
- Consumes: `AdminState` (Task 6), `resolve_roles`, `insert_assignment`.
- Produces:
  - Domain: `access::removal_leaves_no_admin(remaining_assignments, remaining_grants, removed_assignments, removed_grants, today) -> bool`.
  - `grant_role(&PgPool, GrantRole, Moment) -> Result<Uuid, _>`; `GrantRole { tenant_id, actor_membership_id, membership_id, role: RoleChoice, period: Period }`.
  - `revoke_role_assignment(&PgPool, RevokeAssignment, Moment) -> Result<(), _>`; `RevokeAssignment { tenant_id, actor_membership_id, assignment_id, confirm_no_admin: bool }`.
  - `revoke_membership(&PgPool, RevokeMembership, Moment) -> Result<(), _>`; `RevokeMembership { tenant_id, actor_membership_id, membership_id, confirm_no_admin: bool }`.
  - Refusal without confirmation: `MembershipError::WouldLeaveNoAdmin`.
  - Crate-private: `sql::check_last_admin(&AdminState, Date, bool, impl Fn(Uuid, Uuid) -> bool) -> Result<bool, _>`.

- [ ] **Step 1: Write the failing tests**

Create (or replace in full) `backend/crates/domain/src/membership/access.rs`:

```rust
//! Access evaluation (§2.1, §4, §6.3, §6.4). Rights are the union of roles valid today,
//! re-evaluated on every request from the database, never cached from login.

use jiff::civil::Date;

use super::period::Period;
use super::vocabulary::CapabilityClass;

/// What a person may do in one FAU today. Ordered: `Admin` includes every `Member` right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capability {
    None,
    Member,
    Admin,
}

/// One role assignment as the rules need it. `capability` comes from the role row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssignmentView {
    pub capability: CapabilityClass,
    pub period: Period,
    pub revoked: bool,
}

impl AssignmentView {
    pub fn valid_on(&self, day: Date) -> bool {
        !self.revoked && self.period.contains(day)
    }
}

/// One handover grant (§6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrantView {
    pub period: Period,
    pub revoked: bool,
}

impl GrantView {
    pub fn valid_on(&self, day: Date) -> bool {
        !self.revoked && self.period.contains(day)
    }
}

/// The preconditions that come before any role is looked at. Each one alone removes all
/// access: an FAU that is not active, a disabled or unverified account, or a revoked
/// membership (#3412: "a revoked right gives no access through the revoked right").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Standing {
    pub tenant_active: bool,
    pub account_usable: bool,
    pub membership_active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Access {
    pub capability: Capability,
    /// A handover grant is valid today. It gives no document access (§6.3); the HTTP
    /// layer lets it reach only the handover operations.
    pub handover: bool,
    /// When `capability` is `None`, the earliest future start of a role, so the page can
    /// say when access begins (§4).
    pub next_start: Option<Date>,
}

impl Access {
    pub const NONE: Access = Access {
        capability: Capability::None,
        handover: false,
        next_start: None,
    };
}

pub fn evaluate_access(
    standing: Standing,
    assignments: &[AssignmentView],
    grants: &[GrantView],
    today: Date,
) -> Access {
    if !(standing.tenant_active && standing.account_usable && standing.membership_active) {
        return Access::NONE;
    }
    let capability = assignments
        .iter()
        .filter(|a| a.valid_on(today))
        .map(|a| match a.capability {
            CapabilityClass::Member => Capability::Member,
            CapabilityClass::Admin => Capability::Admin,
        })
        .max()
        .unwrap_or(Capability::None);
    let handover = grants.iter().any(|g| g.valid_on(today));
    let next_start = if capability == Capability::None {
        assignments
            .iter()
            .filter(|a| !a.revoked && a.period.starts_on() > today)
            .map(|a| a.period.starts_on())
            .min()
    } else {
        None
    };
    Access {
        capability,
        handover,
        next_start,
    }
}

/// The negation of §6.4's no-admin state: an `admin`-class role valid today or a handover
/// grant valid today. The caller passes only rows whose membership is active and whose
/// account is usable; a revoked person's rows do not count.
pub fn tenant_has_admin(assignments: &[AssignmentView], grants: &[GrantView], today: Date) -> bool {
    assignments
        .iter()
        .any(|a| a.capability == CapabilityClass::Admin && a.valid_on(today))
        || grants.iter().any(|g| g.valid_on(today))
}

/// Spec 7's last-admin safeguard: whether ending `removed_*` would leave a day, from
/// `today` on, on which the FAU has no admin although the removed rows would have given
/// it one. Judging only today would make "revoking the only other admin" impossible to
/// catch -- the revoking admin is valid today by definition -- so the check runs over
/// the removed rows' remaining term: an admin whose own role ends in December revoking
/// the admin who would have carried the FAU until next October leaves a gap.
///
/// Coverage can only drop where a remaining row ends, so checking the first remaining
/// day of each removed row and every remaining end inside it is exhaustive. Handover
/// grants that do not exist yet (they are created when a role ends) are not predicted.
pub fn removal_leaves_no_admin(
    remaining_assignments: &[AssignmentView],
    remaining_grants: &[GrantView],
    removed_assignments: &[AssignmentView],
    removed_grants: &[GrantView],
    today: Date,
) -> bool {
    let covered = |day: Date| tenant_has_admin(remaining_assignments, remaining_grants, day);
    let remaining_ends: Vec<Date> = remaining_assignments
        .iter()
        .filter(|a| !a.revoked && a.capability == CapabilityClass::Admin)
        .map(|a| a.period.ends_on_exclusive())
        .chain(
            remaining_grants
                .iter()
                .filter(|g| !g.revoked)
                .map(|g| g.period.ends_on_exclusive()),
        )
        .collect();
    let removed = removed_assignments
        .iter()
        .filter(|a| !a.revoked && a.capability == CapabilityClass::Admin)
        .map(|a| a.period)
        .chain(
            removed_grants
                .iter()
                .filter(|g| !g.revoked)
                .map(|g| g.period),
        );
    for period in removed {
        if period.has_ended_by(today) {
            continue;
        }
        let first = period.starts_on().max(today);
        if !covered(first) {
            return true;
        }
        if remaining_ends
            .iter()
            .any(|&end| first < end && end < period.ends_on_exclusive() && !covered(end))
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    const OK: Standing = Standing {
        tenant_active: true,
        account_usable: true,
        membership_active: true,
    };

    fn role(capability: CapabilityClass, from: Date, to: Date) -> AssignmentView {
        AssignmentView {
            capability,
            period: Period::new(from, to).unwrap(),
            revoked: false,
        }
    }

    fn grant(from: Date, to: Date) -> GrantView {
        GrantView {
            period: Period::new(from, to).unwrap(),
            revoked: false,
        }
    }

    #[test]
    fn a_role_valid_tomorrow_grants_nothing_today() {
        let today = date(2026, 9, 23);
        let a = [role(
            CapabilityClass::Member,
            date(2026, 9, 24),
            date(2027, 9, 24),
        )];
        let access = evaluate_access(OK, &a, &[], today);
        assert_eq!(access.capability, Capability::None);
        assert_eq!(access.next_start, Some(date(2026, 9, 24)));
    }

    #[test]
    fn the_end_date_itself_grants_nothing() {
        let a = [role(
            CapabilityClass::Member,
            date(2026, 8, 1),
            date(2027, 8, 1),
        )];
        assert_eq!(
            evaluate_access(OK, &a, &[], date(2027, 7, 31)).capability,
            Capability::Member
        );
        assert_eq!(
            evaluate_access(OK, &a, &[], date(2027, 8, 1)).capability,
            Capability::None
        );
    }

    #[test]
    fn rights_are_the_union_of_valid_roles() {
        // #3412's example: admin ends, a member role continues.
        let a = [
            role(CapabilityClass::Admin, date(2026, 8, 1), date(2027, 8, 1)),
            role(CapabilityClass::Member, date(2026, 8, 1), date(2028, 1, 1)),
        ];
        assert_eq!(
            evaluate_access(OK, &a, &[], date(2027, 7, 31)).capability,
            Capability::Admin
        );
        assert_eq!(
            evaluate_access(OK, &a, &[], date(2027, 8, 1)).capability,
            Capability::Member
        );
    }

    #[test]
    fn a_revoked_role_grants_nothing() {
        let mut a = role(CapabilityClass::Admin, date(2026, 8, 1), date(2027, 8, 1));
        a.revoked = true;
        assert_eq!(
            evaluate_access(OK, &[a], &[], date(2026, 9, 23)),
            Access::NONE
        );
    }

    #[test]
    fn each_standing_precondition_removes_all_access() {
        let a = [role(
            CapabilityClass::Admin,
            date(2026, 8, 1),
            date(2027, 8, 1),
        )];
        let g = [grant(date(2026, 8, 1), date(2027, 2, 1))];
        for standing in [
            Standing {
                tenant_active: false,
                ..OK
            },
            Standing {
                account_usable: false,
                ..OK
            },
            Standing {
                membership_active: false,
                ..OK
            },
        ] {
            assert_eq!(
                evaluate_access(standing, &a, &g, date(2026, 9, 23)),
                Access::NONE
            );
        }
    }

    #[test]
    fn a_handover_grant_alone_gives_no_capability() {
        let a = [role(
            CapabilityClass::Admin,
            date(2026, 10, 1),
            date(2027, 10, 1),
        )];
        let g = [grant(date(2027, 10, 1), date(2028, 4, 1))];
        let access = evaluate_access(OK, &a, &g, date(2027, 11, 1));
        assert_eq!(access.capability, Capability::None);
        assert!(access.handover);
        assert_eq!(access.next_start, None);
    }

    #[test]
    fn a_handover_grant_ends_at_its_boundary_or_on_revocation() {
        let g = grant(date(2027, 10, 1), date(2028, 4, 1));
        assert!(evaluate_access(OK, &[], &[g], date(2028, 3, 31)).handover);
        assert!(!evaluate_access(OK, &[], &[g], date(2028, 4, 1)).handover);
        let revoked = GrantView { revoked: true, ..g };
        assert!(!evaluate_access(OK, &[], &[revoked], date(2027, 11, 1)).handover);
    }

    #[test]
    fn the_no_admin_predicate() {
        let today = date(2026, 9, 23);
        let member = role(CapabilityClass::Member, date(2026, 1, 1), date(2027, 1, 1));
        let admin = role(CapabilityClass::Admin, date(2026, 1, 1), date(2027, 1, 1));
        let future_admin = role(CapabilityClass::Admin, date(2026, 10, 1), date(2027, 10, 1));
        let g = grant(date(2026, 8, 1), date(2027, 2, 1));

        assert!(!tenant_has_admin(&[], &[], today));
        assert!(
            !tenant_has_admin(&[member], &[], today),
            "the class decides, not the name"
        );
        assert!(!tenant_has_admin(&[future_admin], &[], today));
        assert!(tenant_has_admin(&[admin], &[], today));
        assert!(
            tenant_has_admin(&[member], &[g], today),
            "a valid handover grant counts"
        );
    }

    #[test]
    fn removing_the_sole_admin_leaves_none() {
        let today = date(2026, 9, 23);
        let only = role(CapabilityClass::Admin, date(2026, 9, 1), date(2027, 10, 1));
        assert!(removal_leaves_no_admin(&[], &[], &[only], &[], today));
    }

    #[test]
    fn removing_one_of_two_equal_admins_leaves_one() {
        let today = date(2026, 9, 23);
        let a = role(CapabilityClass::Admin, date(2026, 9, 1), date(2027, 10, 1));
        let b = role(CapabilityClass::Admin, date(2026, 9, 1), date(2027, 10, 1));
        assert!(!removal_leaves_no_admin(&[a], &[], &[b], &[], today));
    }

    #[test]
    fn removing_the_admin_who_outlasts_you_leaves_a_gap() {
        // Spec 7's "an admin revoking the only other admin".
        let today = date(2026, 9, 23);
        let short = role(CapabilityClass::Admin, date(2026, 1, 1), date(2026, 12, 1));
        let long = role(CapabilityClass::Admin, date(2026, 1, 1), date(2027, 10, 1));
        assert!(removal_leaves_no_admin(&[short], &[], &[long], &[], today));

        let successor = role(CapabilityClass::Admin, date(2026, 12, 1), date(2027, 10, 1));
        assert!(!removal_leaves_no_admin(
            &[short, successor],
            &[],
            &[long],
            &[],
            today
        ));
    }

    #[test]
    fn removing_an_ended_or_member_role_changes_nothing() {
        let today = date(2026, 9, 23);
        let ended = role(CapabilityClass::Admin, date(2025, 9, 1), date(2026, 9, 1));
        let member = role(CapabilityClass::Member, date(2026, 9, 1), date(2027, 9, 1));
        assert!(!removal_leaves_no_admin(
            &[],
            &[],
            &[ended, member],
            &[],
            today
        ));
    }

    #[test]
    fn removing_a_future_admin_with_nobody_else_then_leaves_a_gap() {
        let today = date(2026, 9, 23);
        let current = role(CapabilityClass::Admin, date(2026, 1, 1), date(2026, 12, 1));
        let next = role(CapabilityClass::Admin, date(2026, 12, 1), date(2027, 12, 1));
        assert!(removal_leaves_no_admin(
            &[current],
            &[],
            &[next],
            &[],
            today
        ));
    }

    #[test]
    fn a_remaining_handover_grant_counts_as_coverage() {
        let today = date(2026, 9, 23);
        let g = grant(date(2026, 8, 1), date(2027, 2, 1));
        let admin = role(CapabilityClass::Admin, date(2026, 9, 1), date(2026, 12, 1));
        assert!(!removal_leaves_no_admin(&[], &[g], &[admin], &[], today));
        assert!(removal_leaves_no_admin(&[], &[], &[], &[g], today));
    }
}
```

Create (or replace in full) `backend/crates/app/tests/roles.rs`:

```rust
//! Granting and revoking roles and memberships, and the last-admin safeguard (flow
//! spec §7; §10 "The last-admin safeguard requires confirmation in each of these cases").

mod common;
use common::membership::*;
use common::TestDb;
use fau_domain::membership::vocabulary::CapabilityClass;
use fau_persistence::membership::{
    grant_role, revoke_membership, revoke_role_assignment, GrantRole, MembershipError,
    RevokeAssignment, RevokeMembership, RoleChoice,
};
use sqlx::PgPool;
use uuid::Uuid;

async fn revoked(pool: &PgPool, table: &str, id: Uuid) -> bool {
    sqlx::query_scalar(&format!(
        "select revoked_at is not null from {table} where id = $1"
    ))
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn an_admin_grants_a_role_to_a_member() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let member = add_member(
        &pool,
        &fau,
        "medlem@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;

    let grant = |actor| GrantRole {
        tenant_id: fau.tenant_id,
        actor_membership_id: actor,
        membership_id: member.membership_id,
        role: new_role("Kasserer", CapabilityClass::Member),
        period: period(day(2026, 10, 1), day(2027, 10, 1)),
    };
    assert_eq!(
        grant_role(&pool, grant(member.membership_id), t0)
            .await
            .unwrap_err(),
        MembershipError::NotAuthorized,
        "a member cannot grant, not even to themselves"
    );
    let id = grant_role(&pool, grant(fau.admin_membership_id), t0)
        .await
        .unwrap();
    let granted_by: Option<Uuid> =
        sqlx::query_scalar("select granted_by from role_assignments where id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(granted_by, Some(fau.admin_membership_id));
    assert_eq!(audit_count(&pool, "role.granted").await, 1);
}

#[tokio::test]
async fn a_revoked_membership_cannot_be_granted_a_role() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let member = add_member(
        &pool,
        &fau,
        "medlem@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: member.membership_id,
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .unwrap();
    let err = grant_role(
        &pool,
        GrantRole {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: member.membership_id,
            role: new_role("Kasserer", CapabilityClass::Member),
            period: period(day(2026, 10, 1), day(2027, 10, 1)),
        },
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::MembershipRevoked);
}

#[tokio::test]
async fn an_admin_revokes_a_role_at_once() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let member = add_member(
        &pool,
        &fau,
        "medlem@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    let revoke = RevokeAssignment {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        assignment_id: member.assignment_ids[0],
        confirm_no_admin: false,
    };
    revoke_role_assignment(&pool, revoke, t0).await.unwrap();
    assert!(revoked(&pool, "role_assignments", member.assignment_ids[0]).await);
    assert_eq!(
        revoke_role_assignment(&pool, revoke, t0).await.unwrap_err(),
        MembershipError::AssignmentAlreadyRevoked
    );
}

#[tokio::test]
async fn a_member_cannot_revoke_someone_elses_role() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let member = add_member(
        &pool,
        &fau,
        "medlem@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    let err = revoke_role_assignment(
        &pool,
        RevokeAssignment {
            tenant_id: fau.tenant_id,
            actor_membership_id: member.membership_id,
            assignment_id: fau.admin_assignment_id,
            confirm_no_admin: true,
        },
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::NotAuthorized);
}

#[tokio::test]
async fn revoking_oneself_as_the_only_admin_needs_confirmation() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let revoke = |confirm_no_admin| RevokeAssignment {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        assignment_id: fau.admin_assignment_id,
        confirm_no_admin,
    };

    assert_eq!(
        revoke_role_assignment(&pool, revoke(false), t0)
            .await
            .unwrap_err(),
        MembershipError::WouldLeaveNoAdmin
    );
    assert!(!revoked(&pool, "role_assignments", fau.admin_assignment_id).await);
    revoke_role_assignment(&pool, revoke(true), t0)
        .await
        .unwrap();
    assert!(revoked(&pool, "role_assignments", fau.admin_assignment_id).await);
    let flagged: bool = sqlx::query_scalar(
        "select (params->>'left_no_admin')::boolean from audit_events where action = 'role.revoked'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(flagged);
}

#[tokio::test]
async fn leaving_as_the_only_admin_needs_confirmation() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let leave = |confirm_no_admin| RevokeMembership {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        membership_id: fau.admin_membership_id,
        confirm_no_admin,
    };

    assert_eq!(
        revoke_membership(&pool, leave(false), t0)
            .await
            .unwrap_err(),
        MembershipError::WouldLeaveNoAdmin
    );
    revoke_membership(&pool, leave(true), t0).await.unwrap();
    assert!(revoked(&pool, "memberships", fau.admin_membership_id).await);
    assert!(
        revoked(&pool, "role_assignments", fau.admin_assignment_id).await,
        "leaving ends the roles too"
    );
    assert_eq!(audit_count(&pool, "membership.left").await, 1);
}

#[tokio::test]
async fn revoking_the_only_other_admin_needs_confirmation() {
    // The registrant's own role ends 2027-10-01. The second admin would have carried
    // the FAU until 2028-06-01; revoking them leaves no admin from 2027-10-01.
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let other = add_member(
        &pool,
        &fau,
        "admin2@example.test",
        RoleChoice::Existing(fau.admin_role_id),
        period(day(2026, 9, 23), day(2028, 6, 1)),
        t0,
    )
    .await;
    let revoke = |confirm_no_admin| RevokeMembership {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        membership_id: other.membership_id,
        confirm_no_admin,
    };

    assert_eq!(
        revoke_membership(&pool, revoke(false), t0)
            .await
            .unwrap_err(),
        MembershipError::WouldLeaveNoAdmin
    );
    revoke_membership(&pool, revoke(true), t0).await.unwrap();
    assert_eq!(audit_count(&pool, "membership.revoked").await, 1);
}

#[tokio::test]
async fn revoking_an_admin_whose_term_is_covered_needs_no_confirmation() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let other = add_member(
        &pool,
        &fau,
        "admin2@example.test",
        RoleChoice::Existing(fau.admin_role_id),
        period(day(2026, 9, 23), day(2027, 10, 1)),
        t0,
    )
    .await;
    revoke_role_assignment(
        &pool,
        RevokeAssignment {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            assignment_id: other.assignment_ids[0],
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .expect("the remaining admin covers the same term");
}

#[tokio::test]
async fn leaving_one_fau_leaves_the_other_untouched() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let first = active_fau(&pool, "begge@example.test", t0).await;
    let second = active_fau(&pool, "begge@example.test", t0).await;
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: first.tenant_id,
            actor_membership_id: first.admin_membership_id,
            membership_id: first.admin_membership_id,
            confirm_no_admin: true,
        },
        t0,
    )
    .await
    .unwrap();
    assert!(!revoked(&pool, "memberships", second.admin_membership_id).await);
    assert!(!revoked(&pool, "role_assignments", second.admin_assignment_id).await);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test -p fau-app --test roles`
Expected: compile error: `grant_role`, `revoke_role_assignment`, `revoke_membership` not found.

- [ ] **Step 3: Implement**

In `backend/crates/persistence/src/membership/sql.rs`, replace:

```rust
use fau_domain::membership::access::{tenant_has_admin, AssignmentView, GrantView};
```

with:

```rust
use fau_domain::membership::access::{
    removal_leaves_no_admin, tenant_has_admin, AssignmentView, GrantView,
};
```

In `backend/crates/persistence/src/membership/sql.rs`, replace:

```rust
/// Every admin-class assignment and handover grant held through a usable membership:
/// the inputs to the no-admin predicate (spec 6.4).
pub(crate) struct AdminState {
    assignments: Vec<AssignmentView>,
    grants: Vec<GrantView>,
}

impl AdminState {
    pub(crate) async fn load(
        conn: &mut PgConnection,
        tenant_id: Uuid,
    ) -> Result<Self, MembershipError> {
        let rows: Vec<(String, String, bool)> = sqlx::query_as(
            "select to_char(ra.starts_on, 'YYYY-MM-DD'), to_char(ra.ends_on_exclusive, 'YYYY-MM-DD'),
                    ra.revoked_at is not null
               from role_assignments ra
               join roles r       on r.tenant_id = ra.tenant_id and r.id = ra.role_id
               join memberships m on m.tenant_id = ra.tenant_id and m.id = ra.membership_id
               join accounts a    on a.id = m.account_id
              where ra.tenant_id = $1 and r.capability_class = 'admin'
                and m.revoked_at is null and a.disabled_at is null and a.verified_at is not null",
        )
        .bind(tenant_id)
        .fetch_all(&mut *conn)
        .await?;
        let mut assignments = Vec::with_capacity(rows.len());
        for (starts, ends, revoked) in rows {
            assignments.push(AssignmentView {
                capability: CapabilityClass::Admin,
                period: period_from(&starts, &ends)?,
                revoked,
            });
        }

        let rows: Vec<(String, String, bool)> = sqlx::query_as(
            "select to_char(g.starts_on, 'YYYY-MM-DD'), to_char(g.ends_on_exclusive, 'YYYY-MM-DD'),
                    g.revoked_at is not null
               from handover_grants g
               join role_assignments ra on ra.tenant_id = g.tenant_id and ra.id = g.source_assignment_id
               join memberships m       on m.tenant_id = ra.tenant_id and m.id = ra.membership_id
               join accounts a          on a.id = m.account_id
              where g.tenant_id = $1 and m.revoked_at is null and a.disabled_at is null",
        )
        .bind(tenant_id)
        .fetch_all(&mut *conn)
        .await?;
        let mut grants = Vec::with_capacity(rows.len());
        for (starts, ends, revoked) in rows {
            grants.push(GrantView {
                period: period_from(&starts, &ends)?,
                revoked,
            });
        }
        Ok(Self {
            assignments,
            grants,
        })
    }

    pub(crate) fn has_admin(&self, today: Date) -> bool {
        tenant_has_admin(&self.assignments, &self.grants, today)
    }
}
```

with:

```rust
pub(crate) struct AdminAssignmentRow {
    pub(crate) id: Uuid,
    pub(crate) membership_id: Uuid,
    pub(crate) view: AssignmentView,
}

pub(crate) struct GrantRow {
    pub(crate) source_assignment_id: Uuid,
    pub(crate) membership_id: Uuid,
    pub(crate) view: GrantView,
}

/// Every admin-class assignment and handover grant held through a usable membership:
/// the inputs to the no-admin predicate (spec 6.4), with ids so the last-admin safeguard
/// can ask "and without this one?".
pub(crate) struct AdminState {
    pub(crate) assignments: Vec<AdminAssignmentRow>,
    pub(crate) grants: Vec<GrantRow>,
}

impl AdminState {
    pub(crate) async fn load(
        conn: &mut PgConnection,
        tenant_id: Uuid,
    ) -> Result<Self, MembershipError> {
        let rows: Vec<(Uuid, Uuid, String, String, bool)> = sqlx::query_as(
            "select ra.id, ra.membership_id,
                    to_char(ra.starts_on, 'YYYY-MM-DD'), to_char(ra.ends_on_exclusive, 'YYYY-MM-DD'),
                    ra.revoked_at is not null
               from role_assignments ra
               join roles r       on r.tenant_id = ra.tenant_id and r.id = ra.role_id
               join memberships m on m.tenant_id = ra.tenant_id and m.id = ra.membership_id
               join accounts a    on a.id = m.account_id
              where ra.tenant_id = $1 and r.capability_class = 'admin'
                and m.revoked_at is null and a.disabled_at is null and a.verified_at is not null",
        )
        .bind(tenant_id)
        .fetch_all(&mut *conn)
        .await?;
        let mut assignments = Vec::with_capacity(rows.len());
        for (id, membership_id, starts, ends, revoked) in rows {
            assignments.push(AdminAssignmentRow {
                id,
                membership_id,
                view: AssignmentView {
                    capability: CapabilityClass::Admin,
                    period: period_from(&starts, &ends)?,
                    revoked,
                },
            });
        }

        let rows: Vec<(Uuid, Uuid, String, String, bool)> = sqlx::query_as(
            "select g.source_assignment_id, ra.membership_id,
                    to_char(g.starts_on, 'YYYY-MM-DD'), to_char(g.ends_on_exclusive, 'YYYY-MM-DD'),
                    g.revoked_at is not null
               from handover_grants g
               join role_assignments ra on ra.tenant_id = g.tenant_id and ra.id = g.source_assignment_id
               join memberships m       on m.tenant_id = ra.tenant_id and m.id = ra.membership_id
               join accounts a          on a.id = m.account_id
              where g.tenant_id = $1 and m.revoked_at is null and a.disabled_at is null",
        )
        .bind(tenant_id)
        .fetch_all(&mut *conn)
        .await?;
        let mut grants = Vec::with_capacity(rows.len());
        for (source_assignment_id, membership_id, starts, ends, revoked) in rows {
            grants.push(GrantRow {
                source_assignment_id,
                membership_id,
                view: GrantView {
                    period: period_from(&starts, &ends)?,
                    revoked,
                },
            });
        }
        Ok(Self {
            assignments,
            grants,
        })
    }

    pub(crate) fn has_admin(&self, today: Date) -> bool {
        let assignments: Vec<AssignmentView> = self.assignments.iter().map(|r| r.view).collect();
        let grants: Vec<GrantView> = self.grants.iter().map(|r| r.view).collect();
        tenant_has_admin(&assignments, &grants, today)
    }
}

/// The last-admin safeguard (spec 7). `removed(assignment_id, membership_id)` names the
/// assignments an action ends; a grant goes with its source assignment. Refuses, unless
/// confirmed, an action that `removal_leaves_no_admin` says would leave the FAU without
/// an admin; returns whether it does, for the audit entry.
pub(crate) fn check_last_admin(
    state: &AdminState,
    today: Date,
    confirm_no_admin: bool,
    removed: impl Fn(Uuid, Uuid) -> bool,
) -> Result<bool, MembershipError> {
    let (mut kept_a, mut gone_a) = (Vec::new(), Vec::new());
    for r in &state.assignments {
        if removed(r.id, r.membership_id) {
            gone_a.push(r.view);
        } else {
            kept_a.push(r.view);
        }
    }
    let (mut kept_g, mut gone_g) = (Vec::new(), Vec::new());
    for r in &state.grants {
        if removed(r.source_assignment_id, r.membership_id) {
            gone_g.push(r.view);
        } else {
            kept_g.push(r.view);
        }
    }
    let leaves_none = removal_leaves_no_admin(&kept_a, &kept_g, &gone_a, &gone_g, today);
    if leaves_none && !confirm_no_admin {
        return Err(MembershipError::WouldLeaveNoAdmin);
    }
    Ok(leaves_none)
}
```

Create (or replace in full) `backend/crates/persistence/src/membership/roles.rs`:

```rust
//! Granting and revoking roles and memberships, with the last-admin safeguard (spec 7).

use fau_domain::membership::period::Period;
use fau_domain::time::Moment;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::error::MembershipError;
use super::invitations::{resolve_roles, OfferedRole, RoleChoice};
use super::sql::{
    check_last_admin, date_param, insert_assignment, is_admin_today, lock_tenant,
    membership_usable, require_admin, require_open, ts_param, write_audit, AdminState, Audit,
};

#[derive(Debug, Clone)]
pub struct GrantRole {
    pub tenant_id: Uuid,
    pub actor_membership_id: Uuid,
    pub membership_id: Uuid,
    pub role: RoleChoice,
    pub period: Period,
}

/// Gives an existing member a role for a period.
///
/// **Gate:** the caller must already have passed #3414's second-factor and freshness
/// gate (#3417). Only an admin valid today may grant; a handover grant does not suffice,
/// which is also what stops an outgoing admin extending their own role (spec 6.3).
pub async fn grant_role(
    pool: &PgPool,
    req: GrantRole,
    at: Moment,
) -> Result<Uuid, MembershipError> {
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, req.tenant_id).await?;
    require_open(&state)?;
    require_admin(&mut tx, req.tenant_id, req.actor_membership_id, at.today()).await?;

    let target: Option<bool> = sqlx::query_scalar(
        "select revoked_at is null from memberships where tenant_id = $1 and id = $2 for update",
    )
    .bind(req.tenant_id)
    .bind(req.membership_id)
    .fetch_optional(&mut *tx)
    .await?;
    match target {
        None => return Err(MembershipError::UnknownMembership),
        Some(false) => return Err(MembershipError::MembershipRevoked),
        Some(true) => {}
    }

    let resolved = resolve_roles(
        &mut tx,
        at,
        req.tenant_id,
        Some(req.actor_membership_id),
        &[OfferedRole {
            role: req.role,
            period: req.period,
        }],
    )
    .await?;
    let (role_id, capability, period) = resolved[0];
    let assignment_id = insert_assignment(
        &mut tx,
        req.tenant_id,
        req.membership_id,
        role_id,
        period,
        Some(req.actor_membership_id),
    )
    .await?;
    write_audit(
        &mut tx,
        at,
        Audit::member(
            req.tenant_id,
            req.actor_membership_id,
            "role.granted",
            "role_assignment",
            assignment_id,
            json!({
                "membership_id": req.membership_id,
                "role_id": role_id,
                "capability_class": capability.code(),
                "starts_on": date_param(period.starts_on()),
                "ends_on_exclusive": date_param(period.ends_on_exclusive()),
            }),
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(assignment_id)
}

#[derive(Debug, Clone, Copy)]
pub struct RevokeAssignment {
    pub tenant_id: Uuid,
    pub actor_membership_id: Uuid,
    pub assignment_id: Uuid,
    /// The explicit confirmation of "FAU-et får da ingen administrator" (spec 7).
    /// Without it, a revocation that would leave no admin is refused.
    pub confirm_no_admin: bool,
}

/// Revokes one role assignment, at once, together with any handover grant it already
/// produced (#3412, spec 6.2). Creates no handover grant (spec 7).
///
/// An admin valid today may revoke any assignment; anyone may revoke their own.
///
/// **Gate:** as [`grant_role`].
pub async fn revoke_role_assignment(
    pool: &PgPool,
    req: RevokeAssignment,
    at: Moment,
) -> Result<(), MembershipError> {
    let mut tx = pool.begin().await?;
    lock_tenant(&mut tx, req.tenant_id).await?;
    let row: Option<(Uuid, bool)> = sqlx::query_as(
        "select membership_id, revoked_at is not null from role_assignments
          where tenant_id = $1 and id = $2 for update",
    )
    .bind(req.tenant_id)
    .bind(req.assignment_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (holder, revoked) = row.ok_or(MembershipError::UnknownAssignment)?;
    if revoked {
        return Err(MembershipError::AssignmentAlreadyRevoked);
    }
    let own = holder == req.actor_membership_id
        && membership_usable(&mut tx, req.tenant_id, req.actor_membership_id).await?;
    if !own && !is_admin_today(&mut tx, req.tenant_id, req.actor_membership_id, at.today()).await? {
        return Err(MembershipError::NotAuthorized);
    }

    let admins = AdminState::load(&mut tx, req.tenant_id).await?;
    let leaves_none = check_last_admin(&admins, at.today(), req.confirm_no_admin, |id, _| {
        id == req.assignment_id
    })?;

    let now = ts_param(at.now());
    sqlx::query(
        "update role_assignments set revoked_at = $3::timestamptz where tenant_id = $1 and id = $2",
    )
    .bind(req.tenant_id)
    .bind(req.assignment_id)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "update handover_grants set revoked_at = $3::timestamptz
          where tenant_id = $1 and source_assignment_id = $2 and revoked_at is null",
    )
    .bind(req.tenant_id)
    .bind(req.assignment_id)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    write_audit(
        &mut tx,
        at,
        Audit::member(
            req.tenant_id,
            req.actor_membership_id,
            "role.revoked",
            "role_assignment",
            req.assignment_id,
            json!({ "membership_id": holder, "left_no_admin": leaves_none }),
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub struct RevokeMembership {
    pub tenant_id: Uuid,
    pub actor_membership_id: Uuid,
    /// Equal to `actor_membership_id` when a member leaves (spec 7).
    pub membership_id: Uuid,
    pub confirm_no_admin: bool,
}

/// Revokes a whole membership: an admin removing someone, or a member leaving. Ends
/// every current and future role the membership holds and every handover grant derived
/// from any of its roles, at once. Other FAU-er the account belongs to are untouched
/// (spec 7, ADR-003 decision 6a).
///
/// **Gate:** as [`grant_role`] when an admin removes someone. Leaving is not a
/// privileged admin action and needs no gate.
pub async fn revoke_membership(
    pool: &PgPool,
    req: RevokeMembership,
    at: Moment,
) -> Result<(), MembershipError> {
    let mut tx = pool.begin().await?;
    lock_tenant(&mut tx, req.tenant_id).await?;
    let target: Option<bool> = sqlx::query_scalar(
        "select revoked_at is not null from memberships where tenant_id = $1 and id = $2 for update",
    )
    .bind(req.tenant_id)
    .bind(req.membership_id)
    .fetch_optional(&mut *tx)
    .await?;
    match target {
        None => return Err(MembershipError::UnknownMembership),
        Some(true) => return Err(MembershipError::MembershipRevoked),
        Some(false) => {}
    }
    let leaving = req.actor_membership_id == req.membership_id;
    if !leaving {
        require_admin(&mut tx, req.tenant_id, req.actor_membership_id, at.today()).await?;
    }

    let admins = AdminState::load(&mut tx, req.tenant_id).await?;
    let leaves_none = check_last_admin(&admins, at.today(), req.confirm_no_admin, |_, m| {
        m == req.membership_id
    })?;

    let now = ts_param(at.now());
    sqlx::query(
        "update memberships set revoked_at = $3::timestamptz where tenant_id = $1 and id = $2",
    )
    .bind(req.tenant_id)
    .bind(req.membership_id)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "update role_assignments set revoked_at = $3::timestamptz
          where tenant_id = $1 and membership_id = $2
            and revoked_at is null and ends_on_exclusive > $4::date",
    )
    .bind(req.tenant_id)
    .bind(req.membership_id)
    .bind(&now)
    .bind(date_param(at.today()))
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "update handover_grants g set revoked_at = $3::timestamptz
           from role_assignments ra
          where g.tenant_id = $1 and ra.tenant_id = g.tenant_id
            and ra.id = g.source_assignment_id and ra.membership_id = $2
            and g.revoked_at is null",
    )
    .bind(req.tenant_id)
    .bind(req.membership_id)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    write_audit(
        &mut tx,
        at,
        Audit::member(
            req.tenant_id,
            req.actor_membership_id,
            if leaving {
                "membership.left"
            } else {
                "membership.revoked"
            },
            "membership",
            req.membership_id,
            json!({ "left_no_admin": leaves_none }),
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}
```

Create (or replace in full) `backend/crates/persistence/src/membership/mod.rs`:

```rust
//! The membership foundation's transactions (#3413 flow spec; #3417, #3418). Each public
//! function is one transaction that writes its state change together with its audit
//! entry and, where the spec says so, its outbox message (spec 2.6).
//!
//! Every function takes a [`fau_domain::time::Moment`]: rule-deciding timestamps and
//! "today" come from the caller, never from the database clock.

mod error;
mod invitations;
mod requests;
mod roles;
mod signup;
mod sql;
mod token;

pub use error::{ExistingFau, MembershipError};
pub use invitations::{
    accept_invitation, issue_invitation, resend_invitation, withdraw_invitation, AcceptInvitation,
    Accepted, InvitationChange, IssueInvitation, IssuedInvitation, OfferedRole, RoleChoice,
};
pub use requests::{
    approve_request, create_access_request, create_replacement_proposal, decline_request,
    lapse_requests, CreateAccessRequest, CreateReplacementProposal, RequestDecision,
};
pub use roles::{
    grant_role, revoke_membership, revoke_role_assignment, GrantRole, RevokeAssignment,
    RevokeMembership,
};
pub use signup::{
    activate_tenant, create_pending_tenant, expire_pending_tenants, Activated, Activation,
    PendingSignup, PendingTenant,
};
pub use token::InvitationToken;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test --workspace`
Expected: PASS, and `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings` are clean.

- [ ] **Step 5: Commit** (the controller commits; executors do not)

```bash
git add backend/crates/domain/src/membership/access.rs backend/crates/persistence/src/membership backend/crates/app/tests/roles.rs
git commit -m "Add role grants, revocation and the last-admin safeguard (#3413, #3418)"
```

---

### Task 9: Persistence: handover grants and the recovery contact's admin grant

`create_handover_grants` is the scheduled sweep: every naturally ended admin role on an active FAU gets one grant `[end, handover_boundary(end))`, computed in Rust and stored; revoked roles get none; a late sweep still starts the window at the role's end; a window already over is skipped; re-running creates nothing. `recovery_grant_admin` is the recovery contact's single power in the no-admin state: a `recovery` invitation offering an admin-class role, refused outside that state, refused to anyone not holding the seat, refused to the contact's own address. It writes the permanent audit entry and a notice to every current member (or, with none left, to everyone who held a role in the past 24 months), always copied to EWB as second party; acceptance sends the second notice (already wired in Task 6). This task also carries the end-to-end tests for handover invitations issued in Task 6.

**Files:**
- Modify: `backend/crates/persistence/src/membership/sql.rs`
- Create: `backend/crates/persistence/src/membership/handover.rs`
- Replace in full: `backend/crates/persistence/src/membership/mod.rs`
- Create: `backend/crates/app/tests/handover_recovery.rs`

**Interfaces:**
- Consumes: `handover_period`, `AdminState`, `recovery_notice_recipients`, `insert_invitation`, `resolve_roles`.
- Produces:
  - `create_handover_grants(&PgPool, Moment) -> Result<u64, _>`.
  - `recovery_grant_admin(&PgPool, RecoveryGrant, Moment) -> Result<IssuedInvitation, _>`; `RecoveryGrant { tenant_id, actor: RecoveryActor, recipient: Email, role: RoleChoice, period: Period }`; `RecoveryActor::{Ewb, SchoolRep(VerifiedEmail)}`.
  - `ActorKind::{RecoveryEwb, RecoverySchoolRep}`.

- [ ] **Step 1: Write the failing tests**

Create (or replace in full) `backend/crates/app/tests/handover_recovery.rs`:

```rust
//! Handover grants and the no-admin recovery path (flow spec §6.2–6.4; §10 "Handover
//! and the no-admin state").

mod common;
use common::membership::*;
use common::TestDb;
use fau_domain::membership::acceptance::AcceptanceRefusal;
use fau_domain::membership::vocabulary::CapabilityClass;
use fau_domain::time::Moment;
use fau_persistence::membership::{
    accept_invitation, approve_request, create_access_request, create_handover_grants, grant_role,
    issue_invitation, recovery_grant_admin, revoke_membership, revoke_role_assignment,
    AcceptInvitation, CreateAccessRequest, GrantRole, IssueInvitation, MembershipError,
    OfferedRole, RecoveryActor, RecoveryGrant, RequestDecision, RevokeAssignment, RevokeMembership,
    RoleChoice,
};
use sqlx::PgPool;
use uuid::Uuid;

async fn grant_for(pool: &PgPool, assignment_id: Uuid) -> Option<(Uuid, String, String, bool)> {
    sqlx::query_as(
        "select id, starts_on::text, ends_on_exclusive::text, revoked_at is not null
           from handover_grants where source_assignment_id = $1",
    )
    .bind(assignment_id)
    .fetch_optional(pool)
    .await
    .unwrap()
}

fn accept(token: &str, acceptor: &str) -> AcceptInvitation {
    AcceptInvitation {
        token: token.to_owned(),
        acceptor: verified(acceptor),
        admin_end_override: None,
    }
}

#[tokio::test]
async fn a_naturally_ended_admin_role_gets_a_six_month_grant() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;

    assert_eq!(
        create_handover_grants(&pool, at("2027-09-30T10:00:00Z"))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        create_handover_grants(&pool, at("2027-10-01T10:00:00Z"))
            .await
            .unwrap(),
        1
    );
    let (_, starts, ends, revoked) = grant_for(&pool, fau.admin_assignment_id).await.unwrap();
    assert_eq!(
        (starts.as_str(), ends.as_str(), revoked),
        ("2027-10-01", "2028-04-01", false)
    );
    assert_eq!(
        create_handover_grants(&pool, at("2027-10-02T10:00:00Z"))
            .await
            .unwrap(),
        0,
        "one grant per source assignment"
    );
    assert_eq!(audit_count(&pool, "handover.granted").await, 1);
}

#[tokio::test]
async fn the_grant_boundary_truncates_to_the_end_of_february() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let short = add_member(
        &pool,
        &fau,
        "kort@example.test",
        RoleChoice::Existing(fau.admin_role_id),
        period(day(2026, 9, 23), day(2027, 8, 31)),
        t0,
    )
    .await;

    create_handover_grants(&pool, at("2027-09-15T10:00:00Z"))
        .await
        .unwrap();
    let (_, starts, ends, _) = grant_for(&pool, short.assignment_ids[0]).await.unwrap();
    assert_eq!(
        (starts.as_str(), ends.as_str()),
        ("2027-08-31", "2028-02-29"),
        "a late sweep still starts the window at the role's end"
    );
}

#[tokio::test]
async fn a_revoked_admin_role_gets_no_grant_and_revocation_ends_an_existing_one() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let revoked_early = add_member(
        &pool,
        &fau,
        "a@example.test",
        RoleChoice::Existing(fau.admin_role_id),
        period(day(2026, 9, 23), day(2027, 3, 1)),
        t0,
    )
    .await;
    let ends_naturally = add_member(
        &pool,
        &fau,
        "b@example.test",
        RoleChoice::Existing(fau.admin_role_id),
        period(day(2026, 9, 23), day(2027, 3, 1)),
        t0,
    )
    .await;
    revoke_role_assignment(
        &pool,
        RevokeAssignment {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            assignment_id: revoked_early.assignment_ids[0],
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .unwrap();

    let t1 = at("2027-03-01T10:00:00Z");
    assert_eq!(create_handover_grants(&pool, t1).await.unwrap(), 1);
    assert!(grant_for(&pool, revoked_early.assignment_ids[0])
        .await
        .is_none());
    assert!(grant_for(&pool, ends_naturally.assignment_ids[0])
        .await
        .is_some());

    revoke_role_assignment(
        &pool,
        RevokeAssignment {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            assignment_id: ends_naturally.assignment_ids[0],
            confirm_no_admin: false,
        },
        t1,
    )
    .await
    .unwrap();
    let (_, _, _, revoked) = grant_for(&pool, ends_naturally.assignment_ids[0])
        .await
        .unwrap();
    assert!(
        revoked,
        "revoking the role later ends the grant it produced"
    );
}

/// An FAU whose registrant's admin role ended on 2027-10-01 and whose grant exists; a
/// member role keeps the registrant in the FAU. Returns the FAU, the grant id and the
/// moment inside the window.
async fn outgoing_admin(pool: &PgPool) -> (Fau, Uuid, Moment) {
    let t0 = at(T0);
    let fau = active_fau(pool, "gammel@example.test", t0).await;
    grant_role(
        pool,
        GrantRole {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: fau.admin_membership_id,
            role: new_role("Medlem", CapabilityClass::Member),
            period: period(day(2026, 9, 23), day(2028, 9, 1)),
        },
        t0,
    )
    .await
    .unwrap();
    let inside = at("2027-11-01T10:00:00Z");
    create_handover_grants(pool, inside).await.unwrap();
    let (grant_id, ..) = grant_for(pool, fau.admin_assignment_id).await.unwrap();
    (fau, grant_id, inside)
}

fn handover_invite(
    fau: &Fau,
    grant_id: Uuid,
    recipient: &str,
    role: RoleChoice,
) -> IssueInvitation {
    IssueInvitation {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        recipient: email(recipient),
        roles: vec![OfferedRole {
            role,
            period: period(day(2027, 11, 1), day(2028, 10, 1)),
        }],
        handover_grant_id: Some(grant_id),
    }
}

#[tokio::test]
async fn an_outgoing_admin_brings_in_a_replacement_who_becomes_an_ordinary_admin() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let (fau, grant_id, inside) = outgoing_admin(&pool).await;

    let issued = issue_invitation(
        &pool,
        handover_invite(
            &fau,
            grant_id,
            "ny@example.test",
            RoleChoice::Existing(fau.admin_role_id),
        ),
        inside,
    )
    .await
    .unwrap();
    let mode: String = sqlx::query_scalar("select mode from invitations where id = $1")
        .bind(issued.invitation_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(mode, "handover");
    let accepted = accept_invitation(
        &pool,
        accept(issued.token.expose(), "ny@example.test"),
        inside,
    )
    .await
    .unwrap();

    // The new admin acts as any admin: here, granting a role.
    grant_role(
        &pool,
        GrantRole {
            tenant_id: fau.tenant_id,
            actor_membership_id: accepted.membership_id,
            membership_id: accepted.membership_id,
            role: new_role("Kasserer", CapabilityClass::Member),
            period: period(day(2027, 11, 1), day(2028, 10, 1)),
        },
        inside,
    )
    .await
    .expect("the replacement is an ordinary admin");
}

#[tokio::test]
async fn handover_allows_nothing_else() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let (fau, grant_id, inside) = outgoing_admin(&pool).await;

    // Not extending their own role.
    assert_eq!(
        grant_role(
            &pool,
            GrantRole {
                tenant_id: fau.tenant_id,
                actor_membership_id: fau.admin_membership_id,
                membership_id: fau.admin_membership_id,
                role: RoleChoice::Existing(fau.admin_role_id),
                period: period(day(2027, 11, 1), day(2028, 10, 1)),
            },
            inside,
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized
    );
    // Not inviting themselves.
    assert_eq!(
        issue_invitation(
            &pool,
            handover_invite(
                &fau,
                grant_id,
                "gammel@example.test",
                RoleChoice::Existing(fau.admin_role_id)
            ),
            inside,
        )
        .await
        .unwrap_err(),
        MembershipError::SelfInvitation
    );
    // Not editing the organisation.
    assert_eq!(
        issue_invitation(
            &pool,
            handover_invite(
                &fau,
                grant_id,
                "ny@example.test",
                new_role("Ny rolle", CapabilityClass::Member)
            ),
            inside,
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized
    );
    // Not issuing ordinary invitations, and not approving requests.
    assert_eq!(
        issue_invitation(
            &pool,
            IssueInvitation {
                handover_grant_id: None,
                ..handover_invite(
                    &fau,
                    grant_id,
                    "ny@example.test",
                    RoleChoice::Existing(fau.admin_role_id)
                )
            },
            inside,
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized
    );
    let request_id = create_access_request(
        &pool,
        CreateAccessRequest {
            tenant_id: fau.tenant_id,
            requester: verified("sporsmal@example.test"),
            message: None,
        },
        inside,
    )
    .await
    .unwrap();
    assert_eq!(
        approve_request(
            &pool,
            RequestDecision {
                tenant_id: fau.tenant_id,
                actor_membership_id: fau.admin_membership_id,
                request_id,
            },
            vec![OfferedRole {
                role: RoleChoice::Existing(fau.admin_role_id),
                period: period(day(2027, 11, 1), day(2028, 10, 1)),
            }],
            inside,
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized
    );
    // Not revoking anyone else's role.
    let member = add_member(
        &pool,
        &fau,
        "medlem@example.test",
        new_role("Kasserer", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2028, 9, 1)),
        at(T0),
    )
    .await;
    assert_eq!(
        revoke_role_assignment(
            &pool,
            RevokeAssignment {
                tenant_id: fau.tenant_id,
                actor_membership_id: fau.admin_membership_id,
                assignment_id: member.assignment_ids[0],
                confirm_no_admin: true,
            },
            inside,
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized
    );
    // Not after the boundary.
    assert_eq!(
        issue_invitation(
            &pool,
            handover_invite(
                &fau,
                grant_id,
                "ny@example.test",
                RoleChoice::Existing(fau.admin_role_id)
            ),
            at("2028-04-01T10:00:00Z"),
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized
    );
}

#[tokio::test]
async fn a_handover_invitation_dies_with_its_grant() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let (fau, grant_id, inside) = outgoing_admin(&pool).await;
    let issued = issue_invitation(
        &pool,
        handover_invite(
            &fau,
            grant_id,
            "ny@example.test",
            RoleChoice::Existing(fau.admin_role_id),
        ),
        inside,
    )
    .await
    .unwrap();
    sqlx::query("update handover_grants set revoked_at = now() where id = $1")
        .bind(grant_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();
    assert_eq!(
        accept_invitation(
            &pool,
            accept(issued.token.expose(), "ny@example.test"),
            inside
        )
        .await
        .unwrap_err(),
        MembershipError::Acceptance(AcceptanceRefusal::IssuerLacksAuthority)
    );
}

/// An FAU in the no-admin state with one ordinary member left.
async fn without_admin(pool: &PgPool) -> Fau {
    let t0 = at(T0);
    let fau = active_fau(pool, "admin@example.test", t0).await;
    add_member(
        pool,
        &fau,
        "medlem@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    revoke_membership(
        pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: fau.admin_membership_id,
            confirm_no_admin: true,
        },
        t0,
    )
    .await
    .unwrap();
    fau
}

fn recovery(fau: &Fau, actor: RecoveryActor, recipient: &str, role: RoleChoice) -> RecoveryGrant {
    RecoveryGrant {
        tenant_id: fau.tenant_id,
        actor,
        recipient: email(recipient),
        role,
        period: period(day(2026, 9, 23), day(2027, 10, 1)),
    }
}

#[tokio::test]
async fn in_the_no_admin_state_the_recovery_contact_can_grant_admin() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = without_admin(&pool).await;

    let issued = recovery_grant_admin(
        &pool,
        recovery(
            &fau,
            RecoveryActor::Ewb,
            "ny-leder@example.test",
            RoleChoice::Existing(fau.admin_role_id),
        ),
        t0,
    )
    .await
    .unwrap();
    assert_eq!(audit_count(&pool, "recovery.admin_invited").await, 1);
    assert_eq!(
        outbox_count(&pool, "recovery.invitation_created", "medlem@example.test").await,
        1
    );
    assert_eq!(
        outbox_count(&pool, "recovery.invitation_created", "fau@ewb-solutions.as").await,
        1
    );

    accept_invitation(
        &pool,
        accept(issued.token.expose(), "ny-leder@example.test"),
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        outbox_count(&pool, "recovery.invitation_accepted", "medlem@example.test").await,
        1
    );
    assert_eq!(
        outbox_count(
            &pool,
            "recovery.invitation_accepted",
            "fau@ewb-solutions.as"
        )
        .await,
        1
    );

    // An admin exists again, so the recovery contact's power is gone.
    assert_eq!(
        recovery_grant_admin(
            &pool,
            recovery(
                &fau,
                RecoveryActor::Ewb,
                "tredje@example.test",
                RoleChoice::Existing(fau.admin_role_id)
            ),
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::NotInNoAdminState
    );
}

#[tokio::test]
async fn outside_the_no_admin_state_the_recovery_contact_cannot_grant_admin() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    assert_eq!(
        recovery_grant_admin(
            &pool,
            recovery(
                &fau,
                RecoveryActor::Ewb,
                "ny@example.test",
                RoleChoice::Existing(fau.admin_role_id)
            ),
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::NotInNoAdminState
    );
}

#[tokio::test]
async fn only_the_seat_holder_acts_and_never_for_itself() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = without_admin(&pool).await;
    let admin_role = || RoleChoice::Existing(fau.admin_role_id);

    assert_eq!(
        recovery_grant_admin(
            &pool,
            recovery(
                &fau,
                RecoveryActor::SchoolRep(verified("rektor@skole.example.test")),
                "ny@example.test",
                admin_role(),
            ),
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized,
        "an unseated school representative has no power"
    );
    assert_eq!(
        recovery_grant_admin(
            &pool,
            recovery(
                &fau,
                RecoveryActor::Ewb,
                "fau@ewb-solutions.as",
                admin_role()
            ),
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::SelfInvitation
    );
    assert_eq!(
        recovery_grant_admin(
            &pool,
            recovery(
                &fau,
                RecoveryActor::Ewb,
                "ny@example.test",
                new_role("Medlem", CapabilityClass::Member)
            ),
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::NotAdminRole
    );
}

#[tokio::test]
async fn with_no_members_left_recent_role_holders_are_told() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "sist@example.test", t0).await;
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: fau.admin_membership_id,
            confirm_no_admin: true,
        },
        t0,
    )
    .await
    .unwrap();

    recovery_grant_admin(
        &pool,
        recovery(
            &fau,
            RecoveryActor::Ewb,
            "ny@example.test",
            RoleChoice::Existing(fau.admin_role_id),
        ),
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        outbox_count(&pool, "recovery.invitation_created", "sist@example.test").await,
        1
    );
    assert_eq!(
        outbox_count(&pool, "recovery.invitation_created", "fau@ewb-solutions.as").await,
        1
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test -p fau-app --test handover_recovery`
Expected: compile error: `create_handover_grants`, `recovery_grant_admin`, `RecoveryActor` not found.

- [ ] **Step 3: Implement**

In `backend/crates/persistence/src/membership/sql.rs`, replace:

```rust
/// Who performed an audited action. Matches `audit_events.actor_kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActorKind {
    Member,
    System,
    Registrant,
    Requester,
}

impl ActorKind {
    fn code(self) -> &'static str {
        match self {
            ActorKind::Member => "member",
            ActorKind::System => "system",
            ActorKind::Registrant => "registrant",
            ActorKind::Requester => "requester",
        }
    }
}
```

with:

```rust
/// Who performed an audited action. Matches `audit_events.actor_kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActorKind {
    Member,
    System,
    Registrant,
    Requester,
    RecoveryEwb,
    RecoverySchoolRep,
}

impl ActorKind {
    fn code(self) -> &'static str {
        match self {
            ActorKind::Member => "member",
            ActorKind::System => "system",
            ActorKind::Registrant => "registrant",
            ActorKind::Requester => "requester",
            ActorKind::RecoveryEwb => "recovery_ewb",
            ActorKind::RecoverySchoolRep => "recovery_school_rep",
        }
    }
}
```

Create (or replace in full) `backend/crates/persistence/src/membership/handover.rs`:

```rust
//! Handover grants (spec 6.2) and the recovery contact's admin grant (spec 6.4).

use fau_domain::email::{Email, VerifiedEmail};
use fau_domain::membership::period::Period;
use fau_domain::membership::rules::{handover_period, EWB_OVERSIGHT_ADDRESS};
use fau_domain::membership::vocabulary::{CapabilityClass, InvitationMode, RecoveryHolder};
use fau_domain::time::Moment;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::error::MembershipError;
use super::invitations::{
    insert_invitation, resolve_roles, IssuedInvitation, NewInvitation, OfferedRole, RoleChoice,
};
use super::sql::{
    date_param, enqueue, lock_tenant, parse_date, recovery_notice_recipients, require_open,
    write_audit, ActorKind, AdminState, Audit,
};

/// The scheduled sweep: gives every naturally ended admin role its handover grant
/// (spec 6.2). A role that was revoked gets none. Idempotent -- a source assignment has
/// at most one grant -- and late-safe: a grant created days after the role ended still
/// runs from the role's end, and a window already over is skipped. Returns how many
/// grants were created.
pub async fn create_handover_grants(pool: &PgPool, at: Moment) -> Result<u64, MembershipError> {
    let mut tx = pool.begin().await?;
    let candidates: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
        "select ra.tenant_id, ra.id, to_char(ra.ends_on_exclusive, 'YYYY-MM-DD')
           from role_assignments ra
           join roles r       on r.tenant_id = ra.tenant_id and r.id = ra.role_id
           join memberships m on m.tenant_id = ra.tenant_id and m.id = ra.membership_id
           join tenants t     on t.id = ra.tenant_id
          where r.capability_class = 'admin' and t.status = 'active'
            and ra.revoked_at is null and m.revoked_at is null
            and ra.ends_on_exclusive <= $1::date
            and not exists (select 1 from handover_grants g
                             where g.tenant_id = ra.tenant_id and g.source_assignment_id = ra.id)",
    )
    .bind(date_param(at.today()))
    .fetch_all(&mut *tx)
    .await?;

    let mut created = 0;
    for (tenant_id, assignment_id, ends) in candidates {
        let Some(window) = handover_period(parse_date(&ends)?) else {
            continue;
        };
        if window.has_ended_by(at.today()) {
            continue;
        }
        let grant_id: Option<Uuid> = sqlx::query_scalar(
            "insert into handover_grants
               (tenant_id, id, source_assignment_id, starts_on, ends_on_exclusive)
             values ($1, $2, $3, $4::date, $5::date)
             on conflict (tenant_id, source_assignment_id) do nothing
             returning id",
        )
        .bind(tenant_id)
        .bind(Uuid::now_v7())
        .bind(assignment_id)
        .bind(date_param(window.starts_on()))
        .bind(date_param(window.ends_on_exclusive()))
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(grant_id) = grant_id {
            write_audit(
                &mut tx,
                at,
                Audit::system(
                    tenant_id,
                    "handover.granted",
                    "handover_grant",
                    grant_id,
                    json!({
                        "source_assignment_id": assignment_id,
                        "ends_on_exclusive": date_param(window.ends_on_exclusive()),
                    }),
                ),
            )
            .await?;
            created += 1;
        }
    }
    tx.commit().await?;
    Ok(created)
}

/// Who is acting as the recovery contact. How EWB's operator and a school
/// representative authenticate is #3417's; this names which seat they claim.
#[derive(Debug, Clone)]
pub enum RecoveryActor {
    Ewb,
    SchoolRep(VerifiedEmail),
}

#[derive(Debug, Clone)]
pub struct RecoveryGrant {
    pub tenant_id: Uuid,
    pub actor: RecoveryActor,
    pub recipient: Email,
    /// Must resolve to an `admin`-class role.
    pub role: RoleChoice,
    pub period: Period,
}

/// The recovery contact's one power in the no-admin state (spec 6.4, decision 10):
/// issues a `recovery` invitation offering an admin-class role. Refused unless the
/// actor holds the seat and the FAU has no admin today. The recovery contact cannot
/// invite itself (ADR-003 decision 8).
///
/// Writes a permanent audit entry and a notice to every current member -- or, with no
/// members left, to recent role-holders -- and always to EWB as the second party. The
/// same notice goes out again on acceptance (`accept_invitation`). The 14-day login
/// banner is read from these audit entries by the UI (#3422).
///
/// **Gate:** #3414's step-up re-authentication applies (ADR-003 decision 10) and is the
/// HTTP layer's (#3417).
pub async fn recovery_grant_admin(
    pool: &PgPool,
    req: RecoveryGrant,
    at: Moment,
) -> Result<IssuedInvitation, MembershipError> {
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, req.tenant_id).await?;
    require_open(&state)?;

    let seat: Option<(String, Option<String>)> = sqlx::query_as(
        "select holder, nominee_email from recovery_contacts where tenant_id = $1 for update",
    )
    .bind(req.tenant_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (holder, nominee) = seat.ok_or(MembershipError::NotAuthorized)?;
    let holder = RecoveryHolder::from_code(&holder).ok_or_else(MembershipError::decode)?;
    let (actor_kind, own_address) = match (&req.actor, holder) {
        (RecoveryActor::Ewb, RecoveryHolder::Ewb) => {
            (ActorKind::RecoveryEwb, EWB_OVERSIGHT_ADDRESS.to_owned())
        }
        (RecoveryActor::SchoolRep(email), RecoveryHolder::SchoolRep)
            if nominee.as_deref() == Some(email.email().as_str()) =>
        {
            (
                ActorKind::RecoverySchoolRep,
                email.email().as_str().to_owned(),
            )
        }
        _ => return Err(MembershipError::NotAuthorized),
    };
    if req.recipient.as_str() == own_address {
        return Err(MembershipError::SelfInvitation);
    }
    if AdminState::load(&mut tx, req.tenant_id)
        .await?
        .has_admin(at.today())
    {
        return Err(MembershipError::NotInNoAdminState);
    }

    let resolved = resolve_roles(
        &mut tx,
        at,
        req.tenant_id,
        None,
        &[OfferedRole {
            role: req.role,
            period: req.period,
        }],
    )
    .await?;
    let (role_id, capability, period) = resolved[0];
    if capability != CapabilityClass::Admin {
        return Err(MembershipError::NotAdminRole);
    }

    let recipients = recovery_notice_recipients(&mut tx, req.tenant_id, at.today()).await?;
    let issued = insert_invitation(
        &mut tx,
        at,
        NewInvitation {
            tenant_id: req.tenant_id,
            mode: InvitationMode::Recovery,
            recipient: req.recipient,
            issued_by: None,
            handover_grant_id: None,
            access_request_id: None,
            recovery_holder: Some(holder),
            roles: vec![(role_id, period)],
            actor_kind,
            actor_membership_id: None,
        },
    )
    .await?;
    write_audit(
        &mut tx,
        at,
        Audit {
            tenant_id: Some(req.tenant_id),
            actor_kind,
            actor_account_id: None,
            actor_membership_id: None,
            action: "recovery.admin_invited",
            subject_type: "invitation",
            subject_id: issued.invitation_id,
            params: json!({ "holder": holder.code(), "role_id": role_id }),
        },
    )
    .await?;
    for recipient in &recipients {
        enqueue(
            &mut tx,
            at,
            "recovery.invitation_created",
            recipient,
            json!({ "tenant_id": req.tenant_id, "invitation_id": issued.invitation_id }),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(issued)
}
```

Create (or replace in full) `backend/crates/persistence/src/membership/mod.rs`:

```rust
//! The membership foundation's transactions (#3413 flow spec; #3417, #3418). Each public
//! function is one transaction that writes its state change together with its audit
//! entry and, where the spec says so, its outbox message (spec 2.6).
//!
//! Every function takes a [`fau_domain::time::Moment`]: rule-deciding timestamps and
//! "today" come from the caller, never from the database clock.

mod error;
mod handover;
mod invitations;
mod requests;
mod roles;
mod signup;
mod sql;
mod token;

pub use error::{ExistingFau, MembershipError};
pub use handover::{create_handover_grants, recovery_grant_admin, RecoveryActor, RecoveryGrant};
pub use invitations::{
    accept_invitation, issue_invitation, resend_invitation, withdraw_invitation, AcceptInvitation,
    Accepted, InvitationChange, IssueInvitation, IssuedInvitation, OfferedRole, RoleChoice,
};
pub use requests::{
    approve_request, create_access_request, create_replacement_proposal, decline_request,
    lapse_requests, CreateAccessRequest, CreateReplacementProposal, RequestDecision,
};
pub use roles::{
    grant_role, revoke_membership, revoke_role_assignment, GrantRole, RevokeAssignment,
    RevokeMembership,
};
pub use signup::{
    activate_tenant, create_pending_tenant, expire_pending_tenants, Activated, Activation,
    PendingSignup, PendingTenant,
};
pub use token::InvitationToken;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test --workspace`
Expected: PASS, and `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings` are clean.

- [ ] **Step 5: Commit** (the controller commits; executors do not)

```bash
git add backend/crates/persistence/src/membership backend/crates/app/tests/handover_recovery.rs
git commit -m "Add handover grants and the recovery contact's admin grant (#3413, #3418)"
```

---

### Task 10: Persistence: effective access

The per-request check #3417 will call: reads the standing, the membership's assignments and its grants fresh from the database and applies Task 3's `evaluate_access` with today's Oslo date. An unknown account, tenant or membership is simply `Access::NONE`, so the answer never reveals which one is missing. A freeze keeps read access (ADR-003 decision 7a).

**Files:**
- Create: `backend/crates/persistence/src/membership/access.rs`
- Replace in full: `backend/crates/persistence/src/membership/mod.rs`
- Create: `backend/crates/app/tests/access.rs`

**Interfaces:**
- Consumes: `evaluate_access`, `period_from`.
- Produces: `effective_access(&PgPool, account_id: Uuid, tenant_id: Uuid, Moment) -> Result<Access, MembershipError>`.

- [ ] **Step 1: Write the failing tests**

Create (or replace in full) `backend/crates/app/tests/access.rs`:

```rust
//! The per-request access check (flow spec §2.1, §4; §10 "Access").

mod common;
use common::membership::*;
use common::TestDb;
use fau_domain::membership::access::{Access, Capability};
use fau_domain::membership::vocabulary::CapabilityClass;
use fau_persistence::membership::{
    create_handover_grants, effective_access, revoke_role_assignment, RevokeAssignment,
};
use uuid::Uuid;

#[tokio::test]
async fn the_registrant_is_an_admin_until_the_chosen_end_date() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;

    let during = effective_access(&pool, fau.admin_account_id, fau.tenant_id, at(T0))
        .await
        .unwrap();
    assert_eq!(during.capability, Capability::Admin);
    let last_day = at("2027-09-30T21:59:59Z");
    assert_eq!(
        effective_access(&pool, fau.admin_account_id, fau.tenant_id, last_day)
            .await
            .unwrap()
            .capability,
        Capability::Admin
    );
    let midnight = at("2027-09-30T22:00:00Z");
    assert_eq!(
        effective_access(&pool, fau.admin_account_id, fau.tenant_id, midnight)
            .await
            .unwrap(),
        Access::NONE,
        "access ends at Oslo midnight whether or not any job has run"
    );
}

#[tokio::test]
async fn a_role_valid_tomorrow_grants_nothing_today() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let member = add_member(
        &pool,
        &fau,
        "snart@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 24), day(2027, 9, 1)),
        t0,
    )
    .await;

    let today = effective_access(&pool, member.account_id, fau.tenant_id, t0)
        .await
        .unwrap();
    assert_eq!(today.capability, Capability::None);
    assert_eq!(today.next_start, Some(day(2026, 9, 24)));
    let tomorrow = effective_access(
        &pool,
        member.account_id,
        fau.tenant_id,
        at("2026-09-24T10:00:00Z"),
    )
    .await
    .unwrap();
    assert_eq!(tomorrow.capability, Capability::Member);
}

#[tokio::test]
async fn losing_the_last_valid_role_removes_access_on_the_next_check() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let member = add_member(
        &pool,
        &fau,
        "medlem@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    assert_eq!(
        effective_access(&pool, member.account_id, fau.tenant_id, t0)
            .await
            .unwrap()
            .capability,
        Capability::Member
    );

    revoke_role_assignment(
        &pool,
        RevokeAssignment {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            assignment_id: member.assignment_ids[0],
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        effective_access(&pool, member.account_id, fau.tenant_id, t0)
            .await
            .unwrap(),
        Access::NONE
    );
}

#[tokio::test]
async fn a_handover_grant_alone_is_reported_without_capability() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    let inside = at("2027-10-01T10:00:00Z");
    create_handover_grants(&pool, inside).await.unwrap();

    let access = effective_access(&pool, fau.admin_account_id, fau.tenant_id, inside)
        .await
        .unwrap();
    assert_eq!(
        access.capability,
        Capability::None,
        "no document access (spec 6.3)"
    );
    assert!(access.handover);
}

#[tokio::test]
async fn nothing_is_revealed_about_other_faus_or_unknown_ids() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let a = active_fau(&pool, "a@example.test", t0).await;
    let b = active_fau(&pool, "b@example.test", t0).await;

    for (account, tenant) in [
        (a.admin_account_id, b.tenant_id),
        (Uuid::now_v7(), a.tenant_id),
        (a.admin_account_id, Uuid::now_v7()),
    ] {
        assert_eq!(
            effective_access(&pool, account, tenant, t0).await.unwrap(),
            Access::NONE
        );
    }
}

#[tokio::test]
async fn a_disabled_account_has_no_access_and_a_frozen_fau_keeps_it() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;

    sqlx::query("update tenants set frozen_at = now() where id = $1")
        .bind(fau.tenant_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();
    assert_eq!(
        effective_access(&pool, fau.admin_account_id, fau.tenant_id, t0)
            .await
            .unwrap()
            .capability,
        Capability::Admin,
        "reads continue during a freeze (ADR-003 decision 7a)"
    );

    sqlx::query("update accounts set disabled_at = now() where id = $1")
        .bind(fau.admin_account_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();
    assert_eq!(
        effective_access(&pool, fau.admin_account_id, fau.tenant_id, t0)
            .await
            .unwrap(),
        Access::NONE
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test -p fau-app --test access`
Expected: compile error: `effective_access` not found.

- [ ] **Step 3: Implement**

Create (or replace in full) `backend/crates/persistence/src/membership/access.rs`:

```rust
//! The per-request access check (spec 2.1, 4; #3417 calls it on every request).

use fau_domain::membership::access::{
    evaluate_access, Access, AssignmentView, GrantView, Standing,
};
use fau_domain::membership::vocabulary::CapabilityClass;
use fau_domain::time::Moment;
use sqlx::PgPool;
use uuid::Uuid;

use super::error::MembershipError;
use super::sql::period_from;

/// What `account_id` may do in `tenant_id` today, read fresh from the database. An
/// unknown account, tenant or membership is simply no access -- the answer never
/// reveals which of them is missing (spec 2.7).
pub async fn effective_access(
    pool: &PgPool,
    account_id: Uuid,
    tenant_id: Uuid,
    at: Moment,
) -> Result<Access, MembershipError> {
    let mut tx = pool.begin().await?;
    let standing: Option<(bool, bool, Option<Uuid>, bool)> = sqlx::query_as(
        "select t.status = 'active',
                a.disabled_at is null and a.verified_at is not null,
                m.id,
                coalesce(m.revoked_at is null, false)
           from tenants t
           cross join accounts a
           left join memberships m on m.tenant_id = t.id and m.account_id = a.id
          where t.id = $1 and a.id = $2",
    )
    .bind(tenant_id)
    .bind(account_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((tenant_active, account_usable, Some(membership_id), membership_active)) = standing
    else {
        return Ok(Access::NONE);
    };

    let rows: Vec<(String, String, String, bool)> = sqlx::query_as(
        "select r.capability_class,
                to_char(ra.starts_on, 'YYYY-MM-DD'), to_char(ra.ends_on_exclusive, 'YYYY-MM-DD'),
                ra.revoked_at is not null
           from role_assignments ra
           join roles r on r.tenant_id = ra.tenant_id and r.id = ra.role_id
          where ra.tenant_id = $1 and ra.membership_id = $2",
    )
    .bind(tenant_id)
    .bind(membership_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut assignments = Vec::with_capacity(rows.len());
    for (class, starts, ends, revoked) in rows {
        assignments.push(AssignmentView {
            capability: CapabilityClass::from_code(&class).ok_or_else(MembershipError::decode)?,
            period: period_from(&starts, &ends)?,
            revoked,
        });
    }

    let rows: Vec<(String, String, bool)> = sqlx::query_as(
        "select to_char(g.starts_on, 'YYYY-MM-DD'), to_char(g.ends_on_exclusive, 'YYYY-MM-DD'),
                g.revoked_at is not null
           from handover_grants g
           join role_assignments ra on ra.tenant_id = g.tenant_id and ra.id = g.source_assignment_id
          where g.tenant_id = $1 and ra.membership_id = $2",
    )
    .bind(tenant_id)
    .bind(membership_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut grants = Vec::with_capacity(rows.len());
    for (starts, ends, revoked) in rows {
        grants.push(GrantView {
            period: period_from(&starts, &ends)?,
            revoked,
        });
    }
    tx.commit().await?;

    Ok(evaluate_access(
        Standing {
            tenant_active,
            account_usable,
            membership_active,
        },
        &assignments,
        &grants,
        at.today(),
    ))
}
```

Create (or replace in full) `backend/crates/persistence/src/membership/mod.rs`:

```rust
//! The membership foundation's transactions (#3413 flow spec; #3417, #3418). Each public
//! function is one transaction that writes its state change together with its audit
//! entry and, where the spec says so, its outbox message (spec 2.6).
//!
//! Every function takes a [`fau_domain::time::Moment`]: rule-deciding timestamps and
//! "today" come from the caller, never from the database clock.

mod access;
mod error;
mod handover;
mod invitations;
mod requests;
mod roles;
mod signup;
mod sql;
mod token;

pub use access::effective_access;
pub use error::{ExistingFau, MembershipError};
pub use handover::{create_handover_grants, recovery_grant_admin, RecoveryActor, RecoveryGrant};
pub use invitations::{
    accept_invitation, issue_invitation, resend_invitation, withdraw_invitation, AcceptInvitation,
    Accepted, InvitationChange, IssueInvitation, IssuedInvitation, OfferedRole, RoleChoice,
};
pub use requests::{
    approve_request, create_access_request, create_replacement_proposal, decline_request,
    lapse_requests, CreateAccessRequest, CreateReplacementProposal, RequestDecision,
};
pub use roles::{
    grant_role, revoke_membership, revoke_role_assignment, GrantRole, RevokeAssignment,
    RevokeMembership,
};
pub use signup::{
    activate_tenant, create_pending_tenant, expire_pending_tenants, Activated, Activation,
    PendingSignup, PendingTenant,
};
pub use token::InvitationToken;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd /workspace/backend && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test --workspace`
Expected: PASS, and `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings` are clean.

- [ ] **Step 5: Commit** (the controller commits; executors do not)

```bash
git add backend/crates/persistence/src/membership backend/crates/app/tests/access.rs
git commit -m "Add the effective-access check (#3413, #3417)"
```

---

## Self-Review

Run against the spec after the plan was written. Every code block in this plan was applied task by task to
a copy of the repository and checked with `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`
and the task's tests against the Compose PostgreSQL; after Task 10 the full suite passed (257 tests; the one
failure, `dockerfile_pins_base_images_by_digest`, came from the copy living outside `/workspace` and passes in place).

**1. Spec coverage (§10's test list, then §9).**

| Spec requirement | Where |
| --- | --- |
| Second signup fails, pending or active, names nobody; copy to EWB | Task 5 `a_second_signup_for_a_school_is_refused_and_copied_to_ewb`; DB index in Task 1 |
| Pending expires after 7 days, school free | Task 5 `a_pending_fau_expires_after_seven_days_and_frees_the_school`, `an_expired_pending_fau_does_not_hold_its_school_before_the_sweep` |
| At most three pending per address | Task 5 `one_address_holds_at_most_three_pending_faus` |
| Activation atomic: FAU, role, leader invitation, recovery seat, audit, outbox | Task 5 `activation_writes_everything_together`, `activation_is_all_or_nothing` |
| Leader invitation skipped for the same address; leader's account reused; leader adjusts end date | Task 5, Task 6 `the_leader_accepts_and_may_adjust_the_end_date_within_range`, `an_account_from_another_fau_is_reused` |
| Opening a link accepts nothing | Structural: no persistence function exists for "open"; Task 6 asserts issuing grants nothing until `accept_invitation` |
| Acceptance fails: expired, used, revoked; email mismatch; frozen or not active; issuer without authority | Task 4 unit tests; Task 6 integration tests for each |
| Access request needs passcode verification; no response names an admin | `VerifiedEmail` type in the signature (Task 7); Task 7 decline test checks the requester's message carries no admin id |
| Request limits, 500-character message, 30-day lapse, replacement rules | Task 4 unit tests; Task 7 integration tests |
| A role valid tomorrow grants nothing today; losing the last role removes access on the next check | Task 3 unit tests; Task 10 integration tests |
| Handover grant exactly six months incl. month-end truncation | Task 2 unit tests; Task 9 `the_grant_boundary_truncates_to_the_end_of_february` |
| Revoked admin role produces no grant, and ends one it produced | Task 9 `a_revoked_admin_role_gets_no_grant_and_revocation_ends_an_existing_one` |
| Handover-only admin cannot list members, revoke roles, extend own role | Task 10 (capability `None`, so #3417 refuses the member list); Task 9 `handover_allows_nothing_else` |
| Recovery contact can grant admin only in the no-admin state; notices on create and accept | Task 9 recovery tests |
| Last-admin safeguard: revoking the only other admin, revoking oneself, leaving | Task 8, one test each |
| §9 tables, composite keys, grants, no key material, audit append-only | Task 1 and the standing `schema_review.rs` guards |

Gaps, deliberately: §6.1 warnings and §6.4.1–2's no-admin notification sweep (scheduled work; see Decisions),
school-representative nomination and confirmation (§6.5, table only), request withdrawal by the requester
(status exists, no function).

**2. Placeholder scan.** No "TBD", "TODO", "similar to Task N" or step without its code. Edits to existing
files are exact replace blocks whose old text occurs once (checked mechanically when the plan was applied).

**3. Type consistency.** Names and signatures in each task's **Interfaces** block match the code, which
compiled at every stage. `sql.rs`'s `ActorKind` and `AdminState` change shape in Tasks 6–9; each change is an
exact replace of the previous task's text.

---

## Handover

When all ten tasks are committed:

- Record in `docs/planning-decisions.md` the decisions above that the spec did not already make (the Decisions
  table), and tell #3441 that its migration adds only the `tenants.school_id` foreign key.
- #3417 raises `MINIMUM_CONTRACT_VERSION` to 3 in the change that first calls these functions from `fau serve`,
  and maps `MembershipError` variants to `ErrorCode`s.
- #3410 decides how the sender obtains an invitation link without the outbox ever holding a token.
