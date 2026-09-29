# Remembering a Former Member: Implementation Plan (#3511)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a membership ends, keep its display name and contact address hidden for the member's chosen period (0, 3, 6, 12 or 24 months; 3 by default) and then destroy them. A person who returns within the period is recognised again. Everyone else keeps seeing role and year on that person's old contributions, even after they return.

**Architecture:**
- **Schema.** Migration `0009` adds `memberships.profile_retained_until`. It relaxes 0008's two `*_only_while_current` checks so that a revoked row may hold its fields while that date is set, and restricts `accounts.retention_months` to {0, 3, 6, 12, 24}.
- **Domain (pure).**
  - `fau_domain::membership::retention`: `RetentionMonths` and `keep_until(ended_on, months, today)`.
  - `fau_domain::directory::history`: `ended_on(held, today)`, plus `ActivePeriod`/`active_period`, which split an active membership's history into the current period and earlier ones.
- **Persistence.** One function, `retention::settle_profile`, decides under the tenant lock whether an ended membership's fields are retained (it stamps `profile_retained_until`) or expired (it clears them).
  - The sweep, `grant_role`, `ensure_membership` and `set_retention_months` all go through it.
  - `revoke_membership` stamps or clears in the same statement that revokes.
  - `member_names` gains `MemberName::Returned`, for an active member with an earlier period.
  - `set_retention_months` is the member's own setting.
- **Reads for others are unchanged.** The directory, export and `member_names` decide "ended" at read time and never show an ended membership's name. That rule stays binding.

**Tech Stack:** Rust 1.98.1, sqlx 0.8 on PostgreSQL 17, jiff 0.2, serde_json.

**Spec:** `docs/member-retention-design.md` (agreed with Erik in chat on 29 September 2026; the #3511 card carries it as an attachment). It amends D3 of `docs/groups-directory-chat-calendar-design.md` §4.2 as built by #3502 (`docs/superpowers/plans/2026-09-28-member-directory-3502.md`). Executors read the spec in full before Task 3.

## Global Constraints

- **M1: others see role and year, always.** Only the returning person gets their name, their address and their own open work back. Nothing shown to another member may link a returner to their contributions from an earlier period.
- **M2: the allowed periods are exactly {0, 3, 6, 12, 24} months, and the default is 3.** 0 means today's #3502 behaviour: clear at once.
- **M3: everything is per FAU.** The retained fields stay on the FAU's own membership row, encrypted under that FAU's record key. Nothing crosses to another FAU, and nothing new holds key material.
- **The period starts on the day the membership ended** (`ended_on`, Ruling R2) and runs until `ended_on + months`, exclusive. The fields are retained while `today < profile_retained_until`.
- **An Article 17 erasure clears everything at once**, whether retained or not, and leaves `profile_retained_until` null.
- **Names and addresses are content.** They never appear in audit parameters, a NOTIFY, a log line or a `Debug` output. Audit parameters hold codes and dates only.
- **Every tenant table carries `tenant_id`, and every reference is composite.** No column name may contain `key`, `dek`, `kek`, `secret`, `private`, `passphrase`, `password`, `cipher`, `nonce` or `wrapped`. `schema_review.rs` must stay green.
- **Migrations continue from `0008`.** `0009` inserts `schema_contract` version 9. `MINIMUM_CONTRACT_VERSION` stays 2, because `0009` is additive for old code: old code clears on revocation, which the relaxed checks still accept, and old code never writes `retention_months`. Never edit an applied migration.
- **Dates cross the SQL boundary as text** (`date_param`, `parse_date`, `to_char(..., 'YYYY-MM-DD')`). Rule-deciding time comes from the caller's `Moment`, never the database clock.
- **Order of refusals:** input validation, tenant state, authority, then row state.
- **Locking:** every mutation of a tenant's membership state takes `lock_tenant` first. A function that touches several FAU-er locks them in tenant-id order *before* it writes any account row, so it never waits on a tenant lock while holding the account row (`accept_invitation` takes those locks in the other order).
- **English** for all technical text, code, comments and commit messages. #3511 renders no user-facing strings. Never author Nynorsk.
- **Test commands** run from `/workspace/backend` with these exported:
```bash
cd /workspace/backend
export TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres
export TEST_OPENBAO_ADDR=http://openbao:8200
export TEST_OPENBAO_TOKEN=dev-only-root
```
  The app stack is `docker compose -f /workspace/compose.yaml` (project `fau-app`). **Never run two workspace test runs at once** (Postgres allows 100 connections): check `ps aux | grep '[c]argo test'` first.
- **Before every commit:** `cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings` must be clean.
- **Commits** go on branch `retention-3511`, created from `main`. Every message ends with `(#3511)`, a blank line, and the committing agent's own `Co-Authored-By` trailer. If git has no identity, pass `GIT_AUTHOR_NAME="Erik W. Bjønnes" GIT_AUTHOR_EMAIL=erik@ewb-solutions.as` and the same `GIT_COMMITTER_*` on each command. Never change git config and never push.
- **Security tests must be able to fail.** Each test below names the mutation it catches. Run that mutation once and see the test go red before you report the task done.

## Plan rulings (for Erik's review)

- **R1: The period is stored, not recomputed on every read.** `profile_retained_until` is set when the end is noticed, so a later change of `retention_months` is an explicit recalculation (`set_retention_months`), not a silent side effect. An end is noticed at `revoke_membership` time, or by `settle_profile` for a membership whose roles simply ran out: the sweep, a grant, an acceptance, or a settings change.
- **R2: `ended_on` is the latest day any role stopped being held, and never later than today.** Stopping is the natural end, or the Oslo date of an early revocation. It comes from the same `HeldRole` spans history uses. A membership revoked in October whose roles ran out in June ended in June, so its period runs from June. If a row held no role at all, `ended_on` is today.
- **R3: The check that holds this is "a revoked row holds a field only while `profile_retained_until` is set".** The date comparison itself belongs to the sweep: a check constraint cannot read the clock. Two further checks: `profile_retained_until` needs a field to retain, and an erased row has none.
- **R4: Coming back goes through one function, `retention::reopen_profile`.** It settles first, so an expired profile is cleared and a retained one kept. Then it sets `revoked_at` and `profile_retained_until` to null in one statement, and audits `membership.profile_restored` when something was retained. `ensure_membership` (acceptance, activation) and `grant_role` both call it. This replaces #3502's `clear_if_ended`: *restore if retained, clear if expired* (spec §3).
- **R5: History labels by period, as a new variant.** `MemberName::Returned { name, period }` is produced only for an active membership that had an earlier period. `period.label_on(event_date)` gives `Some(role label)` for an event before the current period started and `None` for "show the name". `Named` keeps its shape, so the directory and its tests are untouched. The current period is the last run of held-role spans that touch or overlap; a gap of one day or more splits periods (Ruling R6).
- **R6: The period's start is taken from role dates, not from when a role was granted.** `role_assignments.created_at` is written by the database clock, which the rules never read. So a membership that sat in a gap before a role that was already booked counts as having ended and restarted, even though `membership_ended` would have said "active" during the gap. The cost is that such a member's contributions made during the gap show role and year. The rule stays one plain function, and it never shows a name wrongly.
- **R7: The prefill hint is exact.** `prepare_acceptance` reports `existing_current = true` for a membership that is active, or ended but still within its period, with the name not erased. For an unstamped natural end it computes the period in Rust, exactly as `settle_profile` would. The prefill *data* is not added here: the screen card (#3417) reads the member's own name when it builds the form.
- **R8: `set_retention_months` takes an `account_id`** that the session (#3417) takes from the verified login, so "own account only" holds by construction. It sets the value on the account and recalculates every stamped period that account holds, in every FAU. A recalculated period that is already over clears at once, audited with cause `retention_shortened`. A longer setting extends a period that is still running. A period that is already over is never extended: it is cleared as expired first (spec §4). It audits only the memberships whose period changed. The account row itself has no audit trail, the same as `locale`.
- **R9: Audit actions.** `membership.profile_retained` {until} when a period is stamped, `membership.profile_restored` {} when a returner is recognised, `membership.retention_changed` {until} when a setting moves a period, and `membership.profile_cleared` with cause `membership_ended` (period 0), `retention_ended` (period over) or `retention_shortened` (setting). All are system actor except `retention_changed`, whose actor is the member.
- **R10: The login email's lapse follows the member's chosen period (Erik, 29 September 2026; spec M5).** ADR-003 §6a's fixed three months is replaced by `accounts.retention_months`: the account lapses that many months after its last membership anywhere ends, at once for none. No account-lapse code exists yet, and #3426 builds it. So #3511 changes no lapse code, only the documents: ADR-003 §6a, the flow spec and the design spec are already amended. The final report puts the binding rule on the #3426 card.

## File structure

| File | Change |
|---|---|
| `backend/migrations/0009_member_retention.sql` | Create: column, checks, retention_months values, contract 9 |
| `backend/crates/domain/src/membership/retention.rs` | Create: `RetentionMonths`, `keep_until` |
| `backend/crates/domain/src/membership/mod.rs` | Modify: `pub mod retention;` |
| `backend/crates/domain/src/directory/history.rs` | Modify: `ended_on`, `ActivePeriod`, `active_period` |
| `backend/crates/persistence/src/membership/names.rs` | Modify: `held_roles` extracted, `Returned` variant |
| `backend/crates/persistence/src/membership/retention.rs` | Modify: `settle_profile`, `reopen_profile`, `clear_profile`, sweep, erasure, `set_retention_months` |
| `backend/crates/persistence/src/membership/roles.rs` | Modify: `revoke_membership` stamps; `grant_role` uses `reopen_profile`; `clear_if_ended` removed |
| `backend/crates/persistence/src/membership/sql.rs` | Modify: `ensure_membership` takes `Moment`, uses `reopen_profile` |
| `backend/crates/persistence/src/membership/invitations.rs` | Modify: `prepare_acceptance` "current or retained"; `ensure_membership` call |
| `backend/crates/persistence/src/membership/signup.rs` | Modify: `ensure_membership` call |
| `backend/crates/persistence/src/membership/error.rs` | Modify: `UnknownAccount` |
| `backend/crates/persistence/src/membership/mod.rs` | Modify: exports |
| `backend/crates/app/tests/retention_schema.rs` | Create: migration 0009 tests |
| `backend/crates/app/tests/member_retention.rs` | Create: behaviour tests for the spec §6 list |
| `backend/crates/app/tests/directory_schema.rs`, `directory_retention.rs`, `member_profile.rs`, `schema_spine.rs` | Modify: #3502 tests whose D3 expectations change |

---

### Task 1: Domain — retention periods

**Files:**
- Create: `backend/crates/domain/src/membership/retention.rs`
- Modify: `backend/crates/domain/src/membership/mod.rs`

**Interfaces:**
- Produces: `fau_domain::membership::retention::{RetentionMonths, keep_until}`.
  - `RetentionMonths::{None, Three, Six, Twelve, TwentyFour}`, `Copy + Eq + Debug`.
  - `RetentionMonths::DEFAULT == Three`, `RetentionMonths::ALL: [Self; 5]`.
  - `fn months(self) -> i32` returns 0/3/6/12/24; `fn from_months(i32) -> Option<Self>`.
  - `pub fn retained_until(ended_on: Date, months: RetentionMonths) -> Option<Date>` is `None` for `None`.
  - `pub fn keep_until(ended_on: Date, months: RetentionMonths, today: Date) -> Option<Date>` is `retained_until` filtered to `today < until`: `Some` means retain until then, `None` means clear now.

- [ ] **Step 1: Create the branch**

```bash
cd /workspace && git switch -c retention-3511 main
```

- [ ] **Step 2: Write the failing tests** — `retention.rs` holding only the test module and `pub mod retention;` in `membership/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn only_the_five_allowed_values_parse() {
        for (n, v) in [
            (0, RetentionMonths::None),
            (3, RetentionMonths::Three),
            (6, RetentionMonths::Six),
            (12, RetentionMonths::Twelve),
            (24, RetentionMonths::TwentyFour),
        ] {
            assert_eq!(RetentionMonths::from_months(n), Some(v));
            assert_eq!(v.months(), n);
        }
        for n in [-1, 1, 2, 4, 5, 7, 11, 13, 18, 25, 36] {
            assert_eq!(RetentionMonths::from_months(n), None, "{n}");
        }
        assert_eq!(RetentionMonths::DEFAULT, RetentionMonths::Three);
        assert_eq!(RetentionMonths::ALL.len(), 5);
    }

    #[test]
    fn the_period_counts_calendar_months_from_the_day_it_ended() {
        let ended = date(2026, 10, 1);
        assert_eq!(retained_until(ended, RetentionMonths::None), None);
        assert_eq!(retained_until(ended, RetentionMonths::Three), Some(date(2027, 1, 1)));
        assert_eq!(retained_until(ended, RetentionMonths::TwentyFour), Some(date(2028, 10, 1)));
        // A month end clamps rather than spilling into the next month.
        assert_eq!(
            retained_until(date(2026, 11, 30), RetentionMonths::Three),
            Some(date(2027, 2, 28))
        );
    }

    #[test]
    fn keep_until_is_the_date_while_it_is_still_ahead_and_none_after() {
        let ended = date(2026, 10, 1);
        let m = RetentionMonths::Three;
        assert_eq!(keep_until(ended, m, date(2026, 12, 31)), Some(date(2027, 1, 1)));
        assert_eq!(keep_until(ended, m, date(2027, 1, 1)), None, "the last day is exclusive");
        assert_eq!(keep_until(ended, RetentionMonths::None, ended), None);
    }
}
```

- [ ] **Step 3: Run and see it fail**

Run: `cargo test -p fau-domain --lib membership::retention`
Expected: compile errors, `RetentionMonths` not found.

- [ ] **Step 4: Implement** above the test module:

```rust
//! How long a former member is remembered (#3511, docs/member-retention-design.md, Erik's
//! M2 of 29 September 2026). When a membership ends, its display name and contact address
//! are kept hidden for the member's chosen period and then destroyed. Pure: persistence
//! reads the account's `retention_months` and the roles the membership held, and asks
//! [`keep_until`] whether to retain or clear.

use jiff::civil::Date;
use jiff::Span;

/// `accounts.retention_months`: the allowed values and nothing else (migration 0009's
/// `accounts_retention_months_is_allowed`). `None` clears at once, #3502's D3 behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetentionMonths {
    None,
    Three,
    Six,
    Twelve,
    TwentyFour,
}

impl RetentionMonths {
    pub const DEFAULT: Self = Self::Three;
    pub const ALL: [Self; 5] = [
        Self::None,
        Self::Three,
        Self::Six,
        Self::Twelve,
        Self::TwentyFour,
    ];

    pub fn months(self) -> i32 {
        match self {
            Self::None => 0,
            Self::Three => 3,
            Self::Six => 6,
            Self::Twelve => 12,
            Self::TwentyFour => 24,
        }
    }

    pub fn from_months(n: i32) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.months() == n)
    }
}

/// The first day the fields are no longer kept: `ended_on` plus the period, a month end
/// clamped (jiff's rule). `None` for a period of none.
pub fn retained_until(ended_on: Date, months: RetentionMonths) -> Option<Date> {
    if months == RetentionMonths::None {
        return None;
    }
    ended_on
        .checked_add(Span::new().months(i64::from(months.months())))
        .ok()
}

/// Whether a membership that ended on `ended_on` still keeps its fields on `today`: the
/// date they go, or `None` to clear them now.
pub fn keep_until(ended_on: Date, months: RetentionMonths, today: Date) -> Option<Date> {
    retained_until(ended_on, months).filter(|until| today < *until)
}
```

- [ ] **Step 5: Run and see it pass**

Run: `cargo test -p fau-domain --lib membership::retention`
Expected: 3 passed.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add backend/crates/domain/src/membership/retention.rs backend/crates/domain/src/membership/mod.rs
git commit -m "Add the retention periods a former member may choose (#3511)

Co-Authored-By: <your trailer>"
```

---

### Task 2: Domain — when a membership ended, and its current period

**Files:**
- Modify: `backend/crates/domain/src/directory/history.rs`

**Interfaces:**
- Consumes: `HeldRole`, `RoleLabel`, `role_label` (already in this file).
- Produces:
  - `pub fn ended_on(held: &[HeldRole], today: Date) -> Date`.
  - `#[derive(Debug, Clone, PartialEq, Eq)] pub struct ActivePeriod { pub since: Date, pub earlier: Vec<HeldRole> }` with `pub fn label_on(&self, on: Date) -> Option<RoleLabel>`.
  - `pub fn active_period(held: &[HeldRole], today: Date) -> ActivePeriod`.

- [ ] **Step 1: Write the failing tests** — append to the existing `mod tests` in `history.rs`:

```rust
    #[test]
    fn a_membership_ended_on_the_last_day_a_role_stopped_never_after_today() {
        let h = two_years(); // Kasserer 2024-09-01..2025-09-01, Leder 2025-09-01..2026-09-01
        assert_eq!(ended_on(&h, date(2026, 10, 15)), date(2026, 9, 1));
        // Revoked the same day a running role was cut short: that role now ends today.
        assert_eq!(ended_on(&h, date(2026, 3, 1)), date(2026, 3, 1));
        assert_eq!(ended_on(&[], date(2026, 3, 1)), date(2026, 3, 1), "held nothing");
    }

    #[test]
    fn one_unbroken_run_of_roles_is_one_period_with_nothing_earlier() {
        let p = active_period(&two_years(), date(2026, 3, 1));
        assert_eq!(p.since, date(2024, 9, 1));
        assert!(p.earlier.is_empty());
        assert_eq!(p.label_on(date(2025, 1, 1)), None, "the name, all through");
    }

    #[test]
    fn after_a_gap_the_current_period_starts_again_and_earlier_events_show_role_and_year() {
        // Kasserer 2024-09-01..2025-09-01, then nothing, then a guest from 2026-02-01.
        let mut h = vec![two_years().remove(0)];
        h.push(held("Gjest", CapabilityClass::Guest, date(2026, 2, 1), date(2026, 6, 1)));
        let p = active_period(&h, date(2026, 3, 1));
        assert_eq!(p.since, date(2026, 2, 1));
        assert_eq!(p.earlier.len(), 1);
        assert_eq!(
            p.label_on(date(2025, 3, 1)).map(|l| (l.role, l.first_year, l.last_year)),
            Some(("Kasserer".into(), 2024, 2025))
        );
        assert_eq!(p.label_on(date(2026, 2, 1)), None, "the first day of the new period");
        assert_eq!(p.label_on(date(2026, 3, 1)), None);
    }

    #[test]
    fn roles_that_touch_or_overlap_are_one_period_and_a_one_day_gap_splits() {
        let touching = [
            held("A", CapabilityClass::Member, date(2025, 1, 1), date(2025, 6, 1)),
            held("B", CapabilityClass::Member, date(2025, 6, 1), date(2026, 6, 1)),
        ];
        assert_eq!(active_period(&touching, date(2026, 1, 1)).since, date(2025, 1, 1));
        let overlapping = [
            held("A", CapabilityClass::Member, date(2025, 1, 1), date(2025, 9, 1)),
            held("B", CapabilityClass::Member, date(2025, 6, 1), date(2026, 6, 1)),
        ];
        assert_eq!(active_period(&overlapping, date(2026, 1, 1)).since, date(2025, 1, 1));
        let gap = [
            held("A", CapabilityClass::Member, date(2025, 1, 1), date(2025, 6, 1)),
            held("B", CapabilityClass::Member, date(2025, 6, 2), date(2026, 6, 1)),
        ];
        let p = active_period(&gap, date(2026, 1, 1));
        assert_eq!(p.since, date(2025, 6, 2));
        assert_eq!(p.earlier.len(), 1);
    }

    #[test]
    fn a_period_still_to_start_counts_from_today() {
        // Returned by invitation today for a role that starts next month: what they do
        // today is theirs, what they did in the old period is not.
        let h = [
            held("A", CapabilityClass::Member, date(2025, 1, 1), date(2025, 6, 1)),
            held("B", CapabilityClass::Member, date(2026, 4, 1), date(2027, 4, 1)),
        ];
        let p = active_period(&h, date(2026, 3, 1));
        assert_eq!(p.since, date(2026, 3, 1));
        assert!(p.label_on(date(2025, 3, 1)).is_some());
        assert_eq!(p.label_on(date(2026, 3, 1)), None);
    }
```

- [ ] **Step 2: Run and see it fail**

Run: `cargo test -p fau-domain --lib directory::history`
Expected: compile errors for `ended_on`, `active_period`.

- [ ] **Step 3: Implement** — after `role_label`, and add a paragraph to the module doc comment: "**A returner** (#3511, Erik's M1): someone active again after an earlier period is shown by name only for events in the current period; earlier events keep their role and year ([`active_period`]).":

```rust
/// The day a membership ended (#3511, plan Ruling R2): the latest day any of its roles
/// stopped being held, never later than `today`. A membership revoked long after its roles
/// ran out ended when they ran out. `today` when it held no role at all.
pub fn ended_on(held: &[HeldRole], today: Date) -> Date {
    held.iter()
        .map(|r| r.until)
        .max()
        .map_or(today, |d| d.min(today))
}

/// An active membership's history split at its current period (#3511, M1): the period
/// started on `since`, and `earlier` holds the roles of every earlier one. Most members
/// have nothing earlier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivePeriod {
    pub since: Date,
    pub earlier: Vec<HeldRole>,
}

impl ActivePeriod {
    /// How an event on `on` shows the person: `None` for their name, or the role and year
    /// from an earlier period.
    pub fn label_on(&self, on: Date) -> Option<RoleLabel> {
        if on >= self.since {
            None
        } else {
            role_label(&self.earlier, on)
        }
    }
}

/// Splits an active membership's roles at its current period (plan Rulings R5, R6).
/// Periods are runs of held spans that touch or overlap; a gap of a day or more starts a
/// new one. The current period is the last run, counted from today if it is still to
/// start.
pub fn active_period(held: &[HeldRole], today: Date) -> ActivePeriod {
    let mut spans: Vec<(Date, Date)> = held.iter().map(|r| (r.from, r.until)).collect();
    spans.sort();
    let mut current: Option<(Date, Date)> = None;
    for (from, until) in spans {
        current = match current {
            Some((f, u)) if from <= u => Some((f, u.max(until))),
            _ => Some((from, until)),
        };
    }
    let start = current.map_or(today, |(from, _)| from);
    ActivePeriod {
        since: start.min(today),
        earlier: held.iter().filter(|r| r.until < start).cloned().collect(),
    }
}
```

- [ ] **Step 4: Run and see it pass**

Run: `cargo test -p fau-domain --lib directory::history`
Expected: all pass, the six existing tests included.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add backend/crates/domain/src/directory/history.rs
git commit -m "Find when a membership ended and where its current period starts (#3511)

Co-Authored-By: <your trailer>"
```

---

### Task 3: Migration 0009

**Files:**
- Create: `backend/migrations/0009_member_retention.sql`
- Create: `backend/crates/app/tests/retention_schema.rs`
- Modify: `backend/crates/app/tests/directory_schema.rs` (`a_revoked_membership_holds_neither_a_name_nor_a_contact_email`)
- Modify: `backend/crates/app/tests/schema_spine.rs` (`retention_months_defaults_to_three`: update the comment only)

**Interfaces:**
- Produces: `memberships.profile_retained_until date` (nullable). Constraints:
  - `memberships_display_name_only_while_current_or_retained`
  - `memberships_contact_email_only_while_current_or_retained`
  - `memberships_retention_needs_a_field`
  - `memberships_erasure_clears_retention`
  - `accounts_retention_months_is_allowed`
- Contract version 9.

- [ ] **Step 1: Check that no persistent database has applied a 0009** (read-only):

```bash
psql "$TEST_DATABASE_URL" -Atc "select version from _sqlx_migrations order by version desc limit 3" 2>/dev/null || true
ls backend/migrations
```
Expected: the highest file is `0008_member_directory.sql`. The test harness migrates fresh databases per test, so the dev `postgres` database may show nothing.

- [ ] **Step 2: Write the failing schema tests** — `retention_schema.rs`. Copy the file's header, `envelope`, `seed`, `set` and `constraint_name` helpers from `directory_schema.rs` lines 1–66, and import `constraint_name` from wherever `directory_schema.rs` imports it:

```rust
//! Migration 0009: a former member's fields are kept for their chosen period (#3511,
//! docs/member-retention-design.md §4).

// ... helpers copied from directory_schema.rs: envelope, seed, set ...

async fn exec(pool: &PgPool, sql: &str, m: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(sql).bind(m).execute(pool).await.map(|_| ())
}

/// Mutation check: keep 0008's `*_only_while_current` checks and the retained revocation
/// is refused.
#[tokio::test]
async fn a_revoked_row_keeps_its_fields_only_while_a_retention_date_is_set() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (_, m) = seed(&pool).await;
    set(&pool, m, "encrypted_display_name", Some(envelope(1, 60))).await.unwrap();
    set(&pool, m, "encrypted_contact_email", Some(envelope(1, 60))).await.unwrap();

    let err = exec(&pool, "update memberships set revoked_at = now() where id = $1", m)
        .await
        .unwrap_err();
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("memberships_display_name_only_while_current_or_retained")
    );
    exec(
        &pool,
        "update memberships set revoked_at = now(), profile_retained_until = '2027-01-01'
          where id = $1",
        m,
    )
    .await
    .unwrap();
    // Dropping the date without the fields is refused; with them it is accepted.
    assert!(exec(&pool, "update memberships set profile_retained_until = null where id = $1", m)
        .await
        .is_err());
    exec(
        &pool,
        "update memberships set profile_retained_until = null,
                encrypted_display_name = null, encrypted_contact_email = null where id = $1",
        m,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_contact_address_alone_is_held_by_its_own_check() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (_, m) = seed(&pool).await;
    set(&pool, m, "encrypted_contact_email", Some(envelope(1, 60))).await.unwrap();
    let err = exec(&pool, "update memberships set revoked_at = now() where id = $1", m)
        .await
        .unwrap_err();
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("memberships_contact_email_only_while_current_or_retained")
    );
}

#[tokio::test]
async fn a_retention_date_needs_something_to_retain_and_an_erasure_clears_it() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (_, m) = seed(&pool).await;
    let err = exec(&pool, "update memberships set profile_retained_until = '2027-01-01' where id = $1", m)
        .await
        .unwrap_err();
    assert_eq!(constraint_name(&err).as_deref(), Some("memberships_retention_needs_a_field"));

    set(&pool, m, "encrypted_display_name", Some(envelope(1, 60))).await.unwrap();
    exec(&pool, "update memberships set profile_retained_until = '2027-01-01' where id = $1", m)
        .await
        .unwrap();
    let err = exec(
        &pool,
        "update memberships set name_erased_at = now(),
                encrypted_display_name = null, encrypted_contact_email = null where id = $1",
        m,
    )
    .await
    .unwrap_err();
    // The field check fires first once the fields are gone; either way it is refused.
    assert!(matches!(
        constraint_name(&err).as_deref(),
        Some("memberships_erasure_clears_retention" | "memberships_retention_needs_a_field")
    ));
}

#[tokio::test]
async fn retention_months_takes_only_the_five_allowed_values() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let acc = Uuid::now_v7();
    sqlx::query("insert into accounts (id, email) values ($1, 'r@example.test')")
        .bind(acc)
        .execute(&pool)
        .await
        .unwrap();
    for ok in [0, 3, 6, 12, 24] {
        sqlx::query("update accounts set retention_months = $2 where id = $1")
            .bind(acc)
            .bind(ok)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("{ok}: {e}"));
    }
    for bad in [-1, 1, 5, 18, 25, 36] {
        let err = sqlx::query("update accounts set retention_months = $2 where id = $1")
            .bind(acc)
            .bind(bad)
            .execute(&pool)
            .await
            .unwrap_err();
        assert_eq!(
            constraint_name(&err).as_deref(),
            Some("accounts_retention_months_is_allowed"),
            "{bad}"
        );
    }
}

#[tokio::test]
async fn the_contract_is_version_nine() {
    let db = TestDb::migrated().await;
    let v: i32 = sqlx::query_scalar("select max(version) from schema_contract")
        .fetch_one(&db.admin_pool())
        .await
        .unwrap();
    assert_eq!(v, 9);
}
```

Also in `directory_schema.rs` `a_revoked_membership_holds_neither_a_name_nor_a_contact_email`, replace the two constraint names with the `_or_retained` names, and rename the test `a_revoked_membership_without_a_retention_date_holds_neither_field`. The rest of the test still holds: it never sets `profile_retained_until`. In `schema_spine.rs`, change the comment "member-elected retention is designed for, not built" to "the default of the member's chosen period (#3511)".

If `migrations.rs` asserts the contract version as a literal 8, change it to 9.

- [ ] **Step 3: Run and see them fail**

Run: `cargo test -p fau-app --test retention_schema --test directory_schema`
Expected: FAIL. `profile_retained_until` does not exist, and the constraint names are unknown.

- [ ] **Step 4: Write the migration** — `0009_member_retention.sql`:

```sql
-- 0009: remembering a former member for a chosen period (#3511;
-- docs/member-retention-design.md, Erik's M1-M4 of 29 September 2026). Amends 0008's D3.
--
-- When a membership ends, its display name and contact address are no longer cleared at
-- once. They are kept, hidden, until profile_retained_until, and then the sweep
-- (fau_persistence's clear_ended_profiles) clears both and the date. Others never see a
-- retained name: every read decides "ended" at read time. A person who returns within the
-- period is recognised again; history for others stays role and year.
--
-- * profile_retained_until: the first day the fields are no longer kept (exclusive). Set
--   when the end is noticed, from the account's retention_months; null while active, once
--   cleared, and after an erasure. A check cannot read the clock, so these checks hold the
--   shape and the sweep holds the date.
-- * accounts.retention_months: the member's chosen period, 0 (clear at once, 0008's
--   behaviour), 3 (the default), 6, 12 or 24 months. 0 was refused before.
alter table memberships add column profile_retained_until date;

alter table memberships
  drop constraint memberships_display_name_only_while_current,
  drop constraint memberships_contact_email_only_while_current;
alter table memberships add constraint memberships_display_name_only_while_current_or_retained
  check (revoked_at is null or encrypted_display_name is null
         or profile_retained_until is not null);
alter table memberships add constraint memberships_contact_email_only_while_current_or_retained
  check (revoked_at is null or encrypted_contact_email is null
         or profile_retained_until is not null);
alter table memberships add constraint memberships_retention_needs_a_field
  check (profile_retained_until is null
         or encrypted_display_name is not null or encrypted_contact_email is not null);
alter table memberships add constraint memberships_erasure_clears_retention
  check (name_erased_at is null or profile_retained_until is null);

-- The inline check from 0002 carries PostgreSQL's generated name.
alter table accounts drop constraint accounts_retention_months_check;
alter table accounts add constraint accounts_retention_months_is_allowed
  check (retention_months in (0, 3, 6, 12, 24));

insert into schema_contract (version) values (9);
```

If `accounts_retention_months_check` is not the generated name, find it with `select conname from pg_constraint where conrelid = 'accounts'::regclass and contype = 'c'` against a migrated test database, and use what it prints.

- [ ] **Step 5: Run and see them pass**

Run: `cargo test -p fau-app --test retention_schema --test directory_schema --test migrations --test schema_review --test schema_spine`
Expected: all pass.

- [ ] **Step 6: Run the whole suite and record what the migration alone breaks**

Run: `cargo test --workspace 2>&1 | tail -40`
Expected: green. Nothing yet writes `profile_retained_until`, and clearing still satisfies the relaxed checks. If anything is red, stop and report it.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add backend/migrations/0009_member_retention.sql backend/crates/app/tests/retention_schema.rs \
        backend/crates/app/tests/directory_schema.rs backend/crates/app/tests/schema_spine.rs \
        backend/crates/app/tests/migrations.rs
git commit -m "Migration 0009: keep a former member's fields while a retention date is set (#3511)

Co-Authored-By: <your trailer>"
```

---

### Task 4: Ending a membership keeps the fields for the period; the sweep clears them after

**Files:**
- Modify: `backend/crates/persistence/src/membership/names.rs` (extract `held_roles`)
- Modify: `backend/crates/persistence/src/membership/retention.rs`
- Modify: `backend/crates/persistence/src/membership/roles.rs` (`revoke_membership`)
- Create: `backend/crates/app/tests/member_retention.rs`
- Modify: `backend/crates/app/tests/directory_retention.rs`, `backend/crates/app/tests/member_profile.rs` (the tests whose D3 expectations change)

**Interfaces:**
- Consumes: `RetentionMonths`, `keep_until` (Task 1); `ended_on` (Task 2); `membership_ended` (`profile.rs`); `lock_tenant`, `write_audit`, `Audit`, `date_param`, `parse_date` (`sql.rs`).
- Produces (all `pub(crate)` in `retention.rs` unless marked):
  - `async fn held_roles(conn: &mut PgConnection, tenant_id: Uuid, ids: &[Uuid]) -> Result<HashMap<Uuid, Vec<HeldRole>>, MembershipError>`, in `names.rs`, `pub(crate)`.
  - `enum Settled { Active, Retained, Cleared, Absent }` (`Debug, Clone, Copy, PartialEq, Eq`).
  - `async fn settle_profile(conn: &mut PgConnection, tenant_id: Uuid, membership_id: Uuid, at: Moment) -> Result<Settled, MembershipError>`. The caller holds the tenant lock.
  - `async fn clear_profile(conn, tenant_id, membership_id, at, cause: &'static str) -> Result<(), MembershipError>`.
  - `async fn account_retention(conn, tenant_id, membership_id) -> Result<RetentionMonths, MembershipError>`.
  - `async fn effective_keep_until(conn, tenant_id, membership_id, stamped: Option<&str>, today: Date) -> Result<Option<Date>, MembershipError>`: the stamped date if it is still ahead, otherwise one computed from `ended_on`. `None` means clear now. Task 5's `prepare_acceptance` uses it too.
  - `pub async fn clear_ended_profiles(pool, at) -> Result<u64, MembershipError>`, same signature, new rule.

- [ ] **Step 1: Write the failing behaviour tests** — create `member_retention.rs`. The helpers repeat `directory_retention.rs`'s so the file stands alone:

```rust
//! Remembering a former member for a chosen period (#3511, docs/member-retention-design.md
//! §6). Ending keeps the name and address hidden for the account's `retention_months`;
//! the sweep clears them after; others see role and year throughout.

mod common;
use common::membership::*;
use common::TestDb;
use fau_crypto::Ciphertext;
use fau_domain::membership::period::Period;
use fau_domain::membership::vocabulary::CapabilityClass;
use fau_persistence::membership::*;
use sqlx::PgPool;
use uuid::Uuid;

fn envelope(marker: u8) -> Ciphertext {
    let mut v = vec![1u8, marker];
    v.extend_from_slice(&[0u8; 40]);
    Ciphertext::from_stored(v)
}

async fn join(pool: &PgPool, fau: &Fau, address: &str, p: Period) -> Uuid {
    add_member(pool, fau, address, new_role("Medlem", CapabilityClass::Member), p, at(T0))
        .await
        .membership_id
}

async fn with_contact(pool: &PgPool, fau: &Fau, m: Uuid, marker: u8) {
    set_contact_email(
        pool,
        SetContactEmail {
            tenant_id: fau.tenant_id,
            membership_id: m,
            encrypted_contact_email: Some(envelope(marker)),
        },
        at(T0),
    )
    .await
    .unwrap();
}

/// (name present, address present, profile_retained_until as text)
async fn state(pool: &PgPool, m: Uuid) -> (bool, bool, Option<String>) {
    sqlx::query_as(
        "select encrypted_display_name is not null, encrypted_contact_email is not null,
                to_char(profile_retained_until, 'YYYY-MM-DD')
           from memberships where id = $1",
    )
    .bind(m)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn set_months_by_sql(pool: &PgPool, address: &str, months: i32) {
    sqlx::query("update accounts set retention_months = $2 where email = $1")
        .bind(address)
        .bind(months)
        .execute(pool)
        .await
        .unwrap();
}

async fn leave(pool: &PgPool, fau: &Fau, m: Uuid, at_: &str) {
    revoke_membership(
        pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: m,
            membership_id: m,
            confirm_no_admin: false,
        },
        at(at_),
    )
    .await
    .unwrap();
}

async fn audits(pool: &PgPool, m: Uuid) -> Vec<(String, String)> {
    sqlx::query_as(
        "select action, params::text from audit_events
          where subject_id = $1
            and action in ('membership.profile_retained', 'membership.profile_cleared',
                           'membership.profile_restored', 'membership.retention_changed')
          order by occurred_at, id",
    )
    .bind(m)
    .fetch_all(pool)
    .await
    .unwrap()
}

fn year() -> Period {
    period(day(2026, 9, 1), day(2027, 9, 1))
}

/// Spec §6 "ending with each period value". Leaving on 2026-10-01 stamps
/// `ended_on + months`, or clears at once for 0.
///
/// Mutation check: clear unconditionally in `revoke_membership` (0008's behaviour) and
/// every non-zero row fails.
#[tokio::test]
async fn leaving_keeps_both_fields_for_the_chosen_period_or_clears_at_once_for_none() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    for (months, expected) in [
        (0, None),
        (3, Some("2027-01-01")),
        (6, Some("2027-04-01")),
        (12, Some("2027-10-01")),
        (24, Some("2028-10-01")),
    ] {
        let address = format!("p{months}@example.test");
        let m = join(&pool, &a, &address, year()).await;
        with_contact(&pool, &a, m, 1).await;
        set_months_by_sql(&pool, &address, months).await;
        leave(&pool, &a, m, "2026-10-01T10:00:00Z").await;
        let kept = expected.is_some();
        assert_eq!(
            state(&pool, m).await,
            (kept, kept, expected.map(str::to_owned)),
            "{months} months"
        );
        let expected_audit = match expected {
            Some(d) => ("membership.profile_retained".to_owned(), format!(r#"{{"until": "{d}"}}"#)),
            None => (
                "membership.profile_cleared".to_owned(),
                r#"{"cause": "membership_ended"}"#.to_owned(),
            ),
        };
        assert_eq!(audits(&pool, m).await, [expected_audit], "{months} months");
    }
}

/// Spec §6 "the sweep before and after the period ends", and a natural end: roles that
/// ran out are stamped by the first sweep, from the day they ran out, not the sweep's day.
///
/// Mutation check: stamp from `at.today()` instead of `ended_on` and the natural-end
/// date comes out 2027-01-15.
#[tokio::test]
async fn the_sweep_stamps_a_natural_end_from_the_day_it_ended_and_clears_after_the_period() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let ran_out = join(&pool, &a, "ranout@example.test", period(day(2026, 9, 1), day(2026, 10, 1))).await;
    let left = join(&pool, &a, "left@example.test", year()).await;
    with_contact(&pool, &a, ran_out, 1).await;
    leave(&pool, &a, left, "2026-10-01T10:00:00Z").await;

    // Two weeks after the natural end: the sweep stamps, clears nothing.
    assert_eq!(clear_ended_profiles(&pool, at("2026-10-15T10:00:00Z")).await.unwrap(), 0);
    assert_eq!(state(&pool, ran_out).await, (true, true, Some("2027-01-01".into())));
    // The day before the period ends: still kept, and idempotent.
    assert_eq!(clear_ended_profiles(&pool, at("2026-12-31T10:00:00Z")).await.unwrap(), 0);
    assert_eq!(state(&pool, left).await, (true, false, Some("2027-01-01".into())));
    // The day it ends: both cleared, with the date.
    assert_eq!(clear_ended_profiles(&pool, at("2027-01-01T10:00:00Z")).await.unwrap(), 2);
    for m in [ran_out, left] {
        assert_eq!(state(&pool, m).await, (false, false, None));
        assert_eq!(
            audits(&pool, m).await.last().unwrap(),
            &("membership.profile_cleared".to_owned(), r#"{"cause": "retention_ended"}"#.to_owned())
        );
    }
    assert_eq!(clear_ended_profiles(&pool, at("2027-01-02T10:00:00Z")).await.unwrap(), 0);
}

/// Spec §6 "retention_months = 0": a natural end with a period of none is cleared by the
/// first sweep, as under D3.
#[tokio::test]
async fn a_period_of_none_is_cleared_by_the_first_sweep() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let m = join(&pool, &a, "none@example.test", period(day(2026, 9, 1), day(2026, 10, 1))).await;
    set_months_by_sql(&pool, "none@example.test", 0).await;
    assert_eq!(clear_ended_profiles(&pool, at("2026-10-01T10:00:00Z")).await.unwrap(), 1);
    assert_eq!(state(&pool, m).await, (false, false, None));
}

/// M1 while retained: others see role and year. Neither the directory nor `member_names`
/// gives the retained name.
///
/// Mutation check: make `member_names` return `Named` for a row with a name regardless of
/// `ended` and the second assertion fails.
#[tokio::test]
async fn while_retained_others_see_only_role_and_year() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let m = join(&pool, &a, "kept@example.test", year()).await;
    leave(&pool, &a, m, "2026-10-01T10:00:00Z").await;
    let viewer = Viewer { tenant_id: a.tenant_id, membership_id: a.admin_membership_id };
    let later = at("2026-11-01T10:00:00Z");
    let names = member_names(&pool, viewer, &[m], later).await.unwrap();
    assert!(matches!(names[0].1, MemberName::Ended(_)), "{:?}", names[0].1);
    let dir = member_directory(&pool, viewer, later).await.unwrap();
    assert!(dir.people.iter().all(|p| p.membership_id != m));
}

/// Spec §6 "erasure during retention": everything at once, and the date with it.
#[tokio::test]
async fn an_erasure_during_retention_clears_everything_at_once() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let m = join(&pool, &a, "erase@example.test", year()).await;
    with_contact(&pool, &a, m, 1).await;
    leave(&pool, &a, m, "2026-10-01T10:00:00Z").await;
    let account: Uuid = sqlx::query_scalar("select account_id from memberships where id = $1")
        .bind(m)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(erase_member_names(&pool, account, at("2026-10-02T10:00:00Z")).await.unwrap(), 1);
    assert_eq!(state(&pool, m).await, (false, false, None));
}

/// Spec §6 "cross-tenant isolation": one person in two FAU-er. Leaving A retains in A only;
/// the sweep that clears A leaves B's active membership alone.
///
/// Mutation check: drop `m.tenant_id = $1` from the sweep's per-tenant select and B's row
/// is settled under A's lock (the lock assertion is indirect: B's fields must survive).
#[tokio::test]
async fn retention_is_per_fau() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin-a@example.test", at(T0)).await;
    let b = active_fau(&pool, "admin-b@example.test", at(T0)).await;
    let in_a = join(&pool, &a, "kari@example.test", year()).await;
    let in_b = join(&pool, &b, "kari@example.test", year()).await;
    with_contact(&pool, &b, in_b, 2).await;
    leave(&pool, &a, in_a, "2026-10-01T10:00:00Z").await;
    assert_eq!(state(&pool, in_a).await.2.as_deref(), Some("2027-01-01"));
    assert_eq!(state(&pool, in_b).await, (true, true, None));
    clear_ended_profiles(&pool, at("2027-01-01T10:00:00Z")).await.unwrap();
    assert_eq!(state(&pool, in_a).await, (false, false, None));
    assert_eq!(state(&pool, in_b).await, (true, true, None));
}
```

`member_directory`'s `DirectoryPerson` exposes the membership id. If its field is named differently from `membership_id`, use that name.

- [ ] **Step 2: Run and see them fail**

Run: `cargo test -p fau-app --test member_retention`
Expected: the period tests FAIL. The fields come back `(false, false, None)`, because revocation still clears.

- [ ] **Step 3: Extract `held_roles` in `names.rs`**

Move the assignment query and the loop that builds `held` out of `member_names` into:

```rust
/// The roles each membership in `ids` actually held, over the days held (an early
/// revocation ends a span on its Oslo date), non-empty spans only, ordered by start.
/// Shared by history (`member_names`) and retention (`settle_profile`, `revoke_membership`),
/// so "when did it end" and "what did it hold" read the same facts.
pub(crate) async fn held_roles(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<HeldRole>>, MembershipError> {
    let assignments: Vec<AssignmentRow> = sqlx::query_as(
        "select ra.membership_id, r.name, r.capability_class,
                to_char(ra.starts_on, 'YYYY-MM-DD'), to_char(ra.ends_on_exclusive, 'YYYY-MM-DD'),
                (extract(epoch from ra.revoked_at) * 1000000)::bigint
           from role_assignments ra
           join roles r on r.tenant_id = ra.tenant_id and r.id = ra.role_id
          where ra.tenant_id = $1 and ra.membership_id = any($2)
          order by ra.membership_id, ra.starts_on, ra.id",
    )
    .bind(tenant_id)
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    let mut held: HashMap<Uuid, Vec<HeldRole>> = HashMap::new();
    for (membership, name, class, starts, ends, revoked_us) in assignments {
        let from = parse_date(&starts)?;
        let mut until = parse_date(&ends)?;
        if let Some(us) = revoked_us {
            until = until.min(oslo_today(from_micros(us)?));
        }
        if from < until {
            held.entry(membership).or_default().push(HeldRole {
                name,
                class: CapabilityClass::from_code(&class).ok_or_else(MembershipError::decode)?,
                from,
                until,
            });
        }
    }
    Ok(held)
}
```

In `member_names`, replace the moved code with `let mut held = held_roles(&mut tx, viewer.tenant_id, &ended).await?;` before `tx.commit()`. Add `PgConnection` to the sqlx import. Run `cargo test -p fau-app --test directory_retention`; it is green as before.

- [ ] **Step 4: Write `settle_profile`, `clear_profile` and `account_retention`, and rewrite the sweep and the erasure, in `retention.rs`**

Replace the module doc comment's first two bullets with the #3511 rule, and add `profile_retained_until = null` to `erase_member_names`'s `set` clause. Then:

```rust
use fau_domain::directory::history::ended_on;
use fau_domain::membership::retention::{keep_until, RetentionMonths};
use jiff::civil::Date;
use sqlx::PgConnection;

use super::names::held_roles;
use super::sql::parse_date;

/// Where a membership's name and address stand after [`settle_profile`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Settled {
    /// The membership is active; its fields, if any, are its own.
    Active,
    /// Ended, and within its period: kept, hidden, until `profile_retained_until`.
    Retained,
    /// Ended and past its period (or with a period of none): cleared just now.
    Cleared,
    /// Nothing to keep: no field, or an erasure.
    Absent,
}

/// The account's chosen period, read through the membership.
pub(crate) async fn account_retention(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
) -> Result<RetentionMonths, MembershipError> {
    let months: i32 = sqlx::query_scalar(
        "select a.retention_months from memberships m join accounts a on a.id = m.account_id
          where m.tenant_id = $1 and m.id = $2",
    )
    .bind(tenant_id)
    .bind(membership_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(MembershipError::UnknownMembership)?;
    RetentionMonths::from_months(months).ok_or_else(MembershipError::decode)
}

/// The date an ended membership's fields go: the stamped one, or, for an end not yet
/// noticed, `keep_until` from the day it ended (plan Ruling R2). `None`: clear now.
pub(crate) async fn effective_keep_until(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
    stamped: Option<&str>,
    today: Date,
) -> Result<Option<Date>, MembershipError> {
    if let Some(d) = stamped {
        let until = parse_date(d)?;
        return Ok((today < until).then_some(until));
    }
    let months = account_retention(conn, tenant_id, membership_id).await?;
    let held = held_roles(conn, tenant_id, &[membership_id])
        .await?
        .remove(&membership_id)
        .unwrap_or_default();
    Ok(keep_until(ended_on(&held, today), months, today))
}

/// Clears both fields and the date, audited with `cause`. The caller holds the lock.
pub(crate) async fn clear_profile(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
    at: Moment,
    cause: &'static str,
) -> Result<(), MembershipError> {
    sqlx::query(
        "update memberships
            set encrypted_display_name = null, encrypted_contact_email = null,
                profile_retained_until = null
          where tenant_id = $1 and id = $2",
    )
    .bind(tenant_id)
    .bind(membership_id)
    .execute(&mut *conn)
    .await?;
    write_audit(
        conn,
        at,
        Audit::system(
            tenant_id,
            "membership.profile_cleared",
            "membership",
            membership_id,
            json!({ "cause": cause }),
        ),
    )
    .await
}

type SettleRow = (bool, bool, Option<String>, bool);

/// Decides, under the tenant lock the caller holds, whether an ended membership keeps its
/// fields: stamps `profile_retained_until` the first time an end is noticed (audited
/// `membership.profile_retained`), and clears once the period is over (`retention_ended`)
/// or when it had none (`membership_ended`). An active membership is left alone.
pub(crate) async fn settle_profile(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
    at: Moment,
) -> Result<Settled, MembershipError> {
    let today = at.today();
    let row: Option<SettleRow> = sqlx::query_as(&format!(
        "select m.name_erased_at is not null,
                (m.encrypted_display_name is not null or m.encrypted_contact_email is not null),
                to_char(m.profile_retained_until, 'YYYY-MM-DD'),
                {}
           from memberships m
          where m.tenant_id = $1 and m.id = $2 for update",
        membership_ended("$3")
    ))
    .bind(tenant_id)
    .bind(membership_id)
    .bind(date_param(today))
    .fetch_optional(&mut *conn)
    .await?;
    let (erased, has_fields, stamped, ended) = row.ok_or(MembershipError::UnknownMembership)?;
    if erased || !has_fields {
        return Ok(Settled::Absent);
    }
    if !ended {
        return Ok(Settled::Active);
    }
    let until =
        effective_keep_until(conn, tenant_id, membership_id, stamped.as_deref(), today).await?;
    match (until, stamped.is_some()) {
        (Some(_), true) => Ok(Settled::Retained),
        (Some(until), false) => {
            sqlx::query(
                "update memberships set profile_retained_until = $3::date
                  where tenant_id = $1 and id = $2",
            )
            .bind(tenant_id)
            .bind(membership_id)
            .bind(date_param(until))
            .execute(&mut *conn)
            .await?;
            write_audit(
                conn,
                at,
                Audit::system(
                    tenant_id,
                    "membership.profile_retained",
                    "membership",
                    membership_id,
                    json!({ "until": date_param(until) }),
                ),
            )
            .await?;
            Ok(Settled::Retained)
        }
        (None, stamped) => {
            let cause = if stamped || account_retention(conn, tenant_id, membership_id).await?
                != RetentionMonths::None
            {
                "retention_ended"
            } else {
                "membership_ended"
            };
            clear_profile(conn, tenant_id, membership_id, at, cause).await?;
            Ok(Settled::Cleared)
        }
    }
}
```

Note the cause rule: a natural end found by the sweep only after its period was already over (for example a 3-month period and a sweep that has not run for 4 months) is `retention_ended`. Only a period of none gives `membership_ended`.

Rewrite `clear_ended_profiles`:

```rust
/// The daily sweep (#3511): settles every ended membership that still holds a field and
/// whose period is not known to be running: an end not yet noticed is stamped, and an
/// expired period is cleared. Per FAU, under that FAU's lock, each row re-decided under
/// it, so a role granted concurrently is never swept past. Returns how many it cleared.
pub async fn clear_ended_profiles(pool: &PgPool, at: Moment) -> Result<u64, MembershipError> {
    let today = date_param(at.today());
    let candidates = |tenant_filter: &str| {
        format!(
            "select {cols} from memberships m
              where {tenant_filter}
                (m.encrypted_display_name is not null or m.encrypted_contact_email is not null)
                and (m.profile_retained_until is null or m.profile_retained_until <= $1::date)
                and {ended}",
            cols = if tenant_filter.is_empty() { "distinct m.tenant_id" } else { "m.id" },
            ended = membership_ended("$1"),
        )
    };
    let tenants: Vec<Uuid> = sqlx::query_scalar(&(candidates("") + " order by m.tenant_id"))
        .bind(&today)
        .fetch_all(pool)
        .await?;
    let mut cleared = 0;
    for tenant_id in tenants {
        let mut tx = pool.begin().await?;
        lock_tenant(&mut tx, tenant_id).await?;
        let ids: Vec<Uuid> =
            sqlx::query_scalar(&(candidates("m.tenant_id = $2 and") + " order by m.id"))
                .bind(&today)
                .bind(tenant_id)
                .fetch_all(&mut *tx)
                .await?;
        for id in ids {
            if settle_profile(&mut tx, tenant_id, id, at).await? == Settled::Cleared {
                cleared += 1;
            }
        }
        tx.commit().await?;
    }
    Ok(cleared)
}
```

- [ ] **Step 5: Stamp at revocation in `revoke_membership`** (`roles.rs`)

Change the target lookup to also return whether a field is held:

```rust
    let target: Option<(bool, bool)> = sqlx::query_as(
        "select revoked_at is not null,
                (encrypted_display_name is not null or encrypted_contact_email is not null)
           from memberships where tenant_id = $1 and id = $2 for update",
    )
```
and match `Some((true, _))` / `Some((false, has_fields))`.

**Move** the `update memberships set revoked_at ...` statement from before the assignment updates to directly after `revoke_ended_admin_roles_with_open_window`, so that `held_roles` sees the cut-short spans. Replace it with:

```rust
    // #3511: the fields are kept, hidden, for the account's period from the day the
    // membership ended (plan Ruling R2), or cleared now for a period of none. Migration
    // 0009's *_only_while_current_or_retained checks refuse a revocation that keeps a
    // field without the date.
    let months = account_retention(&mut tx, req.tenant_id, req.membership_id).await?;
    let held = held_roles(&mut tx, req.tenant_id, &[req.membership_id])
        .await?
        .remove(&req.membership_id)
        .unwrap_or_default();
    let keep = keep_until(ended_on(&held, at.today()), months, at.today()).filter(|_| has_fields);
    sqlx::query(
        "update memberships
            set revoked_at = $3::timestamptz,
                profile_retained_until = $4::date,
                encrypted_display_name  = case when $4::date is null then null else encrypted_display_name end,
                encrypted_contact_email = case when $4::date is null then null else encrypted_contact_email end
          where tenant_id = $1 and id = $2",
    )
    .bind(req.tenant_id)
    .bind(req.membership_id)
    .bind(&now)
    .bind(keep.map(date_param))
    .execute(&mut *tx)
    .await?;
    match keep {
        Some(until) => {
            write_audit(
                &mut tx,
                at,
                Audit::system(
                    req.tenant_id,
                    "membership.profile_retained",
                    "membership",
                    req.membership_id,
                    json!({ "until": date_param(until) }),
                ),
            )
            .await?
        }
        None if has_fields => {
            write_audit(
                &mut tx,
                at,
                Audit::system(
                    req.tenant_id,
                    "membership.profile_cleared",
                    "membership",
                    req.membership_id,
                    json!({ "cause": "membership_ended" }),
                ),
            )
            .await?
        }
        None => {}
    }
```

Imports: `use fau_domain::directory::history::ended_on;`, `use fau_domain::membership::retention::keep_until;`, `use super::names::held_roles;`, `use super::retention::account_retention;`.

Check that `date_param` accepts a `Date` by value for `.map(date_param)`. If it takes a reference, write `.map(|d| date_param(d))`.

- [ ] **Step 6: Run the new tests**

Run: `cargo test -p fau-app --test member_retention`
Expected: all 6 pass.

- [ ] **Step 7: Update the #3502 tests whose expectations change under #3511**

Run: `cargo test -p fau-app --test directory_retention --test member_profile 2>&1 | grep -E "^test .*FAILED|panicked"`

Change each failure as below. Do not weaken any other assertion. If a test fails in a way this list does not cover, stop and report it.
- `directory_retention::the_sweep_clears_the_name_and_address_once_no_role_runs_or_is_still_to_come`: rename to `..._with_a_period_of_none`, and at the start call `set_months_by_sql` (copy the helper) with `0` for every address the test joins. Its counts and audits then hold as written.
- `directory_retention::erasure_shows_former_member_in_every_fau_even_after_the_membership_ended`: if it asserts `(None, None)` after a revocation, set those accounts to 0 months the same way.
- `member_profile::revoking_a_membership_clears_its_name_and_contact_address`: rename to `revoking_a_membership_with_a_period_of_none_clears_both_fields`, and set kari's `retention_months` to 0 before revoking. Add a sibling `revoking_a_membership_keeps_both_fields_for_the_default_period` that asserts `(Some(envelope(3)), Some(envelope(4)))` bytes and `profile_retained_until = '2026-12-23'` (T0 is 2026-09-23).
- Tests that pass a revoked member through re-acceptance are Task 5's; if one fails here, mark it with a `// Task 5` comment and leave it failing for now.

- [ ] **Step 8: Mutation checks**

For each mutation named in Step 1's doc comments: apply it, run `cargo test -p fau-app --test member_retention`, see the named test fail, revert. Record the results in the task report.

- [ ] **Step 9: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace 2>&1 | grep -E "test result|FAILED" | sort | uniq -c
git add -A backend/crates
git commit -m "Keep a former member's fields for their chosen period, then clear them (#3511)

Co-Authored-By: <your trailer>"
```
The suite must be green apart from any tests marked `// Task 5`. List those in the report.

---

### Task 5: Coming back — restore if retained, clear if expired

**Files:**
- Modify: `backend/crates/persistence/src/membership/retention.rs` (`reopen_profile`)
- Modify: `backend/crates/persistence/src/membership/sql.rs` (`ensure_membership`)
- Modify: `backend/crates/persistence/src/membership/invitations.rs` (`accept_invitation` call, `prepare_acceptance`)
- Modify: `backend/crates/persistence/src/membership/signup.rs` (call)
- Modify: `backend/crates/persistence/src/membership/roles.rs` (`grant_role`; delete `clear_if_ended`)
- Modify: `backend/crates/app/tests/member_retention.rs`, `directory_retention.rs`, `member_profile.rs`

**Interfaces:**
- Consumes: `settle_profile`, `Settled`, `effective_keep_until` (Task 4).
- Produces:
  - `pub(crate) async fn reopen_profile(conn: &mut PgConnection, tenant_id: Uuid, membership_id: Uuid, at: Moment) -> Result<Settled, MembershipError>`.
  - `ensure_membership(conn, tenant_id, account_id, expected_id, at: Moment) -> Result<(Uuid, bool, bool), MembershipError>`. The parameter changes from `today: Date` to `at: Moment`, and the third value now means "holds a current or retained profile".
  - `AcceptanceTarget::existing_current` now means current *or retained*.

- [ ] **Step 1: Write the failing tests** — append to `member_retention.rs`:

```rust
async fn reinvite(pool: &PgPool, fau: &Fau, address: &str, role: RoleChoice, p: Period, at_: &str) -> (bool, Uuid) {
    let issued = issue_invitation(
        pool,
        IssueInvitation {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            recipient: email(address),
            roles: vec![OfferedRole { role, period: p }],
            handover_grant_id: None,
            message: None,
        },
        at(at_),
    )
    .await
    .unwrap();
    let token = issued.token.expose().to_owned();
    let target = prepare_acceptance(pool, &token, &verified(address), at(at_)).await.unwrap();
    let accepted = accept_invitation(
        pool,
        AcceptInvitation {
            token,
            acceptor: verified(address),
            admin_end_override: None,
            // A returner states a name again; the address is left unstated.
            profile: MemberProfile {
                membership_id: target.membership_id,
                encrypted_display_name: envelope(9),
                encrypted_contact_email: None,
            },
        },
        at(at_),
    )
    .await
    .unwrap();
    (target.existing_current, accepted.membership_id)
}

async fn contact(pool: &PgPool, m: Uuid) -> Option<Vec<u8>> {
    sqlx::query_scalar("select encrypted_contact_email from memberships where id = $1")
        .bind(m)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Spec §6 "a return within the period": the same row, recognised (`existing_current`),
/// the retained address kept, the date cleared, `profile_restored` audited. Returning as
/// a guest, the person sees only their own group (spec §6 "a guest who returns").
///
/// Mutation checks: drop the `settle` in `reopen_profile` and an expired return keeps the
/// old address (the next test); drop the `profile_retained_until = null` and 0009's
/// checks are fine but the sweep clears a returner (assert below).
#[tokio::test]
async fn a_guest_returning_within_the_period_is_recognised_and_sees_only_their_group() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let m = join(&pool, &a, "back@example.test", year()).await;
    with_contact(&pool, &a, m, 1).await;
    leave(&pool, &a, m, "2026-10-01T10:00:00Z").await;

    let group = create_test_group(&pool, &a, "Dugnad").await; // see note below
    let guest = RoleChoice::New {
        name: RoleName::parse("Gjest").unwrap(),
        capability: CapabilityClass::Guest,
        group_id: Some(group),
    };
    let (recognised, again) = reinvite(
        &pool, &a, "back@example.test", guest,
        period(day(2026, 11, 1), day(2027, 2, 1)), "2026-11-01T10:00:00Z",
    )
    .await;
    assert!(recognised, "within the period: existing_current");
    assert_eq!(again, m, "the same row");
    assert_eq!(contact(&pool, m).await, Some(envelope(1).as_bytes().to_vec()), "address kept");
    assert_eq!(state(&pool, m).await.2, None, "no longer retained");
    assert!(audits(&pool, m).await.iter().any(|(a, _)| a == "membership.profile_restored"));
    // The sweep after the old period's end leaves an active returner alone.
    clear_ended_profiles(&pool, at("2027-01-05T10:00:00Z")).await.unwrap();
    assert_eq!(state(&pool, m).await, (true, true, None));
    // A guest reaches only their own group.
    let viewer = Viewer { tenant_id: a.tenant_id, membership_id: m };
    let groups = list_groups(&pool, viewer, at("2026-11-02T10:00:00Z")).await.unwrap();
    assert_eq!(groups.iter().map(|g| g.id).collect::<Vec<_>>(), [group]);
}

/// Spec §6 "a return after the period": expired first, so nothing comes back and the
/// person is not `existing_current`.
#[tokio::test]
async fn returning_after_the_period_is_a_fresh_start() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let m = join(&pool, &a, "late@example.test", year()).await;
    with_contact(&pool, &a, m, 1).await;
    leave(&pool, &a, m, "2026-10-01T10:00:00Z").await;
    // No sweep has run since the period ended on 2027-01-01.
    let (recognised, again) = reinvite(
        &pool, &a, "late@example.test", new_role("Medlem", CapabilityClass::Member),
        period(day(2027, 2, 1), day(2028, 2, 1)), "2027-02-01T10:00:00Z",
    )
    .await;
    assert!(!recognised);
    assert_eq!(again, m);
    assert_eq!(contact(&pool, m).await, None, "the expired address is gone");
    assert!(audits(&pool, m).await.iter().any(|(a, p)|
        a == "membership.profile_cleared" && p.contains("retention_ended")));
}

/// `grant_role` to a membership whose roles ran out: restore if retained, clear if expired
/// (spec §3, replacing #3502's clear_if_ended).
#[tokio::test]
async fn a_grant_restores_a_retained_profile_and_clears_an_expired_one() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let short = period(day(2026, 9, 1), day(2026, 10, 1));
    let kept = join(&pool, &a, "kept@example.test", short).await;
    let gone = join(&pool, &a, "gone@example.test", short).await;
    for m in [kept, gone] {
        with_contact(&pool, &a, m, 1).await;
    }
    let grant_at = |m: Uuid, at_: &'static str, p: Period| {
        let pool = pool.clone();
        let tenant_id = a.tenant_id;
        let admin = a.admin_membership_id;
        async move {
            grant_role(
                &pool,
                GrantRole {
                    tenant_id,
                    actor_membership_id: admin,
                    membership_id: m,
                    role: new_role("Nytt verv", CapabilityClass::Member),
                    period: p,
                },
                at(at_),
            )
            .await
            .unwrap();
        }
    };
    grant_at(kept, "2026-11-01T10:00:00Z", period(day(2026, 11, 1), day(2027, 11, 1))).await;
    assert_eq!(state(&pool, kept).await, (true, true, None));
    grant_at(gone, "2027-02-01T10:00:00Z", period(day(2027, 2, 1), day(2028, 2, 1))).await;
    assert_eq!(state(&pool, gone).await, (false, false, None));
}
```

The group helper: use whatever `common::groups` offers for creating a group as the FAU admin (`create_group` with an `envelope` name, as `groups.rs` tests do). Add `use common::groups::*;` if it helps, and name the helper after what exists. Add `use fau_domain::membership::vocabulary::RoleName;`. `list_groups`'s return type holds each group's id. Check the field name in `GroupView`.

- [ ] **Step 2: Run and see them fail**

Run: `cargo test -p fau-app --test member_retention`
Expected: the within-period test FAILS on `existing_current` (false) and on the date still being set. The grant test FAILS on the restored `kept` row, whose fields #3502's `clear_if_ended` clears.

- [ ] **Step 3: Write `reopen_profile`** (`retention.rs`):

```rust
/// A returner (#3511 §3, plan Ruling R4): settles the profile -- an expired period is
/// cleared, a retained one kept -- then makes the row current again in one statement
/// (`revoked_at` and `profile_retained_until` null together, as migration 0009's checks
/// require), and audits `membership.profile_restored` when something was retained. The
/// caller holds the tenant lock and adds the role in the same transaction.
pub(crate) async fn reopen_profile(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
    at: Moment,
) -> Result<Settled, MembershipError> {
    let settled = settle_profile(conn, tenant_id, membership_id, at).await?;
    sqlx::query(
        "update memberships set revoked_at = null, profile_retained_until = null
          where tenant_id = $1 and id = $2",
    )
    .bind(tenant_id)
    .bind(membership_id)
    .execute(&mut *conn)
    .await?;
    if settled == Settled::Retained {
        write_audit(
            conn,
            at,
            Audit::system(
                tenant_id,
                "membership.profile_restored",
                "membership",
                membership_id,
                json!({}),
            ),
        )
        .await?;
    }
    Ok(settled)
}
```

- [ ] **Step 4: Use it in `ensure_membership`** (`sql.rs`)

Change the parameter `today: Date` to `at: Moment` (bind `date_param(at.today())`). The existing-row branch becomes:

```rust
    if let Some((id, _was_revoked, erased, _ended)) = existing {
        if id != expected_id {
            return Err(MembershipError::AcceptanceTargetChanged);
        }
        if erased {
            return Err(MembershipError::MembershipErased);
        }
        let settled = super::retention::reopen_profile(conn, tenant_id, id, at).await?;
        return Ok((id, true, matches!(settled, Settled::Active | Settled::Retained)));
    }
```

Trim the select to the columns still used, or keep them and prefix with `_`. Rewrite the doc comment's `already_current` paragraph: it is now true for a row whose profile is current or retained, so that `write_profile` keeps a retained address the returner leaves unstated (spec §3). Update both callers (`invitations.rs:879`, `signup.rs:381`) to pass `at`.

Note: `settled == Active` for a row whose fields are all null returns `Absent`, not `Active`, so `already_current` is false there. It has no address to keep, which is harmless, and it matches `write_profile`'s contract.

- [ ] **Step 5: Use it in `grant_role`** (`roles.rs`)

Replace `clear_if_ended(&mut tx, req.tenant_id, req.membership_id, at).await?;` with `reopen_profile(&mut tx, req.tenant_id, req.membership_id, at).await?;`, and delete `clear_if_ended` with its doc comment. `grant_role` already refuses a revoked membership, so `revoked_at = null` is a no-op here.

- [ ] **Step 6: `prepare_acceptance` reports "current or retained"** (`invitations.rs`)

Replace the select and the mapping:

```rust
    let sql = format!(
        "select m.id, m.name_erased_at is not null, {ended},
                m.encrypted_display_name is not null,
                to_char(m.profile_retained_until, 'YYYY-MM-DD')
           from memberships m join accounts a on a.id = m.account_id
          where m.tenant_id = $1 and a.email = $2",
        ended = super::profile::membership_ended("$3")
    );
    let existing: Option<(Uuid, bool, bool, bool, Option<String>)> = sqlx::query_as(&sql)
        .bind(tenant_id)
        .bind(acceptor.email().as_str())
        .bind(date_param(at.today()))
        .fetch_optional(&mut *tx)
        .await?;
    let target = match existing {
        None => (Uuid::now_v7(), false),
        Some((id, erased, ended, named, stamped)) => {
            let current = !erased
                && (!ended
                    || (named
                        && super::retention::effective_keep_until(
                            &mut tx, tenant_id, id, stamped.as_deref(), at.today(),
                        )
                        .await?
                        .is_some()));
            (id, current)
        }
    };
    tx.commit().await?;
    let (membership_id, existing_current) = target;
```

Rewrite the comment above it: #3511 widens "current" to "current or retained" (spec §3, plan Ruling R7), computed exactly as `settle_profile` will compute it under the lock. Also update the `existing_current` field's doc on `AcceptanceTarget`.

- [ ] **Step 7: Run everything**

Run: `cargo test -p fau-app --test member_retention --test directory_retention --test member_profile --test handover_recovery --test invitations --test signup`

Update the remaining #3502 tests, and only these, to the #3511 expectation:
- `directory_retention::granting_a_role_to_an_ended_membership_clears_what_the_sweep_has_not`: set the account to 0 months first and rename to `..._with_a_period_of_none`. Its audit assertion holds as written.
- `member_profile::a_membership_whose_roles_ran_out_reports_existing_current_false_and_drops_its_address`: split it. With 0 months it keeps its current assertions (rename `..._with_a_period_of_none`). Add a default-period twin asserting `existing_current == true` and the address kept.
- `member_profile::a_membership_revoked_by_other_means_still_counts_as_ended`: it revokes by SQL. If the revoking statement now fails on 0009's check, add `, profile_retained_until = '2026-12-23'` to that statement so it models a retained revocation, and assert `existing_current == true`. Keep the test's intent, that a revoked row counts as ended for the directory, by also asserting that `member_names` returns `Ended`.
- Any test marked `// Task 5` in Task 4.

Expected at the end: `cargo test --workspace` green.

- [ ] **Step 8: Mutation checks** from Step 1's doc comments. Apply each, see the named test fail, revert, and record the result.

- [ ] **Step 9: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A backend/crates
git commit -m "Recognise a member who returns within their period; clear an expired one (#3511)

Co-Authored-By: <your trailer>"
```

---

### Task 6: History labels by membership period

**Files:**
- Modify: `backend/crates/persistence/src/membership/names.rs`
- Modify: `backend/crates/app/tests/member_retention.rs`

**Interfaces:**
- Consumes: `held_roles` (Task 4), `active_period`, `ActivePeriod` (Task 2).
- Produces: `MemberName::Returned { name: Ciphertext, period: ActivePeriod }`, a new variant. Renderers call `period.label_on(event_date)`: `None` shows the decrypted name, `Some(label)` shows the role and year.

- [ ] **Step 1: Write the failing test** — append to `member_retention.rs`:

```rust
/// M1 and spec §6 "history labels for events from an earlier active period after a
/// return": the returner is `Returned`; an event from the old period labels as the role
/// and year, an event from the new one as the name. A member with one unbroken period
/// stays `Named`.
///
/// Mutation check: return `Named` for every active row (0008's behaviour) and the first
/// assertion fails.
#[tokio::test]
async fn after_a_return_old_events_keep_role_and_year_and_new_ones_show_the_name() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let m = join(&pool, &a, "back@example.test", year()).await;
    leave(&pool, &a, m, "2026-10-01T10:00:00Z").await;
    reinvite(
        &pool, &a, "back@example.test", new_role("Sekretær", CapabilityClass::Member),
        period(day(2026, 11, 1), day(2027, 11, 1)), "2026-11-01T10:00:00Z",
    )
    .await;
    let viewer = Viewer { tenant_id: a.tenant_id, membership_id: a.admin_membership_id };
    let names = member_names(&pool, viewer, &[m, a.admin_membership_id], at("2026-11-02T10:00:00Z"))
        .await
        .unwrap();
    let MemberName::Returned { name, period: p } = &names[0].1 else {
        panic!("{:?}", names[0].1)
    };
    assert_eq!(name, &envelope(9));
    assert_eq!(p.since, day(2026, 11, 1));
    let old = p.label_on(day(2026, 9, 15)).expect("role and year");
    assert_eq!((old.role.as_str(), old.first_year, old.last_year), ("Medlem", 2026, 2026));
    assert_eq!(p.label_on(day(2026, 11, 1)), None, "the new period shows the name");
    assert!(matches!(names[1].1, MemberName::Named(_)), "one unbroken period");
}
```

- [ ] **Step 2: Run and see it fail**

Run: `cargo test -p fau-app --test member_retention after_a_return`
Expected: compile error, no variant `Returned`.

- [ ] **Step 3: Implement** in `names.rs`

Add to the enum, after `Unnamed`:

```rust
    /// An active membership that had an earlier period (#3511, Erik's M1): its name,
    /// encrypted as for `Named`, for events on or after `period.since`, and role and year
    /// for anything earlier (`period.label_on`). Others never see the returner's name on
    /// their old contributions. Never produced by the directory.
    Returned { name: Ciphertext, period: ActivePeriod },
```

In `member_names`, load held roles for every non-erased row instead of only the ended ones:

```rust
    let wanted: Vec<Uuid> = rows
        .iter()
        .filter(|(_, _, erased, _)| !erased)
        .map(|(id, ..)| *id)
        .collect();
    let mut held = held_roles(&mut tx, viewer.tenant_id, &wanted).await?;
```

and the active arm becomes:

```rust
            (false, false) => {
                let period = active_period(held.get(id).map_or(&[][..], Vec::as_slice), at.today());
                match name {
                    Some(n) if !period.earlier.is_empty() => MemberName::Returned {
                        name: Ciphertext::from_stored(n.clone()),
                        period,
                    },
                    Some(n) => MemberName::Named(Ciphertext::from_stored(n.clone())),
                    None => MemberName::Unnamed,
                }
            }
```

Update the module doc comment with a fourth bullet: "an active membership with an earlier period is `Returned`: its name for the current period only (#3511, M1)". Update the enum's doc to match. Import `fau_domain::directory::history::{active_period, ActivePeriod}`.

- [ ] **Step 4: Run and see it pass, and check the other `MemberName` matches**

Run: `cargo test -p fau-app --test member_retention --test directory_retention --test directory --test key_chain`
Expected: pass. `key_chain.rs` matches `MemberName::Named` at lines 389 and 461. If either `match` is exhaustive without a wildcard, add a `Returned { .. }` arm that fails the test: those tests never produce one.

- [ ] **Step 5: Mutation check** from Step 1's doc comment. Record the result.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A backend/crates
git commit -m "Label history by membership period so a returner's old work keeps role and year (#3511)

Co-Authored-By: <your trailer>"
```

---

### Task 7: The member's own setting

**Files:**
- Modify: `backend/crates/persistence/src/membership/retention.rs` (`set_retention_months`)
- Modify: `backend/crates/persistence/src/membership/error.rs` (`UnknownAccount`)
- Modify: `backend/crates/persistence/src/membership/mod.rs` (export)
- Modify: `backend/crates/app/tests/member_retention.rs`

**Interfaces:**
- Consumes: `effective_keep_until`, `clear_profile`, `held_roles` (Task 4); `RetentionMonths`, `keep_until`, `ended_on`.
- Produces: `pub async fn set_retention_months(pool: &PgPool, account_id: Uuid, months: RetentionMonths, at: Moment) -> Result<(), MembershipError>`, re-exported from `fau_persistence::membership`, and `MembershipError::UnknownAccount` (`#[error("account not found")]`).

- [ ] **Step 1: Write the failing tests** — append to `member_retention.rs`:

```rust
use fau_domain::membership::retention::RetentionMonths;

async fn account_of(pool: &PgPool, m: Uuid) -> Uuid {
    sqlx::query_scalar("select account_id from memberships where id = $1")
        .bind(m)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Spec §6 "changing the setting shortens an active retention", plan Ruling R8: shorter
/// moves the date, none clears at once, longer extends a running period, and an already
/// expired one is never revived. Across FAU-er.
///
/// Mutation check: drop the `stamped <= today` clear branch and the expired row is
/// extended to 2028 instead of cleared.
#[tokio::test]
async fn the_setting_recalculates_every_running_period_and_never_revives_an_expired_one() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin-a@example.test", at(T0)).await;
    let b = active_fau(&pool, "admin-b@example.test", at(T0)).await;
    let in_a = join(&pool, &a, "kari@example.test", year()).await;
    let in_b = join(&pool, &b, "kari@example.test", year()).await;
    leave(&pool, &a, in_a, "2026-10-01T10:00:00Z").await; // until 2027-01-01
    leave(&pool, &b, in_b, "2026-10-01T10:00:00Z").await;
    let kari = account_of(&pool, in_a).await;
    let nov = at("2026-11-01T10:00:00Z");

    set_retention_months(&pool, kari, RetentionMonths::TwentyFour, nov).await.unwrap();
    assert_eq!(state(&pool, in_a).await.2.as_deref(), Some("2028-10-01"), "extended");
    assert_eq!(state(&pool, in_b).await.2.as_deref(), Some("2028-10-01"), "every FAU");
    assert!(audits(&pool, in_a).await.iter().any(|(a, p)|
        a == "membership.retention_changed" && p.contains("2028-10-01")));

    set_retention_months(&pool, kari, RetentionMonths::Three, nov).await.unwrap();
    assert_eq!(state(&pool, in_a).await.2.as_deref(), Some("2027-01-01"), "shortened");

    // One month after leaving, a period of three is running; none clears it now.
    set_retention_months(&pool, kari, RetentionMonths::None, nov).await.unwrap();
    assert_eq!(state(&pool, in_a).await, (false, false, None));
    assert!(audits(&pool, in_a).await.iter().any(|(a, p)|
        a == "membership.profile_cleared" && p.contains("retention_shortened")));
    let months: i32 = sqlx::query_scalar("select retention_months from accounts where id = $1")
        .bind(kari)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(months, 0);
}

#[tokio::test]
async fn an_expired_period_is_cleared_not_extended() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let m = join(&pool, &a, "late@example.test", year()).await;
    leave(&pool, &a, m, "2026-10-01T10:00:00Z").await; // until 2027-01-01, no sweep since
    let acc = account_of(&pool, m).await;
    set_retention_months(&pool, acc, RetentionMonths::TwentyFour, at("2027-02-01T10:00:00Z"))
        .await
        .unwrap();
    assert_eq!(state(&pool, m).await, (false, false, None));
}

#[tokio::test]
async fn an_unknown_account_is_refused() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    assert_eq!(
        set_retention_months(&pool, Uuid::now_v7(), RetentionMonths::Six, at(T0)).await,
        Err(MembershipError::UnknownAccount)
    );
}
```

If `MembershipError` does not derive `PartialEq`, use `matches!`. It does derive it in `error.rs`'s tests.

- [ ] **Step 2: Run and see them fail**

Run: `cargo test -p fau-app --test member_retention setting expired unknown_account`
Expected: compile errors, `set_retention_months` not found.

- [ ] **Step 3: Implement**

Add `UnknownAccount` to `error.rs` under "Roles and memberships":
```rust
    #[error("account not found")]
    UnknownAccount,
```

In `retention.rs`:

```rust
/// The member's own setting (#3511 §4, plan Ruling R8): how long a membership of theirs is
/// remembered after it ends. `account_id` is the verified login's account (#3417's
/// session), so a member only ever sets their own.
///
/// Recalculates every period the account's memberships are in, in every FAU, from the day
/// each ended: a longer setting extends a running period, a shorter one shortens it, and a
/// period that is now over is cleared at once (`retention_shortened`). A period that had
/// already expired is cleared as expired, never extended. Each moved period is audited as
/// `membership.retention_changed` by the member.
///
/// Locks: the FAU-er are locked in id order *before* the account row is written, so this
/// never holds the account row while waiting for a tenant lock (`accept_invitation` takes
/// them the other way round). As for `erase_member_names`, the list is read before the
/// locks; a membership created meanwhile in another FAU is active and has no period yet,
/// so the new setting applies when it ends.
pub async fn set_retention_months(
    pool: &PgPool,
    account_id: Uuid,
    months: RetentionMonths,
    at: Moment,
) -> Result<(), MembershipError> {
    let today = at.today();
    let mut tx = pool.begin().await?;
    let tenants: Vec<Uuid> = sqlx::query_scalar(
        "select distinct tenant_id from memberships
          where account_id = $1 and profile_retained_until is not null
          order by tenant_id",
    )
    .bind(account_id)
    .fetch_all(&mut *tx)
    .await?;
    for tenant_id in &tenants {
        lock_tenant(&mut tx, *tenant_id).await?;
    }
    let found: Option<Uuid> =
        sqlx::query_scalar("update accounts set retention_months = $2 where id = $1 returning id")
            .bind(account_id)
            .bind(months.months())
            .fetch_optional(&mut *tx)
            .await?;
    found.ok_or(MembershipError::UnknownAccount)?;

    for tenant_id in tenants {
        let rows: Vec<(Uuid, String)> = sqlx::query_as(
            "select id, to_char(profile_retained_until, 'YYYY-MM-DD') from memberships
              where tenant_id = $1 and account_id = $2 and profile_retained_until is not null
              order by id",
        )
        .bind(tenant_id)
        .bind(account_id)
        .fetch_all(&mut *tx)
        .await?;
        for (id, stamped) in rows {
            if parse_date(&stamped)? <= today {
                clear_profile(&mut tx, tenant_id, id, at, "retention_ended").await?;
                continue;
            }
            let held = held_roles(&mut tx, tenant_id, &[id])
                .await?
                .remove(&id)
                .unwrap_or_default();
            match keep_until(ended_on(&held, today), months, today) {
                None => clear_profile(&mut tx, tenant_id, id, at, "retention_shortened").await?,
                Some(until) if date_param(until) != stamped => {
                    sqlx::query(
                        "update memberships set profile_retained_until = $3::date
                          where tenant_id = $1 and id = $2",
                    )
                    .bind(tenant_id)
                    .bind(id)
                    .bind(date_param(until))
                    .execute(&mut *tx)
                    .await?;
                    write_audit(
                        &mut tx,
                        at,
                        Audit::member(
                            tenant_id,
                            id,
                            "membership.retention_changed",
                            "membership",
                            id,
                            json!({ "until": date_param(until) }),
                        ),
                    )
                    .await?;
                }
                Some(_) => {}
            }
        }
    }
    tx.commit().await?;
    Ok(())
}
```

**Why `ended_on` recomputed from held roles is the same day as at stamping:** once a membership has ended, no role span changes. A new grant or acceptance goes through `reopen_profile`, which nulls the date, so a row with a date has had no role added since it ended. Put this sentence in a comment above the `held_roles` call.

Export `set_retention_months` from `mod.rs` next to `clear_ended_profiles`.

Check that `date_param` returns `String`. The comparison `date_param(until) != stamped` relies on that and on both being `YYYY-MM-DD`.

- [ ] **Step 4: Run and see them pass**

Run: `cargo test -p fau-app --test member_retention`
Expected: all pass.

- [ ] **Step 5: Mutation check** from Step 1's doc comment. Record the result.

- [ ] **Step 6: Full suite and commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace 2>&1 | grep -E "test result|FAILED" | sort | uniq -c
git add -A backend/crates
git commit -m "Let a member choose how long a former membership is remembered (#3511)

Co-Authored-By: <your trailer>"
```

---

### Task 8: Docs and record

**Files:**
- Modify: `backend/crates/persistence/src/membership/profile.rs` (module doc's D3 paragraph, `membership_ended` doc), `backend/crates/persistence/src/membership/retention.rs` (module doc), `backend/migrations/0008_member_directory.sql` — **not edited** (applied). The amendment is described in 0009's header, which Task 3 wrote.
- Modify: `docs/member-retention-design.md` (Status line)
- Modify: `docs/planning-decisions.md` (a "#3511 built" section)

- [ ] **Step 1: Sweep the doc comments that still state D3's "neither field outlives the membership"**

Run: `grep -rn "outlives\|D3" backend/crates/persistence/src backend/crates/domain/src | cat`

In each hit, change the statement to the #3511 rule: an ended membership's fields are kept, hidden, for the account's period, then cleared; others see role and year throughout. Keep the D3 reference and add "as amended by #3511 (M1–M4, 29 September 2026)". Do not touch `0008_member_directory.sql`.

- [ ] **Step 2: Mark the spec built** — in `docs/member-retention-design.md` change the Status line to: `Status: **agreed in chat with Erik, 29 September 2026; built on branch retention-3511, not merged.**`

- [ ] **Step 3: Record in `docs/planning-decisions.md`** — append a section `## #3511 built: remembering a former member — <date>` listing:
  - plan Rulings R1–R10, one line each;
  - the privacy text owed to #3426 (spec §5);
  - Erik's M5 (R10): the account lapse follows `retention_months`, which binds #3426;
  - the Bokmål source strings the screen will need, as catalogue entries only: `retention.setting.label` = "Hvor lenge skal vi huske deg etter at du går ut?", with options `retention.none` = "Ikke i det hele tatt", `retention.months` = "{months} måneder" and `retention.default` = "(standard)". Mark them "proposed; the screen card (#3417) owns the final text".

- [ ] **Step 4: Commit**

```bash
git add docs/member-retention-design.md docs/planning-decisions.md backend/crates
git commit -m "Record #3511: retention rulings, privacy text owed to #3426 (#3511)

Co-Authored-By: <your trailer>"
```

---

## Self-review

**Spec coverage** (`docs/member-retention-design.md`):
- §3 states: Active/Retained/Expired/Erased map to `Settled::{Active, Retained, Cleared}` and the erasure. Task 4.
- §3 return within or after the period, the `existing_current` widening, and `grant_role`'s "restore if retained, clear if expired": Task 5.
- §4 migration 0009, all three bullets: Task 3.
- §4 ending, the sweep and the reads-unchanged rule: Task 4 (`while_retained_others_see_only_role_and_year`).
- §4 history labels by period: Task 6.
- §4 the member's own setting, with no extension of an expired period: Task 7.
- §4 erasure: Task 4.
- §4 account email lapse: Ruling R10, noted on #3426 in Task 8.
- §5 privacy: Task 8 records it for #3426; no code.
- §6 tests:
  - each period value: Task 4 `leaving_keeps_…`;
  - the sweep before and after: Task 4;
  - return within: Task 5;
  - return after: Task 5;
  - 0 months: Task 4;
  - erasure during retention: Task 4;
  - cross-tenant: Task 4 `retention_is_per_fau` and Task 7;
  - the guest returner: Task 5;
  - the setting shortens: Task 7;
  - history labels: Task 6.

**Type consistency:**
- `RetentionMonths`, `keep_until`, `retained_until` (Task 1).
- `ended_on`, `ActivePeriod { since, earlier }`, `active_period`, `label_on` (Task 2).
- `held_roles`, `Settled`, `settle_profile`, `clear_profile`, `account_retention`, `effective_keep_until` (Task 4).
- `reopen_profile`, and `ensure_membership(…, at: Moment)` (Task 5).
- `MemberName::Returned { name, period }` (Task 6).
- `set_retention_months(pool, account_id, months, at)` and `UnknownAccount` (Task 7).

**Known judgement calls an executor must not "fix" silently:** R6 (period boundaries come from role dates, not grant times), and the sweep counting only clearings, not stampings. Raise either with the controller if a test disagrees.
