# Register Apply and CLI Implementation Plan (#3441, part 4)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Apply the part 3 sync planner's plan to PostgreSQL as `fau_register`, and ship
`fau register sync [--dry-run] [--seed]` and `fau register export`, so an environment can be seeded
from NSR, Kartverket and SSB and kept in sync by a weekly CronJob.

**Architecture:**
- **Persistence, `crates/persistence/src/register/`:**
  - `snapshot.rs`: `load_snapshot`, the planner's `RegisterSnapshot<Uuid>`, read back exactly;
  - `apply.rs` and `school_ops.rs`: `apply_plan`, one op at a time inside the caller's transaction,
    exactly as `testkit::apply` and the part 3 Handover define it;
  - `reviews.rs`: review items, deduplicated against open ones by the Handover's key;
  - `staging.rs`: the `register_source_records` upsert;
  - `runs.rs`: the advisory lock, the run row, the audit entry and the operator's mail;
  - `export.rs`: the rows `fau register export` prints.
- **CLI, `crates/app/src/register/`:** `sync.rs` runs the Handover's flow on one connection that
  holds a session advisory lock; `fetch.rs` fetches the sources, four NSR details in parallel;
  `render.rs` prints a dry run's plan; `export.rs` writes CSV. Both commands log JSON to stderr,
  because stdout carries their output.
- **Tests:** everything that needs PostgreSQL lives in `crates/app/tests/`, the one database harness.
  The planner's `testkit` becomes a `fau-domain` cargo feature that only fau-app's dev-dependency
  enables, so the SQL applier is checked against `testkit::apply` on the same plans. The CLI tests
  spawn the real `fau` binary against an in-process axum server that serves the committed fixtures.

**Tech Stack:** Rust 1.98.1, sqlx 0.8 (PostgreSQL, text-bound jiff dates as in `membership::sql`),
jiff 0.2, clap 4, tokio, axum 0.8 (test server only), serde_json, sha2. No new external crate: fau-app
gains path dependencies on `fau-register-sources` and a normal dependency on `jiff`, both already in
the workspace.

**Spec:**
- `docs/school-register-design.md` §4 (data model), §5.1-5.3 (the command, the order of work,
  idempotency and the circuit breaker) and §9 (the export);
- `docs/planning-decisions.md`, every #3441 section (24-25 September 2026);
- **the "Handover to part 4" section of `docs/superpowers/plans/2026-09-25-register-sync-planner.md`,
  which is binding.** `crates/domain/src/register/sync/testkit.rs` is its executable form;
- the controller's design brief for part 4, which this plan follows except where the decisions table
  says otherwise;
- the code on branch `school-register-3441`: `crates/domain/src/register/**`,
  `crates/register-sources/**`, `crates/persistence/src/**`, `crates/app/src/{main,config,telemetry}.rs`,
  `crates/app/tests/common/` and `migrations/0004_school_register.sql`.

## Global Constraints

- All technical content (code, comments, test names, commit messages, docs) is in English.
- **Never log or store a Brreg address or contact field.** No address in review details, outbox
  params, audit params, log lines or the dry-run output. NSR addresses live only in `schools` and in
  the staged NSR payload.
- **Errors name the variable, never the value.** No error or log line carries a URL, a DSN, a
  response body, a payload or a bound SQL value: database errors go through
  `fau_persistence::safe_error_kind`, source errors through `SourceError`'s fixed `Display`.
- `crates/domain` still declares no axum, sqlx, tower, hyper, reqwest or uuid; the only domain change
  is the `testkit` feature. The `testkit` feature is never enabled outside a dev-dependency.
- No test calls the network: every source is served by a local server from
  `crates/register-sources/tests/fixtures/`.
- Everything the register writes runs as `fau_register`, and every test that writes the register does
  so through `db.register_pool()` or the `fau` binary with `db.register_url()`. Superuser pools are
  for fixtures and assertions only.
- **Tests,** from `/workspace/backend`, all clean after every task, and each test is written first:
  - `TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test --workspace --no-fail-fast`;
  - `TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test -p fau-app --no-fail-fast --features test-routes`;
  - `cargo fmt --check`;
  - `cargo clippy --workspace --all-targets -- -D warnings`.

  The harness creates and drops one throwaway database per test from its migrated template; nothing
  else touches the test cluster.
- **Commits:**
  - identity via env vars: `GIT_AUTHOR_NAME="Erik W. Bjønnes" GIT_AUTHOR_EMAIL=erik@ewb-solutions.as`,
    and the same for the committer;
  - the message ends with exactly `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`;
  - branch `school-register-3441`, never pushed.
- **Transcribe the code exactly.** Every block below was compiled, formatted with `cargo fmt`, checked
  with clippy and run against PostgreSQL in a scratch copy of the branch, and every task boundary was
  then rebuilt on its own and checked again (fmt, clippy, its tests). If a test fails, the
  transcription is wrong: diff it against this plan before changing any logic or expected value.

## Decisions this plan makes

| Question | Decision | Reason |
| --- | --- | --- |
| Migration 0005 | **None.** 0004 already grants `fau_register` every privilege part 4 uses (list in the Self-Review), and its RLS policies let `fau_register` read, insert and update `schools`. 0004 is not edited either. | Nothing needs widening. 0004 has been applied only to the harness's throwaway template databases: the dev `fau` database is unmigrated and no environment runs the app. |
| Where the persistence tests live | `crates/app/tests/register_apply.rs` and `register_runs.rs`, not in `crates/persistence` | The only database harness (`tests/common/`) is in fau-app, and persistence has no integration tests of its own. |
| How `testkit` is shared | A `testkit` feature on fau-domain: `#[cfg(any(test, feature = "testkit"))] pub mod testkit`, with its `pub(super)` functions made `pub`. Only fau-app's `[dev-dependencies]` enables it. | The brief's preferred option. Resolver 2 does not unify dev-dependency features into `cargo build --release --bin fau` (the Dockerfile's build), and Task 1 checks that with `cargo tree -e normal`. |
| How parity is checked "modulo ids" | The database's snapshot is translated to `u32` ids in UUID order. Both sides plan the same inputs, the SQL applier and `testkit::apply` each apply their plan, and the results compare after every id is replaced by a key (a municipality's number, a school's orgnr or else display name) and every list is sorted. Successor links compare the same way. | UUID order and `u32` order agree, so the planner's id tie-breaks and the order of new ids (`max + 1` against time-ordered UUIDv7s) agree too. Starting from the database rather than from a `u32` snapshot lets a test add rows `testkit` cannot build, such as a tenant or a submitted school. |
| Where the Handover's guard differs from `testkit` | Tested on its own, not for parity: two renumbers in one run write no history for the middle slug. | `testkit::apply` always pushes a history row; the Handover's "skip an empty interval" is SQL-only. |
| `apply_plan`'s signature | `apply_plan(conn, plan, at) -> AppliedCounts`, with no `run` | The applier needs nothing from the run. `record_applied(conn, run, kind, &applied, at)` writes the bookkeeping in the same transaction. |
| When staging is written | Only inside an applied run's transaction, after the plan: `stage_payloads` upserts every fetched payload | **The Handover wins over the brief's "upserts" on every run:** "`NoChange` writes nothing but the run row" (§5.3). A run whose reviews all deduplicate away rolls its staging back with everything else. |
| `last_seen_in_source_at` and `source_checked_at` | An applied run sets `last_seen_in_source_at` on every school whose orgnr it fetched. `source_checked_at` stays null. | The Handover: it can only be refreshed inside an applied run. One statement, and it keeps the column meaningful. |
| Timestamps | `created_at`, `updated_at`, history bounds, `fetched_at` and every run and outbox time come from the run's `Moment`, never `now()` | The persistence convention (`membership::sql`), and it lets tests control slug-history intervals. |
| A new municipality's number | Valid from the run's Oslo date: the day it was first seen. A renumber closes it at `greatest(change date, valid_from + 1 day)`, and the new number opens on the same date. | The true start is unknown. Without the clamp, a seed on the day of a renumber breaks `valid_from < valid_until` and every later sync fails. |
| `search_text` | A municipality: its Norwegian name, then its `municipality_names` by priority, language and name. A school: its display name, then its register name. Recomputed after every op that can change them. | §7 and the Handover (`search::search_text`). |
| Successor links | Set after every school op, in plan order | `schools.successor_id` is an immediate foreign key, and a `Close` can name a school a later `Create` makes (as `testkit` resolves them). |
| The dedup key | An open item (`resolved_at is null`) with the same kind, school, other school and municipality (`is not distinct from`), plus `details->>'municipality_number'` for `unknown_municipality_number`, and `details->>'reason'` plus `coalesce(details->>'number', details->>'old_code')` for `municipality_split_or_merge` | The Handover's corrected key. Other kinds compare no details, so `closure_with_fau` does not re-raise when only its reason text changes. |
| A `mass_change` review item | Not raised. The aborted run row and the `register.sync_aborted` mail are the alert. | The Handover leaves it optional. A queue item with nothing to resolve would need its own dedup and closing rule. |
| The lock | A session-level `pg_try_advisory_lock(0x4641_5530_0000_3441)` on the single connection that carries the whole run | An xact lock would need one transaction open across minutes of fetching. The id sits next to `MIGRATION_LOCK_ID` (`0x4641_5530_0000_0001`). |
| Planning and applying | The first snapshot only decides what to fetch. The plan is made from a second snapshot, read inside the REPEATABLE READ transaction that applies it. | Nothing can change between planning and applying, and no transaction stays open while sources are fetched. |
| What is fetched | Kartverket; SSB (a sync only); the NSR list; then the detail of every list unit with `ErAktiv` and `ErGrunnskole`, plus every orgnr of a non-closed register school that the list carries, each once, four at a time. Register orgnrs missing from the list are only counted and logged. | Closed rows are never changed (part 3), so fetching them is waste. An orgnr NSR no longer lists could answer 404 and fail every weekly run; the planner leaves a missing unit untouched anyway. |
| The SSB window | A seed passes no changes. A sync reads from the Oslo date of the earliest applied seed to today. A sync with no applied seed fails. | The Handover's fixed lookback. Guessing a start date would silently lose changes. |
| The run row | Inserted when the run starts (no outcome), then finished once: `applied`, `no_change`, `aborted` or `failed`. A failure records `failed` with its fixed error text in `abort_reason`. A lock or empty-register refusal writes no row. | The row is the alert source (#3442), so a crash leaves a visible unfinished row, and a failed fetch is visible too. |
| A dry run's row | Kind `dry_run`, outcome `no_change` (it changed nothing), with the counts and abort reason the real run would have had. A dry run whose plan aborts exits 3. | An alert on `outcome = 'aborted'` then never fires for a rehearsal, and the operator still sees the verdict. |
| Exit codes | 0 done, 1 failed, 2 bad arguments (clap), 3 aborted, 4 lock held, 5 refused | The CronJob and an operator can tell them apart. |
| Logs | JSON through the existing telemetry, on stderr (`telemetry::init_to(.., LogStream::Stderr)`) | **Conflicts with the brief's "through the existing telemetry" as it stands,** which writes to stdout, where `--dry-run` prints its plan and `export` its CSV. The container runtime collects both streams. |
| The dry-run output | JSON with the kind, the outcome, the counts, every op, and every review item with its details. A created school carries no attributes; an attribute update lists the changed field names, never their values. | The brief: never an address. |
| Mail | To `fau@ewb-solutions.as` (`fau_domain::membership::rules::EWB_OVERSIGHT_ADDRESS`), tenant null. `register.review_item`: `run_id`, `review_item_id`, `kind`. `register.seed_summary`: `run_id`, `counts`. `register.sync_aborted`: `run_id`, `kind`, `reason` (`empty_source` with `source`, or `mass_change` with its four numbers). | Ids, codes and counts only. |
| Audit | One row per applied run: tenant null, actor `system`, action `register.sync_applied`, subject `register_sync_run` / the run id, params = the counts plus `reviews_written`, `reviews_deduplicated` and `kind` | §5.2 step 7. `audit_events.tenant_id` is already nullable for global events. |
| `register_sync_runs.counts` | Every planner count by name; an applied run adds `reviews_written` and `reviews_deduplicated` | The planner's `reviews` counts items raised, not items written. |
| Configuration | `REGISTER_DATABASE_URL` (required, validated like `DATABASE_URL`); `REGISTER_NSR_URL`, `REGISTER_KARTVERKET_URL`, `REGISTER_SSB_URL` (optional; http or https, a host, no userinfo); `LOG_LEVEL` through a helper shared with `serve`. No `REGISTER_BRREG_URL`. | Part 4 never reads Brreg. A source URL could otherwise put credentials into a log line. |
| What the export lists | Pickable schools only (§7: active, listed or verified, `coalesce(scope_override, in_scope)`), ordered by municipality number and then school id, RFC 4180 quoting, LF line ends. The FAU orgnr is the `linked` row of `school_fau_links`, which stays empty until part 5. | Pending, held, rejected, closed and out-of-scope rows are not outreach targets. §7: nothing orders by a name. Values are not rewritten against spreadsheet formulas; #3431 imports the file as text. |
| The fixture SSB answer in the CLI test | `ssb/changes-2026.json` | A realistic lookback (a boundary adjustment and name changes) that must still come out `no_change`. |

---

## File Structure

```text
backend/
  Cargo.lock                              fau-app depends on fau-register-sources (Task 5)
  crates/domain/
    Cargo.toml                            + [features] testkit (Task 1)
    src/register/sync/mod.rs              testkit behind cfg(any(test, feature = "testkit")) (Task 1)
    src/register/sync/testkit.rs          pub(super) -> pub, doc (Task 1)
  crates/persistence/src/
    lib.rs                                + pub mod register (Task 1)
    register/mod.rs                       module list and re-exports (Tasks 1-4, 6)
    register/error.rs                     RegisterError (Task 1)
    register/sql.rs                       code decoders (Task 1), date/ts binding (Task 2), from_micros (Task 4)
    register/snapshot.rs                  load_snapshot, register_is_empty (Task 1)
    register/apply.rs                     apply_plan, AppliedCounts, NewIds, municipality ops (Task 2; Task 3)
    register/school_ops.rs                one function per SchoolOp (Task 3)
    register/reviews.rs                   write_reviews with dedup, NewReview (Task 3)
    register/staging.rs                   NsrPayload, stage_payloads (Task 3)
    register/runs.rs                      lock, run rows, audit, outbox, seed_date (Task 4)
    register/export.rs                    ExportRow, export_rows (Task 6)
  crates/register-sources/
    src/client.rs                         .partial removed when the final rename fails (Task 6)
    tests/client.rs                       + a_failed_final_rename_leaves_no_partial_file (Task 6)
  crates/app/
    Cargo.toml                            dev-dep fau-domain/testkit (Task 1); deps (Task 5)
    src/main.rs                           `register` subcommand (Task 5; doc in Task 6)
    src/config.rs                         log_level helper, RegisterConfig (Task 5)
    src/telemetry.rs                      LogStream, init_to (Task 5)
    src/register/mod.rs                   RegisterCommand, Exit, run (Task 5; export in Task 6)
    src/register/fetch.rs                 source URLs and the fetch (Task 5)
    src/register/render.rs                the dry-run JSON (Task 5)
    src/register/sync.rs                  the sync flow (Task 5)
    src/register/export.rs                the CSV (Task 6)
    tests/common/mod.rs                   + TestDb::register_url (Task 5)
    tests/common/register.rs              + run_fau_register (Task 5)
    tests/register_apply.rs               loader (Task 1), municipality parity (Task 2), school parity (Task 3)
    tests/register_runs.rs                bookkeeping (Task 4)
    tests/register_cli.rs                 end to end (Task 5)
    tests/register_export.rs              golden CSV (Task 6)
docs/app-foundation-operations.md         + the register commands (Task 6)
```

---

### Task 1: The testkit feature and the snapshot loader

**Files:**
- Modify: `backend/crates/domain/Cargo.toml`
- Modify: `backend/crates/domain/src/register/sync/mod.rs:9-10`
- Modify: `backend/crates/domain/src/register/sync/testkit.rs` (module doc, and every `pub(super) fn`)
- Modify: `backend/crates/persistence/src/lib.rs:8`
- Create: `backend/crates/persistence/src/register/mod.rs`
- Create: `backend/crates/persistence/src/register/error.rs`
- Create: `backend/crates/persistence/src/register/sql.rs`
- Create: `backend/crates/persistence/src/register/snapshot.rs`
- Modify: `backend/crates/app/Cargo.toml` (`[dev-dependencies]`)
- Test: `backend/crates/app/tests/register_apply.rs` (new)

**Interfaces:**
- Consumes: `fau_domain::register::sync::{RegisterSnapshot, MunicipalitySnapshot, SchoolSnapshot,
  SchoolAttributes, MunicipalityStatus, MunicipalitySource, Origin, Verification, SchoolStatus,
  Ownership}` and `fau_domain::register::source::OfficialName`; `crate::pool::safe_error_kind`.
- Produces:
  - `fau_domain::register::sync::testkit` (feature `testkit`): every builder of part 3 (`ts`, `at`,
    `record`, `municipality`, `change`, `unit`, `inputs`, `address`, `fixture_records`, `hosle`, `ntg`,
    `lerberg`, `signo`, `lorenskog_adult`, `wang`, `gran_canaria`, `longyearbyen`, `kjolsdalen`,
    `halsa`, `haltdalen`, `stange_old`, `stange_new`, `fixture_units`, `school`) and
    `apply(&RegisterSnapshot<u32>, &SyncPlan<u32>) -> (RegisterSnapshot<u32>, BTreeMap<u32, u32>)`,
    now `pub`;
  - `fau_persistence::register::RegisterError` (`Database(String)`, `Decode`, `UnknownRow`,
    `PayloadNotUtf8`; `From<sqlx::Error>`);
  - `fau_persistence::register::load_snapshot(&mut PgConnection) -> Result<RegisterSnapshot<Uuid>, RegisterError>`;
  - `fau_persistence::register::register_is_empty(&mut PgConnection) -> Result<bool, RegisterError>`;
  - crate-private in `register::sql`: `municipality_status`, `municipality_source`, `origin`,
    `verification`, `school_status`, `ownership` (code to enum, `Err(RegisterError::Decode)` otherwise).

- [ ] **Step 1: Expose the testkit behind a feature.** In `backend/crates/domain/Cargo.toml`, insert
  this block between the `[dependencies]` table and `[dev-dependencies]`:

```toml
[features]
# The sync planner's test builders and in-memory applier (`register::sync::testkit`), for
# the persistence parity tests in fau-app. Only ever enabled from a dev-dependency, which
# resolver 2 keeps out of every non-test build, so it never reaches the release binary.
testkit = []
```

In `backend/crates/domain/src/register/sync/mod.rs`, replace

```rust
#[cfg(test)]
mod testkit;
```

with

```rust
#[cfg(any(test, feature = "testkit"))]
pub mod testkit;
```

In `backend/crates/domain/src/register/sync/testkit.rs`, extend the module doc: after the line
`//! that crate, so they are written out by hand.` add

```rust
//!
//! Compiled for the domain's own tests and, behind the `testkit` feature, for fau-app's
//! persistence parity tests (part 4), which run [`apply`] next to the SQL applier. Never part
//! of a release build.
```

Then make all 25 builders public and let rustfmt rejoin the signatures it can now fit on one line:

```bash
cd /workspace/backend
sed -i 's/^pub(super) fn /pub fn /' crates/domain/src/register/sync/testkit.rs
grep -c '^pub(super)' crates/domain/src/register/sync/testkit.rs   # expect 0
cargo fmt
```

- [ ] **Step 2: Enable it for fau-app's tests only.** In `backend/crates/app/Cargo.toml`, add as the
  first entry of `[dev-dependencies]`:

```toml
# The sync planner's test builders and in-memory applier, for the SQL applier's parity
# tests (tests/register_apply.rs). A dev-dependency feature: resolver 2 keeps it out of
# `cargo build --release`, so it never reaches the image.
fau-domain = { path = "../domain", features = ["testkit"] }
```

- [ ] **Step 3: Write the failing tests.** Create `backend/crates/app/tests/register_apply.rs`:

```rust
//! The register's persistence (#3441 part 4): the snapshot loader and the SQL applier,
//! against real PostgreSQL as `fau_register`, checked for parity with the planner's in-memory
//! test applier (`fau_domain::register::sync::testkit::apply`).

mod common;

use common::TestDb;
use fau_domain::register::source::OfficialName;
use fau_domain::register::sync::{
    MunicipalitySnapshot, MunicipalitySource, MunicipalityStatus, Origin, Ownership,
    RegisterSnapshot, SchoolAttributes, SchoolSnapshot, SchoolStatus, Verification,
};
use fau_persistence::register::{load_snapshot, register_is_empty};
use sqlx::PgPool;
use uuid::Uuid;

const BAERUM: Uuid = Uuid::from_u128(0x0199_0000_0000_7000_8000_0000_0000_3201);
const OLD_ASKER: Uuid = Uuid::from_u128(0x0199_0000_0000_7000_8000_0000_0000_0220);
const HOSLE: Uuid = Uuid::from_u128(0x0199_0000_0000_7000_8000_0000_0001_0001);
const SUBMITTED: Uuid = Uuid::from_u128(0x0199_0000_0000_7000_8000_0000_0001_0002);

async fn exec(admin: &PgPool, sql: &str) {
    sqlx::raw_sql(sql)
        .execute(admin)
        .await
        .unwrap_or_else(|e| panic!("fixture SQL failed: {e}"));
}

/// Every column the snapshot reads, with an empty string and a null side by side, an old
/// number next to the current one, a dissolved municipality, a live and a closed tenant, and
/// one row of each history table.
async fn fixture(admin: &PgPool) {
    exec(
        admin,
        "insert into municipalities (id, name, official_name, county_number, county_name, slug,
                                     status, dissolved_on, source, search_text)
         values ('01990000-0000-7000-8000-000000003201', 'Bærum', 'Bærum', '32', 'Akershus',
                 '3201-baerum', 'active', null, 'kartverket', 'bærum baerum barum'),
                ('01990000-0000-7000-8000-000000000220', 'Asker', null, '02', 'Akershus',
                 '0220-asker', 'dissolved', '2020-01-01', 'kartverket', 'asker');
         insert into municipality_names (municipality_id, name, language, priority)
         values ('01990000-0000-7000-8000-000000003201', 'Bærum', 'no', 1);
         insert into municipality_numbers (municipality_id, number, valid_from, valid_until)
         values ('01990000-0000-7000-8000-000000003201', '3024', '2020-01-01', '2024-01-01'),
                ('01990000-0000-7000-8000-000000003201', '3201', '2024-01-01', null),
                ('01990000-0000-7000-8000-000000000220', '0220', '1838-01-01', '2020-01-01');
         insert into municipality_slug_history (slug, municipality_id, valid_from, valid_until)
         values ('3024-baerum', '01990000-0000-7000-8000-000000003201',
                 '2020-01-01T00:00:00Z', '2024-01-01T00:00:00Z');
         insert into schools (id, municipality_id, origin, display_name, register_name,
                              display_name_curated, slug, verification, orgnr, ownership,
                              grade_from, grade_to, register_language, website, street_address,
                              postcode, post_town, in_scope, scope_override, status, search_text)
         values ('01990000-0000-7000-8000-000000010001', '01990000-0000-7000-8000-000000003201',
                 'register', 'Hosle', 'Hosle skole', true, 'hosle', 'listed', '974552124',
                 'public', 1, 7, 'nb', '', 'Bispeveien 73', '1362', null, true, false, 'active',
                 'hosle'),
                ('01990000-0000-7000-8000-000000010002', '01990000-0000-7000-8000-000000003201',
                 'submitted', 'Nyskolen', null, false, null, 'pending', null, null,
                 null, null, null, null, null, null, null, true, null, 'active', 'nyskolen');
         insert into school_slug_history (municipality_id, slug, school_id, valid_from, valid_until)
         values ('01990000-0000-7000-8000-000000003201', 'hosle-skole',
                 '01990000-0000-7000-8000-000000010001',
                 '2024-01-01T00:00:00Z', '2025-01-01T00:00:00Z');
         insert into school_orgnr_history (orgnr, school_id, valid_from, valid_until)
         values ('975270920', '01990000-0000-7000-8000-000000010001',
                 '2020-01-01T00:00:00Z', '2024-01-01T00:00:00Z');
         insert into tenants (id, name, status, school_id)
         values ('01990000-0000-7000-8000-000000020001', 'Hosle FAU', 'pending',
                 '01990000-0000-7000-8000-000000010001'),
                ('01990000-0000-7000-8000-000000020002', 'Gammelt FAU', 'closed',
                 '01990000-0000-7000-8000-000000010002');",
    )
    .await;
}

#[tokio::test]
async fn the_snapshot_reads_every_column_back_exactly() {
    let db = TestDb::migrated().await;
    fixture(&db.admin_pool()).await;
    let pool = db.register_pool().await;
    let mut conn = pool.acquire().await.unwrap();

    let snapshot = load_snapshot(&mut conn).await.unwrap();

    assert_eq!(
        snapshot,
        RegisterSnapshot {
            municipalities: vec![
                MunicipalitySnapshot {
                    id: OLD_ASKER,
                    number: "0220".into(),
                    name: "Asker".into(),
                    official_name: None,
                    county_number: "02".into(),
                    county_name: "Akershus".into(),
                    slug: "0220-asker".into(),
                    status: MunicipalityStatus::Dissolved,
                    source: MunicipalitySource::Kartverket,
                    names: vec![],
                },
                MunicipalitySnapshot {
                    id: BAERUM,
                    number: "3201".into(),
                    name: "Bærum".into(),
                    official_name: Some("Bærum".into()),
                    county_number: "32".into(),
                    county_name: "Akershus".into(),
                    slug: "3201-baerum".into(),
                    status: MunicipalityStatus::Active,
                    source: MunicipalitySource::Kartverket,
                    names: vec![OfficialName {
                        name: "Bærum".into(),
                        language: "no".into(),
                        priority: 1,
                    }],
                },
            ],
            schools: vec![
                SchoolSnapshot {
                    id: HOSLE,
                    municipality_id: BAERUM,
                    origin: Origin::Register,
                    orgnr: Some("974552124".into()),
                    register_name: Some("Hosle skole".into()),
                    display_name: "Hosle".into(),
                    display_name_curated: true,
                    slug: Some("hosle".into()),
                    verification: Verification::Listed,
                    status: SchoolStatus::Active,
                    in_scope: true,
                    scope_override: Some(false),
                    has_live_fau: true,
                    attributes: SchoolAttributes {
                        ownership: Some(Ownership::Public),
                        grade_from: Some(1),
                        grade_to: Some(7),
                        language: Some("nb".into()),
                        website: Some(String::new()),
                        street_address: Some("Bispeveien 73".into()),
                        postcode: Some("1362".into()),
                        post_town: None,
                    },
                },
                SchoolSnapshot {
                    id: SUBMITTED,
                    municipality_id: BAERUM,
                    origin: Origin::Submitted,
                    orgnr: None,
                    register_name: None,
                    display_name: "Nyskolen".into(),
                    display_name_curated: false,
                    slug: None,
                    verification: Verification::Pending,
                    status: SchoolStatus::Active,
                    in_scope: true,
                    scope_override: None,
                    has_live_fau: false,
                    attributes: SchoolAttributes::default(),
                },
            ],
            school_orgnr_history: vec![("975270920".into(), HOSLE)],
            school_slug_history: vec![(BAERUM, "hosle-skole".into(), HOSLE)],
            municipality_slug_history: vec![("3024-baerum".into(), BAERUM)],
        }
    );
}

#[tokio::test]
async fn an_empty_register_is_empty_until_it_holds_a_municipality() {
    let db = TestDb::migrated().await;
    let pool = db.register_pool().await;
    let mut conn = pool.acquire().await.unwrap();
    assert!(register_is_empty(&mut conn).await.unwrap());
    assert_eq!(
        load_snapshot(&mut conn).await.unwrap(),
        RegisterSnapshot::default()
    );

    common::register::test_municipality(&db.admin_pool()).await;
    assert!(!register_is_empty(&mut conn).await.unwrap());
}
```

- [ ] **Step 4: Run the tests to verify they fail.**

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test -p fau-app --test register_apply`
Expected: FAIL to compile: `error[E0432]: unresolved import` for `fau_persistence::register`.

- [ ] **Step 5: Write the persistence module.** In `backend/crates/persistence/src/lib.rs`, add
  `pub mod register;` after `mod pool;`, so the module list reads:

```rust
mod contract;
mod health;
pub mod membership;
mod migrate;
mod pool;
pub mod register;
```

Create `backend/crates/persistence/src/register/mod.rs`:

```rust
//! The school register's persistence (#3441 part 4, docs/school-register-design.md §4, §5):
//! the snapshot the sync planner reads, the applier that writes its plan in one transaction,
//! and the run bookkeeping around it. Everything here runs as `fau_register` (D9); nothing
//! reads the database clock for a rule, so every function takes the caller's `Moment`.

mod error;
mod snapshot;
mod sql;

pub use error::RegisterError;
pub use snapshot::{load_snapshot, register_is_empty};
```

Create `backend/crates/persistence/src/register/error.rs`:

```rust
//! The register's one error type. Its `Display` is fixed text per variant: never a bound
//! value, a payload, a name or an address.

use crate::pool::safe_error_kind;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegisterError {
    /// `pool::safe_error_kind`'s fixed description: a SQLSTATE or a fixed word.
    #[error("database error ({0})")]
    Database(String),
    /// A value read back from the register did not decode: a bug or a schema drift.
    #[error("a register row did not decode")]
    Decode,
    /// The plan names a row the register does not hold, or a `Ref::New` no op creates.
    #[error("the plan names a row the register does not hold")]
    UnknownRow,
    /// An NSR payload was not UTF-8 JSON, so it cannot be staged as `jsonb`.
    #[error("an NSR payload is not valid UTF-8")]
    PayloadNotUtf8,
}

impl From<sqlx::Error> for RegisterError {
    fn from(e: sqlx::Error) -> Self {
        RegisterError::Database(safe_error_kind(&e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_database_error_carries_only_the_sqlstate() {
        let e: RegisterError = crate::pool::test_support::database_error("42501").into();
        assert_eq!(e, RegisterError::Database("sqlstate 42501".to_owned()));
        assert_eq!(e.to_string(), "database error (sqlstate 42501)");
    }
}
```

Create `backend/crates/persistence/src/register/sql.rs`:

```rust
//! Shared pieces of the register's SQL: the code columns' mapping onto the domain's enums.

use fau_domain::register::sync::{
    MunicipalitySource, MunicipalityStatus, Origin, Ownership, SchoolStatus, Verification,
};

use super::error::RegisterError;

pub(crate) fn municipality_status(code: &str) -> Result<MunicipalityStatus, RegisterError> {
    match code {
        "active" => Ok(MunicipalityStatus::Active),
        "dissolved" => Ok(MunicipalityStatus::Dissolved),
        _ => Err(RegisterError::Decode),
    }
}

pub(crate) fn municipality_source(code: &str) -> Result<MunicipalitySource, RegisterError> {
    match code {
        "kartverket" => Ok(MunicipalitySource::Kartverket),
        "manual" => Ok(MunicipalitySource::Manual),
        _ => Err(RegisterError::Decode),
    }
}

pub(crate) fn origin(code: &str) -> Result<Origin, RegisterError> {
    match code {
        "register" => Ok(Origin::Register),
        "submitted" => Ok(Origin::Submitted),
        _ => Err(RegisterError::Decode),
    }
}

pub(crate) fn verification(code: &str) -> Result<Verification, RegisterError> {
    match code {
        "listed" => Ok(Verification::Listed),
        "pending" => Ok(Verification::Pending),
        "verified" => Ok(Verification::Verified),
        "rejected" => Ok(Verification::Rejected),
        "held" => Ok(Verification::Held),
        _ => Err(RegisterError::Decode),
    }
}

pub(crate) fn school_status(code: &str) -> Result<SchoolStatus, RegisterError> {
    match code {
        "active" => Ok(SchoolStatus::Active),
        "closed" => Ok(SchoolStatus::Closed),
        _ => Err(RegisterError::Decode),
    }
}

pub(crate) fn ownership(code: Option<&str>) -> Result<Option<Ownership>, RegisterError> {
    match code {
        None => Ok(None),
        Some("public") => Ok(Some(Ownership::Public)),
        Some("private") => Ok(Some(Ownership::Private)),
        Some(_) => Err(RegisterError::Decode),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_decodes_and_an_unknown_one_does_not() {
        assert_eq!(verification("held"), Ok(Verification::Held));
        assert_eq!(verification("Held"), Err(RegisterError::Decode));
        assert_eq!(ownership(None), Ok(None));
        assert_eq!(ownership(Some("private")), Ok(Some(Ownership::Private)));
        assert_eq!(
            municipality_source("manual"),
            Ok(MunicipalitySource::Manual)
        );
        assert_eq!(school_status("closed"), Ok(SchoolStatus::Closed));
        assert_eq!(origin("submitted"), Ok(Origin::Submitted));
        assert_eq!(
            municipality_status("dissolved"),
            Ok(MunicipalityStatus::Dissolved)
        );
    }
}
```

Create `backend/crates/persistence/src/register/snapshot.rs`:

```rust
//! The register as the planner sees it (docs/school-register-design.md §5.2; the part 3
//! handover's "Building the snapshot"). Every query orders by id, so a plan and its diff are
//! reproducible, and every attribute reads back exactly as stored: an empty string stays an
//! empty string and a null stays `None`, or every run would report `UpdateAttributes`.

use std::collections::BTreeMap;

use fau_domain::register::source::OfficialName;
use fau_domain::register::sync::{
    MunicipalitySnapshot, RegisterSnapshot, SchoolAttributes, SchoolSnapshot,
};
use sqlx::PgConnection;
use uuid::Uuid;

use super::error::RegisterError;
use super::sql;

#[derive(sqlx::FromRow)]
struct MunicipalityRow {
    id: Uuid,
    number: Option<String>,
    name: String,
    official_name: Option<String>,
    county_number: String,
    county_name: String,
    slug: String,
    status: String,
    source: String,
}

#[derive(sqlx::FromRow)]
struct SchoolRow {
    id: Uuid,
    municipality_id: Uuid,
    origin: String,
    orgnr: Option<String>,
    register_name: Option<String>,
    display_name: String,
    display_name_curated: bool,
    slug: Option<String>,
    verification: String,
    status: String,
    in_scope: bool,
    scope_override: Option<bool>,
    has_live_fau: bool,
    ownership: Option<String>,
    grade_from: Option<i16>,
    grade_to: Option<i16>,
    register_language: Option<String>,
    website: Option<String>,
    street_address: Option<String>,
    postcode: Option<String>,
    post_town: Option<String>,
}

/// Reads the whole register. Call it inside a REPEATABLE READ transaction, so its six queries
/// see one snapshot.
///
/// A municipality's `number` is its current one (`valid_until is null`). A dissolved
/// municipality has none, so it carries its most recent number; the planner never resolves a
/// dissolved row's number.
pub async fn load_snapshot(
    conn: &mut PgConnection,
) -> Result<RegisterSnapshot<Uuid>, RegisterError> {
    let municipality_rows: Vec<MunicipalityRow> = sqlx::query_as(
        "select m.id,
                (select n.number from municipality_numbers n
                  where n.municipality_id = m.id
                  order by n.valid_until is null desc, n.valid_from desc
                  limit 1) as number,
                m.name, m.official_name, m.county_number, m.county_name, m.slug, m.status,
                m.source
           from municipalities m
          order by m.id",
    )
    .fetch_all(&mut *conn)
    .await?;

    let name_rows: Vec<(Uuid, String, String, i32)> = sqlx::query_as(
        "select municipality_id, name, language, priority from municipality_names
          order by municipality_id, priority, language, name",
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut names: BTreeMap<Uuid, Vec<OfficialName>> = BTreeMap::new();
    for (id, name, language, priority) in name_rows {
        names.entry(id).or_default().push(OfficialName {
            name,
            language,
            priority: u8::try_from(priority).map_err(|_| RegisterError::Decode)?,
        });
    }

    let mut municipalities = Vec::with_capacity(municipality_rows.len());
    for r in municipality_rows {
        municipalities.push(MunicipalitySnapshot {
            id: r.id,
            number: r.number.ok_or(RegisterError::Decode)?,
            name: r.name,
            official_name: r.official_name,
            county_number: r.county_number,
            county_name: r.county_name,
            slug: r.slug,
            status: sql::municipality_status(&r.status)?,
            source: sql::municipality_source(&r.source)?,
            names: names.remove(&r.id).unwrap_or_default(),
        });
    }

    let school_rows: Vec<SchoolRow> = sqlx::query_as(
        "select s.id, s.municipality_id, s.origin, s.orgnr, s.register_name, s.display_name,
                s.display_name_curated, s.slug, s.verification, s.status, s.in_scope,
                s.scope_override,
                exists (select 1 from tenants t
                         where t.school_id = s.id and t.status in ('pending', 'active'))
                  as has_live_fau,
                s.ownership, s.grade_from, s.grade_to, s.register_language, s.website,
                s.street_address, s.postcode, s.post_town
           from schools s
          order by s.id",
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut schools = Vec::with_capacity(school_rows.len());
    for r in school_rows {
        schools.push(SchoolSnapshot {
            id: r.id,
            municipality_id: r.municipality_id,
            origin: sql::origin(&r.origin)?,
            orgnr: r.orgnr,
            register_name: r.register_name,
            display_name: r.display_name,
            display_name_curated: r.display_name_curated,
            slug: r.slug,
            verification: sql::verification(&r.verification)?,
            status: sql::school_status(&r.status)?,
            in_scope: r.in_scope,
            scope_override: r.scope_override,
            has_live_fau: r.has_live_fau,
            attributes: SchoolAttributes {
                ownership: sql::ownership(r.ownership.as_deref())?,
                grade_from: r.grade_from,
                grade_to: r.grade_to,
                language: r.register_language,
                website: r.website,
                street_address: r.street_address,
                postcode: r.postcode,
                post_town: r.post_town,
            },
        });
    }

    let school_orgnr_history: Vec<(String, Uuid)> = sqlx::query_as(
        "select orgnr, school_id from school_orgnr_history order by school_id, orgnr",
    )
    .fetch_all(&mut *conn)
    .await?;
    let school_slug_history: Vec<(Uuid, String, Uuid)> = sqlx::query_as(
        "select municipality_id, slug, school_id from school_slug_history
          order by school_id, valid_from, municipality_id, slug",
    )
    .fetch_all(&mut *conn)
    .await?;
    let municipality_slug_history: Vec<(String, Uuid)> = sqlx::query_as(
        "select slug, municipality_id from municipality_slug_history
          order by municipality_id, valid_from, slug",
    )
    .fetch_all(&mut *conn)
    .await?;

    Ok(RegisterSnapshot {
        municipalities,
        schools,
        school_orgnr_history,
        school_slug_history,
        municipality_slug_history,
    })
}

/// No municipality and no school: what `--seed` requires and a plain sync refuses.
pub async fn register_is_empty(conn: &mut PgConnection) -> Result<bool, RegisterError> {
    Ok(sqlx::query_scalar(
        "select not exists (select 1 from municipalities) and not exists (select 1 from schools)",
    )
    .fetch_one(&mut *conn)
    .await?)
}
```

- [ ] **Step 6: Run the tests to verify they pass.**

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test -p fau-app --test register_apply`
Expected: PASS, 2 tests (`the_snapshot_reads_every_column_back_exactly`,
`an_empty_register_is_empty_until_it_holds_a_municipality`).

Run: `cd /workspace/backend && cargo test -p fau-persistence register`
Expected: PASS, 2 unit tests (`a_database_error_carries_only_the_sqlstate`,
`every_code_decodes_and_an_unknown_one_does_not`).

- [ ] **Step 7: Check that the release build never sees the feature.**

Run: `cd /workspace/backend && cargo tree -p fau-app -e normal -f '{p} [{f}]' -i fau-domain | head -1`
Expected: a line starting `fau-domain v0.1.0` and ending in `[]` (no `testkit`). With `-e normal,dev`
instead, the same line ends in `[testkit]`.

- [ ] **Step 8: The full checks.** Run the four Global Constraints commands. Expected: all clean;
  the domain's 196 unit tests still pass with the testkit now public.

- [ ] **Step 9: Commit.**

```bash
cd /workspace/backend
git add crates/domain/Cargo.toml crates/domain/src/register/sync/mod.rs \
  crates/domain/src/register/sync/testkit.rs crates/persistence/src/lib.rs \
  crates/persistence/src/register/ crates/app/Cargo.toml crates/app/tests/register_apply.rs
GIT_AUTHOR_NAME="Erik W. Bjønnes" GIT_AUTHOR_EMAIL=erik@ewb-solutions.as \
GIT_COMMITTER_NAME="Erik W. Bjønnes" GIT_COMMITTER_EMAIL=erik@ewb-solutions.as \
git commit -m "Load the register snapshot and share the planner's testkit (#3441)" \
  -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: The SQL applier for municipality ops

**Files:**
- Create: `backend/crates/persistence/src/register/apply.rs`
- Modify: `backend/crates/persistence/src/register/sql.rs` (whole file)
- Modify: `backend/crates/persistence/src/register/mod.rs` (whole file)
- Test: `backend/crates/app/tests/register_apply.rs` (imports, then new helpers and tests appended)

**Interfaces:**
- Consumes: Task 1's `load_snapshot`, `RegisterError`; `fau_domain::register::sync::{plan,
  MunicipalityOp, SyncPlan, Counts}`, `fau_domain::register::search::search_text`,
  `testkit::{inputs, apply, record, change, fixture_records, gran_canaria}`.
- Produces:
  - `fau_persistence::register::apply_plan(conn: &mut PgConnection, plan: &SyncPlan<Uuid>, at: Moment) -> Result<AppliedCounts, RegisterError>`
    (this task applies `municipality_ops`; Task 3 replaces the body to add school ops and reviews);
  - `fau_persistence::register::AppliedCounts { counts: Counts, ops: usize }` with
    `wrote_nothing(&self) -> bool` (Task 3 adds two fields);
  - crate-private: `sql::date_param(Date) -> String`, `sql::ts_param(Timestamp) -> String`;
  - in `register_apply.rs`, the parity helpers every later task's tests use: `relabel`, `to_u32`,
    `school_keys`, `canonical`, `successor_links`, `at`, `step` (returns
    `(SyncOutcome<Uuid>, Option<AppliedCounts>)`), `applied`, `text`, `utc`.

**How a municipality op maps to SQL** (the Handover): `Create` inserts the row with `search_text`,
its number valid from the run's Oslo date, and its names. `Renumber` writes the old slug to history
(guarded), sets the new slug and, when `name` is `Some`, the name, then closes the old number and
opens the new one on the same date. `Rename` writes history only when the slug changes. `UpdateDetails`
replaces `official_name`, the county and every `municipality_names` row. Every touched municipality's
`search_text` is recomputed at the end. The history row's `valid_from` is the municipality's last
history row's `valid_until`, else its `created_at`, and the row is skipped when `valid_from` is not
before `valid_until`.

- [ ] **Step 1: Write the failing tests.** In `backend/crates/app/tests/register_apply.rs`, replace
  everything from `mod common;` down to, but not including, the `const BAERUM` line with:

```rust
mod common;

use std::collections::{BTreeSet, HashMap};

use common::TestDb;
use fau_domain::register::source::{CodeChange, MunicipalityRecord, NsrUnit, OfficialName};
use fau_domain::register::sync::testkit::{self, change, fixture_records, gran_canaria, record};
use fau_domain::register::sync::{
    plan, MunicipalitySnapshot, MunicipalitySource, MunicipalityStatus, Origin, Ownership,
    RegisterSnapshot, RowId, RunKind, SchoolAttributes, SchoolSnapshot, SchoolStatus, SyncOutcome,
    SyncPlan, Verification,
};
use fau_domain::time::Moment;
use fau_persistence::register::{apply_plan, load_snapshot, register_is_empty, AppliedCounts};
use jiff::civil::date;
use sqlx::PgPool;
use uuid::Uuid;
```

Then append to the end of the file, after a blank line:

```rust
/// `s` with every id relabelled: `municipality` for a municipality's id, `school` for a
/// school's.
fn relabel<A: RowId, B>(
    s: &RegisterSnapshot<A>,
    municipality: impl Fn(&A) -> B,
    school: impl Fn(&A) -> B,
) -> RegisterSnapshot<B> {
    RegisterSnapshot {
        municipalities: s
            .municipalities
            .iter()
            .map(|m| MunicipalitySnapshot {
                id: municipality(&m.id),
                number: m.number.clone(),
                name: m.name.clone(),
                official_name: m.official_name.clone(),
                county_number: m.county_number.clone(),
                county_name: m.county_name.clone(),
                slug: m.slug.clone(),
                status: m.status,
                source: m.source,
                names: m.names.clone(),
            })
            .collect(),
        schools: s
            .schools
            .iter()
            .map(|x| SchoolSnapshot {
                id: school(&x.id),
                municipality_id: municipality(&x.municipality_id),
                origin: x.origin,
                orgnr: x.orgnr.clone(),
                register_name: x.register_name.clone(),
                display_name: x.display_name.clone(),
                display_name_curated: x.display_name_curated,
                slug: x.slug.clone(),
                verification: x.verification,
                status: x.status,
                in_scope: x.in_scope,
                scope_override: x.scope_override,
                has_live_fau: x.has_live_fau,
                attributes: x.attributes.clone(),
            })
            .collect(),
        school_orgnr_history: s
            .school_orgnr_history
            .iter()
            .map(|(o, id)| (o.clone(), school(id)))
            .collect(),
        school_slug_history: s
            .school_slug_history
            .iter()
            .map(|(m, slug, id)| (municipality(m), slug.clone(), school(id)))
            .collect(),
        municipality_slug_history: s
            .municipality_slug_history
            .iter()
            .map(|(slug, m)| (slug.clone(), municipality(m)))
            .collect(),
    }
}

/// The database's register in the planner's test ids: every id, sorted, numbered from 1. The
/// order is kept, so the planner's id tie-breaks agree, and the test applier's `max + 1` ids
/// sort after every existing one, as UUIDv7s do.
fn to_u32(s: &RegisterSnapshot<Uuid>) -> RegisterSnapshot<u32> {
    let mut ids: Vec<Uuid> = s
        .municipalities
        .iter()
        .map(|m| m.id)
        .chain(s.schools.iter().map(|x| x.id))
        .collect();
    ids.sort();
    let map: HashMap<Uuid, u32> = ids.into_iter().zip(1..).collect();
    relabel(s, |id| map[id], |id| map[id])
}

/// A key per school that survives a change of id type: its orgnr, or else its display name.
fn school_keys<Id: RowId>(s: &RegisterSnapshot<Id>) -> HashMap<Id, String> {
    s.schools
        .iter()
        .map(|x| {
            let key = x.orgnr.clone().unwrap_or_else(|| x.display_name.clone());
            (x.id, format!("s{key}"))
        })
        .collect()
}

/// Ids replaced by keys that survive a change of id type (a municipality by its number, a
/// school by [`school_keys`]) and every list sorted, since the two appliers append in
/// different orders.
fn canonical<Id: RowId>(s: &RegisterSnapshot<Id>) -> RegisterSnapshot<String> {
    let m: HashMap<Id, String> = s
        .municipalities
        .iter()
        .map(|m| (m.id, format!("m{}", m.number)))
        .collect();
    let k = school_keys(s);
    let mut out = relabel(s, |id| m[id].clone(), |id| k[id].clone());
    for m in &mut out.municipalities {
        m.names.sort_by(|a, b| {
            (a.priority, &a.language, &a.name).cmp(&(b.priority, &b.language, &b.name))
        });
    }
    out.municipalities.sort_by(|a, b| a.id.cmp(&b.id));
    out.schools.sort_by(|a, b| a.id.cmp(&b.id));
    out.school_orgnr_history.sort();
    out.school_slug_history.sort();
    out.municipality_slug_history.sort();
    out
}

/// Every `schools.successor_id` link, as (closed school, successor) keys.
async fn successor_links(
    conn: &mut sqlx::PgConnection,
    snapshot: &RegisterSnapshot<Uuid>,
) -> BTreeSet<(String, String)> {
    let links: Vec<(Uuid, Uuid)> =
        sqlx::query_as("select id, successor_id from schools where successor_id is not null")
            .fetch_all(&mut *conn)
            .await
            .unwrap();
    let keys = school_keys(snapshot);
    links
        .into_iter()
        .map(|(closed, successor)| (keys[&closed].clone(), keys[&successor].clone()))
        .collect()
}

fn at(rfc3339: &str) -> Moment {
    Moment::at(rfc3339.parse().expect("an RFC 3339 timestamp"))
}

/// One run, both ways: plans the same inputs against the database's register and against its
/// u32 translation, applies the first with the SQL applier (as `fau_register`, in one
/// transaction, at `at`) and the second with the test applier, and asserts that the two
/// registers, and the successor links this run made, then agree, ids aside. Returns the SQL
/// side's outcome, and what its applier reported.
async fn step(
    pool: &PgPool,
    records: &[MunicipalityRecord],
    changes: &[CodeChange],
    units: &[NsrUnit],
    kind: RunKind,
    at: Moment,
) -> (SyncOutcome<Uuid>, Option<AppliedCounts>) {
    let mut tx = pool.begin().await.unwrap();
    let before = load_snapshot(&mut tx).await.unwrap();
    let links_before = successor_links(&mut tx, &before).await;
    let before_u32 = to_u32(&before);
    let inputs = testkit::inputs(records, changes, units, kind);
    let outcome = plan(&before, &inputs);
    let (expected, expected_links, applied) = match (&outcome, plan(&before_u32, &inputs)) {
        (SyncOutcome::Apply(sql_plan), SyncOutcome::Apply(test_plan)) => {
            let applied = apply_plan(&mut tx, sql_plan, at).await.unwrap();
            let (expected, successors) = testkit::apply(&before_u32, &test_plan);
            let keys = school_keys(&expected);
            let links: BTreeSet<(String, String)> = successors
                .iter()
                .map(|(closed, successor)| (keys[closed].clone(), keys[successor].clone()))
                .collect();
            (expected, links, Some(applied))
        }
        (SyncOutcome::NoChange, SyncOutcome::NoChange) => (before_u32, BTreeSet::new(), None),
        (a, b) => panic!("the two plans disagree: {a:?} against {b:?}"),
    };
    tx.commit().await.unwrap();

    let mut conn = pool.acquire().await.unwrap();
    let after = load_snapshot(&mut conn).await.unwrap();
    assert_eq!(canonical(&after), canonical(&expected));
    let links_after = successor_links(&mut conn, &after).await;
    assert_eq!(
        links_after
            .difference(&links_before)
            .cloned()
            .collect::<BTreeSet<_>>(),
        expected_links
    );
    (outcome, applied)
}

fn applied(outcome: SyncOutcome<Uuid>) -> SyncPlan<Uuid> {
    match outcome {
        SyncOutcome::Apply(plan) => plan,
        other => panic!("expected a plan, got {other:?}"),
    }
}

async fn text(admin: &PgPool, sql: &str) -> Vec<String> {
    sqlx::query_scalar(sql).fetch_all(admin).await.unwrap()
}

/// SQL for a timestamptz column as `2026-09-28T02:30:00Z`, whatever the session's time zone.
fn utc(column: &str) -> String {
    format!("to_char({column} at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')")
}

/// Kartverket's fixture municipalities with one change each: Bærum renumbered 3201 to 3299,
/// Heim renamed Heimdal (its official name and names follow), and Stange's official name
/// changed with its Norwegian name kept.
fn changed_records() -> Vec<MunicipalityRecord> {
    fixture_records()
        .into_iter()
        .map(|r| match r.number.as_str() {
            "3201" => record("3299", "Bærum", "32", "Akershus"),
            "5055" => record("5055", "Heimdal", "50", "Trøndelag"),
            "3413" => MunicipalityRecord {
                official_name: "Stange kommune".into(),
                names: vec![OfficialName {
                    name: "Stange kommune".into(),
                    language: "no".into(),
                    priority: 1,
                }],
                ..r
            },
            _ => r,
        })
        .collect()
}

#[tokio::test]
async fn municipality_ops_apply_like_the_test_applier() {
    let db = TestDb::migrated().await;
    let pool = db.register_pool().await;
    let admin = db.admin_pool();
    // Gran Canaria (2599) is out of scope: NSR is not empty, and no school is created.
    let units = [gran_canaria()];

    let seed = applied(
        step(
            &pool,
            &fixture_records(),
            &[],
            &units,
            RunKind::Seed,
            at("2026-09-28T02:30:00Z"),
        )
        .await
        .0,
    );
    assert_eq!(seed.counts.municipalities_created, 9);
    assert_eq!(
        text(
            &admin,
            "select search_text from municipalities where slug = '3201-baerum'"
        )
        .await,
        ["bærum baerum barum"]
    );

    let changes = [change("3201", "Bærum", "3299", "Bærum", date(2027, 1, 1))];
    let sync = applied(
        step(
            &pool,
            &changed_records(),
            &changes,
            &units,
            RunKind::Sync,
            at("2027-01-04T02:30:00Z"),
        )
        .await
        .0,
    );
    assert_eq!(
        (
            sync.counts.renumbered,
            sync.counts.renamed,
            sync.counts.municipalities_updated
        ),
        (1, 1, 2)
    );
    assert_eq!(
        text(
            &admin,
            &format!(
                "select slug || ' ' || {} || ' ' || {}
                   from municipality_slug_history order by slug",
                utc("valid_from"),
                utc("valid_until")
            )
        )
        .await,
        [
            "3201-baerum 2026-09-28T02:30:00Z 2027-01-04T02:30:00Z",
            "5055-heim 2026-09-28T02:30:00Z 2027-01-04T02:30:00Z",
        ]
    );
    assert_eq!(
        text(
            &admin,
            "select n.number || ' ' || n.valid_from || ' ' || coalesce(n.valid_until::text, '-')
               from municipality_numbers n join municipalities m on m.id = n.municipality_id
              where m.name = 'Bærum' order by n.valid_from"
        )
        .await,
        ["3201 2026-09-28 2027-01-01", "3299 2027-01-01 -"]
    );

    // The same sources again: nothing to do, on either side.
    assert_eq!(
        step(
            &pool,
            &changed_records(),
            &changes,
            &units,
            RunKind::Sync,
            at("2027-01-11T02:30:00Z"),
        )
        .await,
        (SyncOutcome::NoChange, None)
    );
}

/// The Handover's guard, which the test applier does not model: two renumbers in one run
/// write no history row for the middle slug, and a number first seen on the day of its
/// renumber closes a day after it opened.
#[tokio::test]
async fn a_middle_slug_writes_no_history_and_a_same_day_renumber_closes_a_day_later() {
    let db = TestDb::migrated().await;
    let pool = db.register_pool().await;
    let admin = db.admin_pool();
    let units = [gran_canaria()];
    let baerum = [record("3201", "Bærum", "32", "Akershus")];
    step(
        &pool,
        &baerum,
        &[],
        &units,
        RunKind::Seed,
        at("2027-01-01T08:00:00Z"),
    )
    .await;

    let changes = [
        change("3201", "Bærum", "3290", "Bærum", date(2027, 1, 1)),
        change("3290", "Bærum", "3291", "Bærum", date(2027, 6, 1)),
    ];
    let mut tx = pool.begin().await.unwrap();
    let before = load_snapshot(&mut tx).await.unwrap();
    let plan = applied(plan(
        &before,
        &testkit::inputs(
            &[record("3291", "Bærum", "32", "Akershus")],
            &changes,
            &units,
            RunKind::Sync,
        ),
    ));
    assert_eq!(plan.counts.renumbered, 2);
    apply_plan(&mut tx, &plan, at("2027-06-07T02:30:00Z"))
        .await
        .unwrap();
    tx.commit().await.unwrap();

    assert_eq!(
        text(
            &admin,
            &format!(
                "select slug || ' ' || {} || ' ' || {} from municipality_slug_history",
                utc("valid_from"),
                utc("valid_until")
            )
        )
        .await,
        ["3201-baerum 2027-01-01T08:00:00Z 2027-06-07T02:30:00Z"]
    );
    assert_eq!(
        text(
            &admin,
            "select number || ' ' || valid_from || ' ' || coalesce(valid_until::text, '-')
               from municipality_numbers order by valid_from"
        )
        .await,
        [
            "3201 2027-01-01 2027-01-02",
            "3290 2027-01-02 2027-06-01",
            "3291 2027-06-01 -",
        ]
    );
    assert_eq!(
        text(&admin, "select slug from municipalities").await,
        ["3291-baerum"]
    );
}
```

The traced values:
- **The seed** creates the nine `fixture_records()`; Gran Canaria's 2599 raises nothing, and no
  school is in scope, so there is no Svalbard. `search_text(["Bærum", "Bærum"])` is
  `"bærum baerum barum"`.
- **The sync** at 2027-01-04: 3201 is gone from Kartverket and SSB says 3201 → 3299 on 2027-01-01, so
  Bærum renumbers (`renumbered: 1`) with no rename and no detail change (3299's record is the same
  Bærum). Heim → Heimdal is a rename, and its official name and names change too
  (`UpdateDetails`). Stange keeps its name, so only `UpdateDetails`. Total: `(1, 1, 2)`.
- **History:** both old slugs are valid from the seed (`created_at` = 2026-09-28T02:30:00Z) to the sync.
  3201 was first seen on the seed's Oslo date, 2026-09-28, and closes on the change's date.
- **The guard test:** the seed is at 08:00 UTC on 2027-01-01, so 3201 opens on 2027-01-01. The first
  renumber is dated that same day, so it closes on 2027-01-02 (`greatest('2027-01-01', '2027-01-01' + 1)`),
  and 3290 opens then. The second renumber's old slug `3290-baerum` would be valid from the first
  history row's `valid_until` (the sync's own instant) to that same instant, so it is skipped.

- [ ] **Step 2: Run the tests to verify they fail.**

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test -p fau-app --test register_apply`
Expected: FAIL to compile: no `apply_plan` or `AppliedCounts` in `fau_persistence::register`.

- [ ] **Step 3: Write the applier.** Replace the whole of
  `backend/crates/persistence/src/register/sql.rs` with:

```rust
//! Shared pieces of the register's SQL: date and timestamp binding (as text, like
//! `membership::sql`, because sqlx 0.8 has no jiff support) and the code columns' mapping onto
//! the domain's enums.

use fau_domain::register::sync::{
    MunicipalitySource, MunicipalityStatus, Origin, Ownership, SchoolStatus, Verification,
};
use jiff::civil::Date;
use jiff::Timestamp;

use super::error::RegisterError;

pub(crate) fn date_param(d: Date) -> String {
    d.to_string()
}

pub(crate) fn ts_param(t: Timestamp) -> String {
    t.to_string()
}

pub(crate) fn municipality_status(code: &str) -> Result<MunicipalityStatus, RegisterError> {
    match code {
        "active" => Ok(MunicipalityStatus::Active),
        "dissolved" => Ok(MunicipalityStatus::Dissolved),
        _ => Err(RegisterError::Decode),
    }
}

pub(crate) fn municipality_source(code: &str) -> Result<MunicipalitySource, RegisterError> {
    match code {
        "kartverket" => Ok(MunicipalitySource::Kartverket),
        "manual" => Ok(MunicipalitySource::Manual),
        _ => Err(RegisterError::Decode),
    }
}

pub(crate) fn origin(code: &str) -> Result<Origin, RegisterError> {
    match code {
        "register" => Ok(Origin::Register),
        "submitted" => Ok(Origin::Submitted),
        _ => Err(RegisterError::Decode),
    }
}

pub(crate) fn verification(code: &str) -> Result<Verification, RegisterError> {
    match code {
        "listed" => Ok(Verification::Listed),
        "pending" => Ok(Verification::Pending),
        "verified" => Ok(Verification::Verified),
        "rejected" => Ok(Verification::Rejected),
        "held" => Ok(Verification::Held),
        _ => Err(RegisterError::Decode),
    }
}

pub(crate) fn school_status(code: &str) -> Result<SchoolStatus, RegisterError> {
    match code {
        "active" => Ok(SchoolStatus::Active),
        "closed" => Ok(SchoolStatus::Closed),
        _ => Err(RegisterError::Decode),
    }
}

pub(crate) fn ownership(code: Option<&str>) -> Result<Option<Ownership>, RegisterError> {
    match code {
        None => Ok(None),
        Some("public") => Ok(Some(Ownership::Public)),
        Some("private") => Ok(Some(Ownership::Private)),
        Some(_) => Err(RegisterError::Decode),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_decodes_and_an_unknown_one_does_not() {
        assert_eq!(verification("held"), Ok(Verification::Held));
        assert_eq!(verification("Held"), Err(RegisterError::Decode));
        assert_eq!(ownership(None), Ok(None));
        assert_eq!(ownership(Some("private")), Ok(Some(Ownership::Private)));
        assert_eq!(
            municipality_source("manual"),
            Ok(MunicipalitySource::Manual)
        );
        assert_eq!(school_status("closed"), Ok(SchoolStatus::Closed));
        assert_eq!(origin("submitted"), Ok(Origin::Submitted));
        assert_eq!(
            municipality_status("dissolved"),
            Ok(MunicipalityStatus::Dissolved)
        );
    }
}
```

Create `backend/crates/persistence/src/register/apply.rs`:

```rust
//! The SQL applier (docs/school-register-design.md §5.2-5.3; the part 3 plan's "Handover to
//! part 4", whose executable form is `fau_domain::register::sync::testkit::apply`). Every op
//! runs as its own statements inside the caller's transaction, so 0004's constraints check
//! the register after each one, exactly as the test applier's `check_invariants` does.

use std::collections::{BTreeSet, HashMap};

use fau_domain::register::search::search_text;
use fau_domain::register::source::OfficialName;
use fau_domain::register::sync::{Counts, MunicipalityOp, SyncPlan};
use fau_domain::time::Moment;
use jiff::civil::Date;
use sqlx::PgConnection;
use uuid::Uuid;

use super::error::RegisterError;
use super::sql::{date_param, ts_param};

/// What an applied plan wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedCounts {
    /// The planner's own counts, as recorded on the run.
    pub counts: Counts,
    /// How many ops ran.
    pub ops: usize,
}

impl AppliedCounts {
    /// Nothing but bookkeeping would be written: the Handover records such a run as
    /// `no_change`, like a `NoChange` outcome.
    pub fn wrote_nothing(&self) -> bool {
        self.ops == 0
    }
}

/// The fresh UUIDv7 of every `Ref::New`, allocated before any op runs, so a `Close` can name
/// a successor that a later `Create` makes. Municipalities first, then schools, each in op
/// order: ids are time-ordered, so new rows sort after every existing one, as the test
/// applier's `max + 1` ids do.
struct NewIds {
    municipalities: HashMap<u32, Uuid>,
}

impl NewIds {
    fn allocate(plan: &SyncPlan<Uuid>) -> Self {
        let municipalities = plan
            .municipality_ops
            .iter()
            .filter_map(|op| match op {
                MunicipalityOp::Create { new, .. } => Some((*new, Uuid::now_v7())),
                _ => None,
            })
            .collect();
        NewIds { municipalities }
    }
}

/// Applies `plan` inside the caller's transaction: every municipality op in order, then every
/// school op in order.
pub async fn apply_plan(
    conn: &mut PgConnection,
    plan: &SyncPlan<Uuid>,
    at: Moment,
) -> Result<AppliedCounts, RegisterError> {
    let ids = NewIds::allocate(plan);
    let mut touched = BTreeSet::new();
    for op in &plan.municipality_ops {
        let id = apply_municipality_op(conn, op, &ids, at).await?;
        touched.insert(id);
    }
    for id in touched {
        refresh_municipality_search_text(conn, id).await?;
    }
    Ok(AppliedCounts {
        counts: plan.counts,
        ops: plan.municipality_ops.len(),
    })
}

/// One municipality op. Returns the id it touched.
async fn apply_municipality_op(
    conn: &mut PgConnection,
    op: &MunicipalityOp<Uuid>,
    ids: &NewIds,
    at: Moment,
) -> Result<Uuid, RegisterError> {
    let now = ts_param(at.now());
    match op {
        MunicipalityOp::Create {
            new,
            number,
            name,
            official_name,
            county_number,
            county_name,
            slug,
            names,
            source,
        } => {
            let id = *ids
                .municipalities
                .get(new)
                .ok_or(RegisterError::UnknownRow)?;
            sqlx::query(
                "insert into municipalities
                   (id, name, official_name, county_number, county_name, slug, status, source,
                    search_text, created_at, updated_at)
                 values ($1, $2, $3, $4, $5, $6, 'active', $7, '', $8::timestamptz,
                         $8::timestamptz)",
            )
            .bind(id)
            .bind(name)
            .bind(official_name)
            .bind(county_number)
            .bind(county_name)
            .bind(slug)
            .bind(source.code())
            .bind(&now)
            .execute(&mut *conn)
            .await?;
            // The number's true start is unknown: it is valid from the day we first saw it.
            sqlx::query(
                "insert into municipality_numbers (municipality_id, number, valid_from)
                 values ($1, $2, $3::date)",
            )
            .bind(id)
            .bind(number)
            .bind(date_param(at.today()))
            .execute(&mut *conn)
            .await?;
            insert_names(conn, id, names).await?;
            Ok(id)
        }
        MunicipalityOp::Renumber {
            id,
            from,
            to,
            valid_from,
            name,
            old_slug,
            new_slug,
        } => {
            // A renumber always moves the slug: the new one starts with the new number.
            municipality_slug_history(conn, *id, old_slug, at).await?;
            let updated = sqlx::query(
                "update municipalities
                    set slug = $2, name = coalesce($3, name), updated_at = $4::timestamptz
                  where id = $1",
            )
            .bind(id)
            .bind(new_slug)
            .bind(name)
            .bind(&now)
            .execute(&mut *conn)
            .await?;
            expect_one(updated.rows_affected())?;
            renumber(conn, *id, from, to, *valid_from).await?;
            Ok(*id)
        }
        MunicipalityOp::Rename {
            id,
            name,
            old_slug,
            new_slug,
        } => {
            // A case-only change keeps the slug and writes no history (§6).
            if old_slug != new_slug {
                municipality_slug_history(conn, *id, old_slug, at).await?;
            }
            let updated = sqlx::query(
                "update municipalities set name = $2, slug = $3, updated_at = $4::timestamptz
                  where id = $1",
            )
            .bind(id)
            .bind(name)
            .bind(new_slug)
            .bind(&now)
            .execute(&mut *conn)
            .await?;
            expect_one(updated.rows_affected())?;
            Ok(*id)
        }
        MunicipalityOp::UpdateDetails {
            id,
            official_name,
            county_number,
            county_name,
            names,
        } => {
            let updated = sqlx::query(
                "update municipalities
                    set official_name = $2, county_number = $3, county_name = $4,
                        updated_at = $5::timestamptz
                  where id = $1",
            )
            .bind(id)
            .bind(official_name)
            .bind(county_number)
            .bind(county_name)
            .bind(&now)
            .execute(&mut *conn)
            .await?;
            expect_one(updated.rows_affected())?;
            // Replaced whole: Kartverket may drop a name, and fau_register may delete (0004).
            sqlx::query("delete from municipality_names where municipality_id = $1")
                .bind(id)
                .execute(&mut *conn)
                .await?;
            insert_names(conn, *id, names).await?;
            Ok(*id)
        }
    }
}

async fn insert_names(
    conn: &mut PgConnection,
    id: Uuid,
    names: &[OfficialName],
) -> Result<(), RegisterError> {
    for n in names {
        sqlx::query(
            "insert into municipality_names (municipality_id, name, language, priority)
             values ($1, $2, $3, $4)",
        )
        .bind(id)
        .bind(&n.name)
        .bind(&n.language)
        .bind(i32::from(n.priority))
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// Closes the current number at `valid_from` and opens the new one from the same date. A
/// number first seen on or after the change's date (a seed on the day of a renumber) cannot
/// close before it opened, so it closes the day after it opened instead.
async fn renumber(
    conn: &mut PgConnection,
    id: Uuid,
    from: &str,
    to: &str,
    valid_from: Date,
) -> Result<(), RegisterError> {
    let opened = sqlx::query(
        "with closed as (
           update municipality_numbers
              set valid_until = greatest($3::date, valid_from + 1)
            where municipality_id = $1 and number = $2 and valid_until is null
            returning valid_until)
         insert into municipality_numbers (municipality_id, number, valid_from)
         select $1, $4, valid_until from closed",
    )
    .bind(id)
    .bind(from)
    .bind(date_param(valid_from))
    .bind(to)
    .execute(&mut *conn)
    .await?;
    expect_one(opened.rows_affected())
}

/// Moves `old_slug` into history, valid from the end of the municipality's last history row
/// or else from its creation. Skipped when that interval is empty: nobody saw the slug live,
/// e.g. the middle slug of two renumbers in one run (the Handover's guard).
async fn municipality_slug_history(
    conn: &mut PgConnection,
    id: Uuid,
    old_slug: &str,
    at: Moment,
) -> Result<(), RegisterError> {
    sqlx::query(
        "insert into municipality_slug_history (slug, municipality_id, valid_from, valid_until)
         select $1, m.id,
                coalesce((select max(h.valid_until) from municipality_slug_history h
                           where h.municipality_id = m.id),
                         m.created_at),
                $3::timestamptz
           from municipalities m
          where m.id = $2
            and coalesce((select max(h.valid_until) from municipality_slug_history h
                           where h.municipality_id = m.id),
                         m.created_at) < $3::timestamptz",
    )
    .bind(old_slug)
    .bind(id)
    .bind(ts_param(at.now()))
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// §7: every name a municipality answers to, the Norwegian one first, folded for search.
async fn refresh_municipality_search_text(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<(), RegisterError> {
    let name: String = sqlx::query_scalar("select name from municipalities where id = $1")
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
    let names: Vec<String> = sqlx::query_scalar(
        "select name from municipality_names where municipality_id = $1
          order by priority, language, name",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?;
    let text = search_text(std::iter::once(name.as_str()).chain(names.iter().map(String::as_str)));
    sqlx::query("update municipalities set search_text = $2 where id = $1")
        .bind(id)
        .bind(text)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// An op names exactly one row; anything else is a plan against a register it did not read.
fn expect_one(rows: u64) -> Result<(), RegisterError> {
    if rows == 1 {
        Ok(())
    } else {
        Err(RegisterError::UnknownRow)
    }
}
```

Replace the whole of `backend/crates/persistence/src/register/mod.rs` with:

```rust
//! The school register's persistence (#3441 part 4, docs/school-register-design.md §4, §5):
//! the snapshot the sync planner reads, the applier that writes its plan in one transaction,
//! and the run bookkeeping around it. Everything here runs as `fau_register` (D9); nothing
//! reads the database clock for a rule, so every function takes the caller's `Moment`.

mod apply;
mod error;
mod snapshot;
mod sql;

pub use apply::{apply_plan, AppliedCounts};
pub use error::RegisterError;
pub use snapshot::{load_snapshot, register_is_empty};
```

- [ ] **Step 4: Run the tests to verify they pass.**

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test -p fau-app --test register_apply`
Expected: PASS, 4 tests.

- [ ] **Step 5: The full checks.** Run the four Global Constraints commands. Expected: all clean.

- [ ] **Step 6: Commit.**

```bash
cd /workspace/backend
git add crates/persistence/src/register/ crates/app/tests/register_apply.rs
GIT_AUTHOR_NAME="Erik W. Bjønnes" GIT_AUTHOR_EMAIL=erik@ewb-solutions.as \
GIT_COMMITTER_NAME="Erik W. Bjønnes" GIT_COMMITTER_EMAIL=erik@ewb-solutions.as \
git commit -m "Apply municipality ops in SQL, matching the test applier (#3441)" \
  -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: School ops, reviews with dedup, staging, and the Handover's parity scenarios

**Files:**
- Modify: `backend/crates/persistence/src/register/apply.rs` (whole file)
- Create: `backend/crates/persistence/src/register/school_ops.rs`
- Create: `backend/crates/persistence/src/register/reviews.rs`
- Create: `backend/crates/persistence/src/register/staging.rs`
- Modify: `backend/crates/persistence/src/register/mod.rs` (whole file)
- Test: `backend/crates/app/tests/register_apply.rs` (imports, then new tests appended)

**Interfaces:**
- Consumes: Task 2's `apply_plan`, `AppliedCounts`, the parity helpers; `sql::{date_param, ts_param}`;
  `fau_domain::register::scope::{classify, ScopeDecision}`; `sha2`.
- Produces:
  - `apply_plan` now applies every op, links successors and writes review items;
  - `AppliedCounts { counts: Counts, ops: usize, new_reviews: Vec<NewReview>, deduplicated_reviews: usize }`,
    and `wrote_nothing()` is `ops == 0 && new_reviews.is_empty()`;
  - `fau_persistence::register::NewReview { id: Uuid, kind: ReviewKind }` (`Copy`);
  - `fau_persistence::register::NsrPayload { orgnr, body: Vec<u8>, changed_at: Option<Timestamp>, scope: ScopeDecision }`
    with `NsrPayload::new(&NsrUnit, Vec<u8>)`;
  - `fau_persistence::register::stage_payloads(&mut PgConnection, &[NsrPayload], Moment) -> Result<(), RegisterError>`;
  - crate-private: `apply::{NewIds, expect_one}`, `school_ops::{apply_school_op, link_successor,
    refresh_school_search_text}`, `reviews::write_reviews`.

**How a school op maps to SQL** (the Handover): `Create` inserts origin `register`, the given
verification and no FAU link (and no slug for a held row). `Rename` and `Move` write history only for a
`SlugChange` whose `old` is `Some`; a move's row is keyed on `from`. `Close` moves the current slug to
history, clears it, sets the status, `closed_on` and `closure_reason`, and `in_scope = false` for
`OutOfScope`. Successors are linked after every op. A school's history `valid_from` is its last
history row's `valid_until`, else `verified_at`, else `created_at`, with the same skip as for
municipalities.

- [ ] **Step 1: Write the failing tests.** In `backend/crates/app/tests/register_apply.rs`, replace
  everything from `mod common;` down to, but not including, the `const BAERUM` line with:

```rust
mod common;

use std::collections::{BTreeSet, HashMap};

use common::TestDb;
use fau_domain::register::source::{
    CodeChange, MunicipalityRecord, NsrClosure, NsrUnit, OfficialName,
};
use fau_domain::register::sync::testkit::{
    self, change, fixture_records, fixture_units, gran_canaria, hosle, lerberg, ntg, record,
    stange_new, stange_old,
};
use fau_domain::register::sync::{
    plan, MunicipalitySnapshot, MunicipalitySource, MunicipalityStatus, Origin, Ownership,
    RegisterSnapshot, ReviewKind, RowId, RunKind, SchoolAttributes, SchoolSnapshot, SchoolStatus,
    SyncOutcome, SyncPlan, Verification,
};
use fau_domain::time::Moment;
use fau_persistence::register::{
    apply_plan, load_snapshot, register_is_empty, stage_payloads, AppliedCounts, NsrPayload,
};
use jiff::civil::date;
use sqlx::PgPool;
use uuid::Uuid;
```

Then append to the end of the file, after a blank line:

```rust
/// 99 unchanged listed schools in 3201 Bærum, "Skole 0" to "Skole 98": enough that one
/// close or rename stays under the circuit breaker (§5.3).
fn bystanders() -> Vec<NsrUnit> {
    (0..99)
        .map(|i| testkit::unit(&format!("9{i:08}"), &format!("Skole {i}"), "3201"))
        .collect()
}

fn with_bystanders(units: Vec<NsrUnit>) -> Vec<NsrUnit> {
    units.into_iter().chain(bystanders()).collect()
}

/// The Handover's scenario: a seed, then a rename, then a re-registration, then the same
/// inputs again, each checked against the test applier.
#[tokio::test]
async fn seed_rename_reregistration_then_nothing_match_the_test_applier() {
    let db = TestDb::migrated().await;
    let pool = db.register_pool().await;
    let admin = db.admin_pool();
    let records = fixture_records();
    // Stange ungdomsskole is still open under its old number at the seed.
    let stange_open = NsrUnit {
        is_active: true,
        closure: None,
        ..stange_old()
    };
    let seeded: Vec<NsrUnit> = fixture_units()
        .into_iter()
        .filter(|u| u.orgnr != stange_new().orgnr)
        .map(|u| {
            if u.orgnr == stange_open.orgnr {
                stange_open.clone()
            } else {
                u
            }
        })
        .collect();
    let seeded = with_bystanders(seeded);

    let (seed, _) = step(
        &pool,
        &records,
        &[],
        &seeded,
        RunKind::Seed,
        at("2026-09-28T02:30:00Z"),
    )
    .await;
    let seed = applied(seed);
    assert_eq!(
        (
            seed.counts.municipalities_created,
            seed.counts.schools_created
        ),
        (10, 108)
    );

    // Hosle is renamed; NTG loses its website.
    let renamed: Vec<NsrUnit> = seeded
        .iter()
        .cloned()
        .map(|u| match u.orgnr.as_str() {
            "974552124" => NsrUnit {
                name: "Hosle barneskole".into(),
                ..u
            },
            "990672938" => NsrUnit { website: None, ..u },
            _ => u,
        })
        .collect();
    let (rename, _) = step(
        &pool,
        &records,
        &[],
        &renamed,
        RunKind::Sync,
        at("2026-10-05T02:30:00Z"),
    )
    .await;
    let rename = applied(rename);
    assert_eq!(
        (
            rename.counts.schools_renamed,
            rename.counts.schools_updated,
            rename.counts.attribute_losses
        ),
        (1, 1, 1)
    );
    assert_eq!(
        text(
            &admin,
            &format!(
                "select h.slug || ' ' || {} || ' ' || {} || ' ' || s.slug || ' ' || s.search_text
                   from school_slug_history h join schools s on s.id = h.school_id",
                utc("h.valid_from"),
                utc("h.valid_until")
            )
        )
        .await,
        ["hosle-skole 2026-09-28T02:30:00Z 2026-10-05T02:30:00Z hosle-barneskole hosle barneskole"]
    );

    // The old number closes as "Slettet for sammenslåing" and the new one appears.
    let reregistered: Vec<NsrUnit> = renamed
        .iter()
        .filter(|u| u.orgnr != stange_open.orgnr)
        .cloned()
        .chain([stange_old(), stange_new()])
        .collect();
    let (rereg, _) = step(
        &pool,
        &records,
        &[],
        &reregistered,
        RunKind::Sync,
        at("2026-10-12T02:30:00Z"),
    )
    .await;
    let rereg = applied(rereg);
    assert_eq!(
        (rereg.counts.schools_closed, rereg.counts.schools_created),
        (1, 1)
    );
    assert_eq!(
        text(
            &admin,
            "select c.orgnr || ' ' || c.status || ' ' || c.closure_reason || ' ' || c.closed_on
                    || ' ' || coalesce(c.slug, '-') || ' -> ' || s.orgnr || ' ' || s.slug
               from schools c join schools s on s.id = c.successor_id"
        )
        .await,
        ["975270920 closed merged 2024-08-25 - -> 933181995 stange-ungdomsskole"]
    );

    // The same inputs twice: nothing to do.
    for week in ["2026-10-19T02:30:00Z", "2026-10-26T02:30:00Z"] {
        assert_eq!(
            step(&pool, &records, &[], &reregistered, RunKind::Sync, at(week)).await,
            (SyncOutcome::NoChange, None)
        );
    }
}

/// A move, an out-of-scope close, a closure with an FAU, a submission hold and an unknown
/// number in one run; then the same inputs again, whose review items all deduplicate away.
#[tokio::test]
async fn moves_closures_holds_and_reviews_match_the_test_applier() {
    let db = TestDb::migrated().await;
    let pool = db.register_pool().await;
    let admin = db.admin_pool();
    let records = fixture_records();
    step(
        &pool,
        &records,
        &[],
        &with_bystanders(vec![hosle(), ntg(), lerberg()]),
        RunKind::Seed,
        at("2026-09-28T02:30:00Z"),
    )
    .await;
    // Lerberg gets an FAU; someone submits "Ås skole" in Bærum.
    exec(
        &admin,
        "insert into tenants (id, name, status, school_id)
         select '01990000-0000-7000-8000-000000020001', 'Lerberg FAU', 'active', id
           from schools where orgnr = '998516897';
         insert into schools (id, municipality_id, origin, display_name, verification, status,
                              search_text)
         select '01990000-0000-7000-8000-000000030001', m.id, 'submitted', 'Ås skole',
                'pending', 'active', 'ås skole as skole'
           from municipalities m where m.slug = '3201-baerum';",
    )
    .await;

    let units = with_bystanders(vec![
        // Hosle moves to 3314 Øvre Eiker.
        NsrUnit {
            municipality_number: "3314".into(),
            ..hosle()
        },
        // NTG turns into adult education.
        NsrUnit {
            category_ids: vec!["1".into(), "10".into()],
            ..ntg()
        },
        // Lerberg closes, but it has an FAU.
        NsrUnit {
            is_active: false,
            closure: Some(NsrClosure {
                code: "N".into(),
                at: Some(testkit::ts("2026-09-20T10:00:00Z")),
            }),
            ..lerberg()
        },
        testkit::unit("974000001", "Ås skole", "3201"),
        testkit::unit("974000002", "Nordpolen skole", "9999"),
    ]);
    let (first, written) = step(
        &pool,
        &records,
        &[],
        &units,
        RunKind::Sync,
        at("2026-10-05T02:30:00Z"),
    )
    .await;
    let first = applied(first);
    assert_eq!(
        (
            first.counts.schools_moved,
            first.counts.schools_closed,
            first.counts.schools_held,
            first.counts.reviews,
            first.counts.skipped,
        ),
        (1, 1, 1, 3, 1)
    );
    let written = written.expect("applied");
    assert_eq!(
        written
            .new_reviews
            .iter()
            .map(|r| r.kind)
            .collect::<Vec<_>>(),
        [
            ReviewKind::PossibleSubmissionMatch,
            ReviewKind::ClosureWithFau,
            ReviewKind::UnknownMunicipalityNumber
        ]
    );
    assert_eq!(written.deduplicated_reviews, 0);
    assert_eq!(
        text(
            &admin,
            "select s.orgnr || ' ' || s.status || ' ' || coalesce(s.closure_reason, '-') || ' '
                    || s.in_scope || ' ' || s.verification || ' ' || coalesce(s.slug, '-')
                    || ' ' || m.slug
               from schools s join municipalities m on m.id = s.municipality_id
              where s.orgnr in ('974552124', '990672938', '998516897', '974000001')
              order by s.orgnr"
        )
        .await,
        [
            "974000001 active - true held - 3201-baerum",
            "974552124 active - true listed hosle-skole 3314-oevre-eiker",
            "990672938 closed out_of_scope false listed - 3201-baerum",
            "998516897 active - true listed lerberg-skole-og-kompetansesenter 3314-oevre-eiker",
        ]
    );
    assert_eq!(
        text(
            &admin,
            "select m.slug || ' ' || h.slug from school_slug_history h
               join municipalities m on m.id = h.municipality_id
               join schools s on s.id = h.school_id order by s.orgnr"
        )
        .await,
        [
            "3201-baerum hosle-skole",
            "3201-baerum norges-toppidrettsgymnas-ungdomsskole-baerum-as",
        ]
    );
    assert_eq!(
        text(
            &admin,
            "select kind || ' ' || details::text from register_review_items order by kind"
        )
        .await,
        [
            r#"closure_with_fau {"name": "Lerberg skole og kompetansesenter", "orgnr": "998516897", "reason": "closed", "closed_on": "2026-09-20"}"#,
            r#"possible_submission_match {"orgnr": "974000001", "similarity": "1.00", "register_name": "Ås skole", "submitted_name": "Ås skole"}"#,
            r#"unknown_municipality_number {"orgnrs": "974000002", "unit_count": "1", "municipality_number": "9999"}"#,
        ]
    );

    // Again: the closure and the unknown number are raised again, and both deduplicate.
    let (second, written) = step(
        &pool,
        &records,
        &[],
        &units,
        RunKind::Sync,
        at("2026-10-12T02:30:00Z"),
    )
    .await;
    let second = applied(second);
    assert!(second.municipality_ops.is_empty() && second.school_ops.is_empty());
    assert_eq!(second.reviews.len(), 2);
    let written = written.expect("applied");
    assert!(written.wrote_nothing());
    assert_eq!(written.deduplicated_reviews, 2);
    assert_eq!(
        text(&admin, "select count(*)::text from register_review_items").await,
        ["3"]
    );
}

/// Items with null references are told apart by their details: the reason plus the number
/// or old code, and the municipality number.
#[tokio::test]
async fn open_reviews_deduplicate_on_their_discriminating_details() {
    let db = TestDb::migrated().await;
    let pool = db.register_pool().await;
    let admin = db.admin_pool();
    step(
        &pool,
        &fixture_records(),
        &[],
        &with_bystanders(vec![hosle()]),
        RunKind::Seed,
        at("2026-09-28T02:30:00Z"),
    )
    .await;

    // A split of a 1507 nobody holds, a number SSB never explained, and units under two
    // unknown numbers: four items, every one with no school and no municipality.
    let records: Vec<MunicipalityRecord> = fixture_records()
        .into_iter()
        .chain([
            record("1508", "Ålesund", "15", "Møre og Romsdal"),
            record("1580", "Haram", "15", "Møre og Romsdal"),
            record("4699", "Nyby", "46", "Vestland"),
        ])
        .collect();
    let changes = [
        change("1507", "Ålesund", "1508", "Ålesund", date(2024, 1, 1)),
        change("1507", "Ålesund", "1580", "Haram", date(2024, 1, 1)),
    ];
    let mut units = with_bystanders(vec![
        hosle(),
        testkit::unit("974000002", "Nordpolen skole", "9999"),
        testkit::unit("974000003", "Sydpolen skole", "9998"),
    ]);
    let reviews = |applied: Option<AppliedCounts>| {
        let a = applied.expect("applied");
        (a.new_reviews.len(), a.deduplicated_reviews)
    };

    let (_, first) = step(
        &pool,
        &records,
        &changes,
        &units,
        RunKind::Sync,
        at("2026-10-05T02:30:00Z"),
    )
    .await;
    assert_eq!(reviews(first), (4, 0));
    let (_, second) = step(
        &pool,
        &records,
        &changes,
        &units,
        RunKind::Sync,
        at("2026-10-12T02:30:00Z"),
    )
    .await;
    assert_eq!(reviews(second), (0, 4));

    units.push(testkit::unit("974000004", "Månen skole", "9997"));
    let (_, third) = step(
        &pool,
        &records,
        &changes,
        &units,
        RunKind::Sync,
        at("2026-10-19T02:30:00Z"),
    )
    .await;
    assert_eq!(reviews(third), (1, 4));
    assert_eq!(
        text(
            &admin,
            "select kind || ' ' || coalesce(details->>'reason', details->>'municipality_number')
                    || ' ' || coalesce(details->>'old_code', details->>'number', '-')
               from register_review_items order by 1"
        )
        .await,
        [
            "municipality_split_or_merge split 1507",
            "municipality_split_or_merge unknown_number 4699",
            "unknown_municipality_number 9997 -",
            "unknown_municipality_number 9998 -",
            "unknown_municipality_number 9999 -",
        ]
    );
}

#[tokio::test]
async fn payloads_are_staged_with_their_hash_and_scope_and_mark_schools_seen() {
    use sha2::{Digest, Sha256};

    let db = TestDb::migrated().await;
    let pool = db.register_pool().await;
    let admin = db.admin_pool();
    step(
        &pool,
        &fixture_records(),
        &[],
        &[hosle()],
        RunKind::Seed,
        at("2026-09-28T02:30:00Z"),
    )
    .await;

    let first = br#"{"Organisasjonsnummer": "974552124"}"#.to_vec();
    let second = br#"{"Organisasjonsnummer": "974552124", "Navn": "Hosle skole"}"#.to_vec();
    let mut tx = pool.begin().await.unwrap();
    stage_payloads(
        &mut tx,
        &[
            NsrPayload::new(&hosle(), first),
            NsrPayload::new(&gran_canaria(), b"{}".to_vec()),
        ],
        at("2026-10-05T02:30:00Z"),
    )
    .await
    .unwrap();
    stage_payloads(
        &mut tx,
        &[NsrPayload::new(&hosle(), second.clone())],
        at("2026-10-12T02:30:00Z"),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let sha: String = Sha256::digest(&second)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        text(
            &admin,
            &format!(
                "select external_id || ' ' || payload_sha256 || ' ' || in_scope || ' '
                        || scope_reason || ' ' || {} || ' ' || coalesce({}, '-')
                        || ' ' || payload::text
                   from register_source_records order by external_id",
                utc("fetched_at"),
                utc("source_changed_at")
            )
        )
        .await,
        [
            format!(
                r#"974552124 {sha} true in_scope 2026-10-12T02:30:00Z 2026-09-13T01:05:43Z {{"Navn": "Hosle skole", "Organisasjonsnummer": "974552124"}}"#
            ),
            "U90099017 44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a false abroad 2026-10-05T02:30:00Z - {}".to_owned(),
        ]
    );
    assert_eq!(
        text(
            &admin,
            &format!("select {} from schools", utc("last_seen_in_source_at"))
        )
        .await,
        ["2026-10-12T02:30:00Z"]
    );
}
```

The traced values:
- **The seed** has 108 in-scope units: Hosle, NTG, Lerberg, Signo, Longyearbyen, Kjølsdalen, Halsa,
  Haltdalen, Stange under its old number (still open), and 99 bystanders. Lørenskog voksenopplæring
  (adult), Wang (upper secondary) and Gran Canaria (abroad) are out of scope. The municipalities are
  the nine records plus Svalbard: 10.
- **The rename** is 1 of 108 active schools: `1 * 100 > 108 * 5` is false, so no abort. NTG's
  website goes from `Some` to `None`: one update and one attribute loss. The history row runs from the
  seed to the rename, and `search_text(["Hosle barneskole", "Hosle barneskole"])` is `"hosle barneskole"`.
- **The re-registration:** 975270920 closes with code F at 2024-08-25T01:15:10.91Z, which is
  2024-08-25 in Oslo. 933181995 has the same name (similarity 1.00, over 0.6), so it becomes the
  successor, takes the bare slug, and the old school closes as `merged` (§4.5).
- **The second scenario:** Hosle's unit now says 3314, so it moves; its slug `hosle-skole` is free in
  Øvre Eiker and is re-minted as the same text, with the history row under Bærum. NTG turns into adult
  education (category 10) and closes `out_of_scope`, releasing `norges-toppidrettsgymnas-ungdomsskole-baerum-as`.
  Lerberg has an active tenant, so its closure (code N, 2026-09-20 in Oslo) is reviewed, not
  applied. "Ås skole" matches the pending submission with similarity 1.00 and is held with no slug.
  9999 is unknown: one item and one skipped unit. On the second run the held row already exists and
  the planner skips it, so only the closure and the unknown number are raised again, and both
  deduplicate: nothing is written.
- **The dedup scenario:** every item has null references. The split of an unheld 1507 is
  `reason = split, old_code = 1507`; 4699 is Kartverket's unexplained number (`unknown_number`,
  `number = 4699`); 9998 and 9999 are unknown to NSR. 1508 and 1580 are blocked by the split, so they
  raise no `unknown_number` of their own.
- **Staging:** `NsrPayload::new(&gran_canaria(), ..)` classifies as `abroad`. The SHA-256 of `{}` is
  `44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a`; Hosle's is computed in the test.
  PostgreSQL's `jsonb` text orders keys by length, then bytes, so `"Navn"` prints before
  `"Organisasjonsnummer"`, and `review_items.details` print in that order too. Hosle's
  `changed_at` (2026-09-13T01:05:43.46Z) prints to the second.

- [ ] **Step 2: Run the tests to verify they fail.**

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test -p fau-app --test register_apply`
Expected: FAIL to compile: no `stage_payloads` or `NsrPayload` in `fau_persistence::register`.

- [ ] **Step 3: Write the school half.** Replace the whole of
  `backend/crates/persistence/src/register/apply.rs` with:

```rust
//! The SQL applier (docs/school-register-design.md §5.2-5.3; the part 3 plan's "Handover to
//! part 4", whose executable form is `fau_domain::register::sync::testkit::apply`). Every op
//! runs as its own statements inside the caller's transaction, so 0004's constraints check
//! the register after each one, exactly as the test applier's `check_invariants` does.

use std::collections::{BTreeSet, HashMap};

use fau_domain::register::search::search_text;
use fau_domain::register::source::OfficialName;
use fau_domain::register::sync::{Counts, MunicipalityOp, Ref, SchoolOp, SyncPlan};
use fau_domain::time::Moment;
use jiff::civil::Date;
use sqlx::PgConnection;
use uuid::Uuid;

use super::error::RegisterError;
use super::reviews::{write_reviews, NewReview};
use super::school_ops::{apply_school_op, link_successor, refresh_school_search_text};
use super::sql::{date_param, ts_param};

/// What an applied plan wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedCounts {
    /// The planner's own counts, as recorded on the run.
    pub counts: Counts,
    /// How many ops ran.
    pub ops: usize,
    /// The review items written, in plan order.
    pub new_reviews: Vec<NewReview>,
    /// Review items skipped because an open item already has their key.
    pub deduplicated_reviews: usize,
}

impl AppliedCounts {
    /// No op ran and every review deduplicated away: the Handover records such a run as
    /// `no_change`, like a `NoChange` outcome, so the caller rolls it back.
    pub fn wrote_nothing(&self) -> bool {
        self.ops == 0 && self.new_reviews.is_empty()
    }
}

/// The fresh UUIDv7 of every `Ref::New`, allocated before any op runs, so a `Close` can name
/// a successor that a later `Create` makes. Municipalities first, then schools, each in op
/// order: ids are time-ordered, so new rows sort after every existing one, as the test
/// applier's `max + 1` ids do.
pub(super) struct NewIds {
    municipalities: HashMap<u32, Uuid>,
    schools: HashMap<u32, Uuid>,
}

impl NewIds {
    fn allocate(plan: &SyncPlan<Uuid>) -> Self {
        let municipalities = plan
            .municipality_ops
            .iter()
            .filter_map(|op| match op {
                MunicipalityOp::Create { new, .. } => Some((*new, Uuid::now_v7())),
                _ => None,
            })
            .collect();
        let schools = plan
            .school_ops
            .iter()
            .filter_map(|op| match op {
                SchoolOp::Create { new, .. } => Some((*new, Uuid::now_v7())),
                _ => None,
            })
            .collect();
        NewIds {
            municipalities,
            schools,
        }
    }

    pub(super) fn municipality(&self, r: &Ref<Uuid>) -> Result<Uuid, RegisterError> {
        resolve(&self.municipalities, r)
    }

    pub(super) fn school(&self, r: &Ref<Uuid>) -> Result<Uuid, RegisterError> {
        resolve(&self.schools, r)
    }
}

fn resolve(new: &HashMap<u32, Uuid>, r: &Ref<Uuid>) -> Result<Uuid, RegisterError> {
    match r {
        Ref::Existing(id) => Ok(*id),
        Ref::New(n) => new.get(n).copied().ok_or(RegisterError::UnknownRow),
    }
}

/// Applies `plan` inside the caller's transaction, as the Handover orders it: every
/// municipality op, then every school op, each in plan order; then the successor links, since
/// a `Close` can name a school a later `Create` makes; then the review items, deduplicated
/// against open ones.
pub async fn apply_plan(
    conn: &mut PgConnection,
    plan: &SyncPlan<Uuid>,
    at: Moment,
) -> Result<AppliedCounts, RegisterError> {
    let ids = NewIds::allocate(plan);

    let mut touched = BTreeSet::new();
    for op in &plan.municipality_ops {
        touched.insert(apply_municipality_op(conn, op, &ids, at).await?);
    }
    for id in touched {
        refresh_municipality_search_text(conn, id).await?;
    }

    let mut renamed = BTreeSet::new();
    let mut successors = Vec::new();
    for op in &plan.school_ops {
        match op {
            SchoolOp::Rename { id, .. } => {
                renamed.insert(*id);
            }
            SchoolOp::Close {
                id,
                successor: Some(successor),
                ..
            } => successors.push((*id, ids.school(successor)?)),
            _ => {}
        }
        apply_school_op(conn, op, &ids, at).await?;
    }
    for (closed, successor) in successors {
        link_successor(conn, closed, successor).await?;
    }
    for id in renamed {
        refresh_school_search_text(conn, id).await?;
    }

    let (new_reviews, deduplicated_reviews) = write_reviews(conn, &plan.reviews, &ids, at).await?;
    Ok(AppliedCounts {
        counts: plan.counts,
        ops: plan.municipality_ops.len() + plan.school_ops.len(),
        new_reviews,
        deduplicated_reviews,
    })
}

/// One municipality op. Returns the id it touched.
async fn apply_municipality_op(
    conn: &mut PgConnection,
    op: &MunicipalityOp<Uuid>,
    ids: &NewIds,
    at: Moment,
) -> Result<Uuid, RegisterError> {
    let now = ts_param(at.now());
    match op {
        MunicipalityOp::Create {
            new,
            number,
            name,
            official_name,
            county_number,
            county_name,
            slug,
            names,
            source,
        } => {
            let id = ids.municipality(&Ref::New(*new))?;
            sqlx::query(
                "insert into municipalities
                   (id, name, official_name, county_number, county_name, slug, status, source,
                    search_text, created_at, updated_at)
                 values ($1, $2, $3, $4, $5, $6, 'active', $7, '', $8::timestamptz,
                         $8::timestamptz)",
            )
            .bind(id)
            .bind(name)
            .bind(official_name)
            .bind(county_number)
            .bind(county_name)
            .bind(slug)
            .bind(source.code())
            .bind(&now)
            .execute(&mut *conn)
            .await?;
            // The number's true start is unknown: it is valid from the day we first saw it.
            sqlx::query(
                "insert into municipality_numbers (municipality_id, number, valid_from)
                 values ($1, $2, $3::date)",
            )
            .bind(id)
            .bind(number)
            .bind(date_param(at.today()))
            .execute(&mut *conn)
            .await?;
            insert_names(conn, id, names).await?;
            Ok(id)
        }
        MunicipalityOp::Renumber {
            id,
            from,
            to,
            valid_from,
            name,
            old_slug,
            new_slug,
        } => {
            // A renumber always moves the slug: the new one starts with the new number.
            municipality_slug_history(conn, *id, old_slug, at).await?;
            let updated = sqlx::query(
                "update municipalities
                    set slug = $2, name = coalesce($3, name), updated_at = $4::timestamptz
                  where id = $1",
            )
            .bind(id)
            .bind(new_slug)
            .bind(name)
            .bind(&now)
            .execute(&mut *conn)
            .await?;
            expect_one(updated.rows_affected())?;
            renumber(conn, *id, from, to, *valid_from).await?;
            Ok(*id)
        }
        MunicipalityOp::Rename {
            id,
            name,
            old_slug,
            new_slug,
        } => {
            // A case-only change keeps the slug and writes no history (§6).
            if old_slug != new_slug {
                municipality_slug_history(conn, *id, old_slug, at).await?;
            }
            let updated = sqlx::query(
                "update municipalities set name = $2, slug = $3, updated_at = $4::timestamptz
                  where id = $1",
            )
            .bind(id)
            .bind(name)
            .bind(new_slug)
            .bind(&now)
            .execute(&mut *conn)
            .await?;
            expect_one(updated.rows_affected())?;
            Ok(*id)
        }
        MunicipalityOp::UpdateDetails {
            id,
            official_name,
            county_number,
            county_name,
            names,
        } => {
            let updated = sqlx::query(
                "update municipalities
                    set official_name = $2, county_number = $3, county_name = $4,
                        updated_at = $5::timestamptz
                  where id = $1",
            )
            .bind(id)
            .bind(official_name)
            .bind(county_number)
            .bind(county_name)
            .bind(&now)
            .execute(&mut *conn)
            .await?;
            expect_one(updated.rows_affected())?;
            // Replaced whole: Kartverket may drop a name, and fau_register may delete (0004).
            sqlx::query("delete from municipality_names where municipality_id = $1")
                .bind(id)
                .execute(&mut *conn)
                .await?;
            insert_names(conn, *id, names).await?;
            Ok(*id)
        }
    }
}

async fn insert_names(
    conn: &mut PgConnection,
    id: Uuid,
    names: &[OfficialName],
) -> Result<(), RegisterError> {
    for n in names {
        sqlx::query(
            "insert into municipality_names (municipality_id, name, language, priority)
             values ($1, $2, $3, $4)",
        )
        .bind(id)
        .bind(&n.name)
        .bind(&n.language)
        .bind(i32::from(n.priority))
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// Closes the current number at `valid_from` and opens the new one from the same date. A
/// number first seen on or after the change's date (a seed on the day of a renumber) cannot
/// close before it opened, so it closes the day after it opened instead.
async fn renumber(
    conn: &mut PgConnection,
    id: Uuid,
    from: &str,
    to: &str,
    valid_from: Date,
) -> Result<(), RegisterError> {
    let opened = sqlx::query(
        "with closed as (
           update municipality_numbers
              set valid_until = greatest($3::date, valid_from + 1)
            where municipality_id = $1 and number = $2 and valid_until is null
            returning valid_until)
         insert into municipality_numbers (municipality_id, number, valid_from)
         select $1, $4, valid_until from closed",
    )
    .bind(id)
    .bind(from)
    .bind(date_param(valid_from))
    .bind(to)
    .execute(&mut *conn)
    .await?;
    expect_one(opened.rows_affected())
}

/// Moves `old_slug` into history, valid from the end of the municipality's last history row
/// or else from its creation. Skipped when that interval is empty: nobody saw the slug live,
/// e.g. the middle slug of two renumbers in one run (the Handover's guard).
async fn municipality_slug_history(
    conn: &mut PgConnection,
    id: Uuid,
    old_slug: &str,
    at: Moment,
) -> Result<(), RegisterError> {
    sqlx::query(
        "insert into municipality_slug_history (slug, municipality_id, valid_from, valid_until)
         select $1, m.id,
                coalesce((select max(h.valid_until) from municipality_slug_history h
                           where h.municipality_id = m.id),
                         m.created_at),
                $3::timestamptz
           from municipalities m
          where m.id = $2
            and coalesce((select max(h.valid_until) from municipality_slug_history h
                           where h.municipality_id = m.id),
                         m.created_at) < $3::timestamptz",
    )
    .bind(old_slug)
    .bind(id)
    .bind(ts_param(at.now()))
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// §7: every name a municipality answers to, the Norwegian one first, folded for search.
async fn refresh_municipality_search_text(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<(), RegisterError> {
    let name: String = sqlx::query_scalar("select name from municipalities where id = $1")
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
    let names: Vec<String> = sqlx::query_scalar(
        "select name from municipality_names where municipality_id = $1
          order by priority, language, name",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?;
    let text = search_text(std::iter::once(name.as_str()).chain(names.iter().map(String::as_str)));
    sqlx::query("update municipalities set search_text = $2 where id = $1")
        .bind(id)
        .bind(text)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// An op names exactly one row; anything else is a plan against a register it did not read.
pub(super) fn expect_one(rows: u64) -> Result<(), RegisterError> {
    if rows == 1 {
        Ok(())
    } else {
        Err(RegisterError::UnknownRow)
    }
}
```

Create `backend/crates/persistence/src/register/school_ops.rs`:

```rust
//! The school half of the SQL applier: one function per `SchoolOp`, as the Handover and
//! `testkit::apply` define them.

use fau_domain::register::search::search_text;
use fau_domain::register::sync::{ClosureReason, Ref, SchoolAttributes, SchoolOp, SlugChange};
use fau_domain::time::Moment;
use sqlx::PgConnection;
use uuid::Uuid;

use super::apply::{expect_one, NewIds};
use super::error::RegisterError;
use super::sql::{date_param, ts_param};

pub(super) async fn apply_school_op(
    conn: &mut PgConnection,
    op: &SchoolOp<Uuid>,
    ids: &NewIds,
    at: Moment,
) -> Result<(), RegisterError> {
    let now = ts_param(at.now());
    match op {
        SchoolOp::Create {
            new,
            municipality,
            orgnr,
            register_name,
            display_name,
            slug,
            verification,
            in_scope,
            attributes,
            source_changed_at,
        } => {
            // Origin `register`, no FAU link and, for a held row, no slug (the planner never
            // gives it one, and `schools_slug_needs_verification` would refuse it).
            let a = attributes;
            sqlx::query(
                "insert into schools
                   (id, municipality_id, origin, display_name, register_name, slug,
                    verification, orgnr, ownership, grade_from, grade_to, register_language,
                    website, street_address, postcode, post_town, in_scope, status,
                    source_changed_at, last_seen_in_source_at, search_text, created_at,
                    updated_at)
                 values ($1, $2, 'register', $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13,
                         $14, $15, $16, 'active', $17::timestamptz, $18::timestamptz, $19,
                         $18::timestamptz, $18::timestamptz)",
            )
            .bind(ids.school(&Ref::New(*new))?)
            .bind(ids.municipality(municipality)?)
            .bind(display_name)
            .bind(register_name)
            .bind(slug)
            .bind(verification.code())
            .bind(orgnr)
            .bind(a.ownership.map(|o| o.code()))
            .bind(a.grade_from)
            .bind(a.grade_to)
            .bind(&a.language)
            .bind(&a.website)
            .bind(&a.street_address)
            .bind(&a.postcode)
            .bind(&a.post_town)
            .bind(in_scope)
            .bind(source_changed_at.map(ts_param))
            .bind(&now)
            .bind(search_text([display_name.as_str(), register_name.as_str()]))
            .execute(&mut *conn)
            .await?;
        }
        SchoolOp::Rename {
            id,
            register_name,
            display_name,
            slug,
        } => {
            // `old: None` is a slugless school getting its first slug: no history.
            if let Some(SlugChange { old: Some(old), .. }) = slug {
                school_slug_history(conn, *id, None, old, at).await?;
            }
            let updated = sqlx::query(
                "update schools
                    set register_name = $2, display_name = coalesce($3, display_name),
                        slug = coalesce($4, slug), updated_at = $5::timestamptz
                  where id = $1",
            )
            .bind(id)
            .bind(register_name)
            .bind(display_name)
            .bind(slug.as_ref().map(|c| c.new.as_str()))
            .bind(&now)
            .execute(&mut *conn)
            .await?;
            expect_one(updated.rows_affected())?;
        }
        SchoolOp::UpdateAttributes { id, attributes } => {
            update_attributes(conn, *id, attributes, &now).await?;
        }
        SchoolOp::Move { id, from, to, slug } => {
            // The old slug's history row is keyed on the old municipality (§4.2), even when
            // the text stays the same.
            if let Some(SlugChange { old: Some(old), .. }) = slug {
                school_slug_history(conn, *id, Some(*from), old, at).await?;
            }
            let updated = sqlx::query(
                "update schools
                    set municipality_id = $2, slug = coalesce($3, slug),
                        updated_at = $4::timestamptz
                  where id = $1",
            )
            .bind(id)
            .bind(ids.municipality(to)?)
            .bind(slug.as_ref().map(|c| c.new.as_str()))
            .bind(&now)
            .execute(&mut *conn)
            .await?;
            expect_one(updated.rows_affected())?;
        }
        SchoolOp::Close {
            id,
            reason,
            closed_on,
            ..
        } => {
            // The slug goes to history and is cleared, so `schools_slug_current` lets a
            // successor take it (ADR-002 rule 3). The successor is linked after every op.
            let slug: Option<String> = sqlx::query_scalar("select slug from schools where id = $1")
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?
                .ok_or(RegisterError::UnknownRow)?;
            if let Some(slug) = slug {
                school_slug_history(conn, *id, None, &slug, at).await?;
            }
            let updated = sqlx::query(
                "update schools
                    set status = 'closed', closed_on = $2::date, closure_reason = $3,
                        slug = null, in_scope = in_scope and not $4,
                        updated_at = $5::timestamptz
                  where id = $1",
            )
            .bind(id)
            .bind(date_param(*closed_on))
            .bind(reason.code())
            .bind(*reason == ClosureReason::OutOfScope)
            .bind(&now)
            .execute(&mut *conn)
            .await?;
            expect_one(updated.rows_affected())?;
        }
    }
    Ok(())
}

async fn update_attributes(
    conn: &mut PgConnection,
    id: Uuid,
    a: &SchoolAttributes,
    now: &str,
) -> Result<(), RegisterError> {
    let updated = sqlx::query(
        "update schools
            set ownership = $2, grade_from = $3, grade_to = $4, register_language = $5,
                website = $6, street_address = $7, postcode = $8, post_town = $9,
                updated_at = $10::timestamptz
          where id = $1",
    )
    .bind(id)
    .bind(a.ownership.map(|o| o.code()))
    .bind(a.grade_from)
    .bind(a.grade_to)
    .bind(&a.language)
    .bind(&a.website)
    .bind(&a.street_address)
    .bind(&a.postcode)
    .bind(&a.post_town)
    .bind(now)
    .execute(&mut *conn)
    .await?;
    expect_one(updated.rows_affected())
}

/// Moves `slug` into history under `municipality` (the school's current one when `None`),
/// valid from the end of the school's last history row, or else from when it got a slug:
/// `verified_at` for a verified school, `created_at` for one created listed. Skipped when that
/// interval is empty, as for municipalities.
async fn school_slug_history(
    conn: &mut PgConnection,
    id: Uuid,
    municipality: Option<Uuid>,
    slug: &str,
    at: Moment,
) -> Result<(), RegisterError> {
    sqlx::query(
        "insert into school_slug_history (municipality_id, slug, school_id, valid_from, valid_until)
         select coalesce($2, s.municipality_id), $3, s.id, f.valid_from, $4::timestamptz
           from schools s
          cross join lateral (
                select coalesce((select max(h.valid_until) from school_slug_history h
                                  where h.school_id = s.id),
                                s.verified_at, s.created_at) as valid_from) f
          where s.id = $1 and f.valid_from < $4::timestamptz",
    )
    .bind(id)
    .bind(municipality)
    .bind(slug)
    .bind(ts_param(at.now()))
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `schools.successor_id`, set once every `Create` has run.
pub(super) async fn link_successor(
    conn: &mut PgConnection,
    closed: Uuid,
    successor: Uuid,
) -> Result<(), RegisterError> {
    let updated = sqlx::query("update schools set successor_id = $2 where id = $1")
        .bind(closed)
        .bind(successor)
        .execute(&mut *conn)
        .await?;
    expect_one(updated.rows_affected())
}

/// §7: a school is found by its display name and its NSR name.
pub(super) async fn refresh_school_search_text(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<(), RegisterError> {
    let (display_name, register_name): (String, Option<String>) =
        sqlx::query_as("select display_name, register_name from schools where id = $1")
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
    let text = search_text(std::iter::once(display_name.as_str()).chain(register_name.as_deref()));
    sqlx::query("update schools set search_text = $2 where id = $1")
        .bind(id)
        .bind(text)
        .execute(&mut *conn)
        .await?;
    Ok(())
}
```

Create `backend/crates/persistence/src/register/reviews.rs`:

```rust
//! Review items (docs/school-register-design.md §4.3), written with deduplication against
//! open ones. The planner cannot see open items, so without this each weekly run would raise
//! every unresolved item again (the Handover's "Reviews").

use fau_domain::register::sync::{ReviewItem, ReviewKind};
use fau_domain::time::Moment;
use serde_json::{Map, Value};
use sqlx::PgConnection;
use uuid::Uuid;

use super::apply::NewIds;
use super::error::RegisterError;
use super::sql::ts_param;

/// A review item this run wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewReview {
    pub id: Uuid,
    pub kind: ReviewKind,
}

/// The part of `details` that tells apart items sharing a kind and null references: the
/// municipality number for `unknown_municipality_number`, and the reason plus the number or
/// old code for `municipality_split_or_merge`. `(municipality_number, reason, number_or_code)`.
fn discriminator<Id>(item: &ReviewItem<Id>) -> (Option<&str>, Option<&str>, Option<&str>) {
    let detail = |key: &str| {
        item.details
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value.as_str())
    };
    match item.kind {
        ReviewKind::UnknownMunicipalityNumber => (detail("municipality_number"), None, None),
        ReviewKind::MunicipalitySplitOrMerge => (
            None,
            detail("reason"),
            detail("number").or_else(|| detail("old_code")),
        ),
        _ => (None, None, None),
    }
}

/// Writes every item no open item already covers. An item is covered when an open item has
/// the same kind, school, other school and municipality, and the same [`discriminator`].
/// Returns the items written, in plan order, and how many were skipped.
pub(super) async fn write_reviews(
    conn: &mut PgConnection,
    items: &[ReviewItem<Uuid>],
    ids: &NewIds,
    at: Moment,
) -> Result<(Vec<NewReview>, usize), RegisterError> {
    let mut written = Vec::new();
    let mut skipped = 0;
    for item in items {
        let school = item.school.as_ref().map(|r| ids.school(r)).transpose()?;
        let other_school = item
            .other_school
            .as_ref()
            .map(|r| ids.school(r))
            .transpose()?;
        let municipality = item
            .municipality
            .as_ref()
            .map(|r| ids.municipality(r))
            .transpose()?;
        let (number, reason, code) = discriminator(item);
        let open: bool = sqlx::query_scalar(
            "select exists (
               select 1 from register_review_items
                where resolved_at is null and kind = $1
                  and school_id is not distinct from $2
                  and other_school_id is not distinct from $3
                  and municipality_id is not distinct from $4
                  and ($5::text is null or details->>'municipality_number' = $5)
                  and ($6::text is null or details->>'reason' = $6)
                  and ($7::text is null
                       or coalesce(details->>'number', details->>'old_code') = $7))",
        )
        .bind(item.kind.code())
        .bind(school)
        .bind(other_school)
        .bind(municipality)
        .bind(number)
        .bind(reason)
        .bind(code)
        .fetch_one(&mut *conn)
        .await?;
        if open {
            skipped += 1;
            continue;
        }
        let details: Map<String, Value> = item
            .details
            .iter()
            .map(|(k, v)| ((*k).to_owned(), Value::from(v.as_str())))
            .collect();
        let id = Uuid::now_v7();
        sqlx::query(
            "insert into register_review_items
               (id, kind, school_id, other_school_id, municipality_id, details, created_at)
             values ($1, $2, $3, $4, $5, $6::jsonb, $7::timestamptz)",
        )
        .bind(id)
        .bind(item.kind.code())
        .bind(school)
        .bind(other_school)
        .bind(municipality)
        .bind(Value::Object(details).to_string())
        .bind(ts_param(at.now()))
        .execute(&mut *conn)
        .await?;
        written.push(NewReview {
            id,
            kind: item.kind,
        });
    }
    Ok((written, skipped))
}
```

Create `backend/crates/persistence/src/register/staging.rs`:

```rust
//! NSR staging (docs/school-register-design.md §4.3, §5.2 step 4): the last payload seen
//! per orgnr, with its SHA-256 and scope, for provenance. NSR only: Brreg payloads carry
//! addresses and are never stored.

use fau_domain::register::scope::{classify, ScopeDecision};
use fau_domain::register::source::NsrUnit;
use fau_domain::time::Moment;
use jiff::Timestamp;
use sha2::{Digest, Sha256};
use sqlx::PgConnection;

use super::error::RegisterError;
use super::sql::ts_param;

/// One fetched NSR detail: the raw bytes the unit was parsed from, and what the register
/// keeps beside them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NsrPayload {
    pub orgnr: String,
    pub body: Vec<u8>,
    pub changed_at: Option<Timestamp>,
    /// The filter's own verdict (§2.3), before any operator override.
    pub scope: ScopeDecision,
}

impl NsrPayload {
    pub fn new(unit: &NsrUnit, body: Vec<u8>) -> Self {
        NsrPayload {
            orgnr: unit.orgnr.clone(),
            body,
            changed_at: unit.changed_at,
            scope: classify(&unit.scope_facts()),
        }
    }
}

/// Upserts every payload by `(source, external_id)`, then marks every school whose orgnr was
/// fetched as seen at `at`. Call it only inside an applied run: a `no_change` run writes
/// nothing but its run row (§5.3).
pub async fn stage_payloads(
    conn: &mut PgConnection,
    payloads: &[NsrPayload],
    at: Moment,
) -> Result<(), RegisterError> {
    let now = ts_param(at.now());
    for p in payloads {
        let json = std::str::from_utf8(&p.body).map_err(|_| RegisterError::PayloadNotUtf8)?;
        let sha: String = Sha256::digest(&p.body)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        sqlx::query(
            "insert into register_source_records
               (source, external_id, payload, payload_sha256, source_changed_at, fetched_at,
                in_scope, scope_reason)
             values ('nsr', $1, $2::jsonb, $3, $4::timestamptz, $5::timestamptz, $6, $7)
             on conflict (source, external_id) do update
               set payload = excluded.payload, payload_sha256 = excluded.payload_sha256,
                   source_changed_at = excluded.source_changed_at,
                   fetched_at = excluded.fetched_at, in_scope = excluded.in_scope,
                   scope_reason = excluded.scope_reason",
        )
        .bind(&p.orgnr)
        .bind(json)
        .bind(sha)
        .bind(p.changed_at.map(ts_param))
        .bind(&now)
        .bind(p.scope == ScopeDecision::InScope)
        .bind(p.scope.code())
        .execute(&mut *conn)
        .await?;
    }
    let orgnrs: Vec<&str> = payloads.iter().map(|p| p.orgnr.as_str()).collect();
    sqlx::query(
        "update schools set last_seen_in_source_at = $1::timestamptz where orgnr = any($2)",
    )
    .bind(&now)
    .bind(&orgnrs)
    .execute(&mut *conn)
    .await?;
    Ok(())
}
```

Replace the whole of `backend/crates/persistence/src/register/mod.rs` with:

```rust
//! The school register's persistence (#3441 part 4, docs/school-register-design.md §4, §5):
//! the snapshot the sync planner reads, the applier that writes its plan in one transaction,
//! and the run bookkeeping around it. Everything here runs as `fau_register` (D9); nothing
//! reads the database clock for a rule, so every function takes the caller's `Moment`.

mod apply;
mod error;
mod reviews;
mod school_ops;
mod snapshot;
mod sql;
mod staging;

pub use apply::{apply_plan, AppliedCounts};
pub use error::RegisterError;
pub use reviews::NewReview;
pub use snapshot::{load_snapshot, register_is_empty};
pub use staging::{stage_payloads, NsrPayload};
```

- [ ] **Step 4: Run the tests to verify they pass.**

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test -p fau-app --test register_apply`
Expected: PASS, 8 tests.

- [ ] **Step 5: Prove the tests can fail.** Each change below must make the named test fail; undo it
  afterwards.
  - In `reviews.rs`, make `UnknownMunicipalityNumber`'s discriminator `(None, None, None)`:
    `open_reviews_deduplicate_on_their_discriminating_details` fails.
  - In `apply.rs`, skip the `link_successor` loop: `seed_rename_reregistration_then_nothing_match_the_test_applier` fails.
  - In `school_ops.rs`, change `in_scope = in_scope and not $4` to `in_scope = in_scope or $4`:
    `moves_closures_holds_and_reviews_match_the_test_applier` fails.

- [ ] **Step 6: The full checks.** Run the four Global Constraints commands. Expected: all clean.

- [ ] **Step 7: Commit.**

```bash
cd /workspace/backend
git add crates/persistence/src/register/ crates/app/tests/register_apply.rs
GIT_AUTHOR_NAME="Erik W. Bjønnes" GIT_AUTHOR_EMAIL=erik@ewb-solutions.as \
GIT_COMMITTER_NAME="Erik W. Bjønnes" GIT_COMMITTER_EMAIL=erik@ewb-solutions.as \
git commit -m "Apply school ops, deduplicate reviews and stage NSR payloads (#3441)" \
  -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Run bookkeeping, the advisory lock, audit and outbox

**Files:**
- Create: `backend/crates/persistence/src/register/runs.rs`
- Modify: `backend/crates/persistence/src/register/sql.rs` (add `from_micros`)
- Modify: `backend/crates/persistence/src/register/mod.rs` (whole file)
- Test: `backend/crates/app/tests/register_runs.rs` (new)

**Interfaces:**
- Consumes: Task 3's `AppliedCounts`, `NewReview`; `sql::ts_param`;
  `fau_domain::membership::rules::EWB_OVERSIGHT_ADDRESS`; `fau_domain::time::oslo_today`;
  `fau_domain::register::sync::{AbortReason, Counts, RunKind}`.
- Produces, all in `fau_persistence::register`:
  - `REGISTER_SYNC_LOCK_ID: i64 = 0x4641_5530_0000_3441`, `SYNC_APPLIED_ACTION: &str = "register.sync_applied"`;
  - `try_lock(&mut PgConnection) -> Result<bool, RegisterError>`, `unlock(&mut PgConnection) -> Result<(), RegisterError>`;
  - `counts_json(&Counts) -> serde_json::Value`, `abort_reason_text(&AbortReason) -> String`
    (`empty_source:nsr`, `mass_change:closes=3,renames=0,attribute_losses=0,active=100`);
  - `start_run(conn, kind: RunKind, dry_run: bool, at: Moment) -> Result<Uuid, RegisterError>`;
  - `record_no_change(conn, run: Uuid, at)`, `record_dry_run(conn, run, counts: &Counts, abort: Option<&AbortReason>, at)`,
    `record_failed(conn, run, reason: &str, at)`, `record_aborted(conn, run, kind, reason: &AbortReason, counts: &Counts, at)`,
    `record_applied(conn, run, kind, applied: &AppliedCounts, at)`, each `-> Result<(), RegisterError>`,
    and each refusing (`Err(UnknownRow)`) a run that is already finished;
  - `seed_date(conn) -> Result<Option<Date>, RegisterError>`;
  - crate-private: `sql::from_micros(i64) -> Result<Timestamp, RegisterError>`.

- [ ] **Step 1: Write the failing tests.** Create `backend/crates/app/tests/register_runs.rs`:

```rust
//! Register run bookkeeping (#3441 part 4): the advisory lock, the run row, the audit entry
//! and the operator's mail, as `fau_register`.

mod common;

use common::TestDb;
use fau_domain::register::sync::testkit::{self, fixture_records, hosle, lerberg};
use fau_domain::register::sync::{plan, AbortReason, Counts, RunKind, SyncOutcome, SyncPlan};
use fau_domain::time::Moment;
use fau_persistence::register::{
    apply_plan, load_snapshot, record_aborted, record_applied, record_dry_run, record_failed,
    record_no_change, seed_date, start_run, try_lock, unlock, RegisterError,
};
use jiff::civil::date;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

fn at(rfc3339: &str) -> Moment {
    Moment::at(rfc3339.parse().expect("an RFC 3339 timestamp"))
}

fn applied(outcome: SyncOutcome<Uuid>) -> SyncPlan<Uuid> {
    match outcome {
        SyncOutcome::Apply(plan) => plan,
        other => panic!("expected a plan, got {other:?}"),
    }
}

/// Plans `units` against the database's register and applies the plan as a run of `kind`
/// at `at`, recording it: start, apply, record, commit, as `fau register sync` does.
async fn run(
    pool: &PgPool,
    units: &[fau_domain::register::source::NsrUnit],
    kind: RunKind,
    at: Moment,
) -> Uuid {
    let mut conn = pool.acquire().await.unwrap();
    let id = start_run(&mut conn, kind, false, at).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    let snapshot = load_snapshot(&mut tx).await.unwrap();
    let plan = applied(plan(
        &snapshot,
        &testkit::inputs(&fixture_records(), &[], units, kind),
    ));
    let counts = apply_plan(&mut tx, &plan, at).await.unwrap();
    record_applied(&mut tx, id, kind, &counts, at)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    id
}

async fn run_row(admin: &PgPool, id: Uuid) -> (String, Option<String>, Value, Option<String>) {
    sqlx::query_as(
        "select kind, outcome, counts, abort_reason from register_sync_runs where id = $1",
    )
    .bind(id)
    .fetch_one(admin)
    .await
    .unwrap()
}

async fn outbox(admin: &PgPool) -> Vec<(Option<Uuid>, String, String, Value)> {
    sqlx::query_as(
        "select tenant_id, template, recipient_email, params from outbox order by created_at, id",
    )
    .fetch_all(admin)
    .await
    .unwrap()
}

fn counts(pairs: &[(&str, u64)]) -> Value {
    let mut v = json!({
        "municipalities_created": 0, "municipalities_updated": 0, "renumbered": 0,
        "renamed": 0, "schools_created": 0, "schools_renamed": 0, "schools_updated": 0,
        "attribute_losses": 0, "schools_moved": 0, "schools_closed": 0, "schools_held": 0,
        "reviews": 0, "skipped": 0,
    });
    for (k, n) in pairs {
        v[*k] = json!(n);
    }
    v
}

#[tokio::test]
async fn one_session_holds_the_lock_until_it_lets_go() {
    let db = TestDb::migrated().await;
    let pool = db.register_pool().await;
    let mut first = pool.acquire().await.unwrap();
    let mut second = pool.acquire().await.unwrap();
    assert!(try_lock(&mut first).await.unwrap());
    assert!(!try_lock(&mut second).await.unwrap(), "held by the first");
    unlock(&mut first).await.unwrap();
    assert!(try_lock(&mut second).await.unwrap());
}

#[tokio::test]
async fn an_applied_seed_records_its_run_one_audit_entry_and_one_summary_mail() {
    let db = TestDb::migrated().await;
    let pool = db.register_pool().await;
    let admin = db.admin_pool();
    let id = run(&pool, &[hosle()], RunKind::Seed, at("2026-09-28T02:30:00Z")).await;

    let mut expected = counts(&[("municipalities_created", 9), ("schools_created", 1)]);
    expected["reviews_written"] = json!(0);
    expected["reviews_deduplicated"] = json!(0);
    assert_eq!(
        run_row(&admin, id).await,
        (
            "seed".into(),
            Some("applied".into()),
            expected.clone(),
            None
        )
    );

    let audit: Vec<(Option<Uuid>, String, String, String, Uuid, Value)> = sqlx::query_as(
        "select tenant_id, actor_kind, action, subject_type, subject_id, params from audit_events",
    )
    .fetch_all(&admin)
    .await
    .unwrap();
    let mut params = expected.clone();
    params["kind"] = json!("seed");
    assert_eq!(
        audit,
        [(
            None,
            "system".into(),
            "register.sync_applied".into(),
            "register_sync_run".into(),
            id,
            params
        )]
    );

    assert_eq!(
        outbox(&admin).await,
        [(
            None,
            "register.seed_summary".into(),
            "fau@ewb-solutions.as".into(),
            json!({ "run_id": id, "counts": expected })
        )]
    );
}

#[tokio::test]
async fn an_applied_sync_mails_each_review_item_it_wrote() {
    let db = TestDb::migrated().await;
    let pool = db.register_pool().await;
    let admin = db.admin_pool();
    run(
        &pool,
        &[hosle(), lerberg()],
        RunKind::Seed,
        at("2026-09-28T02:30:00Z"),
    )
    .await;
    sqlx::query("delete from outbox")
        .execute(&admin)
        .await
        .unwrap();
    // Two units under numbers nobody knows: two review items.
    let units = [
        hosle(),
        lerberg(),
        testkit::unit("974000002", "Nordpolen skole", "9999"),
        testkit::unit("974000003", "Sydpolen skole", "9998"),
    ];
    let id = run(&pool, &units, RunKind::Sync, at("2026-10-05T02:30:00Z")).await;

    let items: Vec<(Uuid, String)> =
        sqlx::query_as("select id, kind from register_review_items order by id")
            .fetch_all(&admin)
            .await
            .unwrap();
    assert_eq!(items.len(), 2);
    let mails = outbox(&admin).await;
    assert_eq!(
        mails,
        items
            .iter()
            .map(|(item, kind)| (
                None,
                "register.review_item".to_owned(),
                "fau@ewb-solutions.as".to_owned(),
                json!({ "run_id": id, "review_item_id": item, "kind": kind })
            ))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        run_row(&admin, id).await.2["reviews_written"],
        json!(2),
        "the run records what it wrote"
    );
}

#[tokio::test]
async fn an_abort_records_its_reason_and_one_mail_and_nothing_else() {
    let db = TestDb::migrated().await;
    let pool = db.register_pool().await;
    let admin = db.admin_pool();
    let mut conn = pool.acquire().await.unwrap();
    let now = at("2026-10-05T02:30:00Z");

    let empty = start_run(&mut conn, RunKind::Seed, false, now)
        .await
        .unwrap();
    record_aborted(
        &mut conn,
        empty,
        RunKind::Seed,
        &AbortReason::EmptySource { source: "nsr" },
        &Counts::default(),
        now,
    )
    .await
    .unwrap();
    let mass = start_run(&mut conn, RunKind::Sync, false, now)
        .await
        .unwrap();
    let mass_counts = Counts {
        schools_closed: 3,
        ..Counts::default()
    };
    record_aborted(
        &mut conn,
        mass,
        RunKind::Sync,
        &AbortReason::MassChange {
            closes: 3,
            renames: 0,
            attribute_losses: 0,
            active: 100,
        },
        &mass_counts,
        now,
    )
    .await
    .unwrap();

    assert_eq!(
        run_row(&admin, empty).await,
        (
            "seed".into(),
            Some("aborted".into()),
            counts(&[]),
            Some("empty_source:nsr".into())
        )
    );
    assert_eq!(
        run_row(&admin, mass).await,
        (
            "sync".into(),
            Some("aborted".into()),
            counts(&[("schools_closed", 3)]),
            Some("mass_change:closes=3,renames=0,attribute_losses=0,active=100".into())
        )
    );
    let to = || "fau@ewb-solutions.as".to_owned();
    assert_eq!(
        outbox(&admin).await,
        [
            (
                None,
                "register.sync_aborted".to_owned(),
                to(),
                json!({ "run_id": empty, "kind": "seed", "reason": "empty_source", "source": "nsr" })
            ),
            (
                None,
                "register.sync_aborted".to_owned(),
                to(),
                json!({
                    "run_id": mass, "kind": "sync", "reason": "mass_change",
                    "closes": 3, "renames": 0, "attribute_losses": 0, "active": 100
                })
            ),
        ]
    );
    let audited: i64 = sqlx::query_scalar("select count(*) from audit_events")
        .fetch_one(&admin)
        .await
        .unwrap();
    assert_eq!(audited, 0, "only an applied run is audited");
}

#[tokio::test]
async fn no_change_dry_run_and_failed_rows_finish_once() {
    let db = TestDb::migrated().await;
    let pool = db.register_pool().await;
    let admin = db.admin_pool();
    let mut conn = pool.acquire().await.unwrap();
    let now = at("2026-10-05T02:30:00Z");

    let quiet = start_run(&mut conn, RunKind::Sync, false, now)
        .await
        .unwrap();
    record_no_change(&mut conn, quiet, now).await.unwrap();
    let dry = start_run(&mut conn, RunKind::Sync, true, now)
        .await
        .unwrap();
    record_dry_run(
        &mut conn,
        dry,
        &Counts {
            schools_created: 2,
            ..Counts::default()
        },
        None,
        now,
    )
    .await
    .unwrap();
    let failed = start_run(&mut conn, RunKind::Sync, false, now)
        .await
        .unwrap();
    record_failed(&mut conn, failed, "source Nsr: Transport", now)
        .await
        .unwrap();

    assert_eq!(
        run_row(&admin, quiet).await,
        ("sync".into(), Some("no_change".into()), counts(&[]), None)
    );
    assert_eq!(
        run_row(&admin, dry).await,
        (
            "dry_run".into(),
            Some("no_change".into()),
            counts(&[("schools_created", 2)]),
            None
        )
    );
    assert_eq!(
        run_row(&admin, failed).await,
        (
            "sync".into(),
            Some("failed".into()),
            json!({}),
            Some("source Nsr: Transport".into())
        )
    );
    assert_eq!(
        record_no_change(&mut conn, quiet, now).await,
        Err(RegisterError::UnknownRow),
        "a finished run is never finished again"
    );
    assert!(outbox(&admin).await.is_empty());
}

#[tokio::test]
async fn the_ssb_lookback_starts_on_the_oslo_date_of_the_first_applied_seed() {
    let db = TestDb::migrated().await;
    let pool = db.register_pool().await;
    let mut conn = pool.acquire().await.unwrap();
    assert_eq!(seed_date(&mut conn).await.unwrap(), None);

    // A dry-run seed and an aborted seed do not count.
    let now = at("2026-09-20T12:00:00Z");
    let dry = start_run(&mut conn, RunKind::Seed, true, now)
        .await
        .unwrap();
    record_dry_run(&mut conn, dry, &Counts::default(), None, now)
        .await
        .unwrap();
    let aborted = start_run(&mut conn, RunKind::Seed, false, now)
        .await
        .unwrap();
    record_aborted(
        &mut conn,
        aborted,
        RunKind::Seed,
        &AbortReason::EmptySource { source: "nsr" },
        &Counts::default(),
        now,
    )
    .await
    .unwrap();
    assert_eq!(seed_date(&mut conn).await.unwrap(), None);

    // 23:30 UTC on 27 September is already the 28th in Oslo.
    run(&pool, &[hosle()], RunKind::Seed, at("2026-09-27T23:30:00Z")).await;
    assert_eq!(seed_date(&mut conn).await.unwrap(), Some(date(2026, 9, 28)));
}

#[tokio::test]
async fn the_runtime_role_cannot_record_a_run() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let mut conn = pool.acquire().await.unwrap();
    assert_eq!(
        start_run(&mut conn, RunKind::Sync, false, at("2026-10-05T02:30:00Z")).await,
        Err(RegisterError::Database("sqlstate 42501".into()))
    );
}
```

The traced values: a seed of `[hosle()]` against `fixture_records()` creates 9 municipalities and 1
school (no Svalbard). The sync's two units under 9999 and 9998 raise two `unknown_municipality_number`
items, so two `register.review_item` mails, in the items' order. `2026-09-27T23:30:00Z` is 01:30 on
the 28th in Oslo (UTC+2 in September). `fau_app` has no privilege on `register_sync_runs`: SQLSTATE
42501.

- [ ] **Step 2: Run the tests to verify they fail.**

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test -p fau-app --test register_runs`
Expected: FAIL to compile: no `record_applied`, `start_run`, `try_lock` (and the rest) in
`fau_persistence::register`.

- [ ] **Step 3: Write the bookkeeping.** In `backend/crates/persistence/src/register/sql.rs`, insert
  after `ts_param`:

```rust
/// A timestamp read back as microseconds since the epoch.
pub(crate) fn from_micros(us: i64) -> Result<Timestamp, RegisterError> {
    Timestamp::from_microsecond(us).map_err(|_| RegisterError::Decode)
}
```

Create `backend/crates/persistence/src/register/runs.rs`:

```rust
//! Run bookkeeping (docs/school-register-design.md §4.3, §5.1-5.3): the advisory lock that
//! keeps a manual run and the CronJob apart, the `register_sync_runs` row that is the alert
//! source (#3442), one audit entry per applied run, and the operator's mail.
//!
//! Audit and mail are global: no tenant, actor `system`, and params that hold ids, codes and
//! counts only. Mail goes to the operational address (§5.3) through 0004's global templates.

use fau_domain::membership::rules::EWB_OVERSIGHT_ADDRESS;
use fau_domain::register::sync::{AbortReason, Counts, RunKind};
use fau_domain::time::{oslo_today, Moment};
use jiff::civil::Date;
use serde_json::{json, Value};
use sqlx::PgConnection;
use uuid::Uuid;

use super::apply::AppliedCounts;
use super::error::RegisterError;
use super::sql::{from_micros, ts_param};

/// The session-level advisory lock every `fau register sync` holds for its whole run. Fixed,
/// and distinct from `MIGRATION_LOCK_ID`.
pub const REGISTER_SYNC_LOCK_ID: i64 = 0x4641_5530_0000_3441;

/// The one audit action of an applied run.
pub const SYNC_APPLIED_ACTION: &str = "register.sync_applied";

/// Takes [`REGISTER_SYNC_LOCK_ID`] for this session without waiting. `false` means another
/// run holds it. The lock lives until [`unlock`] or until the connection closes, so the
/// caller keeps this one connection for the whole run.
pub async fn try_lock(conn: &mut PgConnection) -> Result<bool, RegisterError> {
    Ok(sqlx::query_scalar("select pg_try_advisory_lock($1)")
        .bind(REGISTER_SYNC_LOCK_ID)
        .fetch_one(&mut *conn)
        .await?)
}

pub async fn unlock(conn: &mut PgConnection) -> Result<(), RegisterError> {
    sqlx::query("select pg_advisory_unlock($1)")
        .bind(REGISTER_SYNC_LOCK_ID)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

fn kind_code(kind: RunKind) -> &'static str {
    match kind {
        RunKind::Seed => "seed",
        RunKind::Sync => "sync",
    }
}

/// `register_sync_runs.counts`: every planner count by name.
pub fn counts_json(c: &Counts) -> Value {
    json!({
        "municipalities_created": c.municipalities_created,
        "municipalities_updated": c.municipalities_updated,
        "renumbered": c.renumbered,
        "renamed": c.renamed,
        "schools_created": c.schools_created,
        "schools_renamed": c.schools_renamed,
        "schools_updated": c.schools_updated,
        "attribute_losses": c.attribute_losses,
        "schools_moved": c.schools_moved,
        "schools_closed": c.schools_closed,
        "schools_held": c.schools_held,
        "reviews": c.reviews,
        "skipped": c.skipped,
    })
}

/// `register_sync_runs.abort_reason`: a code and its numbers, never a name.
pub fn abort_reason_text(reason: &AbortReason) -> String {
    match reason {
        AbortReason::EmptySource { source } => format!("empty_source:{source}"),
        AbortReason::MassChange {
            closes,
            renames,
            attribute_losses,
            active,
        } => format!(
            "mass_change:closes={closes},renames={renames},attribute_losses={attribute_losses},active={active}"
        ),
    }
}

/// Inserts the run's row, unfinished, and returns its id. Written before anything is
/// fetched, so a run that dies part-way leaves a row with no outcome behind.
pub async fn start_run(
    conn: &mut PgConnection,
    kind: RunKind,
    dry_run: bool,
    at: Moment,
) -> Result<Uuid, RegisterError> {
    let id = Uuid::now_v7();
    let code = if dry_run { "dry_run" } else { kind_code(kind) };
    sqlx::query(
        "insert into register_sync_runs (id, kind, started_at) values ($1, $2, $3::timestamptz)",
    )
    .bind(id)
    .bind(code)
    .bind(ts_param(at.now()))
    .execute(&mut *conn)
    .await?;
    Ok(id)
}

async fn finish(
    conn: &mut PgConnection,
    run: Uuid,
    outcome: &str,
    counts: Value,
    abort_reason: Option<&str>,
    at: Moment,
) -> Result<(), RegisterError> {
    let updated = sqlx::query(
        "update register_sync_runs
            set finished_at = $2::timestamptz, outcome = $3, counts = $4::jsonb,
                abort_reason = $5
          where id = $1 and finished_at is null",
    )
    .bind(run)
    .bind(ts_param(at.now()))
    .bind(outcome)
    .bind(counts.to_string())
    .bind(abort_reason)
    .execute(&mut *conn)
    .await?;
    if updated.rows_affected() == 1 {
        Ok(())
    } else {
        Err(RegisterError::UnknownRow)
    }
}

/// §5.3: nothing to do. Writes nothing but the run row.
pub async fn record_no_change(
    conn: &mut PgConnection,
    run: Uuid,
    at: Moment,
) -> Result<(), RegisterError> {
    finish(
        conn,
        run,
        "no_change",
        counts_json(&Counts::default()),
        None,
        at,
    )
    .await
}

/// A dry run changed nothing, so its outcome is `no_change`; its counts and abort reason say
/// what the real run would have done.
pub async fn record_dry_run(
    conn: &mut PgConnection,
    run: Uuid,
    counts: &Counts,
    abort: Option<&AbortReason>,
    at: Moment,
) -> Result<(), RegisterError> {
    let reason = abort.map(abort_reason_text);
    finish(
        conn,
        run,
        "no_change",
        counts_json(counts),
        reason.as_deref(),
        at,
    )
    .await
}

/// A run that failed before it could plan or apply, e.g. a source that did not answer.
/// `reason` is a fixed description, never a response body or a URL.
pub async fn record_failed(
    conn: &mut PgConnection,
    run: Uuid,
    reason: &str,
    at: Moment,
) -> Result<(), RegisterError> {
    finish(conn, run, "failed", json!({}), Some(reason), at).await
}

/// The circuit breaker or an empty source stopped the run (§5.3): the run row and the
/// `register.sync_aborted` mail, and nothing else.
pub async fn record_aborted(
    conn: &mut PgConnection,
    run: Uuid,
    kind: RunKind,
    reason: &AbortReason,
    counts: &Counts,
    at: Moment,
) -> Result<(), RegisterError> {
    let text = abort_reason_text(reason);
    finish(conn, run, "aborted", counts_json(counts), Some(&text), at).await?;
    let mut params = json!({ "run_id": run, "kind": kind_code(kind) });
    match reason {
        AbortReason::EmptySource { source } => {
            params["reason"] = json!("empty_source");
            params["source"] = json!(source);
        }
        AbortReason::MassChange {
            closes,
            renames,
            attribute_losses,
            active,
        } => {
            params["reason"] = json!("mass_change");
            params["closes"] = json!(closes);
            params["renames"] = json!(renames);
            params["attribute_losses"] = json!(attribute_losses);
            params["active"] = json!(active);
        }
    }
    enqueue(conn, "register.sync_aborted", params, at).await
}

/// An applied run, inside the transaction that applied it: the run row, one audit entry, and
/// the mail. A seed sends one `register.seed_summary` with the counts rather than a mail per
/// review item (Erik, 24 September 2026); a sync sends one `register.review_item` per item it
/// wrote.
pub async fn record_applied(
    conn: &mut PgConnection,
    run: Uuid,
    kind: RunKind,
    applied: &AppliedCounts,
    at: Moment,
) -> Result<(), RegisterError> {
    let mut counts = counts_json(&applied.counts);
    counts["reviews_written"] = json!(applied.new_reviews.len());
    counts["reviews_deduplicated"] = json!(applied.deduplicated_reviews);
    finish(conn, run, "applied", counts.clone(), None, at).await?;

    let mut params = counts.clone();
    params["kind"] = json!(kind_code(kind));
    sqlx::query(
        "insert into audit_events
           (id, tenant_id, actor_kind, action, subject_type, subject_id, occurred_at, params)
         values ($1, null, 'system', $2, 'register_sync_run', $3, $4::timestamptz, $5::jsonb)",
    )
    .bind(Uuid::now_v7())
    .bind(SYNC_APPLIED_ACTION)
    .bind(run)
    .bind(ts_param(at.now()))
    .bind(params.to_string())
    .execute(&mut *conn)
    .await?;

    match kind {
        RunKind::Seed => {
            enqueue(
                conn,
                "register.seed_summary",
                json!({ "run_id": run, "counts": counts }),
                at,
            )
            .await
        }
        RunKind::Sync => {
            for review in &applied.new_reviews {
                enqueue(
                    conn,
                    "register.review_item",
                    json!({
                        "run_id": run,
                        "review_item_id": review.id,
                        "kind": review.kind.code(),
                    }),
                    at,
                )
                .await?;
            }
            Ok(())
        }
    }
}

async fn enqueue(
    conn: &mut PgConnection,
    template: &'static str,
    params: Value,
    at: Moment,
) -> Result<(), RegisterError> {
    sqlx::query(
        "insert into outbox (id, tenant_id, template, recipient_email, params, created_at)
         values ($1, null, $2, $3, $4::jsonb, $5::timestamptz)",
    )
    .bind(Uuid::now_v7())
    .bind(template)
    .bind(EWB_OVERSIGHT_ADDRESS)
    .bind(params.to_string())
    .bind(ts_param(at.now()))
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Where the fixed SSB lookback starts (the Handover): the Oslo date of the earliest applied
/// seed. `None` before the register is seeded.
pub async fn seed_date(conn: &mut PgConnection) -> Result<Option<Date>, RegisterError> {
    let started: Option<i64> = sqlx::query_scalar(
        "select (extract(epoch from min(started_at)) * 1000000)::bigint
           from register_sync_runs where kind = 'seed' and outcome = 'applied'",
    )
    .fetch_one(&mut *conn)
    .await?;
    started
        .map(|us| from_micros(us).map(oslo_today))
        .transpose()
}
```

Replace the whole of `backend/crates/persistence/src/register/mod.rs` with:

```rust
//! The school register's persistence (#3441 part 4, docs/school-register-design.md §4, §5):
//! the snapshot the sync planner reads, the applier that writes its plan in one transaction,
//! and the run bookkeeping around it. Everything here runs as `fau_register` (D9); nothing
//! reads the database clock for a rule, so every function takes the caller's `Moment`.

mod apply;
mod error;
mod reviews;
mod runs;
mod school_ops;
mod snapshot;
mod sql;
mod staging;

pub use apply::{apply_plan, AppliedCounts};
pub use error::RegisterError;
pub use reviews::NewReview;
pub use runs::{
    abort_reason_text, counts_json, record_aborted, record_applied, record_dry_run, record_failed,
    record_no_change, seed_date, start_run, try_lock, unlock, REGISTER_SYNC_LOCK_ID,
    SYNC_APPLIED_ACTION,
};
pub use snapshot::{load_snapshot, register_is_empty};
pub use staging::{stage_payloads, NsrPayload};
```

- [ ] **Step 4: Run the tests to verify they pass.**

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test -p fau-app --test register_runs`
Expected: PASS, 7 tests.

- [ ] **Step 5: The full checks.** Run the four Global Constraints commands. Expected: all clean.

- [ ] **Step 6: Commit.**

```bash
cd /workspace/backend
git add crates/persistence/src/register/ crates/app/tests/register_runs.rs
GIT_AUTHOR_NAME="Erik W. Bjønnes" GIT_AUTHOR_EMAIL=erik@ewb-solutions.as \
GIT_COMMITTER_NAME="Erik W. Bjønnes" GIT_COMMITTER_EMAIL=erik@ewb-solutions.as \
git commit -m "Record register runs with their lock, audit entry and mail (#3441)" \
  -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: `fau register sync`, its configuration, and the end-to-end tests

**Files:**
- Modify: `backend/crates/app/Cargo.toml`, and `backend/Cargo.lock` follows
- Modify: `backend/crates/app/src/config.rs` (`log_level` helper, `RegisterConfig`, one unit test)
- Modify: `backend/crates/app/src/telemetry.rs` (`LogStream`, `init_to`)
- Modify: `backend/crates/app/src/main.rs` (`register` subcommand)
- Create: `backend/crates/app/src/register/mod.rs`
- Create: `backend/crates/app/src/register/fetch.rs`
- Create: `backend/crates/app/src/register/render.rs`
- Create: `backend/crates/app/src/register/sync.rs`
- Modify: `backend/crates/app/tests/common/mod.rs` (`TestDb::register_url`)
- Modify: `backend/crates/app/tests/common/register.rs` (`run_fau_register`)
- Test: `backend/crates/app/tests/register_cli.rs` (new)

**Interfaces:**
- Consumes: everything Tasks 1-4 export from `fau_persistence::register`; `fau_register_sources::client::{SourceClient, SourceUrls}`
  (`municipalities`, `code_changes`, `nsr_all_units`, `nsr_unit_with_payload`) and `SourceError`;
  `fau_domain::register::sync::{plan, SyncInputs, SyncOutcome, RunKind}`.
- Produces:
  - `config::RegisterConfig { database_url: Secret<String>, log_level: String, nsr_url, kartverket_url, ssb_url: Option<Url> }`
    with `from_env()`;
  - `telemetry::LogStream { Stdout, Stderr }` and `telemetry::init_to(&str, &'static str, LogStream)`;
  - `register::RegisterCommand::Sync { dry_run: bool, seed: bool }`, `register::Exit`
    (`Done = 0, Failed = 1, Aborted = 3, Locked = 4, Refused = 5`) and
    `register::run(RegisterCommand, &'static str) -> ExitCode`;
  - the test harness: `TestDb::register_url() -> String` and
    `common::register::run_fau_register(args: &[&str], env: &[(&str, &str)]) -> std::process::Output`.

**The flow** (`sync.rs`), on one connection:
1. connect as `REGISTER_DATABASE_URL`; take the lock, or exit 4 with "another run holds the lock";
2. refuse `--seed` on a non-empty register, and a sync on an empty one (exit 5, no run row);
3. insert the run row (kind `seed`, `sync` or `dry_run`);
4. a sync reads the seed date, or fails;
5. load a snapshot for the fetch set, and fetch (Kartverket, SSB for a sync, the NSR list, the details);
6. open a REPEATABLE READ transaction, load the snapshot again, and plan;
7. a dry run prints the plan, rolls back and records its row (exit 0, or 3 on an abort);
8. `NoChange`: roll back, record `no_change`. `Abort`: roll back, record `aborted` and the mail, exit 3;
9. `Apply`: apply; if it wrote nothing, roll back and record `no_change`; otherwise stage the payloads,
   record `applied` (audit and mail), and commit.

Any error after step 3 records `failed` with its fixed text and exits 1.

- [ ] **Step 1: Write the failing tests.** In `backend/crates/app/tests/common/mod.rs`, add this method
  to `impl TestDb`, right after `migration_url`:

```rust
    /// A `REGISTER_DATABASE_URL` for the register role `fau_register`, pointed at this
    /// database. Same caveat as [`TestDb::url`].
    pub fn register_url(&self) -> String {
        self.role_url("fau_register")
    }
```

In `backend/crates/app/tests/common/register.rs`, add `use std::time::Duration;` above
`use sqlx::PgPool;` (with a blank line between them), and append:

```rust

/// Runs `fau register <args>` with a clean environment plus `env`, bounded to a minute, and
/// returns its output.
pub async fn run_fau_register(args: &[&str], env: &[(&str, &str)]) -> std::process::Output {
    let mut cmd = super::fau_command("register", env);
    cmd.args(args).kill_on_drop(true);
    tokio::time::timeout(Duration::from_secs(60), cmd.output())
        .await
        .expect("fau register did not exit within a minute")
        .expect("run fau register")
}
```

(`fau_command` is private to `common`, and `register` is its child module, so it may call it.)

Create `backend/crates/app/tests/register_cli.rs`:

```rust
//! `fau register sync` end to end (#3441 part 4): the real binary, as `fau_register`,
//! against a real test database, with every source URL pointed at an in-process server
//! that serves the recorded fixtures in crates/register-sources/tests/fixtures/. No test
//! calls the network.

mod common;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::{Path, Query};
use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;
use common::register::run_fau_register;
use common::TestDb;
use fau_persistence::register::try_lock;
use serde_json::{json, Value};
use sqlx::PgPool;

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../register-sources/tests/fixtures"
);

/// Every NSR unit with a recorded detail.
const ORGNRS: [&str; 13] = [
    "933181995",
    "974552124",
    "974554682",
    "974795655",
    "975270920",
    "986779795",
    "990672938",
    "998245508",
    "998516897",
    "998666783",
    "998670799",
    "999038182",
    "U90099017",
];

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(format!("{FIXTURES}/{path}")).expect("a recorded fixture")
}

/// NSR's `/v4/enheter` for the recorded units, on one page: each detail cut down to the
/// list model's fields.
fn nsr_list() -> Value {
    let items: Vec<Value> = ORGNRS
        .iter()
        .map(|orgnr| {
            let d: Value =
                serde_json::from_slice(&fixture(&format!("nsr/enhet-{orgnr}.json"))).unwrap();
            json!({
                "Organisasjonsnummer": d["Organisasjonsnummer"],
                "Navn": d["Navn"],
                "Kommunenummer": d["Kommune"]["Kommunenummer"],
                "ErAktiv": d["ErAktiv"],
                "ErSkole": d["ErSkole"],
                "ErGrunnskole": d["ErGrunnskole"],
                "DatoEndret": d["DatoEndret"],
            })
        })
        .collect();
    json!({
        "Sidenummer": 1, "AntallPerSide": 1000, "AntallSider": 1,
        "TotaltAntallEnheter": items.len(), "EnhetListe": items,
    })
}

/// The in-process sources. `empty_nsr` makes NSR answer with an empty list; `ssb_queries`
/// records every SSB `(from, to)`.
struct Sources {
    base: String,
    empty_nsr: Arc<AtomicBool>,
    ssb_queries: Arc<Mutex<Vec<(String, String)>>>,
}

impl Sources {
    async fn start() -> Self {
        let empty_nsr = Arc::new(AtomicBool::new(false));
        let ssb_queries = Arc::new(Mutex::new(Vec::new()));
        let (empty, queries) = (empty_nsr.clone(), ssb_queries.clone());
        let app = Router::new()
            .route(
                "/nsr/v4/enheter",
                get(move || {
                    let empty = empty.load(Ordering::SeqCst);
                    async move {
                        let body = if empty {
                            json!({
                                "Sidenummer": 1, "AntallPerSide": 1000, "AntallSider": 1,
                                "TotaltAntallEnheter": 0, "EnhetListe": [],
                            })
                        } else {
                            nsr_list()
                        };
                        serde_json::to_vec(&body).unwrap()
                    }
                }),
            )
            .route(
                "/nsr/v4/enhet/{orgnr}",
                get(|Path(orgnr): Path<String>| async move {
                    match std::fs::read(format!("{FIXTURES}/nsr/enhet-{orgnr}.json")) {
                        Ok(body) => (StatusCode::OK, body),
                        Err(_) => (StatusCode::NOT_FOUND, Vec::new()),
                    }
                }),
            )
            .route(
                "/kv/fylkerkommuner",
                get(|| async { fixture("kartverket/fylkerkommuner.json") }),
            )
            .route(
                "/ssb/classifications/131/changes",
                get(move |Query(q): Query<HashMap<String, String>>| {
                    queries
                        .lock()
                        .unwrap()
                        .push((q["from"].clone(), q["to"].clone()));
                    async { fixture("ssb/changes-2026.json") }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Sources {
            base: format!("http://{addr}"),
            empty_nsr,
            ssb_queries,
        }
    }

    /// The environment of a run as `database_url`'s role.
    fn env(&self, database_url: &str) -> Vec<(&'static str, String)> {
        vec![
            ("REGISTER_DATABASE_URL", database_url.to_owned()),
            ("REGISTER_NSR_URL", format!("{}/nsr", self.base)),
            ("REGISTER_KARTVERKET_URL", format!("{}/kv", self.base)),
            ("REGISTER_SSB_URL", format!("{}/ssb", self.base)),
            ("LOG_LEVEL", "info".to_owned()),
        ]
    }
}

async fn fau(args: &[&str], env: &[(&'static str, String)]) -> std::process::Output {
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    run_fau_register(args, &env).await
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

async fn count(admin: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(&format!("select count(*) from {table}"))
        .fetch_one(admin)
        .await
        .unwrap()
}

async fn runs(admin: &PgPool) -> Vec<(String, Option<String>, Option<String>)> {
    sqlx::query_as(
        "select kind, outcome, abort_reason from register_sync_runs order by started_at, id",
    )
    .fetch_all(admin)
    .await
    .unwrap()
}

async fn outbox_templates(admin: &PgPool) -> Vec<String> {
    sqlx::query_scalar("select template from outbox order by created_at, id")
        .fetch_all(admin)
        .await
        .unwrap()
}

/// Two addresses from the recorded fixtures (Hosle skole's visiting and postal address).
/// Neither may ever reach a log line or the dry-run output.
const ADDRESSES: [&str; 2] = ["Bispeveien", "Postboks 700"];

#[tokio::test]
async fn a_seed_then_a_sync_of_the_same_sources_changes_nothing() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let sources = Sources::start().await;
    let env = sources.env(&db.register_url());

    let seed = fau(&["sync", "--seed"], &env).await;
    assert_eq!(seed.status.code(), Some(0), "{}", stderr(&seed));
    // 357 Kartverket municipalities and Svalbard; the nine in-scope units; the twelve
    // active grunnskoler staged (the inactive 975270920 is never fetched on a seed).
    assert_eq!(count(&admin, "municipalities").await, 358);
    assert_eq!(count(&admin, "schools").await, 9);
    assert_eq!(count(&admin, "register_source_records").await, 12);
    assert_eq!(count(&admin, "register_review_items").await, 0);
    assert_eq!(count(&admin, "audit_events").await, 1);
    assert_eq!(
        runs(&admin).await,
        [("seed".into(), Some("applied".into()), None)]
    );
    let summary: Value =
        sqlx::query_scalar("select params from outbox where template = 'register.seed_summary'")
            .fetch_one(&admin)
            .await
            .unwrap();
    assert_eq!(
        (
            &summary["counts"]["municipalities_created"],
            &summary["counts"]["schools_created"]
        ),
        (&json!(358), &json!(9))
    );
    assert!(
        sources.ssb_queries.lock().unwrap().is_empty(),
        "a seed reads no SSB changes"
    );
    let hosle: String = sqlx::query_scalar(
        "select m.slug || '/' || s.slug from schools s
           join municipalities m on m.id = s.municipality_id where s.orgnr = '974552124'",
    )
    .fetch_one(&admin)
    .await
    .unwrap();
    assert_eq!(hosle, "3201-baerum/hosle-skole");

    let sync = fau(&["sync"], &env).await;
    assert_eq!(sync.status.code(), Some(0), "{}", stderr(&sync));
    assert_eq!(
        runs(&admin).await,
        [
            ("seed".into(), Some("applied".into()), None),
            ("sync".into(), Some("no_change".into()), None),
        ]
    );
    assert_eq!(outbox_templates(&admin).await, ["register.seed_summary"]);
    // The fixed lookback: from the seed's Oslo date to the sync's.
    let dates: Vec<String> = sqlx::query_scalar(
        "select to_char(started_at at time zone 'Europe/Oslo', 'YYYY-MM-DD')
           from register_sync_runs order by started_at",
    )
    .fetch_all(&admin)
    .await
    .unwrap();
    assert_eq!(
        *sources.ssb_queries.lock().unwrap(),
        [(dates[0].clone(), dates[1].clone())]
    );

    for out in [&seed, &sync] {
        let log = stderr(out);
        assert!(
            log.lines()
                .all(|l| serde_json::from_str::<Value>(l).is_ok()),
            "every log line is JSON: {log}"
        );
        assert!(log.contains("register sync finished"), "{log}");
        for secret in ADDRESSES
            .iter()
            .copied()
            .chain(["fau_register:", "127.0.0.1"])
        {
            assert!(!log.contains(secret), "{secret} reached the log: {log}");
        }
        assert!(out.stdout.is_empty(), "a real run prints nothing on stdout");
    }
}

#[tokio::test]
async fn a_dry_run_prints_the_plan_and_writes_only_its_run_row() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let sources = Sources::start().await;
    let env = sources.env(&db.register_url());

    let dry = fau(&["sync", "--seed", "--dry-run"], &env).await;
    assert_eq!(dry.status.code(), Some(0), "{}", stderr(&dry));
    let printed = String::from_utf8(dry.stdout.clone()).unwrap();
    for address in ADDRESSES {
        assert!(!printed.contains(address), "{address} in the dry run");
    }
    let plan: Value = serde_json::from_str(&printed).expect("the dry run prints JSON");
    assert_eq!(
        (&plan["kind"], &plan["outcome"]),
        (&json!("seed"), &json!("apply"))
    );
    assert_eq!(plan["counts"]["municipalities_created"], json!(358));
    assert_eq!(plan["counts"]["schools_created"], json!(9));
    let hosle: Vec<&Value> = plan["school_ops"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|op| op["orgnr"] == json!("974552124"))
        .collect();
    assert_eq!(hosle.len(), 1);
    assert_eq!(
        (
            &hosle[0]["op"],
            &hosle[0]["slug"],
            &hosle[0]["verification"]
        ),
        (
            &json!("create_school"),
            &json!("hosle-skole"),
            &json!("listed")
        )
    );

    assert_eq!(
        runs(&admin).await,
        [("dry_run".into(), Some("no_change".into()), None)]
    );
    assert_eq!(count(&admin, "municipalities").await, 0);
    assert_eq!(count(&admin, "register_source_records").await, 0);
    assert_eq!(count(&admin, "outbox").await, 0);

    // A dry run leaves the register empty, so the seed itself may still run.
    let seed = fau(&["sync", "--seed"], &env).await;
    assert_eq!(seed.status.code(), Some(0), "{}", stderr(&seed));
}

#[tokio::test]
async fn the_sync_refuses_an_empty_register_a_second_seed_and_a_held_lock() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let sources = Sources::start().await;
    let env = sources.env(&db.register_url());

    let unseeded = fau(&["sync"], &env).await;
    assert_eq!(unseeded.status.code(), Some(5), "{}", stderr(&unseeded));
    assert!(stderr(&unseeded).contains("the register is empty: seed it first with --seed"));
    assert!(runs(&admin).await.is_empty(), "a refusal writes no run row");

    let seed = fau(&["sync", "--seed"], &env).await;
    assert_eq!(seed.status.code(), Some(0), "{}", stderr(&seed));
    let again = fau(&["sync", "--seed"], &env).await;
    assert_eq!(again.status.code(), Some(5), "{}", stderr(&again));
    assert!(stderr(&again).contains("--seed refuses a register that is not empty"));

    let pool = db.register_pool().await;
    let mut holder = pool.acquire().await.unwrap();
    assert!(try_lock(&mut holder).await.unwrap());
    let locked = fau(&["sync"], &env).await;
    assert_eq!(locked.status.code(), Some(4), "{}", stderr(&locked));
    assert!(stderr(&locked).contains("another run holds the lock"));

    assert_eq!(
        runs(&admin).await,
        [("seed".into(), Some("applied".into()), None)]
    );
}

#[tokio::test]
async fn an_empty_nsr_list_aborts_the_seed_and_mails_the_operator() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let sources = Sources::start().await;
    sources.empty_nsr.store(true, Ordering::SeqCst);

    let out = fau(&["sync", "--seed"], &sources.env(&db.register_url())).await;
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    assert_eq!(
        runs(&admin).await,
        [(
            "seed".into(),
            Some("aborted".into()),
            Some("empty_source:nsr".into())
        )]
    );
    assert_eq!(outbox_templates(&admin).await, ["register.sync_aborted"]);
    assert_eq!(count(&admin, "municipalities").await, 0);
}

#[tokio::test]
async fn the_runtime_role_cannot_run_the_sync() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let sources = Sources::start().await;

    let out = fau(&["sync", "--seed"], &sources.env(&db.url())).await;
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(stderr(&out).contains("sqlstate 42501"), "{}", stderr(&out));
    assert!(runs(&admin).await.is_empty());
    assert_eq!(count(&admin, "municipalities").await, 0);
}

#[tokio::test]
async fn configuration_errors_name_the_variable_and_never_its_value() {
    let missing = run_fau_register(&["sync"], &[]).await;
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(
        stderr(&missing),
        "fau: configuration variable REGISTER_DATABASE_URL is missing\n"
    );

    let leaky = run_fau_register(
        &["sync"],
        &[
            ("REGISTER_DATABASE_URL", "postgres://u:p@127.0.0.1:1/fau"),
            (
                "REGISTER_NSR_URL",
                "https://user:banana-sentinel@data-nsr.udir.no",
            ),
        ],
    )
    .await;
    assert_eq!(leaky.status.code(), Some(1));
    assert_eq!(
        stderr(&leaky),
        "fau: configuration variable REGISTER_NSR_URL is not a valid value\n"
    );
}
```

The traced values: the recorded Kartverket file lists 357 municipalities, and Longyearbyen adds
Svalbard: 358. The NSR list carries all 13 recorded units; 12 are `ErAktiv` and `ErGrunnskole` (not
975270920), so 12 details are fetched and staged. Of those, Lørenskog voksenopplæring, Wang and Gran
Canaria are out of scope: 9 schools, as in part 3's `a_seed_creates_exactly_the_in_scope_fixture_units`.
The 2026 SSB file is a boundary adjustment (3118 is still in Kartverket) and name changes whose code
Kartverket still lists, so the sync plans `NoChange`. `fau_app` may run the lock and the emptiness
check (it can read the public register), and fails on the `register_sync_runs` insert with 42501.

Add the config unit test to the `tests` module at the end of `backend/crates/app/src/config.rs`:

```rust
    #[test]
    fn a_source_url_must_be_http_with_a_host_and_no_credentials() {
        assert!(source_url("REGISTER_NSR_URL", "http://127.0.0.1:9/nsr").is_ok());
        assert!(source_url("REGISTER_NSR_URL", "https://data-nsr.udir.no").is_ok());
        for raw in [
            "ftp://data-nsr.udir.no",
            "not a url",
            "https://user:banana-sentinel@data-nsr.udir.no",
            "https://banana-sentinel@data-nsr.udir.no",
        ] {
            let e = source_url("REGISTER_NSR_URL", raw).unwrap_err();
            assert_eq!(
                e.to_string(),
                "configuration variable REGISTER_NSR_URL is not a valid value",
                "{raw}"
            );
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail.**

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test -p fau-app --test register_cli`
Expected: FAIL, 6 tests: each fails its first exit-code assertion, because clap rejects the unknown
`register` subcommand with exit code 2.

Run: `cd /workspace/backend && cargo test -p fau-app --bin fau a_source_url`
Expected: FAIL to compile: `source_url` not found.

- [ ] **Step 3: Dependencies.** In `backend/crates/app/Cargo.toml`, add to `[dependencies]` after
  `fau-persistence`:

```toml
fau-register-sources = { path = "../register-sources" }
```

and after `clap`:

```toml
# The register sync's clock: `Moment::at(Timestamp::now())`.
jiff = { workspace = true }
```

In `[dev-dependencies]`, `jiff` is now a normal dependency, so replace

```toml
# Test-only: dates for the membership fixtures, and an independent SHA-256 to check
# that invitations store the token's hash and never the token.
jiff = { workspace = true }
sha2 = { workspace = true }
```

with

```toml
# Test-only: an independent SHA-256 to check that invitations store the token's hash
# and never the token, and that staged NSR payloads carry their own.
sha2 = { workspace = true }
```

- [ ] **Step 4: Configuration.** In `backend/crates/app/src/config.rs`, `ServeConfig::from_env` and
  `RegisterConfig::from_env` share the `LOG_LEVEL` rule. In `ServeConfig::from_env`, replace the whole
  block from `let log_level_raw = optional("LOG_LEVEL")...` down to and including
  `let log_level = log_level_raw.trim().to_owned();` (its two long comments included) with:

```rust
        let log_level = log_level()?;
```

Insert this function right before `fn validate_public_base_url` (it is the removed block, moved):

```rust
/// `LOG_LEVEL`, validated and trimmed; `info` when unset. Shared by every command that
/// installs the JSON log subscriber.
fn log_level() -> Result<String, ConfigError> {
    let log_level_raw = optional("LOG_LEVEL").unwrap_or_else(|| DEFAULT_LOG_LEVEL.to_owned());
    // Validated as a `LevelFilter` -- not the fuller `tracing_subscriber::EnvFilter`
    // grammar `telemetry::init` builds from it -- because `EnvFilter` treats an
    // unrecognised bare word as a *target* name (matching every level for it)
    // rather than rejecting it, so a typo such as `banana-sentinel`
    // would silently be accepted as "log everything from a crate named
    // banana-sentinel" instead of failing loudly. `LevelFilter::from_str` has no
    // such fallback: it only accepts trace/debug/info/warn/error/off (or 0-5),
    // which is all `LOG_LEVEL` is meant to carry. `parsed` (this module's helper)
    // discards the parse error itself, since `EnvFilter`'s own parse errors quote
    // the offending input, and a configuration value must never reach the output.
    parsed::<LevelFilter>("LOG_LEVEL", &log_level_raw)?;
    // Stored *trimmed*, not the raw value `parsed` validated: `parsed` trims
    // before parsing (`raw.trim().parse()`), so a value like `"info "` passes
    // validation, but `EnvFilter`'s own grammar does not tolerate the same
    // trailing whitespace the same way -- an untrimmed bare word that fails
    // `LevelFilter::from_str` is read by `EnvFilter` as a *target name* enabling
    // only that one bogus target, not as the global default level, which
    // silently turns off ordinary application logging. Trimming here, once,
    // keeps `telemetry::init` (which builds the actual `EnvFilter` from this
    // field) working from the same value that was actually validated.
    Ok(log_level_raw.trim().to_owned())
}
```

Insert this block right before `#[cfg(test)]` (after `impl MigrateConfig`):

```rust
/// `fau register ...` (#3441, docs/school-register-design.md §5.1). It connects as
/// `fau_register` (D9), never as the runtime role. The source URLs default to the public
/// APIs; the overrides exist for tests, which point them at a local server.
#[derive(Debug, Clone)]
pub struct RegisterConfig {
    pub database_url: Secret<String>,
    pub log_level: String,
    pub nsr_url: Option<Url>,
    pub kartverket_url: Option<Url>,
    pub ssb_url: Option<Url>,
}

impl RegisterConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            database_url: postgres_url(
                "REGISTER_DATABASE_URL",
                required("REGISTER_DATABASE_URL")?,
            )?,
            log_level: log_level()?,
            nsr_url: optional_source_url("REGISTER_NSR_URL")?,
            kartverket_url: optional_source_url("REGISTER_KARTVERKET_URL")?,
            ssb_url: optional_source_url("REGISTER_SSB_URL")?,
        })
    }
}

fn optional_source_url(name: &'static str) -> Result<Option<Url>, ConfigError> {
    optional(name).map(|raw| source_url(name, &raw)).transpose()
}

/// An http(s) base URL with a host and no credentials: the sync logs which source failed,
/// and a URL with userinfo must never be able to reach a log line.
fn source_url(name: &'static str, raw: &str) -> Result<Url, ConfigError> {
    let invalid = || err(name, ConfigProblem::Invalid);
    let url = Url::parse(raw.trim()).map_err(|_| invalid())?;
    let credentials = !url.username().is_empty() || url.password().is_some();
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() || credentials {
        return Err(invalid());
    }
    Ok(url)
}
```

- [ ] **Step 5: Logs on stderr.** In `backend/crates/app/src/telemetry.rs`:

Replace the module doc's first sentence after the title,

```rust
//! JSON to stdout is the only log transport this crate has: every line carries a
//! conventional top-level `level` (`TRACE`/`DEBUG`/`INFO`/`WARN`/`ERROR`), which is
```

with

```rust
//! JSON lines are the only log transport this crate has -- on stdout for `serve`, on
//! stderr for the `register` commands ([`LogStream`]) -- and every line carries a
//! conventional top-level `level` (`TRACE`/`DEBUG`/`INFO`/`WARN`/`ERROR`), which is
```

Replace the body of `pub fn init` (keep its doc comment) and add `LogStream` and `init_to` after it:

```rust
pub fn init(log_level: &str, service_version: &'static str) {
    init_to(log_level, service_version, LogStream::Stdout);
}

/// Where [`JsonLineLayer`] writes. `serve` logs to stdout, its only log transport. The
/// `register` commands log to stderr instead, because their stdout is their output: the
/// dry-run plan and the export CSV. The container runtime collects both streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogStream {
    Stdout,
    Stderr,
}

/// As [`init`], writing every line to `stream`.
pub fn init_to(log_level: &str, service_version: &'static str, stream: LogStream) {
    let filter = EnvFilter::try_new(format!(
        "{SQLX_DEFAULT_DIRECTIVE},{REQUEST_SPAN_DIRECTIVE},{log_level}"
    ))
    .expect("LOG_LEVEL was already validated and trimmed by config");

    tracing_subscriber::registry()
        .with(filter)
        .with(JsonLineLayer::new(service_version, stream))
        .init();

    install_panic_hook();
}
```

Replace the layer's doc line, struct and constructor:

```rust
/// Writes one JSON object per event, directly to its [`LogStream`]. See the module doc comment
/// for why this exists instead of `tracing_subscriber::fmt`'s built-in JSON
/// formatter.
struct JsonLineLayer {
    service_version: &'static str,
    stream: LogStream,
}

impl JsonLineLayer {
    fn new(service_version: &'static str, stream: LogStream) -> Self {
        Self {
            service_version,
            stream,
        }
    }
}
```

and, at the end of `on_event`, replace

```rust
        if let Ok(text) = serde_json::to_string(&Value::Object(line)) {
            let mut stdout = std::io::stdout().lock();
            let _ = writeln!(stdout, "{text}");
        }
```

with

```rust
        if let Ok(text) = serde_json::to_string(&Value::Object(line)) {
            let _ = match self.stream {
                LogStream::Stdout => writeln!(std::io::stdout().lock(), "{text}"),
                LogStream::Stderr => writeln!(std::io::stderr().lock(), "{text}"),
            };
        }
```

- [ ] **Step 6: The command.** In `backend/crates/app/src/main.rs`, add `mod register;` after
  `mod readiness;`; add the variant to `enum Command`, after `Migrate,`:

```rust
    /// The school register (#3441): sync it from its public sources.
    Register {
        #[command(subcommand)]
        command: register::RegisterCommand,
    },
```

and in `main`'s `match cli.command`, add the arm after `Some(Command::Migrate) => run(migrate),` and
update the `None` message:

```rust
        Some(Command::Register { command }) => register::run(command, SERVICE_VERSION),
        None => {
            eprintln!("fau: no command given; expected `serve`, `migrate` or `register`");
            ExitCode::FAILURE
        }
```

Create `backend/crates/app/src/register/mod.rs`:

```rust
//! `fau register ...` (#3441, docs/school-register-design.md §5.1): the weekly sync and its
//! seed. It runs as `fau_register` (D9), logs JSON to stderr, and keeps stdout for its own
//! output.
//!
//! Exit codes, which the CronJob (#3424) and an operator read:
//! - 0: the run finished: applied, no change, or a dry run printed;
//! - 1: it failed: configuration, the database, or a source;
//! - 2: bad arguments (clap's own);
//! - 3: the planner aborted it (an empty source or the circuit breaker, §5.3);
//! - 4: another run holds the lock;
//! - 5: refused: `--seed` on a register that is not empty, or a sync on an empty one.

mod fetch;
mod render;
mod sync;

use std::process::ExitCode;

use crate::config::RegisterConfig;
use crate::telemetry::{self, LogStream};

#[derive(clap::Subcommand)]
pub(crate) enum RegisterCommand {
    /// Sync the register from NSR, Kartverket and SSB (§5.2).
    Sync {
        /// Plan, print the plan as JSON on stdout, and write nothing but a `dry_run` run row.
        #[arg(long)]
        dry_run: bool,
        /// The first run, on an empty register: creates every municipality and school.
        #[arg(long)]
        seed: bool,
    },
}

/// How a register command ended. The numbers are the process exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Exit {
    Done = 0,
    Failed = 1,
    Aborted = 3,
    Locked = 4,
    Refused = 5,
}

impl From<Exit> for ExitCode {
    fn from(exit: Exit) -> Self {
        ExitCode::from(exit as u8)
    }
}

pub(crate) fn run(command: RegisterCommand, service_version: &'static str) -> ExitCode {
    let config = match RegisterConfig::from_env() {
        Ok(config) => config,
        Err(e) => {
            // Before any subscriber exists: plain text, the variable name and never its value.
            eprintln!("fau: {e}");
            return Exit::Failed.into();
        }
    };
    telemetry::init_to(&config.log_level, service_version, LogStream::Stderr);
    let rt = tokio::runtime::Runtime::new().expect("build a tokio runtime for `register`");
    let exit = rt.block_on(async {
        match command {
            RegisterCommand::Sync { dry_run, seed } => sync::sync(&config, dry_run, seed).await,
        }
    });
    exit.into()
}
```

Create `backend/crates/app/src/register/fetch.rs`:

```rust
//! §5.2 steps 1-3: Kartverket, then SSB, then the NSR list and the detail of every unit the
//! run needs. Nothing here logs a URL, a body or an address.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use fau_domain::register::source::{CodeChange, MunicipalityRecord, NsrUnit};
use fau_domain::register::sync::{RegisterSnapshot, RunKind, SchoolStatus};
use fau_register_sources::client::{SourceClient, SourceUrls};
use fau_register_sources::SourceError;
use jiff::civil::Date;
use url::Url;
use uuid::Uuid;

use crate::config::RegisterConfig;

/// §5.2 step 3: "about a minute at four in parallel".
const DETAIL_FETCHES_IN_PARALLEL: usize = 4;

/// Everything one run fetched.
pub(super) struct Fetched {
    pub municipalities: Vec<MunicipalityRecord>,
    pub code_changes: Vec<CodeChange>,
    /// Each unit with the raw bytes it was parsed from, sorted by orgnr.
    pub units: Vec<(NsrUnit, Vec<u8>)>,
    /// Register orgnrs NSR's list no longer carries: left untouched, and only counted.
    pub absent_from_list: usize,
}

pub(super) fn source_urls(config: &RegisterConfig) -> SourceUrls {
    let base = |url: &Option<Url>, default: String| {
        url.as_ref()
            .map_or(default, |u| u.as_str().trim_end_matches('/').to_owned())
    };
    let production = SourceUrls::production();
    SourceUrls {
        nsr: base(&config.nsr_url, production.nsr),
        kartverket: base(&config.kartverket_url, production.kartverket),
        ssb: base(&config.ssb_url, production.ssb),
        // Part 5 reads Brreg; this run never does.
        brreg: production.brreg,
    }
}

/// Fetches in the Handover's order. A seed is passed no code changes at all; a sync reads
/// SSB over the fixed lookback from `ssb_from` (the seed date) to today. The NSR detail pass
/// covers every active grunnskole in the list plus every orgnr of a school the register
/// holds open, once each.
pub(super) async fn fetch(
    client: Arc<SourceClient>,
    snapshot: &RegisterSnapshot<Uuid>,
    kind: RunKind,
    ssb_from: Option<Date>,
    today: Date,
) -> Result<Fetched, SourceError> {
    let municipalities = client.municipalities().await?;
    let code_changes = match (kind, ssb_from) {
        (RunKind::Sync, Some(from)) => client.code_changes(from, today).await?,
        _ => Vec::new(),
    };

    let list = client.nsr_all_units().await?;
    let listed: BTreeSet<&str> = list.iter().map(|u| u.orgnr.as_str()).collect();
    let mut wanted: BTreeSet<String> = list
        .iter()
        .filter(|u| u.is_active && u.is_primary_school)
        .map(|u| u.orgnr.clone())
        .collect();
    let mut absent_from_list = 0;
    for school in &snapshot.schools {
        if school.status == SchoolStatus::Closed {
            continue;
        }
        if let Some(orgnr) = &school.orgnr {
            if listed.contains(orgnr.as_str()) {
                wanted.insert(orgnr.clone());
            } else {
                absent_from_list += 1;
            }
        }
    }

    let units = details(client, wanted.into_iter().collect()).await?;
    Ok(Fetched {
        municipalities,
        code_changes,
        units,
        absent_from_list,
    })
}

/// [`DETAIL_FETCHES_IN_PARALLEL`] workers draining one queue. The first error ends the run:
/// a partial detail pass is never planned from (§5.3).
async fn details(
    client: Arc<SourceClient>,
    orgnrs: Vec<String>,
) -> Result<Vec<(NsrUnit, Vec<u8>)>, SourceError> {
    let queue = Arc::new(Mutex::new(orgnrs.into_iter()));
    let mut workers = tokio::task::JoinSet::new();
    for _ in 0..DETAIL_FETCHES_IN_PARALLEL {
        let (client, queue) = (client.clone(), queue.clone());
        workers.spawn(async move {
            let mut fetched = Vec::new();
            loop {
                let next = queue.lock().expect("the detail queue lock").next();
                let Some(orgnr) = next else {
                    return Ok::<_, SourceError>(fetched);
                };
                fetched.push(client.nsr_unit_with_payload(&orgnr).await?);
            }
        });
    }
    let mut all = Vec::new();
    while let Some(joined) = workers.join_next().await {
        all.extend(joined.expect("a detail fetch worker panicked")?);
    }
    all.sort_by(|a, b| a.0.orgnr.cmp(&b.0.orgnr));
    Ok(all)
}
```

Create `backend/crates/app/src/register/render.rs`:

```rust
//! `--dry-run`'s output: the plan as JSON, with ops, reviews and counts, and never an
//! address. A created school's attributes are left out, and an attribute update names the
//! fields it changes rather than their values.

use fau_domain::register::sync::{
    MunicipalityOp, Ref, RegisterSnapshot, ReviewItem, RunKind, SchoolAttributes, SchoolOp,
    SlugChange, SyncOutcome,
};
use fau_persistence::register::{abort_reason_text, counts_json};
use serde_json::{json, Map, Value};
use uuid::Uuid;

fn reference(r: &Ref<Uuid>) -> Value {
    match r {
        Ref::Existing(id) => json!(id),
        Ref::New(n) => json!(format!("new:{n}")),
    }
}

fn slug_change(c: &Option<SlugChange>) -> (Value, Value) {
    match c {
        Some(c) => (json!(c.old), json!(c.new)),
        None => (Value::Null, Value::Null),
    }
}

fn municipality_op(op: &MunicipalityOp<Uuid>) -> Value {
    match op {
        MunicipalityOp::Create {
            new,
            number,
            name,
            slug,
            source,
            ..
        } => json!({
            "op": "create_municipality", "new": format!("new:{new}"), "number": number,
            "name": name, "slug": slug, "source": source.code(),
        }),
        MunicipalityOp::Renumber {
            id,
            from,
            to,
            valid_from,
            name,
            old_slug,
            new_slug,
        } => json!({
            "op": "renumber_municipality", "municipality": id, "from": from, "to": to,
            "valid_from": valid_from.to_string(), "name": name, "old_slug": old_slug,
            "new_slug": new_slug,
        }),
        MunicipalityOp::Rename {
            id,
            name,
            old_slug,
            new_slug,
        } => json!({
            "op": "rename_municipality", "municipality": id, "name": name,
            "old_slug": old_slug, "new_slug": new_slug,
        }),
        MunicipalityOp::UpdateDetails {
            id,
            official_name,
            county_number,
            county_name,
            names,
        } => json!({
            "op": "update_municipality", "municipality": id, "official_name": official_name,
            "county_number": county_number, "county_name": county_name,
            "names": names.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(),
        }),
    }
}

/// The names of the fields that differ, never their values.
fn changed_fields(old: &SchoolAttributes, new: &SchoolAttributes) -> Vec<&'static str> {
    let mut changed = Vec::new();
    if old.ownership != new.ownership {
        changed.push("ownership");
    }
    if old.grade_from != new.grade_from {
        changed.push("grade_from");
    }
    if old.grade_to != new.grade_to {
        changed.push("grade_to");
    }
    if old.language != new.language {
        changed.push("language");
    }
    if old.website != new.website {
        changed.push("website");
    }
    if old.street_address != new.street_address {
        changed.push("street_address");
    }
    if old.postcode != new.postcode {
        changed.push("postcode");
    }
    if old.post_town != new.post_town {
        changed.push("post_town");
    }
    changed
}

fn school_op(op: &SchoolOp<Uuid>, snapshot: &RegisterSnapshot<Uuid>) -> Value {
    match op {
        SchoolOp::Create {
            new,
            municipality,
            orgnr,
            register_name,
            display_name,
            slug,
            verification,
            ..
        } => json!({
            "op": "create_school", "new": format!("new:{new}"),
            "municipality": reference(municipality), "orgnr": orgnr,
            "register_name": register_name, "display_name": display_name, "slug": slug,
            "verification": verification.code(),
        }),
        SchoolOp::Rename {
            id,
            register_name,
            display_name,
            slug,
        } => {
            let (old_slug, new_slug) = slug_change(slug);
            json!({
                "op": "rename_school", "school": id, "register_name": register_name,
                "display_name": display_name, "old_slug": old_slug, "new_slug": new_slug,
            })
        }
        SchoolOp::UpdateAttributes { id, attributes } => {
            let old = snapshot
                .schools
                .iter()
                .find(|s| s.id == *id)
                .map(|s| s.attributes.clone())
                .unwrap_or_default();
            json!({
                "op": "update_attributes", "school": id,
                "changed": changed_fields(&old, attributes),
            })
        }
        SchoolOp::Move { id, from, to, slug } => {
            let (old_slug, new_slug) = slug_change(slug);
            json!({
                "op": "move_school", "school": id, "from": from, "to": reference(to),
                "old_slug": old_slug, "new_slug": new_slug,
            })
        }
        SchoolOp::Close {
            id,
            reason,
            closed_on,
            successor,
        } => json!({
            "op": "close_school", "school": id, "reason": reason.code(),
            "closed_on": closed_on.to_string(), "successor": successor.as_ref().map(reference),
        }),
    }
}

/// Review details already hold ids, codes, names and orgnrs only (ruling, 24 September 2026).
fn review(item: &ReviewItem<Uuid>) -> Value {
    let details: Map<String, Value> = item
        .details
        .iter()
        .map(|(k, v)| ((*k).to_owned(), json!(v)))
        .collect();
    json!({
        "kind": item.kind.code(),
        "school": item.school.as_ref().map(reference),
        "other_school": item.other_school.as_ref().map(reference),
        "municipality": item.municipality.as_ref().map(reference),
        "details": details,
    })
}

pub(super) fn plan_json(
    kind: RunKind,
    outcome: &SyncOutcome<Uuid>,
    snapshot: &RegisterSnapshot<Uuid>,
) -> Value {
    let kind = match kind {
        RunKind::Seed => "seed",
        RunKind::Sync => "sync",
    };
    match outcome {
        SyncOutcome::NoChange => json!({ "kind": kind, "outcome": "no_change" }),
        SyncOutcome::Abort { reason, counts } => json!({
            "kind": kind, "outcome": "abort", "abort_reason": abort_reason_text(reason),
            "counts": counts_json(counts),
        }),
        SyncOutcome::Apply(plan) => json!({
            "kind": kind, "outcome": "apply", "counts": counts_json(&plan.counts),
            "municipality_ops": plan.municipality_ops.iter().map(municipality_op).collect::<Vec<_>>(),
            "school_ops": plan.school_ops.iter().map(|op| school_op(op, snapshot)).collect::<Vec<_>>(),
            "reviews": plan.reviews.iter().map(review).collect::<Vec<_>>(),
        }),
    }
}
```

Create `backend/crates/app/src/register/sync.rs`:

```rust
//! `fau register sync [--dry-run] [--seed]` (docs/school-register-design.md §5.1-5.3, and
//! the part 3 plan's "Handover to part 4").
//!
//! One connection carries the whole run, because it holds the session advisory lock. The
//! first snapshot only decides what to fetch. The plan is made from a second snapshot, read
//! in the REPEATABLE READ transaction that then applies it, so the register cannot change
//! between planning and applying.

use std::sync::Arc;

use fau_domain::register::sync::{plan, RunKind, SyncInputs, SyncOutcome};
use fau_domain::time::Moment;
use fau_persistence::register::{
    abort_reason_text, apply_plan, load_snapshot, record_aborted, record_applied, record_dry_run,
    record_failed, record_no_change, register_is_empty, seed_date, stage_payloads, start_run,
    try_lock, NsrPayload, RegisterError,
};
use fau_register_sources::client::SourceClient;
use fau_register_sources::SourceError;
use sqlx::{Connection, PgConnection};
use uuid::Uuid;

use super::fetch::{fetch, source_urls};
use super::render::plan_json;
use super::Exit;
use crate::config::RegisterConfig;

/// Why a run failed. `Display` is fixed text plus a source's or the database's fixed
/// classification: never a URL, a body, a payload or an address.
#[derive(Debug, thiserror::Error)]
enum SyncError {
    #[error("{0}")]
    Database(#[from] RegisterError),
    #[error("source error ({0})")]
    Source(#[from] SourceError),
    #[error("the register has no applied seed run to start the SSB lookback from")]
    NoSeed,
}

impl From<sqlx::Error> for SyncError {
    fn from(e: sqlx::Error) -> Self {
        SyncError::Database(e.into())
    }
}

pub(super) async fn sync(config: &RegisterConfig, dry_run: bool, seed: bool) -> Exit {
    let at = Moment::at(jiff::Timestamp::now());
    let kind = if seed { RunKind::Seed } else { RunKind::Sync };
    let mut conn = match PgConnection::connect(config.database_url.expose()).await {
        Ok(conn) => conn,
        Err(e) => {
            let e = SyncError::from(e);
            tracing::error!(error = %e, "could not connect to the register database");
            return Exit::Failed;
        }
    };
    match begin(&mut conn, kind, dry_run, at).await {
        Ok(Begun::Run(run)) => match execute(&mut conn, config, kind, dry_run, run, at).await {
            Ok(exit) => exit,
            Err(e) => {
                tracing::error!(run_id = %run, error = %e, "register sync failed");
                if let Err(record) = record_failed(&mut conn, run, &e.to_string(), at).await {
                    tracing::error!(run_id = %run, error = %record, "could not record the failure");
                }
                Exit::Failed
            }
        },
        Ok(Begun::Refused(exit)) => exit,
        Err(e) => {
            tracing::error!(error = %e, "register sync failed before it started");
            Exit::Failed
        }
    }
}

enum Begun {
    Run(Uuid),
    Refused(Exit),
}

/// The lock, the empty-register rule (the Handover: `--seed` refuses a non-empty register,
/// and a sync refuses an empty one), then the run row. A refusal writes nothing.
async fn begin(
    conn: &mut PgConnection,
    kind: RunKind,
    dry_run: bool,
    at: Moment,
) -> Result<Begun, SyncError> {
    if !try_lock(conn).await? {
        tracing::error!("another run holds the lock");
        return Ok(Begun::Refused(Exit::Locked));
    }
    match (kind, register_is_empty(conn).await?) {
        (RunKind::Seed, false) => {
            tracing::error!("--seed refuses a register that is not empty");
            Ok(Begun::Refused(Exit::Refused))
        }
        (RunKind::Sync, true) => {
            tracing::error!("the register is empty: seed it first with --seed");
            Ok(Begun::Refused(Exit::Refused))
        }
        _ => {
            let run = start_run(conn, kind, dry_run, at).await?;
            tracing::info!(run_id = %run, kind = ?kind, dry_run, "register sync started");
            Ok(Begun::Run(run))
        }
    }
}

async fn execute(
    conn: &mut PgConnection,
    config: &RegisterConfig,
    kind: RunKind,
    dry_run: bool,
    run: Uuid,
    at: Moment,
) -> Result<Exit, SyncError> {
    let ssb_from = match kind {
        RunKind::Seed => None,
        RunKind::Sync => Some(seed_date(conn).await?.ok_or(SyncError::NoSeed)?),
    };
    let held = load_snapshot(conn).await?;
    let client = Arc::new(SourceClient::new(source_urls(config))?);
    let fetched = fetch(client, &held, kind, ssb_from, at.today()).await?;
    if fetched.absent_from_list > 0 {
        tracing::warn!(
            run_id = %run,
            count = fetched.absent_from_list,
            "register orgnrs absent from the NSR list are left untouched"
        );
    }
    let units: Vec<_> = fetched.units.iter().map(|(u, _)| u.clone()).collect();
    let inputs = SyncInputs {
        municipalities: &fetched.municipalities,
        code_changes: &fetched.code_changes,
        units: &units,
        kind,
        at,
    };

    let mut tx = conn.begin().await?;
    sqlx::query("set transaction isolation level repeatable read")
        .execute(&mut *tx)
        .await?;
    let snapshot = load_snapshot(&mut tx).await?;
    let outcome = plan(&snapshot, &inputs);

    if dry_run {
        println!(
            "{}",
            serde_json::to_string_pretty(&plan_json(kind, &outcome, &snapshot))
                .expect("a JSON value always serialises")
        );
        tx.rollback().await?;
        return Ok(match &outcome {
            SyncOutcome::Abort { reason, counts } => {
                record_dry_run(conn, run, counts, Some(reason), at).await?;
                Exit::Aborted
            }
            SyncOutcome::Apply(plan) => {
                record_dry_run(conn, run, &plan.counts, None, at).await?;
                Exit::Done
            }
            SyncOutcome::NoChange => {
                record_dry_run(conn, run, &Default::default(), None, at).await?;
                Exit::Done
            }
        });
    }

    match outcome {
        SyncOutcome::NoChange => {
            tx.rollback().await?;
            record_no_change(conn, run, at).await?;
            tracing::info!(run_id = %run, outcome = "no_change", "register sync finished");
            Ok(Exit::Done)
        }
        SyncOutcome::Abort { reason, counts } => {
            tx.rollback().await?;
            record_aborted(conn, run, kind, &reason, &counts, at).await?;
            tracing::error!(
                run_id = %run,
                abort_reason = %abort_reason_text(&reason),
                "register sync aborted"
            );
            Ok(Exit::Aborted)
        }
        SyncOutcome::Apply(plan) => {
            let applied = apply_plan(&mut tx, &plan, at).await?;
            if applied.wrote_nothing() {
                // Every review deduplicated away and there was no op: nothing to keep.
                tx.rollback().await?;
                record_no_change(conn, run, at).await?;
                tracing::info!(
                    run_id = %run,
                    outcome = "no_change",
                    reviews_deduplicated = applied.deduplicated_reviews,
                    "register sync finished"
                );
                return Ok(Exit::Done);
            }
            let payloads: Vec<NsrPayload> = fetched
                .units
                .into_iter()
                .map(|(unit, body)| NsrPayload::new(&unit, body))
                .collect();
            stage_payloads(&mut tx, &payloads, at).await?;
            record_applied(&mut tx, run, kind, &applied, at).await?;
            tx.commit().await?;
            tracing::info!(
                run_id = %run,
                outcome = "applied",
                ops = applied.ops,
                reviews_written = applied.new_reviews.len(),
                reviews_deduplicated = applied.deduplicated_reviews,
                "register sync finished"
            );
            Ok(Exit::Done)
        }
    }
}
```

- [ ] **Step 7: Run the tests to verify they pass.**

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test -p fau-app --test register_cli`
Expected: PASS, 6 tests.

Run: `cd /workspace/backend && cargo test -p fau-app --bin fau`
Expected: PASS, including `a_source_url_must_be_http_with_a_host_and_no_credentials`.

Run: `cd /workspace/backend && cargo run -q -p fau-app --bin fau -- register sync --help`
Expected: usage `fau register sync [OPTIONS]` with `--dry-run` and `--seed`.

- [ ] **Step 8: The full checks.** Run the four Global Constraints commands. Expected: all clean,
  including the existing `config.rs`, `logging.rs` and `cli.rs` suites (the `serve` log stream and
  its `LOG_LEVEL` rules are unchanged).

- [ ] **Step 9: Commit.**

```bash
cd /workspace/backend
git add Cargo.lock crates/app/Cargo.toml crates/app/src/config.rs crates/app/src/telemetry.rs \
  crates/app/src/main.rs crates/app/src/register/ crates/app/tests/common/mod.rs \
  crates/app/tests/common/register.rs crates/app/tests/register_cli.rs
GIT_AUTHOR_NAME="Erik W. Bjønnes" GIT_AUTHOR_EMAIL=erik@ewb-solutions.as \
GIT_COMMITTER_NAME="Erik W. Bjønnes" GIT_COMMITTER_EMAIL=erik@ewb-solutions.as \
git commit -m "Add fau register sync with --seed and --dry-run (#3441)" \
  -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: `fau register export`, the operations docs, and the parked `.partial` fix

**Files:**
- Create: `backend/crates/persistence/src/register/export.rs`
- Modify: `backend/crates/persistence/src/register/mod.rs` (whole file)
- Create: `backend/crates/app/src/register/export.rs`
- Modify: `backend/crates/app/src/register/mod.rs` (whole file)
- Modify: `backend/crates/app/src/main.rs` (one doc line)
- Modify: `backend/crates/register-sources/src/client.rs` (`download_brreg_bulk`)
- Test: `backend/crates/register-sources/tests/client.rs` (one new test)
- Test: `backend/crates/app/tests/register_export.rs` (new)
- Modify: `docs/app-foundation-operations.md` (a new section)

**Interfaces:**
- Consumes: Task 5's `RegisterConfig`, `Exit`, `RegisterCommand`, `run_fau_register`, `register_url`;
  `common::register::test_municipality` (0301 Oslo, id `01990000-0000-7000-8000-000000000301`).
- Produces:
  - `fau_persistence::register::ExportRow { school_id: Uuid, orgnr: Option<String>, municipality_number: String, display_name: String, path: String, fau_orgnr: Option<String> }`
    and `export_rows(&mut PgConnection) -> Result<Vec<ExportRow>, RegisterError>`;
  - `register::RegisterCommand::Export`, i.e. `fau register export`.

- [ ] **Step 1: Write the failing tests.** Create `backend/crates/app/tests/register_export.rs`:

```rust
//! `fau register export` (#3441 part 4, docs/school-register-design.md §9): the real binary,
//! as `fau_register`, against a register whose every row has a fixed id, compared with the
//! exact CSV.

mod common;

use common::register::{run_fau_register, test_municipality};
use common::TestDb;

#[tokio::test]
async fn the_export_lists_every_pickable_school_as_csv() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    // 0301 Oslo, id ...0301.
    test_municipality(&admin).await;
    sqlx::raw_sql(
        "insert into municipalities (id, name, county_number, county_name, slug, status, source,
                                     search_text)
         values ('01990000-0000-7000-8000-000000003201', 'Bærum', '32', 'Akershus',
                 '3201-baerum', 'active', 'kartverket', 'bærum');
         insert into municipality_numbers (municipality_id, number, valid_from, valid_until)
         values ('01990000-0000-7000-8000-000000003201', '3024', '2020-01-01', '2024-01-01'),
                ('01990000-0000-7000-8000-000000003201', '3201', '2024-01-01', null);
         insert into schools (id, municipality_id, origin, display_name, register_name, slug,
                              verification, orgnr, in_scope, scope_override, status, closed_on,
                              closure_reason, search_text)
         values
           -- Listed, with a slug and a linked FAU.
           ('01990000-0000-7000-8000-000000000001', '01990000-0000-7000-8000-000000003201',
            'register', 'Hosle skole', 'Hosle skole', 'hosle-skole', 'listed', '974552124',
            true, null, 'active', null, null, 'hosle skole'),
           -- Listed, no slug, and a name that needs quoting.
           ('01990000-0000-7000-8000-000000000002', '01990000-0000-7000-8000-000000000301',
            'register', 'Skole \"Nord\", avd. 2', 'Skole Nord', null, 'listed', '900000002',
            true, null, 'active', null, null, 'skole nord'),
           -- Verified submission without an orgnr.
           ('01990000-0000-7000-8000-000000000003', '01990000-0000-7000-8000-000000000301',
            'submitted', 'Nyskolen', null, 'nyskolen', 'verified', null,
            true, null, 'active', null, null, 'nyskolen'),
           -- Out of scope by the filter, kept in by an operator.
           ('01990000-0000-7000-8000-000000000004', '01990000-0000-7000-8000-000000000301',
            'register', 'Sykehusskolen', 'Sykehusskolen', 'sykehusskolen', 'listed',
            '900000004', false, true, 'active', null, null, 'sykehusskolen'),
           -- Never exported: closed, held, pending, and in scope but overridden out.
           ('01990000-0000-7000-8000-000000000005', '01990000-0000-7000-8000-000000000301',
            'register', 'Nedlagt skole', 'Nedlagt skole', null, 'listed', '900000005',
            true, null, 'closed', '2026-01-01', 'closed', 'nedlagt skole'),
           ('01990000-0000-7000-8000-000000000006', '01990000-0000-7000-8000-000000000301',
            'register', 'Holdt skole', 'Holdt skole', null, 'held', '900000006',
            true, null, 'active', null, null, 'holdt skole'),
           ('01990000-0000-7000-8000-000000000007', '01990000-0000-7000-8000-000000000301',
            'submitted', 'Ventende skole', null, null, 'pending', null,
            true, null, 'active', null, null, 'ventende skole'),
           ('01990000-0000-7000-8000-000000000008', '01990000-0000-7000-8000-000000000301',
            'register', 'Voksenskolen', 'Voksenskolen', 'voksenskolen', 'listed', '900000008',
            true, false, 'active', null, null, 'voksenskolen');
         insert into registered_faus (orgnr, registered_name, organisation_form,
                                      municipality_number, status, last_seen_in_source_at)
         values ('913591100', 'FAU HOSLE SKOLE', 'FLI', '3201', 'active',
                 '2026-09-28T02:30:00Z');
         insert into school_fau_links (school_id, fau_orgnr, method, state)
         values ('01990000-0000-7000-8000-000000000001', '913591100', 'address', 'linked');",
    )
    .execute(&admin)
    .await
    .expect("the export fixture");

    let url = db.register_url();
    let out = run_fau_register(&["export"], &[("REGISTER_DATABASE_URL", url.as_str())]).await;
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "school_id,orgnr,municipality_number,display_name,path,fau_orgnr\n\
         01990000-0000-7000-8000-000000000002,900000002,0301,\"Skole \"\"Nord\"\", avd. 2\",/s/01990000-0000-7000-8000-000000000002,\n\
         01990000-0000-7000-8000-000000000003,,0301,Nyskolen,/fau/0301-oslo/nyskolen,\n\
         01990000-0000-7000-8000-000000000004,900000004,0301,Sykehusskolen,/fau/0301-oslo/sykehusskolen,\n\
         01990000-0000-7000-8000-000000000001,974552124,3201,Hosle skole,/fau/3201-baerum/hosle-skole,913591100\n"
    );
}
```

The traced order: 0301 sorts before 3201, and within 0301 the ids 2, 3 and 4 ascend. School 2 has no
slug, so its path is `/s/<uuid>`, and its name holds a comma and quotes, so it is quoted with the
quotes doubled. School 3 has no orgnr: an empty field. School 4 is out of scope by the filter but kept
in by `scope_override = true`. Schools 5 to 8 are closed, held, pending, and overridden out. Only
Hosle has a `linked` FAU.

In `backend/crates/register-sources/tests/client.rs`, add before
`an_unreachable_source_is_a_transport_error`:

```rust
/// The rename is the last step: when it fails (here `dest` is a non-empty directory, which
/// a file cannot replace), the `.partial` file is removed like on any other failure.
#[tokio::test]
async fn a_failed_final_rename_leaves_no_partial_file() {
    let (c, hits) = client().await;
    let dir = std::env::temp_dir().join(format!("fau-brreg-rename-test-{}", std::process::id()));
    let dest = dir.join("enheter.json.gz");
    std::fs::create_dir_all(dest.join("occupied")).unwrap();
    let err = c.download_brreg_bulk(&dest).await.unwrap_err();
    assert_eq!((err.source, err.kind), (Source::Brreg, SourceErrorKind::Io));
    assert_eq!(hits.load(Ordering::SeqCst), 1, "the download itself ran");
    assert!(
        !dir.join("enheter.json.gz.partial").exists(),
        "the partial file must not survive a failed rename"
    );
    assert!(dest.join("occupied").is_dir(), "dest is left as it was");
    std::fs::remove_dir_all(&dir).unwrap();
}
```

- [ ] **Step 2: Run the tests to verify they fail.**

Run: `cd /workspace/backend && cargo test -p fau-register-sources --test client a_failed_final_rename`
Expected: FAIL with `the partial file must not survive a failed rename`. The failed run leaves its
directory behind; remove it: `rm -rf /tmp/fau-brreg-rename-test-*`.

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test -p fau-app --test register_export`
Expected: FAIL: exit code 2, since clap does not know `export` yet.

- [ ] **Step 3: The `.partial` fix.** In `backend/crates/register-sources/src/client.rs`, in the doc
  comment of `download_brreg_bulk`, replace

```rust
    /// has flushed successfully. On any error the partial file is removed (best effort) and
    /// `dest` is never created or overwritten.
```

with

```rust
    /// has flushed successfully. On any error, the final rename's included, the partial file
    /// is removed (best effort) and `dest` is never created or overwritten.
```

and replace the function's closing `match`:

```rust
        match result {
            Ok(written) => {
                tokio::fs::rename(&partial, dest).await.map_err(io)?;
                Ok(written)
            }
            Err(err) => {
                let _ = tokio::fs::remove_file(&partial).await;
                Err(err)
            }
        }
```

with

```rust
        // The rename is the last step that can fail, so it is covered by the same cleanup:
        // a `.partial` file never outlives a failed download (part 2's carried-over gap).
        let result = match result {
            Ok(written) => tokio::fs::rename(&partial, dest)
                .await
                .map(|()| written)
                .map_err(io),
            Err(err) => Err(err),
        };
        if result.is_err() {
            let _ = tokio::fs::remove_file(&partial).await;
        }
        result
```

- [ ] **Step 4: The export.** Create `backend/crates/persistence/src/register/export.rs`:

```rust
//! `fau register export` (docs/school-register-design.md §9): the register's school identity
//! for #3431, which keys every prospect row on our school UUID and never edits school data.
//! Only pickable schools (§7) are listed: active, listed or verified, and effectively in
//! scope. Pending, held, rejected, closed and out-of-scope rows are never outreach targets.

use sqlx::PgConnection;
use uuid::Uuid;

use super::error::RegisterError;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct ExportRow {
    pub school_id: Uuid,
    pub orgnr: Option<String>,
    pub municipality_number: String,
    pub display_name: String,
    /// `/fau/<municipality slug>/<school slug>`, or `/s/<uuid>` for a school without a slug.
    pub path: String,
    /// The linked Brreg FAU (§4.6). Empty until part 5's matcher links one.
    pub fau_orgnr: Option<String>,
}

/// Every pickable school, by municipality number and then id: a stable order that never
/// sorts by a name (§7, #3439).
pub async fn export_rows(conn: &mut PgConnection) -> Result<Vec<ExportRow>, RegisterError> {
    Ok(sqlx::query_as(
        "select s.id as school_id, s.orgnr, n.number as municipality_number, s.display_name,
                case when s.slug is null then '/s/' || s.id::text
                     else '/fau/' || m.slug || '/' || s.slug end as path,
                (select l.fau_orgnr from school_fau_links l
                  where l.school_id = s.id and l.state = 'linked') as fau_orgnr
           from schools s
           join municipalities m on m.id = s.municipality_id
           join municipality_numbers n on n.municipality_id = m.id and n.valid_until is null
          where s.status = 'active' and s.verification in ('listed', 'verified')
            and coalesce(s.scope_override, s.in_scope)
          order by n.number, s.id",
    )
    .fetch_all(&mut *conn)
    .await?)
}
```

Replace the whole of `backend/crates/persistence/src/register/mod.rs` with:

```rust
//! The school register's persistence (#3441 part 4, docs/school-register-design.md §4, §5):
//! the snapshot the sync planner reads, the applier that writes its plan in one transaction,
//! and the run bookkeeping around it. Everything here runs as `fau_register` (D9); nothing
//! reads the database clock for a rule, so every function takes the caller's `Moment`.

mod apply;
mod error;
mod export;
mod reviews;
mod runs;
mod school_ops;
mod snapshot;
mod sql;
mod staging;

pub use apply::{apply_plan, AppliedCounts};
pub use error::RegisterError;
pub use export::{export_rows, ExportRow};
pub use reviews::NewReview;
pub use runs::{
    abort_reason_text, counts_json, record_aborted, record_applied, record_dry_run, record_failed,
    record_no_change, seed_date, start_run, try_lock, unlock, REGISTER_SYNC_LOCK_ID,
    SYNC_APPLIED_ACTION,
};
pub use snapshot::{load_snapshot, register_is_empty};
pub use staging::{stage_payloads, NsrPayload};
```

Create `backend/crates/app/src/register/export.rs`:

```rust
//! `fau register export`: the pickable schools as CSV on stdout (§9), for #3431.
//!
//! RFC 4180 quoting (a field with a comma, a quote or a line break is quoted, and its
//! quotes doubled) and LF line ends. Values are written as they are: whoever opens the file
//! in a spreadsheet imports it as text.

use fau_persistence::register::{export_rows, ExportRow, RegisterError};
use sqlx::{Connection, PgConnection};

use super::Exit;
use crate::config::RegisterConfig;

const HEADER: &str = "school_id,orgnr,municipality_number,display_name,path,fau_orgnr";

fn field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

pub(super) fn csv(rows: &[ExportRow]) -> String {
    let mut out = format!("{HEADER}\n");
    for r in rows {
        let line = [
            r.school_id.to_string(),
            field(r.orgnr.as_deref().unwrap_or("")),
            field(&r.municipality_number),
            field(&r.display_name),
            field(&r.path),
            field(r.fau_orgnr.as_deref().unwrap_or("")),
        ]
        .join(",");
        out.push_str(&line);
        out.push('\n');
    }
    out
}

pub(super) async fn export(config: &RegisterConfig) -> Exit {
    let rows: Result<Vec<ExportRow>, RegisterError> = async {
        let mut conn = PgConnection::connect(config.database_url.expose()).await?;
        export_rows(&mut conn).await
    }
    .await;
    match rows {
        Ok(rows) => {
            print!("{}", csv(&rows));
            tracing::info!(schools = rows.len(), "register export finished");
            Exit::Done
        }
        Err(e) => {
            tracing::error!(error = %e, "register export failed");
            Exit::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn a_field_is_quoted_only_when_it_must_be() {
        assert_eq!(field("Hosle skole"), "Hosle skole");
        assert_eq!(
            field("Skole \"Nord\", avd. 2"),
            "\"Skole \"\"Nord\"\", avd. 2\""
        );
        assert_eq!(field("to\nlinjer"), "\"to\nlinjer\"");
        assert_eq!(field(""), "");
    }

    #[test]
    fn missing_values_are_empty_fields() {
        let row = ExportRow {
            school_id: Uuid::from_u128(1),
            orgnr: None,
            municipality_number: "0301".into(),
            display_name: "Nyskolen".into(),
            path: "/s/00000000-0000-0000-0000-000000000001".into(),
            fau_orgnr: None,
        };
        assert_eq!(
            csv(&[row]),
            "school_id,orgnr,municipality_number,display_name,path,fau_orgnr\n\
             00000000-0000-0000-0000-000000000001,,0301,Nyskolen,/s/00000000-0000-0000-0000-000000000001,\n"
        );
    }
}
```

Replace the whole of `backend/crates/app/src/register/mod.rs` with:

```rust
//! `fau register ...` (#3441, docs/school-register-design.md §5.1, §9): the weekly sync, its
//! seed, and the export. It runs as `fau_register` (D9), logs JSON to stderr, and keeps
//! stdout for its own output.
//!
//! Exit codes, which the CronJob (#3424) and an operator read:
//! - 0: the run finished: applied, no change, or a dry run printed;
//! - 1: it failed: configuration, the database, or a source;
//! - 2: bad arguments (clap's own);
//! - 3: the planner aborted it (an empty source or the circuit breaker, §5.3);
//! - 4: another run holds the lock;
//! - 5: refused: `--seed` on a register that is not empty, or a sync on an empty one.

mod export;
mod fetch;
mod render;
mod sync;

use std::process::ExitCode;

use crate::config::RegisterConfig;
use crate::telemetry::{self, LogStream};

#[derive(clap::Subcommand)]
pub(crate) enum RegisterCommand {
    /// Sync the register from NSR, Kartverket and SSB (§5.2).
    Sync {
        /// Plan, print the plan as JSON on stdout, and write nothing but a `dry_run` run row.
        #[arg(long)]
        dry_run: bool,
        /// The first run, on an empty register: creates every municipality and school.
        #[arg(long)]
        seed: bool,
    },
    /// Print every pickable school as CSV on stdout, for the prospect register (#3431).
    Export,
}

/// How a register command ended. The numbers are the process exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Exit {
    Done = 0,
    Failed = 1,
    Aborted = 3,
    Locked = 4,
    Refused = 5,
}

impl From<Exit> for ExitCode {
    fn from(exit: Exit) -> Self {
        ExitCode::from(exit as u8)
    }
}

pub(crate) fn run(command: RegisterCommand, service_version: &'static str) -> ExitCode {
    let config = match RegisterConfig::from_env() {
        Ok(config) => config,
        Err(e) => {
            // Before any subscriber exists: plain text, the variable name and never its value.
            eprintln!("fau: {e}");
            return Exit::Failed.into();
        }
    };
    telemetry::init_to(&config.log_level, service_version, LogStream::Stderr);
    let rt = tokio::runtime::Runtime::new().expect("build a tokio runtime for `register`");
    let exit = rt.block_on(async {
        match command {
            RegisterCommand::Sync { dry_run, seed } => sync::sync(&config, dry_run, seed).await,
            RegisterCommand::Export => export::export(&config).await,
        }
    });
    exit.into()
}
```

In `backend/crates/app/src/main.rs`, replace the `Register` variant's doc line with:

```rust
    /// The school register (#3441): sync it from its public sources, or export it.
```

- [ ] **Step 5: Run the tests to verify they pass.**

Run: `cd /workspace/backend && cargo test -p fau-register-sources --test client`
Expected: PASS, 14 tests.

Run: `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test -p fau-app --test register_export`
Expected: PASS, 1 test.

Run: `cd /workspace/backend && cargo test -p fau-app --bin fau register::export`
Expected: PASS, 2 tests.

- [ ] **Step 6: Document the commands.** In `docs/app-foundation-operations.md`, insert this section
  immediately before `## Port variables`:

````markdown
## The school register: `fau register sync` and `fau register export`

`fau register sync` seeds the school register from NSR, Kartverket and SSB, and then keeps it in
step with them (docs/school-register-design.md §5, #3441). `fau register export` prints the register's
pickable schools as CSV for the prospect register (§9, #3431). Both run as `fau_register`, never as
`fau_app`, and both log JSON lines on **stderr**: stdout carries only a dry run's plan or the CSV.

| Variable | Required | Meaning |
| --- | --- | --- |
| `REGISTER_DATABASE_URL` | yes | A `postgres://` URL for the `fau_register` role. Errors name the variable, never the value. |
| `REGISTER_NSR_URL`, `REGISTER_KARTVERKET_URL`, `REGISTER_SSB_URL` | no | Base URLs that replace the public APIs, for tests. http or https, with a host and no userinfo. |
| `LOG_LEVEL` | no | As for `serve`; `info` when unset. |

| Command | What it does |
| --- | --- |
| `fau register sync --seed --dry-run` | Plans the first run and prints it as JSON. Writes only a `dry_run` row to `register_sync_runs`. |
| `fau register sync --seed` | The first run, on an empty register. Refuses a register that already holds a municipality or a school. |
| `fau register sync --dry-run` | Plans a sync and prints it. |
| `fau register sync` | The weekly sync. Refuses an empty register. |
| `fau register export > register.csv` | `school_id,orgnr,municipality_number,display_name,path,fau_orgnr`, one row per pickable school, ordered by municipality number and id. |

| Exit code | Meaning |
| --- | --- |
| 0 | Done: applied, nothing to change, a dry run printed, or the export written |
| 1 | Failed: configuration, the database or a source. A started run is recorded as `failed`. |
| 2 | Bad arguments |
| 3 | Aborted by the planner: a source returned nothing, or the circuit breaker tripped (§5.3). The run is recorded as `aborted`, and `register.sync_aborted` is queued to `fau@ewb-solutions.as`. |
| 4 | Another run holds the register's advisory lock (`0x4641553000003441`) |
| 5 | Refused: `--seed` on a non-empty register, or a sync on an empty one |

**Seeding an environment.** After `fau migrate`, run `fau register sync --seed --dry-run` and read
the plan, then `fau register sync --seed` once. The seed sends one `register.seed_summary` mail; later
syncs send one `register.review_item` per new review item. Seed in full before the first sync is
scheduled: with fewer than 50 active schools, a single closure trips the 2% circuit breaker. The SSB
lookback starts on the seed's date, so a sync on a register with no applied seed fails.

**The weekly CronJob** belongs to #3424: `fau register sync`, Mondays 04:30 with
`timeZone: Europe/Oslo`, `concurrencyPolicy: Forbid`, the same image, and egress to
`data-nsr.udir.no`, `api.kartverket.no` and `data.ssb.no` (and `data.brreg.no` once part 5 reads
Brreg). The advisory lock keeps a manual run and the CronJob apart as well. Each run leaves one
`register_sync_runs` row, which is the alert source (#3442): `aborted` or `failed`, or a row with no
`finished_at` from a run that died.

Against the local Compose stack, from the host:

```
cd backend
REGISTER_DATABASE_URL="postgres://fau_register:${FAU_REGISTER_PASSWORD:-fau_register}@localhost:${FAU_DB_PORT:-5433}/fau" \
  cargo run -q -p fau-app --bin fau -- register sync --seed --dry-run
```

This calls the live public APIs. The test suites never do: `tests/register_cli.rs` serves the
recorded fixtures from a local server instead.
````

- [ ] **Step 7: The full checks.** Run the four Global Constraints commands. Expected: all clean.

- [ ] **Step 8: Commit.**

```bash
cd /workspace/backend
git add crates/persistence/src/register/ crates/app/src/register/ crates/app/src/main.rs \
  crates/register-sources/src/client.rs crates/register-sources/tests/client.rs \
  crates/app/tests/register_export.rs ../docs/app-foundation-operations.md
GIT_AUTHOR_NAME="Erik W. Bjønnes" GIT_AUTHOR_EMAIL=erik@ewb-solutions.as \
GIT_COMMITTER_NAME="Erik W. Bjønnes" GIT_COMMITTER_EMAIL=erik@ewb-solutions.as \
git commit -m "Add fau register export, document the register commands, drop a stale .partial (#3441)" \
  -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Self-Review

- **The brief's scope, item by item:**
  - the snapshot loader, deterministic order, exact round trip, `has_live_fau` from pending or active
    tenants: Task 1, `the_snapshot_reads_every_column_back_exactly` (an empty string next to a null,
    a pending and a closed tenant, an old number next to the current one, a dissolved municipality);
  - the applier in one transaction, `Ref::New` to UUIDv7 before any op: Tasks 2-3 (`NewIds`);
  - slug history with the `valid_from` derivation and the empty-interval skip:
    `a_middle_slug_writes_no_history_and_a_same_day_renumber_closes_a_day_later`, and the exact
    intervals in `municipality_ops_apply_like_the_test_applier` and
    `seed_rename_reregistration_then_nothing_match_the_test_applier`;
  - municipality number history: the same two tests;
  - successor links, `in_scope = false` on `OutOfScope`, no slug on a held row: Task 3's two parity
    scenarios, and the mutations in Task 3 Step 5;
  - reviews deduplicated by the corrected key: `moves_closures_holds_and_reviews_match_the_test_applier`
    (an Apply whose reviews all deduplicate writes nothing) and
    `open_reviews_deduplicate_on_their_discriminating_details` (four items with null references);
  - staging (payload, SHA-256, scope code, `fetched_at`):
    `payloads_are_staged_with_their_hash_and_scope_and_mark_schools_seen`;
  - run rows, the audit row, an aborted run's row and reason: Task 4;
  - the advisory lock: `one_session_holds_the_lock_until_it_lets_go`, and the CLI's refusal with
    exit 4;
  - the three mails, params with ids and codes only: Task 4;
  - the role: every writing test uses `register_pool()` or `register_url()`;
    `the_runtime_role_cannot_record_a_run` and `the_runtime_role_cannot_run_the_sync` show that
    `fau_app` cannot;
  - `RegisterConfig`, errors by variable name: Task 5 (`configuration_errors_name_the_variable_and_never_its_value`
    and `a_source_url_must_be_http_with_a_host_and_no_credentials`);
  - the sync flow, steps 1-9: Task 5's six end-to-end tests (seed, then `no_change` with the SSB
    lookback from the seed date; the dry run; the three refusals; the empty-NSR abort; the runtime
    role), and `record_no_change` on a fully deduplicated Apply in `sync.rs`;
  - `--dry-run` prints JSON with ops, reviews and counts, never an address, and writes only a
    `dry_run` row: `a_dry_run_prints_the_plan_and_writes_only_its_run_row`;
  - JSON logs that carry no URL, address or payload: the log checks in
    `a_seed_then_a_sync_of_the_same_sources_changes_nothing`;
  - the export's columns, paths, sorting and escaping: Task 6's golden test and its two unit tests;
  - the `.partial` fix: `a_failed_final_rename_leaves_no_partial_file`;
  - the CronJob's command, exit codes and variables documented: Task 6 Step 6.
- **The Handover's contract:** allocate first, then apply in order; `Close` with history and a
  cleared slug; `OutOfScope` clears `in_scope`; `Create` with origin `register` and no FAU link; no
  `school_orgnr_history` writes; history only for `old: Some`, keyed on `from` for a move;
  `MunicipalityOp::Rename` history only when the slug changes; `Renumber` closes and opens numbers
  and folds in the name; `UpdateDetails` replaces the names; dedup; a fully deduplicated Apply is
  `no_change`; the snapshot's exact attributes, deterministic order and closed rows' slugs; `NoChange`
  writes only the run row; `MassChange` in `abort_reason`; unit dedup by orgnr (the planner already
  does it, and the fetch asks for each orgnr once); the fixed SSB lookback, no changes for a seed;
  `--seed` refuses a non-empty register. The one Handover point not implemented is the dissolved
  municipality's number closure, which is an operator's action with no op, and so no code in part 4.
- **Privileges used, all granted by 0004 to `fau_register`:** select, insert and update on
  `municipalities`, `municipality_numbers`, `municipality_slug_history`, `schools` (RLS: read,
  insert and update all `using (true)`), `school_slug_history`, `register_source_records`
  (`on conflict do update` needs all three), `register_sync_runs`, `register_review_items`;
  select, insert and delete on `municipality_names`; select on `school_orgnr_history`,
  `school_fau_links` and `tenants`; insert on `audit_events` and `outbox`. The advisory lock functions are
  executable by every role. So no migration 0005.
- **Placeholders:** none. Every step has its code, and every expected value was produced by a run.
- **Type consistency:**
  - `AppliedCounts` gains `new_reviews` and `deduplicated_reviews` in Task 3; Task 2's tests only
    name the type, so they compile unchanged;
  - `step` is written once, in Task 2, in its final form;
  - `NsrPayload::new(&NsrUnit, Vec<u8>)` (Task 3) is what `sync.rs` (Task 5) calls;
  - `record_applied(conn, run, kind, &AppliedCounts, at)` (Task 4) is what `sync.rs` calls;
  - `RegisterCommand` (Task 5) gains `Export` in Task 6.
- **Known limitations, left as they are:**
  - a school that got its first slug through a `Rename` (`old: None`) and is renamed again later
    starts its history row at `created_at`, not at the first rename. No history row records when a
    slugless school got its slug;
  - `municipality_names` is keyed on `(municipality_id, name)`: if Kartverket ever listed one name in
    two languages for a municipality, the run would fail loudly on the primary key. No municipality in
    today's recorded Kartverket file does (the seed of all 357 passes);
  - a school's `in_scope` goes stale when an operator's `scope_override = true` keeps it open (the
    Handover's known gap);
  - a run that crashes after `start_run` leaves an unfinished row, which is the intended alert; a
    crash between the snapshot and the commit changes nothing, since the apply is one transaction.
- **Not in this plan:** Brreg matching, `registered_faus` and FAU links (part 5); the review CLI and
  the review screen (#3499); the CronJob manifest (#3424); rulings for Erik, which the controller
  records in docs/planning-decisions.md after the build.
