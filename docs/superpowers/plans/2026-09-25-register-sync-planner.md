# Register Sync Planner Implementation Plan (#3441, part 3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `fau_domain::register::sync`, a pure planner. It reads a snapshot of the register and
the freshly fetched sources, and returns what to create, rename, renumber, move and close, plus the
review items. It also runs the circuit breaker. Nothing here touches SQL, HTTP or mail: part 4
builds the persistence applier and `fau register sync`.

**Architecture:**
- **One module directory**, `crates/domain/src/register/sync/`:
  - `types.rs`: the snapshot, the inputs, the ops, the review items and the outcome;
  - `similarity.rs`: pg_trgm's `similarity()` in Rust, and the two thresholds;
  - `municipalities.rs`: step 1 (renumbers, renames, splits, Svalbard);
  - `schools.rs`: step 5 (creates, renames, moves, closures, re-registrations, submission holds);
  - `mod.rs`: `plan()`, which runs both planners, counts and applies the circuit breaker.
- **Generic over the caller's row id**, as `Id: RowId` (`Copy + Eq + Hash + Ord + Debug`). The domain
  has no uuid dependency: the persistence applier will use `Uuid`, and the tests use `u32`.
- **The tests** build their values by hand from the recorded fixtures' facts
  (`crates/register-sources/tests/fixtures/README.md`), because the domain may not depend on the
  sources crate. A test-only in-memory applier, `testkit::apply`, applies a plan to a snapshot. It
  proves that a second run on the same sources gives `NoChange`, and it is the executable
  specification the part 4 SQL applier must match.

**Tech Stack:** Rust 1.98.1, jiff 0.2 (already a domain dependency), and the existing
`register::{slug, search, scope, source}` modules. No new dependencies.

**Spec:**
- `docs/school-register-design.md` §2.2–2.4 (municipalities, scope, renumbering), §4.5
  (submissions and re-registrations), §5.2–5.3 (order of work, idempotency, circuit breaker) and §6
  (slugs);
- `docs/planning-decisions.md`, the #3441 sections of 24 September 2026: "School register decided"
  (D2, D3, D5, D8) and "Register foundation built" (held rows get no orgnr history; review details
  keep a held row's orgnr and name);
- the controller's design brief for part 3, whose rulings this plan follows except where the table
  below says otherwise;
- the code on branch `school-register-3441`: `crates/domain/src/register/{slug,search,scope,source}.rs`,
  `crates/domain/src/time.rs` and `migrations/0004_school_register.sql`.

## Global Constraints

- `crates/domain` declares no axum, sqlx, tower, hyper or reqwest (`tests/dependency_boundary.rs`),
  and no uuid. This plan adds no dependency at all.
- All technical content (code, comments, test names, commit messages) is in English.
- **Review-item details never carry an address.** They hold ids, codes, names and orgnrs only
  (ruling, 24 September 2026). NSR addresses appear in `SchoolAttributes`, never in `details`.
- The planner never panics on data. `.expect` is used only for true invariants, with the reason in
  its message.
- Code follows the existing domain style: doc comments cite spec sections, value types derive
  `Debug, Clone, PartialEq, Eq` (plus `Copy` for small enums), and there is no `unsafe`.
- **Tests:**
  - `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test --workspace`;
  - `cargo fmt --check`;
  - `cargo clippy --workspace --all-targets -- -D warnings`.

  All three must be clean after every task, and tests are written first. The planner's own tests
  never touch the database.
- **Commits:**
  - identity via env vars: `GIT_AUTHOR_NAME="Erik W. Bjønnes" GIT_AUTHOR_EMAIL=erik@ewb-solutions.as`,
    and the same for the committer;
  - the message ends with exactly `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`;
  - branch `school-register-3441`, never pushed.
- **Transcribe the code exactly.** Every block below was compiled, run, formatted with `cargo fmt`
  and checked with clippy in a scratch copy of the branch, one task at a time. If a test fails, the
  transcription is wrong: diff it against this plan before changing any logic or expected value.

## Decisions this plan makes

| Question | Decision | Reason |
| --- | --- | --- |
| Where the similarity values in the tests come from | PostgreSQL 17.5's pg_trgm 1.6 `similarity()`, run on the `search_query` form of each pair in a throwaway database that was dropped afterwards. The tests hold them as literals. Two are also traced by hand in a comment. | The brief allows it, and it pins the Rust function to the database's own arithmetic. |
| Module layout | A `sync/` directory with five files and a test-only `testkit.rs` | Each file has one step of §5.2, and each fits in one reviewer's head. |
| The `Id` bound | A blanket trait `RowId: Copy + Eq + Hash + Ord + Debug` | One name instead of five bounds on every function. It is implemented for every type that satisfies them. |
| How Tasks 2–4 stay clippy-clean before `plan()` exists | `#[allow(dead_code)]` on `mod municipalities;` and `mod schools;` in `sync/mod.rs`, removed in Task 5 | The alternative is making internal planner functions public API. Task 5's `mod.rs` has no `allow`, and the Self-Review checks it. |
| `MunicipalityOp::Rename.id` | `Id`, not the brief's `Ref<Id>` | Only an existing municipality can be renamed. One created in this run already carries Kartverket's name. |
| `Rename` for a case-only change | `old_slug == new_slug`, and the applier writes no history row | This keeps the brief's field shape. §6: a case-only change changes no slug. |
| Counts | Adds `municipalities_updated` to the brief's list. `schools_created` counts Listed creates, and `schools_held` counts Held creates. | Otherwise an applied run with only `UpdateDetails` would record all zeros, and a held create would be counted twice. |
| `ClosureReason` | Four variants: Closed, Merged, Duplicate, OutOfScope. A test checks each code against 0004's list. | 0004 also allows `rejected`, but only a reviewer sets it, never the sync. |
| How the code lists are pinned to 0004 | A domain unit test reads the migration with `include_str!` and compares the `kind`, `closure_reason`, `verification`, `ownership` and `source` lists | It checks exact equality with no database. The app's insert-style test for `SCOPE_REASON_CODES` can only show that every domain code is accepted. |
| Which existing schools the sync changes | Only Listed and Verified rows. Held, Pending and Rejected rows are left alone, as are Closed ones. | The brief names Held. Pending and Rejected rows await a human too (§4.4). |
| A unit that matches a school only through `school_orgnr_history` | It drives no op. It only stops a duplicate create. | That old number is closed for good. If it drove ops, its closure would close the school that carries on under the new number. |
| An existing school whose orgnr is missing from `units` | Left untouched | The caller fetches every orgnr the register holds, so an absence is a fetch gap, not a closure. A missing unit that closed anything would bypass the circuit breaker's intent. |
| Closure and the unit's municipality number | Closing needs no resolvable number. Only a blocked municipality of the *school* stops it. | NSR keeps a closed unit's old number (Lysaker skole under 1719, §2.1). Otherwise such schools would never close. |
| Out-of-scope units with an unknown number | Raise nothing and are not counted as skipped | 2599 (abroad) would otherwise raise an item every run. `skipped` counts units the planner would have changed or created. |
| `unknown_municipality_number` details | `municipality_number`, `unit_count`, and at most 20 `orgnrs` | Keeps the item under 0004's 2 KB `details` cap. |
| `municipality_split_or_merge` details | Always carries a `reason`: `split`, `merge`, `target_in_use`, `unsluggable`, `unknown_number` or `absent_from_kartverket`. A group item adds `old_code`, `old_name`, `new_codes` and `new_names` (comma-joined, in code order). A per-municipality item adds `number` and `name`. | One review kind covers six situations, so the reviewer needs to know which one it is. |
| A renumber that is already applied | Ignored when no active municipality holds the old code | SSB changes are fed again until a run applies. This keeps a re-run idempotent. |
| A renumber whose target number is already held | Reviewed as `target_in_use`, and the group is blocked | Two entities cannot share a current number (0004's `municipality_numbers_current`). |
| Which municipalities resolve numbers | Active ones only. A dissolved row neither resolves nor gets an "absent from Kartverket" item. | NSR's active units carry current numbers only (§4.1). |
| Svalbard | `official_name: None`, created in both Seed and Sync runs, and never compared with Kartverket | Neither source has an official name for it. The brief's rule 1 is not limited to a seed. |
| Comparing official names | As a set, sorted by priority, language and name | The database returns `municipality_names` in no particular order. |
| Which current slugs count as taken | Every snapshot school's current slug, closed rows included, except the slugs of schools closing in this run | 0004's `schools_slug_current` index covers closed rows too. The applier clears a slug when it closes a school (see the handover), so after that it is free. The brief's "non-closed schools" matches that once the applier holds to its contract. |
| Whether a rename or move frees the old slug in the same run | No | The old slug goes to that school's history, which still counts as taken (§6). |
| Slug allocation order | Re-registration hand-overs first, then renames and moves, then new schools. The ops are ordered closes, creates, then each school's rename, attribute update and move. | A successor gets its predecessor's slug before anything else can take it. Existing schools come before new ones. Only closes release slugs, so any apply order after the closes is safe. |
| A move's slug | Always a `SlugChange` when the school has a slug, even with the same text. A rename in the same run carries no slug, and the move re-mints it in the target municipality. | The history row is keyed on the old municipality (§4.2), so it must be written even when the text is unchanged. |
| Re-registration: which closures qualify | Only NSR codes F ("Slettet for sammenslåing") and S ("Slettet"), and an inactive unit | **The spec wins over the brief here.** §4.5 names exactly those two, and the brief accepted any closure, including an active unit that left scope. |
| Re-registration: the 30-day window | None. The only candidates are schools closing in this run, and schools with an FAU whose unit is closed F or S. | **The brief's ruling, recorded as asked.** It contradicts §4.5's "within 30 days". It is followed because implementing the window changes part 4's contract (new snapshot fields, and an op to link a successor to an already-closed school), which is the controller's call. See the reply for why the ruling is risky. |
| Re-registration: choosing a match | The highest similarity at or above 0.6. Each closing school gets at most one successor. On a tie, the school with an FAU wins, so a human sees it. | Deterministic, and it errs towards review. |
| Re-registration without an FAU | The new school is Listed, takes the old slug, and the old school's `Close` names it as `successor` | As the brief says. |
| Submission match | The most similar Pending or Verified submitted school in the same municipality at or above 0.45. Several units may each match the same submission. | §4.5. Each held row gets its own review item. |
| A name with nothing sluggable | Created with `slug: None`, and no review item | **The brief's ruling, recorded as asked.** D5 curation fixes it. |
| `similarity` in details | Two decimals, e.g. `"0.57"` | Readable in the review CLI. The threshold decides with the full f32. |
| `SchoolAttributes::from_unit` | Visiting address, falling back to the postal address field by field | As the brief says. The reply flags a risk in it. |
| Circuit-breaker tests | Use 100 listed schools. Tests of one-off changes add 99 unchanged "bystander" schools. | With fewer than 50 active schools, a single close trips the 2% rule. That is correct for production, where a seed is exempt, but it would hide what those tests check. |
| `EmptySource` | Checked before anything else, Kartverket first, for both kinds | §5.3: an empty list is an outage. |

---

## Controller rulings on the plan-writer's findings (25 September 2026)

- **Re-registration only follows NSR closure codes F and S** (§4.5), as the plan does. This
  overrides the brief.
- **No 30-day window, kept, with its limitation recorded.** Re-registration is only recognised
  when the old number's closure and the new number arrive in the same run. Otherwise:
  - if the new number appears in an earlier run, while the old school is still active and holds
    the slug, the new school is created Listed with a suffixed slug, and the old one later closes
    with no successor link;
  - if the old number closes in an earlier run, the old school releases its slug when it closes,
    so the new school later takes the bare slug, again with no successor link.

  A suffixed slug only happens while the old school still holds it. Both cases are rare (they
  need the two numbers to straddle a weekly run), are fixable by operator curation (D5) and the
  review screen (#3499), and are left for the matcher/review work, not solved here.
- **A closure with an FAU raises a review item and does not close,** following D8, which is Erik's
  decision. §10's "closes the row" line predates it.
- **Address fallback is whole-address, not field-by-field.** `SchoolAttributes::from_unit` uses
  the visiting address when it has a street, and otherwise the postal address entirely, so a
  postal street never pairs with a visiting postcode. The Task 1 implementer applies this, with a
  test, as a deviation from the plan's code.
- **The circuit breaker stays at 2% / 5%.** On a register with fewer than 50 active schools, one
  closure aborts a sync. Each environment is therefore seeded in full before its first sync, as
  the handover says.

## File Structure

```text
backend/crates/domain/src/register/
  mod.rs                  + pub mod sync; module doc updated (Task 1)
  sync/mod.rs             new: module list and re-exports (Task 1); plan(), mass_change, tally (Task 5)
  sync/types.rs           new: every public type, REVIEW_KIND_CODES, SchoolAttributes::from_unit (Task 1)
  sync/similarity.rs      new: similarity, SUBMISSION_MATCH_THRESHOLD, REREGISTRATION_THRESHOLD (Task 1)
  sync/municipalities.rs  new: MunicipalityState, plan_municipalities (Task 2)
  sync/schools.rs         new: SlugBook, plan_schools (Task 3); re-registration and submission holds (Task 4)
  sync/testkit.rs         new, test-only: builders (Task 2), fixture units (Task 3), apply() (Task 5)
```

No other file changes. `crates/domain/Cargo.toml` is untouched.

---

### Task 1: Types and similarity

**Files:**
- Modify: `backend/crates/domain/src/register/mod.rs`
- Create: `backend/crates/domain/src/register/sync/mod.rs`
- Create: `backend/crates/domain/src/register/sync/types.rs`
- Create: `backend/crates/domain/src/register/sync/similarity.rs`

**Interfaces:**
- Consumes:
  - `register::source::{CodeChange, MunicipalityRecord, NsrUnit, NsrAddress, OfficialName}`;
  - `register::search::search_query(&str) -> String`;
  - `time::Moment`.
- Produces, re-exported from `fau_domain::register::sync`:
  - the trait `RowId`;
  - `Ref<Id> { Existing(Id), New(u32) }`;
  - the snapshot: `RegisterSnapshot<Id>` (implements `Default`), `MunicipalitySnapshot<Id>`,
    `SchoolSnapshot<Id>`, `SchoolAttributes` (with `SchoolAttributes::from_unit(&NsrUnit)`), and the
    enums `MunicipalityStatus`, `MunicipalitySource`, `Origin`, `Verification`, `SchoolStatus` and
    `Ownership`;
  - the inputs: `SyncInputs<'a>` and `RunKind`;
  - the ops: `MunicipalityOp<Id>`, `SchoolOp<Id>`, `SlugChange`, `CreateVerification` and
    `ClosureReason`;
  - the reviews: `ReviewItem<Id>`, `ReviewKind` and `REVIEW_KIND_CODES: [&str; 10]`;
  - the outcome: `Counts`, `SyncPlan<Id>`, `AbortReason` and `SyncOutcome<Id>`;
  - `similarity(&str, &str) -> f32`, `SUBMISSION_MATCH_THRESHOLD: f32 = 0.45` and
    `REREGISTRATION_THRESHOLD: f32 = 0.6`.

- [ ] **Step 1: Register the module.** Replace the whole of `backend/crates/domain/src/register/mod.rs` with:

```rust
//! The school and municipality register (#3441, docs/school-register-design.md):
//! pure rules over names and register facts, and the sync planner. Fetching and storing
//! live in other crates.

pub mod brreg;
pub mod scope;
pub mod search;
pub mod slug;
pub mod source;
pub mod sync;
mod text;
```

Create `backend/crates/domain/src/register/sync/mod.rs`:

```rust
//! The register sync planner (#3441, docs/school-register-design.md §2.4, §4.5, §5.2-5.3).
//! Pure: it reads a snapshot of the register and the freshly fetched sources, and returns a
//! plan of what to create, rename, renumber, move and close, plus the review items. No SQL,
//! no HTTP, no mail: the persistence applier and `fau register sync` apply the plan.

mod similarity;
mod types;

pub use similarity::{similarity, REREGISTRATION_THRESHOLD, SUBMISSION_MATCH_THRESHOLD};
pub use types::*;
```

- [ ] **Step 2: Write the failing tests.** Create `backend/crates/domain/src/register/sync/types.rs`
  containing only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::register::source::NsrAddress;

    /// Pulls the quoted codes out of the `check (<column> in (...))` that follows `anchor`.
    fn codes_after(sql: &str, anchor: &str, column: &str) -> Vec<String> {
        let from = sql.find(anchor).expect("anchor is in 0004");
        let rest = &sql[from..];
        let open = rest
            .find(&format!("{column} in ("))
            .expect("check is in 0004");
        let list = &rest[open..];
        let list = &list[..list.find(')').expect("the list closes")];
        list.split('\'')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect()
    }

    const MIGRATION: &str = include_str!("../../../../../migrations/0004_school_register.sql");

    #[test]
    fn review_kind_codes_match_0004_exactly() {
        let codes = codes_after(MIGRATION, "create table register_review_items", "kind");
        assert_eq!(codes, REVIEW_KIND_CODES);
        use ReviewKind::*;
        let kinds = [
            ClosureWithFau,
            PossibleReregistration,
            PossibleSubmissionMatch,
            MunicipalitySplitOrMerge,
            MassChange,
            UnknownMunicipalityNumber,
            FauSeveralAtSchool,
            FauSeveralSchools,
            FauMatchConflict,
            SubmissionMatchesListedSchool,
        ];
        assert_eq!(kinds.map(ReviewKind::code), REVIEW_KIND_CODES);
    }

    #[test]
    fn closure_reasons_are_0004_codes() {
        let codes = codes_after(MIGRATION, "create table schools", "closure_reason");
        assert_eq!(
            codes,
            ["closed", "merged", "duplicate", "rejected", "out_of_scope"]
        );
        use ClosureReason::*;
        for reason in [Closed, Merged, Duplicate, OutOfScope] {
            assert!(codes.iter().any(|c| c == reason.code()), "{reason:?}");
        }
    }

    #[test]
    fn verification_ownership_and_source_codes_are_0004_codes() {
        let verification = codes_after(MIGRATION, "create table schools", "verification");
        for v in [CreateVerification::Listed, CreateVerification::Held] {
            assert!(verification.iter().any(|c| c == v.code()), "{v:?}");
        }
        let ownership = codes_after(MIGRATION, "create table schools", "ownership");
        assert_eq!(
            [Ownership::Public, Ownership::Private].map(Ownership::code),
            ownership.as_slice()
        );
        let source = codes_after(MIGRATION, "create table municipalities", "source");
        assert_eq!(
            [MunicipalitySource::Kartverket, MunicipalitySource::Manual]
                .map(MunicipalitySource::code),
            source.as_slice()
        );
    }

    fn hosle() -> NsrUnit {
        NsrUnit {
            orgnr: "974552124".into(),
            name: "Hosle skole".into(),
            municipality_number: "3201".into(),
            is_school: true,
            is_active: true,
            is_primary_school: true,
            is_private: false,
            category_ids: vec!["1".into(), "3".into(), "5".into(), "32".into()],
            nace: vec![(1, "85.201".into())],
            grade_from: Some(1),
            grade_to: Some(7),
            language: Some("nb".into()),
            website: Some("www.hosle.no".into()),
            visiting: NsrAddress {
                street: Some("Bispeveien 73".into()),
                postcode: Some("1362".into()),
                post_town: Some("HOSLE".into()),
            },
            postal: NsrAddress {
                street: Some("Postboks 700".into()),
                postcode: Some("1304".into()),
                post_town: Some("SANDVIKA".into()),
            },
            closure: None,
            changed_at: None,
        }
    }

    #[test]
    fn attributes_take_the_visiting_address() {
        assert_eq!(
            SchoolAttributes::from_unit(&hosle()),
            SchoolAttributes {
                ownership: Some(Ownership::Public),
                grade_from: Some(1),
                grade_to: Some(7),
                language: Some("nb".into()),
                website: Some("www.hosle.no".into()),
                street_address: Some("Bispeveien 73".into()),
                postcode: Some("1362".into()),
                post_town: Some("HOSLE".into()),
            }
        );
    }

    #[test]
    fn attributes_fall_back_to_the_postal_address_field_by_field() {
        let unit = NsrUnit {
            is_private: true,
            visiting: NsrAddress {
                street: None,
                postcode: Some("1362".into()),
                post_town: None,
            },
            ..hosle()
        };
        let a = SchoolAttributes::from_unit(&unit);
        assert_eq!(a.ownership, Some(Ownership::Private));
        assert_eq!(a.street_address.as_deref(), Some("Postboks 700"));
        assert_eq!(a.postcode.as_deref(), Some("1362"));
        assert_eq!(a.post_town.as_deref(), Some("SANDVIKA"));
    }
}
```

Create `backend/crates/domain/src/register/sync/similarity.rs` containing only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Expected values are PostgreSQL 17's `select similarity(a, b)` with pg_trgm, run on the
    /// `search_query` forms of the names, 24 September 2026.
    #[test]
    fn matches_pg_trgm() {
        let cases: [(&str, &str, f32); 8] = [
            ("Stange ungdomsskole", "Stange ungdomskole", 0.857_142_87),
            ("Stange ungdomsskole", "Stange ungdomsskule", 0.739_130_44),
            ("Stange skole", "Stange ungdomsskole", 0.523_809_55),
            ("Hosle skole", "Hosle skule", 0.571_428_6),
            ("Hosle skole", "Bekkestua skole", 0.285_714_3),
            ("Fornebu", "Fornebu skole", 0.571_428_6),
            ("Fornebu skule", "Fornebu skole", 0.647_058_84),
            (
                "Lerberg skole",
                "Lerberg skole og kompetansesenter",
                0.411_764_7,
            ),
        ];
        for (a, b, expected) in cases {
            let got = similarity(a, b);
            assert!(
                (got - expected).abs() < 1e-6,
                "{a} / {b}: {got} != {expected}"
            );
            assert_eq!(similarity(b, a), got, "symmetric: {a} / {b}");
        }
    }

    #[test]
    fn hand_traced_fractions() {
        // "hosle skole": hosle gives "  h", " ho", hos, osl, sle, "le "; skole gives "  s",
        // " sk", sko, kol, ole and "le " again: 11 trigrams. "hosle skule" also has 11, and
        // they share 8, so 8 / (11 + 11 - 8) = 8 / 14.
        assert_eq!(similarity("Hosle skole", "Hosle skule"), 8.0 / 14.0);
        // "oksenoya" has 9 trigrams, "oksenoya skole" 15, sharing all 9: exactly 9 / 15.
        assert_eq!(similarity("Oksenøya", "Oksenøya skole"), 9.0 / 15.0);
    }

    #[test]
    fn the_thresholds_are_inclusive() {
        // 9 / 15 rounds to the same f32 as the literal 0.6.
        assert!(similarity("Oksenøya", "Oksenøya skole") >= REREGISTRATION_THRESHOLD);
        assert!(similarity("Fornebu", "Fornebu skole") >= SUBMISSION_MATCH_THRESHOLD);
        assert!(similarity("Fornebu", "Fornebu skole") < REREGISTRATION_THRESHOLD);
        assert!(
            similarity("Lerberg skole", "Lerberg skole og kompetansesenter")
                < SUBMISSION_MATCH_THRESHOLD
        );
    }

    #[test]
    fn names_are_compared_in_their_query_form() {
        assert_eq!(similarity("Bærum", "barum"), 1.0);
        assert_eq!(
            similarity("STANGE  Ungdomsskole", "stange ungdomsskole"),
            1.0
        );
    }

    #[test]
    fn nothing_to_compare_is_zero() {
        assert_eq!(similarity("", ""), 0.0);
        assert_eq!(similarity("?!", "Школа"), 0.0);
        assert_eq!(similarity("abc", ""), 0.0);
        assert_eq!(similarity("a", "b"), 0.0);
    }
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cd /workspace/backend && cargo test -p fau-domain register::sync`
Expected: compile errors, since `REVIEW_KIND_CODES`, `ReviewKind`, `similarity` and the rest are
undefined.

- [ ] **Step 4: Implement.** Put this above the test module in `types.rs`:

```rust
//! The planner's vocabulary: the register snapshot it reads, the inputs it is given, and the
//! plan it returns (docs/school-register-design.md §4.1-4.3, §5.2-5.3).

use std::fmt::Debug;
use std::hash::Hash;

use jiff::civil::Date;
use jiff::Timestamp;

use crate::register::source::{CodeChange, MunicipalityRecord, NsrUnit, OfficialName};
use crate::time::Moment;

/// The caller's row id. The domain has no uuid dependency, so the planner is generic over it:
/// the persistence applier uses `Uuid`, the tests use `u32`.
pub trait RowId: Copy + Eq + Hash + Ord + Debug {}
impl<T: Copy + Eq + Hash + Ord + Debug> RowId for T {}

/// A row that exists, or one this plan creates. `New` is numbered from 0 in creation order,
/// with separate counters for municipalities and schools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Ref<Id> {
    Existing(Id),
    New(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MunicipalityStatus {
    Active,
    Dissolved,
}

/// `municipalities.source`: Svalbard is `Manual`, since neither source lists it (D2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MunicipalitySource {
    Kartverket,
    Manual,
}

impl MunicipalitySource {
    pub fn code(self) -> &'static str {
        match self {
            MunicipalitySource::Kartverket => "kartverket",
            MunicipalitySource::Manual => "manual",
        }
    }
}

/// One `municipalities` row with its current number and its names (§4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MunicipalitySnapshot<Id> {
    pub id: Id,
    /// The current number (`municipality_numbers` where `valid_until is null`).
    pub number: String,
    /// The Norwegian name, which the slug follows (D3).
    pub name: String,
    pub official_name: Option<String>,
    pub county_number: String,
    pub county_name: String,
    pub slug: String,
    pub status: MunicipalityStatus,
    pub source: MunicipalitySource,
    pub names: Vec<OfficialName>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Register,
    Submitted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verification {
    Listed,
    Pending,
    Verified,
    Rejected,
    Held,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchoolStatus {
    Active,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ownership {
    Public,
    Private,
}

impl Ownership {
    /// `schools.ownership`.
    pub fn code(self) -> &'static str {
        match self {
            Ownership::Public => "public",
            Ownership::Private => "private",
        }
    }
}

/// The NSR facts a school row carries, compared as a whole to decide `UpdateAttributes`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SchoolAttributes {
    pub ownership: Option<Ownership>,
    pub grade_from: Option<i16>,
    pub grade_to: Option<i16>,
    pub language: Option<String>,
    pub website: Option<String>,
    pub street_address: Option<String>,
    pub postcode: Option<String>,
    pub post_town: Option<String>,
}

impl SchoolAttributes {
    /// The visiting address, falling back to the postal address field by field where a
    /// visiting field is absent.
    pub fn from_unit(unit: &NsrUnit) -> Self {
        let pick = |visiting: &Option<String>, postal: &Option<String>| {
            visiting.clone().or_else(|| postal.clone())
        };
        SchoolAttributes {
            ownership: Some(if unit.is_private {
                Ownership::Private
            } else {
                Ownership::Public
            }),
            grade_from: unit.grade_from,
            grade_to: unit.grade_to,
            language: unit.language.clone(),
            website: unit.website.clone(),
            street_address: pick(&unit.visiting.street, &unit.postal.street),
            postcode: pick(&unit.visiting.postcode, &unit.postal.postcode),
            post_town: pick(&unit.visiting.post_town, &unit.postal.post_town),
        }
    }
}

/// One `schools` row, as far as the planner needs it (§4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchoolSnapshot<Id> {
    pub id: Id,
    pub municipality_id: Id,
    pub origin: Origin,
    pub orgnr: Option<String>,
    pub register_name: Option<String>,
    pub display_name: String,
    pub display_name_curated: bool,
    pub slug: Option<String>,
    pub verification: Verification,
    pub status: SchoolStatus,
    pub in_scope: bool,
    pub scope_override: Option<bool>,
    /// A pending or active tenant references the school (D8).
    pub has_live_fau: bool,
    pub attributes: SchoolAttributes,
}

/// The register as the planner sees it. Built by the caller in one read transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterSnapshot<Id> {
    pub municipalities: Vec<MunicipalitySnapshot<Id>>,
    pub schools: Vec<SchoolSnapshot<Id>>,
    /// `(orgnr, school)`: each school's former orgnrs.
    pub school_orgnr_history: Vec<(String, Id)>,
    /// `(municipality, slug, school)`.
    pub school_slug_history: Vec<(Id, String, Id)>,
    /// `(slug, municipality)`. Not read by the planner: a municipality slug starts with its
    /// unique current number, so it cannot collide. Carried for the applier and the tests.
    pub municipality_slug_history: Vec<(String, Id)>,
}

impl<Id> Default for RegisterSnapshot<Id> {
    fn default() -> Self {
        RegisterSnapshot {
            municipalities: Vec::new(),
            schools: Vec::new(),
            school_orgnr_history: Vec::new(),
            school_slug_history: Vec::new(),
            municipality_slug_history: Vec::new(),
        }
    }
}

/// `register_sync_runs.kind`, minus `dry_run`, which is the caller's business: a dry run
/// plans exactly like the run it previews.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKind {
    /// The first run on an empty register: unknown municipality numbers are created.
    Seed,
    Sync,
}

/// What the caller fetched for this run (§5.2 steps 1-3).
#[derive(Debug, Clone, Copy)]
pub struct SyncInputs<'a> {
    /// Kartverket's current municipalities.
    pub municipalities: &'a [MunicipalityRecord],
    /// SSB's changes since the last applied run.
    pub code_changes: &'a [CodeChange],
    /// The NSR detail of every active grunnskole, plus every unit whose orgnr the register
    /// already holds.
    pub units: &'a [NsrUnit],
    pub kind: RunKind,
    pub at: Moment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MunicipalityOp<Id> {
    Create {
        new: u32,
        number: String,
        name: String,
        official_name: Option<String>,
        county_number: String,
        county_name: String,
        slug: String,
        names: Vec<OfficialName>,
        source: MunicipalitySource,
    },
    /// A one-to-one SSB change (§2.4 case 1). The old number's validity ends on `valid_from`.
    Renumber {
        id: Id,
        from: String,
        to: String,
        valid_from: Date,
        old_slug: String,
        new_slug: String,
    },
    /// The Norwegian name changed. `old_slug == new_slug` when only the case changed: the
    /// applier then writes no history row.
    Rename {
        id: Id,
        name: String,
        old_slug: String,
        new_slug: String,
    },
    /// Everything but the Norwegian name and the slug (D3: an official-name change moves no
    /// URL).
    UpdateDetails {
        id: Id,
        official_name: Option<String>,
        county_number: String,
        county_name: String,
        names: Vec<OfficialName>,
    },
}

/// A slug moving into history (§6, "Immutability and history").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlugChange {
    pub old: String,
    pub new: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateVerification {
    Listed,
    /// Held back while a possible match is reviewed (§4.5). Never has a slug.
    Held,
}

impl CreateVerification {
    /// `schools.verification`.
    pub fn code(self) -> &'static str {
        match self {
            CreateVerification::Listed => "listed",
            CreateVerification::Held => "held",
        }
    }
}

/// The closure reasons the planner assigns. `rejected`, the fifth value 0004 allows, is set
/// by a reviewer, never by the sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosureReason {
    Closed,
    Merged,
    Duplicate,
    OutOfScope,
}

impl ClosureReason {
    /// `schools.closure_reason`.
    pub fn code(self) -> &'static str {
        match self {
            ClosureReason::Closed => "closed",
            ClosureReason::Merged => "merged",
            ClosureReason::Duplicate => "duplicate",
            ClosureReason::OutOfScope => "out_of_scope",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchoolOp<Id> {
    Create {
        new: u32,
        municipality: Ref<Id>,
        orgnr: String,
        register_name: String,
        display_name: String,
        slug: Option<String>,
        verification: CreateVerification,
        in_scope: bool,
        attributes: SchoolAttributes,
        source_changed_at: Option<Timestamp>,
    },
    /// NSR's name changed. `display_name` is `None` when the display name is curated (D5).
    Rename {
        id: Id,
        register_name: String,
        display_name: Option<String>,
        slug: Option<SlugChange>,
    },
    UpdateAttributes {
        id: Id,
        attributes: SchoolAttributes,
    },
    /// The school's NSR municipality now resolves to another municipality entity. The old
    /// slug goes to history under `from` (§4.2).
    Move {
        id: Id,
        from: Id,
        to: Ref<Id>,
        slug: Option<SlugChange>,
    },
    /// The applier moves the school's slug to history and clears it, so the slug can be
    /// re-issued (ADR-002 rule 3).
    Close {
        id: Id,
        reason: ClosureReason,
        closed_on: Date,
        successor: Option<Ref<Id>>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewKind {
    ClosureWithFau,
    PossibleReregistration,
    PossibleSubmissionMatch,
    MunicipalitySplitOrMerge,
    MassChange,
    UnknownMunicipalityNumber,
    FauSeveralAtSchool,
    FauSeveralSchools,
    FauMatchConflict,
    SubmissionMatchesListedSchool,
}

impl ReviewKind {
    /// `register_review_items.kind`.
    pub fn code(self) -> &'static str {
        match self {
            ReviewKind::ClosureWithFau => "closure_with_fau",
            ReviewKind::PossibleReregistration => "possible_reregistration",
            ReviewKind::PossibleSubmissionMatch => "possible_submission_match",
            ReviewKind::MunicipalitySplitOrMerge => "municipality_split_or_merge",
            ReviewKind::MassChange => "mass_change",
            ReviewKind::UnknownMunicipalityNumber => "unknown_municipality_number",
            ReviewKind::FauSeveralAtSchool => "fau_several_at_school",
            ReviewKind::FauSeveralSchools => "fau_several_schools",
            ReviewKind::FauMatchConflict => "fau_match_conflict",
            ReviewKind::SubmissionMatchesListedSchool => "submission_matches_listed_school",
        }
    }
}

/// Every code `register_review_items.kind` may hold, in 0004's order. A test pins it against
/// the migration.
pub const REVIEW_KIND_CODES: [&str; 10] = [
    "closure_with_fau",
    "possible_reregistration",
    "possible_submission_match",
    "municipality_split_or_merge",
    "mass_change",
    "unknown_municipality_number",
    "fau_several_at_school",
    "fau_several_schools",
    "fau_match_conflict",
    "submission_matches_listed_school",
];

/// One `register_review_items` row. `details` holds ids, codes, names and orgnrs only, never
/// an address (ruling, 24 September 2026).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewItem<Id> {
    pub kind: ReviewKind,
    pub school: Option<Ref<Id>>,
    pub other_school: Option<Ref<Id>>,
    pub municipality: Option<Ref<Id>>,
    pub details: Vec<(&'static str, String)>,
}

/// `register_sync_runs.counts`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub municipalities_created: usize,
    pub municipalities_updated: usize,
    pub renumbered: usize,
    pub renamed: usize,
    /// Listed creates. Held creates count as `schools_held` only.
    pub schools_created: usize,
    /// Register-name changes: the circuit breaker's renames.
    pub schools_renamed: usize,
    pub schools_updated: usize,
    pub schools_moved: usize,
    pub schools_closed: usize,
    pub schools_held: usize,
    pub reviews: usize,
    /// Units left alone because their municipality is blocked or unknown.
    pub skipped: usize,
}

/// Ordered so a sequential applier works: municipality ops, then school closes (which release
/// slugs), then creates, renames, attribute updates and moves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncPlan<Id> {
    pub municipality_ops: Vec<MunicipalityOp<Id>>,
    pub school_ops: Vec<SchoolOp<Id>>,
    pub reviews: Vec<ReviewItem<Id>>,
    pub counts: Counts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbortReason {
    /// Kartverket or NSR returned nothing: an outage, not a country without schools.
    EmptySource { source: &'static str },
    /// §5.3: more than 2% of active schools closing, or more than 5% renamed.
    MassChange {
        closes: usize,
        renames: usize,
        active: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome<Id> {
    NoChange,
    Apply(SyncPlan<Id>),
    Abort { reason: AbortReason, counts: Counts },
}
```

Put this above the test module in `similarity.rs`:

```rust
//! Trigram similarity, computed as PostgreSQL's pg_trgm computes `similarity()`, over the
//! §7 query form of each name. The planner runs in memory, so it cannot ask the database.

use std::collections::BTreeSet;

use crate::register::search::search_query;

/// §4.5: a new NSR unit this similar to a pending or verified submitted school in the same
/// municipality is held for review. "0.45 to start; tune it on real review outcomes".
pub const SUBMISSION_MATCH_THRESHOLD: f32 = 0.45;

/// §4.5 "the same or a very similar name": a new NSR unit this similar to a school closing in
/// the same run and municipality is its re-registration.
pub const REREGISTRATION_THRESHOLD: f32 = 0.6;

/// pg_trgm's `similarity(a, b)` over `search_query(a)` and `search_query(b)`: each word padded
/// as `"  " + word + " "`, the set of its 3-character windows, then |A ∩ B| / |A ∪ B|. Two
/// names with no trigrams at all are 0, as in pg_trgm.
pub fn similarity(a: &str, b: &str) -> f32 {
    let (ta, tb) = (trigrams(&search_query(a)), trigrams(&search_query(b)));
    let common = ta.intersection(&tb).count();
    let union = ta.len() + tb.len() - common;
    if union == 0 {
        return 0.0;
    }
    common as f32 / union as f32
}

/// `search_query` output is ASCII `[a-z0-9]` words joined by single spaces, so byte windows
/// are character windows.
fn trigrams(query: &str) -> BTreeSet<[u8; 3]> {
    let mut set = BTreeSet::new();
    for word in query.split(' ').filter(|w| !w.is_empty()) {
        let padded = format!("  {word} ");
        for w in padded.as_bytes().windows(3) {
            set.insert([w[0], w[1], w[2]]);
        }
    }
    set
}
```

- [ ] **Step 5: Run to verify pass**

Run: `cd /workspace/backend && cargo test -p fau-domain register::sync`
Expected: 10 tests pass (5 in `types`, 5 in `similarity`). Then run the full test command, `cargo fmt --check` and
`cargo clippy --workspace --all-targets -- -D warnings`, all clean.

- [ ] **Step 6: Commit**

```bash
cd /workspace/backend
git add crates/domain/src/register/mod.rs crates/domain/src/register/sync/
GIT_AUTHOR_NAME="Erik W. Bjønnes" GIT_AUTHOR_EMAIL=erik@ewb-solutions.as \
GIT_COMMITTER_NAME="Erik W. Bjønnes" GIT_COMMITTER_EMAIL=erik@ewb-solutions.as \
git commit -m "Add the sync planner's types and pg_trgm-compatible similarity (#3441)" \
  -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: The municipality planner

**Files:**
- Modify: `backend/crates/domain/src/register/sync/mod.rs`
- Create: `backend/crates/domain/src/register/sync/testkit.rs`
- Create: `backend/crates/domain/src/register/sync/municipalities.rs`

**Interfaces:**
- Consumes the Task 1 types, plus:
  - `register::slug::municipality_slug`;
  - `register::scope::{classify, ScopeDecision}`.
- Produces (crate-private, `pub(super)`):
  - `plan_municipalities<Id: RowId>(&RegisterSnapshot<Id>, &SyncInputs<'_>) -> MunicipalityPlan<Id>`,
    where `MunicipalityPlan<Id> { ops: Vec<MunicipalityOp<Id>>, reviews: Vec<ReviewItem<Id>>, state: MunicipalityState<Id> }`;
  - `MunicipalityState<Id>`, with three fields:
    - `by_number: BTreeMap<String, Ref<Id>>`: active municipalities by number, after renumbers and
      creates;
    - `blocked_numbers: BTreeSet<String>`;
    - `blocked_ids: BTreeSet<Id>`.

    Task 3 reads it;
  - `SVALBARD_NUMBER = "2100"`;
  - test builders in `testkit`: `ts`, `at` (Monday 28 September 2026, 04:30 Oslo), `record`,
    `municipality`, `change`, `unit` and `inputs`.

**The rules, in order** (§2.4, §5.2 step 1, and the brief's municipality rules):
1. **SSB changes.**
   - Drop rows whose codes and names are all the same.
   - Group the rest by old code, and drop every group whose old code Kartverket still lists: that
     is a boundary adjustment or a name change.
   - A gone code mapping to exactly one new code, which no other gone code maps to, is a
     `Renumber`, and the slug is re-minted from the current name.
   - Anything else raises `municipality_split_or_merge` and blocks every code in the group.
2. **Each Kartverket record** against the municipality holding its number, which is never a manual
   one:
   - a changed Norwegian name gives `Rename`;
   - a changed official name, county or name list gives `UpdateDetails`;
   - an unknown number gives `Create` in a seed, and a review item in a sync.
3. **An active Kartverket municipality absent from Kartverket** is reviewed, never dissolved.
4. **Svalbard** is created as a manual entry once an in-scope NSR unit is filed under 2100.

- [ ] **Step 1: Test scaffolding.** Replace `backend/crates/domain/src/register/sync/mod.rs` with:

```rust
//! The register sync planner (#3441, docs/school-register-design.md §2.4, §4.5, §5.2-5.3).
//! Pure: it reads a snapshot of the register and the freshly fetched sources, and returns a
//! plan of what to create, rename, renumber, move and close, plus the review items. No SQL,
//! no HTTP, no mail: the persistence applier and `fau register sync` apply the plan.

// Wired into `plan()` in Task 5; until then only its tests call it.
#[allow(dead_code)]
mod municipalities;
mod similarity;
#[cfg(test)]
mod testkit;
mod types;

pub use similarity::{similarity, REREGISTRATION_THRESHOLD, SUBMISSION_MATCH_THRESHOLD};
pub use types::*;
```

Create `backend/crates/domain/src/register/sync/testkit.rs`:

```rust
//! Builders for the planner's tests. The values mirror the recorded fixtures in
//! crates/register-sources/tests/fixtures/ (see its README); the domain cannot depend on
//! that crate, so they are written out by hand.

use jiff::civil::Date;
use jiff::Timestamp;

use crate::register::slug::municipality_slug;
use crate::register::source::{CodeChange, MunicipalityRecord, NsrAddress, NsrUnit, OfficialName};
use crate::time::Moment;

use super::types::{
    MunicipalitySnapshot, MunicipalitySource, MunicipalityStatus, RunKind, SyncInputs,
};

pub(super) fn ts(s: &str) -> Timestamp {
    s.parse().expect("a valid timestamp literal")
}

/// Monday 28 September 2026, 04:30 in Oslo: the weekly CronJob's slot (§5.1).
pub(super) fn at() -> Moment {
    Moment::at(ts("2026-09-28T02:30:00Z"))
}

/// A Kartverket record whose official name and only name are the Norwegian name.
pub(super) fn record(
    number: &str,
    name: &str,
    county_number: &str,
    county_name: &str,
) -> MunicipalityRecord {
    MunicipalityRecord {
        number: number.into(),
        norwegian_name: name.into(),
        official_name: name.into(),
        county_number: county_number.into(),
        county_name: county_name.into(),
        names: vec![OfficialName {
            name: name.into(),
            language: "no".into(),
            priority: 1,
        }],
    }
}

/// The active, Kartverket-sourced row a seed would have made from `r`.
pub(super) fn municipality(id: u32, r: &MunicipalityRecord) -> MunicipalitySnapshot<u32> {
    MunicipalitySnapshot {
        id,
        number: r.number.clone(),
        name: r.norwegian_name.clone(),
        official_name: Some(r.official_name.clone()),
        county_number: r.county_number.clone(),
        county_name: r.county_name.clone(),
        slug: municipality_slug(&r.number, &r.norwegian_name).expect("fixture names slug"),
        status: MunicipalityStatus::Active,
        source: MunicipalitySource::Kartverket,
        names: r.names.clone(),
    }
}

pub(super) fn change(
    old_code: &str,
    old_name: &str,
    new_code: &str,
    new_name: &str,
    occurred_on: Date,
) -> CodeChange {
    CodeChange {
        old_code: old_code.into(),
        old_name: old_name.into(),
        new_code: new_code.into(),
        new_name: new_name.into(),
        occurred_on,
    }
}

/// An active, in-scope public grunnskole (grades 1-7, Bokmål) with no address or website.
pub(super) fn unit(orgnr: &str, name: &str, municipality_number: &str) -> NsrUnit {
    NsrUnit {
        orgnr: orgnr.into(),
        name: name.into(),
        municipality_number: municipality_number.into(),
        is_school: true,
        is_active: true,
        is_primary_school: true,
        is_private: false,
        category_ids: vec!["1".into(), "3".into(), "5".into(), "32".into()],
        nace: vec![(1, "85.201".into())],
        grade_from: Some(1),
        grade_to: Some(7),
        language: Some("nb".into()),
        website: None,
        visiting: NsrAddress::default(),
        postal: NsrAddress::default(),
        closure: None,
        changed_at: None,
    }
}

pub(super) fn inputs<'a>(
    municipalities: &'a [MunicipalityRecord],
    code_changes: &'a [CodeChange],
    units: &'a [NsrUnit],
    kind: RunKind,
) -> SyncInputs<'a> {
    SyncInputs {
        municipalities,
        code_changes,
        units,
        kind,
        at: at(),
    }
}
```

- [ ] **Step 2: Write the failing tests.** Create `backend/crates/domain/src/register/sync/municipalities.rs`
  containing only this test module:

```rust
#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::super::testkit::{change, inputs, municipality, record, unit};
    use super::*;

    fn run(
        snapshot: &RegisterSnapshot<u32>,
        records: &[MunicipalityRecord],
        changes: &[CodeChange],
        units: &[crate::register::source::NsrUnit],
        kind: RunKind,
    ) -> MunicipalityPlan<u32> {
        plan_municipalities(snapshot, &inputs(records, changes, units, kind))
    }

    fn snapshot(municipalities: Vec<MunicipalitySnapshot<u32>>) -> RegisterSnapshot<u32> {
        RegisterSnapshot {
            municipalities,
            ..RegisterSnapshot::default()
        }
    }

    fn no_names(name: &str) -> Vec<OfficialName> {
        vec![OfficialName {
            name: name.into(),
            language: "no".into(),
            priority: 1,
        }]
    }

    #[test]
    fn a_seed_creates_every_kartverket_municipality_and_svalbard() {
        let records = [
            record("3201", "Bærum", "32", "Akershus"),
            record("3413", "Stange", "34", "Innlandet"),
        ];
        // Longyearbyen skole grunnskole, 974795655, is filed under 2100.
        let units = [unit("974795655", "Longyearbyen skole grunnskole", "2100")];
        let plan = run(
            &RegisterSnapshot::default(),
            &records,
            &[],
            &units,
            RunKind::Seed,
        );
        assert_eq!(
            plan.ops,
            [
                MunicipalityOp::Create {
                    new: 0,
                    number: "3201".into(),
                    name: "Bærum".into(),
                    official_name: Some("Bærum".into()),
                    county_number: "32".into(),
                    county_name: "Akershus".into(),
                    slug: "3201-baerum".into(),
                    names: no_names("Bærum"),
                    source: MunicipalitySource::Kartverket,
                },
                MunicipalityOp::Create {
                    new: 1,
                    number: "3413".into(),
                    name: "Stange".into(),
                    official_name: Some("Stange".into()),
                    county_number: "34".into(),
                    county_name: "Innlandet".into(),
                    slug: "3413-stange".into(),
                    names: no_names("Stange"),
                    source: MunicipalitySource::Kartverket,
                },
                MunicipalityOp::Create {
                    new: 2,
                    number: "2100".into(),
                    name: "Svalbard".into(),
                    official_name: None,
                    county_number: "21".into(),
                    county_name: "Svalbard".into(),
                    slug: "2100-svalbard".into(),
                    names: no_names("Svalbard"),
                    source: MunicipalitySource::Manual,
                },
            ]
        );
        assert!(plan.reviews.is_empty());
        assert_eq!(
            plan.state.by_number,
            BTreeMap::from([
                ("2100".to_owned(), Ref::New(2)),
                ("3201".to_owned(), Ref::New(0)),
                ("3413".to_owned(), Ref::New(1)),
            ])
        );
    }

    #[test]
    fn svalbard_needs_an_in_scope_unit_and_is_created_once() {
        let records = [record("3201", "Bærum", "32", "Akershus")];
        let inactive = crate::register::source::NsrUnit {
            is_active: false,
            ..unit("974795655", "Longyearbyen skole grunnskole", "2100")
        };
        let plan = run(
            &RegisterSnapshot::default(),
            &records,
            &[],
            &[inactive],
            RunKind::Seed,
        );
        assert_eq!(plan.ops.len(), 1, "Bærum only: {:?}", plan.ops);

        let svalbard = MunicipalitySnapshot {
            source: MunicipalitySource::Manual,
            official_name: None,
            ..municipality(2, &record("2100", "Svalbard", "21", "Svalbard"))
        };
        let plan = run(
            &snapshot(vec![municipality(1, &records[0]), svalbard]),
            &records,
            &[],
            &[unit("974795655", "Longyearbyen skole grunnskole", "2100")],
            RunKind::Sync,
        );
        assert!(plan.ops.is_empty(), "{:?}", plan.ops);
        assert!(
            plan.reviews.is_empty(),
            "a manual entry is never compared with Kartverket: {:?}",
            plan.reviews
        );
        assert_eq!(plan.state.by_number["2100"], Ref::Existing(2));
    }

    #[test]
    fn a_one_to_one_ssb_change_is_a_renumber() {
        // 1 January 2024: 3024 Bærum in Viken became 3201 Bærum in Akershus.
        let old = municipality(1, &record("3024", "Bærum", "30", "Viken"));
        let records = [record("3201", "Bærum", "32", "Akershus")];
        let changes = [change("3024", "Bærum", "3201", "Bærum", date(2024, 1, 1))];
        let plan = run(&snapshot(vec![old]), &records, &changes, &[], RunKind::Sync);
        assert_eq!(
            plan.ops,
            [
                MunicipalityOp::Renumber {
                    id: 1,
                    from: "3024".into(),
                    to: "3201".into(),
                    valid_from: date(2024, 1, 1),
                    old_slug: "3024-baerum".into(),
                    new_slug: "3201-baerum".into(),
                },
                MunicipalityOp::UpdateDetails {
                    id: 1,
                    official_name: Some("Bærum".into()),
                    county_number: "32".into(),
                    county_name: "Akershus".into(),
                    names: no_names("Bærum"),
                },
            ]
        );
        assert!(plan.reviews.is_empty(), "{:?}", plan.reviews);
        assert_eq!(
            plan.state.by_number,
            BTreeMap::from([("3201".to_owned(), Ref::Existing(1))])
        );

        // The same change again, once applied, does nothing.
        let applied = municipality(1, &records[0]);
        let again = run(
            &snapshot(vec![applied]),
            &records,
            &changes,
            &[],
            RunKind::Sync,
        );
        assert!(again.ops.is_empty() && again.reviews.is_empty());
    }

    #[test]
    fn a_split_is_reviewed_and_blocks_every_code_in_it() {
        // 1 January 2024: 1507 Ålesund became 1508 Ålesund and 1580 Haram.
        let aalesund = municipality(1, &record("1507", "Ålesund", "15", "Møre og Romsdal"));
        let records = [
            record("1508", "Ålesund", "15", "Møre og Romsdal"),
            record("1580", "Haram", "15", "Møre og Romsdal"),
        ];
        let changes = [
            change("1507", "Ålesund", "1508", "Ålesund", date(2024, 1, 1)),
            change("1507", "Ålesund", "1580", "Haram", date(2024, 1, 1)),
        ];
        let plan = run(
            &snapshot(vec![aalesund]),
            &records,
            &changes,
            &[],
            RunKind::Sync,
        );
        assert!(plan.ops.is_empty(), "{:?}", plan.ops);
        assert_eq!(
            plan.reviews,
            [ReviewItem {
                kind: ReviewKind::MunicipalitySplitOrMerge,
                school: None,
                other_school: None,
                municipality: Some(Ref::Existing(1)),
                details: vec![
                    ("reason", "split".into()),
                    ("old_code", "1507".into()),
                    ("old_name", "Ålesund".into()),
                    ("new_codes", "1508,1580".into()),
                    ("new_names", "Ålesund,Haram".into()),
                ],
            }],
            "one item for the group; no unknown-number or absent item for its codes"
        );
        assert_eq!(
            plan.state.blocked_numbers,
            BTreeSet::from(["1507".into(), "1508".into(), "1580".into()])
        );
        assert_eq!(plan.state.blocked_ids, BTreeSet::from([1]));
    }

    #[test]
    fn a_merger_is_reviewed_once_per_old_code() {
        // 1 January 2020: 1571 Halsa and 5011 Hemne became part of 5055 Heim.
        let halsa = municipality(1, &record("1571", "Halsa", "15", "Møre og Romsdal"));
        let hemne = municipality(2, &record("5011", "Hemne", "50", "Trøndelag"));
        let records = [record("5055", "Heim", "50", "Trøndelag")];
        let changes = [
            change("1571", "Halsa", "5055", "Heim", date(2020, 1, 1)),
            change("5011", "Hemne", "5055", "Heim", date(2020, 1, 1)),
        ];
        let plan = run(
            &snapshot(vec![halsa, hemne]),
            &records,
            &changes,
            &[],
            RunKind::Sync,
        );
        assert!(plan.ops.is_empty(), "{:?}", plan.ops);
        let reasons: Vec<_> = plan
            .reviews
            .iter()
            .map(|r| {
                (
                    r.municipality,
                    r.details[0].1.as_str(),
                    r.details[1].1.as_str(),
                )
            })
            .collect();
        assert_eq!(
            reasons,
            [
                (Some(Ref::Existing(1)), "merge", "1571"),
                (Some(Ref::Existing(2)), "merge", "5011"),
            ]
        );
        assert_eq!(
            plan.state.blocked_numbers,
            BTreeSet::from(["1571".into(), "5011".into(), "5055".into()])
        );
    }

    #[test]
    fn the_2026_changes_are_a_boundary_adjustment_and_name_changes() {
        // 3118 Indre Østfold "changed" to 3207 and 3216 while continuing, and 0301, 5006
        // and 5536 changed their official names only.
        let records = [
            record("0301", "Oslo", "03", "Oslo"),
            record("3118", "Indre Østfold", "31", "Østfold"),
            record("3207", "Nordre Follo", "32", "Akershus"),
            record("3216", "Vestby", "32", "Akershus"),
        ];
        let rows = records
            .iter()
            .zip(1..)
            .map(|(r, id)| municipality(id, r))
            .collect();
        let d = date(2026, 1, 1);
        let changes = [
            change("0301", "Oslo", "0301", "Oslo - Oslove", d),
            change("3118", "Indre Østfold", "3118", "Indre Østfold", d),
            change("3118", "Indre Østfold", "3207", "Nordre Follo", d),
            change("3118", "Indre Østfold", "3216", "Vestby", d),
        ];
        let plan = run(&snapshot(rows), &records, &changes, &[], RunKind::Sync);
        assert!(plan.ops.is_empty(), "{:?}", plan.ops);
        assert!(plan.reviews.is_empty(), "{:?}", plan.reviews);
        assert!(plan.state.blocked_numbers.is_empty());
    }

    #[test]
    fn an_official_name_change_moves_no_slug() {
        // D3: "Oslo" to "Oslo - Oslove" is a detail, not a rename.
        let oslo = municipality(1, &record("0301", "Oslo", "03", "Oslo"));
        let names = vec![
            OfficialName {
                name: "Oslo".into(),
                language: "no".into(),
                priority: 1,
            },
            OfficialName {
                name: "Oslove".into(),
                language: "sma".into(),
                priority: 2,
            },
        ];
        let records = [MunicipalityRecord {
            official_name: "Oslo - Oslove".into(),
            names: names.clone(),
            ..record("0301", "Oslo", "03", "Oslo")
        }];
        let plan = run(&snapshot(vec![oslo]), &records, &[], &[], RunKind::Sync);
        assert_eq!(
            plan.ops,
            [MunicipalityOp::UpdateDetails {
                id: 1,
                official_name: Some("Oslo - Oslove".into()),
                county_number: "03".into(),
                county_name: "Oslo".into(),
                names,
            }]
        );
    }

    #[test]
    fn names_in_another_order_are_no_change() {
        let names = vec![
            OfficialName {
                name: "Kárášjohka".into(),
                language: "se".into(),
                priority: 1,
            },
            OfficialName {
                name: "Karasjok".into(),
                language: "no".into(),
                priority: 2,
            },
        ];
        let r = MunicipalityRecord {
            official_name: "Kárášjohka".into(),
            names: names.clone(),
            ..record("5610", "Karasjok", "56", "Finnmark")
        };
        let mut row = municipality(1, &r);
        row.names.reverse();
        let plan = run(&snapshot(vec![row]), &[r], &[], &[], RunKind::Sync);
        assert!(plan.ops.is_empty(), "{:?}", plan.ops);
    }

    #[test]
    fn a_norwegian_name_change_is_a_rename() {
        let baerum = municipality(1, &record("3201", "Bærum", "32", "Akershus"));
        let renamed = [MunicipalityRecord {
            norwegian_name: "Store Bærum".into(),
            ..record("3201", "Bærum", "32", "Akershus")
        }];
        let plan = run(
            &snapshot(vec![baerum.clone()]),
            &renamed,
            &[],
            &[],
            RunKind::Sync,
        );
        assert_eq!(
            plan.ops,
            [MunicipalityOp::Rename {
                id: 1,
                name: "Store Bærum".into(),
                old_slug: "3201-baerum".into(),
                new_slug: "3201-store-baerum".into(),
            }]
        );

        // A case-only change updates the name and keeps the slug.
        let shouted = [MunicipalityRecord {
            norwegian_name: "BÆRUM".into(),
            ..record("3201", "Bærum", "32", "Akershus")
        }];
        let plan = run(&snapshot(vec![baerum]), &shouted, &[], &[], RunKind::Sync);
        assert_eq!(
            plan.ops,
            [MunicipalityOp::Rename {
                id: 1,
                name: "BÆRUM".into(),
                old_slug: "3201-baerum".into(),
                new_slug: "3201-baerum".into(),
            }]
        );
    }

    #[test]
    fn a_sync_never_creates_an_unexplained_number() {
        let records = [record("3201", "Bærum", "32", "Akershus")];
        let plan = run(
            &RegisterSnapshot::default(),
            &records,
            &[],
            &[],
            RunKind::Sync,
        );
        assert!(plan.ops.is_empty());
        assert_eq!(
            plan.reviews,
            [ReviewItem {
                kind: ReviewKind::MunicipalitySplitOrMerge,
                school: None,
                other_school: None,
                municipality: None,
                details: vec![
                    ("reason", "unknown_number".into()),
                    ("number", "3201".into()),
                    ("name", "Bærum".into()),
                ],
            }]
        );
        assert!(plan.state.by_number.is_empty());
    }

    #[test]
    fn a_municipality_absent_from_kartverket_is_reviewed_not_dissolved() {
        let records = [record("3201", "Bærum", "32", "Akershus")];
        let svalbard = MunicipalitySnapshot {
            source: MunicipalitySource::Manual,
            official_name: None,
            ..municipality(3, &record("2100", "Svalbard", "21", "Svalbard"))
        };
        let dissolved = MunicipalitySnapshot {
            status: MunicipalityStatus::Dissolved,
            ..municipality(4, &record("1719", "Levanger", "17", "Nord-Trøndelag"))
        };
        let rows = vec![
            municipality(1, &records[0]),
            municipality(2, &record("1507", "Ålesund", "15", "Møre og Romsdal")),
            svalbard,
            dissolved,
        ];
        let plan = run(&snapshot(rows), &records, &[], &[], RunKind::Sync);
        assert!(plan.ops.is_empty(), "{:?}", plan.ops);
        assert_eq!(
            plan.reviews,
            [ReviewItem {
                kind: ReviewKind::MunicipalitySplitOrMerge,
                school: None,
                other_school: None,
                municipality: Some(Ref::Existing(2)),
                details: vec![
                    ("reason", "absent_from_kartverket".into()),
                    ("number", "1507".into()),
                    ("name", "Ålesund".into()),
                ],
            }],
            "neither the manual Svalbard entry nor a dissolved row is reviewed"
        );
        assert_eq!(
            plan.state.by_number.get("1507"),
            Some(&Ref::Existing(2)),
            "its schools still resolve"
        );
        assert_eq!(plan.state.by_number.get("1719"), None);
    }
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cd /workspace/backend && cargo test -p fau-domain register::sync::municipalities`
Expected: compile errors, since `plan_municipalities` and `MunicipalityPlan` are undefined.

- [ ] **Step 4: Implement.** Put this above the test module in `municipalities.rs`:

```rust
//! Step 1 of a run: municipalities (docs/school-register-design.md §2.4, §5.2 step 1).

use std::collections::{BTreeMap, BTreeSet};

use crate::register::scope::{classify, ScopeDecision};
use crate::register::slug::municipality_slug;
use crate::register::source::{CodeChange, MunicipalityRecord, OfficialName};

use super::types::{
    MunicipalityOp, MunicipalitySnapshot, MunicipalitySource, MunicipalityStatus, Ref,
    RegisterSnapshot, ReviewItem, ReviewKind, RowId, RunKind, SyncInputs,
};

/// Svalbard is not a municipality in Kartverket or SSB, but NSR files Longyearbyen skole under
/// 2100 (§2.2, D2).
pub(super) const SVALBARD_NUMBER: &str = "2100";

/// What the school planner needs to know about municipalities after step 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MunicipalityState<Id> {
    /// Active municipalities by current number, after this plan's renumbers and creates.
    pub by_number: BTreeMap<String, Ref<Id>>,
    /// Every old and new code of an unexplained SSB group: nothing touches them this run.
    pub blocked_numbers: BTreeSet<String>,
    /// Existing municipalities that hold a blocked number.
    pub blocked_ids: BTreeSet<Id>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MunicipalityPlan<Id> {
    pub ops: Vec<MunicipalityOp<Id>>,
    pub reviews: Vec<ReviewItem<Id>>,
    pub state: MunicipalityState<Id>,
}

/// An active municipality as this plan leaves it: a renumber changes its number and slug.
struct Working<'a, Id> {
    row: &'a MunicipalitySnapshot<Id>,
    number: String,
    slug: String,
}

pub(super) fn plan_municipalities<Id: RowId>(
    snapshot: &RegisterSnapshot<Id>,
    inputs: &SyncInputs<'_>,
) -> MunicipalityPlan<Id> {
    let mut ops = Vec::new();
    let mut reviews = Vec::new();
    let kartverket: BTreeSet<&str> = inputs
        .municipalities
        .iter()
        .map(|r| r.number.as_str())
        .collect();

    let mut working: BTreeMap<Id, Working<'_, Id>> = BTreeMap::new();
    let mut by_number: BTreeMap<String, Ref<Id>> = BTreeMap::new();
    for row in &snapshot.municipalities {
        if row.status == MunicipalityStatus::Active {
            by_number.insert(row.number.clone(), Ref::Existing(row.id));
            working.insert(
                row.id,
                Working {
                    row,
                    number: row.number.clone(),
                    slug: row.slug.clone(),
                },
            );
        }
    }

    // SSB: group by old code, skipping rows that change nothing. An old code Kartverket still
    // lists is a boundary adjustment or a name change, never a renumber (§2.4 case 3).
    let mut groups: BTreeMap<&str, Vec<&CodeChange>> = BTreeMap::new();
    for c in inputs.code_changes {
        if c.old_code == c.new_code && c.old_name == c.new_name {
            continue;
        }
        if kartverket.contains(c.old_code.as_str()) {
            continue;
        }
        groups.entry(c.old_code.as_str()).or_default().push(c);
    }
    let mut gone_groups_per_target: BTreeMap<&str, usize> = BTreeMap::new();
    for changes in groups.values() {
        let targets: BTreeSet<&str> = changes.iter().map(|c| c.new_code.as_str()).collect();
        for t in targets {
            *gone_groups_per_target.entry(t).or_default() += 1;
        }
    }

    let mut blocked_numbers: BTreeSet<String> = BTreeSet::new();
    for (old, changes) in &groups {
        let targets: BTreeMap<&str, &str> = changes
            .iter()
            .map(|c| (c.new_code.as_str(), c.new_name.as_str()))
            .collect();
        let holder = match by_number.get(*old) {
            Some(Ref::Existing(id)) => Some(*id),
            _ => None,
        };
        let one_to_one = match targets.keys().next() {
            Some(to) if targets.len() == 1 && gone_groups_per_target[to] == 1 => Some(*to),
            _ => None,
        };
        let reason = match (one_to_one, holder) {
            // Already applied, or never held: there is nothing to renumber.
            (Some(_), None) => continue,
            (Some(to), Some(id)) => {
                let w = &working[&id];
                if w.row.source == MunicipalitySource::Manual {
                    continue;
                }
                if by_number.contains_key(to) {
                    "target_in_use"
                } else if let Ok(new_slug) = municipality_slug(to, &w.row.name) {
                    ops.push(MunicipalityOp::Renumber {
                        id,
                        from: (*old).to_owned(),
                        to: to.to_owned(),
                        valid_from: changes[0].occurred_on,
                        old_slug: w.slug.clone(),
                        new_slug: new_slug.clone(),
                    });
                    by_number.remove(*old);
                    by_number.insert(to.to_owned(), Ref::Existing(id));
                    let w = working.get_mut(&id).expect("the holder is a working row");
                    w.number = to.to_owned();
                    w.slug = new_slug;
                    continue;
                } else {
                    "unsluggable"
                }
            }
            (None, _) if targets.len() > 1 => "split",
            (None, _) => "merge",
        };
        blocked_numbers.insert((*old).to_owned());
        blocked_numbers.extend(targets.keys().map(|t| (*t).to_owned()));
        reviews.push(ReviewItem {
            kind: ReviewKind::MunicipalitySplitOrMerge,
            school: None,
            other_school: None,
            municipality: holder.map(Ref::Existing),
            details: vec![
                ("reason", reason.to_owned()),
                ("old_code", (*old).to_owned()),
                ("old_name", changes[0].old_name.clone()),
                ("new_codes", join(targets.keys().copied())),
                ("new_names", join(targets.values().copied())),
            ],
        });
    }

    // Kartverket against the municipality holding each number.
    let mut next_new = 0u32;
    for r in inputs.municipalities {
        if blocked_numbers.contains(&r.number) {
            continue;
        }
        match by_number.get(&r.number) {
            Some(Ref::Existing(id)) => {
                let w = &working[id];
                if w.row.source == MunicipalitySource::Manual {
                    continue;
                }
                if r.norwegian_name != w.row.name {
                    match municipality_slug(&r.number, &r.norwegian_name) {
                        Ok(new_slug) => ops.push(MunicipalityOp::Rename {
                            id: *id,
                            name: r.norwegian_name.clone(),
                            old_slug: w.slug.clone(),
                            new_slug,
                        }),
                        Err(_) => reviews.push(record_review(r, "unsluggable", Some(*id))),
                    }
                }
                if details_differ(w.row, r) {
                    ops.push(MunicipalityOp::UpdateDetails {
                        id: *id,
                        official_name: Some(r.official_name.clone()),
                        county_number: r.county_number.clone(),
                        county_name: r.county_name.clone(),
                        names: r.names.clone(),
                    });
                }
            }
            // Kartverket listed the number twice; the first record created it.
            Some(Ref::New(_)) => {}
            None => match (inputs.kind, municipality_slug(&r.number, &r.norwegian_name)) {
                (RunKind::Seed, Ok(slug)) => {
                    ops.push(MunicipalityOp::Create {
                        new: next_new,
                        number: r.number.clone(),
                        name: r.norwegian_name.clone(),
                        official_name: Some(r.official_name.clone()),
                        county_number: r.county_number.clone(),
                        county_name: r.county_name.clone(),
                        slug,
                        names: r.names.clone(),
                        source: MunicipalitySource::Kartverket,
                    });
                    by_number.insert(r.number.clone(), Ref::New(next_new));
                    next_new += 1;
                }
                (RunKind::Seed, Err(_)) => reviews.push(record_review(r, "unsluggable", None)),
                // SSB has not explained a new number: an operator decides (§2.4 case 3).
                (RunKind::Sync, _) => reviews.push(record_review(r, "unknown_number", None)),
            },
        }
    }

    // Never dissolved automatically: an operator decides.
    for w in working.values() {
        if w.row.source == MunicipalitySource::Kartverket
            && !kartverket.contains(w.number.as_str())
            && !blocked_numbers.contains(&w.number)
        {
            reviews.push(ReviewItem {
                kind: ReviewKind::MunicipalitySplitOrMerge,
                school: None,
                other_school: None,
                municipality: Some(Ref::Existing(w.row.id)),
                details: vec![
                    ("reason", "absent_from_kartverket".to_owned()),
                    ("number", w.number.clone()),
                    ("name", w.row.name.clone()),
                ],
            });
        }
    }

    // The manual Svalbard entry, once NSR has an in-scope school there.
    let svalbard_needed = inputs.units.iter().any(|u| {
        u.municipality_number == SVALBARD_NUMBER
            && classify(&u.scope_facts()) == ScopeDecision::InScope
    });
    if svalbard_needed
        && !by_number.contains_key(SVALBARD_NUMBER)
        && !blocked_numbers.contains(SVALBARD_NUMBER)
    {
        ops.push(MunicipalityOp::Create {
            new: next_new,
            number: SVALBARD_NUMBER.to_owned(),
            name: "Svalbard".to_owned(),
            official_name: None,
            county_number: "21".to_owned(),
            county_name: "Svalbard".to_owned(),
            slug: "2100-svalbard".to_owned(),
            names: vec![OfficialName {
                name: "Svalbard".to_owned(),
                language: "no".to_owned(),
                priority: 1,
            }],
            source: MunicipalitySource::Manual,
        });
        by_number.insert(SVALBARD_NUMBER.to_owned(), Ref::New(next_new));
    }

    let blocked_ids = working
        .values()
        .filter(|w| blocked_numbers.contains(&w.number))
        .map(|w| w.row.id)
        .collect();
    MunicipalityPlan {
        ops,
        reviews,
        state: MunicipalityState {
            by_number,
            blocked_numbers,
            blocked_ids,
        },
    }
}

fn record_review<Id>(r: &MunicipalityRecord, reason: &str, id: Option<Id>) -> ReviewItem<Id> {
    ReviewItem {
        kind: ReviewKind::MunicipalitySplitOrMerge,
        school: None,
        other_school: None,
        municipality: id.map(Ref::Existing),
        details: vec![
            ("reason", reason.to_owned()),
            ("number", r.number.clone()),
            ("name", r.norwegian_name.clone()),
        ],
    }
}

fn details_differ<Id>(row: &MunicipalitySnapshot<Id>, r: &MunicipalityRecord) -> bool {
    row.official_name.as_deref() != Some(r.official_name.as_str())
        || row.county_number != r.county_number
        || row.county_name != r.county_name
        || sorted(&row.names) != sorted(&r.names)
}

/// Names compare as a set: the database returns them in no particular order.
fn sorted(names: &[OfficialName]) -> Vec<&OfficialName> {
    let mut v: Vec<&OfficialName> = names.iter().collect();
    v.sort_by(|a, b| (a.priority, &a.language, &a.name).cmp(&(b.priority, &b.language, &b.name)));
    v
}

fn join<'a>(items: impl Iterator<Item = &'a str>) -> String {
    items.collect::<Vec<_>>().join(",")
}
```

- [ ] **Step 5: Run to verify pass**

Run: `cd /workspace/backend && cargo test -p fau-domain register::sync`
Expected: 21 tests pass (11 new). Then run the full test command, fmt and clippy, all clean.

- [ ] **Step 6: Commit**

```bash
cd /workspace/backend
git add crates/domain/src/register/sync/
GIT_AUTHOR_NAME="Erik W. Bjønnes" GIT_AUTHOR_EMAIL=erik@ewb-solutions.as \
GIT_COMMITTER_NAME="Erik W. Bjønnes" GIT_COMMITTER_EMAIL=erik@ewb-solutions.as \
git commit -m "Plan municipality renumbers, renames and splits (#3441)" \
  -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: The school planner core

**Files:**
- Modify: `backend/crates/domain/src/register/sync/mod.rs`
- Modify: `backend/crates/domain/src/register/sync/testkit.rs`
- Create: `backend/crates/domain/src/register/sync/schools.rs`

**Interfaces:**
- Consumes:
  - `MunicipalityState<Id>` (Task 2);
  - `register::slug::{first_free_slug, slugify}`;
  - `register::scope::{classify, effective_in_scope, ScopeDecision}`;
  - `time::{oslo_today, Moment}`.
- Produces (crate-private):
  - `plan_schools<Id: RowId>(&RegisterSnapshot<Id>, &SyncInputs<'_>, &MunicipalityState<Id>) -> SchoolPlan<Id>`,
    where `SchoolPlan<Id> { ops: Vec<SchoolOp<Id>>, reviews: Vec<ReviewItem<Id>>, skipped: usize }`;
  - the internal `SlugBook`, `Closing`, `FauClosing`, `Candidate`, `closure_facts`, `new_number`
    and `plan_update`, which Task 4 keeps;
  - test builders in `testkit`:
    - `address` and `fixture_records`;
    - the thirteen fixture units, from `hosle()` to `stange_new()`, and `fixture_units()`;
    - `school(id, municipality_id, &unit, slug)`, which builds the listed row a seed would have
      made.

**The rules** (§5.2 step 5, §6, and the brief's school rules), per unit:
- **A unit maps to a school by current orgnr.** A unit found only in `school_orgnr_history` drives
  nothing.
- **An existing school:**
  - Closed, Held, Pending and Rejected schools are left alone;
  - a school in a blocked municipality is skipped;
  - an inactive or not-effectively-in-scope unit gives `Close`, or `closure_with_fau` when an FAU
    is attached. The reason is D Duplicate, F Merged, otherwise Closed, and OutOfScope for an
    active unit. The date is the Oslo date of `closure.at`, else today;
  - otherwise the planner emits `Rename`, `UpdateAttributes` and `Move` as needed.
- **A new active, in-scope unit** in a resolved municipality is created Listed. Its slug is
  `first_free_slug(slugify(name), post_town, taken)`, or `None` when nothing is sluggable.
- **Unknown numbers** give one `unknown_municipality_number` item per number.

- [ ] **Step 1: Test scaffolding.** In `backend/crates/domain/src/register/sync/mod.rs`, replace the
  whole file with:

```rust
//! The register sync planner (#3441, docs/school-register-design.md §2.4, §4.5, §5.2-5.3).
//! Pure: it reads a snapshot of the register and the freshly fetched sources, and returns a
//! plan of what to create, rename, renumber, move and close, plus the review items. No SQL,
//! no HTTP, no mail: the persistence applier and `fau register sync` apply the plan.

// Wired into `plan()` in Task 5; until then only its tests call it.
#[allow(dead_code)]
mod municipalities;
// Wired into `plan()` in Task 5; until then only its tests call it.
#[allow(dead_code)]
mod schools;
mod similarity;
#[cfg(test)]
mod testkit;
mod types;

pub use similarity::{similarity, REREGISTRATION_THRESHOLD, SUBMISSION_MATCH_THRESHOLD};
pub use types::*;
```

In `backend/crates/domain/src/register/sync/testkit.rs`, replace

```rust
use crate::register::source::{CodeChange, MunicipalityRecord, NsrAddress, NsrUnit, OfficialName};
```

with

```rust
use crate::register::source::{
    CodeChange, MunicipalityRecord, NsrAddress, NsrClosure, NsrUnit, OfficialName,
};
```

then replace

```rust
use super::types::{
    MunicipalitySnapshot, MunicipalitySource, MunicipalityStatus, RunKind, SyncInputs,
};
```

with

```rust
use super::types::{
    MunicipalitySnapshot, MunicipalitySource, MunicipalityStatus, Origin, RunKind,
    SchoolAttributes, SchoolSnapshot, SchoolStatus, SyncInputs, Verification,
};
```

and append to the end of the file:

```rust
pub(super) fn address(street: &str, postcode: &str, post_town: &str) -> NsrAddress {
    NsrAddress {
        street: Some(street.into()),
        postcode: Some(postcode.into()),
        post_town: Some(post_town.into()),
    }
}

/// The Kartverket records of the fixture units' municipalities, in Kartverket's shape.
pub(super) fn fixture_records() -> Vec<MunicipalityRecord> {
    vec![
        record("3201", "Bærum", "32", "Akershus"),
        record("3314", "Øvre Eiker", "33", "Buskerud"),
        record("3907", "Sandefjord", "39", "Vestfold"),
        record("3222", "Lørenskog", "32", "Akershus"),
        record("3107", "Fredrikstad", "31", "Østfold"),
        record("4649", "Stad", "46", "Vestland"),
        record("5055", "Heim", "50", "Trøndelag"),
        record("5026", "Holtålen", "50", "Trøndelag"),
        record("3413", "Stange", "34", "Innlandet"),
    ]
}

/// Hosle skole, 974552124: an ordinary public school in 3201 Bærum.
pub(super) fn hosle() -> NsrUnit {
    NsrUnit {
        website: Some("www.hosle.no".into()),
        visiting: address("Bispeveien 73", "1362", "HOSLE"),
        postal: address("Postboks 700", "1304", "SANDVIKA"),
        changed_at: Some(ts("2026-09-13T01:05:43.46Z")),
        ..unit("974552124", "Hosle skole", "3201")
    }
}

/// Norges Toppidrettsgymnas ungdomsskole Bærum AS, 990672938: private.
pub(super) fn ntg() -> NsrUnit {
    NsrUnit {
        is_private: true,
        category_ids: vec!["1".into(), "4".into(), "16".into(), "32".into()],
        grade_from: Some(8),
        grade_to: Some(10),
        website: Some("www.ntg.no".into()),
        visiting: address("Hans Burums vei 30", "1357", "BEKKESTUA"),
        postal: address("Postboks 134", "1319", "BEKKESTUA"),
        changed_at: Some(ts("2026-09-13T01:38:47.433Z")),
        ..unit(
            "990672938",
            "Norges Toppidrettsgymnas ungdomsskole Bærum AS",
            "3201",
        )
    }
}

/// Lerberg skole og kompetansesenter, 998516897: combined, 85.201 then 85.310.
pub(super) fn lerberg() -> NsrUnit {
    NsrUnit {
        category_ids: vec!["1".into(), "2".into(), "3".into(), "6".into(), "32".into()],
        nace: vec![(1, "85.201".into()), (2, "85.310".into())],
        grade_from: Some(8),
        grade_to: Some(10),
        visiting: address("Ringeriksveien 2", "3303", "HOKKSUND"),
        postal: address("Postboks 117", "3301", "HOKKSUND"),
        changed_at: Some(ts("2026-09-13T01:39:57.313Z")),
        ..unit("998516897", "Lerberg skole og kompetansesenter", "3314")
    }
}

/// Signo Grunn- og videregående skole AS, 998666783: a special school (85.202).
pub(super) fn signo() -> NsrUnit {
    NsrUnit {
        is_private: true,
        category_ids: vec![
            "1".into(),
            "2".into(),
            "4".into(),
            "16".into(),
            "32".into(),
            "12".into(),
        ],
        nace: vec![
            (1, "85.202".into()),
            (2, "85.310".into()),
            (3, "85.201".into()),
        ],
        grade_to: Some(10),
        website: Some("www.signo.no".into()),
        visiting: address("Molandveien 29", "3158", "ANDEBU"),
        changed_at: Some(ts("2026-09-13T01:39:59.673Z")),
        ..unit("998666783", "Signo Grunn- og videregående skole AS", "3907")
    }
}

/// Lørenskog voksenopplæring, 999038182: adult education. Out of scope, so only the facts
/// the filter reads are copied.
pub(super) fn lorenskog_adult() -> NsrUnit {
    NsrUnit {
        category_ids: vec![
            "1".into(),
            "3".into(),
            "5".into(),
            "32".into(),
            "10".into(),
            "25".into(),
        ],
        nace: vec![(1, "85.593".into()), (2, "85.201".into())],
        ..unit("999038182", "Lørenskog voksenopplæring", "3222")
    }
}

/// Wang Fredrikstad AS, 986779795: upper secondary as its primary NACE code. Out of scope.
pub(super) fn wang() -> NsrUnit {
    NsrUnit {
        is_private: true,
        nace: vec![(1, "85.310".into()), (2, "85.201".into())],
        ..unit("986779795", "Wang Fredrikstad AS", "3107")
    }
}

/// Den norske skole - Gran Canaria, U90099017: abroad (2599). Out of scope.
pub(super) fn gran_canaria() -> NsrUnit {
    NsrUnit {
        is_private: true,
        ..unit("U90099017", "Den norske skole - Gran Canaria", "2599")
    }
}

/// Longyearbyen skole grunnskole, 974795655: Svalbard (2100).
pub(super) fn longyearbyen() -> NsrUnit {
    NsrUnit {
        category_ids: vec!["1".into(), "2".into(), "3".into(), "5".into(), "32".into()],
        nace: vec![(1, "85.201".into()), (2, "85.310".into())],
        grade_to: Some(10),
        visiting: address("Vei 500 156", "9170", "LONGYEARBYEN"),
        postal: address("Postboks 350", "9171", "LONGYEARBYEN"),
        changed_at: Some(ts("2026-09-13T01:28:40.183Z")),
        ..unit("974795655", "Longyearbyen skole grunnskole", "2100")
    }
}

/// Kjølsdalen montessoriskule SA, 998245508: Nynorsk, private.
pub(super) fn kjolsdalen() -> NsrUnit {
    NsrUnit {
        is_private: true,
        category_ids: vec!["1".into(), "4".into(), "16".into(), "32".into()],
        grade_to: Some(10),
        language: Some("nn".into()),
        website: Some("www.kjolsdalenmontessori.no".into()),
        visiting: address("Daltunvegen 19", "6776", "KJØLSDALEN"),
        changed_at: Some(ts("2026-09-13T01:39:54.1Z")),
        ..unit("998245508", "Kjølsdalen montessoriskule SA", "4649")
    }
}

/// Halsa barne- og ungdomsskole, 998670799: no website.
pub(super) fn halsa() -> NsrUnit {
    NsrUnit {
        grade_to: Some(10),
        visiting: address("Glåmsmyrvegen 63", "6683", "VÅGLAND"),
        postal: address("Trondheimsveien 1", "7200", "KYRKSÆTERØRA"),
        changed_at: Some(ts("2026-09-13T01:40:00.23Z")),
        ..unit("998670799", "Halsa barne- og ungdomsskole", "5055")
    }
}

/// Holtålen kommune Haltdalen oppvekstsenter avd skole, 974554682: the municipality in the
/// name.
pub(super) fn haltdalen() -> NsrUnit {
    NsrUnit {
        website: Some("www.holtalenskolene.no".into()),
        visiting: address("Knuten 121", "7383", "HALTDALEN"),
        changed_at: Some(ts("2026-09-13T01:23:55.68Z")),
        ..unit(
            "974554682",
            "Holtålen kommune Haltdalen oppvekstsenter avd skole",
            "5026",
        )
    }
}

/// Stange ungdomsskole's old number, 975270920: closed as "Slettet for sammenslåing".
pub(super) fn stange_old() -> NsrUnit {
    NsrUnit {
        is_active: false,
        grade_from: None,
        grade_to: None,
        website: Some("www.stange.kommune.no".into()),
        visiting: address("Kongsvegen 12", "2335", "STANGE"),
        postal: address("Postboks 214", "2336", "STANGE"),
        closure: Some(NsrClosure {
            code: "F".into(),
            at: Some(ts("2024-08-25T01:15:10.91Z")),
        }),
        changed_at: Some(ts("2026-03-18T13:24:18.55Z")),
        ..unit("975270920", "Stange ungdomsskole", "3413")
    }
}

/// Stange ungdomsskole's new number, 933181995.
pub(super) fn stange_new() -> NsrUnit {
    NsrUnit {
        grade_from: Some(8),
        grade_to: Some(10),
        visiting: address("Ljøstadvegen 7", "2335", "STANGE"),
        postal: address("Postboks 214", "2336", "STANGE"),
        changed_at: Some(ts("2026-09-13T01:19:29.99Z")),
        ..unit("933181995", "Stange ungdomsskole", "3413")
    }
}

/// Every fixture unit, in the README's order.
pub(super) fn fixture_units() -> Vec<NsrUnit> {
    vec![
        hosle(),
        ntg(),
        lerberg(),
        signo(),
        lorenskog_adult(),
        wang(),
        gran_canaria(),
        longyearbyen(),
        kjolsdalen(),
        halsa(),
        haltdalen(),
        stange_old(),
        stange_new(),
    ]
}

/// The listed, active register row a seed would have made from `unit`.
pub(super) fn school(
    id: u32,
    municipality_id: u32,
    unit: &NsrUnit,
    slug: &str,
) -> SchoolSnapshot<u32> {
    SchoolSnapshot {
        id,
        municipality_id,
        origin: Origin::Register,
        orgnr: Some(unit.orgnr.clone()),
        register_name: Some(unit.name.clone()),
        display_name: unit.name.clone(),
        display_name_curated: false,
        slug: Some(slug.into()),
        verification: Verification::Listed,
        status: SchoolStatus::Active,
        in_scope: true,
        scope_override: None,
        has_live_fau: false,
        attributes: SchoolAttributes::from_unit(unit),
    }
}
```

- [ ] **Step 2: Write the failing tests.** Create `backend/crates/domain/src/register/sync/schools.rs`
  containing only this test module:

```rust
#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::super::testkit::{
        address, at, fixture_records, fixture_units, gran_canaria, hosle, inputs, school, ts, unit,
    };
    use super::super::types::RunKind;
    use super::*;
    use crate::register::source::NsrClosure;

    fn state(numbers: &[(&str, Ref<u32>)]) -> MunicipalityState<u32> {
        MunicipalityState {
            by_number: numbers.iter().map(|(n, r)| ((*n).to_owned(), *r)).collect(),
            blocked_numbers: BTreeSet::new(),
            blocked_ids: BTreeSet::new(),
        }
    }

    fn run(
        snapshot: &RegisterSnapshot<u32>,
        units: &[NsrUnit],
        state: &MunicipalityState<u32>,
    ) -> SchoolPlan<u32> {
        plan_schools(snapshot, &inputs(&[], &[], units, RunKind::Sync), state)
    }

    fn with_schools(schools: Vec<SchoolSnapshot<u32>>) -> RegisterSnapshot<u32> {
        RegisterSnapshot {
            schools,
            ..RegisterSnapshot::default()
        }
    }

    /// Bærum is municipality 1 in these tests.
    fn baerum() -> MunicipalityState<u32> {
        state(&[("3201", Ref::Existing(1))])
    }

    fn closed(unit: NsrUnit, code: &str, at: &str) -> NsrUnit {
        NsrUnit {
            is_active: false,
            closure: Some(NsrClosure {
                code: code.into(),
                at: Some(ts(at)),
            }),
            ..unit
        }
    }

    fn slugs(plan: &SchoolPlan<u32>) -> Vec<(String, Ref<u32>, Option<String>)> {
        plan.ops
            .iter()
            .filter_map(|op| match op {
                SchoolOp::Create {
                    orgnr,
                    municipality,
                    slug,
                    ..
                } => Some((orgnr.clone(), *municipality, slug.clone())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_seed_creates_exactly_the_in_scope_fixture_units() {
        // The seed's municipality creates, in fixture_records() order, then Svalbard.
        let mut numbers: Vec<(String, Ref<u32>)> = fixture_records()
            .iter()
            .zip(0..)
            .map(|(r, n)| (r.number.clone(), Ref::New(n)))
            .collect();
        numbers.push(("2100".into(), Ref::New(9)));
        let numbers: Vec<(&str, Ref<u32>)> =
            numbers.iter().map(|(n, r)| (n.as_str(), *r)).collect();
        let plan = run(
            &RegisterSnapshot::default(),
            &fixture_units(),
            &state(&numbers),
        );
        let s = |x: &str| Some(x.to_owned());
        assert_eq!(
            slugs(&plan),
            [
                ("974552124".into(), Ref::New(0), s("hosle-skole")),
                (
                    "990672938".into(),
                    Ref::New(0),
                    s("norges-toppidrettsgymnas-ungdomsskole-baerum-as")
                ),
                (
                    "998516897".into(),
                    Ref::New(1),
                    s("lerberg-skole-og-kompetansesenter")
                ),
                (
                    "998666783".into(),
                    Ref::New(2),
                    s("signo-grunn-og-videregaaende-skole-as")
                ),
                (
                    "974795655".into(),
                    Ref::New(9),
                    s("longyearbyen-skole-grunnskole")
                ),
                (
                    "998245508".into(),
                    Ref::New(5),
                    s("kjoelsdalen-montessoriskule-sa")
                ),
                (
                    "998670799".into(),
                    Ref::New(6),
                    s("halsa-barne-og-ungdomsskole")
                ),
                (
                    "974554682".into(),
                    Ref::New(7),
                    s("holtaalen-kommune-haltdalen-oppvekstsenter-avd-skole")
                ),
                ("933181995".into(), Ref::New(8), s("stange-ungdomsskole")),
            ],
            "adult education, VGS-primary, 2599 and the inactive Stange number are not created"
        );
        assert_eq!(plan.ops.len(), 9, "creates only");
        assert!(
            plan.reviews.is_empty(),
            "an out-of-scope 2599 unit raises nothing: {:?}",
            plan.reviews
        );
        assert_eq!(plan.skipped, 0);
        assert_eq!(
            plan.ops[0],
            SchoolOp::Create {
                new: 0,
                municipality: Ref::New(0),
                orgnr: "974552124".into(),
                register_name: "Hosle skole".into(),
                display_name: "Hosle skole".into(),
                slug: s("hosle-skole"),
                verification: CreateVerification::Listed,
                in_scope: true,
                attributes: SchoolAttributes::from_unit(&hosle()),
                source_changed_at: Some(ts("2026-09-13T01:05:43.46Z")),
            }
        );
    }

    #[test]
    fn an_unchanged_school_gives_no_op() {
        let snapshot = with_schools(vec![school(10, 1, &hosle(), "hosle-skole")]);
        let plan = run(&snapshot, &[hosle()], &baerum());
        assert!(plan.ops.is_empty() && plan.reviews.is_empty(), "{plan:?}");
    }

    #[test]
    fn a_rename_mints_a_new_slug() {
        let snapshot = with_schools(vec![school(10, 1, &hosle(), "hosle-skole")]);
        let renamed = NsrUnit {
            name: "Hosle barneskole".into(),
            ..hosle()
        };
        let plan = run(&snapshot, &[renamed], &baerum());
        assert_eq!(
            plan.ops,
            [SchoolOp::Rename {
                id: 10,
                register_name: "Hosle barneskole".into(),
                display_name: Some("Hosle barneskole".into()),
                slug: Some(SlugChange {
                    old: "hosle-skole".into(),
                    new: "hosle-barneskole".into(),
                }),
            }]
        );
    }

    #[test]
    fn a_case_only_rename_keeps_the_slug() {
        let snapshot = with_schools(vec![school(10, 1, &hosle(), "hosle-skole")]);
        let shouted = NsrUnit {
            name: "HOSLE SKOLE".into(),
            ..hosle()
        };
        let plan = run(&snapshot, &[shouted], &baerum());
        assert_eq!(
            plan.ops,
            [SchoolOp::Rename {
                id: 10,
                register_name: "HOSLE SKOLE".into(),
                display_name: Some("HOSLE SKOLE".into()),
                slug: None,
            }]
        );
    }

    #[test]
    fn a_curated_display_name_is_kept() {
        let curated = SchoolSnapshot {
            display_name: "Hosle skole (Bærum)".into(),
            display_name_curated: true,
            ..school(10, 1, &hosle(), "hosle-skole")
        };
        let renamed = NsrUnit {
            name: "Hosle barneskole".into(),
            ..hosle()
        };
        let plan = run(&with_schools(vec![curated]), &[renamed], &baerum());
        assert_eq!(
            plan.ops,
            [SchoolOp::Rename {
                id: 10,
                register_name: "Hosle barneskole".into(),
                display_name: None,
                slug: None,
            }]
        );
    }

    #[test]
    fn changed_attributes_are_updated() {
        let snapshot = with_schools(vec![school(10, 1, &hosle(), "hosle-skole")]);
        let moved_site = NsrUnit {
            website: Some("hosle.baerum.kommune.no".into()),
            ..hosle()
        };
        let plan = run(&snapshot, std::slice::from_ref(&moved_site), &baerum());
        assert_eq!(
            plan.ops,
            [SchoolOp::UpdateAttributes {
                id: 10,
                attributes: SchoolAttributes::from_unit(&moved_site),
            }]
        );
    }

    #[test]
    fn a_closure_without_an_fau_closes_the_school() {
        let snapshot = with_schools(vec![school(10, 1, &hosle(), "hosle-skole")]);
        // 22:30 UTC on 31 July is 00:30 on 1 August in Oslo.
        let cases = [
            ("N", ClosureReason::Closed),
            ("F", ClosureReason::Merged),
            ("D", ClosureReason::Duplicate),
        ];
        for (code, reason) in cases {
            let plan = run(
                &snapshot,
                &[closed(hosle(), code, "2026-07-31T22:30:00Z")],
                &baerum(),
            );
            assert_eq!(
                plan.ops,
                [SchoolOp::Close {
                    id: 10,
                    reason,
                    closed_on: date(2026, 8, 1),
                    successor: None,
                }],
                "{code}"
            );
            assert!(plan.reviews.is_empty());
        }

        // Inactive with no closure record: closed today.
        let inactive = NsrUnit {
            is_active: false,
            ..hosle()
        };
        let plan = run(&snapshot, &[inactive], &baerum());
        assert_eq!(
            plan.ops,
            [SchoolOp::Close {
                id: 10,
                reason: ClosureReason::Closed,
                closed_on: at().today(),
                successor: None,
            }]
        );
    }

    #[test]
    fn an_active_unit_that_left_scope_closes_as_out_of_scope() {
        let adult = NsrUnit {
            category_ids: vec!["10".into()],
            ..hosle()
        };
        let snapshot = with_schools(vec![school(10, 1, &hosle(), "hosle-skole")]);
        let plan = run(&snapshot, std::slice::from_ref(&adult), &baerum());
        let expected = [SchoolOp::Close {
            id: 10,
            reason: ClosureReason::OutOfScope,
            closed_on: date(2026, 9, 28),
            successor: None,
        }];
        assert_eq!(plan.ops, expected);

        // An operator's override wins both ways (D2).
        let kept = SchoolSnapshot {
            scope_override: Some(true),
            ..school(10, 1, &hosle(), "hosle-skole")
        };
        let plan = run(&with_schools(vec![kept]), &[adult], &baerum());
        assert!(
            !plan
                .ops
                .iter()
                .any(|op| matches!(op, SchoolOp::Close { .. })),
            "{:?}",
            plan.ops
        );
        let marked_out = SchoolSnapshot {
            scope_override: Some(false),
            ..school(10, 1, &hosle(), "hosle-skole")
        };
        let plan = run(&with_schools(vec![marked_out]), &[hosle()], &baerum());
        assert_eq!(plan.ops, expected);
    }

    #[test]
    fn a_closure_with_an_fau_is_reviewed_not_applied() {
        let attached = SchoolSnapshot {
            has_live_fau: true,
            ..school(10, 1, &hosle(), "hosle-skole")
        };
        let plan = run(
            &with_schools(vec![attached]),
            &[closed(hosle(), "N", "2026-07-31T22:30:00Z")],
            &baerum(),
        );
        assert!(plan.ops.is_empty(), "{:?}", plan.ops);
        assert_eq!(
            plan.reviews,
            [ReviewItem {
                kind: ReviewKind::ClosureWithFau,
                school: Some(Ref::Existing(10)),
                other_school: None,
                municipality: Some(Ref::Existing(1)),
                details: vec![
                    ("orgnr", "974552124".into()),
                    ("name", "Hosle skole".into()),
                    ("reason", "closed".into()),
                    ("closed_on", "2026-08-01".into()),
                ],
            }]
        );
    }

    #[test]
    fn held_pending_and_rejected_rows_are_untouched() {
        for verification in [
            Verification::Held,
            Verification::Pending,
            Verification::Rejected,
        ] {
            let row = SchoolSnapshot {
                verification,
                slug: None,
                ..school(10, 1, &hosle(), "unused")
            };
            let renamed_and_closed = NsrUnit {
                name: "Hosle barneskole".into(),
                ..closed(hosle(), "N", "2026-07-31T22:30:00Z")
            };
            for u in [hosle(), renamed_and_closed] {
                let plan = run(&with_schools(vec![row.clone()]), &[u], &baerum());
                assert!(
                    plan.ops.is_empty() && plan.reviews.is_empty(),
                    "{verification:?}: {plan:?}"
                );
            }
        }
    }

    #[test]
    fn a_closed_school_is_never_reopened_or_recreated() {
        let gone = SchoolSnapshot {
            status: SchoolStatus::Closed,
            slug: None,
            ..school(10, 1, &hosle(), "unused")
        };
        let plan = run(&with_schools(vec![gone]), &[hosle()], &baerum());
        assert!(plan.ops.is_empty() && plan.reviews.is_empty(), "{plan:?}");
    }

    #[test]
    fn a_former_orgnr_neither_closes_nor_duplicates_its_school() {
        // Stange ungdomsskole kept its row under the new number; the old one is history.
        let row = school(
            10,
            1,
            &unit("933181995", "Stange ungdomsskole", "3201"),
            "stange-ungdomsskole",
        );
        let snapshot = RegisterSnapshot {
            schools: vec![row],
            school_orgnr_history: vec![("975270920".into(), 10)],
            ..RegisterSnapshot::default()
        };
        let old_closed = closed(
            unit("975270920", "Stange ungdomsskole", "3201"),
            "F",
            "2024-08-25T01:15:10.91Z",
        );
        let reactivated = unit("975270920", "Stange ungdomsskole", "3201");
        for u in [old_closed, reactivated] {
            let plan = run(
                &snapshot,
                &[unit("933181995", "Stange ungdomsskole", "3201"), u],
                &baerum(),
            );
            assert!(plan.ops.is_empty() && plan.reviews.is_empty(), "{plan:?}");
        }
    }

    #[test]
    fn an_unknown_municipality_number_skips_the_unit_with_one_item_per_number() {
        let hosle_row = school(10, 1, &hosle(), "hosle-skole");
        let hosle_moved_to_9999 = NsrUnit {
            municipality_number: "9999".into(),
            ..hosle()
        };
        let units = [
            hosle_moved_to_9999,
            unit("900000001", "Ny skole", "9999"),
            unit("900000002", "Annen skole", "8888"),
            gran_canaria(),
        ];
        let plan = run(&with_schools(vec![hosle_row]), &units, &baerum());
        assert!(
            plan.ops.is_empty(),
            "the existing school is skipped, not closed or moved: {:?}",
            plan.ops
        );
        assert_eq!(plan.skipped, 3);
        let item = |number: &str, count: &str, orgnrs: &str| ReviewItem {
            kind: ReviewKind::UnknownMunicipalityNumber,
            school: None,
            other_school: None,
            municipality: None,
            details: vec![
                ("municipality_number", number.into()),
                ("unit_count", count.into()),
                ("orgnrs", orgnrs.into()),
            ],
        };
        assert_eq!(
            plan.reviews,
            [
                item("8888", "1", "900000002"),
                item("9999", "2", "974552124,900000001"),
            ],
            "nothing for the out-of-scope 2599 unit"
        );
    }

    #[test]
    fn a_blocked_municipality_skips_its_schools() {
        let brattvaag = unit("974585715", "Brattvåg barneskule", "1580");
        let spjelkavik = unit("974585723", "Spjelkavik barneskule", "1508");
        let snapshot = with_schools(vec![
            school(10, 1, &brattvaag, "brattvaag-barneskule"),
            school(11, 1, &spjelkavik, "spjelkavik-barneskule"),
        ]);
        let state = MunicipalityState {
            by_number: BTreeMap::from([("1507".to_owned(), Ref::Existing(1))]),
            blocked_numbers: BTreeSet::from(["1507".into(), "1508".into(), "1580".into()]),
            blocked_ids: BTreeSet::from([1]),
        };
        let new_in_haram = unit("900000003", "Ny Haram skule", "1580");
        let plan = run(&snapshot, &[brattvaag, spjelkavik, new_in_haram], &state);
        assert!(plan.ops.is_empty() && plan.reviews.is_empty(), "{plan:?}");
        assert_eq!(plan.skipped, 3);
    }

    #[test]
    fn a_school_whose_number_resolves_elsewhere_moves() {
        // After the 1507 split is resolved, Brattvåg barneskule's NSR number 1580 resolves
        // to Haram (2), while its row is still under Ålesund (1).
        let brattvaag = unit("974585715", "Brattvåg barneskule", "1580");
        let snapshot = with_schools(vec![school(10, 1, &brattvaag, "brattvaag-barneskule")]);
        let state = state(&[("1508", Ref::Existing(1)), ("1580", Ref::Existing(2))]);
        let plan = run(&snapshot, &[brattvaag], &state);
        assert_eq!(
            plan.ops,
            [SchoolOp::Move {
                id: 10,
                from: 1,
                to: Ref::Existing(2),
                slug: Some(SlugChange {
                    old: "brattvaag-barneskule".into(),
                    new: "brattvaag-barneskule".into(),
                }),
            }]
        );
    }

    #[test]
    fn a_second_school_of_the_same_name_takes_its_post_town() {
        let first = NsrUnit {
            visiting: address("Gamle Ringeriksvei 38", "1357", "BEKKESTUA"),
            ..unit("900000004", "Bekkestua skole", "3201")
        };
        let second = NsrUnit {
            visiting: address("Bispeveien 1", "1362", "HOSLE"),
            ..unit("900000005", "Bekkestua skole", "3201")
        };
        let plan = run(&RegisterSnapshot::default(), &[first, second], &baerum());
        assert_eq!(
            slugs(&plan),
            [
                (
                    "900000004".into(),
                    Ref::Existing(1),
                    Some("bekkestua-skole".into())
                ),
                (
                    "900000005".into(),
                    Ref::Existing(1),
                    Some("bekkestua-skole-hosle".into())
                ),
            ]
        );
    }

    #[test]
    fn a_history_slug_is_taken_unless_its_holder_is_closed() {
        // Eikeli was renamed away from "eikeli-skole" and is still active; Jar skole closed.
        let eikeli = SchoolSnapshot {
            display_name: "Eikeli videregående".into(),
            ..school(
                10,
                1,
                &unit("900000006", "Eikeli videregående", "3201"),
                "eikeli-videregaaende",
            )
        };
        let jar = SchoolSnapshot {
            status: SchoolStatus::Closed,
            slug: None,
            ..school(11, 1, &unit("900000007", "Jar skole", "3201"), "unused")
        };
        let snapshot = RegisterSnapshot {
            schools: vec![eikeli, jar],
            school_slug_history: vec![(1, "eikeli-skole".into(), 10), (1, "jar-skole".into(), 11)],
            ..RegisterSnapshot::default()
        };
        let units = [
            unit("900000006", "Eikeli videregående", "3201"),
            unit("900000008", "Eikeli skole", "3201"),
            unit("900000009", "Jar skole", "3201"),
        ];
        let plan = run(&snapshot, &units, &baerum());
        assert_eq!(
            slugs(&plan),
            [
                (
                    "900000008".into(),
                    Ref::Existing(1),
                    Some("eikeli-skole-2".into())
                ),
                (
                    "900000009".into(),
                    Ref::Existing(1),
                    Some("jar-skole".into())
                ),
            ],
            "no post town on these units, so the number follows"
        );
    }

    #[test]
    fn an_unsluggable_name_is_created_without_a_slug() {
        let plan = run(
            &RegisterSnapshot::default(),
            &[unit("900000010", "Школа", "3201")],
            &baerum(),
        );
        assert_eq!(slugs(&plan), [("900000010".into(), Ref::Existing(1), None)]);
    }
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cd /workspace/backend && cargo test -p fau-domain register::sync::schools`
Expected: compile errors, since `plan_schools`, `SchoolPlan` and `MunicipalityState` are not in
scope.

- [ ] **Step 4: Implement.** Put this above the test module in `schools.rs`:

```rust
//! Step 5 of a run: schools (docs/school-register-design.md §4.5, §5.2 step 5, §6).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use jiff::civil::Date;

use crate::register::scope::{classify, effective_in_scope, ScopeDecision};
use crate::register::slug::{first_free_slug, slugify};
use crate::register::source::NsrUnit;
use crate::time::{oslo_today, Moment};

use super::municipalities::MunicipalityState;
use super::types::{
    ClosureReason, CreateVerification, Ref, RegisterSnapshot, ReviewItem, ReviewKind, RowId,
    SchoolAttributes, SchoolOp, SchoolSnapshot, SchoolStatus, SlugChange, SyncInputs, Verification,
};

/// At most this many orgnrs are listed in one `unknown_municipality_number` item, so the
/// details stay under 0004's 2 KB cap.
const MAX_LISTED_ORGNRS: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SchoolPlan<Id> {
    pub ops: Vec<SchoolOp<Id>>,
    pub reviews: Vec<ReviewItem<Id>>,
    /// Units left alone because their municipality is blocked or unknown.
    pub skipped: usize,
}

/// A school without an FAU that this run closes.
struct Closing<'a, Id> {
    school: &'a SchoolSnapshot<Id>,
    reason: ClosureReason,
    closed_on: Date,
    successor: Option<Ref<Id>>,
}

/// A school with an FAU whose unit closed or left scope: reviewed, never closed (D8).
struct FauClosing<'a, Id> {
    school: &'a SchoolSnapshot<Id>,
    reason: ClosureReason,
    closed_on: Date,
}

/// An active, in-scope unit with no school, in a resolved municipality. Every candidate is
/// created, so its index is its `Ref::New` number.
struct Candidate<'a, Id> {
    unit: &'a NsrUnit,
    municipality: Ref<Id>,
}

enum Resolved<Id> {
    To(Ref<Id>),
    Blocked,
    Unknown,
}

fn resolve<Id: RowId>(state: &MunicipalityState<Id>, number: &str) -> Resolved<Id> {
    if state.blocked_numbers.contains(number) {
        return Resolved::Blocked;
    }
    match state.by_number.get(number) {
        Some(r) => Resolved::To(*r),
        None => Resolved::Unknown,
    }
}

/// Who holds which school slug, per municipality, as this plan goes along (§6 collisions).
struct SlugBook<Id> {
    current: HashMap<(Ref<Id>, String), Ref<Id>>,
    /// `(municipality, slug)` to every school that held it.
    history: HashMap<(Ref<Id>, String), Vec<Id>>,
    /// Closed in the snapshot, or closing in this plan.
    closed: BTreeSet<Id>,
}

impl<Id: RowId> SlugBook<Id> {
    fn new(snapshot: &RegisterSnapshot<Id>) -> Self {
        let mut book = SlugBook {
            current: HashMap::new(),
            history: HashMap::new(),
            closed: BTreeSet::new(),
        };
        for s in &snapshot.schools {
            if let Some(slug) = &s.slug {
                book.current.insert(
                    (Ref::Existing(s.municipality_id), slug.clone()),
                    Ref::Existing(s.id),
                );
            }
            if s.status == SchoolStatus::Closed {
                book.closed.insert(s.id);
            }
        }
        for (m, slug, s) in &snapshot.school_slug_history {
            book.history
                .entry((Ref::Existing(*m), slug.clone()))
                .or_default()
                .push(*s);
        }
        book
    }

    /// A closing school gives up its slug, and its history stops counting as taken.
    fn release(&mut self, school: &SchoolSnapshot<Id>) {
        if let Some(slug) = &school.slug {
            self.current
                .remove(&(Ref::Existing(school.municipality_id), slug.clone()));
        }
        self.closed.insert(school.id);
    }

    /// Taken for `me`: held now by another school, or held in history by a different school
    /// that is not closed (ADR-002 rule 3).
    fn is_taken(&self, municipality: Ref<Id>, slug: &str, me: Ref<Id>) -> bool {
        let key = (municipality, slug.to_owned());
        if self.current.get(&key).is_some_and(|holder| *holder != me) {
            return true;
        }
        self.history.get(&key).is_some_and(|holders| {
            holders
                .iter()
                .any(|h| Ref::Existing(*h) != me && !self.closed.contains(h))
        })
    }

    fn take(&mut self, municipality: Ref<Id>, slug: &str, holder: Ref<Id>) {
        self.current.insert((municipality, slug.to_owned()), holder);
    }

    /// §6: the base, then `-<post town>`, then numbered.
    fn mint(
        &mut self,
        municipality: Ref<Id>,
        base: &str,
        post_town: Option<&str>,
        me: Ref<Id>,
    ) -> String {
        let slug = first_free_slug(base, post_town, |c| self.is_taken(municipality, c, me));
        self.take(municipality, &slug, me);
        slug
    }
}

pub(super) fn plan_schools<Id: RowId>(
    snapshot: &RegisterSnapshot<Id>,
    inputs: &SyncInputs<'_>,
    state: &MunicipalityState<Id>,
) -> SchoolPlan<Id> {
    let by_orgnr: HashMap<&str, &SchoolSnapshot<Id>> = snapshot
        .schools
        .iter()
        .filter_map(|s| s.orgnr.as_deref().map(|o| (o, s)))
        .collect();
    let former_orgnrs: BTreeSet<&str> = snapshot
        .school_orgnr_history
        .iter()
        .map(|(o, _)| o.as_str())
        .collect();

    let mut skipped = 0;
    let mut unknown: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut closings: Vec<Closing<'_, Id>> = Vec::new();
    let mut fau_closings: Vec<FauClosing<'_, Id>> = Vec::new();
    let mut updates: Vec<(&SchoolSnapshot<Id>, &NsrUnit, Ref<Id>)> = Vec::new();
    let mut candidates: Vec<Candidate<'_, Id>> = Vec::new();

    for unit in inputs.units {
        let classified = classify(&unit.scope_facts()) == ScopeDecision::InScope;
        let existing = by_orgnr.get(unit.orgnr.as_str()).copied();
        let Some(school) = existing else {
            // A former orgnr only stops a duplicate: its closure must not close the school
            // that carries on under a new number.
            if classified && !former_orgnrs.contains(unit.orgnr.as_str()) {
                match resolve(state, &unit.municipality_number) {
                    Resolved::To(municipality) => candidates.push(Candidate { unit, municipality }),
                    Resolved::Blocked => skipped += 1,
                    Resolved::Unknown => {
                        skipped += 1;
                        unknown
                            .entry(unit.municipality_number.as_str())
                            .or_default()
                            .push(unit.orgnr.as_str());
                    }
                }
            }
            continue;
        };
        // Closed rows are never reopened; held, pending and rejected rows await a human.
        if school.status == SchoolStatus::Closed
            || !matches!(
                school.verification,
                Verification::Listed | Verification::Verified
            )
        {
            continue;
        }
        if state.blocked_ids.contains(&school.municipality_id) {
            skipped += 1;
            continue;
        }
        if !unit.is_active || !effective_in_scope(classified, school.scope_override) {
            let (reason, closed_on) = closure_facts(unit, inputs.at);
            if school.has_live_fau {
                fau_closings.push(FauClosing {
                    school,
                    reason,
                    closed_on,
                });
            } else {
                closings.push(Closing {
                    school,
                    reason,
                    closed_on,
                    successor: None,
                });
            }
            continue;
        }
        match resolve(state, &unit.municipality_number) {
            Resolved::To(municipality) => updates.push((school, unit, municipality)),
            Resolved::Blocked => skipped += 1,
            Resolved::Unknown => {
                skipped += 1;
                unknown
                    .entry(unit.municipality_number.as_str())
                    .or_default()
                    .push(unit.orgnr.as_str());
            }
        }
    }

    let mut book = SlugBook::new(snapshot);
    for c in &closings {
        book.release(c.school);
    }

    let mut reviews = Vec::new();

    let mut update_ops = Vec::new();
    for (school, unit, to) in updates {
        plan_update(school, unit, to, &mut book, &mut update_ops);
    }

    let mut ops: Vec<SchoolOp<Id>> = closings
        .iter()
        .map(|c| SchoolOp::Close {
            id: c.school.id,
            reason: c.reason,
            closed_on: c.closed_on,
            successor: c.successor,
        })
        .collect();
    for (i, cand) in candidates.iter().enumerate() {
        let me = Ref::New(new_number(i));
        let attributes = SchoolAttributes::from_unit(cand.unit);
        // A name with nothing sluggable gets no slug; D5 curation fixes it.
        let slug = slugify(&cand.unit.name).ok().map(|base| {
            book.mint(
                cand.municipality,
                &base,
                attributes.post_town.as_deref(),
                me,
            )
        });
        ops.push(SchoolOp::Create {
            new: new_number(i),
            municipality: cand.municipality,
            orgnr: cand.unit.orgnr.clone(),
            register_name: cand.unit.name.clone(),
            display_name: cand.unit.name.clone(),
            slug,
            verification: CreateVerification::Listed,
            // Only an in-scope unit becomes a candidate.
            in_scope: true,
            attributes,
            source_changed_at: cand.unit.changed_at,
        });
    }
    ops.extend(update_ops);

    for f in &fau_closings {
        reviews.push(ReviewItem {
            kind: ReviewKind::ClosureWithFau,
            school: Some(Ref::Existing(f.school.id)),
            other_school: None,
            municipality: Some(Ref::Existing(f.school.municipality_id)),
            details: vec![
                ("orgnr", f.school.orgnr.clone().unwrap_or_default()),
                ("name", f.school.display_name.clone()),
                ("reason", f.reason.code().to_owned()),
                ("closed_on", f.closed_on.to_string()),
            ],
        });
    }
    for (number, orgnrs) in unknown {
        reviews.push(ReviewItem {
            kind: ReviewKind::UnknownMunicipalityNumber,
            school: None,
            other_school: None,
            municipality: None,
            details: vec![
                ("municipality_number", number.to_owned()),
                ("unit_count", orgnrs.len().to_string()),
                (
                    "orgnrs",
                    orgnrs
                        .iter()
                        .take(MAX_LISTED_ORGNRS)
                        .copied()
                        .collect::<Vec<_>>()
                        .join(","),
                ),
            ],
        });
    }

    SchoolPlan {
        ops,
        reviews,
        skipped,
    }
}

/// `Ref::New` numbers are `u32`; a run never sees four billion new schools.
fn new_number(i: usize) -> u32 {
    u32::try_from(i).expect("fewer than 2^32 new schools in one run")
}

/// Why and when a school closes: the NSR closure code (D duplicate, F merged, anything else
/// closed), or out of scope for a unit that is still active.
fn closure_facts(unit: &NsrUnit, at: Moment) -> (ClosureReason, Date) {
    if unit.is_active {
        return (ClosureReason::OutOfScope, at.today());
    }
    let reason = match unit.closure.as_ref().map(|c| c.code.as_str()) {
        Some("D") => ClosureReason::Duplicate,
        Some("F") => ClosureReason::Merged,
        _ => ClosureReason::Closed,
    };
    let closed_on = unit
        .closure
        .as_ref()
        .and_then(|c| c.at)
        .map(oslo_today)
        .unwrap_or_else(|| at.today());
    (reason, closed_on)
}

/// A continuing school: rename, new attributes, and a move between municipality entities.
fn plan_update<Id: RowId>(
    school: &SchoolSnapshot<Id>,
    unit: &NsrUnit,
    to: Ref<Id>,
    book: &mut SlugBook<Id>,
    ops: &mut Vec<SchoolOp<Id>>,
) {
    let me = Ref::Existing(school.id);
    let here = Ref::Existing(school.municipality_id);
    let moving = to != here;
    let attributes = SchoolAttributes::from_unit(unit);
    let post_town = attributes.post_town.as_deref();

    let renamed = school.register_name.as_deref() != Some(unit.name.as_str());
    let display_name = if renamed && !school.display_name_curated {
        unit.name.as_str()
    } else {
        school.display_name.as_str()
    };
    if renamed {
        // A move re-mints the slug in the target municipality instead.
        let slug = match &school.slug {
            Some(old) if !moving && !school.display_name_curated => slugify(&unit.name)
                .ok()
                .filter(|base| base != old)
                .map(|base| book.mint(here, &base, post_town, me))
                .filter(|new| new != old)
                .map(|new| SlugChange {
                    old: old.clone(),
                    new,
                }),
            _ => None,
        };
        ops.push(SchoolOp::Rename {
            id: school.id,
            register_name: unit.name.clone(),
            display_name: (!school.display_name_curated).then(|| unit.name.clone()),
            slug,
        });
    }
    if attributes != school.attributes {
        ops.push(SchoolOp::UpdateAttributes {
            id: school.id,
            attributes: attributes.clone(),
        });
    }
    if moving {
        // Always a change when the school has a slug: the old one goes to history under the
        // old municipality even if the text is the same.
        let slug = school.slug.as_ref().map(|old| {
            let base = slugify(display_name).unwrap_or_else(|_| old.clone());
            SlugChange {
                old: old.clone(),
                new: book.mint(to, &base, post_town, me),
            }
        });
        ops.push(SchoolOp::Move {
            id: school.id,
            from: school.municipality_id,
            to,
            slug,
        });
    }
}
```

- [ ] **Step 5: Run to verify pass**

Run: `cd /workspace/backend && cargo test -p fau-domain register::sync`
Expected: 39 tests pass (18 new). Then run the full test command, fmt and clippy, all clean.

- [ ] **Step 6: Commit**

```bash
cd /workspace/backend
git add crates/domain/src/register/sync/
GIT_AUTHOR_NAME="Erik W. Bjønnes" GIT_AUTHOR_EMAIL=erik@ewb-solutions.as \
GIT_COMMITTER_NAME="Erik W. Bjønnes" GIT_COMMITTER_EMAIL=erik@ewb-solutions.as \
git commit -m "Plan school creates, renames, moves and closures (#3441)" \
  -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Re-registration and submission holds

**Files:**
- Modify: `backend/crates/domain/src/register/sync/schools.rs`

**Interfaces:**
- Consumes `similarity`, `REREGISTRATION_THRESHOLD` and `SUBMISSION_MATCH_THRESHOLD` (Task 1),
  and everything in `schools.rs` from Task 3.
- Produces the same `plan_schools` signature. Its behaviour now includes re-registrations and
  submission holds. It also adds the internal items `Placement`, `reregistrable`, `place` and
  `best`.

**The rules** (§4.5, the brief, and this plan's F/S decision). For each new unit, in order:
1. **Re-registration.** Look for a predecessor C in the same resolved municipality whose unit is
   closed F or S in this run, with `similarity(C.display_name, unit.name) >= 0.6`:
   - **C has an FAU:** create the new school Held with no slug, and raise
     `possible_reregistration`. It replaces C's `closure_with_fau`, and C is not closed.
   - **C has no FAU:** create it Listed with C's slug, and C's `Close` names it as `successor`.
2. **Submission match.** Otherwise, look for a Pending or Verified submitted school in the same
   municipality with similarity at least 0.45. If there is one, create the new school Held with no
   slug, and raise `possible_submission_match`.
3. **Otherwise** create it Listed with a minted slug, as in Task 3.

- [ ] **Step 1: Write the failing tests.** In the test module of `schools.rs`, replace

```rust
    use super::super::testkit::{
        address, at, fixture_records, fixture_units, gran_canaria, hosle, inputs, school, ts, unit,
    };
```

with

```rust
    use super::super::testkit::{
        address, at, fixture_records, fixture_units, gran_canaria, hosle, inputs, school,
        stange_new, stange_old, ts, unit,
    };
```

and paste the following inside `mod tests`, after the last test and before the module's closing
`}`:

```rust
    /// Stange is municipality 20 in these tests.
    fn stange() -> MunicipalityState<u32> {
        state(&[("3413", Ref::Existing(20))])
    }

    fn stange_row(has_live_fau: bool) -> SchoolSnapshot<u32> {
        SchoolSnapshot {
            has_live_fau,
            ..school(30, 20, &stange_old(), "stange-ungdomsskole")
        }
    }

    fn stange_create(slug: Option<&str>, verification: CreateVerification) -> SchoolOp<u32> {
        SchoolOp::Create {
            new: 0,
            municipality: Ref::Existing(20),
            orgnr: "933181995".into(),
            register_name: "Stange ungdomsskole".into(),
            display_name: "Stange ungdomsskole".into(),
            slug: slug.map(str::to_owned),
            verification,
            in_scope: true,
            attributes: SchoolAttributes::from_unit(&stange_new()),
            source_changed_at: Some(ts("2026-09-13T01:19:29.99Z")),
        }
    }

    #[test]
    fn a_reregistration_without_an_fau_hands_over_the_slug() {
        // 975270920 closed as "Slettet for sammenslåing" on 25 August 2024; 933181995 is the
        // same school under a new number.
        let plan = run(
            &with_schools(vec![stange_row(false)]),
            &[stange_old(), stange_new()],
            &stange(),
        );
        assert_eq!(
            plan.ops,
            [
                SchoolOp::Close {
                    id: 30,
                    reason: ClosureReason::Merged,
                    closed_on: date(2024, 8, 25),
                    successor: Some(Ref::New(0)),
                },
                stange_create(Some("stange-ungdomsskole"), CreateVerification::Listed),
            ]
        );
        assert!(plan.reviews.is_empty(), "{:?}", plan.reviews);
    }

    #[test]
    fn a_reregistration_with_an_fau_is_held_for_review() {
        let plan = run(
            &with_schools(vec![stange_row(true)]),
            &[stange_old(), stange_new()],
            &stange(),
        );
        assert_eq!(
            plan.ops,
            [stange_create(None, CreateVerification::Held)],
            "no close, and a held row has no slug"
        );
        assert_eq!(
            plan.reviews,
            [ReviewItem {
                kind: ReviewKind::PossibleReregistration,
                school: Some(Ref::Existing(30)),
                other_school: Some(Ref::New(0)),
                municipality: Some(Ref::Existing(20)),
                details: vec![
                    ("old_orgnr", "975270920".into()),
                    ("new_orgnr", "933181995".into()),
                    ("old_name", "Stange ungdomsskole".into()),
                    ("new_name", "Stange ungdomsskole".into()),
                    ("similarity", "1.00".into()),
                ],
            }],
            "instead of closure_with_fau"
        );
    }

    #[test]
    fn an_fau_school_that_only_left_scope_is_no_reregistration() {
        // Still active in NSR, but marked out of scope: the new unit is its own school.
        let out = SchoolSnapshot {
            scope_override: Some(false),
            ..stange_row(true)
        };
        let still_active_old = NsrUnit {
            is_active: true,
            closure: None,
            ..stange_old()
        };
        let plan = run(
            &with_schools(vec![out]),
            &[still_active_old, stange_new()],
            &stange(),
        );
        assert_eq!(
            plan.ops,
            [stange_create(
                Some("stange-ungdomsskole-stange"),
                CreateVerification::Listed
            )],
            "the FAU school keeps its slug, so the new one takes its post town"
        );
        assert_eq!(plan.reviews.len(), 1);
        assert_eq!(plan.reviews[0].kind, ReviewKind::ClosureWithFau);
        assert_eq!(
            plan.reviews[0].details[2],
            ("reason", "out_of_scope".to_owned())
        );
    }

    #[test]
    fn a_closing_school_below_the_threshold_gets_no_successor() {
        // similarity("Stange skole", "Stange ungdomsskole") is 0.5238, below 0.6.
        let old = SchoolSnapshot {
            display_name: "Stange skole".into(),
            register_name: Some("Stange skole".into()),
            ..school(30, 20, &stange_old(), "stange-skole")
        };
        let old_unit = NsrUnit {
            name: "Stange skole".into(),
            ..stange_old()
        };
        let plan = run(
            &with_schools(vec![old]),
            &[old_unit, stange_new()],
            &stange(),
        );
        assert_eq!(
            plan.ops,
            [
                SchoolOp::Close {
                    id: 30,
                    reason: ClosureReason::Merged,
                    closed_on: date(2024, 8, 25),
                    successor: None,
                },
                stange_create(Some("stange-ungdomsskole"), CreateVerification::Listed),
            ]
        );
    }

    #[test]
    fn only_an_f_or_s_closure_is_a_reregistration() {
        let closed_as = |code: &str| NsrUnit {
            closure: Some(NsrClosure {
                code: code.into(),
                at: Some(ts("2024-08-25T01:15:10.91Z")),
            }),
            ..stange_old()
        };
        let successor = |plan: &SchoolPlan<u32>| match &plan.ops[0] {
            SchoolOp::Close { successor, .. } => *successor,
            other => panic!("expected a close first, got {other:?}"),
        };
        for code in ["F", "S"] {
            let plan = run(
                &with_schools(vec![stange_row(false)]),
                &[closed_as(code), stange_new()],
                &stange(),
            );
            assert_eq!(successor(&plan), Some(Ref::New(0)), "{code}");
        }
        for code in ["D", "N", "O", "U"] {
            let plan = run(
                &with_schools(vec![stange_row(false)]),
                &[closed_as(code), stange_new()],
                &stange(),
            );
            assert_eq!(successor(&plan), None, "{code}");
            assert_eq!(
                plan.ops[1],
                stange_create(Some("stange-ungdomsskole"), CreateVerification::Listed),
                "a fresh mint: the closing school released the slug"
            );

            let plan = run(
                &with_schools(vec![stange_row(true)]),
                &[closed_as(code), stange_new()],
                &stange(),
            );
            assert_eq!(
                plan.ops,
                [stange_create(
                    Some("stange-ungdomsskole-stange"),
                    CreateVerification::Listed
                )],
                "{code}: the FAU school is reviewed and keeps its slug"
            );
            assert_eq!(plan.reviews[0].kind, ReviewKind::ClosureWithFau);
        }
    }

    fn fornebu_submission(verification: Verification, municipality_id: u32) -> SchoolSnapshot<u32> {
        SchoolSnapshot {
            id: 40,
            municipality_id,
            origin: Origin::Submitted,
            orgnr: None,
            register_name: None,
            display_name: "Fornebu".into(),
            display_name_curated: false,
            slug: None,
            verification,
            status: SchoolStatus::Active,
            in_scope: true,
            scope_override: None,
            has_live_fau: true,
            attributes: SchoolAttributes::default(),
        }
    }

    #[test]
    fn a_unit_similar_to_a_submitted_school_is_held() {
        // similarity("Fornebu", "Fornebu skole") is 0.5714: at least 0.45.
        let fornebu = unit("900000011", "Fornebu skole", "3201");
        for verification in [Verification::Pending, Verification::Verified] {
            let plan = run(
                &with_schools(vec![fornebu_submission(verification, 1)]),
                std::slice::from_ref(&fornebu),
                &baerum(),
            );
            assert_eq!(slugs(&plan), [("900000011".into(), Ref::Existing(1), None)]);
            assert!(matches!(
                plan.ops[0],
                SchoolOp::Create {
                    verification: CreateVerification::Held,
                    ..
                }
            ));
            assert_eq!(
                plan.reviews,
                [ReviewItem {
                    kind: ReviewKind::PossibleSubmissionMatch,
                    school: Some(Ref::Existing(40)),
                    other_school: Some(Ref::New(0)),
                    municipality: Some(Ref::Existing(1)),
                    details: vec![
                        ("orgnr", "900000011".into()),
                        ("register_name", "Fornebu skole".into()),
                        ("submitted_name", "Fornebu".into()),
                        ("similarity", "0.57".into()),
                    ],
                }],
                "{verification:?}"
            );
        }
    }

    #[test]
    fn a_submission_match_needs_the_same_municipality_a_live_submission_and_the_threshold() {
        let listed = |plan: &SchoolPlan<u32>| {
            plan.reviews.is_empty()
                && matches!(
                    plan.ops[..],
                    [SchoolOp::Create {
                        verification: CreateVerification::Listed,
                        ..
                    }]
                )
        };
        let fornebu = unit("900000011", "Fornebu skole", "3201");
        let elsewhere = fornebu_submission(Verification::Pending, 2);
        let rejected = fornebu_submission(Verification::Rejected, 1);
        for row in [elsewhere, rejected] {
            let plan = run(
                &with_schools(vec![row]),
                std::slice::from_ref(&fornebu),
                &baerum(),
            );
            assert!(listed(&plan), "{plan:?}");
        }
        // similarity("Lerberg skole", "Lerberg skole og kompetansesenter") is 0.4118.
        let lerberg_submitted = SchoolSnapshot {
            display_name: "Lerberg skole".into(),
            ..fornebu_submission(Verification::Pending, 1)
        };
        let lerberg = unit("998516897", "Lerberg skole og kompetansesenter", "3201");
        let plan = run(
            &with_schools(vec![lerberg_submitted]),
            &[lerberg],
            &baerum(),
        );
        assert!(listed(&plan), "{plan:?}");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cd /workspace/backend && cargo test -p fau-domain register::sync::schools`
Expected: a compile error, because `Origin` is not in scope. The new tests use it through
`use super::*`, and only Task 4's code imports it.

- [ ] **Step 3: Implement.** In `schools.rs`, replace everything above the `#[cfg(test)]` line with:

```rust
//! Step 5 of a run: schools (docs/school-register-design.md §4.5, §5.2 step 5, §6).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use jiff::civil::Date;

use crate::register::scope::{classify, effective_in_scope, ScopeDecision};
use crate::register::slug::{first_free_slug, slugify};
use crate::register::source::NsrUnit;
use crate::time::{oslo_today, Moment};

use super::municipalities::MunicipalityState;
use super::similarity::{similarity, REREGISTRATION_THRESHOLD, SUBMISSION_MATCH_THRESHOLD};
use super::types::{
    ClosureReason, CreateVerification, Origin, Ref, RegisterSnapshot, ReviewItem, ReviewKind,
    RowId, SchoolAttributes, SchoolOp, SchoolSnapshot, SchoolStatus, SlugChange, SyncInputs,
    Verification,
};

/// At most this many orgnrs are listed in one `unknown_municipality_number` item, so the
/// details stay under 0004's 2 KB cap.
const MAX_LISTED_ORGNRS: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SchoolPlan<Id> {
    pub ops: Vec<SchoolOp<Id>>,
    pub reviews: Vec<ReviewItem<Id>>,
    /// Units left alone because their municipality is blocked or unknown.
    pub skipped: usize,
}

/// A school without an FAU that this run closes.
struct Closing<'a, Id> {
    school: &'a SchoolSnapshot<Id>,
    reason: ClosureReason,
    closed_on: Date,
    /// See [`reregistrable`].
    reregistrable: bool,
    successor: Option<Ref<Id>>,
}

/// A school with an FAU whose unit closed or left scope: reviewed, never closed (D8).
struct FauClosing<'a, Id> {
    school: &'a SchoolSnapshot<Id>,
    /// See [`reregistrable`].
    reregistrable: bool,
    reason: ClosureReason,
    closed_on: Date,
    /// A new unit in this run is its possible re-registration.
    reregistered: bool,
}

/// An active, in-scope unit with no school, in a resolved municipality. Every candidate is
/// created, so its index is its `Ref::New` number.
struct Candidate<'a, Id> {
    unit: &'a NsrUnit,
    municipality: Ref<Id>,
}

/// How a candidate is created (§4.5).
enum Placement {
    /// Takes over a closing school's slug. `None` if that school had none.
    Successor(Option<String>),
    /// Held: a school with an FAU may be the same school.
    HeldReregistration,
    /// Held: a submitted school may be the same school.
    HeldSubmission,
    /// Listed, with a freshly minted slug.
    Listed,
}

enum Resolved<Id> {
    To(Ref<Id>),
    Blocked,
    Unknown,
}

fn resolve<Id: RowId>(state: &MunicipalityState<Id>, number: &str) -> Resolved<Id> {
    if state.blocked_numbers.contains(number) {
        return Resolved::Blocked;
    }
    match state.by_number.get(number) {
        Some(r) => Resolved::To(*r),
        None => Resolved::Unknown,
    }
}

/// Who holds which school slug, per municipality, as this plan goes along (§6 collisions).
struct SlugBook<Id> {
    current: HashMap<(Ref<Id>, String), Ref<Id>>,
    /// `(municipality, slug)` to every school that held it.
    history: HashMap<(Ref<Id>, String), Vec<Id>>,
    /// Closed in the snapshot, or closing in this plan.
    closed: BTreeSet<Id>,
}

impl<Id: RowId> SlugBook<Id> {
    fn new(snapshot: &RegisterSnapshot<Id>) -> Self {
        let mut book = SlugBook {
            current: HashMap::new(),
            history: HashMap::new(),
            closed: BTreeSet::new(),
        };
        for s in &snapshot.schools {
            if let Some(slug) = &s.slug {
                book.current.insert(
                    (Ref::Existing(s.municipality_id), slug.clone()),
                    Ref::Existing(s.id),
                );
            }
            if s.status == SchoolStatus::Closed {
                book.closed.insert(s.id);
            }
        }
        for (m, slug, s) in &snapshot.school_slug_history {
            book.history
                .entry((Ref::Existing(*m), slug.clone()))
                .or_default()
                .push(*s);
        }
        book
    }

    /// A closing school gives up its slug, and its history stops counting as taken.
    fn release(&mut self, school: &SchoolSnapshot<Id>) {
        if let Some(slug) = &school.slug {
            self.current
                .remove(&(Ref::Existing(school.municipality_id), slug.clone()));
        }
        self.closed.insert(school.id);
    }

    /// Taken for `me`: held now by another school, or held in history by a different school
    /// that is not closed (ADR-002 rule 3).
    fn is_taken(&self, municipality: Ref<Id>, slug: &str, me: Ref<Id>) -> bool {
        let key = (municipality, slug.to_owned());
        if self.current.get(&key).is_some_and(|holder| *holder != me) {
            return true;
        }
        self.history.get(&key).is_some_and(|holders| {
            holders
                .iter()
                .any(|h| Ref::Existing(*h) != me && !self.closed.contains(h))
        })
    }

    fn take(&mut self, municipality: Ref<Id>, slug: &str, holder: Ref<Id>) {
        self.current.insert((municipality, slug.to_owned()), holder);
    }

    /// §6: the base, then `-<post town>`, then numbered.
    fn mint(
        &mut self,
        municipality: Ref<Id>,
        base: &str,
        post_town: Option<&str>,
        me: Ref<Id>,
    ) -> String {
        let slug = first_free_slug(base, post_town, |c| self.is_taken(municipality, c, me));
        self.take(municipality, &slug, me);
        slug
    }
}

pub(super) fn plan_schools<Id: RowId>(
    snapshot: &RegisterSnapshot<Id>,
    inputs: &SyncInputs<'_>,
    state: &MunicipalityState<Id>,
) -> SchoolPlan<Id> {
    let by_orgnr: HashMap<&str, &SchoolSnapshot<Id>> = snapshot
        .schools
        .iter()
        .filter_map(|s| s.orgnr.as_deref().map(|o| (o, s)))
        .collect();
    let former_orgnrs: BTreeSet<&str> = snapshot
        .school_orgnr_history
        .iter()
        .map(|(o, _)| o.as_str())
        .collect();

    let mut skipped = 0;
    let mut unknown: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut closings: Vec<Closing<'_, Id>> = Vec::new();
    let mut fau_closings: Vec<FauClosing<'_, Id>> = Vec::new();
    let mut updates: Vec<(&SchoolSnapshot<Id>, &NsrUnit, Ref<Id>)> = Vec::new();
    let mut candidates: Vec<Candidate<'_, Id>> = Vec::new();

    for unit in inputs.units {
        let classified = classify(&unit.scope_facts()) == ScopeDecision::InScope;
        let existing = by_orgnr.get(unit.orgnr.as_str()).copied();
        let Some(school) = existing else {
            // A former orgnr only stops a duplicate: its closure must not close the school
            // that carries on under a new number.
            if classified && !former_orgnrs.contains(unit.orgnr.as_str()) {
                match resolve(state, &unit.municipality_number) {
                    Resolved::To(municipality) => candidates.push(Candidate { unit, municipality }),
                    Resolved::Blocked => skipped += 1,
                    Resolved::Unknown => {
                        skipped += 1;
                        unknown
                            .entry(unit.municipality_number.as_str())
                            .or_default()
                            .push(unit.orgnr.as_str());
                    }
                }
            }
            continue;
        };
        // Closed rows are never reopened; held, pending and rejected rows await a human.
        if school.status == SchoolStatus::Closed
            || !matches!(
                school.verification,
                Verification::Listed | Verification::Verified
            )
        {
            continue;
        }
        if state.blocked_ids.contains(&school.municipality_id) {
            skipped += 1;
            continue;
        }
        if !unit.is_active || !effective_in_scope(classified, school.scope_override) {
            let (reason, closed_on) = closure_facts(unit, inputs.at);
            if school.has_live_fau {
                fau_closings.push(FauClosing {
                    school,
                    reregistrable: reregistrable(unit),
                    reason,
                    closed_on,
                    reregistered: false,
                });
            } else {
                closings.push(Closing {
                    school,
                    reason,
                    closed_on,
                    reregistrable: reregistrable(unit),
                    successor: None,
                });
            }
            continue;
        }
        match resolve(state, &unit.municipality_number) {
            Resolved::To(municipality) => updates.push((school, unit, municipality)),
            Resolved::Blocked => skipped += 1,
            Resolved::Unknown => {
                skipped += 1;
                unknown
                    .entry(unit.municipality_number.as_str())
                    .or_default()
                    .push(unit.orgnr.as_str());
            }
        }
    }

    let mut book = SlugBook::new(snapshot);
    for c in &closings {
        book.release(c.school);
    }

    let mut reviews = Vec::new();

    // Re-registrations and submission matches first, so a successor gets its predecessor's
    // slug before any rename can take it (§4.5).
    let mut placements = Vec::with_capacity(candidates.len());
    for (i, cand) in candidates.iter().enumerate() {
        let me = Ref::New(new_number(i));
        let placement = place(
            cand,
            me,
            snapshot,
            &mut closings,
            &mut fau_closings,
            &mut reviews,
        );
        if let Placement::Successor(Some(slug)) = &placement {
            book.take(cand.municipality, slug, me);
        }
        placements.push(placement);
    }

    let mut update_ops = Vec::new();
    for (school, unit, to) in updates {
        plan_update(school, unit, to, &mut book, &mut update_ops);
    }

    let mut ops: Vec<SchoolOp<Id>> = closings
        .iter()
        .map(|c| SchoolOp::Close {
            id: c.school.id,
            reason: c.reason,
            closed_on: c.closed_on,
            successor: c.successor,
        })
        .collect();
    for (i, (cand, placement)) in candidates.iter().zip(placements).enumerate() {
        let me = Ref::New(new_number(i));
        let attributes = SchoolAttributes::from_unit(cand.unit);
        let (verification, slug) = match placement {
            Placement::Successor(Some(slug)) => (CreateVerification::Listed, Some(slug)),
            Placement::HeldReregistration | Placement::HeldSubmission => {
                (CreateVerification::Held, None)
            }
            Placement::Successor(None) | Placement::Listed => (
                CreateVerification::Listed,
                // A name with nothing sluggable gets no slug; D5 curation fixes it.
                slugify(&cand.unit.name).ok().map(|base| {
                    book.mint(
                        cand.municipality,
                        &base,
                        attributes.post_town.as_deref(),
                        me,
                    )
                }),
            ),
        };
        ops.push(SchoolOp::Create {
            new: new_number(i),
            municipality: cand.municipality,
            orgnr: cand.unit.orgnr.clone(),
            register_name: cand.unit.name.clone(),
            display_name: cand.unit.name.clone(),
            slug,
            verification,
            // Only an in-scope unit becomes a candidate.
            in_scope: true,
            attributes,
            source_changed_at: cand.unit.changed_at,
        });
    }
    ops.extend(update_ops);

    for f in &fau_closings {
        if !f.reregistered {
            reviews.push(ReviewItem {
                kind: ReviewKind::ClosureWithFau,
                school: Some(Ref::Existing(f.school.id)),
                other_school: None,
                municipality: Some(Ref::Existing(f.school.municipality_id)),
                details: vec![
                    ("orgnr", f.school.orgnr.clone().unwrap_or_default()),
                    ("name", f.school.display_name.clone()),
                    ("reason", f.reason.code().to_owned()),
                    ("closed_on", f.closed_on.to_string()),
                ],
            });
        }
    }
    for (number, orgnrs) in unknown {
        reviews.push(ReviewItem {
            kind: ReviewKind::UnknownMunicipalityNumber,
            school: None,
            other_school: None,
            municipality: None,
            details: vec![
                ("municipality_number", number.to_owned()),
                ("unit_count", orgnrs.len().to_string()),
                (
                    "orgnrs",
                    orgnrs
                        .iter()
                        .take(MAX_LISTED_ORGNRS)
                        .copied()
                        .collect::<Vec<_>>()
                        .join(","),
                ),
            ],
        });
    }

    SchoolPlan {
        ops,
        reviews,
        skipped,
    }
}

/// `Ref::New` numbers are `u32`; a run never sees four billion new schools.
fn new_number(i: usize) -> u32 {
    u32::try_from(i).expect("fewer than 2^32 new schools in one run")
}

/// Why and when a school closes: the NSR closure code (D duplicate, F merged, anything else
/// closed), or out of scope for a unit that is still active.
fn closure_facts(unit: &NsrUnit, at: Moment) -> (ClosureReason, Date) {
    if unit.is_active {
        return (ClosureReason::OutOfScope, at.today());
    }
    let reason = match unit.closure.as_ref().map(|c| c.code.as_str()) {
        Some("D") => ClosureReason::Duplicate,
        Some("F") => ClosureReason::Merged,
        _ => ClosureReason::Closed,
    };
    let closed_on = unit
        .closure
        .as_ref()
        .and_then(|c| c.at)
        .map(oslo_today)
        .unwrap_or_else(|| at.today());
    (reason, closed_on)
}

/// §4.5: only a unit closed as "Slettet for sammenslåing" (F) or "Slettet" (S) can have
/// re-registered under a new number.
fn reregistrable(unit: &NsrUnit) -> bool {
    !unit.is_active
        && unit
            .closure
            .as_ref()
            .is_some_and(|c| c.code == "F" || c.code == "S")
}

/// §4.5: a re-registration of a school closing in this run, else a possible match with a
/// submitted school, else an ordinary listed school.
fn place<Id: RowId>(
    cand: &Candidate<'_, Id>,
    me: Ref<Id>,
    snapshot: &RegisterSnapshot<Id>,
    closings: &mut [Closing<'_, Id>],
    fau_closings: &mut [FauClosing<'_, Id>],
    reviews: &mut Vec<ReviewItem<Id>>,
) -> Placement {
    let name = cand.unit.name.as_str();
    let in_municipality =
        |s: &SchoolSnapshot<Id>| Ref::Existing(s.municipality_id) == cand.municipality;

    let best_closing = best(
        closings
            .iter()
            .enumerate()
            .filter(|(_, c)| c.reregistrable && c.successor.is_none() && in_municipality(c.school)),
        |(_, c)| similarity(&c.school.display_name, name),
        REREGISTRATION_THRESHOLD,
    );
    let best_fau = best(
        fau_closings
            .iter()
            .enumerate()
            .filter(|(_, f)| f.reregistrable && !f.reregistered && in_municipality(f.school)),
        |(_, f)| similarity(&f.school.display_name, name),
        REREGISTRATION_THRESHOLD,
    );
    // The more similar predecessor wins; on a tie, the one with an FAU, which a human sees.
    let pick_fau = match (&best_closing, &best_fau) {
        (Some((_, c)), Some((_, f))) => f >= c,
        (None, Some(_)) => true,
        _ => false,
    };
    if pick_fau {
        let ((i, _), sim) = best_fau.expect("pick_fau implies a match");
        let f = &mut fau_closings[i];
        f.reregistered = true;
        reviews.push(ReviewItem {
            kind: ReviewKind::PossibleReregistration,
            school: Some(Ref::Existing(f.school.id)),
            other_school: Some(me),
            municipality: Some(cand.municipality),
            details: vec![
                ("old_orgnr", f.school.orgnr.clone().unwrap_or_default()),
                ("new_orgnr", cand.unit.orgnr.clone()),
                ("old_name", f.school.display_name.clone()),
                ("new_name", cand.unit.name.clone()),
                ("similarity", format!("{sim:.2}")),
            ],
        });
        return Placement::HeldReregistration;
    }
    if let Some(((i, _), _)) = best_closing {
        let c = &mut closings[i];
        c.successor = Some(me);
        return Placement::Successor(c.school.slug.clone());
    }

    let submitted = best(
        snapshot.schools.iter().filter(|s| {
            s.origin == Origin::Submitted
                && s.status == SchoolStatus::Active
                && matches!(
                    s.verification,
                    Verification::Pending | Verification::Verified
                )
                && in_municipality(s)
        }),
        |s| similarity(&s.display_name, name),
        SUBMISSION_MATCH_THRESHOLD,
    );
    if let Some((p, sim)) = submitted {
        reviews.push(ReviewItem {
            kind: ReviewKind::PossibleSubmissionMatch,
            school: Some(Ref::Existing(p.id)),
            other_school: Some(me),
            municipality: Some(cand.municipality),
            details: vec![
                ("orgnr", cand.unit.orgnr.clone()),
                ("register_name", cand.unit.name.clone()),
                ("submitted_name", p.display_name.clone()),
                ("similarity", format!("{sim:.2}")),
            ],
        });
        return Placement::HeldSubmission;
    }
    Placement::Listed
}

/// The most similar item at or above `threshold`; the first one on a tie.
fn best<T>(
    items: impl Iterator<Item = T>,
    score: impl Fn(&T) -> f32,
    threshold: f32,
) -> Option<(T, f32)> {
    let mut found: Option<(T, f32)> = None;
    for item in items {
        let s = score(&item);
        if s >= threshold && found.as_ref().is_none_or(|(_, best)| s > *best) {
            found = Some((item, s));
        }
    }
    found
}

/// A continuing school: rename, new attributes, and a move between municipality entities.
fn plan_update<Id: RowId>(
    school: &SchoolSnapshot<Id>,
    unit: &NsrUnit,
    to: Ref<Id>,
    book: &mut SlugBook<Id>,
    ops: &mut Vec<SchoolOp<Id>>,
) {
    let me = Ref::Existing(school.id);
    let here = Ref::Existing(school.municipality_id);
    let moving = to != here;
    let attributes = SchoolAttributes::from_unit(unit);
    let post_town = attributes.post_town.as_deref();

    let renamed = school.register_name.as_deref() != Some(unit.name.as_str());
    let display_name = if renamed && !school.display_name_curated {
        unit.name.as_str()
    } else {
        school.display_name.as_str()
    };
    if renamed {
        // A move re-mints the slug in the target municipality instead.
        let slug = match &school.slug {
            Some(old) if !moving && !school.display_name_curated => slugify(&unit.name)
                .ok()
                .filter(|base| base != old)
                .map(|base| book.mint(here, &base, post_town, me))
                .filter(|new| new != old)
                .map(|new| SlugChange {
                    old: old.clone(),
                    new,
                }),
            _ => None,
        };
        ops.push(SchoolOp::Rename {
            id: school.id,
            register_name: unit.name.clone(),
            display_name: (!school.display_name_curated).then(|| unit.name.clone()),
            slug,
        });
    }
    if attributes != school.attributes {
        ops.push(SchoolOp::UpdateAttributes {
            id: school.id,
            attributes: attributes.clone(),
        });
    }
    if moving {
        // Always a change when the school has a slug: the old one goes to history under the
        // old municipality even if the text is the same.
        let slug = school.slug.as_ref().map(|old| {
            let base = slugify(display_name).unwrap_or_else(|_| old.clone());
            SlugChange {
                old: old.clone(),
                new: book.mint(to, &base, post_town, me),
            }
        });
        ops.push(SchoolOp::Move {
            id: school.id,
            from: school.municipality_id,
            to,
            slug,
        });
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cd /workspace/backend && cargo test -p fau-domain register::sync`
Expected: 46 tests pass (7 new). Then run the full test command, fmt and clippy, all clean.

- [ ] **Step 5: Commit**

```bash
cd /workspace/backend
git add crates/domain/src/register/sync/schools.rs
GIT_AUTHOR_NAME="Erik W. Bjønnes" GIT_AUTHOR_EMAIL=erik@ewb-solutions.as \
GIT_COMMITTER_NAME="Erik W. Bjønnes" GIT_COMMITTER_EMAIL=erik@ewb-solutions.as \
git commit -m "Hold re-registrations and submission matches for review (#3441)" \
  -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: `plan()`, the circuit breaker and idempotency

**Files:**
- Modify: `backend/crates/domain/src/register/sync/mod.rs`
- Modify: `backend/crates/domain/src/register/sync/testkit.rs`

**Interfaces:**
- Consumes `plan_municipalities` (Task 2) and `plan_schools` (Tasks 3–4).
- Produces the public entry point
  `pub fn plan<Id: RowId>(snapshot: &RegisterSnapshot<Id>, inputs: &SyncInputs<'_>) -> SyncOutcome<Id>`.
  - **`Abort { EmptySource }`** when Kartverket's list is empty (checked first), or NSR's is.
  - **`Abort { MassChange }`**, for a Sync only, when `closes * 100 > active * 2` or
    `renames * 100 > active * 5`. Here `active` counts Active schools that are Listed or Verified.
  - **`NoChange`** when there are no ops and no reviews.
  - **`Apply`** otherwise.
- Adds the test-only `testkit::apply(&RegisterSnapshot<u32>, &SyncPlan<u32>) -> RegisterSnapshot<u32>`.

- [ ] **Step 1: The test applier.** In `backend/crates/domain/src/register/sync/testkit.rs`, replace
  the line `use jiff::civil::Date;` with

```rust
use std::collections::{HashMap, HashSet};

use jiff::civil::Date;
```

then replace the `use super::types::{ ... };` block with

```rust
use super::types::{
    CreateVerification, MunicipalityOp, MunicipalitySnapshot, MunicipalitySource,
    MunicipalityStatus, Origin, Ref, RegisterSnapshot, RunKind, SchoolAttributes, SchoolOp,
    SchoolSnapshot, SchoolStatus, SlugChange, SyncInputs, SyncPlan, Verification,
};
```

and append to the end of the file:

```rust
/// Applies a plan to a snapshot as the persistence applier (part 4) must: every `Ref::New`
/// gets its id before any op runs, so a `Close` can name a successor created after it; then
/// the ops run in order. A close moves the slug to history and clears it. The executable
/// specification the SQL applier has to match.
pub(super) fn apply(
    snapshot: &RegisterSnapshot<u32>,
    plan: &SyncPlan<u32>,
) -> RegisterSnapshot<u32> {
    let mut next = snapshot
        .municipalities
        .iter()
        .map(|m| m.id)
        .chain(snapshot.schools.iter().map(|s| s.id))
        .max()
        .map_or(1, |id| id + 1);
    let mut fresh = || {
        next += 1;
        next - 1
    };
    let new_municipalities: HashMap<u32, u32> = plan
        .municipality_ops
        .iter()
        .filter_map(|op| match op {
            MunicipalityOp::Create { new, .. } => Some((*new, fresh())),
            _ => None,
        })
        .collect();
    let new_schools: HashMap<u32, u32> = plan
        .school_ops
        .iter()
        .filter_map(|op| match op {
            SchoolOp::Create { new, .. } => Some((*new, fresh())),
            _ => None,
        })
        .collect();
    let municipality_id = |r: &Ref<u32>| match r {
        Ref::Existing(id) => *id,
        Ref::New(n) => new_municipalities[n],
    };

    let mut out = snapshot.clone();
    for op in &plan.municipality_ops {
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
            } => out.municipalities.push(MunicipalitySnapshot {
                id: new_municipalities[new],
                number: number.clone(),
                name: name.clone(),
                official_name: official_name.clone(),
                county_number: county_number.clone(),
                county_name: county_name.clone(),
                slug: slug.clone(),
                status: MunicipalityStatus::Active,
                source: *source,
                names: names.clone(),
            }),
            MunicipalityOp::Renumber {
                id,
                to,
                old_slug,
                new_slug,
                ..
            } => {
                let m = municipality_mut(&mut out, *id);
                m.number = to.clone();
                m.slug = new_slug.clone();
                out.municipality_slug_history.push((old_slug.clone(), *id));
            }
            MunicipalityOp::Rename {
                id,
                name,
                old_slug,
                new_slug,
            } => {
                let m = municipality_mut(&mut out, *id);
                m.name = name.clone();
                if old_slug != new_slug {
                    m.slug = new_slug.clone();
                    out.municipality_slug_history.push((old_slug.clone(), *id));
                }
            }
            MunicipalityOp::UpdateDetails {
                id,
                official_name,
                county_number,
                county_name,
                names,
            } => {
                let m = municipality_mut(&mut out, *id);
                m.official_name = official_name.clone();
                m.county_number = county_number.clone();
                m.county_name = county_name.clone();
                m.names = names.clone();
            }
        }
    }
    for op in &plan.school_ops {
        match op {
            SchoolOp::Close { id, .. } => {
                let s = school_mut(&mut out, *id);
                s.status = SchoolStatus::Closed;
                let released = s.slug.take().map(|slug| (s.municipality_id, slug, *id));
                out.school_slug_history.extend(released);
            }
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
                ..
            } => out.schools.push(SchoolSnapshot {
                id: new_schools[new],
                municipality_id: municipality_id(municipality),
                origin: Origin::Register,
                orgnr: Some(orgnr.clone()),
                register_name: Some(register_name.clone()),
                display_name: display_name.clone(),
                display_name_curated: false,
                slug: slug.clone(),
                verification: match verification {
                    CreateVerification::Listed => Verification::Listed,
                    CreateVerification::Held => Verification::Held,
                },
                status: SchoolStatus::Active,
                in_scope: *in_scope,
                scope_override: None,
                has_live_fau: false,
                attributes: attributes.clone(),
            }),
            SchoolOp::Rename {
                id,
                register_name,
                display_name,
                slug,
            } => {
                let s = school_mut(&mut out, *id);
                s.register_name = Some(register_name.clone());
                if let Some(name) = display_name {
                    s.display_name = name.clone();
                }
                if let Some(SlugChange { old, new }) = slug {
                    s.slug = Some(new.clone());
                    let m = s.municipality_id;
                    out.school_slug_history.push((m, old.clone(), *id));
                }
            }
            SchoolOp::UpdateAttributes { id, attributes } => {
                school_mut(&mut out, *id).attributes = attributes.clone();
            }
            SchoolOp::Move { id, from, to, slug } => {
                let s = school_mut(&mut out, *id);
                s.municipality_id = municipality_id(to);
                if let Some(SlugChange { old, new }) = slug {
                    s.slug = Some(new.clone());
                    out.school_slug_history.push((*from, old.clone(), *id));
                }
            }
        }
    }

    // The database's own rules: one current slug per municipality, none on a held row.
    let mut seen = HashSet::new();
    for s in &out.schools {
        if let Some(slug) = &s.slug {
            assert!(s.verification != Verification::Held, "held row with a slug");
            assert!(
                seen.insert((s.municipality_id, slug.clone())),
                "duplicate slug {slug}"
            );
        }
    }
    out
}

fn municipality_mut(out: &mut RegisterSnapshot<u32>, id: u32) -> &mut MunicipalitySnapshot<u32> {
    out.municipalities
        .iter_mut()
        .find(|m| m.id == id)
        .expect("the plan names an existing municipality")
}

fn school_mut(out: &mut RegisterSnapshot<u32>, id: u32) -> &mut SchoolSnapshot<u32> {
    out.schools
        .iter_mut()
        .find(|s| s.id == id)
        .expect("the plan names an existing school")
}
```

- [ ] **Step 2: Write the failing tests.** Append this test module to the end of
  `backend/crates/domain/src/register/sync/mod.rs`, after the `pub use` lines:

```rust
#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::testkit::{
        apply, change, fixture_records, fixture_units, hosle, inputs, municipality, record, school,
        stange_new, stange_old, unit,
    };
    use super::*;
    use crate::register::source::{CodeChange, MunicipalityRecord, NsrUnit};

    fn applied(outcome: SyncOutcome<u32>) -> SyncPlan<u32> {
        match outcome {
            SyncOutcome::Apply(plan) => plan,
            other => panic!("expected a plan, got {other:?}"),
        }
    }

    /// `count` unchanged listed schools, "Skole 0" onwards, with ids from 1000. One change
    /// among them stays under the circuit breaker, which a register of one school would trip.
    fn bystanders(
        municipality_id: u32,
        number: &str,
        count: u32,
    ) -> (Vec<SchoolSnapshot<u32>>, Vec<NsrUnit>) {
        let units: Vec<NsrUnit> = (0..count)
            .map(|i| unit(&format!("9{i:08}"), &format!("Skole {i}"), number))
            .collect();
        let schools = units
            .iter()
            .zip(0..)
            .map(|(u, i)| school(1000 + i, municipality_id, u, &format!("skole-{i}")))
            .collect();
        (schools, units)
    }

    /// Plans, applies, and checks that the very same inputs then change nothing.
    fn apply_and_rerun(
        snapshot: &RegisterSnapshot<u32>,
        records: &[MunicipalityRecord],
        changes: &[CodeChange],
        units: &[NsrUnit],
        kind: RunKind,
    ) -> (SyncPlan<u32>, RegisterSnapshot<u32>) {
        let plan = applied(super::plan(
            snapshot,
            &inputs(records, changes, units, kind),
        ));
        let after = apply(snapshot, &plan);
        assert_eq!(
            super::plan(&after, &inputs(records, changes, units, RunKind::Sync)),
            SyncOutcome::NoChange,
            "a second run on the same sources writes nothing"
        );
        (plan, after)
    }

    #[test]
    fn an_empty_source_aborts_every_kind_of_run() {
        let records = [record("3201", "Bærum", "32", "Akershus")];
        let units = [hosle()];
        let empty = RegisterSnapshot::<u32>::default();
        for kind in [RunKind::Seed, RunKind::Sync] {
            assert_eq!(
                super::plan(&empty, &inputs(&[], &[], &units, kind)),
                SyncOutcome::Abort {
                    reason: AbortReason::EmptySource {
                        source: "kartverket"
                    },
                    counts: Counts::default(),
                }
            );
            assert_eq!(
                super::plan(&empty, &inputs(&records, &[], &[], kind)),
                SyncOutcome::Abort {
                    reason: AbortReason::EmptySource { source: "nsr" },
                    counts: Counts::default(),
                }
            );
        }
    }

    #[test]
    fn a_seed_from_the_fixtures_then_a_sync_changes_nothing() {
        let (plan, after) = apply_and_rerun(
            &RegisterSnapshot::default(),
            &fixture_records(),
            &[],
            &fixture_units(),
            RunKind::Seed,
        );
        assert_eq!(
            plan.counts,
            Counts {
                municipalities_created: 10,
                schools_created: 9,
                ..Counts::default()
            }
        );
        assert!(plan.reviews.is_empty(), "{:?}", plan.reviews);

        let svalbard = after
            .municipalities
            .iter()
            .find(|m| m.number == "2100")
            .expect("Svalbard is created");
        assert_eq!(svalbard.source, MunicipalitySource::Manual);
        assert_eq!(svalbard.slug, "2100-svalbard");
        let longyearbyen = after
            .schools
            .iter()
            .find(|s| s.orgnr.as_deref() == Some("974795655"))
            .expect("Longyearbyen skole is created");
        assert_eq!(longyearbyen.municipality_id, svalbard.id);

        let baerum = after
            .municipalities
            .iter()
            .find(|m| m.number == "3201")
            .expect("Bærum is created");
        assert_eq!(baerum.slug, "3201-baerum");
        let hosle_row = after
            .schools
            .iter()
            .find(|s| s.orgnr.as_deref() == Some("974552124"))
            .expect("Hosle skole is created");
        assert_eq!(hosle_row.slug.as_deref(), Some("hosle-skole"));
        assert_eq!(hosle_row.municipality_id, baerum.id);

        assert_eq!(
            super::plan(
                &after,
                &inputs(&fixture_records(), &[], &fixture_units(), RunKind::Seed)
            ),
            SyncOutcome::NoChange,
            "a repeated seed is a no-op too"
        );
    }

    #[test]
    fn a_renumber_moves_the_slug_to_history_and_touches_no_school() {
        let snapshot = RegisterSnapshot {
            municipalities: vec![municipality(1, &record("3024", "Bærum", "30", "Viken"))],
            schools: vec![school(10, 1, &hosle(), "hosle-skole")],
            ..RegisterSnapshot::default()
        };
        let records = [record("3201", "Bærum", "32", "Akershus")];
        let changes = [change("3024", "Bærum", "3201", "Bærum", date(2024, 1, 1))];
        let (plan, after) =
            apply_and_rerun(&snapshot, &records, &changes, &[hosle()], RunKind::Sync);
        assert!(plan.school_ops.is_empty(), "{:?}", plan.school_ops);
        assert_eq!(
            plan.counts,
            Counts {
                renumbered: 1,
                municipalities_updated: 1,
                ..Counts::default()
            }
        );
        assert_eq!(after.municipalities[0].slug, "3201-baerum");
        assert_eq!(
            after.municipality_slug_history,
            [("3024-baerum".to_owned(), 1)]
        );
    }

    #[test]
    fn a_split_counts_its_schools_as_skipped() {
        let snapshot = RegisterSnapshot {
            municipalities: vec![municipality(
                1,
                &record("1507", "Ålesund", "15", "Møre og Romsdal"),
            )],
            schools: vec![
                school(
                    10,
                    1,
                    &unit("974585715", "Brattvåg barneskule", "1507"),
                    "brattvaag-barneskule",
                ),
                school(
                    11,
                    1,
                    &unit("974585723", "Spjelkavik barneskule", "1507"),
                    "spjelkavik-barneskule",
                ),
            ],
            ..RegisterSnapshot::default()
        };
        let records = [
            record("1508", "Ålesund", "15", "Møre og Romsdal"),
            record("1580", "Haram", "15", "Møre og Romsdal"),
        ];
        let changes = [
            change("1507", "Ålesund", "1508", "Ålesund", date(2024, 1, 1)),
            change("1507", "Ålesund", "1580", "Haram", date(2024, 1, 1)),
        ];
        let units = [
            unit("974585715", "Brattvåg barneskule", "1580"),
            unit("974585723", "Spjelkavik barneskule", "1508"),
        ];
        let plan = applied(super::plan(
            &snapshot,
            &inputs(&records, &changes, &units, RunKind::Sync),
        ));
        assert!(plan.municipality_ops.is_empty() && plan.school_ops.is_empty());
        assert_eq!(
            plan.counts,
            Counts {
                reviews: 1,
                skipped: 2,
                ..Counts::default()
            }
        );
        assert_eq!(plan.reviews[0].kind, ReviewKind::MunicipalitySplitOrMerge);
    }

    #[test]
    fn a_rename_writes_slug_history_once() {
        let baerum = record("3201", "Bærum", "32", "Akershus");
        let (mut schools, mut units) = bystanders(1, "3201", 99);
        schools.insert(0, school(10, 1, &hosle(), "hosle-skole"));
        units.push(NsrUnit {
            name: "Hosle barneskole".into(),
            ..hosle()
        });
        let snapshot = RegisterSnapshot {
            municipalities: vec![municipality(1, &baerum)],
            schools,
            ..RegisterSnapshot::default()
        };
        let (plan, after) = apply_and_rerun(&snapshot, &[baerum], &[], &units, RunKind::Sync);
        assert_eq!(plan.counts.schools_renamed, 1);
        assert_eq!(after.schools[0].slug.as_deref(), Some("hosle-barneskole"));
        assert_eq!(
            after.school_slug_history,
            [(1, "hosle-skole".to_owned(), 10)]
        );
    }

    #[test]
    fn a_reregistration_applies_and_then_changes_nothing() {
        let stange = record("3413", "Stange", "34", "Innlandet");
        let (mut schools, mut units) = bystanders(20, "3413", 99);
        schools.insert(0, school(30, 20, &stange_old(), "stange-ungdomsskole"));
        units.extend([stange_old(), stange_new()]);
        let snapshot = RegisterSnapshot {
            municipalities: vec![municipality(20, &stange)],
            schools,
            ..RegisterSnapshot::default()
        };
        let (plan, after) = apply_and_rerun(&snapshot, &[stange], &[], &units, RunKind::Sync);
        assert_eq!(
            plan.counts,
            Counts {
                schools_created: 1,
                schools_closed: 1,
                ..Counts::default()
            }
        );
        let old = &after.schools[0];
        assert_eq!(old.status, SchoolStatus::Closed);
        assert_eq!(old.slug, None);
        let new = after.schools.last().expect("the new school is appended");
        assert_eq!(new.orgnr.as_deref(), Some("933181995"));
        assert_eq!(new.slug.as_deref(), Some("stange-ungdomsskole"));
        assert_eq!(
            after.school_slug_history,
            [(20, "stange-ungdomsskole".to_owned(), 30)]
        );
    }

    /// Bærum with 100 listed schools, "Skole 0" to "Skole 99", each unchanged in NSR.
    fn hundred_schools() -> (RegisterSnapshot<u32>, Vec<NsrUnit>) {
        let (schools, units) = bystanders(1, "3201", 100);
        let snapshot = RegisterSnapshot {
            municipalities: vec![municipality(1, &record("3201", "Bærum", "32", "Akershus"))],
            schools,
            ..RegisterSnapshot::default()
        };
        (snapshot, units)
    }

    fn run_hundred(
        change_first: usize,
        how: impl Fn(NsrUnit) -> NsrUnit,
        kind: RunKind,
    ) -> SyncOutcome<u32> {
        let (snapshot, units) = hundred_schools();
        let units: Vec<NsrUnit> = units
            .into_iter()
            .enumerate()
            .map(|(i, u)| if i < change_first { how(u) } else { u })
            .collect();
        let records = [record("3201", "Bærum", "32", "Akershus")];
        super::plan(&snapshot, &inputs(&records, &[], &units, kind))
    }

    fn inactive(u: NsrUnit) -> NsrUnit {
        NsrUnit {
            is_active: false,
            ..u
        }
    }

    fn renamed(u: NsrUnit) -> NsrUnit {
        NsrUnit {
            name: format!("{} ny", u.name),
            ..u
        }
    }

    #[test]
    fn two_percent_closing_passes_and_three_percent_aborts() {
        let plan = applied(run_hundred(2, inactive, RunKind::Sync));
        assert_eq!(plan.counts.schools_closed, 2);
        match run_hundred(3, inactive, RunKind::Sync) {
            SyncOutcome::Abort { reason, counts } => {
                assert_eq!(
                    reason,
                    AbortReason::MassChange {
                        closes: 3,
                        renames: 0,
                        active: 100
                    }
                );
                assert_eq!(counts.schools_closed, 3);
            }
            other => panic!("expected an abort, got {other:?}"),
        }
    }

    #[test]
    fn five_percent_renamed_passes_and_six_percent_aborts() {
        let plan = applied(run_hundred(5, renamed, RunKind::Sync));
        assert_eq!(plan.counts.schools_renamed, 5);
        assert_eq!(
            run_hundred(6, renamed, RunKind::Sync),
            SyncOutcome::Abort {
                reason: AbortReason::MassChange {
                    closes: 0,
                    renames: 6,
                    active: 100
                },
                counts: Counts {
                    schools_renamed: 6,
                    ..Counts::default()
                },
            }
        );
    }

    #[test]
    fn a_seed_has_no_circuit_breaker() {
        let plan = applied(run_hundred(3, inactive, RunKind::Seed));
        assert_eq!(plan.counts.schools_closed, 3);
    }
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cd /workspace/backend && cargo test -p fau-domain register::sync::tests`
Expected: compile errors: ``cannot find function `plan` in module `super` ``.

- [ ] **Step 4: Implement.** In `mod.rs`, replace everything above the `#[cfg(test)]` line with the
  block below. This also removes both `#[allow(dead_code)]` attributes and their comments:

```rust
//! The register sync planner (#3441, docs/school-register-design.md §2.4, §4.5, §5.2-5.3).
//! Pure: it reads a snapshot of the register and the freshly fetched sources, and returns a
//! plan of what to create, rename, renumber, move and close, plus the review items. No SQL,
//! no HTTP, no mail: the persistence applier and `fau register sync` apply the plan.

mod municipalities;
mod schools;
mod similarity;
#[cfg(test)]
mod testkit;
mod types;

pub use similarity::{similarity, REREGISTRATION_THRESHOLD, SUBMISSION_MATCH_THRESHOLD};
pub use types::*;

use municipalities::plan_municipalities;
use schools::plan_schools;

/// One run's plan (§5.2), or why it must not be applied (§5.3).
pub fn plan<Id: RowId>(
    snapshot: &RegisterSnapshot<Id>,
    inputs: &SyncInputs<'_>,
) -> SyncOutcome<Id> {
    for (empty, source) in [
        (inputs.municipalities.is_empty(), "kartverket"),
        (inputs.units.is_empty(), "nsr"),
    ] {
        if empty {
            return SyncOutcome::Abort {
                reason: AbortReason::EmptySource { source },
                counts: Counts::default(),
            };
        }
    }

    let municipalities = plan_municipalities(snapshot, inputs);
    let schools = plan_schools(snapshot, inputs, &municipalities.state);
    let mut reviews = municipalities.reviews;
    reviews.extend(schools.reviews);
    let counts = tally(
        &municipalities.ops,
        &schools.ops,
        reviews.len(),
        schools.skipped,
    );

    if inputs.kind == RunKind::Sync {
        if let Some(reason) = mass_change(snapshot, &counts) {
            return SyncOutcome::Abort { reason, counts };
        }
    }
    if municipalities.ops.is_empty() && schools.ops.is_empty() && reviews.is_empty() {
        return SyncOutcome::NoChange;
    }
    SyncOutcome::Apply(SyncPlan {
        municipality_ops: municipalities.ops,
        school_ops: schools.ops,
        reviews,
        counts,
    })
}

/// §5.3's circuit breaker: more than 2% of active schools closing, or more than 5% renamed.
/// Integer arithmetic, so 2 of 100 passes and 3 of 100 does not.
fn mass_change<Id>(snapshot: &RegisterSnapshot<Id>, counts: &Counts) -> Option<AbortReason> {
    let active = snapshot
        .schools
        .iter()
        .filter(|s| {
            s.status == SchoolStatus::Active
                && matches!(
                    s.verification,
                    Verification::Listed | Verification::Verified
                )
        })
        .count();
    let (closes, renames) = (counts.schools_closed, counts.schools_renamed);
    let tripped = active > 0 && (closes * 100 > active * 2 || renames * 100 > active * 5);
    tripped.then_some(AbortReason::MassChange {
        closes,
        renames,
        active,
    })
}

fn tally<Id>(
    municipality_ops: &[MunicipalityOp<Id>],
    school_ops: &[SchoolOp<Id>],
    reviews: usize,
    skipped: usize,
) -> Counts {
    let mut c = Counts {
        reviews,
        skipped,
        ..Counts::default()
    };
    for op in municipality_ops {
        match op {
            MunicipalityOp::Create { .. } => c.municipalities_created += 1,
            MunicipalityOp::Renumber { .. } => c.renumbered += 1,
            MunicipalityOp::Rename { .. } => c.renamed += 1,
            MunicipalityOp::UpdateDetails { .. } => c.municipalities_updated += 1,
        }
    }
    for op in school_ops {
        match op {
            SchoolOp::Create {
                verification: CreateVerification::Listed,
                ..
            } => c.schools_created += 1,
            SchoolOp::Create {
                verification: CreateVerification::Held,
                ..
            } => c.schools_held += 1,
            SchoolOp::Rename { .. } => c.schools_renamed += 1,
            SchoolOp::UpdateAttributes { .. } => c.schools_updated += 1,
            SchoolOp::Move { .. } => c.schools_moved += 1,
            SchoolOp::Close { .. } => c.schools_closed += 1,
        }
    }
    c
}
```

- [ ] **Step 5: Run to verify pass**

Run: `cd /workspace/backend && cargo test -p fau-domain register::sync`
Expected: 55 tests pass (9 new). Then run
`grep -rn "allow(dead_code)" crates/domain/src/register/sync/`, which must print nothing. Then run
the full test command, fmt and clippy, all clean.

- [ ] **Step 6: Commit**

```bash
cd /workspace/backend
git add crates/domain/src/register/sync/
GIT_AUTHOR_NAME="Erik W. Bjønnes" GIT_AUTHOR_EMAIL=erik@ewb-solutions.as \
GIT_COMMITTER_NAME="Erik W. Bjønnes" GIT_COMMITTER_EMAIL=erik@ewb-solutions.as \
git commit -m "Wire the sync planner with its circuit breaker, proven idempotent (#3441)" \
  -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Handover to part 4: the applier's contract

`testkit::apply` is the executable form of this contract. The SQL applier must behave the same way.

- **Allocate first, then apply in order.** Give every `Ref::New` a UUIDv7 before running any op,
  with separate counters for municipalities and schools. A `Close` can name a successor that a
  later `Create` makes. Then apply `municipality_ops` in order, followed by `school_ops` in order,
  all in one transaction (§5.3).
- **`Close`** sets the status, `closed_on`, `closure_reason` (`ClosureReason::code`) and
  `successor_id`. It also **moves the current slug into `school_slug_history` and sets `slug`
  null**. Otherwise `schools_slug_current` blocks the successor's hand-over, and the planner treats
  a closed row's slug as free.
- **`Close` with `OutOfScope`** should also set `in_scope = false`. One known gap: an operator
  override `scope_override = true` keeps a school open while its `in_scope` column goes stale. The
  planner does not report it.
- **`Create`:**
  - the row gets origin `register`, the given verification, and `search_text` from
    `search::search_text`;
  - **a Held row gets no FAU link** (ruling, 24 September 2026), so it can be deleted when a
    review resolves.
- **`school_orgnr_history` holds former orgnrs only.** No op writes it. It is written only by
  the "same school" outcome of a re-registration review, which moves the old orgnr there. A
  school's current orgnr lives on `schools.orgnr` and never in history.
- **Renames and renumbers:**
  - **`Rename` and `Move`** write a `school_slug_history` row only when they carry a `SlugChange`
    whose `old` is `Some`. `old: None` is a slugless school getting its first slug: set it, write
    no history. A move's history row is keyed on `from`, the old municipality, even when the text
    is the same;
  - **`MunicipalityOp::Rename`** writes `municipality_slug_history` only when `old_slug != new_slug`;
  - **`Renumber`** closes the old `municipality_numbers` row at `valid_from`, opens a new one from
    `valid_from`, and always writes the slug history. When `name` is `Some`, it also sets the
    Norwegian name: a same-run rename is folded in, `new_slug` is already minted from the new
    number and name, and no separate `Rename` follows for that municipality;
  - **slug-history `valid_from`** is the entity's last history row's `valid_until`, or else its
    `created_at` (a municipality, or a school created Listed) or `verified_at` (a school that got
    its slug on verification). A school that gets its first slug through a `Rename`
    (`SlugChange.old` is `None`) writes no history row. **When the guard fails,
    `valid_from >= valid_until`, skip the history row**: nobody saw that slug live. That happens
    for the intermediate slug of two renumbers in one run (5001→1234→5501), or for two slug changes
    in one transaction timestamp (controller ruling, final re-review).
- **A dissolved municipality** (an operator's decision, never the planner's) must also close its
  current `municipality_numbers` row, or `municipality_numbers_current` keeps the number taken.
- **`UpdateDetails`** replaces the whole of `municipality_names` for that municipality, since
  `fau_register` may delete names (0004).
- **Reviews:**
  - **Deduplicate review items against open ones.** The planner cannot see open items, so
    without this each weekly run repeats every unresolved item. Skip an item when an open item
    already has the same key:
    - `kind`, `school`, `other_school` and `municipality`; plus
    - a discriminator from `details`: `municipality_number` for `unknown_municipality_number`
      (those items have no municipality or school), and `reason` plus `number` or `old_code`
      (whichever is present) for `municipality_split_or_merge`, where several items can share a
      null municipality.

    Every item the planner builds carries those keys, and a test pins it for each reason.
  - **An Apply whose ops are empty and whose reviews all dedupe away** is recorded as
    `no_change`, like a `NoChange` outcome.
  - **Re-raised items.** Some items are raised on every run until someone resolves them:
    - `municipality_split_or_merge` for an absent or unexplained municipality;
    - `closure_with_fau`;
    - `unknown_municipality_number`.

    A school with an FAU that was re-registered raises `possible_reregistration` once. On the next
    run, while its unit stays closed, it raises `closure_with_fau`, because the new unit is now a
    held school rather than a candidate. The reviewer sees both until the first is resolved.
- **Building the snapshot:**
  - `SchoolAttributes` columns map as `ownership`, `grade_from`, `grade_to`, `register_language`
    (from `language`), `website`, `street_address`, `postcode` and `post_town`. The snapshot must
    read them back exactly, including an empty string versus null, or every run reports
    `UpdateAttributes` (and a null read back as `Some("")`, or the reverse, can trip the
    attribute-loss breaker);
  - **use a deterministic `ORDER BY`** (by id) for every snapshot query. The planner breaks its
    own ties by id, but a stable order keeps plans and diffs reproducible;
  - `has_live_fau` is "a tenant with status pending or active references the school";
  - every school's current slug goes in, closed ones included.
- **`NoChange` writes nothing** but the run row (§5.3). So `last_seen_in_source_at` can only be
  refreshed inside an applied run, or dropped.
- **`Abort { MassChange }`** is the `mass_change` alert. Record it in `register_sync_runs.abort_reason`,
  and raise a `mass_change` review item if part 4 wants one in the queue. It trips on more than 2%
  of active schools closing, more than 5% renamed, or (controller ruling) more than 5% losing an
  attribute: `attribute_losses` counts schools whose `UpdateAttributes` turns any field from
  `Some` into `None`, and is in both `Counts` and the abort reason.
- **`Utgaattype` drift.** If NSR stopped sending closure codes, closed units would lose their
  F/S codes and no re-registration would be recognised. The breaker does not see that; the part 2
  parser is meant to, by failing loudly on missing required fields. **Known gap:** `Utgaattype`
  is optional in `crates/register-sources/src/nsr.rs` (`utgaattype: Option<IdDto>`), so a
  missing code is not an error: an inactive unit without one closes as `closed`, with no
  re-registration. Not changed here; a decision for the sources crate.
- **A small register trips the breaker.** With fewer than 50 active schools, any single close
  aborts a sync. Seed each environment fully, since seeds are exempt, before scheduling the
  CronJob.
- **Deduplicate the input units by orgnr.** The caller merges "every active grunnskole" with
  "every orgnr the register holds", and the planner assumes each orgnr appears once.
- **The SSB window is a fixed lookback that starts at the seed date** (mandatory, controller
  ruling), not "since the last run". **A seed run is passed no code changes at all.** A seed given
  an old split in its window would create neither target and block both for good. A run that aborts or fails would otherwise lose changes for good. Re-reading old changes
  is safe: an applied renumber finds no holder of its old code and is skipped, and a split or
  merge whose old code nobody holds and whose targets are all held counts as resolved and blocks
  nothing. The seed's own window is empty (it starts at the seed date), which matters because
  on an empty register every SSB group has no holder.
- **Before the first sync:** backfill `register_name` on every register-origin school, or the
  first sync reports every one of them as renamed and the breaker aborts. **`--seed` must refuse
  a non-empty register** (controller ruling): a seed has no breaker and creates municipalities
  for unknown numbers.
- **The SQL applier must pass the same scenarios as the test applier,** including seed, then a
  rename, then a re-registration, then the same inputs twice (`NoChange`), with the constraints
  checked after each op. Ops come in an order that satisfies 0004's unique indexes at every
  step, which `testkit::apply` asserts.

## Self-Review

- **Spec coverage:**
  - §2.4 renumber, split, merger, boundary adjustment and official-name change: Task 2's tests,
    plus Task 5's end-to-end renumber and split;
  - §2.3 / D2 scope, the combined and special schools, 2599, adult education, VGS-primary, and
    operator overrides both ways: Task 3;
  - D2 Svalbard: Task 2, and Task 5's seed test (Longyearbyen under manual 2100);
  - D3 slugs from the Norwegian name: Task 2's rename and official-name tests;
  - D5 curated display names: Task 3;
  - D8 closures with an FAU are reviews: Tasks 3–4;
  - §4.5 submission holds and re-registrations: Task 4;
  - §5.2 step 5 unknown municipality numbers: Task 3;
  - §5.3 idempotency (`NoChange` on a re-run), the empty-source and mass-change aborts: Task 5;
  - §6 collisions, post town, numbering, and history taken unless its holder is closed: Task 3;
  - the review kind codes against 0004: Task 1.
- **Every scenario the brief requires has a named test:**
  - seed: `a_seed_creates_exactly_the_in_scope_fixture_units` and
    `a_seed_from_the_fixtures_then_a_sync_changes_nothing`;
  - rename: `a_rename_mints_a_new_slug`, `a_case_only_rename_keeps_the_slug`,
    `a_curated_display_name_is_kept` and `a_rename_writes_slug_history_once`;
  - renumber: `a_one_to_one_ssb_change_is_a_renumber` and
    `a_renumber_moves_the_slug_to_history_and_touches_no_school`;
  - split: `a_split_is_reviewed_and_blocks_every_code_in_it` and
    `a_split_counts_its_schools_as_skipped`;
  - boundary: `the_2026_changes_are_a_boundary_adjustment_and_name_changes`;
  - official name: `an_official_name_change_moves_no_slug`;
  - closures: `a_closure_without_an_fau_closes_the_school` and
    `a_closure_with_an_fau_is_reviewed_not_applied`;
  - re-registration: `a_reregistration_without_an_fau_hands_over_the_slug` and
    `a_reregistration_with_an_fau_is_held_for_review`;
  - submission: `a_unit_similar_to_a_submitted_school_is_held`;
  - unknown number: `an_unknown_municipality_number_skips_the_unit_with_one_item_per_number`;
  - breaker: `two_percent_closing_passes_and_three_percent_aborts`,
    `five_percent_renamed_passes_and_six_percent_aborts` and
    `an_empty_source_aborts_every_kind_of_run`;
  - held rows: `held_pending_and_rejected_rows_are_untouched`;
  - closed rows: `a_closed_school_is_never_reopened_or_recreated`;
  - slug collisions: `a_second_school_of_the_same_name_takes_its_post_town` and
    `a_history_slug_is_taken_unless_its_holder_is_closed`;
  - codes: `review_kind_codes_match_0004_exactly`.
- **Placeholders:** none. Every step carries complete code, and every expected value comes from a
  run in the scratch copy (the similarity literals from pg_trgm itself).
- **Type consistency:**
  - `MunicipalityState` (Task 2) is read by `plan_schools` (Task 3);
  - `Closing.successor` (Task 3) is set by `place_all` (Task 4, after fix round 1);
  - the `testkit` builders from Tasks 2–3 are used by Task 5;
  - `SchoolPlan.skipped` feeds `Counts.skipped`;
  - no `#[allow(dead_code)]` survives Task 5.
- **Not in this plan:**
  - the SQL applier, the snapshot query and `fau register sync` (part 4);
  - Brreg matching (part 5);
  - mail.
