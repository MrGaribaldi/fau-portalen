# Groups, the Guest Member Type and One Authorization Function: Implementation Plan (#3501)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Arbitrary groups inside an FAU (open by default, closable by an admin, optionally bound to a unit or cohort so that membership follows roles), a third capability class `guest` that reaches only its own groups, and one authorization rule that every read path and the per-FAU change stream go through.

**Architecture:**
- **Schema.** Migration `0006` lands #3412's remaining school structure (school years, cohorts, organization units, the units' cohorts) and closes the foreign keys `0002` left open on `roles`. Migration `0007` adds `groups`, `group_members`, `roles.group_id` and the `guest` capability class.
- **The rule.** `fau_domain::authz::decide` is a pure function over the viewer's capability, facts about the target and the action. `fau_persistence::membership::authorize` is the one function that reads those facts from the database for a single resource. List reads load the same facts through the same SQL fragment and call `decide`.
- **Group transactions** live beside the membership transactions and follow their rules: tenant lock, then authority, then row state, with audit in the same transaction.
- **The change stream.** A mutation issues a Postgres `NOTIFY` inside its transaction. Each process runs one `Hub`, which holds the process's only `LISTEN` connection and fans each notification out to `Subscription`s. Every delivery goes through `authorize` at the moment of delivery. A revocation closes the affected membership's subscriptions.

**Tech Stack:** Rust 1.98.1, sqlx 0.8 on PostgreSQL 17 (`PgListener` for `LISTEN`), tokio (`sync` feature added to `fau-persistence` for `mpsc`), jiff, serde_json, and the existing `fau-crypto`/`fau-keys` envelope for group names.

**Spec:** `docs/groups-directory-chat-calendar-design.md` (accepted 26 September 2026 on #3500): §3 in full, §8's rows for #3412, #3418 and #3419, and §10's authorization and SSE tests. The plan builds on `docs/tenant-role-history-design.md` (#3412), `docs/key-service-design.md` (record key, AAD) and `docs/fau-creation-and-membership-flow.md` (#3413/#3418). Executors read the spec's §3 before Task 3.

## Global Constraints

- **Every tenant table carries `tenant_id`, and every reference between tenant tables is composite.** For example, `foreign key (tenant_id, group_id) references groups (tenant_id, id)`, never a bare id. `schema_review.rs`'s FK-pairing test enforces this for every new table.
- **No key material in any application table.** Column names must not contain `key`, `dek`, `kek`, `secret`, `private`, `passphrase`, `password`, `cipher`, `nonce` or `wrapped`. `schema_review.rs` fails otherwise; the encrypted column is `encrypted_name`.
- **Group names are content.** The caller encrypts them under `fau_crypto::Unit::Record { tenant }` with `Aad::new(tenant_id, "groups", "encrypted_name", group_id)`. Persistence accepts and returns only `fau_crypto::Ciphertext`. A group name never appears in audit parameters, in a NOTIFY payload, in a log line or in a `Debug` output.
- **Unit, cohort and school-year names are plaintext.** They are school structure, like role names, and fall under ADR-003 decision 6's plaintext list.
- **Ids are UUIDv7** (`Uuid::now_v7()`, ADR-002). `group_id` is chosen by the caller before encryption so that the AAD can bind the name to the row. This follows the pattern of the #3418 messages.
- **Dates cross the SQL boundary as text** (`'2027-10-01'` with `::date`, RFC 3339 with `::timestamptz`), as `membership/sql.rs` documents. Rule-deciding time always comes from the caller's `Moment`, never from SQL `now()`. Test fixtures may use literal timestamps.
- **Membership ordering rule:** tenant state first, then authority, then row state. Every group mutation takes `lock_tenant` first.
- **What a denial reveals.** A viewer who cannot read a resource gets the same answer as for an id that does not exist (`UnknownGroup` for groups). A viewer who can read but may not act gets `NotAuthorized`.
- **Migrations continue from `0005`.** `0006` inserts `schema_contract` version 6 and `0007` inserts version 7; the tests assert `max(version) = migration_file_count()`. `MINIMUM_CONTRACT_VERSION` stays 2 (Ruling R20). Never edit an applied migration.
- **Check-constraint names the tests read** must be exactly as written here. The most important is `roles_capability_class_check`, which is dropped and re-added under the same name.
- **One `LISTEN` connection per process, never one per viewer** (planning decision, 27 September 2026, load testing).
- **English** for all technical text, code, comments and commit messages. #3501 adds **no user-facing strings** (Ruling R19). Never author Nynorsk.
- **Test commands** run from `/workspace/backend` with these three variables exported:
  ```bash
  export TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres
  export TEST_OPENBAO_ADDR=http://openbao:8200
  export TEST_OPENBAO_TOKEN=dev-only-root
  ```
  The app stack is `docker compose -f /workspace/compose.yaml` (project `fau-app`), never `/workspace/docker-compose.yml`. If `db` does not resolve, follow the memory note "agent reaches app DB" and warn Erik first, because attaching reloads the terminal.
- **Before every commit:** `cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings` must be clean.
- **Commits:** on branch `groups-3501`. Every message ends with `(#3501)`, then a blank line, then `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. If git has no identity, pass `GIT_AUTHOR_NAME="Erik W. Bjønnes" GIT_AUTHOR_EMAIL=erik@ewb-solutions.as` and the same `GIT_COMMITTER_*` per command. Never change git config and never push.

## Plan decisions and rulings (for Erik's review)

The spec leaves these open, and the plan decides them. Each carries its reason and the cost if it is wrong.

- **R1: The change stream stops at the `Subscription`, and #3501 mounts no HTTP route.** The SSE HTTP route waits for #3417's sessions. #3501 builds `Hub` and `Subscription` in `fau-persistence`. `Subscription::recv()` returns `None` once the stream is closed, which is what ends an axum `Sse` body.
  - Why no route now: the app has no sessions, so nothing identifies a viewer, and an unauthenticated stream would be an open door. A route that nothing mounts would fail `clippy -D warnings` as dead code, and integration tests cannot import the binary crate.
  - The acceptance test ("revoking a role closes live SSE streams") is therefore proven at the `Subscription` (Task 6).
  - Cost if wrong: #3417 adds a route of about 20 lines (see "What later cards get"). Nothing here changes.
- **R2: Delivery.** One `PgListener` per process fans out to every subscription. Each delivery is authorized for its viewer when it is delivered, with the database read fresh.
  - A subscription more than 64 deliveries behind is closed.
  - A listener reconnect closes every subscription, because notifications sent during the gap are lost.
  - The client reconnects and refetches.
  - Cost if wrong: a bounded buffer and a close are simpler than replay. Replay needs a sequence per FAU.
- **R3: What closes a stream.** These close every live subscription of that membership, even when the person keeps other roles, so their reconnect is authorized afresh:
  - a role assignment revoked;
  - a membership revoked or left;
  - a hand-added group membership removed.

  Other changes do not close streams:
  - a role that simply reaches its end date closes the stream at its next delivery attempt (`NoAccess`), with no timer;
  - closing or archiving a group closes nothing, because per-delivery filtering already covers it.

  Cost if wrong: an expired viewer's idle connection lingers until the next event or a disconnect. It receives nothing.
- **R4: A guest role names exactly one group,** through a nullable `roles.group_id` column and the check `(capability_class = 'guest') = (group_id is not null)`.
  - The spec says "at least one group". Exactly one satisfies that with a plain check constraint instead of a trigger over a join table.
  - A guest in two groups holds two guest roles, or is added to the second group by hand.
  - Cost if wrong: a `role_groups` join table and a one-column data migration.
- **R5: Only a guest role may name a group.** Member and admin roles reach groups through a unit or cohort binding, or by being added by hand.
- **R6: Binding and derived membership.**
  - A group binds to at most one of a unit or a cohort (`groups_bound_to_at_most_one`).
  - A role holder is in the group when their role, valid today, names the group (guest), sits on the group's unit, or sits on the group's cohort.
  - There is no traversal through `unit_cohorts`: a role on class 3A is not in a group bound to cohort K2018. This avoids surprise access.
  - Cost if wrong: widen one shared SQL fragment (`ROLE_FOLLOWS_GROUP`).
- **R7: School structure is plaintext.** Unit, cohort and school-year names are plaintext (ADR-003 decision 6: roles, FAU and school names). Group names are encrypted (spec §3.1).
- **R8: The scope of #3412's remainder.**
  - Included: `school_years` (because `organization_unit` belongs to a school year in #3412), `cohorts`, `organization_units` and `unit_cohorts`.
  - Excluded: `unit_relation`, which nothing in #3500 uses.
  - No transactions create these rows. The initial school configuration flow is its own work (#3412 and #3413 put it with admins), so tests seed with SQL.
  - The runtime role gets `select, insert` only.
  - Cost if wrong: a later migration adds grants.
- **R9: Unit kind codes are English**: `grade`, `class`, `base` and `teaching_group`. #3412's `gruppe` is renamed so that it cannot be confused with FAU groups. `grade_level` is 1–10 (grunnskole). Cost if wrong: widen a check constraint.
- **R10: Uniqueness.** Cohort names are unique per FAU, and unit names are unique per school year.
- **R11: The shape of "one function".** One rule and one reader:
  - the rule is `decide`;
  - the reader is `authorize`, the only database-reading entry point for a single resource;
  - list reads (`list_groups`) read the same facts with the same SQL fragment (`in_group_sql()`) in the same snapshot and call `decide` per row, which avoids N+1 queries;
  - a test asserts that `list_groups` equals `authorize` over every group for every viewer class.
- **R12: Hidden means not found, for every resource.** This generalises the §3.3 closed-group ruling.
  - If a viewer cannot read a resource, every action on it answers `Hidden`.
  - `Forbidden` is used only where the viewer can read.
  - A guest facing FAU-wide content gets `Hidden`.
  - A viewer with no standing gets `NoAccess`, which takes precedence over everything else.
- **R13: Group records and archived groups.**
  - "Write" on the group record means managing it, so it is admin-only.
  - An archived group stays readable. Nobody, admins included, may write content into it. Admins may still manage it, for example to remove members.
  - Archiving is one-way in the MVP.
- **R14: A frozen FAU.** It refuses the additive group actions (create, rename, open, add a member) and allows the reducing ones (close, archive, remove a member). This is the same line #3418 drew. The frozen check stays out of `authorize`: it is tenant state, checked before authority.
- **R15: Revoking a membership soft-removes its hand-added group memberships,** each audited. Otherwise `ensure_membership`, which reopens the same row on a re-invite, would hand a removed guest their closed groups back.
- **R16: Guests are not FAU-wide recipients.** Recovery notices go to current members and admins, and in the 24-month fallback to recent holders of member or admin roles. Guests are excluded, because an FAU-wide audience never includes guests (§3.2).
- **R17: A handover invitation cannot offer a guest role.** Inviting guests is admin-only (§3.3), and handover is not admin authority.
- **R18: Group-name bounds.** `GroupName` is at most 100 characters. Its ciphertext is 42–512 bytes with version byte 1, checked both in persistence (`GroupNameMalformed`) and in the database (`groups_name_is_an_envelope`).
- **R19: No user-facing strings and no new `ErrorCode`.** #3501 has no HTTP surface. #3417 maps `UnknownGroup` to `not_found`, and adds a forbidden code with its Bokmål source string when it maps `NotAuthorized`.
- **R20: The schema contract minimum stays 2.** This follows 0003–0005: the migrations are additive, and only new code creates guest rows or groups.
- **R21: Audit order.** The #3418 read-path audit below was done while writing this plan, before `Guest` exists in code. Its fixes land in Task 5, before any transaction can create a guest role, which first happens in Task 9. Task 2 maps `Guest` to its own `Capability::Guest`, below `Member`, so no existing capability comparison can lift a guest to member rights. Before Task 9, only test fixtures create guest roles, in SQL.
- **R22: A group's member list is readable exactly when the group is.** This is consistent with directory §4.4.
- **R23: Removal records only `removed_at`.** Who removed the member is in `audit_events`, and the spec lists no `removed_by` column.

**Spec against code:**
- Spec §1 says "migrations stop at `0004`". The code has `0005_wrapped_keys.sql` from #3506, which was decided later (27 September), so the plan continues from `0005`.
- #3412 says every ordinary member reads everything. #3500, accepted later on 26 September, restricts that with closed groups and guests, so #3500 wins.
- Spec §8 gives #3418 "display name captured when an invitation is accepted". That field is directory data (§4.1) and belongs to #3502, not #3501.

## Read-path audit of #3418 (spec §3.3, last bullet)

Done while writing this plan, on branch `groups-3501` at `f01de45`, before `Guest` existed:

| Path | Shortcut found | Treatment |
|---|---|---|
| `effective_access` (`membership/access.rs`) | Returns `Capability::Member` for any valid role, and #3412 lets `Member` read everything | Task 2 adds `Capability::Guest` below `Member`. Task 4 splits out `membership_access`, which `authorize` and the hub share, and documents that a capability is not permission to read a resource |
| `access_request_message` (`membership/requests.rs`) | `require_admin` directly | Task 5 routes it through `authorize(Resource::Fau, Action::Manage)` in a read snapshot. Behaviour is unchanged for admins and members |
| `invitation_message` (`membership/invitations.rs`) | None: authorized by the token plus a matching verified address; the reader is not a member | Unchanged, and documented as outside `authorize` |
| `current_member_emails` / `recovery_notice_recipients` (`membership/sql.rs`) | "Any role valid today" counts as a member, so guests would receive FAU-wide recovery notices | Task 5 excludes guest-class roles (R16) |
| `admin_emails`, `is_admin_today`, `AdminState`, the handover sweep | Filter on `capability_class = 'admin'` explicitly | Unaffected |
| `resolve_roles` / `issue_invitation` (handover mode) | A handover could offer an existing guest role | Task 9 refuses it (R17) |

No path returns documents or audit reads yet (#3419, #3421), so none needs rerouting.

## What later cards get from #3501, and nothing more

- **#3419 (folders and documents).** `Resource::audience(folder.group_id)` gives a folder's audience, and the composite FK target is `groups (tenant_id, id)`. A document resource resolves its folder's audience and calls `authorize`, and adds rows to the matrix in `tests/authorization.rs`.
- **#3502 (directory).** `list_group_members` returns manual and role-derived members; guests are the holders of `CapabilityClass::Guest` roles. Display names and contact emails are #3502's.
- **#3503 (chat).**
  - `Resource::audience(thread.group_id)`.
  - `Change::Changed(resource)` through the pub(crate) `notify` inside the posting transaction.
  - A `Resource::Thread` variant resolving its audience inside `authorize`, with matrix rows.
- **#3504 (calendar).** `authorize` never touches the key service, so the feed handler can call it at fetch time.
- **#3505 (poll).** `Denied::Forbidden` for a reader without write access, which becomes 403 at the edge.
- **#3417 (HTTP).** Starts one `Hub` in `run_server` and mounts the stream once a session names the viewer:
  ```rust
  // Sketch for #3417, not built here:
  let sub = hub.subscribe(viewer).await?;
  let stream = futures_util::stream::unfold(sub, |mut s| async move {
      s.recv().await.map(|r| (Ok::<_, std::convert::Infallible>(Event::default().event(r.kind_code()).data(r.id_text())), s))
  });
  Sse::new(stream).keep_alive(KeepAlive::default())
  ```
  #3417 adds the `kind_code`/`id_text` helpers with the route.

## File Structure

```
backend/
  migrations/0006_organization_structure.sql      NEW  school_years, cohorts, organization_units, unit_cohorts; roles FKs
  migrations/0007_groups.sql                      NEW  groups, group_members, roles.group_id, the guest class
  crates/domain/src/
    lib.rs                                        modify: pub mod authz
    authz.rs                                      NEW  Action, GroupFacts, Target, Denied, Decision, decide
    membership/vocabulary.rs                      modify: CapabilityClass::Guest, Visibility, GroupName
    membership/access.rs                          modify: Capability::Guest
  crates/persistence/
    Cargo.toml                                    modify: tokio "sync"
    src/membership/mod.rs                         modify: modules and re-exports
    src/membership/access.rs                      modify: membership_access, shared by authorize and the hub
    src/membership/authz.rs                       NEW  Viewer, Resource, authorize, read_transaction, in_group_sql
    src/membership/events.rs                      NEW  Change, notify, Hub, Subscription
    src/membership/groups.rs                      NEW  group transactions and reads
    src/membership/error.rs                       modify: group and role-group variants
    src/membership/sql.rs                         modify: guests out of FAU-wide recipient lists
    src/membership/requests.rs                    modify: access_request_message through authorize
    src/membership/roles.rs                       modify: notify on revocation; leave groups on membership revoke
    src/membership/invitations.rs                 modify: guest roles naming a group; not under handover
  crates/app/tests/
    common/mod.rs                                 modify: pub mod groups
    common/groups.rs                              NEW  SQL fixtures and the shared eight-viewer World
    common/membership.rs                          modify: new_role passes group_id: None
    organization_schema.rs                        NEW  0006
    groups_schema.rs                              NEW  0007
    membership_schema.rs                          modify: groups_visibility_check joins the code-set test
    authorization.rs                              NEW  the matrix (acceptance) and derived membership
    read_paths.rs                                 NEW  the #3418 audit fixes
    events.rs                                     NEW  the change stream (acceptance: revocation closes)
    groups.rs                                     NEW  group transactions
    group_reads.rs                                NEW  list/get/members agree with authorize
    guests.rs                                     NEW  guest invitations
    key_chain.rs                                  modify: group names under the record key, end to end
docs/planning-decisions.md                        modify: record the rulings (Task 10)
```

---

### Task 1: Migration 0006: #3412's school structure, and the open foreign keys on `roles`

**Files:**
- Create: `backend/migrations/0006_organization_structure.sql`
- Test: `backend/crates/app/tests/organization_schema.rs`

**Interfaces:**
- Consumes: migrations `0001`–`0005`; `common::membership::school(pool, label) -> Uuid` (existing test helper).
- Produces for later tasks, as tables:
  - `school_years (tenant_id, id, name, starts_on, ends_on_exclusive)`;
  - `cohorts (tenant_id, id, name)`;
  - `organization_units (tenant_id, id, school_year_id, kind, name)`, where `kind` is one of `grade`, `class`, `base`, `teaching_group`;
  - `unit_cohorts (tenant_id, unit_id, cohort_id, grade_level)`;
  - foreign keys `roles_unit_fk` and `roles_cohort_fk`;
  - `fau_app` has `select, insert` on all four tables;
  - `schema_contract` version 6.

- [ ] **Step 1: Write the failing test**

Create `backend/crates/app/tests/organization_schema.rs`:

```rust
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
    sqlx::query("insert into tenants (id, name, status, school_id) values ($1, 'FAU', 'active', $2)")
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

async fn unit(pool: &PgPool, tenant: Uuid, year: Uuid, kind: &str, name: &str) -> Result<Uuid, sqlx::Error> {
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

async fn link(pool: &PgPool, tenant: Uuid, unit: Uuid, cohort: Uuid, grade: i16) -> Result<(), sqlx::Error> {
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

async fn role(pool: &PgPool, tenant: Uuid, unit: Option<Uuid>, cohort: Option<Uuid>) -> Result<Uuid, sqlx::Error> {
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
    def.split('\'').skip(1).step_by(2).map(str::to_owned).collect()
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
    let err = unit(&pool, t, year, "gruppe", "Gruppe 1").await.unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("23514"));
    assert_eq!(constraint_name(&err).as_deref(), Some("organization_units_kind_check"));
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
        let u = unit(&pool, t, year, "class", &format!("X{bad}")).await.unwrap();
        let err = link(&pool, t, u, k, bad).await.unwrap_err();
        assert_eq!(sqlstate(&err).as_deref(), Some("23514"), "grade {bad}");
        assert_eq!(constraint_name(&err).as_deref(), Some("unit_cohorts_grade_level_check"));
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
    assert_eq!(constraint_name(&err).as_deref(), Some("cohorts_name_unique"));

    let this_year = school_year(&pool, t, "2026/27").await;
    let next_year = school_year(&pool, t, "2027/28").await;
    unit(&pool, t, this_year, "class", "3A").await.unwrap();
    // Historical rows are kept, so next year's 3A is a new unit, not a rename.
    unit(&pool, t, next_year, "class", "3A").await.unwrap();
    let err = unit(&pool, t, this_year, "class", "3A").await.unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("23505"));
    assert_eq!(constraint_name(&err).as_deref(), Some("organization_units_name_unique"));
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
    assert_eq!(constraint_name(&err).as_deref(), Some("school_year_period_is_non_empty"));
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
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p fau-app --test organization_schema`
Expected: FAIL. Every test panics on a missing relation (`relation "school_years" does not exist`, `"cohorts"` or `"organization_units"`).

- [ ] **Step 3: Write the migration**

Create `backend/migrations/0006_organization_structure.sql`:

```sql
-- 0006: the rest of #3412's school structure, which the groups design (#3500, §3.1 and §8)
-- needs before groups can bind to it: school years, cohorts, organization units and the
-- cohorts a unit spans. It also closes the foreign keys 0002 left open on roles.unit_id and
-- roles.cohort_id.
--
-- This is school structure, not a pupil register: no table here names a child. Names are
-- plaintext, like role names (ADR-003 decision 6's plaintext list).
--
-- Out of scope, deliberately:
--   * #3412's unit_relation. Nothing in #3500 uses it.
--   * Any transaction that writes these tables. The initial school configuration flow is
--     its own work, so the runtime role gets select and insert only.
--
-- Unit kinds are English codes. #3412's "gruppe" becomes teaching_group so that it cannot
-- be confused with the FAU groups of migration 0007.

create table school_years (
  tenant_id         uuid        not null references tenants (id),
  id                uuid        not null,
  name              text        not null check (length(name) between 1 and 100),
  -- Local calendar dates, half-open, like every period in this schema.
  starts_on         date        not null,
  ends_on_exclusive date        not null,
  created_at        timestamptz not null default now(),
  primary key (tenant_id, id),
  constraint school_year_period_is_non_empty check (starts_on < ends_on_exclusive)
);

-- A cohort is a stable identity for a year group across school years (#3412), e.g. K2019.
create table cohorts (
  tenant_id  uuid        not null references tenants (id),
  id         uuid        not null,
  name       text        not null check (length(name) between 1 and 100),
  created_at timestamptz not null default now(),
  primary key (tenant_id, id),
  constraint cohorts_name_unique unique (tenant_id, name)
);

-- A unit belongs to one school year, and historical rows are kept: next year's 3A is a new
-- row, so the past keeps showing the structure that applied at the time.
create table organization_units (
  tenant_id      uuid        not null references tenants (id),
  id             uuid        not null,
  school_year_id uuid        not null,
  kind           text        not null check (kind in ('grade', 'class', 'base', 'teaching_group')),
  name           text        not null check (length(name) between 1 and 100),
  created_at     timestamptz not null default now(),
  primary key (tenant_id, id),
  foreign key (tenant_id, school_year_id) references school_years (tenant_id, id),
  constraint organization_units_name_unique unique (tenant_id, school_year_id, name)
);

-- Many-to-many, so a base can span two cohorts at different grades (#3412's example).
create table unit_cohorts (
  tenant_id   uuid     not null references tenants (id),
  unit_id     uuid     not null,
  cohort_id   uuid     not null,
  -- Grunnskole: grades 1 to 10.
  grade_level smallint not null check (grade_level between 1 and 10),
  primary key (tenant_id, unit_id, cohort_id),
  foreign key (tenant_id, unit_id)   references organization_units (tenant_id, id),
  foreign key (tenant_id, cohort_id) references cohorts (tenant_id, id)
);
create index unit_cohorts_cohort_idx on unit_cohorts (tenant_id, cohort_id);

-- The foreign keys 0002 promised. The composite form, never a bare references on the id.
alter table roles
  add constraint roles_unit_fk   foreign key (tenant_id, unit_id)   references organization_units (tenant_id, id),
  add constraint roles_cohort_fk foreign key (tenant_id, cohort_id) references cohorts (tenant_id, id);
create index roles_unit_idx   on roles (tenant_id, unit_id)   where unit_id is not null;
create index roles_cohort_idx on roles (tenant_id, cohort_id) where cohort_id is not null;

grant select, insert on school_years, cohorts, organization_units, unit_cohorts to fau_app;

insert into schema_contract (version) values (6);
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p fau-app --test organization_schema --test schema_review --test migrations --test membership_schema`
Expected: all pass. `organization_schema` has 7 passing tests. `schema_review`'s FK-pairing and key-material guards cover the new tables, and `migrations` counts six files.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add backend/migrations/0006_organization_structure.sql backend/crates/app/tests/organization_schema.rs
git commit -m "Migrate #3412's school structure and close the roles foreign keys (#3501)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Migration 0007: groups, group members and the guest class; the vocabulary that names them

**Files:**
- Create: `backend/migrations/0007_groups.sql`
- Modify: `backend/crates/domain/src/membership/vocabulary.rs` (`CapabilityClass`, its `codes!` line, new `Visibility`, new `GroupName`, tests)
- Modify: `backend/crates/domain/src/membership/access.rs:9-15` (`Capability`), `:84-89` (the class match), tests
- Modify: `backend/crates/app/tests/membership_schema.rs` (`check_constraints_allow_exactly_the_domain_code_sets`)
- Test: `backend/crates/app/tests/groups_schema.rs`

**Interfaces:**
- Consumes: Task 1's `organization_units` and `cohorts`.
- Produces:
  - `CapabilityClass::Guest` (code `"guest"`), and `Capability::{None, Guest, Member, Admin}` in that order;
  - `evaluate_access` maps `Guest` to `Capability::Guest`;
  - `Visibility::{Open, Closed}` with `code()`, `from_code()` and `ALL` (codes `"open"`, `"closed"`);
  - `GroupName::parse(&str) -> Result<GroupName, NameError>`, `GroupName::MAX_CHARS = 100` and `as_str()`, with a redacted `Debug`.
- Tables:
  - `groups (tenant_id, id, encrypted_name bytea, visibility, unit_id, cohort_id, created_by, created_at, archived_at)`;
  - `group_members (tenant_id, id, group_id, membership_id, added_by, added_at, removed_at)`;
  - `roles.group_id`.
- Constraints:
  - `roles_capability_class_check`, which allows `'member', 'admin', 'guest'`;
  - `roles_guest_names_a_group`, `roles_group_fk` and `groups_visibility_check`;
  - `groups_bound_to_at_most_one` and `groups_name_is_an_envelope`;
  - `group_members_one_current` (a partial unique index) and `group_member_removed_after_added`.
- Contract version 7.

- [ ] **Step 1: Write the failing tests**

In `backend/crates/domain/src/membership/access.rs`, add to `mod tests`:

```rust
    #[test]
    fn a_guest_role_gives_guest_capability_and_a_member_role_outranks_it() {
        let today = date(2026, 9, 23);
        let guest = role(CapabilityClass::Guest, date(2026, 9, 1), date(2027, 9, 1));
        let member = role(CapabilityClass::Member, date(2026, 9, 1), date(2027, 9, 1));
        assert_eq!(
            evaluate_access(OK, &[guest], &[], today).capability,
            Capability::Guest
        );
        assert_eq!(
            evaluate_access(OK, &[guest, member], &[], today).capability,
            Capability::Member,
            "rights are the union of valid roles"
        );
        assert!(Capability::None < Capability::Guest && Capability::Guest < Capability::Member);
        assert!(
            !tenant_has_admin(&[guest], &[], today),
            "a guest role is never admin coverage"
        );
    }
```

In `backend/crates/domain/src/membership/vocabulary.rs`'s `mod tests`:
- add `round_trips!(visibility_round_trips, Visibility);` after `round_trips!(recovery_holder_round_trips, RecoveryHolder);`;
- in `codes_match_the_database_check_constraints`, replace the `CapabilityClass` expectation `["member", "admin"]` with `["member", "admin", "guest"]` and append:

```rust
        assert_eq!(
            Visibility::ALL.iter().map(|v| v.code()).collect::<Vec<_>>(),
            ["open", "closed"]
        );
```

- in `names_are_redacted_in_debug`, append:

```rust
        let group = GroupName::parse("Oppfølging av sak med rektor").unwrap();
        assert_eq!(format!("{group:?}"), "GroupName([redacted])");
        assert!(!format!("{:?}", Some(&group)).contains("rektor"));
```

- in `names_are_trimmed_and_bounded`, append:

```rust
        assert_eq!(GroupName::parse(" Dugnad ").unwrap().as_str(), "Dugnad");
        assert!(GroupName::parse(&"ø".repeat(100)).is_ok());
        assert_eq!(GroupName::parse(&"ø".repeat(101)), Err(NameError::TooLong));
```

In `backend/crates/app/tests/membership_schema.rs`, in `check_constraints_allow_exactly_the_domain_code_sets`:
- change the `use` line inside the function to also import `Visibility`;
- add this case to `cases` after the `roles` entry:

```rust
        (
            "groups",
            "groups_visibility_check",
            codes(Visibility::ALL, Visibility::code),
        ),
```

Create `backend/crates/app/tests/groups_schema.rs`:

```rust
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
        ("insert into tenants (id, name, status, school_id) values ($1, 'FAU', 'active', $2)", vec![tenant, school]),
        ("insert into accounts (id, email) values ($1, $1::text || '@example.test')", vec![account]),
        ("insert into memberships (tenant_id, id, account_id) values ($1, $2, $3)", vec![tenant, membership, account]),
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
        ("insert into cohorts (tenant_id, id, name) values ($1, $2, 'K2019')", vec![tenant, cohort]),
    ] {
        let mut q = sqlx::query(sql);
        for b in binds {
            q = q.bind(b);
        }
        q.execute(pool).await.unwrap();
    }
    Seed { tenant, membership, unit, cohort }
}

async fn group(pool: &PgPool, s: &Seed, name: Vec<u8>, unit: Option<Uuid>, cohort: Option<Uuid>) -> Result<Uuid, sqlx::Error> {
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

async fn role(pool: &PgPool, s: &Seed, class: &str, group: Option<Uuid>) -> Result<Uuid, sqlx::Error> {
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

    role(&pool, &s, "guest", Some(g)).await.expect("a guest role naming a group");
    role(&pool, &s, "member", None).await.expect("a member role naming none");
    for (class, group) in [("guest", None), ("member", Some(g)), ("admin", Some(g))] {
        let err = role(&pool, &s, class, group).await.unwrap_err();
        assert_eq!(sqlstate(&err).as_deref(), Some("23514"), "{class}");
        assert_eq!(constraint_name(&err).as_deref(), Some("roles_guest_names_a_group"));
    }
    let err = role(&pool, &s, "owner", None).await.unwrap_err();
    assert_eq!(constraint_name(&err).as_deref(), Some("roles_capability_class_check"));
}

#[tokio::test]
async fn a_group_name_must_be_shaped_like_an_envelope() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let s = seed(&pool).await;
    group(&pool, &s, envelope(42), None, None).await.expect("the shortest envelope");
    group(&pool, &s, envelope(512), None, None).await.expect("the longest envelope");
    for bad in [b"Juleballkomiteen".to_vec(), envelope(41), envelope(513), vec![2u8; 60]] {
        let err = group(&pool, &s, bad, None, None).await.unwrap_err();
        assert_eq!(sqlstate(&err).as_deref(), Some("23514"));
        assert_eq!(constraint_name(&err).as_deref(), Some("groups_name_is_an_envelope"));
    }
}

#[tokio::test]
async fn a_group_binds_to_at_most_one_unit_or_cohort_of_its_own_fau() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let s = seed(&pool).await;
    group(&pool, &s, envelope(42), Some(s.unit), None).await.unwrap();
    group(&pool, &s, envelope(42), None, Some(s.cohort)).await.unwrap();
    let err = group(&pool, &s, envelope(42), Some(s.unit), Some(s.cohort)).await.unwrap_err();
    assert_eq!(constraint_name(&err).as_deref(), Some("groups_bound_to_at_most_one"));

    let other = seed(&pool).await;
    let err = group(&pool, &s, envelope(42), Some(other.unit), None).await.unwrap_err();
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
    assert_eq!(constraint_name(&err).as_deref(), Some("group_members_one_current"));

    // Removal is soft, and history keeps both rows.
    sqlx::query("update group_members set removed_at = '2026-09-24T10:00:00Z' where id = $1")
        .bind(first)
        .execute(&pool)
        .await
        .unwrap();
    add(&pool, &s, g, "2026-09-25T10:00:00Z").await.expect("re-added after removal");
}

#[tokio::test]
async fn removal_cannot_precede_addition() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let s = seed(&pool).await;
    let g = group(&pool, &s, envelope(42), None, None).await.unwrap();
    let row = add(&pool, &s, g, "2026-09-23T10:00:00Z").await.unwrap();
    let err = sqlx::query("update group_members set removed_at = '2026-09-22T10:00:00Z' where id = $1")
        .bind(row)
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(constraint_name(&err).as_deref(), Some("group_member_removed_after_added"));
}

#[tokio::test]
async fn the_runtime_role_cannot_delete_groups_or_their_history() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let s = seed(&admin).await;
    let app = db.app_pool().await;
    let g = group(&app, &s, envelope(42), None, None).await.expect("fau_app may insert");
    add(&app, &s, g, "2026-09-23T10:00:00Z").await.expect("fau_app may insert members");
    for sql in [
        "delete from group_members where group_id = $1",
        "delete from groups where id = $1",
    ] {
        let err = sqlx::query(sql).bind(g).execute(&app).await.unwrap_err();
        assert_eq!(sqlstate(&err).as_deref(), Some("42501"), "{sql}");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p fau-domain membership:: ; cargo test -p fau-app --test groups_schema --test membership_schema`
Expected: `fau-domain` does not compile (`no variant named Guest`, `cannot find type Visibility`, `GroupName`). `groups_schema` fails on `relation "groups" does not exist`.

- [ ] **Step 3: Extend the vocabulary**

In `backend/crates/domain/src/membership/vocabulary.rs`, replace the `CapabilityClass` enum with:

```rust
/// The privilege a role grants. The class decides, never the role's name (§2.3). A guest
/// reaches only the groups its roles name or it was added to (groups design §3.1, D10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityClass {
    Member,
    Admin,
    Guest,
}
```

Add after the `RecoveryHolder` enum:

```rust
/// Who besides its members and the admins may see a group (groups design §3.3, D11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Open,
    Closed,
}
```

Replace `codes!(CapabilityClass { Member => "member", Admin => "admin" });` with:

```rust
codes!(CapabilityClass { Member => "member", Admin => "admin", Guest => "guest" });
codes!(Visibility { Open => "open", Closed => "closed" });
```

Add after `FauName`'s `impl` block:

```rust
/// A group's name ("Dugnadskomiteen", "Oppfølging av sak med rektor"). Content, so it is
/// encrypted before it reaches persistence (groups design §3.1), and `Debug` is redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct GroupName(String);

impl GroupName {
    pub const MAX_CHARS: usize = 100;

    pub fn parse(raw: &str) -> Result<Self, NameError> {
        validate_name(raw, Self::MAX_CHARS).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for GroupName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("GroupName([redacted])")
    }
}
```

In `backend/crates/domain/src/membership/access.rs`, replace the `Capability` enum and its doc with:

```rust
/// What a person may do in one FAU today. Ordered: each class includes every right of the
/// ones before it within the resources it can reach. `Guest` reaches only its own groups
/// (groups design §3.3), so a capability is never, on its own, permission to read a
/// resource: ask `fau_persistence::membership::authorize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capability {
    None,
    Guest,
    Member,
    Admin,
}
```

And in `evaluate_access`, replace the class match with:

```rust
        .map(|a| match a.capability {
            CapabilityClass::Guest => Capability::Guest,
            CapabilityClass::Member => Capability::Member,
            CapabilityClass::Admin => Capability::Admin,
        })
```

- [ ] **Step 4: Write the migration**

Create `backend/migrations/0007_groups.sql`:

```sql
-- 0007: groups, their members and the guest class (#3501; groups design §3.1).
--
-- Groups are arbitrary: a committee, a year, a working group. They are open to members by
-- default, and an admin can close one. A group may bind to one unit or one cohort (0006).
-- Its membership is then derived from the roles held on that unit or cohort on the current
-- Europe/Oslo date. That derivation is a query (fau_persistence's in_group_sql), never copied
-- rows, so turnover needs no job.
--
-- The name is content ("Oppfølging av sak med rektor"). It is encrypted by the backend
-- under the FAU's record key with AAD (tenant, 'groups', 'encrypted_name', id). This table
-- holds the envelope only, and no key material. The check below is structural: version
-- byte 1, and 41 bytes of nonce and tag around a 1..400-byte name.
create table groups (
  tenant_id      uuid        not null references tenants (id),
  id             uuid        not null,
  encrypted_name bytea       not null,
  visibility     text        not null default 'open' check (visibility in ('open', 'closed')),
  unit_id        uuid,
  cohort_id      uuid,
  created_by     uuid        not null,
  created_at     timestamptz not null,
  -- Archiving is one-way in the MVP: the group stays readable as history, takes no writes.
  archived_at    timestamptz,
  primary key (tenant_id, id),
  foreign key (tenant_id, unit_id)    references organization_units (tenant_id, id),
  foreign key (tenant_id, cohort_id)  references cohorts (tenant_id, id),
  foreign key (tenant_id, created_by) references memberships (tenant_id, id),
  constraint groups_bound_to_at_most_one check (num_nonnulls(unit_id, cohort_id) <= 1),
  constraint groups_name_is_an_envelope
    check (get_byte(encrypted_name, 0) = 1 and octet_length(encrypted_name) between 42 and 512)
);
create index groups_unit_idx   on groups (tenant_id, unit_id)   where unit_id is not null;
create index groups_cohort_idx on groups (tenant_id, cohort_id) where cohort_id is not null;

-- Hand-added members. Removal is soft, because group history is FAU history. Who removed
-- whom is in audit_events.
create table group_members (
  tenant_id     uuid        not null references tenants (id),
  id            uuid        not null,
  group_id      uuid        not null,
  membership_id uuid        not null,
  added_by      uuid        not null,
  added_at      timestamptz not null,
  removed_at    timestamptz,
  primary key (tenant_id, id),
  foreign key (tenant_id, group_id)      references groups (tenant_id, id),
  foreign key (tenant_id, membership_id) references memberships (tenant_id, id),
  foreign key (tenant_id, added_by)      references memberships (tenant_id, id),
  constraint group_member_removed_after_added check (removed_at is null or removed_at >= added_at)
);
create unique index group_members_one_current
  on group_members (tenant_id, group_id, membership_id) where removed_at is null;
create index group_members_membership_idx
  on group_members (tenant_id, membership_id) where removed_at is null;

-- The third capability class. A guest role names exactly one group, and only a guest role
-- names one. A guest in two groups holds two guest roles, or is added to the second by hand.
alter table roles add column group_id uuid;
alter table roles add constraint roles_group_fk
  foreign key (tenant_id, group_id) references groups (tenant_id, id);
-- Same name as 0002's generated one, which the code-set agreement test reads.
alter table roles drop constraint roles_capability_class_check;
alter table roles add constraint roles_capability_class_check
  check (capability_class in ('member', 'admin', 'guest'));
alter table roles add constraint roles_guest_names_a_group
  check ((capability_class = 'guest') = (group_id is not null));
create index roles_group_idx on roles (tenant_id, group_id) where group_id is not null;

-- No delete: groups are archived and members removed softly.
grant select, insert, update on groups, group_members to fau_app;

insert into schema_contract (version) values (7);
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p fau-domain && cargo test -p fau-app --test groups_schema --test membership_schema --test schema_review --test migrations --test organization_schema`
Expected: all pass. `groups_schema` has 6 passing tests. The domain crate's existing tests plus the new access test pass.

Then run `cargo build --workspace --all-targets`. It must compile: no other code matches `CapabilityClass` exhaustively (checked: only `evaluate_access` does).

- [ ] **Step 6: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add backend/migrations/0007_groups.sql backend/crates/domain/src/membership/vocabulary.rs \
  backend/crates/domain/src/membership/access.rs backend/crates/app/tests/groups_schema.rs \
  backend/crates/app/tests/membership_schema.rs
git commit -m "Add groups, group members and the guest capability class (#3501)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: The authorization rule: `fau_domain::authz::decide`

**Files:**
- Create: `backend/crates/domain/src/authz.rs`
- Modify: `backend/crates/domain/src/lib.rs` (add `pub mod authz;` after `pub mod email;`)

**Interfaces:**
- Consumes: `Capability` (Task 2) and `Visibility` (Task 2).
- Produces (all `Debug, Clone, Copy, PartialEq, Eq`):
  - `pub enum Action { Read, Write, Manage }`;
  - `pub struct GroupFacts { pub visibility: Visibility, pub archived: bool, pub viewer_in_group: bool }`;
  - `pub enum Target { Fau, Group(GroupFacts), GroupContent(GroupFacts) }`;
  - `pub enum Denied { NoAccess, Hidden, Forbidden }`;
  - `pub type Decision = Result<(), Denied>`;
  - `pub fn decide(capability: Capability, target: Target, action: Action) -> Decision`.

The rule is spec §3.3's table plus rulings R12 and R13:

| Viewer | FAU-wide | Open group | Closed group |
|---|---|---|---|
| Admin | read + write | read + write | read + write |
| Member | read + write | read; write if in the group | read + write only if in the group, hidden otherwise |
| Guest | hidden | read + write only if in the group, hidden otherwise | read + write only if in the group, hidden otherwise |
| No standing | `NoAccess` | `NoAccess` | `NoAccess` |

- **Manage** is admin-only everywhere. On `Target::Group`, **Write** means Manage.
- An **archived** group refuses content writes from everyone, admins included.

- [ ] **Step 1: Write the failing test**

Create `backend/crates/domain/src/authz.rs` with only the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::membership::access::Capability::{self, Admin, Guest, Member};
    use crate::membership::vocabulary::Visibility::{Closed, Open};

    const A: Decision = Ok(());
    const F: Decision = Err(Denied::Forbidden);
    const H: Decision = Err(Denied::Hidden);

    fn facts(visibility: Visibility, viewer_in_group: bool) -> GroupFacts {
        GroupFacts {
            visibility,
            archived: false,
            viewer_in_group,
        }
    }

    fn all(c: Capability, t: Target) -> [Decision; 3] {
        [
            decide(c, t, Action::Read),
            decide(c, t, Action::Write),
            decide(c, t, Action::Manage),
        ]
    }

    #[test]
    fn spec_table_for_content() {
        // (capability, visibility or FAU-wide, in the group, [read, write, manage])
        let rows: &[(Capability, Option<Visibility>, bool, [Decision; 3])] = &[
            (Admin, None, false, [A, A, A]),
            (Member, None, false, [A, A, F]),
            (Guest, None, false, [H, H, H]),
            (Admin, Some(Open), true, [A, A, A]),
            (Admin, Some(Open), false, [A, A, A]),
            (Admin, Some(Closed), true, [A, A, A]),
            (Admin, Some(Closed), false, [A, A, A]),
            (Member, Some(Open), true, [A, A, F]),
            (Member, Some(Open), false, [A, F, F]),
            (Member, Some(Closed), true, [A, A, F]),
            (Member, Some(Closed), false, [H, H, H]),
            (Guest, Some(Open), true, [A, A, F]),
            (Guest, Some(Open), false, [H, H, H]),
            (Guest, Some(Closed), true, [A, A, F]),
            (Guest, Some(Closed), false, [H, H, H]),
        ];
        for &(c, v, inside, want) in rows {
            let t = match v {
                None => Target::Fau,
                Some(v) => Target::GroupContent(facts(v, inside)),
            };
            assert_eq!(all(c, t), want, "{c:?} {v:?} in={inside}");
        }
    }

    #[test]
    fn writing_to_a_group_record_is_managing_it() {
        for v in [Open, Closed] {
            assert_eq!(all(Admin, Target::Group(facts(v, false))), [A, A, A]);
            assert_eq!(all(Member, Target::Group(facts(v, true))), [A, F, F]);
            assert_eq!(all(Guest, Target::Group(facts(v, true))), [A, F, F]);
        }
        assert_eq!(all(Member, Target::Group(facts(Open, false))), [A, F, F]);
        assert_eq!(all(Member, Target::Group(facts(Closed, false))), [H, H, H]);
        assert_eq!(all(Guest, Target::Group(facts(Open, false))), [H, H, H]);
    }

    #[test]
    fn no_standing_outranks_everything() {
        let targets = [
            Target::Fau,
            Target::Group(facts(Open, true)),
            Target::GroupContent(facts(Closed, true)),
        ];
        for t in targets {
            for a in [Action::Read, Action::Write, Action::Manage] {
                assert_eq!(decide(Capability::None, t, a), Err(Denied::NoAccess));
            }
        }
    }

    #[test]
    fn an_archived_group_is_read_only_for_everyone_but_still_managed() {
        let archived = GroupFacts {
            visibility: Open,
            archived: true,
            viewer_in_group: true,
        };
        assert_eq!(all(Admin, Target::GroupContent(archived)), [A, F, A]);
        assert_eq!(all(Member, Target::GroupContent(archived)), [A, F, F]);
        assert_eq!(all(Guest, Target::GroupContent(archived)), [A, F, F]);
        assert_eq!(all(Admin, Target::Group(archived)), [A, A, A]);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p fau-domain authz`
Expected: does not compile, because `authz` is not declared in `lib.rs`. After adding `pub mod authz;`, it still fails: `cannot find type Decision`, `decide`.

- [ ] **Step 3: Write the implementation**

Put this above the test module in `backend/crates/domain/src/authz.rs`, and add `pub mod authz;` to `lib.rs`:

```rust
//! The authorization rule (groups design §3.3). One pure function decides what a viewer may
//! do with a resource, from facts read from the database in the same transaction as the
//! operation. `fau_persistence::membership::authorize` reads those facts for one resource.
//! List reads load the same facts per row with the same SQL and call [`decide`] too, so
//! the rule exists exactly once.
//!
//! Security note: this defends against mistakes and casual misuse by people with
//! legitimate access (CLAUDE.md's threat-model boundary). Encryption keys stay per FAU and
//! per document (spec §3.4), so a guest's restriction is authorization, not cryptography.

use crate::membership::access::Capability;
use crate::membership::vocabulary::Visibility;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Seeing that the resource exists, and reading it.
    Read,
    /// Posting, voting and editing content (§3.3).
    Write,
    /// Creating, closing and archiving groups, adding members and inviting guests. Admin
    /// only in the MVP (§3.3; a group lead role is deferred).
    Manage,
}

/// What the database says about a group, for one viewer, today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupFacts {
    pub visibility: Visibility,
    pub archived: bool,
    /// A current member: added by hand and not removed, or holding a role valid today that
    /// the group follows (its guest role, its unit or its cohort; §3.1).
    pub viewer_in_group: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Content addressed to the whole FAU, `group_id` null (§3.2): members and admins,
    /// never guests.
    Fau,
    /// The group itself: that it exists, its name, its member list.
    Group(GroupFacts),
    /// Content whose audience is the group (§3.2).
    GroupContent(GroupFacts),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Denied {
    /// No valid standing in the FAU at all. A live stream for this viewer closes.
    NoAccess,
    /// The viewer may not read the resource, so for them it does not exist: answered
    /// exactly as an unknown id (§3.3's closed-group ruling, generalised).
    Hidden,
    /// The viewer may read the resource but not do this to it.
    Forbidden,
}

pub type Decision = Result<(), Denied>;

pub fn decide(capability: Capability, target: Target, action: Action) -> Decision {
    let can_read = match (capability, target) {
        (Capability::None, _) => return Err(Denied::NoAccess),
        (Capability::Admin, _) => true,
        (Capability::Member, Target::Fau) => true,
        (Capability::Guest, Target::Fau) => false,
        (Capability::Member, Target::Group(g) | Target::GroupContent(g)) => {
            g.visibility == Visibility::Open || g.viewer_in_group
        }
        (Capability::Guest, Target::Group(g) | Target::GroupContent(g)) => g.viewer_in_group,
    };
    if !can_read {
        return Err(Denied::Hidden);
    }
    let admin = capability == Capability::Admin;
    let allowed = match (action, target) {
        (Action::Read, _) => true,
        (Action::Manage, _) | (Action::Write, Target::Group(_)) => admin,
        // Only admins and members read FAU-wide content, and both may write it.
        (Action::Write, Target::Fau) => true,
        (Action::Write, Target::GroupContent(g)) => !g.archived && (admin || g.viewer_in_group),
    };
    if allowed {
        Ok(())
    } else {
        Err(Denied::Forbidden)
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p fau-domain authz && cargo test -p fau-domain --test dependency_boundary`
Expected: 4 `authz` tests pass. The dependency boundary still holds, since no crate was added.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add backend/crates/domain/src/authz.rs backend/crates/domain/src/lib.rs
git commit -m "Add the authorization rule as one pure function (#3501)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: `authorize`, the one function that reads the database, and the authorization matrix (acceptance)

**Files:**
- Modify: `backend/crates/persistence/src/membership/access.rs` (whole file below)
- Create: `backend/crates/persistence/src/membership/authz.rs`
- Modify: `backend/crates/persistence/src/membership/mod.rs` (declare `mod authz;`, re-export)
- Create: `backend/crates/app/tests/common/groups.rs`
- Modify: `backend/crates/app/tests/common/mod.rs` (add `pub mod groups;` after `pub mod membership;`)
- Test: `backend/crates/app/tests/authorization.rs`

**Interfaces:**
- Consumes: `decide`, `Action`, `Decision`, `Denied`, `GroupFacts`, `Target` (Task 3), and the 0007 tables (Task 2).
- Produces:
  - `pub struct Viewer { pub tenant_id: Uuid, pub membership_id: Uuid }` (`Debug, Clone, Copy, PartialEq, Eq, Hash`);
  - `pub enum Resource { Fau, Group(Uuid), GroupContent(Uuid) }` (same derives), with `pub fn Resource::audience(group_id: Option<Uuid>) -> Resource`;
  - `pub async fn authorize(conn: &mut PgConnection, viewer: Viewer, resource: Resource, action: Action, at: Moment) -> Result<Decision, MembershipError>`;
  - `pub async fn read_transaction(pool: &PgPool) -> Result<Transaction<'static, Postgres>, MembershipError>`.

  All four are re-exported from `fau_persistence::membership`.
- Crate-internal:
  - `pub(crate) async fn membership_access(conn: &mut PgConnection, tenant_id: Uuid, membership_id: Uuid, today: Date) -> Result<Access, MembershipError>`;
  - `pub(crate) fn in_group_sql() -> String`, which uses `$2` for the membership and `$3` for the date as text, with the group row aliased `g`;
  - `pub(crate) const ROLE_FOLLOWS_GROUP` and `pub(crate) const ASSIGNMENT_VALID_ON_3`.
- Test fixtures in `common::groups`:
  - `placeholder_name()`;
  - `seed_group`, `seed_bound_group`, `seed_group_member`, `seed_role`, `seed_unit`, `seed_cohort`;
  - `add_guest`, `live_assignment`, `revoke`;
  - `VIEWERS`, `World`, `world(pool)`.

- [ ] **Step 1: Write the fixtures and the failing tests**

Add `pub mod groups;` to `backend/crates/app/tests/common/mod.rs` after `pub mod membership;`.

Create `backend/crates/app/tests/common/groups.rs`:

```rust
//! Group fixtures for the integration tests. Groups, hand-added members, guest roles and
//! school structure are written straight to the database, so a test of `authorize` or of
//! the group transactions does not depend on the code it exercises. People still join
//! through the invitation transactions (`add_member`), as everywhere else.

use fau_domain::membership::period::Period;
use fau_domain::membership::vocabulary::{CapabilityClass, Visibility};
use fau_domain::time::Moment;
use fau_persistence::membership::{
    revoke_role_assignment, Accepted, RevokeAssignment, RoleChoice, Viewer,
};
use sqlx::PgPool;
use uuid::Uuid;

use super::membership::*;

/// Every fixture row is dated T0, so a transaction at T0 or later may remove it
/// (`group_member_removed_after_added`).
const FIXTURE_AT: &str = T0;

/// Shaped like fau-crypto's envelope (version byte 1, then 41 bytes), so
/// `groups_name_is_an_envelope` accepts it. Never decrypted.
pub fn placeholder_name() -> Vec<u8> {
    let mut v = vec![1u8];
    v.extend_from_slice(&[0u8; 41]);
    v
}

pub async fn seed_group(pool: &PgPool, fau: &Fau, visibility: Visibility) -> Uuid {
    seed_bound_group(pool, fau, visibility, None, None).await
}

pub async fn seed_bound_group(
    pool: &PgPool,
    fau: &Fau,
    visibility: Visibility,
    unit: Option<Uuid>,
    cohort: Option<Uuid>,
) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into groups (tenant_id, id, encrypted_name, visibility, unit_id, cohort_id, created_by, created_at)
         values ($1, $2, $3, $4, $5, $6, $7, $8::timestamptz)",
    )
    .bind(fau.tenant_id)
    .bind(id)
    .bind(placeholder_name())
    .bind(visibility.code())
    .bind(unit)
    .bind(cohort)
    .bind(fau.admin_membership_id)
    .bind(FIXTURE_AT)
    .execute(pool)
    .await
    .expect("seed a group");
    id
}

pub async fn seed_group_member(pool: &PgPool, fau: &Fau, group: Uuid, membership: Uuid) {
    sqlx::query(
        "insert into group_members (tenant_id, id, group_id, membership_id, added_by, added_at)
         values ($1, $2, $3, $4, $5, $6::timestamptz)",
    )
    .bind(fau.tenant_id)
    .bind(Uuid::now_v7())
    .bind(group)
    .bind(membership)
    .bind(fau.admin_membership_id)
    .bind(FIXTURE_AT)
    .execute(pool)
    .await
    .expect("seed a group member");
}

pub async fn seed_role(
    pool: &PgPool,
    tenant: Uuid,
    class: CapabilityClass,
    group: Option<Uuid>,
    unit: Option<Uuid>,
    cohort: Option<Uuid>,
) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into roles (tenant_id, id, name, capability_class, group_id, unit_id, cohort_id)
         values ($1, $2, 'Fixture', $3, $4, $5, $6)",
    )
    .bind(tenant)
    .bind(id)
    .bind(class.code())
    .bind(group)
    .bind(unit)
    .bind(cohort)
    .execute(pool)
    .await
    .expect("seed a role");
    id
}

/// A unit in a school year of its own, so names never collide.
pub async fn seed_unit(pool: &PgPool, tenant: Uuid) -> Uuid {
    let (year, unit) = (Uuid::now_v7(), Uuid::now_v7());
    sqlx::query(
        "insert into school_years (tenant_id, id, name, starts_on, ends_on_exclusive)
         values ($1, $2, '2026/27', '2026-08-01', '2027-08-01')",
    )
    .bind(tenant)
    .bind(year)
    .execute(pool)
    .await
    .expect("seed a school year");
    sqlx::query(
        "insert into organization_units (tenant_id, id, school_year_id, kind, name)
         values ($1, $2, $3, 'grade', $4)",
    )
    .bind(tenant)
    .bind(unit)
    .bind(year)
    .bind(format!("unit-{unit}"))
    .execute(pool)
    .await
    .expect("seed a unit");
    unit
}

pub async fn seed_cohort(pool: &PgPool, tenant: Uuid) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query("insert into cohorts (tenant_id, id, name) values ($1, $2, $3)")
        .bind(tenant)
        .bind(id)
        .bind(format!("cohort-{id}"))
        .execute(pool)
        .await
        .expect("seed a cohort");
    id
}

/// `address` joins as a guest of `group`, through a real invitation.
pub async fn add_guest(
    pool: &PgPool,
    fau: &Fau,
    address: &str,
    group: Uuid,
    period: Period,
    at: Moment,
) -> Accepted {
    let role = seed_role(pool, fau.tenant_id, CapabilityClass::Guest, Some(group), None, None).await;
    add_member(pool, fau, address, RoleChoice::Existing(role), period, at).await
}

/// The membership's oldest unrevoked role assignment.
pub async fn live_assignment(pool: &PgPool, membership: Uuid) -> Uuid {
    sqlx::query_scalar(
        "select id from role_assignments where membership_id = $1 and revoked_at is null order by id limit 1",
    )
    .bind(membership)
    .fetch_one(pool)
    .await
    .expect("a live assignment")
}

/// The FAU's admin revokes one assignment.
pub async fn revoke(pool: &PgPool, fau: &Fau, assignment_id: Uuid, at: Moment) {
    revoke_role_assignment(
        pool,
        RevokeAssignment {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            assignment_id,
            confirm_no_admin: false,
        },
        at,
    )
    .await
    .expect("revoke");
}

/// The eight viewers the matrix runs over: every capability class, in and out of the
/// groups. `none_*` held a member role that was revoked, so they have no standing.
pub const VIEWERS: [&str; 8] = [
    "admin_in", "admin_out", "member_in", "member_out", "guest_in", "guest_out", "none_in", "none_out",
];

/// One FAU with three groups and eight viewers, at T0 (23 September 2026).
///
/// - `open` and `closed`: `admin_in`, `member_in` and `none_in` are added to both by hand.
///   `guest_in` holds a guest role naming `open` and is added to `closed` by hand, so both
///   ways into a group are exercised.
/// - `other`: a closed group whose only member is `guest_out`, by its guest role.
/// - `admin_out` and `member_out` are in no group.
pub struct World {
    pub fau: Fau,
    pub open: Uuid,
    pub closed: Uuid,
    pub other: Uuid,
    members: Vec<(&'static str, Uuid)>,
}

impl World {
    pub fn membership(&self, name: &str) -> Uuid {
        self.members
            .iter()
            .find(|(n, _)| *n == name)
            .unwrap_or_else(|| panic!("no viewer named {name}"))
            .1
    }

    pub fn viewer(&self, name: &str) -> Viewer {
        Viewer {
            tenant_id: self.fau.tenant_id,
            membership_id: self.membership(name),
        }
    }
}

async fn plain_member(pool: &PgPool, fau: &Fau, address: &str, year: Period, t0: Moment) -> Uuid {
    add_member(pool, fau, address, new_role("Medlem", CapabilityClass::Member), year, t0)
        .await
        .membership_id
}

pub async fn world(pool: &PgPool) -> World {
    let t0 = at(T0);
    let fau = active_fau(pool, "admin@example.test", t0).await;
    let open = seed_group(pool, &fau, Visibility::Open).await;
    let closed = seed_group(pool, &fau, Visibility::Closed).await;
    let other = seed_group(pool, &fau, Visibility::Closed).await;
    let year = period(day(2026, 9, 1), day(2027, 9, 1));

    let admin_in = fau.admin_membership_id;
    let admin_out = add_member(
        pool,
        &fau,
        "admin-out@example.test",
        RoleChoice::Existing(fau.admin_role_id),
        year,
        t0,
    )
    .await
    .membership_id;
    let member_in = plain_member(pool, &fau, "member-in@example.test", year, t0).await;
    let member_out = plain_member(pool, &fau, "member-out@example.test", year, t0).await;
    let none_in = plain_member(pool, &fau, "none-in@example.test", year, t0).await;
    let none_out = plain_member(pool, &fau, "none-out@example.test", year, t0).await;
    let guest_in = add_guest(pool, &fau, "guest-in@example.test", open, year, t0).await.membership_id;
    let guest_out = add_guest(pool, &fau, "guest-out@example.test", other, year, t0).await.membership_id;

    for m in [admin_in, member_in, none_in] {
        seed_group_member(pool, &fau, open, m).await;
        seed_group_member(pool, &fau, closed, m).await;
    }
    seed_group_member(pool, &fau, closed, guest_in).await;
    for m in [none_in, none_out] {
        revoke(pool, &fau, live_assignment(pool, m).await, t0).await;
    }

    World {
        members: vec![
            ("admin_in", admin_in),
            ("admin_out", admin_out),
            ("member_in", member_in),
            ("member_out", member_out),
            ("guest_in", guest_in),
            ("guest_out", guest_out),
            ("none_in", none_in),
            ("none_out", none_out),
        ],
        fau,
        open,
        closed,
        other,
    }
}
```

Create `backend/crates/app/tests/authorization.rs`:

```rust
//! The authorization matrix (groups design §3.3 and §10): viewer class × group visibility ×
//! in or out of the group × resource type, through the one function that reads the
//! database. **A new resource type must add its rows to `MATRIX`.**

mod common;
use common::groups::*;
use common::membership::*;
use common::TestDb;
use fau_domain::authz::{Action, Decision, Denied};
use fau_domain::membership::vocabulary::{CapabilityClass, Visibility};
use fau_persistence::membership::{authorize, grant_role, GrantRole, Resource, RoleChoice, Viewer};
use sqlx::PgPool;
use uuid::Uuid;

/// Allowed, Forbidden, Hidden (answered as not found), No access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum O {
    A,
    F,
    H,
    N,
}
use O::*;

fn outcome(d: Decision) -> O {
    match d {
        Ok(()) => A,
        Err(Denied::Forbidden) => F,
        Err(Denied::Hidden) => H,
        Err(Denied::NoAccess) => N,
    }
}

#[derive(Debug, Clone, Copy)]
enum R {
    /// FAU-wide content (audience null).
    Fau,
    OpenContent,
    ClosedContent,
    OpenGroup,
    ClosedGroup,
    /// A group id that does not exist: must look exactly like a hidden one.
    UnknownGroup,
}

/// (viewer, resource, read, write, manage). Every viewer in `VIEWERS`, every resource type.
#[rustfmt::skip]
const MATRIX: &[(&str, R, O, O, O)] = &[
    // FAU-wide content: members and admins, never guests.
    ("admin_in",   R::Fau, A, A, A),
    ("admin_out",  R::Fau, A, A, A),
    ("member_in",  R::Fau, A, A, F),
    ("member_out", R::Fau, A, A, F),
    ("guest_in",   R::Fau, H, H, H),
    ("guest_out",  R::Fau, H, H, H),
    ("none_in",    R::Fau, N, N, N),
    ("none_out",   R::Fau, N, N, N),
    // Content in an open group: members read; only those in it write.
    ("admin_in",   R::OpenContent, A, A, A),
    ("admin_out",  R::OpenContent, A, A, A),
    ("member_in",  R::OpenContent, A, A, F),
    ("member_out", R::OpenContent, A, F, F),
    ("guest_in",   R::OpenContent, A, A, F),
    ("guest_out",  R::OpenContent, H, H, H),
    ("none_in",    R::OpenContent, N, N, N),
    ("none_out",   R::OpenContent, N, N, N),
    // Content in a closed group: only those in it, and admins.
    ("admin_in",   R::ClosedContent, A, A, A),
    ("admin_out",  R::ClosedContent, A, A, A),
    ("member_in",  R::ClosedContent, A, A, F),
    ("member_out", R::ClosedContent, H, H, H),
    ("guest_in",   R::ClosedContent, A, A, F),
    ("guest_out",  R::ClosedContent, H, H, H),
    ("none_in",    R::ClosedContent, N, N, N),
    ("none_out",   R::ClosedContent, N, N, N),
    // An open group itself: writing to it is managing it, which is admin-only.
    ("admin_in",   R::OpenGroup, A, A, A),
    ("admin_out",  R::OpenGroup, A, A, A),
    ("member_in",  R::OpenGroup, A, F, F),
    ("member_out", R::OpenGroup, A, F, F),
    ("guest_in",   R::OpenGroup, A, F, F),
    ("guest_out",  R::OpenGroup, H, H, H),
    ("none_in",    R::OpenGroup, N, N, N),
    ("none_out",   R::OpenGroup, N, N, N),
    // A closed group itself: its existence and name are hidden from those outside it.
    ("admin_in",   R::ClosedGroup, A, A, A),
    ("admin_out",  R::ClosedGroup, A, A, A),
    ("member_in",  R::ClosedGroup, A, F, F),
    ("member_out", R::ClosedGroup, H, H, H),
    ("guest_in",   R::ClosedGroup, A, F, F),
    ("guest_out",  R::ClosedGroup, H, H, H),
    ("none_in",    R::ClosedGroup, N, N, N),
    ("none_out",   R::ClosedGroup, N, N, N),
    // An unknown id: indistinguishable from a hidden group, even for an admin.
    ("admin_in",   R::UnknownGroup, H, H, H),
    ("admin_out",  R::UnknownGroup, H, H, H),
    ("member_in",  R::UnknownGroup, H, H, H),
    ("member_out", R::UnknownGroup, H, H, H),
    ("guest_in",   R::UnknownGroup, H, H, H),
    ("guest_out",  R::UnknownGroup, H, H, H),
    ("none_in",    R::UnknownGroup, N, N, N),
    ("none_out",   R::UnknownGroup, N, N, N),
];

async fn check(pool: &PgPool, viewer: Viewer, resource: Resource, action: Action, at_: &str) -> O {
    let mut conn = pool.acquire().await.unwrap();
    outcome(authorize(&mut conn, viewer, resource, action, at(at_)).await.unwrap())
}

#[tokio::test]
async fn the_authorization_matrix() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let unknown = Uuid::now_v7();

    // Every viewer appears once per resource type.
    for r in ["Fau", "OpenContent", "ClosedContent", "OpenGroup", "ClosedGroup", "UnknownGroup"] {
        let mut viewers: Vec<&str> = MATRIX
            .iter()
            .filter(|row| format!("{:?}", row.1) == r)
            .map(|row| row.0)
            .collect();
        viewers.sort_unstable();
        let mut all = VIEWERS.to_vec();
        all.sort_unstable();
        assert_eq!(viewers, all, "{r}");
    }

    let mut failures = Vec::new();
    for &(viewer, r, read, write, manage) in MATRIX {
        let resource = match r {
            R::Fau => Resource::Fau,
            R::OpenContent => Resource::GroupContent(w.open),
            R::ClosedContent => Resource::GroupContent(w.closed),
            R::OpenGroup => Resource::Group(w.open),
            R::ClosedGroup => Resource::Group(w.closed),
            R::UnknownGroup => Resource::Group(unknown),
        };
        for (action, want) in [(Action::Read, read), (Action::Write, write), (Action::Manage, manage)] {
            let got = check(&pool, w.viewer(viewer), resource, action, T0).await;
            if got != want {
                failures.push(format!("{viewer} {r:?} {action:?}: want {want:?}, got {got:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[tokio::test]
async fn an_archived_group_is_read_only_but_still_managed() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    sqlx::query("update groups set archived_at = $1::timestamptz where id = $2")
        .bind(T0)
        .bind(w.open)
        .execute(&pool)
        .await
        .unwrap();
    let content = Resource::GroupContent(w.open);
    assert_eq!(check(&pool, w.viewer("admin_out"), content, Action::Read, T0).await, A);
    assert_eq!(check(&pool, w.viewer("admin_out"), content, Action::Write, T0).await, F);
    assert_eq!(check(&pool, w.viewer("member_in"), content, Action::Write, T0).await, F);
    assert_eq!(check(&pool, w.viewer("guest_in"), content, Action::Read, T0).await, A);
    assert_eq!(
        check(&pool, w.viewer("admin_out"), Resource::Group(w.open), Action::Manage, T0).await,
        A
    );
}

#[tokio::test]
async fn a_bound_group_follows_roles_on_its_unit_or_cohort_on_the_current_date() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let unit = seed_unit(&pool, fau.tenant_id).await;
    let cohort = seed_cohort(&pool, fau.tenant_id).await;
    let unit_role = seed_role(&pool, fau.tenant_id, CapabilityClass::Member, None, Some(unit), None).await;
    let cohort_role = seed_role(&pool, fau.tenant_id, CapabilityClass::Member, None, None, Some(cohort)).await;
    let by_unit = seed_bound_group(&pool, &fau, Visibility::Closed, Some(unit), None).await;
    let by_cohort = seed_bound_group(&pool, &fau, Visibility::Closed, None, Some(cohort)).await;

    // A member for two years, contact parent on the unit until New Year.
    let parent = add_member(
        &pool,
        &fau,
        "kontakt@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 1), day(2028, 9, 1)),
        t0,
    )
    .await;
    grant_role(
        &pool,
        GrantRole {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: parent.membership_id,
            role: RoleChoice::Existing(unit_role),
            period: period(day(2026, 9, 1), day(2027, 1, 1)),
        },
        t0,
    )
    .await
    .unwrap();
    let v = Viewer { tenant_id: fau.tenant_id, membership_id: parent.membership_id };
    let read = Action::Read;
    assert_eq!(check(&pool, v, Resource::GroupContent(by_unit), read, T0).await, A);
    assert_eq!(check(&pool, v, Resource::GroupContent(by_cohort), read, T0).await, H);
    // On 1 January the unit role has ended; no job ran, and the group no longer holds them.
    let new_year = "2026-12-31T23:00:00Z";
    assert_eq!(check(&pool, v, Resource::GroupContent(by_unit), read, new_year).await, H);
    assert_eq!(check(&pool, v, Resource::Fau, read, new_year).await, A, "still a member");

    // A cohort role reaches the cohort's group, not the unit's: there is no traversal
    // through unit_cohorts (Ruling R6).
    let cohort_parent = add_member(
        &pool,
        &fau,
        "kull@example.test",
        RoleChoice::Existing(cohort_role),
        period(day(2026, 9, 1), day(2027, 9, 1)),
        t0,
    )
    .await;
    let c = Viewer { tenant_id: fau.tenant_id, membership_id: cohort_parent.membership_id };
    assert_eq!(check(&pool, c, Resource::GroupContent(by_cohort), read, T0).await, A);
    assert_eq!(check(&pool, c, Resource::GroupContent(by_unit), read, T0).await, H);

    // A bound group can still take a hand-added member.
    seed_group_member(&pool, &fau, by_unit, cohort_parent.membership_id).await;
    assert_eq!(check(&pool, c, Resource::GroupContent(by_unit), read, T0).await, A);
}

#[tokio::test]
async fn a_revocation_takes_effect_on_the_next_call() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let v = w.viewer("member_in");
    let closed = Resource::GroupContent(w.closed);
    assert_eq!(check(&pool, v, closed, Action::Read, T0).await, A);

    sqlx::query(
        "update group_members set removed_at = $1::timestamptz where group_id = $2 and membership_id = $3",
    )
    .bind(T0)
    .bind(w.closed)
    .bind(v.membership_id)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(check(&pool, v, closed, Action::Read, T0).await, H);

    revoke(&pool, &w.fau, live_assignment(&pool, v.membership_id).await, at(T0)).await;
    assert_eq!(check(&pool, v, Resource::Fau, Action::Read, T0).await, N);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p fau-app --test authorization`
Expected: does not compile, because `authorize`, `Resource` and `Viewer` are not in `fau_persistence::membership`.

- [ ] **Step 3: Share the per-membership access check**

Replace `backend/crates/persistence/src/membership/access.rs` with:

```rust
//! The per-request access check (spec 2.1, 4; #3417 calls it on every request), and its
//! per-membership form, which `authorize` and the change stream share.
//!
//! A capability is standing, not permission. `Capability::Guest` reaches only its own
//! groups, and even a member does not read a closed group they are not in. So resource
//! reads ask `authorize` (groups design §3.3), and nothing may read `capability >= Member`
//! as "may read everything" (the #3418 read-path audit, #3501).

use fau_domain::membership::access::{
    evaluate_access, Access, AssignmentView, GrantView, Standing,
};
use fau_domain::membership::vocabulary::CapabilityClass;
use fau_domain::time::Moment;
use jiff::civil::Date;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::error::MembershipError;
use super::sql::{period_from, ACCOUNT_USABLE, MEMBERSHIP_NOT_REVOKED};

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
    // One snapshot for every read (final review M3): at READ COMMITTED each statement
    // sees its own snapshot, so a revocation committing between them could pair a
    // membership's old standing with its new assignments. REPEATABLE READ fixes the
    // snapshot at the first query; READ ONLY because nothing here writes. It must be the
    // transaction's first statement.
    sqlx::query("set transaction isolation level repeatable read, read only")
        .execute(&mut *tx)
        .await?;
    let membership_id: Option<Uuid> =
        sqlx::query_scalar("select id from memberships where tenant_id = $1 and account_id = $2")
            .bind(tenant_id)
            .bind(account_id)
            .fetch_optional(&mut *tx)
            .await?;
    let access = match membership_id {
        Some(id) => membership_access(&mut tx, tenant_id, id, at.today()).await?,
        None => Access::NONE,
    };
    tx.commit().await?;
    Ok(access)
}

/// What `membership_id` may do in `tenant_id` on `today`, on the caller's connection, so
/// it shares the caller's transaction and snapshot. An unknown membership is no access.
pub(crate) async fn membership_access(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
    today: Date,
) -> Result<Access, MembershipError> {
    // The two `Standing` fields that `USABLE_ACCOUNT` normally checks together, read from
    // its own named conjuncts rather than retyped by hand (fix round 1, task 10):
    // `sql::tests::usable_account_is_the_conjunction_of_its_two_parts` keeps all three
    // in sync.
    let sql = format!(
        "select t.status = 'active', ({ACCOUNT_USABLE}), {MEMBERSHIP_NOT_REVOKED}
           from memberships m
           join tenants t  on t.id = m.tenant_id
           join accounts a on a.id = m.account_id
          where m.tenant_id = $1 and m.id = $2"
    );
    let standing: Option<(bool, bool, bool)> = sqlx::query_as(&sql)
        .bind(tenant_id)
        .bind(membership_id)
        .fetch_optional(&mut *conn)
        .await?;
    let Some((tenant_active, account_usable, membership_active)) = standing else {
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
    .fetch_all(&mut *conn)
    .await?;
    let mut assignments = Vec::with_capacity(rows.len());
    for (class, starts, ends, revoked) in rows {
        assignments.push(AssignmentView {
            capability: CapabilityClass::from_code(&class).ok_or_else(MembershipError::decode)?,
            period: period_from(&starts, &ends)?,
            revoked,
        });
    }

    // Defence in depth (Task 9's pattern in `sql.rs`, `AdminState::load` and
    // `handover_grant_valid`): a grant whose source assignment has since been revoked
    // is ignored outright, not just marked `revoked` on the view. Revoking an
    // assignment already cascades to revoke any grant it produced
    // (`revoke_role_assignment`), so this is a second, independent check rather than
    // the only one.
    let rows: Vec<(String, String, bool)> = sqlx::query_as(
        "select to_char(g.starts_on, 'YYYY-MM-DD'), to_char(g.ends_on_exclusive, 'YYYY-MM-DD'),
                g.revoked_at is not null
           from handover_grants g
           join role_assignments ra on ra.tenant_id = g.tenant_id and ra.id = g.source_assignment_id
          where g.tenant_id = $1 and ra.membership_id = $2 and ra.revoked_at is null",
    )
    .bind(tenant_id)
    .bind(membership_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut grants = Vec::with_capacity(rows.len());
    for (starts, ends, revoked) in rows {
        grants.push(GrantView {
            period: period_from(&starts, &ends)?,
            revoked,
        });
    }

    Ok(evaluate_access(
        Standing {
            tenant_active,
            account_usable,
            membership_active,
        },
        &assignments,
        &grants,
        today,
    ))
}
```

- [ ] **Step 4: Write `authorize`**

Create `backend/crates/persistence/src/membership/authz.rs`:

```rust
//! The one authorization function (groups design §3.3). Every read path, every group
//! mutation and every change-stream delivery asks [`authorize`]. It reads the database on
//! each call -- the viewer's standing today and the facts about the resource -- and
//! applies `fau_domain::authz::decide`, so a revocation takes effect on the next call.
//!
//! List reads (`groups::list_groups`) read the same facts with the same SQL
//! ([`in_group_sql`]) in one snapshot and call `decide` per row (Ruling R11). A test holds
//! the two equal.
//!
//! **Adding a resource type** (#3419's folders and documents, #3503's threads, ...)
//! means:
//! - a new [`Resource`] variant, resolved here to a `Target`, usually through its row's
//!   `group_id` and [`Resource::audience`];
//! - new rows in `tests/authorization.rs`.

use fau_domain::authz::{decide, Action, Decision, Denied, GroupFacts, Target};
use fau_domain::membership::access::Capability;
use fau_domain::membership::vocabulary::Visibility;
use fau_domain::time::Moment;
use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::access::membership_access;
use super::error::MembershipError;
use super::sql::date_param;

/// Who is asking: a membership in one FAU. #3417 resolves the session's account to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Viewer {
    pub tenant_id: Uuid,
    pub membership_id: Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Resource {
    /// FAU-wide content (audience null, §3.2), and the FAU itself for `Manage`: creating
    /// a group.
    Fau,
    /// A group itself: that it exists, its name, its member list.
    Group(Uuid),
    /// Content whose audience is the group.
    GroupContent(Uuid),
}

impl Resource {
    /// The audience of a resource that carries one nullable `group_id` (§3.2): null is
    /// FAU-wide. Threads, events, polls and folders resolve through this.
    pub fn audience(group_id: Option<Uuid>) -> Self {
        match group_id {
            Some(id) => Resource::GroupContent(id),
            None => Resource::Fau,
        }
    }
}

/// A role puts its holder into group `g` when it names the group (a guest role) or sits
/// on the unit or cohort the group is bound to (§3.1). Literal matches only: no traversal
/// through `unit_cohorts` (Ruling R6). `r` is `roles`.
pub(crate) const ROLE_FOLLOWS_GROUP: &str =
    "(r.group_id = g.id or r.unit_id = g.unit_id or r.cohort_id = g.cohort_id)";

/// Assignment `ra` is valid on the date bound as `$3`.
pub(crate) const ASSIGNMENT_VALID_ON_3: &str =
    "ra.revoked_at is null and ra.starts_on <= $3::date and ra.ends_on_exclusive > $3::date";

/// Whether membership `$2` is a current member of the group row aliased `g` on date `$3`:
/// added by hand and not removed, or holding a role valid on `$3` that the group follows.
/// Derived, never copied, so turnover needs no job (§3.1).
pub(crate) fn in_group_sql() -> String {
    format!(
        "(exists (select 1 from group_members gm
                   where gm.tenant_id = g.tenant_id and gm.group_id = g.id
                     and gm.membership_id = $2 and gm.removed_at is null)
          or exists (select 1 from role_assignments ra
                       join roles r on r.tenant_id = ra.tenant_id and r.id = ra.role_id
                      where ra.tenant_id = g.tenant_id and ra.membership_id = $2
                        and {ASSIGNMENT_VALID_ON_3}
                        and {ROLE_FOLLOWS_GROUP}))"
    )
}

/// Decides `action` on `resource` for `viewer`, reading the database on `conn`. Mutations
/// call it inside their own transaction, after `lock_tenant`. Read paths call it inside
/// [`read_transaction`], so the decision and the read see one snapshot.
///
/// The error is only ever a database failure. A refusal is the inner `Err(Denied)`:
/// - `NoAccess`: the viewer has no standing in the FAU;
/// - `Hidden`: the viewer may not read the resource, or the resource does not exist, and
///   the two look the same;
/// - `Forbidden`: the viewer may read the resource but not do this to it.
pub async fn authorize(
    conn: &mut PgConnection,
    viewer: Viewer,
    resource: Resource,
    action: Action,
    at: Moment,
) -> Result<Decision, MembershipError> {
    let access = membership_access(conn, viewer.tenant_id, viewer.membership_id, at.today()).await?;
    if access.capability == Capability::None {
        return Ok(Err(Denied::NoAccess));
    }
    let target = match resource {
        Resource::Fau => Target::Fau,
        Resource::Group(id) => match group_facts(conn, viewer, id, at).await? {
            Some(facts) => Target::Group(facts),
            None => return Ok(Err(Denied::Hidden)),
        },
        Resource::GroupContent(id) => match group_facts(conn, viewer, id, at).await? {
            Some(facts) => Target::GroupContent(facts),
            None => return Ok(Err(Denied::Hidden)),
        },
    };
    Ok(decide(access.capability, target, action))
}

async fn group_facts(
    conn: &mut PgConnection,
    viewer: Viewer,
    group_id: Uuid,
    at: Moment,
) -> Result<Option<GroupFacts>, MembershipError> {
    let sql = format!(
        "select g.visibility, g.archived_at is not null, {}
           from groups g
          where g.tenant_id = $1 and g.id = $4",
        in_group_sql()
    );
    let row: Option<(String, bool, bool)> = sqlx::query_as(&sql)
        .bind(viewer.tenant_id)
        .bind(viewer.membership_id)
        .bind(date_param(at.today()))
        .bind(group_id)
        .fetch_optional(&mut *conn)
        .await?;
    match row {
        None => Ok(None),
        Some((visibility, archived, viewer_in_group)) => Ok(Some(GroupFacts {
            visibility: Visibility::from_code(&visibility).ok_or_else(MembershipError::decode)?,
            archived,
            viewer_in_group,
        })),
    }
}

/// A snapshot for a read path: REPEATABLE READ, READ ONLY, so the authorization and the
/// read see the same database state (the reasoning of final review M3 in
/// `effective_access`).
pub async fn read_transaction(pool: &PgPool) -> Result<Transaction<'static, Postgres>, MembershipError> {
    let mut tx = pool.begin().await?;
    sqlx::query("set transaction isolation level repeatable read, read only")
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_audience_column_maps_to_a_resource() {
        let g = Uuid::now_v7();
        assert_eq!(Resource::audience(None), Resource::Fau);
        assert_eq!(Resource::audience(Some(g)), Resource::GroupContent(g));
    }

    #[test]
    fn in_group_is_built_from_the_shared_fragments() {
        let sql = in_group_sql();
        assert!(sql.contains(ROLE_FOLLOWS_GROUP) && sql.contains(ASSIGNMENT_VALID_ON_3));
    }
}
```

In `backend/crates/persistence/src/membership/mod.rs`, add `mod authz;` after `mod access;`, and add after `pub use access::effective_access;`:

```rust
pub use authz::{authorize, read_transaction, Resource, Viewer};
```

Also add this paragraph to the end of the module doc comment:

```rust
//!
//! **Authorization** (groups design §3.3, #3501): `authorize` is the one function that
//! reads the database to decide what a membership may do with a resource. Every read path
//! and every change-stream delivery goes through it or through `fau_domain::authz::decide`
//! on the same facts.
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p fau-persistence authz && cargo test -p fau-app --test authorization --test access`
Expected:
- the 2 unit tests pass;
- `authorization` passes 4 tests, and `the_authorization_matrix` reports no failures across 48 rows × 3 actions;
- `access`'s existing tests all still pass, which proves the `effective_access` refactor kept its behaviour.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add backend/crates/persistence/src/membership/access.rs backend/crates/persistence/src/membership/authz.rs \
  backend/crates/persistence/src/membership/mod.rs backend/crates/app/tests/common/mod.rs \
  backend/crates/app/tests/common/groups.rs backend/crates/app/tests/authorization.rs
git commit -m "Add authorize, the one function that decides access, with the authorization matrix (#3501)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: The #3418 read-path audit: route through `authorize`, keep guests out of FAU-wide recipient lists

**Files:**
- Modify: `backend/crates/persistence/src/membership/authz.rs` (add `denied`)
- Modify: `backend/crates/persistence/src/membership/requests.rs` (`access_request_message`, and imports)
- Modify: `backend/crates/persistence/src/membership/sql.rs` (`current_member_emails` doc, `member_emails`, `recovery_notice_recipients`)
- Test: `backend/crates/app/tests/read_paths.rs`

**Interfaces:**
- Consumes: `authorize`, `read_transaction`, `Resource`, `Viewer` (Task 4), and the fixtures `seed_group`, `add_guest` (Task 4).
- Produces: `pub(crate) fn denied(d: Denied, hidden: MembershipError) -> MembershipError`, which maps `Forbidden` and `NoAccess` to `NotAuthorized` and `Hidden` to `hidden`. Tasks 7 and 8 use it.

This task implements the "Read-path audit" table at the top of this plan. `invitation_message` is left alone: its reader is the invitee, who is not a member yet, and the token plus a matching verified address authorize it.

- [ ] **Step 1: Write the failing tests**

Create `backend/crates/app/tests/read_paths.rs`:

```rust
//! The #3418 read-path audit (groups design §3.3, last bullet; #3501). The membership
//! foundation predates guests, so every path that read "any valid role" as "a member" was
//! checked. These tests pin the places where a guest would otherwise have counted as an
//! FAU-wide member.

mod common;
use common::groups::*;
use common::membership::*;
use common::TestDb;
use fau_domain::membership::access::Capability;
use fau_domain::membership::vocabulary::{CapabilityClass, Visibility};
use fau_persistence::membership::{
    access_request_message, create_access_request, effective_access, recovery_grant_admin,
    revoke_membership, AccessRequestMessage, CreateAccessRequest, MembershipError, RecoveryActor,
    RecoveryGrant, RevokeMembership, RoleChoice,
};
use sqlx::PgPool;
use uuid::Uuid;

fn ct(s: &str) -> fau_crypto::MessageCiphertext {
    fau_crypto::MessageCiphertext::new(format!("vault:v1:{s}")).unwrap()
}

async fn admin_leaves(pool: &PgPool, fau: &Fau) {
    revoke_membership(
        pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: fau.admin_membership_id,
            confirm_no_admin: true,
        },
        at(T0),
    )
    .await
    .unwrap();
}

async fn recover(pool: &PgPool, fau: &Fau) {
    recovery_grant_admin(
        pool,
        RecoveryGrant {
            tenant_id: fau.tenant_id,
            actor: RecoveryActor::Ewb,
            recipient: email("ny-leder@example.test"),
            role: RoleChoice::Existing(fau.admin_role_id),
            period: period(day(2026, 9, 23), day(2027, 10, 1)),
        },
        at(T0),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_guest_is_a_guest_and_cannot_read_an_access_request_message() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let group = seed_group(&pool, &fau, Visibility::Open).await;
    let guest = add_guest(&pool, &fau, "gjest@example.test", group, period(day(2026, 9, 1), day(2027, 9, 1)), t0).await;
    assert_eq!(
        effective_access(&pool, guest.account_id, fau.tenant_id, t0).await.unwrap().capability,
        Capability::Guest
    );

    let request_id = Uuid::now_v7();
    create_access_request(
        &pool,
        CreateAccessRequest {
            tenant_id: fau.tenant_id,
            requester: verified("ny@example.test"),
            message: Some(AccessRequestMessage {
                request_id,
                ciphertext: ct("c2VhbGVk"),
            }),
        },
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        access_request_message(&pool, fau.tenant_id, request_id, fau.admin_membership_id, t0)
            .await
            .unwrap(),
        Some(ct("c2VhbGVk"))
    );
    for request in [request_id, Uuid::now_v7()] {
        assert_eq!(
            access_request_message(&pool, fau.tenant_id, request, guest.membership_id, t0)
                .await
                .unwrap_err(),
            MembershipError::NotAuthorized,
            "authority before the row, and a guest has none"
        );
    }
}

#[tokio::test]
async fn guests_are_not_told_about_a_recovery() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let year = period(day(2026, 9, 23), day(2027, 9, 1));
    add_member(&pool, &fau, "medlem@example.test", new_role("Medlem", CapabilityClass::Member), year, t0).await;
    let group = seed_group(&pool, &fau, Visibility::Open).await;
    add_guest(&pool, &fau, "gjest@example.test", group, year, t0).await;
    admin_leaves(&pool, &fau).await;

    recover(&pool, &fau).await;
    assert_eq!(outbox_count(&pool, "recovery.invitation_created", "medlem@example.test").await, 1);
    assert_eq!(outbox_count(&pool, "recovery.invitation_created", "gjest@example.test").await, 0);
}

#[tokio::test]
async fn with_no_members_left_the_fallback_skips_guests_too() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let group = seed_group(&pool, &fau, Visibility::Open).await;
    add_guest(&pool, &fau, "gjest@example.test", group, period(day(2026, 9, 23), day(2027, 9, 1)), t0).await;
    admin_leaves(&pool, &fau).await;

    recover(&pool, &fau).await;
    // The guest is no FAU-wide member, so nobody current remains: the 24-month fallback
    // reaches the admin who left, and still not the guest.
    assert_eq!(outbox_count(&pool, "recovery.invitation_created", "admin@example.test").await, 1);
    assert_eq!(outbox_count(&pool, "recovery.invitation_created", "gjest@example.test").await, 0);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p fau-app --test read_paths`
Expected:
- `a_guest_is_a_guest…` passes already, because `require_admin` refuses a guest too. It pins the behaviour across the rewrite.
- `guests_are_not_told_about_a_recovery` FAILS with `left: 1, right: 0` on the guest's count.
- `with_no_members_left…` FAILS: the guest is told, and the admin is not, since the guest counted as a current member.

- [ ] **Step 3: Implement**

In `backend/crates/persistence/src/membership/authz.rs`, add after `read_transaction`:

```rust
/// A refusal as the error a transaction returns. `hidden` is the answer for a resource the
/// viewer may not know exists (`UnknownGroup` for a group); `NotAuthorized` covers a
/// viewer who may see the resource but not act on it, or who has no standing at all.
pub(crate) fn denied(d: Denied, hidden: MembershipError) -> MembershipError {
    match d {
        Denied::Hidden => hidden,
        Denied::Forbidden | Denied::NoAccess => MembershipError::NotAuthorized,
    }
}
```

In `backend/crates/persistence/src/membership/requests.rs`:
- add `use fau_domain::authz::Action;` to the `fau_domain` imports;
- add `use super::authz::{authorize, denied, read_transaction, Resource, Viewer};` to the `super::` imports;
- replace the doc comment and the first line of the body of `access_request_message`, which is `let mut tx = pool.begin().await?;` followed by `require_admin(&mut tx, tenant_id, actor_membership_id, at.today()).await?;`, with:

```rust
/// The message on a request, for the approval screen (flow spec §5.2). Authority first,
/// through `authorize` as the FAU-level `Manage` right (the #3418 read-path audit, #3501),
/// in one read snapshot: a non-admin learns nothing about whether the request exists.
pub async fn access_request_message(
    pool: &PgPool,
    tenant_id: Uuid,
    request_id: Uuid,
    actor_membership_id: Uuid,
    at: Moment,
) -> Result<Option<fau_crypto::MessageCiphertext>, MembershipError> {
    let mut tx = read_transaction(pool).await?;
    let viewer = Viewer {
        tenant_id,
        membership_id: actor_membership_id,
    };
    authorize(&mut tx, viewer, Resource::Fau, Action::Manage, at)
        .await?
        .map_err(|d| denied(d, MembershipError::NotAuthorized))?;
```

Leave the rest of the function body (the `select encrypted_message …`, the commit and the decode) unchanged. `require_admin` stays imported, since other functions in the file use it.

In `backend/crates/persistence/src/membership/sql.rs`:
- replace the doc comment on `current_member_emails` with:

```rust
/// Addresses of every current member: a usable membership with a member- or admin-class
/// role valid today. A guest is not an FAU-wide member (groups design §3.2), so a guest
/// role alone does not count (#3501's read-path audit, Ruling R16).
```

- in `member_emails`, add this line directly after `            and ($3 = false or r.capability_class = 'admin')`:

```sql
            and r.capability_class <> 'guest'
```

- in `recovery_notice_recipients`, replace the fallback query's SQL string with:

```rust
            "select distinct a.email
               from role_assignments ra
               join roles r       on r.tenant_id = ra.tenant_id and r.id = ra.role_id
               join memberships m on m.tenant_id = ra.tenant_id and m.id = ra.membership_id
               join accounts a    on a.id = m.account_id
              where ra.tenant_id = $1 and a.disabled_at is null
                and r.capability_class <> 'guest'
                and ra.starts_on <= $2::date and ra.ends_on_exclusive > $3::date
              order by a.email",
```

- and change its doc comment's first sentence to: `/// Recovery notices (spec 6.4.4, ADR-003 decision 10): every current member; when none remain, everyone who held a member or admin role in the past 24 months -- never a guest (Ruling R16).`

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p fau-app --test read_paths --test requests --test handover_recovery --test key_chain`
Expected: all pass. `requests`' `a_message_is_stored_as_given_and_read_back_only_by_an_admin` still gets `NotAuthorized` for a member, and `handover_recovery`'s recovery tests still find their recipients.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add backend/crates/persistence/src/membership/authz.rs backend/crates/persistence/src/membership/requests.rs \
  backend/crates/persistence/src/membership/sql.rs backend/crates/app/tests/read_paths.rs
git commit -m "Route the #3418 read paths through authorize and keep guests off FAU-wide notices (#3501)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Change notifications and the per-FAU stream hub; revocation closes live streams (acceptance)

**Files:**
- Modify: `backend/crates/persistence/Cargo.toml`: `tokio = { workspace = true }` becomes `tokio = { workspace = true, features = ["sync"] }`, with the comment `# mpsc for the change stream's subscriptions (membership/events.rs).` above it
- Create: `backend/crates/persistence/src/membership/events.rs`
- Modify: `backend/crates/persistence/src/membership/mod.rs` (`mod events;`, re-exports)
- Modify: `backend/crates/persistence/src/membership/roles.rs` (`revoke_role_assignment`, `revoke_membership`: notify)
- Test: `backend/crates/app/tests/events.rs`

**Interfaces:**
- Consumes: `authorize`, `Resource`, `Viewer`, `membership_access` (Task 4), and the fixtures `world`, `live_assignment`, `revoke` (Task 4).
- Produces, all re-exported from `fau_persistence::membership`:
  - `pub const EVENTS_CHANNEL: &str = "fau_events"` and `pub const SUBSCRIPTION_BUFFER: usize = 64`;
  - `pub enum Change { Changed(Resource), AccessRevoked { membership_id: Uuid } }`, with `pub fn payload(self, tenant_id: Uuid) -> String` and `pub fn parse(&str) -> Option<(Uuid, Change)>`;
  - `pub type HubClock = Arc<dyn Fn() -> Timestamp + Send + Sync>`;
  - `#[derive(Clone)] pub struct Hub`, with:
    - `pub async fn start(pool: PgPool, clock: HubClock) -> Result<Hub, MembershipError>`;
    - `pub async fn subscribe(&self, viewer: Viewer) -> Result<Subscription, MembershipError>`, which refuses with `NotAuthorized` when the viewer has no standing;
    - `pub fn subscriber_count(&self) -> usize`;
  - `pub struct Subscription`, with `pub async fn recv(&mut self) -> Option<Resource>`, which returns `None` once the stream is closed.
- Crate-internal: `pub(crate) async fn notify(conn: &mut PgConnection, tenant_id: Uuid, change: Change) -> Result<(), MembershipError>`. Tasks 7 and later call it inside their transactions.

- [ ] **Step 1: Write the failing tests**

Create `backend/crates/app/tests/events.rs`:

```rust
//! The per-FAU change stream (groups design §3.3 and §10, "SSE"): no delivery reaches a
//! viewer who may not read the resource, a guest beside a closed group included, and
//! revocation closes a live stream. `Subscription::recv` returning `None` is what ends an
//! SSE body (Ruling R1).

mod common;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::groups::*;
use common::membership::*;
use common::TestDb;
use fau_persistence::membership::{
    revoke_membership, Change, Hub, HubClock, MembershipError, Resource, RevokeMembership,
    Subscription, EVENTS_CHANNEL,
};
use jiff::Timestamp;
use sqlx::PgPool;
use uuid::Uuid;

/// A hub on its own pool, with a clock the test can move, starting at T0.
async fn hub(db: &TestDb) -> (Hub, Arc<Mutex<Timestamp>>) {
    let now = Arc::new(Mutex::new(T0.parse::<Timestamp>().unwrap()));
    let read = now.clone();
    let clock: HubClock = Arc::new(move || *read.lock().unwrap());
    (Hub::start(db.app_pool().await, clock).await.expect("start the hub"), now)
}

async fn next(sub: &mut Subscription) -> Option<Resource> {
    tokio::time::timeout(Duration::from_secs(5), sub.recv())
        .await
        .expect("a delivery or a close within five seconds")
}

async fn announce(pool: &PgPool, tenant: Uuid, change: Change) {
    sqlx::query("select pg_notify($1, $2)")
        .bind(EVENTS_CHANNEL)
        .bind(change.payload(tenant))
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_change_reaches_only_the_viewers_who_may_read_it() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let (hub, _) = hub(&db).await;
    let mut admin = hub.subscribe(w.viewer("admin_out")).await.unwrap();
    let mut member = hub.subscribe(w.viewer("member_out")).await.unwrap();
    let mut guest_in = hub.subscribe(w.viewer("guest_in")).await.unwrap();
    let mut guest_out = hub.subscribe(w.viewer("guest_out")).await.unwrap();
    let t = w.fau.tenant_id;
    let changed = |g| Change::Changed(Resource::Group(g));

    for g in [w.closed, w.open, w.other] {
        announce(&pool, t, changed(g)).await;
    }
    for g in [w.closed, w.open, w.other] {
        assert_eq!(next(&mut admin).await, Some(Resource::Group(g)), "admins hear everything");
    }
    assert_eq!(next(&mut member).await, Some(Resource::Group(w.open)));
    assert_eq!(next(&mut guest_in).await, Some(Resource::Group(w.closed)));
    assert_eq!(next(&mut guest_in).await, Some(Resource::Group(w.open)));
    assert_eq!(next(&mut guest_out).await, Some(Resource::Group(w.other)));

    // Nothing unauthorized is queued behind those. The next change each viewer may read is
    // the next thing it hears.
    announce(&pool, t, changed(w.open)).await;
    assert_eq!(next(&mut member).await, Some(Resource::Group(w.open)), "not `other`");
    assert_eq!(next(&mut guest_in).await, Some(Resource::Group(w.open)), "not `other`");
    announce(&pool, t, changed(w.other)).await;
    assert_eq!(
        next(&mut guest_out).await,
        Some(Resource::Group(w.other)),
        "a guest beside two closed groups heard nothing about them, or about the open one"
    );
}

#[tokio::test]
async fn revoking_a_role_closes_the_stream() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let (hub, _) = hub(&db).await;
    let mut member = hub.subscribe(w.viewer("member_in")).await.unwrap();
    let mut bystander = hub.subscribe(w.viewer("member_out")).await.unwrap();
    assert_eq!(hub.subscriber_count(), 2);

    revoke(&pool, &w.fau, live_assignment(&pool, w.membership("member_in")).await, at(T0)).await;
    assert_eq!(next(&mut member).await, None, "the revoked member's stream closes");

    announce(&pool, w.fau.tenant_id, Change::Changed(Resource::Group(w.open))).await;
    assert_eq!(next(&mut bystander).await, Some(Resource::Group(w.open)), "other streams stay open");
    assert_eq!(hub.subscriber_count(), 1);
    assert_eq!(
        hub.subscribe(w.viewer("member_in")).await.unwrap_err(),
        MembershipError::NotAuthorized,
        "without a valid role a new stream is refused"
    );
}

#[tokio::test]
async fn revoking_a_membership_closes_the_stream() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let (hub, _) = hub(&db).await;
    let mut leaver = hub.subscribe(w.viewer("guest_in")).await.unwrap();
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: w.fau.tenant_id,
            actor_membership_id: w.membership("guest_in"),
            membership_id: w.membership("guest_in"),
            confirm_no_admin: false,
        },
        at(T0),
    )
    .await
    .unwrap();
    assert_eq!(next(&mut leaver).await, None);
}

#[tokio::test]
async fn a_stream_whose_role_has_ended_closes_at_its_next_delivery() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let (hub, now) = hub(&db).await;
    // member_out's role ends on 1 September 2027; the registrant's admin role on 1 October.
    let mut member = hub.subscribe(w.viewer("member_out")).await.unwrap();
    let mut admin = hub.subscribe(w.viewer("admin_in")).await.unwrap();
    *now.lock().unwrap() = "2027-09-01T10:00:00Z".parse().unwrap();

    announce(&pool, w.fau.tenant_id, Change::Changed(Resource::Group(w.open))).await;
    assert_eq!(next(&mut member).await, None, "no standing any more: closed, not merely filtered");
    assert_eq!(next(&mut admin).await, Some(Resource::Group(w.open)));
}

#[tokio::test]
async fn a_guest_may_subscribe_and_a_person_without_standing_may_not() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let (hub, _) = hub(&db).await;
    let _guest = hub.subscribe(w.viewer("guest_out")).await.expect("a guest has standing");
    for gone in ["none_in", "none_out"] {
        assert_eq!(
            hub.subscribe(w.viewer(gone)).await.unwrap_err(),
            MembershipError::NotAuthorized,
            "{gone}"
        );
    }
    assert_eq!(hub.subscriber_count(), 1, "a refused subscription leaves nothing behind");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p fau-app --test events`
Expected: does not compile, because `Change`, `Hub`, `HubClock`, `Subscription` and `EVENTS_CHANNEL` are not in `fau_persistence::membership`.

- [ ] **Step 3: Write the hub**

Change the `tokio` line in `backend/crates/persistence/Cargo.toml` as listed under Files.

Create `backend/crates/persistence/src/membership/events.rs`:

```rust
//! Change notifications and the per-FAU change stream (groups design §3.3 and §5.4;
//! ADR-003 decision 5a).
//!
//! - **Writing.** A mutation calls [`notify`] inside its own transaction. PostgreSQL
//!   delivers a NOTIFY only when the transaction commits, so a rolled-back change announces
//!   nothing.
//! - **Payload.** Ids and a kind code only ([`Change::payload`]): never a name, never content.
//! - **Listening.** Each process runs one [`Hub`]. It holds the process's only LISTEN
//!   connection (planning decision of 27 September 2026: one per pod, never one per viewer)
//!   and fans each notification out to that FAU's [`Subscription`]s.
//! - **Filtering.** Every delivery is authorized for its viewer through [`authorize`] at
//!   the moment of delivery, reading the database. A viewer never learns that something
//!   they cannot read has changed, so a guest beside a closed group hears nothing about it.
//! - **Closing** (Rulings R2, R3). Every closed subscription ends with
//!   [`Subscription::recv`] returning `None`, which ends the SSE body. The client
//!   reconnects and is authorized afresh. A subscription closes when:
//!   - [`Change::AccessRevoked`] arrives for its membership;
//!   - its viewer has lost all standing (a role reached its end date), at its next delivery;
//!   - it falls more than [`SUBSCRIPTION_BUFFER`] deliveries behind;
//!   - the listener reconnects, which closes every subscription, since the gap cannot be
//!     replayed.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use fau_domain::authz::{Action, Decision, Denied};
use fau_domain::membership::access::Capability;
use fau_domain::time::Moment;
use jiff::Timestamp;
use serde_json::{json, Value};
use sqlx::postgres::PgListener;
use sqlx::{PgConnection, PgPool};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;

use super::access::membership_access;
use super::authz::{authorize, Resource, Viewer};
use super::error::MembershipError;
use crate::pool::safe_error_kind;

/// The one channel every change is announced on.
pub const EVENTS_CHANNEL: &str = "fau_events";

/// How many deliveries a subscription may fall behind before it is closed.
pub const SUBSCRIPTION_BUFFER: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Something about the resource changed. Viewers who may read it are told its id, and
    /// fetch it themselves inside their session.
    Changed(Resource),
    /// A membership's access shrank: every live subscription it holds closes.
    AccessRevoked { membership_id: Uuid },
}

impl Change {
    /// The NOTIFY payload: the tenant, a kind code and ids.
    pub fn payload(self, tenant_id: Uuid) -> String {
        let v = match self {
            Change::Changed(Resource::Fau) => json!({ "t": tenant_id, "k": "fau" }),
            Change::Changed(Resource::Group(id)) => json!({ "t": tenant_id, "k": "group", "id": id }),
            Change::Changed(Resource::GroupContent(id)) => {
                json!({ "t": tenant_id, "k": "group_content", "id": id })
            }
            Change::AccessRevoked { membership_id } => {
                json!({ "t": tenant_id, "k": "access", "m": membership_id })
            }
        };
        v.to_string()
    }

    /// The inverse of [`Change::payload`]. `None` for anything else.
    pub fn parse(payload: &str) -> Option<(Uuid, Change)> {
        let v: Value = serde_json::from_str(payload).ok()?;
        let uuid = |key: &str| -> Option<Uuid> { v.get(key)?.as_str()?.parse().ok() };
        let tenant = uuid("t")?;
        let change = match v.get("k")?.as_str()? {
            "fau" => Change::Changed(Resource::Fau),
            "group" => Change::Changed(Resource::Group(uuid("id")?)),
            "group_content" => Change::Changed(Resource::GroupContent(uuid("id")?)),
            "access" => Change::AccessRevoked {
                membership_id: uuid("m")?,
            },
            _ => return None,
        };
        Some((tenant, change))
    }
}

/// Announces `change` when the caller's transaction commits.
pub(crate) async fn notify(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    change: Change,
) -> Result<(), MembershipError> {
    sqlx::query("select pg_notify($1, $2)")
        .bind(EVENTS_CHANNEL)
        .bind(change.payload(tenant_id))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// The hub's notion of now. Production passes `Arc::new(jiff::Timestamp::now)`; tests pass
/// a clock they can move.
pub type HubClock = Arc<dyn Fn() -> Timestamp + Send + Sync>;

struct Subscriber {
    id: u64,
    membership_id: Uuid,
    sender: mpsc::Sender<Resource>,
}

struct HubInner {
    pool: PgPool,
    clock: HubClock,
    subscribers: Mutex<HashMap<Uuid, Vec<Subscriber>>>,
    next_id: AtomicU64,
    listener: Mutex<Option<JoinHandle<()>>>,
}

/// One per process: the LISTEN connection and every live subscription.
#[derive(Clone)]
pub struct Hub {
    inner: Arc<HubInner>,
}

impl Hub {
    /// Connects the LISTEN connection (taken from `pool` for the hub's lifetime) and starts
    /// the listener task, which ends when the last clone of the hub is dropped.
    pub async fn start(pool: PgPool, clock: HubClock) -> Result<Self, MembershipError> {
        let mut listener = PgListener::connect_with(&pool).await?;
        listener.listen(EVENTS_CHANNEL).await?;
        let inner = Arc::new(HubInner {
            pool,
            clock,
            subscribers: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(0),
            listener: Mutex::new(None),
        });
        let task = tokio::spawn(listen(listener, Arc::downgrade(&inner)));
        *inner.listener.lock().expect("hub listener lock") = Some(task);
        Ok(Self { inner })
    }

    /// Opens a stream for `viewer`. Refused with `NotAuthorized` unless the viewer has
    /// standing in the FAU today; a guest has standing.
    pub async fn subscribe(&self, viewer: Viewer) -> Result<Subscription, MembershipError> {
        let (sender, receiver) = mpsc::channel(SUBSCRIPTION_BUFFER);
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        // Registered before the standing check. A revocation that commits after the check
        // is announced after registration and closes this subscription; one that committed
        // before it is seen by the check.
        self.inner
            .subscribers()
            .entry(viewer.tenant_id)
            .or_default()
            .push(Subscriber {
                id,
                membership_id: viewer.membership_id,
                sender,
            });
        let subscription = Subscription {
            receiver,
            tenant_id: viewer.tenant_id,
            id,
            hub: Arc::downgrade(&self.inner),
        };
        let mut conn = self.inner.pool.acquire().await?;
        let access = membership_access(
            &mut conn,
            viewer.tenant_id,
            viewer.membership_id,
            self.inner.now().today(),
        )
        .await?;
        if access.capability == Capability::None {
            // Dropping `subscription` unregisters it.
            return Err(MembershipError::NotAuthorized);
        }
        Ok(subscription)
    }

    /// Live subscriptions across every FAU, for tests and metrics.
    pub fn subscriber_count(&self) -> usize {
        self.inner.subscribers().values().map(Vec::len).sum()
    }
}

/// One viewer's stream. Dropping it unregisters it.
pub struct Subscription {
    receiver: mpsc::Receiver<Resource>,
    tenant_id: Uuid,
    id: u64,
    hub: Weak<HubInner>,
}

impl Subscription {
    /// The next changed resource the viewer may read, or `None` once the stream is closed.
    pub async fn recv(&mut self) -> Option<Resource> {
        self.receiver.recv().await
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(hub) = self.hub.upgrade() {
            hub.remove(self.tenant_id, self.id);
        }
    }
}

impl fmt::Debug for Subscription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Subscription")
            .field("tenant_id", &self.tenant_id)
            .field("id", &self.id)
            .finish()
    }
}

impl Drop for HubInner {
    fn drop(&mut self) {
        if let Ok(slot) = self.listener.get_mut() {
            if let Some(task) = slot.take() {
                task.abort();
            }
        }
    }
}

impl HubInner {
    fn subscribers(&self) -> MutexGuard<'_, HashMap<Uuid, Vec<Subscriber>>> {
        self.subscribers.lock().expect("hub subscribers lock")
    }

    fn now(&self) -> Moment {
        Moment::at((self.clock)())
    }

    /// Keeps only the tenant's subscribers for which `keep` holds. Dropping a subscriber
    /// drops its sender, which closes its stream.
    fn retain(&self, tenant_id: Uuid, keep: impl Fn(&Subscriber) -> bool) {
        let mut map = self.subscribers();
        let empty = match map.get_mut(&tenant_id) {
            Some(subs) => {
                subs.retain(&keep);
                subs.is_empty()
            }
            None => false,
        };
        if empty {
            map.remove(&tenant_id);
        }
    }

    fn remove(&self, tenant_id: Uuid, id: u64) {
        self.retain(tenant_id, |s| s.id != id);
    }

    fn close_all(&self) {
        self.subscribers().clear();
    }

    async fn dispatch(&self, tenant_id: Uuid, change: Change) {
        match change {
            Change::AccessRevoked { membership_id } => {
                self.retain(tenant_id, |s| s.membership_id != membership_id);
            }
            Change::Changed(resource) => {
                let targets: Vec<(u64, Uuid, mpsc::Sender<Resource>)> = self
                    .subscribers()
                    .get(&tenant_id)
                    .map(|subs| {
                        subs.iter()
                            .map(|s| (s.id, s.membership_id, s.sender.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                for (id, membership_id, sender) in targets {
                    let viewer = Viewer {
                        tenant_id,
                        membership_id,
                    };
                    match self.decide(viewer, resource).await {
                        Ok(Ok(())) => {
                            if sender.try_send(resource).is_err() {
                                // Full (too far behind) or already dropped.
                                self.remove(tenant_id, id);
                            }
                        }
                        Ok(Err(Denied::NoAccess)) => self.remove(tenant_id, id),
                        Ok(Err(Denied::Hidden | Denied::Forbidden)) => {}
                        Err(e) => {
                            tracing::warn!(error = %e, "could not authorize a change for a stream; closing it");
                            self.remove(tenant_id, id);
                        }
                    }
                }
            }
        }
    }

    async fn decide(&self, viewer: Viewer, resource: Resource) -> Result<Decision, MembershipError> {
        let mut conn = self.pool.acquire().await?;
        authorize(&mut conn, viewer, resource, Action::Read, self.now()).await
    }
}

async fn listen(mut listener: PgListener, weak: Weak<HubInner>) {
    loop {
        let next = listener.try_recv().await;
        let Some(hub) = weak.upgrade() else {
            return;
        };
        match next {
            Ok(Some(notification)) => match Change::parse(notification.payload()) {
                Some((tenant_id, change)) => hub.dispatch(tenant_id, change).await,
                None => tracing::warn!("ignored a malformed change notification"),
            },
            Ok(None) => {
                // The connection dropped and was re-established; anything sent meanwhile
                // is lost, so every stream closes and its client refetches.
                tracing::warn!("change listener reconnected; closing every stream");
                hub.close_all();
            }
            Err(e) => {
                tracing::warn!(kind = %safe_error_kind(&e), "change listener failed; closing every stream");
                hub.close_all();
                drop(hub);
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_payload_carries_ids_and_a_kind_only() {
        let (t, g, m) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let group: Value =
            serde_json::from_str(&Change::Changed(Resource::Group(g)).payload(t)).unwrap();
        assert_eq!(group, json!({ "t": t, "k": "group", "id": g }));
        let access: Value =
            serde_json::from_str(&Change::AccessRevoked { membership_id: m }.payload(t)).unwrap();
        assert_eq!(access, json!({ "t": t, "k": "access", "m": m }));
    }

    #[test]
    fn every_change_round_trips() {
        let (t, g, m) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        for c in [
            Change::Changed(Resource::Fau),
            Change::Changed(Resource::Group(g)),
            Change::Changed(Resource::GroupContent(g)),
            Change::AccessRevoked { membership_id: m },
        ] {
            assert_eq!(Change::parse(&c.payload(t)), Some((t, c)));
        }
    }

    #[test]
    fn a_malformed_payload_is_ignored() {
        let unknown_kind = format!(r#"{{"t":"{}","k":"poll"}}"#, Uuid::now_v7());
        for bad in [
            "",
            "{}",
            "not json",
            r#"{"t":"x","k":"group","id":"y"}"#,
            unknown_kind.as_str(),
        ] {
            assert_eq!(Change::parse(bad), None, "{bad}");
        }
    }
}
```

In `backend/crates/persistence/src/membership/mod.rs`, add `mod events;` after `mod error;`, and add after the `authz` re-export:

```rust
pub use events::{Change, Hub, HubClock, Subscription, EVENTS_CHANNEL, SUBSCRIPTION_BUFFER};
```

- [ ] **Step 4: Announce revocations**

In `backend/crates/persistence/src/membership/roles.rs`, add `use super::events::{notify, Change};` to the imports.

In `revoke_role_assignment`, directly before its `write_audit(` call, which follows the `update handover_grants set revoked_at …` statement, insert:

```rust
    notify(
        &mut tx,
        req.tenant_id,
        Change::AccessRevoked {
            membership_id: holder,
        },
    )
    .await?;
```

In `revoke_membership`, directly after the `withdraw_pending_requests(…).await?;` call and before its `write_audit(`, insert:

```rust
    notify(
        &mut tx,
        req.tenant_id,
        Change::AccessRevoked {
            membership_id: req.membership_id,
        },
    )
    .await?;
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p fau-persistence events && cargo test -p fau-app --test events --test roles --test authorization`
Expected: the 3 unit tests pass, and `events` passes 5 tests, each within the five-second timeout. `roles` and `authorization` still pass, since the notifications have no listener there and cost nothing.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add backend/crates/persistence/Cargo.toml backend/Cargo.lock backend/crates/persistence/src/membership/events.rs \
  backend/crates/persistence/src/membership/mod.rs backend/crates/persistence/src/membership/roles.rs \
  backend/crates/app/tests/events.rs
git commit -m "Add the per-FAU change stream: one listener, per-viewer filtering, revocation closes (#3501)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

(`Cargo.lock` changes only if enabling tokio's `sync` feature changes the lock file. `git add` of an unchanged file is harmless.)

---

### Task 7: Group management: create, rename, close/open, archive, add and remove members

**Files:**
- Create: `backend/crates/persistence/src/membership/groups.rs`
- Modify: `backend/crates/persistence/src/membership/error.rs` (group variants)
- Modify: `backend/crates/persistence/src/membership/mod.rs` (`mod groups;`, re-exports)
- Modify: `backend/crates/persistence/src/membership/roles.rs` (`revoke_membership` leaves every group)
- Test: `backend/crates/app/tests/groups.rs`

**Interfaces:**
- Consumes:
  - `authorize`, `denied`, `Resource`, `Viewer` (Tasks 4 and 5), and `notify`, `Change`, `Hub` (Task 6);
  - `lock_tenant`, `require_open`, `membership_and_account_state`, `write_audit`, `Audit`, `ts_param` from `membership/sql.rs` (existing).
- Produces, all re-exported from `fau_persistence::membership`:
  - `pub const GROUP_NAME_AAD: (&str, &str) = ("groups", "encrypted_name")` and `pub const GROUP_NAME_CIPHERTEXT_BYTES: RangeInclusive<usize> = 42..=512`;
  - `pub enum GroupBinding { Unit(Uuid), Cohort(Uuid) }` (`Debug, Clone, Copy, PartialEq, Eq`);
  - request structs, every one with `tenant_id: Uuid` and `actor_membership_id: Uuid`:
    - `CreateGroup { group_id: Uuid, encrypted_name: Ciphertext, visibility: Visibility, binding: Option<GroupBinding> }`;
    - `RenameGroup { group_id: Uuid, encrypted_name: Ciphertext }`;
    - `SetGroupVisibility { group_id: Uuid, visibility: Visibility }`;
    - `ArchiveGroup { group_id: Uuid }`;
    - `GroupMemberChange { group_id: Uuid, membership_id: Uuid }`;
  - transactions, each taking `(pool: &PgPool, req, at: Moment)`:
    - `create_group(…) -> Result<Uuid, MembershipError>`;
    - `rename_group`, `set_group_visibility`, `archive_group`, `add_group_member` and `remove_group_member`, each returning `Result<(), MembershipError>`;
  - new `MembershipError` variants: `UnknownGroup`, `GroupArchived`, `AlreadyInGroup`, `NotInGroup`, `GroupNameMalformed`, `UnknownUnit` and `UnknownCohort`.
- Crate-internal: `pub(crate) async fn remove_from_all_groups(conn: &mut PgConnection, tenant_id: Uuid, actor_membership_id: Uuid, membership_id: Uuid, at: Moment) -> Result<(), MembershipError>`.

Audit actions (`subject_type` is `group`):
- `group.created`, with params `visibility`, `unit_id` and `cohort_id`;
- `group.renamed`, `group.closed`, `group.opened` and `group.archived`, with empty params;
- `group.member_added` and `group.member_removed`, with param `membership_id`, plus `cause: "membership_revoked"` when the removal comes from a revoked membership.

A group name never appears in any of them.

- [ ] **Step 1: Write the failing tests**

Create `backend/crates/app/tests/groups.rs`:

```rust
//! Group management (groups design §3.1 and §3.3): admin-only, audited, announced on the
//! change stream. Names arrive encrypted; these tests use an envelope-shaped placeholder.

mod common;
use std::sync::Arc;
use std::time::Duration;

use common::groups::*;
use common::membership::*;
use common::TestDb;
use fau_crypto::Ciphertext;
use fau_domain::authz::{Action, Denied};
use fau_domain::membership::vocabulary::{CapabilityClass, Visibility};
use fau_persistence::membership::{
    add_group_member, archive_group, authorize, create_group, remove_group_member, rename_group,
    revoke_membership, set_group_visibility, ArchiveGroup, CreateGroup, GroupBinding,
    GroupMemberChange, Hub, MembershipError, RenameGroup, Resource, RevokeMembership,
    SetGroupVisibility, Subscription,
};
use sqlx::PgPool;
use uuid::Uuid;

fn name() -> Ciphertext {
    Ciphertext::from_stored(placeholder_name())
}

fn create(tenant_id: Uuid, actor: Uuid, visibility: Visibility, binding: Option<GroupBinding>) -> CreateGroup {
    CreateGroup {
        tenant_id,
        actor_membership_id: actor,
        group_id: Uuid::now_v7(),
        encrypted_name: name(),
        visibility,
        binding,
    }
}

fn change(fau: &Fau, actor: Uuid, group_id: Uuid, membership_id: Uuid) -> GroupMemberChange {
    GroupMemberChange {
        tenant_id: fau.tenant_id,
        actor_membership_id: actor,
        group_id,
        membership_id,
    }
}

fn visibility(fau: &Fau, group_id: Uuid, visibility: Visibility) -> SetGroupVisibility {
    SetGroupVisibility {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        group_id,
        visibility,
    }
}

fn rename(fau: &Fau, group_id: Uuid) -> RenameGroup {
    RenameGroup {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        group_id,
        encrypted_name: name(),
    }
}

fn archive(fau: &Fau, group_id: Uuid) -> ArchiveGroup {
    ArchiveGroup {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        group_id,
    }
}

async fn params(pool: &PgPool, action: &str) -> Vec<serde_json::Value> {
    let rows: Vec<String> = sqlx::query_scalar(
        "select params::text from audit_events where action = $1 order by occurred_at, id",
    )
    .bind(action)
    .fetch_all(pool)
    .await
    .unwrap();
    rows.iter().map(|r| serde_json::from_str(r).unwrap()).collect()
}

async fn member(pool: &PgPool, fau: &Fau, address: &str) -> Uuid {
    add_member(
        pool,
        fau,
        address,
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 1), day(2027, 9, 1)),
        at(T0),
    )
    .await
    .membership_id
}

#[tokio::test]
async fn an_admin_creates_a_group_with_its_ciphertext_and_an_audit_entry() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    let req = create(fau.tenant_id, fau.admin_membership_id, Visibility::Closed, None);
    let id = create_group(&pool, req.clone(), at(T0)).await.unwrap();
    assert_eq!(id, req.group_id, "the id the name's AAD is bound to");

    let (stored, vis): (Vec<u8>, String) =
        sqlx::query_as("select encrypted_name, visibility from groups where id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored, placeholder_name());
    assert_eq!(vis, "closed");
    assert_eq!(
        params(&pool, "group.created").await,
        [serde_json::json!({ "visibility": "closed", "unit_id": null, "cohort_id": null })],
        "ids and codes only, never the name"
    );
}

#[tokio::test]
async fn only_an_admin_manages_groups_and_a_hidden_group_stays_hidden() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let t0 = at(T0);
    for who in ["member_in", "guest_in", "none_in"] {
        assert_eq!(
            create_group(&pool, create(w.fau.tenant_id, w.membership(who), Visibility::Open, None), t0)
                .await
                .unwrap_err(),
            MembershipError::NotAuthorized,
            "{who}"
        );
    }
    let someone = w.membership("member_out");
    // A member who can see the open group may not manage it...
    assert_eq!(
        add_group_member(&pool, change(&w.fau, w.membership("member_in"), w.open, someone), t0)
            .await
            .unwrap_err(),
        MembershipError::NotAuthorized
    );
    // ...and one who cannot see the closed group learns nothing about it, exactly as for an
    // id that does not exist.
    for group in [w.closed, Uuid::now_v7()] {
        assert_eq!(
            add_group_member(&pool, change(&w.fau, w.membership("member_out"), group, someone), t0)
                .await
                .unwrap_err(),
            MembershipError::UnknownGroup
        );
    }
    assert_eq!(
        add_group_member(&pool, change(&w.fau, w.fau.admin_membership_id, Uuid::now_v7(), someone), t0)
            .await
            .unwrap_err(),
        MembershipError::UnknownGroup,
        "an admin naming an unknown group"
    );
}

#[tokio::test]
async fn a_group_binds_only_to_a_unit_or_cohort_of_its_own_fau() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let other = active_fau(&pool, "annen@example.test", t0).await;
    let unit = seed_unit(&pool, fau.tenant_id).await;
    let foreign_cohort = seed_cohort(&pool, other.tenant_id).await;
    let admin = fau.admin_membership_id;

    let bound = create_group(&pool, create(fau.tenant_id, admin, Visibility::Open, Some(GroupBinding::Unit(unit))), t0)
        .await
        .unwrap();
    let stored: Option<Uuid> = sqlx::query_scalar("select unit_id from groups where id = $1")
        .bind(bound)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, Some(unit));
    assert_eq!(
        create_group(&pool, create(fau.tenant_id, admin, Visibility::Open, Some(GroupBinding::Unit(Uuid::now_v7()))), t0)
            .await
            .unwrap_err(),
        MembershipError::UnknownUnit
    );
    assert_eq!(
        create_group(&pool, create(fau.tenant_id, admin, Visibility::Open, Some(GroupBinding::Cohort(foreign_cohort))), t0)
            .await
            .unwrap_err(),
        MembershipError::UnknownCohort
    );
}

#[tokio::test]
async fn a_malformed_name_is_refused_before_the_database() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    for bad in [vec![1u8; 10], vec![2u8; 60], vec![1u8; 513]] {
        let mut req = create(fau.tenant_id, fau.admin_membership_id, Visibility::Open, None);
        req.encrypted_name = Ciphertext::from_stored(bad);
        assert_eq!(
            create_group(&pool, req, at(T0)).await.unwrap_err(),
            MembershipError::GroupNameMalformed
        );
    }
}

#[tokio::test]
async fn members_are_added_once_and_removed_softly() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let admin = fau.admin_membership_id;
    let group = seed_group(&pool, &fau, Visibility::Closed).await;
    let m = member(&pool, &fau, "m@example.test").await;

    add_group_member(&pool, change(&fau, admin, group, m), t0).await.unwrap();
    assert_eq!(
        add_group_member(&pool, change(&fau, admin, group, m), t0).await.unwrap_err(),
        MembershipError::AlreadyInGroup
    );
    remove_group_member(&pool, change(&fau, admin, group, m), t0).await.unwrap();
    assert_eq!(
        remove_group_member(&pool, change(&fau, admin, group, m), t0).await.unwrap_err(),
        MembershipError::NotInGroup
    );
    add_group_member(&pool, change(&fau, admin, group, m), t0).await.unwrap();
    assert_eq!(
        count(&pool, &format!("select count(*) from group_members where group_id = '{group}'")).await,
        2,
        "history keeps the removed row"
    );
    assert_eq!(audit_count(&pool, "group.member_added").await, 2);
    assert_eq!(
        params(&pool, "group.member_removed").await,
        [serde_json::json!({ "membership_id": m })]
    );

    assert_eq!(
        add_group_member(&pool, change(&fau, admin, group, Uuid::now_v7()), t0).await.unwrap_err(),
        MembershipError::UnknownMembership
    );
    let leaver = member(&pool, &fau, "borte@example.test").await;
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: leaver,
            membership_id: leaver,
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        add_group_member(&pool, change(&fau, admin, group, leaver), t0).await.unwrap_err(),
        MembershipError::MembershipRevoked
    );
}

#[tokio::test]
async fn closing_opening_renaming_and_archiving() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let admin = fau.admin_membership_id;
    let group = seed_group(&pool, &fau, Visibility::Open).await;
    let m = member(&pool, &fau, "m@example.test").await;
    add_group_member(&pool, change(&fau, admin, group, m), t0).await.unwrap();

    set_group_visibility(&pool, visibility(&fau, group, Visibility::Closed), t0).await.unwrap();
    set_group_visibility(&pool, visibility(&fau, group, Visibility::Closed), t0).await.unwrap();
    assert_eq!(audit_count(&pool, "group.closed").await, 1, "a no-op writes nothing");
    set_group_visibility(&pool, visibility(&fau, group, Visibility::Open), t0).await.unwrap();
    assert_eq!(audit_count(&pool, "group.opened").await, 1);

    let mut renamed = rename(&fau, group);
    let mut bytes = placeholder_name();
    bytes[1] = 7;
    renamed.encrypted_name = Ciphertext::from_stored(bytes.clone());
    rename_group(&pool, renamed, t0).await.unwrap();
    let stored: Vec<u8> = sqlx::query_scalar("select encrypted_name from groups where id = $1")
        .bind(group)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, bytes);
    assert_eq!(params(&pool, "group.renamed").await, [serde_json::json!({})]);

    archive_group(&pool, archive(&fau, group), t0).await.unwrap();
    assert_eq!(audit_count(&pool, "group.archived").await, 1);
    let other = member(&pool, &fau, "ny@example.test").await;
    for err in [
        rename_group(&pool, rename(&fau, group), t0).await.unwrap_err(),
        set_group_visibility(&pool, visibility(&fau, group, Visibility::Closed), t0).await.unwrap_err(),
        add_group_member(&pool, change(&fau, admin, group, other), t0).await.unwrap_err(),
        archive_group(&pool, archive(&fau, group), t0).await.unwrap_err(),
    ] {
        assert_eq!(err, MembershipError::GroupArchived);
    }
    remove_group_member(&pool, change(&fau, admin, group, m), t0)
        .await
        .expect("an archived group still loses members");
}

#[tokio::test]
async fn a_frozen_fau_refuses_additions_and_allows_reductions() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let admin = fau.admin_membership_id;
    let group = seed_group(&pool, &fau, Visibility::Open).await;
    let m = member(&pool, &fau, "m@example.test").await;
    let n = member(&pool, &fau, "n@example.test").await;
    add_group_member(&pool, change(&fau, admin, group, m), t0).await.unwrap();
    sqlx::query("update tenants set frozen_at = $1::timestamptz where id = $2")
        .bind(T0)
        .bind(fau.tenant_id)
        .execute(&pool)
        .await
        .unwrap();

    for err in [
        create_group(&pool, create(fau.tenant_id, admin, Visibility::Open, None), t0).await.unwrap_err(),
        rename_group(&pool, rename(&fau, group), t0).await.unwrap_err(),
        add_group_member(&pool, change(&fau, admin, group, n), t0).await.unwrap_err(),
    ] {
        assert_eq!(err, MembershipError::TenantFrozen);
    }
    set_group_visibility(&pool, visibility(&fau, group, Visibility::Closed), t0).await.expect("closing reduces access");
    assert_eq!(
        set_group_visibility(&pool, visibility(&fau, group, Visibility::Open), t0).await.unwrap_err(),
        MembershipError::TenantFrozen
    );
    remove_group_member(&pool, change(&fau, admin, group, m), t0).await.expect("removal reduces access");
    archive_group(&pool, archive(&fau, group), t0).await.expect("archiving reduces access");
}

#[tokio::test]
async fn revoking_a_membership_removes_it_from_every_group_for_good() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let w = world(&pool).await;
    let gone = w.membership("member_in");
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: w.fau.tenant_id,
            actor_membership_id: w.fau.admin_membership_id,
            membership_id: gone,
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        count(&pool, &format!("select count(*) from group_members where membership_id = '{gone}' and removed_at is null")).await,
        0
    );
    let removed = params(&pool, "group.member_removed").await;
    assert_eq!(removed.len(), 2, "open and closed");
    assert!(removed.iter().all(|p| p["cause"] == "membership_revoked"));

    // Re-invited, the same membership row reopens, but the closed group is not handed back.
    let back = add_member(
        &pool,
        &w.fau,
        "member-in@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 1), day(2027, 9, 1)),
        t0,
    )
    .await;
    assert_eq!(back.membership_id, gone);
    let mut conn = pool.acquire().await.unwrap();
    assert_eq!(
        authorize(&mut conn, w.viewer("member_in"), Resource::GroupContent(w.closed), Action::Read, t0)
            .await
            .unwrap(),
        Err(Denied::Hidden)
    );
}

async fn next(sub: &mut Subscription) -> Option<Resource> {
    tokio::time::timeout(Duration::from_secs(5), sub.recv())
        .await
        .expect("a delivery or a close within five seconds")
}

#[tokio::test]
async fn group_changes_reach_the_stream_and_a_removal_closes_it() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let w = world(&pool).await;
    let hub = Hub::start(db.app_pool().await, Arc::new(|| T0.parse::<jiff::Timestamp>().unwrap()))
        .await
        .unwrap();
    let mut admin = hub.subscribe(w.viewer("admin_out")).await.unwrap();
    let mut outsider = hub.subscribe(w.viewer("member_out")).await.unwrap();
    let mut insider = hub.subscribe(w.viewer("member_in")).await.unwrap();

    let secret = create_group(&pool, create(w.fau.tenant_id, w.fau.admin_membership_id, Visibility::Closed, None), t0)
        .await
        .unwrap();
    assert_eq!(next(&mut admin).await, Some(Resource::Group(secret)));

    remove_group_member(&pool, change(&w.fau, w.fau.admin_membership_id, w.open, w.membership("member_in")), t0)
        .await
        .unwrap();
    assert_eq!(next(&mut insider).await, None, "removal from a group closes the stream");

    rename_group(&pool, rename(&w.fau, w.open), t0).await.unwrap();
    assert_eq!(
        next(&mut outsider).await,
        Some(Resource::Group(w.open)),
        "the member outside the new closed group heard only of the open one"
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p fau-app --test groups`
Expected: does not compile, because `create_group`, `CreateGroup` and the other group transactions are not in `fau_persistence::membership`.

- [ ] **Step 3: Add the error variants**

In `backend/crates/persistence/src/membership/error.rs`, add before `    // Infrastructure.`:

```rust
    // Groups (#3501).
    #[error("group not found")]
    UnknownGroup,
    #[error("the group is archived")]
    GroupArchived,
    #[error("the membership is already in the group")]
    AlreadyInGroup,
    #[error("the membership was not added to the group by hand")]
    NotInGroup,
    #[error("the group name is not a well-formed ciphertext")]
    GroupNameMalformed,
    #[error("organization unit not found")]
    UnknownUnit,
    #[error("cohort not found")]
    UnknownCohort,

```

- [ ] **Step 4: Write the group transactions**

Create `backend/crates/persistence/src/membership/groups.rs`:

```rust
//! Groups (groups design §3.1 and §3.3; #3501): arbitrary groups inside an FAU, open by
//! default and closable, optionally bound to a unit or a cohort so that their membership
//! follows roles.
//!
//! Managing groups is admin-only in the MVP. Every check goes through `authorize`, and
//! every change is audited and announced on the change stream in the same transaction.
//!
//! **Names are content.** The caller encrypts a name under the FAU's record key
//! (`fau_crypto::Unit::Record`) with `Aad::new(tenant_id, GROUP_NAME_AAD.0,
//! GROUP_NAME_AAD.1, group_id)` before calling in. This module stores and returns the
//! ciphertext only, and never writes a name to audit, to a NOTIFY or to a log.
//!
//! **Order** (as in the rest of `membership`): tenant state, then authority, then row state.
//! A frozen FAU refuses the additive actions (create, rename, open, add a member) and
//! allows the reducing ones (close, archive, remove a member), the line #3418 drew (Ruling
//! R14). An actor who may not see a group gets `UnknownGroup`, exactly as for an id that
//! does not exist.

use std::ops::RangeInclusive;

use fau_crypto::Ciphertext;
use fau_domain::authz::Action;
use fau_domain::membership::vocabulary::Visibility;
use fau_domain::time::Moment;
use serde_json::{json, Value};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::authz::{authorize, denied, Resource, Viewer};
use super::error::MembershipError;
use super::events::{notify, Change};
use super::sql::{
    lock_tenant, membership_and_account_state, require_open, ts_param, write_audit, Audit,
};

/// The associated data a group name is encrypted with, with the tenant and the group's id.
pub const GROUP_NAME_AAD: (&str, &str) = ("groups", "encrypted_name");

/// fau-crypto's envelope around a 1..=`GroupName::MAX_CHARS` name: 41 bytes of version,
/// nonce and tag, plus 1 to 400 bytes of UTF-8. The same bounds as the database's
/// `groups_name_is_an_envelope`.
pub const GROUP_NAME_CIPHERTEXT_BYTES: RangeInclusive<usize> = 42..=512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupBinding {
    Unit(Uuid),
    Cohort(Uuid),
}

#[derive(Debug, Clone)]
pub struct CreateGroup {
    pub tenant_id: Uuid,
    pub actor_membership_id: Uuid,
    /// Chosen by the caller (UUIDv7) before encrypting, so the name's AAD binds to it.
    pub group_id: Uuid,
    pub encrypted_name: Ciphertext,
    pub visibility: Visibility,
    pub binding: Option<GroupBinding>,
}

#[derive(Debug, Clone)]
pub struct RenameGroup {
    pub tenant_id: Uuid,
    pub actor_membership_id: Uuid,
    pub group_id: Uuid,
    pub encrypted_name: Ciphertext,
}

#[derive(Debug, Clone, Copy)]
pub struct SetGroupVisibility {
    pub tenant_id: Uuid,
    pub actor_membership_id: Uuid,
    pub group_id: Uuid,
    pub visibility: Visibility,
}

#[derive(Debug, Clone, Copy)]
pub struct ArchiveGroup {
    pub tenant_id: Uuid,
    pub actor_membership_id: Uuid,
    pub group_id: Uuid,
}

#[derive(Debug, Clone, Copy)]
pub struct GroupMemberChange {
    pub tenant_id: Uuid,
    pub actor_membership_id: Uuid,
    pub group_id: Uuid,
    pub membership_id: Uuid,
}

fn check_name(name: &Ciphertext) -> Result<(), MembershipError> {
    if GROUP_NAME_CIPHERTEXT_BYTES.contains(&name.len()) && name.as_bytes()[0] == 1 {
        Ok(())
    } else {
        Err(MembershipError::GroupNameMalformed)
    }
}

fn audit(tenant_id: Uuid, actor: Uuid, action: &'static str, group_id: Uuid, params: Value) -> Audit {
    Audit::member(tenant_id, actor, action, "group", group_id, params)
}

/// Authority to manage the group, then its archived flag under a row lock.
async fn manage(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    actor: Uuid,
    group_id: Uuid,
    at: Moment,
) -> Result<bool, MembershipError> {
    let viewer = Viewer {
        tenant_id,
        membership_id: actor,
    };
    authorize(conn, viewer, Resource::Group(group_id), Action::Manage, at)
        .await?
        .map_err(|d| denied(d, MembershipError::UnknownGroup))?;
    Ok(sqlx::query_scalar(
        "select archived_at is not null from groups where tenant_id = $1 and id = $2 for update",
    )
    .bind(tenant_id)
    .bind(group_id)
    .fetch_one(&mut *conn)
    .await?)
}

async fn require_exists(
    conn: &mut PgConnection,
    table: &'static str,
    tenant_id: Uuid,
    id: Uuid,
    missing: MembershipError,
) -> Result<(), MembershipError> {
    let sql = format!("select exists (select 1 from {table} where tenant_id = $1 and id = $2)");
    let found: bool = sqlx::query_scalar(&sql)
        .bind(tenant_id)
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
    if found {
        Ok(())
    } else {
        Err(missing)
    }
}

/// Creates a group (§3.1). Admin only; refused while the FAU is frozen.
pub async fn create_group(pool: &PgPool, req: CreateGroup, at: Moment) -> Result<Uuid, MembershipError> {
    check_name(&req.encrypted_name)?;
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, req.tenant_id).await?;
    require_open(&state)?;
    let viewer = Viewer {
        tenant_id: req.tenant_id,
        membership_id: req.actor_membership_id,
    };
    authorize(&mut tx, viewer, Resource::Fau, Action::Manage, at)
        .await?
        .map_err(|d| denied(d, MembershipError::NotAuthorized))?;
    let (unit_id, cohort_id) = match req.binding {
        None => (None, None),
        Some(GroupBinding::Unit(id)) => {
            require_exists(&mut tx, "organization_units", req.tenant_id, id, MembershipError::UnknownUnit).await?;
            (Some(id), None)
        }
        Some(GroupBinding::Cohort(id)) => {
            require_exists(&mut tx, "cohorts", req.tenant_id, id, MembershipError::UnknownCohort).await?;
            (None, Some(id))
        }
    };
    sqlx::query(
        "insert into groups
           (tenant_id, id, encrypted_name, visibility, unit_id, cohort_id, created_by, created_at)
         values ($1, $2, $3, $4, $5, $6, $7, $8::timestamptz)",
    )
    .bind(req.tenant_id)
    .bind(req.group_id)
    .bind(req.encrypted_name.as_bytes())
    .bind(req.visibility.code())
    .bind(unit_id)
    .bind(cohort_id)
    .bind(req.actor_membership_id)
    .bind(ts_param(at.now()))
    .execute(&mut *tx)
    .await?;
    write_audit(
        &mut tx,
        at,
        audit(
            req.tenant_id,
            req.actor_membership_id,
            "group.created",
            req.group_id,
            json!({ "visibility": req.visibility.code(), "unit_id": unit_id, "cohort_id": cohort_id }),
        ),
    )
    .await?;
    notify(&mut tx, req.tenant_id, Change::Changed(Resource::Group(req.group_id))).await?;
    tx.commit().await?;
    Ok(req.group_id)
}

/// Replaces a group's encrypted name. Admin only; refused while frozen or archived.
pub async fn rename_group(pool: &PgPool, req: RenameGroup, at: Moment) -> Result<(), MembershipError> {
    check_name(&req.encrypted_name)?;
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, req.tenant_id).await?;
    require_open(&state)?;
    if manage(&mut tx, req.tenant_id, req.actor_membership_id, req.group_id, at).await? {
        return Err(MembershipError::GroupArchived);
    }
    sqlx::query("update groups set encrypted_name = $3 where tenant_id = $1 and id = $2")
        .bind(req.tenant_id)
        .bind(req.group_id)
        .bind(req.encrypted_name.as_bytes())
        .execute(&mut *tx)
        .await?;
    write_audit(&mut tx, at, audit(req.tenant_id, req.actor_membership_id, "group.renamed", req.group_id, json!({}))).await?;
    notify(&mut tx, req.tenant_id, Change::Changed(Resource::Group(req.group_id))).await?;
    tx.commit().await?;
    Ok(())
}

/// Closes or opens a group (D11). Closing reduces access, so it is allowed while the FAU is
/// frozen; opening is not. Setting the current visibility again writes nothing.
pub async fn set_group_visibility(pool: &PgPool, req: SetGroupVisibility, at: Moment) -> Result<(), MembershipError> {
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, req.tenant_id).await?;
    if req.visibility == Visibility::Open {
        require_open(&state)?;
    }
    if manage(&mut tx, req.tenant_id, req.actor_membership_id, req.group_id, at).await? {
        return Err(MembershipError::GroupArchived);
    }
    let current: String = sqlx::query_scalar("select visibility from groups where tenant_id = $1 and id = $2")
        .bind(req.tenant_id)
        .bind(req.group_id)
        .fetch_one(&mut *tx)
        .await?;
    if current == req.visibility.code() {
        tx.commit().await?;
        return Ok(());
    }
    sqlx::query("update groups set visibility = $3 where tenant_id = $1 and id = $2")
        .bind(req.tenant_id)
        .bind(req.group_id)
        .bind(req.visibility.code())
        .execute(&mut *tx)
        .await?;
    let action = match req.visibility {
        Visibility::Closed => "group.closed",
        Visibility::Open => "group.opened",
    };
    write_audit(&mut tx, at, audit(req.tenant_id, req.actor_membership_id, action, req.group_id, json!({}))).await?;
    notify(&mut tx, req.tenant_id, Change::Changed(Resource::Group(req.group_id))).await?;
    tx.commit().await?;
    Ok(())
}

/// Archives a group, one way (Ruling R13): it stays readable as history and takes no
/// writes. Allowed while frozen.
pub async fn archive_group(pool: &PgPool, req: ArchiveGroup, at: Moment) -> Result<(), MembershipError> {
    let mut tx = pool.begin().await?;
    lock_tenant(&mut tx, req.tenant_id).await?;
    if manage(&mut tx, req.tenant_id, req.actor_membership_id, req.group_id, at).await? {
        return Err(MembershipError::GroupArchived);
    }
    sqlx::query("update groups set archived_at = $3::timestamptz where tenant_id = $1 and id = $2")
        .bind(req.tenant_id)
        .bind(req.group_id)
        .bind(ts_param(at.now()))
        .execute(&mut *tx)
        .await?;
    write_audit(&mut tx, at, audit(req.tenant_id, req.actor_membership_id, "group.archived", req.group_id, json!({}))).await?;
    notify(&mut tx, req.tenant_id, Change::Changed(Resource::Group(req.group_id))).await?;
    tx.commit().await?;
    Ok(())
}

/// Adds a member by hand: anyone with a usable membership, a guest or a member of a bound
/// group's unit included (§3.1). Admin only; refused while frozen or archived.
pub async fn add_group_member(pool: &PgPool, req: GroupMemberChange, at: Moment) -> Result<(), MembershipError> {
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, req.tenant_id).await?;
    require_open(&state)?;
    if manage(&mut tx, req.tenant_id, req.actor_membership_id, req.group_id, at).await? {
        return Err(MembershipError::GroupArchived);
    }
    match membership_and_account_state(&mut tx, req.tenant_id, req.membership_id).await? {
        None => return Err(MembershipError::UnknownMembership),
        Some((true, _)) => return Err(MembershipError::MembershipRevoked),
        Some((false, false)) => return Err(MembershipError::AccountDisabled),
        Some((false, true)) => {}
    }
    let already: bool = sqlx::query_scalar(
        "select exists (select 1 from group_members
                         where tenant_id = $1 and group_id = $2 and membership_id = $3
                           and removed_at is null)",
    )
    .bind(req.tenant_id)
    .bind(req.group_id)
    .bind(req.membership_id)
    .fetch_one(&mut *tx)
    .await?;
    if already {
        return Err(MembershipError::AlreadyInGroup);
    }
    sqlx::query(
        "insert into group_members (tenant_id, id, group_id, membership_id, added_by, added_at)
         values ($1, $2, $3, $4, $5, $6::timestamptz)",
    )
    .bind(req.tenant_id)
    .bind(Uuid::now_v7())
    .bind(req.group_id)
    .bind(req.membership_id)
    .bind(req.actor_membership_id)
    .bind(ts_param(at.now()))
    .execute(&mut *tx)
    .await?;
    write_audit(
        &mut tx,
        at,
        audit(req.tenant_id, req.actor_membership_id, "group.member_added", req.group_id, json!({ "membership_id": req.membership_id })),
    )
    .await?;
    notify(&mut tx, req.tenant_id, Change::Changed(Resource::Group(req.group_id))).await?;
    tx.commit().await?;
    Ok(())
}

/// Removes a hand-added member, softly. Membership through a role is ended on the role,
/// not here (`NotInGroup`). Allowed while frozen and on an archived group. Closes the
/// removed member's live streams (Ruling R3).
pub async fn remove_group_member(pool: &PgPool, req: GroupMemberChange, at: Moment) -> Result<(), MembershipError> {
    let mut tx = pool.begin().await?;
    lock_tenant(&mut tx, req.tenant_id).await?;
    manage(&mut tx, req.tenant_id, req.actor_membership_id, req.group_id, at).await?;
    let row: Option<Uuid> = sqlx::query_scalar(
        "select id from group_members
          where tenant_id = $1 and group_id = $2 and membership_id = $3 and removed_at is null
          for update",
    )
    .bind(req.tenant_id)
    .bind(req.group_id)
    .bind(req.membership_id)
    .fetch_optional(&mut *tx)
    .await?;
    let row = row.ok_or(MembershipError::NotInGroup)?;
    sqlx::query("update group_members set removed_at = $3::timestamptz where tenant_id = $1 and id = $2")
        .bind(req.tenant_id)
        .bind(row)
        .bind(ts_param(at.now()))
        .execute(&mut *tx)
        .await?;
    write_audit(
        &mut tx,
        at,
        audit(req.tenant_id, req.actor_membership_id, "group.member_removed", req.group_id, json!({ "membership_id": req.membership_id })),
    )
    .await?;
    notify(&mut tx, req.tenant_id, Change::AccessRevoked { membership_id: req.membership_id }).await?;
    notify(&mut tx, req.tenant_id, Change::Changed(Resource::Group(req.group_id))).await?;
    tx.commit().await?;
    Ok(())
}

/// `revoke_membership`'s cascade (Ruling R15): the membership leaves every group it was
/// added to by hand, each removal audited. Without it, a re-invite (which reopens the same
/// membership row) would hand a removed person their closed groups back. Runs inside the
/// caller's transaction, under its tenant lock.
pub(crate) async fn remove_from_all_groups(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    actor_membership_id: Uuid,
    membership_id: Uuid,
    at: Moment,
) -> Result<(), MembershipError> {
    let groups: Vec<Uuid> = sqlx::query_scalar(
        "update group_members set removed_at = $3::timestamptz
          where tenant_id = $1 and membership_id = $2 and removed_at is null
         returning group_id",
    )
    .bind(tenant_id)
    .bind(membership_id)
    .bind(ts_param(at.now()))
    .fetch_all(&mut *conn)
    .await?;
    for group_id in groups {
        write_audit(
            conn,
            at,
            audit(
                tenant_id,
                actor_membership_id,
                "group.member_removed",
                group_id,
                json!({ "membership_id": membership_id, "cause": "membership_revoked" }),
            ),
        )
        .await?;
        notify(conn, tenant_id, Change::Changed(Resource::Group(group_id))).await?;
    }
    Ok(())
}
```

In `backend/crates/persistence/src/membership/mod.rs`, add `mod groups;` after `mod events;`, and after the `events` re-export add:

```rust
pub use groups::{
    add_group_member, archive_group, create_group, remove_group_member, rename_group,
    set_group_visibility, ArchiveGroup, CreateGroup, GroupBinding, GroupMemberChange,
    RenameGroup, SetGroupVisibility, GROUP_NAME_AAD, GROUP_NAME_CIPHERTEXT_BYTES,
};
```

In `backend/crates/persistence/src/membership/roles.rs`, add `use super::groups::remove_from_all_groups;`. In `revoke_membership`, directly after the `withdraw_pending_requests(…).await?;` call, before the `notify(` Task 6 added, insert:

```rust
    remove_from_all_groups(
        &mut tx,
        req.tenant_id,
        req.actor_membership_id,
        req.membership_id,
        at,
    )
    .await?;
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p fau-app --test groups --test roles --test events --test authorization`
Expected: `groups` passes 9 tests. `roles` (including the last-admin and leave tests), `events` and `authorization` still pass.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add backend/crates/persistence/src/membership/groups.rs backend/crates/persistence/src/membership/error.rs \
  backend/crates/persistence/src/membership/mod.rs backend/crates/persistence/src/membership/roles.rs \
  backend/crates/app/tests/groups.rs
git commit -m "Add group management: admin-only, audited, announced, frozen-aware (#3501)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: Group reads (list, get, members) through the same rule

**Files:**
- Modify: `backend/crates/persistence/src/membership/groups.rs` (append the reads, extend imports)
- Modify: `backend/crates/persistence/src/membership/mod.rs` (re-exports)
- Test: `backend/crates/app/tests/group_reads.rs`

**Interfaces:**
- Consumes:
  - `decide`, `GroupFacts`, `Target` (Task 3);
  - `authorize`, `read_transaction`, `denied`, `in_group_sql`, `ROLE_FOLLOWS_GROUP`, `ASSIGNMENT_VALID_ON_3`, `membership_access` (Tasks 4 and 5);
  - `GroupBinding` (Task 7).
- Produces, re-exported from `fau_persistence::membership`:
  - `pub struct GroupView { pub group_id: Uuid, pub encrypted_name: Ciphertext, pub visibility: Visibility, pub archived: bool, pub binding: Option<GroupBinding>, pub viewer_in_group: bool }` (`Debug, Clone, PartialEq, Eq`);
  - `pub struct GroupMemberView { pub membership_id: Uuid, pub added_by_hand: bool, pub through_role: bool }` (`Debug, Clone, Copy, PartialEq, Eq`);
  - `pub async fn list_groups(pool: &PgPool, viewer: Viewer, at: Moment) -> Result<Vec<GroupView>, MembershipError>`, ordered by id, holding only the groups the viewer may read, and `NotAuthorized` without standing;
  - `pub async fn get_group(pool: &PgPool, viewer: Viewer, group_id: Uuid, at: Moment) -> Result<GroupView, MembershipError>`, which returns `UnknownGroup` when the group is hidden or unknown;
  - `pub async fn list_group_members(pool: &PgPool, viewer: Viewer, group_id: Uuid, at: Moment) -> Result<Vec<GroupMemberView>, MembershipError>`: readable exactly when the group is (R22), holding current members with standing, ordered by membership id.

- [ ] **Step 1: Write the failing tests**

Create `backend/crates/app/tests/group_reads.rs`:

```rust
//! Group reads (groups design §3.3): a list holds exactly the groups `authorize` lets the
//! viewer read (Ruling R11), a hidden group is not found, and a member list follows the
//! group's own visibility (R22).

mod common;
use common::groups::*;
use common::membership::*;
use common::TestDb;
use fau_crypto::Ciphertext;
use fau_domain::authz::Action;
use fau_domain::membership::vocabulary::{CapabilityClass, Visibility};
use fau_domain::membership::period::Period;
use fau_persistence::membership::{
    authorize, get_group, list_group_members, list_groups, GroupMemberView, MembershipError,
    Resource, RoleChoice, Viewer,
};
use sqlx::PgPool;
use uuid::Uuid;

#[tokio::test]
async fn a_list_holds_exactly_what_authorize_lets_the_viewer_read() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let t0 = at(T0);
    let mut conn = pool.acquire().await.unwrap();
    for name in VIEWERS {
        let viewer = w.viewer(name);
        let listed = list_groups(&pool, viewer, t0).await;
        if name.starts_with("none") {
            assert_eq!(listed.unwrap_err(), MembershipError::NotAuthorized, "{name}");
            continue;
        }
        let mut listed: Vec<Uuid> = listed.unwrap().iter().map(|g| g.group_id).collect();
        listed.sort_unstable();
        let mut readable = Vec::new();
        for g in [w.open, w.closed, w.other] {
            if authorize(&mut conn, viewer, Resource::Group(g), Action::Read, t0).await.unwrap().is_ok() {
                readable.push(g);
            }
        }
        readable.sort_unstable();
        assert_eq!(listed, readable, "{name}");
    }

    assert_eq!(ids(&pool, &w, "admin_out").await, sorted(vec![w.open, w.closed, w.other]));
    assert_eq!(ids(&pool, &w, "member_out").await, vec![w.open]);
    assert_eq!(ids(&pool, &w, "guest_in").await, sorted(vec![w.open, w.closed]));
    assert_eq!(ids(&pool, &w, "guest_out").await, vec![w.other]);
}

async fn ids(pool: &PgPool, w: &World, name: &str) -> Vec<Uuid> {
    let listed = list_groups(pool, w.viewer(name), at(T0)).await.unwrap();
    sorted(listed.iter().map(|g| g.group_id).collect())
}

fn sorted(mut v: Vec<Uuid>) -> Vec<Uuid> {
    v.sort_unstable();
    v
}

async fn join(pool: &PgPool, fau: &Fau, address: &str, role: RoleChoice, p: Period) -> Uuid {
    add_member(pool, fau, address, role, p, at(T0)).await.membership_id
}

#[tokio::test]
async fn a_hidden_group_is_not_found_and_a_visible_one_carries_its_facts() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let t0 = at(T0);
    for group in [w.closed, Uuid::now_v7()] {
        assert_eq!(
            get_group(&pool, w.viewer("member_out"), group, t0).await.unwrap_err(),
            MembershipError::UnknownGroup
        );
    }
    let seen = get_group(&pool, w.viewer("guest_in"), w.closed, t0).await.unwrap();
    assert_eq!(seen.group_id, w.closed);
    assert_eq!(seen.encrypted_name, Ciphertext::from_stored(placeholder_name()));
    assert_eq!(seen.visibility, Visibility::Closed);
    assert!(!seen.archived && seen.binding.is_none() && seen.viewer_in_group);
    let admin_view = get_group(&pool, w.viewer("admin_out"), w.closed, t0).await.unwrap();
    assert!(!admin_view.viewer_in_group, "an admin reads a group without being in it");
}

#[tokio::test]
async fn a_member_list_shows_role_holders_and_hand_added_members_with_standing() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let unit = seed_unit(&pool, fau.tenant_id).await;
    let unit_role = seed_role(&pool, fau.tenant_id, CapabilityClass::Member, None, Some(unit), None).await;
    let group = seed_bound_group(&pool, &fau, Visibility::Closed, Some(unit), None).await;
    let year = period(day(2026, 9, 1), day(2027, 9, 1));
    let by_role = || RoleChoice::Existing(unit_role);

    let holder = join(&pool, &fau, "rolle@example.test", by_role(), year).await;
    let by_hand = join(&pool, &fau, "hand@example.test", new_role("Medlem", CapabilityClass::Member), year).await;
    let both = join(&pool, &fau, "begge@example.test", by_role(), year).await;
    let short = join(&pool, &fau, "kort@example.test", by_role(), period(day(2026, 9, 1), day(2026, 10, 1))).await;
    let revoked = join(&pool, &fau, "borte@example.test", by_role(), year).await;
    seed_group_member(&pool, &fau, group, by_hand).await;
    seed_group_member(&pool, &fau, group, both).await;
    revoke(&pool, &fau, live_assignment(&pool, revoked).await, t0).await;

    let admin = Viewer { tenant_id: fau.tenant_id, membership_id: fau.admin_membership_id };
    let mut want = vec![
        GroupMemberView { membership_id: holder, added_by_hand: false, through_role: true },
        GroupMemberView { membership_id: by_hand, added_by_hand: true, through_role: false },
        GroupMemberView { membership_id: both, added_by_hand: true, through_role: true },
        GroupMemberView { membership_id: short, added_by_hand: false, through_role: true },
    ];
    want.sort_by_key(|m| m.membership_id);
    assert_eq!(list_group_members(&pool, admin, group, t0).await.unwrap(), want);

    // On 1 October the short role has ended: no job ran, and the list no longer holds it.
    want.retain(|m| m.membership_id != short);
    assert_eq!(
        list_group_members(&pool, admin, group, at("2026-10-01T10:00:00Z")).await.unwrap(),
        want
    );

    // A member outside the closed group cannot see who is in it, or that it exists.
    let outsider = Viewer { tenant_id: fau.tenant_id, membership_id: by_hand };
    sqlx::query("update group_members set removed_at = $1::timestamptz where membership_id = $2")
        .bind(T0)
        .bind(by_hand)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        list_group_members(&pool, outsider, group, t0).await.unwrap_err(),
        MembershipError::UnknownGroup
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p fau-app --test group_reads`
Expected: does not compile, because `list_groups`, `get_group`, `list_group_members` and `GroupMemberView` are missing.

- [ ] **Step 3: Implement the reads**

In `backend/crates/persistence/src/membership/groups.rs`, extend the imports:
- `use fau_domain::authz::{decide, Action, GroupFacts, Target};`, replacing `use fau_domain::authz::Action;`;
- `use fau_domain::membership::access::Capability;`;
- `use super::access::membership_access;`;
- `use super::authz::{authorize, denied, in_group_sql, read_transaction, Resource, Viewer, ASSIGNMENT_VALID_ON_3, ROLE_FOLLOWS_GROUP};`, replacing the existing `super::authz` line;
- add `date_param` and `USABLE_ACCOUNT` to the `super::sql` import.

Append:

```rust
/// A group as a viewer may see it. The name is the stored ciphertext; the session decrypts
/// it under the record key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupView {
    pub group_id: Uuid,
    pub encrypted_name: Ciphertext,
    pub visibility: Visibility,
    pub archived: bool,
    pub binding: Option<GroupBinding>,
    /// Whether the viewer is a current member (by hand or through a role).
    pub viewer_in_group: bool,
}

/// One current member of a group, and how they are in it. A person can be both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupMemberView {
    pub membership_id: Uuid,
    pub added_by_hand: bool,
    pub through_role: bool,
}

type GroupRow = (Uuid, Vec<u8>, String, bool, Option<Uuid>, Option<Uuid>, bool);

fn group_select() -> String {
    format!(
        "select g.id, g.encrypted_name, g.visibility, g.archived_at is not null,
                g.unit_id, g.cohort_id, {}
           from groups g
          where g.tenant_id = $1",
        in_group_sql()
    )
}

fn view(row: GroupRow) -> Result<GroupView, MembershipError> {
    let (group_id, name, visibility, archived, unit_id, cohort_id, viewer_in_group) = row;
    let binding = match (unit_id, cohort_id) {
        (None, None) => None,
        (Some(unit), None) => Some(GroupBinding::Unit(unit)),
        (None, Some(cohort)) => Some(GroupBinding::Cohort(cohort)),
        (Some(_), Some(_)) => return Err(MembershipError::decode()),
    };
    Ok(GroupView {
        group_id,
        encrypted_name: Ciphertext::from_stored(name),
        visibility: Visibility::from_code(&visibility).ok_or_else(MembershipError::decode)?,
        archived,
        binding,
        viewer_in_group,
    })
}

/// Every group the viewer may read (§3.3): all of them for an admin; open ones and their
/// own closed ones for a member; only their own for a guest. One snapshot, with the same
/// facts and the same rule `authorize` uses (Ruling R11).
pub async fn list_groups(pool: &PgPool, viewer: Viewer, at: Moment) -> Result<Vec<GroupView>, MembershipError> {
    let mut tx = read_transaction(pool).await?;
    let access = membership_access(&mut tx, viewer.tenant_id, viewer.membership_id, at.today()).await?;
    if access.capability == Capability::None {
        return Err(MembershipError::NotAuthorized);
    }
    let sql = format!("{} order by g.id", group_select());
    let rows: Vec<GroupRow> = sqlx::query_as(&sql)
        .bind(viewer.tenant_id)
        .bind(viewer.membership_id)
        .bind(date_param(at.today()))
        .fetch_all(&mut *tx)
        .await?;
    tx.commit().await?;
    let mut visible = Vec::new();
    for row in rows {
        let group = view(row)?;
        let facts = GroupFacts {
            visibility: group.visibility,
            archived: group.archived,
            viewer_in_group: group.viewer_in_group,
        };
        if decide(access.capability, Target::Group(facts), Action::Read).is_ok() {
            visible.push(group);
        }
    }
    Ok(visible)
}

/// One group, if the viewer may read it; `UnknownGroup` otherwise, exactly as for an id
/// that does not exist.
pub async fn get_group(pool: &PgPool, viewer: Viewer, group_id: Uuid, at: Moment) -> Result<GroupView, MembershipError> {
    let mut tx = read_transaction(pool).await?;
    authorize(&mut tx, viewer, Resource::Group(group_id), Action::Read, at)
        .await?
        .map_err(|d| denied(d, MembershipError::UnknownGroup))?;
    let sql = format!("{} and g.id = $4", group_select());
    let row: GroupRow = sqlx::query_as(&sql)
        .bind(viewer.tenant_id)
        .bind(viewer.membership_id)
        .bind(date_param(at.today()))
        .bind(group_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    view(row)
}

/// The group's current members: added by hand and not removed, or holding a role valid
/// today that the group follows. Only people with standing today count: a usable
/// membership with some role valid today. Readable exactly when the group is (Ruling R22).
pub async fn list_group_members(
    pool: &PgPool,
    viewer: Viewer,
    group_id: Uuid,
    at: Moment,
) -> Result<Vec<GroupMemberView>, MembershipError> {
    let mut tx = read_transaction(pool).await?;
    authorize(&mut tx, viewer, Resource::Group(group_id), Action::Read, at)
        .await?
        .map_err(|d| denied(d, MembershipError::UnknownGroup))?;
    let sql = format!(
        "select s.membership_id, bool_or(s.by_hand), bool_or(not s.by_hand)
           from (select gm.membership_id, true as by_hand
                   from group_members gm
                  where gm.tenant_id = $1 and gm.group_id = $2 and gm.removed_at is null
                 union all
                 select ra.membership_id, false
                   from groups g
                   join roles r             on r.tenant_id = g.tenant_id and {ROLE_FOLLOWS_GROUP}
                   join role_assignments ra on ra.tenant_id = r.tenant_id and ra.role_id = r.id
                  where g.tenant_id = $1 and g.id = $2 and {ASSIGNMENT_VALID_ON_3}) s
           join memberships m on m.tenant_id = $1 and m.id = s.membership_id
           join accounts a    on a.id = m.account_id
          where {USABLE_ACCOUNT}
            and exists (select 1 from role_assignments ra
                         where ra.tenant_id = m.tenant_id and ra.membership_id = m.id
                           and {ASSIGNMENT_VALID_ON_3})
          group by s.membership_id
          order by s.membership_id"
    );
    let rows: Vec<(Uuid, bool, bool)> = sqlx::query_as(&sql)
        .bind(viewer.tenant_id)
        .bind(group_id)
        .bind(date_param(at.today()))
        .fetch_all(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(rows
        .into_iter()
        .map(|(membership_id, added_by_hand, through_role)| GroupMemberView {
            membership_id,
            added_by_hand,
            through_role,
        })
        .collect())
}
```

(`list_group_members` binds `$2` to the group, not the viewer. `ASSIGNMENT_VALID_ON_3` only reads `$3`, the date, so reusing the fragment is correct here.)

In `mod.rs`, add `get_group`, `list_group_members`, `list_groups`, `GroupMemberView` and `GroupView` to the `groups` re-export.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p fau-app --test group_reads --test groups --test authorization`
Expected: `group_reads` passes 3 tests; `groups` and `authorization` still pass.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add backend/crates/persistence/src/membership/groups.rs backend/crates/persistence/src/membership/mod.rs \
  backend/crates/app/tests/group_reads.rs
git commit -m "Add group reads that agree with authorize, hiding what a viewer may not read (#3501)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: Guest roles in invitations

**Files:**
- Modify: `backend/crates/persistence/src/membership/invitations.rs` (`RoleChoice::New`, `resolve_roles`, `issue_invitation`)
- Modify: `backend/crates/persistence/src/membership/error.rs` (`RoleGroupMismatch`)
- Modify: `backend/crates/app/tests/common/membership.rs` (`new_role`)
- Test: `backend/crates/app/tests/guests.rs`

**Interfaces:**
- Consumes: `archive_group` and `ArchiveGroup` (Task 7), and `authorize` and `Resource` (Task 4).
- Produces:
  - `RoleChoice::New { name: RoleName, capability: CapabilityClass, group_id: Option<Uuid> }`, where `group_id` is `Some` exactly when `capability` is `Guest`;
  - `MembershipError::RoleGroupMismatch`;
  - a handover invitation offering a guest role is refused with `NotAuthorized` (R17);
  - offering a guest role whose group is unknown gives `UnknownGroup`, and one whose group is archived gives `GroupArchived`;
  - the `role.created` audit params gain `group_id`, which is null for a non-guest role.

- [ ] **Step 1: Write the failing tests**

In `backend/crates/app/tests/common/membership.rs`, change `new_role` to:

```rust
pub fn new_role(name: &str, capability: CapabilityClass) -> RoleChoice {
    RoleChoice::New {
        name: RoleName::parse(name).unwrap(),
        capability,
        group_id: None,
    }
}
```

Create `backend/crates/app/tests/guests.rs`:

```rust
//! The guest member type (groups design §3.1, §8's #3418 row): an admin invites a guest to
//! one group through a guest role naming it. The guest reaches that group and nothing
//! FAU-wide; a handover grant cannot invite one.

mod common;
use common::groups::*;
use common::membership::*;
use common::TestDb;
use fau_domain::authz::{Action, Denied};
use fau_domain::membership::access::Capability;
use fau_domain::membership::vocabulary::{CapabilityClass, RoleName, Visibility};
use fau_persistence::membership::{
    archive_group, authorize, create_handover_grants, effective_access, grant_role,
    issue_invitation, ArchiveGroup, GrantRole, IssueInvitation, MembershipError, OfferedRole,
    Resource, RoleChoice, Viewer,
};
use sqlx::PgPool;
use uuid::Uuid;

fn guest_role(group: Option<Uuid>) -> RoleChoice {
    RoleChoice::New {
        name: RoleName::parse("Gjest").unwrap(),
        capability: CapabilityClass::Guest,
        group_id: group,
    }
}

fn invite(fau: &Fau, role: RoleChoice) -> IssueInvitation {
    IssueInvitation {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        recipient: email("gjest@example.test"),
        roles: vec![OfferedRole {
            role,
            period: period(day(2026, 9, 23), day(2027, 9, 1)),
        }],
        handover_grant_id: None,
        message: None,
    }
}

async fn decision(pool: &PgPool, viewer: Viewer, resource: Resource, action: Action) -> Result<(), Denied> {
    let mut conn = pool.acquire().await.unwrap();
    authorize(&mut conn, viewer, resource, action, at(T0)).await.unwrap()
}

#[tokio::test]
async fn an_admin_invites_a_guest_to_one_group() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let group = seed_group(&pool, &fau, Visibility::Closed).await;
    let other = seed_group(&pool, &fau, Visibility::Open).await;
    let guest = add_member(
        &pool,
        &fau,
        "gjest@example.test",
        guest_role(Some(group)),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;

    assert_eq!(
        effective_access(&pool, guest.account_id, fau.tenant_id, t0).await.unwrap().capability,
        Capability::Guest
    );
    let v = Viewer { tenant_id: fau.tenant_id, membership_id: guest.membership_id };
    assert_eq!(decision(&pool, v, Resource::GroupContent(group), Action::Read).await, Ok(()));
    assert_eq!(decision(&pool, v, Resource::GroupContent(group), Action::Write).await, Ok(()));
    assert_eq!(decision(&pool, v, Resource::Fau, Action::Read).await, Err(Denied::Hidden));
    assert_eq!(
        decision(&pool, v, Resource::GroupContent(other), Action::Read).await,
        Err(Denied::Hidden),
        "not even an open group the guest was not invited to"
    );

    let (class, named): (String, Option<Uuid>) = sqlx::query_as(
        "select r.capability_class, r.group_id from role_assignments ra
           join roles r on r.tenant_id = ra.tenant_id and r.id = ra.role_id
          where ra.membership_id = $1",
    )
    .bind(guest.membership_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((class.as_str(), named), ("guest", Some(group)));
    let created: String = sqlx::query_scalar(
        "select params::text from audit_events where action = 'role.created' and subject_id = (
           select role_id from role_assignments where membership_id = $1)",
    )
    .bind(guest.membership_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let created: serde_json::Value = serde_json::from_str(&created).unwrap();
    assert_eq!(created["capability_class"], "guest");
    assert_eq!(created["group_id"], group.to_string());
}

#[tokio::test]
async fn a_guest_role_names_a_group_and_only_a_guest_role_may() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let group = seed_group(&pool, &fau, Visibility::Open).await;
    let roles_before = count(&pool, "select count(*) from roles").await;
    let member_naming_group = RoleChoice::New {
        name: RoleName::parse("Medlem").unwrap(),
        capability: CapabilityClass::Member,
        group_id: Some(group),
    };
    for role in [guest_role(None), member_naming_group] {
        assert_eq!(
            issue_invitation(&pool, invite(&fau, role), t0).await.unwrap_err(),
            MembershipError::RoleGroupMismatch
        );
    }
    assert_eq!(count(&pool, "select count(*) from roles").await, roles_before, "nothing written");
    assert_eq!(count(&pool, "select count(*) from invitations where mode = 'normal'").await, 0);
}

#[tokio::test]
async fn a_guest_role_cannot_name_an_unknown_or_archived_group() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    assert_eq!(
        issue_invitation(&pool, invite(&fau, guest_role(Some(Uuid::now_v7()))), t0).await.unwrap_err(),
        MembershipError::UnknownGroup
    );

    let group = seed_group(&pool, &fau, Visibility::Open).await;
    let existing = seed_role(&pool, fau.tenant_id, CapabilityClass::Guest, Some(group), None, None).await;
    archive_group(
        &pool,
        ArchiveGroup {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            group_id: group,
        },
        t0,
    )
    .await
    .unwrap();
    for role in [guest_role(Some(group)), RoleChoice::Existing(existing)] {
        assert_eq!(
            issue_invitation(&pool, invite(&fau, role), t0).await.unwrap_err(),
            MembershipError::GroupArchived
        );
    }
}

#[tokio::test]
async fn an_admin_may_grant_a_member_a_guest_role_too() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let group = seed_group(&pool, &fau, Visibility::Closed).await;
    let member = add_member(
        &pool,
        &fau,
        "m@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 1), day(2027, 9, 1)),
        t0,
    )
    .await;
    let v = Viewer { tenant_id: fau.tenant_id, membership_id: member.membership_id };
    assert_eq!(decision(&pool, v, Resource::GroupContent(group), Action::Read).await, Err(Denied::Hidden));
    grant_role(
        &pool,
        GrantRole {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: member.membership_id,
            role: guest_role(Some(group)),
            period: period(day(2026, 9, 23), day(2027, 9, 1)),
        },
        t0,
    )
    .await
    .unwrap();
    assert_eq!(decision(&pool, v, Resource::GroupContent(group), Action::Write).await, Ok(()));
    assert_eq!(
        effective_access(&pool, member.account_id, fau.tenant_id, t0).await.unwrap().capability,
        Capability::Member,
        "rights are the union: still a member FAU-wide"
    );
}

#[tokio::test]
async fn a_handover_grant_cannot_invite_a_guest() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    // An outgoing admin who stays a member, inside their six-month handover window.
    let fau = active_fau(&pool, "gammel@example.test", t0).await;
    grant_role(
        &pool,
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
    let group = seed_group(&pool, &fau, Visibility::Open).await;
    let guest = seed_role(&pool, fau.tenant_id, CapabilityClass::Guest, Some(group), None, None).await;
    let inside = at("2027-11-01T10:00:00Z");
    create_handover_grants(&pool, inside).await.unwrap();
    let grant_id: Uuid = sqlx::query_scalar("select id from handover_grants where source_assignment_id = $1")
        .bind(fau.admin_assignment_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    let handover = |role| IssueInvitation {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        recipient: email("ny@example.test"),
        roles: vec![OfferedRole {
            role,
            period: period(day(2027, 11, 1), day(2028, 10, 1)),
        }],
        handover_grant_id: Some(grant_id),
        message: None,
    };
    assert_eq!(
        issue_invitation(&pool, handover(RoleChoice::Existing(guest)), inside).await.unwrap_err(),
        MembershipError::NotAuthorized,
        "inviting guests is admin-only (Ruling R17)"
    );
    issue_invitation(&pool, handover(RoleChoice::Existing(fau.admin_role_id)), inside)
        .await
        .expect("the handover still offers the admin role it exists for");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p fau-app --test guests`
Expected: does not compile, because `RoleChoice::New` has no field `group_id` and `MembershipError::RoleGroupMismatch` does not exist.

- [ ] **Step 3: Implement**

In `backend/crates/persistence/src/membership/error.rs`, under the `// Roles and memberships.` group, after `NotInNoAdminState`, add:

```rust
    #[error("a guest role must name one group, and only a guest role may")]
    RoleGroupMismatch,
```

In `backend/crates/persistence/src/membership/invitations.rs`:

1. Replace `RoleChoice` with:

```rust
/// What an issuer offers: an existing role, or a new one created with the invitation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoleChoice {
    Existing(Uuid),
    New {
        name: RoleName,
        capability: CapabilityClass,
        /// The group a guest role reaches (groups design §3.1, Ruling R4). `Some` exactly
        /// when `capability` is `Guest`; `RoleGroupMismatch` otherwise.
        group_id: Option<Uuid>,
    },
}
```

2. In `resolve_roles`, replace the `RoleChoice::Existing(id) => { … }` arm with:

```rust
            RoleChoice::Existing(id) => {
                let row: Option<(String, bool)> = sqlx::query_as(
                    "select r.capability_class, g.archived_at is not null
                       from roles r
                       left join groups g on g.tenant_id = r.tenant_id and g.id = r.group_id
                      where r.tenant_id = $1 and r.id = $2",
                )
                .bind(tenant_id)
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?;
                let (class, group_archived) = row.ok_or(MembershipError::UnknownRole)?;
                if group_archived {
                    return Err(MembershipError::GroupArchived);
                }
                (
                    *id,
                    CapabilityClass::from_code(&class).ok_or_else(MembershipError::decode)?,
                )
            }
```

(For a role without a group, the left join gives `null is not null`, which is `false`.)

3. Replace the `RoleChoice::New { name, capability } => {` arm's opening, its `insert`, and its audit `params` so that the arm reads:

```rust
            RoleChoice::New {
                name,
                capability,
                group_id,
            } => {
                if (*capability == CapabilityClass::Guest) != group_id.is_some() {
                    return Err(MembershipError::RoleGroupMismatch);
                }
                if let Some(group) = group_id {
                    let archived: Option<bool> = sqlx::query_scalar(
                        "select archived_at is not null from groups where tenant_id = $1 and id = $2",
                    )
                    .bind(tenant_id)
                    .bind(group)
                    .fetch_optional(&mut *conn)
                    .await?;
                    match archived {
                        None => return Err(MembershipError::UnknownGroup),
                        Some(true) => return Err(MembershipError::GroupArchived),
                        Some(false) => {}
                    }
                }
                let id = Uuid::now_v7();
                sqlx::query(
                    "insert into roles (tenant_id, id, name, capability_class, group_id)
                     values ($1, $2, $3, $4, $5)",
                )
                .bind(tenant_id)
                .bind(id)
                .bind(name.as_str())
                .bind(capability.code())
                .bind(group_id)
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
                        params: json!({ "capability_class": capability.code(), "group_id": group_id }),
                    },
                )
                .await?;
                (id, *capability)
            }
```

4. In `issue_invitation`, directly after `let roles = resolve_roles(…).await?;`, insert:

```rust
    // Inviting guests is admin-only (groups design §3.3); a handover grant is not admin
    // authority (Ruling R17).
    if mode == InvitationMode::Handover
        && roles.iter().any(|(_, class, _)| *class == CapabilityClass::Guest)
    {
        return Err(MembershipError::NotAuthorized);
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p fau-app --test guests --test invitations --test handover_recovery --test roles --test signup`
Expected: `guests` passes 5 tests, and the existing invitation, handover, role and signup tests still pass. `cargo build --workspace --all-targets` compiles, and outside `persistence`, `RoleChoice::New` is constructed only by `new_role` and by `guests.rs`.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add backend/crates/persistence/src/membership/invitations.rs backend/crates/persistence/src/membership/error.rs \
  backend/crates/app/tests/common/membership.rs backend/crates/app/tests/guests.rs
git commit -m "Invite guests through a guest role naming one group; never under handover (#3501)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 10: Group names under the record key end to end; record the rulings; full verification

**Files:**
- Modify: `backend/crates/app/tests/key_chain.rs` (one new test and one import line)
- Modify: `docs/planning-decisions.md` (append a section below the uncommitted "Erik's go-ahead for #3501" section already in the working tree; keep that section)

**Interfaces:**
- Consumes:
  - `create_group`, `CreateGroup`, `list_groups`, `Viewer` and `GROUP_NAME_AAD` (Tasks 4, 7 and 8);
  - `GroupName` and `Visibility` (Task 2);
  - `data_key`, `KeyCache`, `Keys` (#3506, existing).
- Produces: proof that a group name is stored only as a record-key envelope bound to its row, plus the recorded rulings.

- [ ] **Step 1: Write the end-to-end test**

In `backend/crates/app/tests/key_chain.rs`, add `use fau_domain::membership::vocabulary::{GroupName, Visibility};` after the `fau_crypto` import, and append:

```rust
/// Groups design §3.1: a group's name is content, encrypted under the FAU's record key and
/// bound to its own row. Nothing readable reaches Postgres.
#[tokio::test]
async fn group_names_are_encrypted_under_the_record_key_and_bound_to_their_row() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let keys = keys().await;
    let cache = KeyCache::new(
        keys.clone(),
        Arc::new(jiff::Timestamp::now),
        SignedDuration::from_mins(30),
    );
    let record = Unit::Record {
        tenant: fau.tenant_id,
    };
    let session = Uuid::now_v7();
    let key = {
        let mut c = pool.acquire().await.unwrap();
        data_key(&mut c, &keys, &cache, session, &record).await.unwrap()
    };

    let name = GroupName::parse("Oppfølging av sak med rektor").unwrap();
    let group_id = Uuid::now_v7();
    let (table, column) = GROUP_NAME_AAD;
    let aad = Aad::new(fau.tenant_id, table, column, group_id);
    create_group(
        &pool,
        CreateGroup {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            group_id,
            encrypted_name: encrypt(&key, &aad, name.as_str()).unwrap(),
            visibility: Visibility::Closed,
            binding: None,
        },
        t0,
    )
    .await
    .unwrap();

    let stored: Vec<u8> = sqlx::query_scalar("select encrypted_name from groups where id = $1")
        .bind(group_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let plain = name.as_str().as_bytes();
    assert!(!stored.windows(plain.len()).any(|w| w == plain));
    assert!(!stored.windows(6).any(|w| w == b"rektor"));

    let viewer = Viewer {
        tenant_id: fau.tenant_id,
        membership_id: fau.admin_membership_id,
    };
    let listed = list_groups(&pool, viewer, t0).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        decrypt(&key, &aad, &listed[0].encrypted_name).unwrap().as_str(),
        name.as_str()
    );
    // Bound to its row: the same ciphertext under another group's id does not open.
    let elsewhere = Aad::new(fau.tenant_id, table, column, Uuid::now_v7());
    assert!(decrypt(&key, &elsewhere, &listed[0].encrypted_name).is_err());
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p fau-app --test key_chain`
Expected: both tests pass. The compose `openbao` must be up and configured, as in #3506. If it is not, run `docker compose -f /workspace/compose.yaml up -d openbao openbao-config` and wait for it to become healthy.

- [ ] **Step 3: Record the rulings**

Append to `docs/planning-decisions.md`:

```markdown

## #3501 built: rulings for Erik's review — 27 September 2026

Groups, the guest member type and one authorization function were built on `groups-3501`
following docs/superpowers/plans/2026-09-27-groups-guests-authorization-3501.md. The
plan's rulings, open to challenge:

- **The change stream.**
  - It stops at `Hub`/`Subscription` in `fau-persistence`. The SSE route waits for #3417's
    sessions, since nothing identifies a viewer yet.
  - Each process holds one `LISTEN` connection, and every delivery is re-authorized for its
    viewer.
  - These close a membership's streams: revoking a role or a membership, and removing a
    hand-added group member.
  - A role reaching its end date closes the stream at its next delivery, without a timer.
  - A listener reconnect, or a stream more than 64 deliveries behind, closes streams; the
    client reconnects and refetches.
- **Guest roles.** A guest role names exactly one group (`roles.group_id`, check
  `(capability_class = 'guest') = (group_id is not null)`), and only a guest role names a
  group. A guest in two groups holds two roles or is added by hand.
- **Binding.** A group binds to at most one unit or cohort. A role holder is in it when the
  role, valid today, names the group, sits on its unit or sits on its cohort. There is no
  traversal through `unit_cohorts`.
- **#3412's remainder.**
  - Migrated: school years, cohorts, organization units and unit cohorts, all plaintext.
    `unit_relation` is not migrated.
  - No transactions create them yet: the initial school configuration is its own work.
  - Unit kinds are `grade`, `class`, `base` and `teaching_group`, and grades are 1–10.
- **Hidden means not found for every resource.** A viewer who cannot read something gets
  the same answer as for an unknown id. `Forbidden` is used only where the viewer can read.
- **Writing and freezing.**
  - Writing to a group record is managing it, which is admin-only.
  - An archived group is read-only for everyone, admins included, and archiving is
    one-way.
  - A frozen FAU refuses create, rename, open and add-member. It allows close, archive and
    remove-member.
- **Revoking a membership soft-removes its hand-added group memberships,** so a re-invite
  does not return closed groups.
- **Guests get no FAU-wide notices.** They receive no recovery notices, neither as current
  members nor in the 24-month fallback, and a handover grant cannot invite a guest.
- **Names.** A group name is at most 100 characters, and its ciphertext is 42–512 bytes
  with version byte 1, checked in code and in the database.
- **No user-facing strings and no new error codes** until #3417 maps these errors at the edge.
- **Schema contract.** The minimum stays 2, since 0006 and 0007 are additive.

The #3418 read-path audit found two shortcuts:
- `access_request_message` checked for an admin directly; it is now routed through
  `authorize`.
- Recovery recipients counted any role; guests are now excluded.

`effective_access` now reports `Capability::Guest`. The spec's "migrations stop at 0004" is
stale: 0005 (#3506) came later.
```

- [ ] **Step 4: Full verification**

Run, from `/workspace/backend` with the three test variables exported:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --features fau-app/test-routes -- -D warnings
cargo test --workspace
cargo test -p fau-app --features test-routes --test panic_handling --test shutdown
```

Expected: formatting and both clippy runs are clean, and every test passes. That includes these new test binaries:
- `organization_schema` (7 tests);
- `groups_schema` (6);
- `authorization` (4);
- `read_paths` (3);
- `events` (5);
- `groups` (9);
- `group_reads` (3);
- `guests` (5);
- the new `key_chain` test.

It also includes the domain `authz` (4), persistence `authz` (2) and `events` (3) unit tests. The pre-existing suites must be unchanged in outcome.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/app/tests/key_chain.rs docs/planning-decisions.md
git commit -m "Prove group names are record-key envelopes end to end; record the #3501 rulings (#3501)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

After this, the controller:
- runs the final whole-branch review (superpowers:requesting-code-review);
- updates #3501 in Favro, attaching this plan and the rulings, since Erik reads Favro, not the repo;
- leaves merging and pushing to Erik.

---

## Self-review (done while writing)

**Dry run.** Before handing over, the plan's code was applied to a throwaway copy of `backend/` at `f01de45` outside the repository. The results:
- `cargo build --workspace --all-targets` and `cargo clippy --workspace --all-targets -- -D warnings`, with and without `test-routes`, were clean;
- `cargo test --workspace --no-fail-fast` passed every suite against the compose `db` and `openbao`;
- the `test-routes` suites passed too.

`cargo fmt` reflows some blocks, which is why every commit step runs it first.

**Spec coverage (§3, §8, §10):**

| Requirement | Where it lands |
|---|---|
| §3.1 `groups` with encrypted name, `visibility`, unit or cohort binding, `archived_at` | Task 2 (schema), Task 7 (transactions), Task 10 (encryption end to end) |
| §3.1 `group_members` with soft, audited removal | Task 2, Task 7 |
| §3.1 derived membership of bound groups, with no job | Task 4 (`in_group_sql`, bound-group test), Task 8 (member list) |
| §3.1 a bound group can take manual members | Task 4, Task 8 |
| §3.1 `guest` class and `CapabilityClass::Guest`; a guest role names a group | Task 2 (check, R4), Task 9 |
| §3.1 and §8 #3412's tables first, with the open FKs closed | Task 1 |
| §3.2 audience: one nullable `group_id`, null is FAU-wide and never guests | `Resource::audience` (Task 4), the matrix's FAU rows |
| §3.3 the table | Task 3 (rule), Task 4 (matrix) |
| §3.3 managing groups is admin-only | Task 3 (`Manage`), Task 7 |
| §3.3 closed group hidden | R12, the matrix, Task 7, Task 8 |
| §3.3 SSE filtered through the same function | Task 6 |
| §3.3 revocation takes effect on the next request and closes live connections | Task 4, Task 6, Task 7 |
| §3.3 #3418 read-path audit before `Guest` | Audit table, Task 5, R21 |
| §3.4 no per-group keys | Nothing built, as specified |
| §8 #3418: invitations grant a guest role naming a group | Task 9 |
| §8 #3419: folders get a `group_id` audience | "What later cards get", `Resource::audience` |
| §10 matrix over viewer × visibility × in/out × resource type | Task 4 (48 rows × 3 actions) |
| §10 SSE: no event reaches an unauthorized viewer, including a guest beside a closed group; revocation closes | Task 6 |

§8's "display name captured when an invitation is accepted" belongs to #3502 (spec §4.1) and is deliberately not built.

**Placeholder scan.** No TBD, TODO or "similar to Task N". Every code step carries its code.

**Type consistency, checked across tasks:**
- `Viewer { tenant_id, membership_id }`;
- `Resource::{Fau, Group, GroupContent}`;
- `authorize(conn, viewer, resource, action, at) -> Result<Decision, MembershipError>`;
- `denied(d, hidden)` and `membership_access(conn, tenant_id, membership_id, today)`;
- `in_group_sql()` with `$2` for the membership and `$3` for the date;
- `Change::{Changed, AccessRevoked { membership_id }}`, `Hub::start(pool, clock)` and `Subscription::recv`;
- `GroupMemberChange { tenant_id, actor_membership_id, group_id, membership_id }`;
- `RoleChoice::New { name, capability, group_id }`.
