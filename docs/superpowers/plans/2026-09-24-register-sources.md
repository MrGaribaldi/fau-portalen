# Register Sources Implementation Plan (#3441, part 2)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the four public sources (NSR, Kartverket, SSB and Brreg) into plain, tested register
values. That means a new crate `fau-register-sources` with HTTP fetching and JSON parsing, the value
types in `fau_domain::register::source` that the sync planner (part 3) consumes, and recorded
fixtures, so no test calls the network.

**Architecture:**
- **Value types** live in the domain, as `fau_domain::register::source`: plain structs with no
  serde and no HTTP. The part 3 sync planner, which is pure domain code, takes them as input.
- **The crate `fau-register-sources`** owns everything source-shaped:
  - private serde DTOs mirroring each API's JSON;
  - conversion into the domain values;
  - a streaming filter over Brreg's 210 MB bulk file;
  - a small `reqwest` client whose base URLs are configurable.
- **Tests** parse the recorded fixtures in `crates/register-sources/tests/fixtures/`, committed
  with this plan and described in its README. The client is tested against a real local HTTP
  server serving those fixtures: real sockets, no mocks.

**Tech Stack:** Rust 1.98.1, reqwest 0.13 (rustls, no default features), serde / serde_json 1,
flate2 1 (pure-Rust backend), jiff 0.2, tokio 1; axum 0.8 as a dev-dependency for the
test server.

**Spec:**
- `docs/school-register-design.md` §2.1 (NSR), §2.2 (Kartverket, SSB), §2.3 (scope), §2.5 (Brreg),
  §5.1–5.2 (what the sync fetches), §9 and §10 (fixtures, no network in CI);
- `docs/planning-decisions.md`, the #3441 sections of 24 September 2026, including "FAU contact
  e-mail: fetched live by outreach, never stored in the product".

## Global Constraints

- **Brreg personal data is never read.** Brreg records carry `c/o <parent>` address lines and
  `epostadresse`, `mobil` and `telefon`. The Brreg DTO must declare **no** field for e-mail,
  phone, mobile, website, `aktivitet` or `vedtektsfestetFormaal`, so serde never materialises them.
  Address lines are kept only in `BrregAddress`, whose `Debug` redacts them. Nothing here logs,
  stores or returns an address in an error.
- **Missing required fields fail loudly** (§5.3), and never become nulls. Required fields:
  - NSR: `Organisasjonsnummer`, `Navn`, `Kommune.Kommunenummer`, `ErAktiv`, `ErSkole`,
    `ErGrunnskole`;
  - Kartverket: `kommunenummer`, `kommunenavn`, `kommunenavnNorsk`, `fylkesnummer`, `fylkesnavn`;
  - SSB: `oldCode`, `newCode`, `changeOccurred`;
  - Brreg: `organisasjonsnummer`, `navn`, `organisasjonsform.kode`.
- **Empty strings are absent.** NSR sends `""` for a missing website or postcode, so it becomes
  `None`. Kartverket pads `gyldigeNavn` with `{navn: null, sprak: null}` entries, which are skipped.
- **Language tags are BCP 47:** Norwegian → `no`, Northern Sami → `se`, Southern Sami → `sma`, Lule
  Sami → `smj`, Kven Finnish → `fkv`. An unknown Kartverket language is an error (a new language
  must be added deliberately). NSR `Maalform` B → `nb`, N → `nn`, anything else → `None`.
- `crates/domain` declares no axum/sqlx/tower/hyper/reqwest (`tests/dependency_boundary.rs`).
- Errors carry fixed English text, a source name and a kind, never a response body or an address.
- No test calls the network. The client's base URLs are parameters; production URLs are constants:
  - `https://data-nsr.udir.no`
  - `https://api.kartverket.no/kommuneinfo/v1`
  - `https://data.ssb.no/api/klass/v1`
  - `https://data.brreg.no/enhetsregisteret/api`
- Timeouts: connect 10 s, request 60 s, Brreg bulk download 600 s. User-Agent `fau-register/<crate version>`.
- **Tests:**
  - `cd /workspace/backend && TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test --workspace`;
  - `cargo fmt --check`;
  - `cargo clippy --workspace --all-targets -- -D warnings`.

  All must be clean after every task, and tests are written first.
- **Commits:**
  - identity via env vars: `GIT_AUTHOR_NAME="Erik W. Bjønnes" GIT_AUTHOR_EMAIL=erik@ewb-solutions.as`,
    same for the committer;
  - the message ends with exactly `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`;
  - branch `school-register-3441`, never pushed.

## Decisions this plan makes

| Question | Decision | Reason |
| --- | --- | --- |
| Where the value types live | `fau_domain::register::source` | The part 3 planner is pure domain code and cannot depend on an HTTP crate. |
| Where the DTOs and fetching live | A new crate, `fau-register-sources` | It keeps reqwest, flate2 and serde_json out of the domain and persistence crates. The app's CLI (part 3) depends on it. |
| NSR closure date | `NsrClosure { code, at: Option<Timestamp> }` | `UtgaattDato` is an instant. Converting to a local date for `schools.closed_on` belongs to the planner, which already works in Europe/Oslo. |
| How the Brreg bulk file is parsed | Download it to a caller-given file, then stream it: `flate2::read::GzDecoder` feeds a serde `SeqAccess` visitor that turns one element at a time into a DTO and drops it | The file is 1.18 million units. Collecting them would need gigabytes, and streaming keeps memory flat. |
| NSR list paging | Pages of 1,000 until a page is empty, capped at `AntallSider` from the first page. Hard limit of 100 pages, which is an error | 19 pages today. An upstream bug must not loop forever. |
| `Debug` on `NsrUnit` | Derived | NSR addresses are the institution's, which is public data (§8). |

---

## File Structure

```text
backend/
  Cargo.toml                                     + member crates/register-sources; flate2, reqwest in [workspace.dependencies]
  crates/domain/src/register/mod.rs              + pub mod source;
  crates/domain/src/register/source.rs           new: MunicipalityRecord, OfficialName, CodeChange, NsrUnit, NsrAddress, NsrClosure, BrregFau, BrregAddress
  crates/register-sources/Cargo.toml             new
  crates/register-sources/src/lib.rs             new: module list, re-exports
  crates/register-sources/src/error.rs           new: SourceError, SourceErrorKind, Source
  crates/register-sources/src/nsr.rs             new: DTOs, parse_list_page, parse_unit
  crates/register-sources/src/kartverket.rs      new: DTOs, parse_municipalities
  crates/register-sources/src/ssb.rs             new: DTOs, parse_changes
  crates/register-sources/src/brreg.rs           new: DTO, for_each_fau (streaming)
  crates/register-sources/src/client.rs          new: SourceUrls, SourceClient
  crates/register-sources/tests/fixtures/...     committed with this plan (see its README)
  crates/register-sources/tests/parse.rs         new: fixture parsing tests (Tasks 2-4)
  crates/register-sources/tests/client.rs        new: local-server client tests (Task 5)
```

---

### Task 1: Domain: source value types

**Files:**
- Create: `backend/crates/domain/src/register/source.rs`
- Modify: `backend/crates/domain/src/register/mod.rs` (`pub mod source;`, alphabetical)

**Interfaces:**
- Consumes: `register::scope::{primary_nace, NsrScopeFacts}` (existing).
- Produces the types below. Later tasks construct them. Part 3 consumes them.

- [ ] **Step 1: Write the failing tests** at the bottom of `source.rs`, after the types are declared
  in Step 3. Write the test module first, then run to see it fail on the missing items.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::register::scope::{classify, OutOfScopeReason, ScopeDecision};

    fn unit() -> NsrUnit {
        NsrUnit {
            orgnr: "998516897".into(),
            name: "Lerberg skole og kompetansesenter".into(),
            municipality_number: "3314".into(),
            is_school: true,
            is_active: true,
            is_primary_school: true,
            is_private: false,
            category_ids: vec!["1".into(), "2".into(), "3".into(), "6".into(), "32".into()],
            nace: vec![(2, "85.310".into()), (1, "85.201".into())],
            grade_from: Some(8),
            grade_to: Some(10),
            language: Some("nb".into()),
            website: None,
            visiting: NsrAddress::default(),
            postal: NsrAddress::default(),
            closure: None,
            changed_at: None,
        }
    }

    #[test]
    fn scope_facts_take_the_priority_one_nace_code() {
        let facts = unit().scope_facts();
        assert_eq!(facts.primary_nace.as_deref(), Some("85.201"));
        assert_eq!(facts.municipality_number, "3314");
        assert_eq!(classify(&facts), ScopeDecision::InScope, "a combined school is in scope");
        let vgs = NsrUnit { nace: vec![(1, "85.320".into())], ..unit() };
        assert_eq!(classify(&vgs.scope_facts()), ScopeDecision::OutOfScope(OutOfScopeReason::UpperSecondary));
    }

    #[test]
    fn brreg_addresses_never_reach_debug_output() {
        let fau = BrregFau {
            orgnr: "913591100".into(),
            registered_name: "FAU STORØYA SKOLE".into(),
            organisation_form: "FLI".into(),
            municipality_number: Some("3201".into()),
            business_address: BrregAddress { lines: vec!["c/o Kari Nordmann".into(), "Oppdiktet vei 1".into()], postcode: Some("1364".into()) },
            postal_address: BrregAddress::default(),
        };
        let shown = format!("{fau:?}");
        assert!(!shown.contains("Nordmann") && !shown.contains("Oppdiktet"), "{shown}");
        assert!(shown.contains("913591100") && shown.contains("1364"), "orgnr and postcode are not personal: {shown}");
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd /workspace/backend && cargo test -p fau-domain register::source`
Expected: compile errors, because the types are undefined.

- [ ] **Step 3: Implement** (above the tests):

```rust
//! Source records as the register's rules see them (docs/school-register-design.md §2). The
//! crate `fau-register-sources` fetches and parses the APIs, and hands over these plain values.
//! Brreg addresses exist only inside [`BrregAddress`], in memory, and its `Debug` never
//! prints them (planning-decisions, 24 September 2026).

use jiff::civil::Date;
use jiff::Timestamp;

use super::scope::{primary_nace, NsrScopeFacts};

/// One official name of a municipality, in one language (Kartverket `gyldigeNavn`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficialName {
    pub name: String,
    /// BCP 47: `no`, `se`, `sma`, `smj`, `fkv`.
    pub language: String,
    pub priority: u8,
}

/// A current municipality from Kartverket (§2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MunicipalityRecord {
    pub number: String,
    /// `kommunenavnNorsk`, which the slug follows (D3).
    pub norwegian_name: String,
    /// `kommunenavn`, the priority-1 official name.
    pub official_name: String,
    pub county_number: String,
    pub county_name: String,
    pub names: Vec<OfficialName>,
}

/// One SSB Klass 131 code change (§2.2): a renumber, a rename, a split, a merger or a
/// boundary adjustment. The planner tells them apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeChange {
    pub old_code: String,
    pub old_name: String,
    pub new_code: String,
    pub new_name: String,
    pub occurred_on: Date,
}

/// An institution's address in NSR. Public data, not personal (§8).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NsrAddress {
    pub street: Option<String>,
    pub postcode: Option<String>,
    pub post_town: Option<String>,
}

/// NSR `Utgaattype` (any code but `A`) and `UtgaattDato`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NsrClosure {
    /// D, F, N, O, S or U (§2.1).
    pub code: String,
    pub at: Option<Timestamp>,
}

/// One NSR unit, from the detail endpoint (§2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NsrUnit {
    pub orgnr: String,
    pub name: String,
    pub municipality_number: String,
    pub is_school: bool,
    pub is_active: bool,
    pub is_primary_school: bool,
    pub is_private: bool,
    pub category_ids: Vec<String>,
    /// `(Prioritet, Kode)` pairs, in NSR's order.
    pub nace: Vec<(i64, String)>,
    pub grade_from: Option<i16>,
    pub grade_to: Option<i16>,
    /// BCP 47 from `Maalform`: `nb` or `nn`.
    pub language: Option<String>,
    pub website: Option<String>,
    pub visiting: NsrAddress,
    pub postal: NsrAddress,
    pub closure: Option<NsrClosure>,
    pub changed_at: Option<Timestamp>,
}

impl NsrUnit {
    /// The facts the scope filter reads (§2.3).
    pub fn scope_facts(&self) -> NsrScopeFacts {
        NsrScopeFacts {
            is_school: self.is_school,
            is_active: self.is_active,
            is_primary_school: self.is_primary_school,
            municipality_number: self.municipality_number.clone(),
            category_ids: self.category_ids.clone(),
            primary_nace: primary_nace(self.nace.iter().map(|(p, c)| (*p, c.as_str()))).map(str::to_owned),
        }
    }
}

/// A Brreg address. The lines are often a parent's home address, so they stay in memory and
/// `Debug` prints only how many there are.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct BrregAddress {
    pub lines: Vec<String>,
    pub postcode: Option<String>,
}

impl std::fmt::Debug for BrregAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrregAddress")
            .field("lines", &format_args!("<{} redacted>", self.lines.len()))
            .field("postcode", &self.postcode)
            .finish()
    }
}

/// A Brreg entity that looks like an FAU (§2.5). There are no e-mail or phone fields, on
/// purpose (Erik, 24 September 2026).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrregFau {
    pub orgnr: String,
    pub registered_name: String,
    pub organisation_form: String,
    pub municipality_number: Option<String>,
    pub business_address: BrregAddress,
    pub postal_address: BrregAddress,
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cd /workspace/backend && cargo test -p fau-domain`, then fmt and clippy.
Expected: PASS.

- [ ] **Step 5: Commit**: `git commit -m "Add register source value types to the domain (#3441)"`

---

### Task 2: The `fau-register-sources` crate and NSR parsing

**Files:**
- Modify: `backend/Cargo.toml`. Add `"crates/register-sources"` to `members`. In `[workspace.dependencies]` add:
  - `flate2 = "1"`
  - `reqwest = { version = "0.13", default-features = false, features = ["rustls"] }`
- Create: `backend/crates/register-sources/Cargo.toml`, `src/lib.rs`, `src/error.rs`, `src/nsr.rs`, `tests/parse.rs`

**Interfaces:**
- Consumes: `fau_domain::register::source::{NsrUnit, NsrAddress, NsrClosure}` (Task 1).
- Produces:
  - `fau_register_sources::nsr::{parse_list_page, parse_unit, NsrListPage, NsrListItem}`;
  - `fau_register_sources::{SourceError, SourceErrorKind, Source}`.

  `parse_list_page(&[u8]) -> Result<NsrListPage, SourceError>` returns an `NsrListPage` with
  `page: u32`, `page_count: u32`, `total: u32` and `units: Vec<NsrListItem>`. `NsrListItem` has
  `orgnr`, `name`, `municipality_number`, `is_active`, `is_school`, `is_primary_school` and
  `changed_at: Option<Timestamp>`. `parse_unit(&[u8]) -> Result<NsrUnit, SourceError>`.

- [ ] **Step 1: Crate files**

`backend/crates/register-sources/Cargo.toml`:

```toml
# Fetching and parsing the school register's public sources (#3441): NSR, Kartverket, SSB and
# Brreg. Source-shaped code lives here so the domain stays free of HTTP and JSON.
[package]
name = "fau-register-sources"
version.workspace = true
edition.workspace = true
license.workspace = true
publish = false

[dependencies]
fau-domain = { path = "../domain" }
flate2 = { workspace = true }
jiff = { workspace = true }
reqwest = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true }

[dev-dependencies]
axum = { workspace = true }
```

`src/lib.rs`:

```rust
//! The school register's public sources (docs/school-register-design.md §2): fetching, and
//! turning each API's JSON into `fau_domain::register::source` values. No test calls the
//! network; `tests/fixtures/` holds recorded responses.

pub mod brreg;
pub mod client;
mod error;
pub mod kartverket;
pub mod nsr;
pub mod ssb;

pub use error::{Source, SourceError, SourceErrorKind};
```

For this task, create `brreg.rs`, `client.rs`, `kartverket.rs` and `ssb.rs` each with only a
one-line `//!` doc comment. Tasks 3–5 fill them.

`src/error.rs`:

```rust
//! Errors carry the source and a fixed kind, never a response body or an address.

/// Which public source an error came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Nsr,
    Kartverket,
    Ssb,
    Brreg,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceErrorKind {
    /// The response did not match the expected shape, e.g. a required field was missing.
    /// `detail` names the missing field or the error class, plus a position, never a value.
    Parse { detail: String },
    /// A value was present but not one we accept, e.g. an unknown Kartverket language.
    UnexpectedValue { field: &'static str },
    /// A non-success HTTP status.
    Status { status: u16 },
    /// Transport failure or timeout.
    Transport,
    /// NSR paging did not end within the hard limit.
    TooManyPages,
    /// Reading or writing the local Brreg download failed.
    Io,
}

/// Display and Error are implemented by hand: thiserror would treat a field named `source`
/// as the error's cause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceError {
    pub source: Source,
    pub kind: SourceErrorKind,
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {:?}", self.source, self.kind)
    }
}

impl std::error::Error for SourceError {}

impl SourceError {
    pub(crate) fn new(source: Source, kind: SourceErrorKind) -> Self {
        Self { source, kind }
    }

    /// Never serde_json's full message: an invalid-type error quotes the input's value. Keep
    /// the missing field's name, or else the error class, plus the position.
    pub(crate) fn parse(source: Source, err: &serde_json::Error) -> Self {
        let msg = err.to_string();
        let missing = msg.strip_prefix("missing field `").and_then(|rest| rest.split('`').next());
        let detail = match missing {
            Some(field) => format!("missing field {field} at line {} column {}", err.line(), err.column()),
            None => format!("{:?} error at line {} column {}", err.classify(), err.line(), err.column()),
        };
        Self::new(source, SourceErrorKind::Parse { detail })
    }
}
```

Remove `thiserror` from this crate's `[dependencies]`, since nothing uses it. A test in Step 2
pins that a mistyped value's text never appears in the error.

- [ ] **Step 2: Write the failing tests**

`backend/crates/register-sources/tests/parse.rs` (Tasks 3 and 4 append to it):

```rust
//! Parsing the recorded fixtures (tests/fixtures/README.md). These are real API responses,
//! and the expected values below are taken from them.

use fau_domain::register::scope::{classify, OutOfScopeReason, ScopeDecision};
use fau_register_sources::{nsr, Source, SourceErrorKind};

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{path}", env!("CARGO_MANIFEST_DIR")))
        .unwrap_or_else(|e| panic!("fixture {path}: {e}"))
}

fn unit(orgnr: &str) -> fau_domain::register::source::NsrUnit {
    nsr::parse_unit(&fixture(&format!("nsr/enhet-{orgnr}.json"))).unwrap()
}

#[test]
fn nsr_list_page_carries_paging_and_slim_units() {
    let page = nsr::parse_list_page(&fixture("nsr/list-page-1-of-5.json")).unwrap();
    assert_eq!((page.page, page.page_count, page.total), (1, 3670, 18350));
    let orgnrs: Vec<_> = page.units.iter().map(|u| u.orgnr.as_str()).collect();
    assert_eq!(orgnrs, ["U99999999", "U90099999", "U90099021", "U90099020", "U90099018"]);
    assert_eq!(page.units[0].municipality_number, "2599");
    assert!(!page.units[0].is_active);
}

#[test]
fn nsr_municipality_list_for_baerum() {
    let page = nsr::parse_list_page(&fixture("nsr/kommune-3201.json")).unwrap();
    assert_eq!(page.units.len(), 218);
    assert_eq!(page.units.iter().filter(|u| u.is_active && u.is_primary_school).count(), 45);
}

#[test]
fn hosle_skole_in_full() {
    let u = unit("974552124");
    assert_eq!(u.name, "Hosle skole");
    assert_eq!(u.municipality_number, "3201");
    assert!(u.is_school && u.is_active && u.is_primary_school && !u.is_private);
    assert_eq!(u.category_ids, ["1", "3", "5", "32"]);
    assert_eq!(u.nace, [(1, "85.201".to_owned())]);
    assert_eq!((u.grade_from, u.grade_to), (Some(1), Some(7)));
    assert_eq!(u.language.as_deref(), Some("nb"));
    assert_eq!(u.website.as_deref(), Some("www.hosle.no"));
    assert_eq!(u.visiting.street.as_deref(), Some("Bispeveien 73"));
    assert_eq!(u.visiting.postcode.as_deref(), Some("1362"));
    assert_eq!(u.visiting.post_town.as_deref(), Some("HOSLE"));
    assert_eq!(u.closure, None, "Utgaattype A means not closed");
    assert_eq!(u.changed_at.unwrap().to_string(), "2026-09-13T01:05:43.46Z");
    assert_eq!(classify(&u.scope_facts()), ScopeDecision::InScope);
}

#[test]
fn every_fixture_unit_classifies_as_section_2_3_says() {
    use OutOfScopeReason::*;
    let cases = [
        ("974552124", ScopeDecision::InScope),                    // ordinary
        ("990672938", ScopeDecision::InScope),                    // private
        ("998516897", ScopeDecision::InScope),                    // combined
        ("998666783", ScopeDecision::InScope),                    // special, 85.202
        ("974795655", ScopeDecision::InScope),                    // Svalbard, 2100
        ("998245508", ScopeDecision::InScope),                    // Nynorsk
        ("998670799", ScopeDecision::InScope),                    // no website
        ("974554682", ScopeDecision::InScope),                    // municipality prefix
        ("933181995", ScopeDecision::InScope),                    // Stange, new number
        ("975270920", ScopeDecision::OutOfScope(Inactive)),       // Stange, old number
        ("999038182", ScopeDecision::OutOfScope(AdultEducation)),
        ("986779795", ScopeDecision::OutOfScope(UpperSecondary)),
        ("U90099017", ScopeDecision::OutOfScope(Abroad)),
    ];
    for (orgnr, expected) in cases {
        assert_eq!(classify(&unit(orgnr).scope_facts()), expected, "{orgnr}");
    }
}

#[test]
fn nsr_empty_strings_and_codes_become_absent_or_mapped() {
    assert_eq!(unit("998516897").website, None, "Lerberg sends an empty website");
    assert_eq!(unit("998670799").website, None, "Halsa sends none");
    assert_eq!(unit("998245508").language.as_deref(), Some("nn"));
    assert_eq!(unit("U90099017").visiting.postcode, None, "abroad: empty postcode");
}

#[test]
fn a_closed_unit_carries_its_reason_and_time() {
    let old = unit("975270920");
    assert!(!old.is_active);
    let closure = old.closure.expect("Slettet for sammenslåing");
    assert_eq!(closure.code, "F");
    assert_eq!(closure.at.unwrap().to_string(), "2024-08-25T01:15:10.91Z");
}

#[test]
fn a_missing_required_field_fails_loudly() {
    let mut v: serde_json::Value = serde_json::from_slice(&fixture("nsr/enhet-974552124.json")).unwrap();
    v.as_object_mut().unwrap().remove("Navn");
    let err = nsr::parse_unit(&serde_json::to_vec(&v).unwrap()).unwrap_err();
    assert_eq!(err.source, Source::Nsr);
    match err.kind {
        SourceErrorKind::Parse { detail } => assert!(detail.contains("Navn"), "{detail}"),
        other => panic!("expected a parse error, got {other:?}"),
    }
}

#[test]
fn a_parse_error_never_quotes_the_input() {
    let mut v: serde_json::Value = serde_json::from_slice(&fixture("nsr/enhet-974552124.json")).unwrap();
    v["ErAktiv"] = serde_json::Value::String("SECRET-LOOKING-VALUE".into());
    let err = nsr::parse_unit(&serde_json::to_vec(&v).unwrap()).unwrap_err();
    assert!(!format!("{err:?} {err}").contains("SECRET-LOOKING-VALUE"));
}
```

Add serde_json to `[dev-dependencies]` too (`serde_json = { workspace = true }` is already a
normal dependency, so tests can use it without change).

- [ ] **Step 3: Run to verify failure**

Run: `cd /workspace/backend && cargo test -p fau-register-sources --test parse`
Expected: compile errors, because `nsr::parse_list_page` and `nsr::parse_unit` are undefined.

- [ ] **Step 4: Implement `src/nsr.rs`**

```rust
//! NSR v4 (docs/school-register-design.md §2.1): the paged list and the unit detail.

use fau_domain::register::source::{NsrAddress, NsrClosure, NsrUnit};
use jiff::Timestamp;
use serde::Deserialize;

use crate::error::{Source, SourceError, SourceErrorKind};

/// One page of `/v4/enheter` or `/v4/enheter/kommune/{nr}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NsrListPage {
    pub page: u32,
    pub page_count: u32,
    pub total: u32,
    pub units: Vec<NsrListItem>,
}

/// The slim list model: enough to decide which details to fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NsrListItem {
    pub orgnr: String,
    pub name: String,
    pub municipality_number: String,
    pub is_active: bool,
    pub is_school: bool,
    pub is_primary_school: bool,
    pub changed_at: Option<Timestamp>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ListDto {
    #[serde(default)]
    sidenummer: Option<u32>,
    #[serde(default)]
    antall_sider: Option<u32>,
    #[serde(default)]
    totalt_antall_enheter: Option<u32>,
    enhet_liste: Vec<ListItemDto>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ListItemDto {
    organisasjonsnummer: String,
    navn: String,
    kommunenummer: String,
    er_aktiv: bool,
    er_skole: bool,
    er_grunnskole: bool,
    dato_endret: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UnitDto {
    organisasjonsnummer: String,
    navn: String,
    kommune: KommuneDto,
    er_aktiv: bool,
    er_skole: bool,
    er_grunnskole: bool,
    #[serde(default)]
    er_privatskole: bool,
    #[serde(default)]
    skolekategorier: Vec<IdDto>,
    #[serde(default)]
    naeringskoder: Vec<NaceDto>,
    #[serde(rename = "SkoletrinnGSFra")]
    skoletrinn_gs_fra: Option<i16>,
    #[serde(rename = "SkoletrinnGSTil")]
    skoletrinn_gs_til: Option<i16>,
    maalform: Option<IdDto>,
    internettadresse: Option<String>,
    beliggenhetsadresse: Option<AddressDto>,
    postadresse: Option<AddressDto>,
    utgaattype: Option<IdDto>,
    utgaatt_dato: Option<String>,
    dato_endret: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct KommuneDto {
    kommunenummer: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct IdDto {
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct NaceDto {
    prioritet: i64,
    kode: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AddressDto {
    adresse: Option<String>,
    postnummer: Option<String>,
    poststed: Option<String>,
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
}

fn timestamp(s: Option<String>, field: &'static str) -> Result<Option<Timestamp>, SourceError> {
    match non_empty(s) {
        None => Ok(None),
        Some(s) => s
            .parse::<Timestamp>()
            .map(Some)
            .map_err(|_| SourceError::new(Source::Nsr, SourceErrorKind::UnexpectedValue { field })),
    }
}

fn address(a: Option<AddressDto>) -> NsrAddress {
    match a {
        None => NsrAddress::default(),
        Some(a) => NsrAddress {
            street: non_empty(a.adresse),
            postcode: non_empty(a.postnummer),
            post_town: non_empty(a.poststed),
        },
    }
}

pub fn parse_list_page(bytes: &[u8]) -> Result<NsrListPage, SourceError> {
    let dto: ListDto = serde_json::from_slice(bytes).map_err(|e| SourceError::parse(Source::Nsr, &e))?;
    let units = dto
        .enhet_liste
        .into_iter()
        .map(|u| {
            Ok(NsrListItem {
                orgnr: u.organisasjonsnummer,
                name: u.navn,
                municipality_number: u.kommunenummer,
                is_active: u.er_aktiv,
                is_school: u.er_skole,
                is_primary_school: u.er_grunnskole,
                changed_at: timestamp(u.dato_endret, "DatoEndret")?,
            })
        })
        .collect::<Result<Vec<_>, SourceError>>()?;
    let total = dto.totalt_antall_enheter.unwrap_or(units.len() as u32);
    Ok(NsrListPage {
        page: dto.sidenummer.unwrap_or(1),
        page_count: dto.antall_sider.unwrap_or(1),
        total,
        units,
    })
}

pub fn parse_unit(bytes: &[u8]) -> Result<NsrUnit, SourceError> {
    let d: UnitDto = serde_json::from_slice(bytes).map_err(|e| SourceError::parse(Source::Nsr, &e))?;
    let language = match d.maalform.map(|m| m.id).as_deref() {
        Some("B") => Some("nb".to_owned()),
        Some("N") => Some("nn".to_owned()),
        _ => None,
    };
    let closure = match d.utgaattype.map(|t| t.id) {
        Some(code) if code != "A" => Some(NsrClosure { code, at: timestamp(d.utgaatt_dato, "UtgaattDato")? }),
        _ => None,
    };
    Ok(NsrUnit {
        orgnr: d.organisasjonsnummer,
        name: d.navn,
        municipality_number: d.kommune.kommunenummer,
        is_school: d.er_skole,
        is_active: d.er_aktiv,
        is_primary_school: d.er_grunnskole,
        is_private: d.er_privatskole,
        category_ids: d.skolekategorier.into_iter().map(|c| c.id).collect(),
        nace: d.naeringskoder.into_iter().map(|n| (n.prioritet, n.kode)).collect(),
        grade_from: d.skoletrinn_gs_fra,
        grade_to: d.skoletrinn_gs_til,
        language,
        website: non_empty(d.internettadresse),
        visiting: address(d.beliggenhetsadresse),
        postal: address(d.postadresse),
        closure,
        changed_at: timestamp(d.dato_endret, "DatoEndret")?,
    })
}
```

Kommune-list responses (`/v4/enheter/kommune/{nr}`) carry only `EnhetListe`, with no paging
fields. The `#[serde(default)]` options cover that: page 1 of 1, with the unit count as the total.

- [ ] **Step 5: Run to verify pass**

Run: `cd /workspace/backend && cargo test -p fau-register-sources`, then the whole workspace, fmt and clippy.
Expected: PASS. If an expected value in the tests disagrees with a fixture, re-check it against the
fixture file itself (`jq`). The fixtures are the truth, and the plan's values were read from them.

- [ ] **Step 6: Commit**: `git commit -m "Add the register sources crate and NSR parsing (#3441)"`

---

### Task 3: Kartverket and SSB parsing

**Files:**
- Modify: `backend/crates/register-sources/src/kartverket.rs`, `src/ssb.rs`, `tests/parse.rs` (append)

**Interfaces:**
- Produces:
  - `kartverket::parse_municipalities(&[u8]) -> Result<Vec<MunicipalityRecord>, SourceError>`, from `/fylkerkommuner`;
  - `ssb::parse_changes(&[u8]) -> Result<Vec<CodeChange>, SourceError>`.

- [ ] **Step 1: Append the failing tests to `tests/parse.rs`**

```rust
use fau_domain::register::source::OfficialName;
use fau_register_sources::{kartverket, ssb};

fn names(n: &[(&str, &str, u8)]) -> Vec<OfficialName> {
    n.iter().map(|(name, lang, p)| OfficialName { name: (*name).into(), language: (*lang).into(), priority: *p }).collect()
}

#[test]
fn kartverket_lists_every_current_municipality() {
    let all = kartverket::parse_municipalities(&fixture("kartverket/fylkerkommuner.json")).unwrap();
    assert_eq!(all.len(), 357);
    assert!(all.iter().all(|m| m.number.len() == 4 && m.county_number.len() == 2));
    assert!(!all.iter().any(|m| m.number == "2100"), "Svalbard is not a municipality (§2.2)");
}

#[test]
fn kaafjord_has_three_official_names() {
    let all = kartverket::parse_municipalities(&fixture("kartverket/fylkerkommuner.json")).unwrap();
    let k = all.iter().find(|m| m.number == "5540").unwrap();
    assert_eq!(k.norwegian_name, "Kåfjord");
    assert_eq!(k.official_name, "Gáivuotna");
    assert_eq!((k.county_number.as_str(), k.county_name.as_str()), ("55", "Troms"));
    assert_eq!(k.names, names(&[("Gáivuotna", "se", 1), ("Kåfjord", "no", 2), ("Kaivuono", "fkv", 3)]));
}

#[test]
fn kartverket_padding_entries_are_skipped() {
    let all = kartverket::parse_municipalities(&fixture("kartverket/fylkerkommuner.json")).unwrap();
    let oslo = all.iter().find(|m| m.number == "0301").unwrap();
    assert_eq!(oslo.names, names(&[("Oslo", "no", 1)]));
    assert_eq!(oslo.norwegian_name, "Oslo");
}

#[test]
fn every_kartverket_language_is_mapped() {
    let all = kartverket::parse_municipalities(&fixture("kartverket/fylkerkommuner.json")).unwrap();
    let mut langs: Vec<_> = all.iter().flat_map(|m| m.names.iter().map(|n| n.language.as_str())).collect();
    langs.sort();
    langs.dedup();
    assert_eq!(langs, ["fkv", "no", "se", "sma", "smj"]);
}

#[test]
fn an_unknown_kartverket_language_is_an_error() {
    let mut v: serde_json::Value = serde_json::from_slice(&fixture("kartverket/fylkerkommuner.json")).unwrap();
    v[0]["kommuner"][0]["gyldigeNavn"][0]["sprak"] = "Klingon".into();
    let err = kartverket::parse_municipalities(&serde_json::to_vec(&v).unwrap()).unwrap_err();
    assert_eq!(err.kind, SourceErrorKind::UnexpectedValue { field: "sprak" });
}

#[test]
fn ssb_changes_of_2024_include_the_renumbering_and_the_split() {
    let c = ssb::parse_changes(&fixture("ssb/changes-2024.json")).unwrap();
    assert_eq!(c.len(), 118);
    let has = |old: &str, new: &str| c.iter().any(|x| x.old_code == old && x.new_code == new);
    assert!(has("3024", "3201"), "Bærum renumbered");
    assert!(has("1507", "1508") && has("1507", "1580"), "Ålesund split into Ålesund and Haram");
    assert!(c.iter().all(|x| x.occurred_on.to_string() == "2024-01-01"));
    let rana = c.iter().find(|x| x.old_code == "1833").unwrap();
    assert_eq!((rana.new_code.as_str(), rana.new_name.as_str()), ("1833", "Rana - Raane"), "a name-only change");
}

#[test]
fn ssb_changes_of_2026_include_the_boundary_adjustment() {
    let c = ssb::parse_changes(&fixture("ssb/changes-2026.json")).unwrap();
    assert_eq!(c.len(), 6);
    let from_3118: Vec<_> = c.iter().filter(|x| x.old_code == "3118").map(|x| x.new_code.as_str()).collect();
    assert_eq!(from_3118, ["3118", "3207", "3216"], "3118 continues: a boundary adjustment, not a merger");
}
```

- [ ] **Step 2: Run to verify failure**, via `cargo test -p fau-register-sources --test parse`. It fails to compile.

- [ ] **Step 3: Implement**

`src/kartverket.rs`:

```rust
//! Kartverket's Administrative enheter API, `/fylkerkommuner` (§2.2): every current
//! municipality with its county and every official name, in one call.

use fau_domain::register::source::{MunicipalityRecord, OfficialName};
use serde::Deserialize;

use crate::error::{Source, SourceError, SourceErrorKind};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CountyDto {
    fylkesnummer: String,
    fylkesnavn: String,
    kommuner: Vec<MunicipalityDto>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MunicipalityDto {
    kommunenummer: String,
    kommunenavn: String,
    kommunenavn_norsk: String,
    #[serde(default)]
    gyldige_navn: Vec<NameDto>,
}

#[derive(Deserialize)]
struct NameDto {
    navn: Option<String>,
    prioritet: u8,
    sprak: Option<String>,
}

/// Kartverket's language names to BCP 47. A new language must be added here deliberately.
fn language(sprak: &str) -> Option<&'static str> {
    Some(match sprak {
        "Norwegian" => "no",
        "Northern Sami" => "se",
        "Southern Sami" => "sma",
        "Lule Sami" => "smj",
        "Kven Finnish" => "fkv",
        _ => return None,
    })
}

pub fn parse_municipalities(bytes: &[u8]) -> Result<Vec<MunicipalityRecord>, SourceError> {
    let counties: Vec<CountyDto> =
        serde_json::from_slice(bytes).map_err(|e| SourceError::parse(Source::Kartverket, &e))?;
    let mut out = Vec::new();
    for county in counties {
        for m in county.kommuner {
            let mut names = Vec::new();
            for n in m.gyldige_navn {
                // Kartverket pads the list with {navn: null, sprak: null}.
                let (Some(name), Some(sprak)) = (n.navn, n.sprak) else { continue };
                let language = language(&sprak).ok_or_else(|| {
                    SourceError::new(Source::Kartverket, SourceErrorKind::UnexpectedValue { field: "sprak" })
                })?;
                names.push(OfficialName { name, language: language.to_owned(), priority: n.prioritet });
            }
            names.sort_by_key(|n| n.priority);
            out.push(MunicipalityRecord {
                number: m.kommunenummer,
                norwegian_name: m.kommunenavn_norsk,
                official_name: m.kommunenavn,
                county_number: county.fylkesnummer.clone(),
                county_name: county.fylkesnavn.clone(),
                names,
            });
        }
    }
    Ok(out)
}
```

`src/ssb.rs`:

```rust
//! SSB Klass classification 131, `/changes?from=&to=` (§2.2): municipality code changes.

use fau_domain::register::source::CodeChange;
use jiff::civil::Date;
use serde::Deserialize;

use crate::error::{Source, SourceError, SourceErrorKind};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChangesDto {
    #[serde(default)]
    code_changes: Vec<ChangeDto>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChangeDto {
    old_code: String,
    #[serde(default)]
    old_name: String,
    new_code: String,
    #[serde(default)]
    new_name: String,
    change_occurred: String,
}

pub fn parse_changes(bytes: &[u8]) -> Result<Vec<CodeChange>, SourceError> {
    let dto: ChangesDto = serde_json::from_slice(bytes).map_err(|e| SourceError::parse(Source::Ssb, &e))?;
    dto.code_changes
        .into_iter()
        .map(|c| {
            let occurred_on = c.change_occurred.parse::<Date>().map_err(|_| {
                SourceError::new(Source::Ssb, SourceErrorKind::UnexpectedValue { field: "changeOccurred" })
            })?;
            Ok(CodeChange { old_code: c.old_code, old_name: c.old_name, new_code: c.new_code, new_name: c.new_name, occurred_on })
        })
        .collect()
}
```

- [ ] **Step 4: Run to verify pass**, via `cargo test -p fau-register-sources`, then the workspace, fmt and clippy.

- [ ] **Step 5: Commit**: `git commit -m "Parse Kartverket municipalities and SSB code changes (#3441)"`

---

### Task 4: Brreg: stream the bulk file, keep only FAU-er

**Files:**
- Modify: `backend/crates/register-sources/src/brreg.rs`, `tests/parse.rs` (append)

**Interfaces:**
- Consumes: `fau_domain::register::brreg::is_fau_name` (existing) and `source::{BrregFau, BrregAddress}` (Task 1).
- Produces: `brreg::for_each_fau<R: std::io::Read>(gzipped: R, f: impl FnMut(BrregFau)) -> Result<BrregStats, SourceError>`,
  where `BrregStats { pub units_seen: u64, pub faus: u64 }`. Part 3's CLI calls it on the
  downloaded file.

- [ ] **Step 1: Append the failing tests**

```rust
use fau_register_sources::brreg;
use std::io::Write;

fn gzipped(path: &str) -> Vec<u8> {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    enc.write_all(&fixture(path)).unwrap();
    enc.finish().unwrap()
}

fn faus() -> (Vec<fau_domain::register::source::BrregFau>, brreg::BrregStats) {
    let mut out = Vec::new();
    let stats = brreg::for_each_fau(&gzipped("brreg/enheter-sample.json")[..], |f| out.push(f)).unwrap();
    (out, stats)
}

#[test]
fn only_fli_units_with_an_fau_name_are_kept() {
    let (faus, stats) = faus();
    assert_eq!(stats.units_seen, 9);
    assert_eq!(stats.faus, 8, "FAUSKE IDRETTSLAG ALPINT is not an FAU");
    assert!(!faus.iter().any(|f| f.orgnr == "988936871"));
    let hosle = faus.iter().find(|f| f.orgnr == "918316450").unwrap();
    assert_eq!(hosle.registered_name, "HOSLE FAU");
    assert_eq!(hosle.organisation_form, "FLI");
    assert_eq!(hosle.municipality_number.as_deref(), Some("3201"));
    assert_eq!(hosle.business_address.lines, ["Bispeveien 73"]);
    assert_eq!(hosle.business_address.postcode.as_deref(), Some("1362"));
    assert_eq!(hosle.postal_address, Default::default());
}

#[test]
fn contact_fields_are_never_read() {
    // 913591100 carries an invented e-mail and mobile (fixtures README). The value types have
    // no field for them, and nothing the parser returns may contain them.
    let (faus, _) = faus();
    let shown = format!("{faus:?}");
    for needle in ["example.invalid", "+4700000000", "Nordmann", "Oppdiktet"] {
        assert!(!shown.contains(needle), "{needle} leaked into {shown}");
    }
}

#[test]
fn a_truncated_file_is_an_error() {
    let full = fixture("brreg/enheter-sample.json");
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    enc.write_all(&full[..full.len() / 2]).unwrap();
    let err = brreg::for_each_fau(&enc.finish().unwrap()[..], |_| {}).unwrap_err();
    assert_eq!(err.source, fau_register_sources::Source::Brreg);
}

#[test]
fn a_unit_without_a_name_fails_loudly() {
    let mut v: serde_json::Value = serde_json::from_slice(&fixture("brreg/enheter-sample.json")).unwrap();
    v[0].as_object_mut().unwrap().remove("navn");
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    enc.write_all(&serde_json::to_vec(&v).unwrap()).unwrap();
    let err = brreg::for_each_fau(&enc.finish().unwrap()[..], |_| {}).unwrap_err();
    assert!(matches!(err.kind, SourceErrorKind::Parse { .. }));
}
```

Add `flate2 = { workspace = true }` to `[dev-dependencies]`. It is already a normal dependency,
so that is only needed if the test crate cannot see it. Normal dependencies are visible to
integration tests, so no change is expected.

- [ ] **Step 2: Run to verify failure**, via `cargo test -p fau-register-sources --test parse`. It fails to compile.

- [ ] **Step 3: Implement `src/brreg.rs`**

```rust
//! Brreg's Enhetsregisteret bulk file, `enheter/lastned`: a gzip JSON array of every main
//! unit, 1.18 million of them (§2.5). It is streamed one unit at a time, and only FAU-like
//! entities are kept.
//!
//! The DTO below declares no field for e-mail, phone, mobile, website or free text, so serde
//! never materialises them (Erik, 24 September 2026: contact data is fetched live by
//! outreach, never by the product). Address lines are kept only inside `BrregAddress`,
//! whose `Debug` redacts them.

use std::fmt;
use std::io::Read;

use fau_domain::register::brreg::is_fau_name;
use fau_domain::register::source::{BrregAddress, BrregFau};
use serde::de::{DeserializeSeed, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

use crate::error::{Source, SourceError};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BrregStats {
    pub units_seen: u64,
    pub faus: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnitDto {
    organisasjonsnummer: String,
    navn: String,
    organisasjonsform: FormDto,
    forretningsadresse: Option<AddressDto>,
    postadresse: Option<AddressDto>,
}

#[derive(Deserialize)]
struct FormDto {
    kode: String,
}

#[derive(Deserialize)]
struct AddressDto {
    #[serde(default)]
    adresse: Vec<Option<String>>,
    postnummer: Option<String>,
    kommunenummer: Option<String>,
}

fn address(a: Option<&AddressDto>) -> BrregAddress {
    match a {
        None => BrregAddress::default(),
        Some(a) => BrregAddress {
            lines: a.adresse.iter().flatten().map(|l| l.trim().to_owned()).filter(|l| !l.is_empty()).collect(),
            postcode: a.postnummer.clone().filter(|p| !p.trim().is_empty()),
        },
    }
}

struct Each<'f, F: FnMut(BrregFau)> {
    f: &'f mut F,
    stats: &'f mut BrregStats,
}

impl<'de, F: FnMut(BrregFau)> DeserializeSeed<'de> for Each<'_, F> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_seq(self)
    }
}

impl<'de, F: FnMut(BrregFau)> Visitor<'de> for Each<'_, F> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an array of Enhetsregisteret units")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        while let Some(u) = seq.next_element::<UnitDto>()? {
            self.stats.units_seen += 1;
            if u.organisasjonsform.kode != "FLI" || !is_fau_name(&u.navn) {
                continue;
            }
            self.stats.faus += 1;
            let municipality_number = u.forretningsadresse.as_ref().and_then(|a| a.kommunenummer.clone());
            (self.f)(BrregFau {
                orgnr: u.organisasjonsnummer,
                registered_name: u.navn,
                organisation_form: u.organisasjonsform.kode,
                municipality_number,
                business_address: address(u.forretningsadresse.as_ref()),
                postal_address: address(u.postadresse.as_ref()),
            });
        }
        Ok(())
    }
}

/// Streams a gzipped Enhetsregisteret array, calling `f` for each FAU-like entity (FLI plus
/// an FAU word in the name). A truncated file or a unit missing a required field is an error,
/// never a silent partial result.
pub fn for_each_fau<R: Read>(gzipped: R, mut f: impl FnMut(BrregFau)) -> Result<BrregStats, SourceError> {
    let reader = std::io::BufReader::new(flate2::read::GzDecoder::new(gzipped));
    let mut de = serde_json::Deserializer::from_reader(reader);
    let mut stats = BrregStats::default();
    Each { f: &mut f, stats: &mut stats }
        .deserialize(&mut de)
        .and_then(|()| de.end())
        .map_err(|e| SourceError::parse(Source::Brreg, &e))?;
    Ok(stats)
}
```

A truncated gzip stream surfaces as an I/O error inside serde_json, which is also a
`serde_json::Error`, so the single `map_err` covers both cases. Brreg sends address lines as an
array of strings. `Option<String>` elements tolerate a stray `null`.

- [ ] **Step 4: Run to verify pass**, via `cargo test -p fau-register-sources`, then the workspace, fmt and clippy.

- [ ] **Step 5: Commit**: `git commit -m "Stream Brreg's bulk file and keep only FAU entities (#3441)"`

---

### Task 5: The HTTP client, tested against a real local server

**Files:**
- Modify: `backend/crates/register-sources/src/client.rs`
- Create: `backend/crates/register-sources/tests/client.rs`

**Interfaces:**
- Produces, for part 3's CLI:
  - `SourceUrls { nsr, kartverket, ssb, brreg: String }`, with `SourceUrls::production()`;
  - `SourceClient::new(urls: SourceUrls) -> Result<SourceClient, SourceError>`.

  Async methods on `SourceClient`:

  | Method | Returns |
  | --- | --- |
  | `nsr_all_units(&self)` | `Result<Vec<NsrListItem>, SourceError>`: every page, 1,000 per page |
  | `nsr_units_in(&self, municipality_number: &str)` | `Result<Vec<NsrListItem>, SourceError>` |
  | `nsr_unit(&self, orgnr: &str)` | `Result<NsrUnit, SourceError>` |
  | `municipalities(&self)` | `Result<Vec<MunicipalityRecord>, SourceError>` |
  | `code_changes(&self, from: Date, to: Date)` | `Result<Vec<CodeChange>, SourceError>` |
  | `download_brreg_bulk(&self, dest: &Path)` | `Result<u64, SourceError>`: bytes written |

- [ ] **Step 1: Write the failing tests** in `tests/client.rs`

```rust
//! The client against a real HTTP server on 127.0.0.1, serving the recorded fixtures: real
//! sockets, real reqwest, no mocks.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::{Path, Query};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::Router;
use fau_register_sources::client::{SourceClient, SourceUrls};
use fau_register_sources::{Source, SourceErrorKind};

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{path}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

/// NSR paging served from one recorded unit list, three units per page, so the client has to
/// follow several pages and stop at an empty one.
fn paged_nsr(q: &std::collections::HashMap<String, String>) -> Vec<u8> {
    let list: serde_json::Value = serde_json::from_slice(&fixture("nsr/kommune-3201.json")).unwrap();
    let all = list["EnhetListe"].as_array().unwrap()[..7].to_vec();
    let page: usize = q["sidenummer"].parse().unwrap();
    let chunk: Vec<_> = all.chunks(3).nth(page - 1).map(|c| c.to_vec()).unwrap_or_default();
    serde_json::to_vec(&serde_json::json!({
        "Sidenummer": page, "AntallPerSide": 3, "AntallSider": 3, "TotaltAntallEnheter": 7, "EnhetListe": chunk
    }))
    .unwrap()
}

async fn serve(hits: Arc<AtomicUsize>) -> String {
    let app = Router::new()
        .route("/nsr/v4/enheter", get(|Query(q): Query<std::collections::HashMap<String, String>>| async move { paged_nsr(&q) }))
        .route("/nsr/v4/enheter/kommune/{nr}", get(|Path(nr): Path<String>| async move {
            if nr == "3201" { (StatusCode::OK, fixture("nsr/kommune-3201.json")) } else { (StatusCode::NOT_FOUND, Vec::new()) }
        }))
        .route("/nsr/v4/enhet/{orgnr}", get(|Path(o): Path<String>| async move { fixture(&format!("nsr/enhet-{o}.json")) }))
        .route("/kv/fylkerkommuner", get(|| async { fixture("kartverket/fylkerkommuner.json") }))
        .route("/ssb/classifications/131/changes", get(|Query(q): Query<std::collections::HashMap<String, String>>, h: HeaderMap| async move {
            assert_eq!(h.get("accept").unwrap(), "application/json", "SSB returns XML without it");
            assert_eq!((q["from"].as_str(), q["to"].as_str()), ("2023-12-01", "2024-01-31"));
            fixture("ssb/changes-2024.json")
        }))
        .route("/brreg/enheter/lastned", get(move || {
            let hits = hits.clone();
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
                std::io::Write::write_all(&mut enc, &fixture("brreg/enheter-sample.json")).unwrap();
                enc.finish().unwrap()
            }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

async fn client() -> (SourceClient, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let base = serve(hits.clone()).await;
    let urls = SourceUrls {
        nsr: format!("{base}/nsr"),
        kartverket: format!("{base}/kv"),
        ssb: format!("{base}/ssb"),
        brreg: format!("{base}/brreg"),
    };
    (SourceClient::new(urls).unwrap(), hits)
}

#[tokio::test]
async fn nsr_paging_follows_every_page() {
    let (c, _) = client().await;
    let units = c.nsr_all_units().await.unwrap();
    assert_eq!(units.len(), 7, "3 + 3 + 1 across three pages");
}

#[tokio::test]
async fn nsr_municipality_list_and_detail() {
    let (c, _) = client().await;
    assert_eq!(c.nsr_units_in("3201").await.unwrap().len(), 218);
    assert_eq!(c.nsr_unit("974552124").await.unwrap().name, "Hosle skole");
}

#[tokio::test]
async fn an_http_error_status_is_reported_with_its_source() {
    let (c, _) = client().await;
    let err = c.nsr_units_in("9999").await.unwrap_err();
    assert_eq!((err.source, err.kind), (Source::Nsr, SourceErrorKind::Status { status: 404 }));
}

#[tokio::test]
async fn municipalities_and_changes() {
    let (c, _) = client().await;
    assert_eq!(c.municipalities().await.unwrap().len(), 357);
    let from = "2023-12-01".parse().unwrap();
    let to = "2024-01-31".parse().unwrap();
    assert_eq!(c.code_changes(from, to).await.unwrap().len(), 118);
}

#[tokio::test]
async fn brreg_bulk_download_lands_on_disk_and_streams() {
    let (c, hits) = client().await;
    let dir = std::env::temp_dir().join(format!("fau-brreg-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dest = dir.join("enheter.json.gz");
    let written = c.download_brreg_bulk(&dest).await.unwrap();
    assert!(written > 0);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let stats = fau_register_sources::brreg::for_each_fau(std::fs::File::open(&dest).unwrap(), |_| {}).unwrap();
    assert_eq!(stats.faus, 8);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn an_unreachable_source_is_a_transport_error() {
    let dead = SourceUrls {
        nsr: "http://127.0.0.1:9/nsr".into(),
        kartverket: "http://127.0.0.1:9/kv".into(),
        ssb: "http://127.0.0.1:9/ssb".into(),
        brreg: "http://127.0.0.1:9/brreg".into(),
    };
    let err = SourceClient::new(dead).unwrap().municipalities().await.unwrap_err();
    assert_eq!((err.source, err.kind), (Source::Kartverket, SourceErrorKind::Transport));
}

#[test]
fn production_urls_are_the_documented_ones() {
    let u = SourceUrls::production();
    assert_eq!(u.nsr, "https://data-nsr.udir.no");
    assert_eq!(u.kartverket, "https://api.kartverket.no/kommuneinfo/v1");
    assert_eq!(u.ssb, "https://data.ssb.no/api/klass/v1");
    assert_eq!(u.brreg, "https://data.brreg.no/enhetsregisteret/api");
}
```

Dev-dependencies: add `tokio = { workspace = true }` (the workspace's feature set includes
`macros`, `rt-multi-thread` and `net`). `axum` is already in the Task 2 manifest.

- [ ] **Step 2: Run to verify failure**, via `cargo test -p fau-register-sources --test client`. It fails to compile.

- [ ] **Step 3: Implement `src/client.rs`**

```rust
//! A small client for the four sources. Base URLs are parameters, so tests point it at a
//! local server, and production uses [`SourceUrls::production`].

use std::path::Path;
use std::time::Duration;

use fau_domain::register::source::{CodeChange, MunicipalityRecord, NsrUnit};
use jiff::civil::Date;
use tokio::io::AsyncWriteExt;

use crate::error::{Source, SourceError, SourceErrorKind};
use crate::nsr::NsrListItem;
use crate::{kartverket, nsr, ssb};

/// NSR pages are requested 1,000 at a time. 19 pages cover the whole register today (§2.1).
const NSR_PAGE_SIZE: u32 = 1000;
/// A hard stop, so an upstream paging bug cannot loop forever.
const NSR_MAX_PAGES: u32 = 100;
const BRREG_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceUrls {
    pub nsr: String,
    pub kartverket: String,
    pub ssb: String,
    pub brreg: String,
}

impl SourceUrls {
    pub fn production() -> Self {
        Self {
            nsr: "https://data-nsr.udir.no".into(),
            kartverket: "https://api.kartverket.no/kommuneinfo/v1".into(),
            ssb: "https://data.ssb.no/api/klass/v1".into(),
            brreg: "https://data.brreg.no/enhetsregisteret/api".into(),
        }
    }
}

pub struct SourceClient {
    http: reqwest::Client,
    urls: SourceUrls,
}

impl SourceClient {
    pub fn new(urls: SourceUrls) -> Result<Self, SourceError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .user_agent(concat!("fau-register/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| SourceError::new(Source::Nsr, SourceErrorKind::Transport))?;
        Ok(Self { http, urls })
    }

    async fn get(&self, source: Source, url: &str, query: &[(&str, String)], accept_json: bool) -> Result<Vec<u8>, SourceError> {
        let mut req = self.http.get(url).query(query);
        if accept_json {
            req = req.header(reqwest::header::ACCEPT, "application/json");
        }
        let resp = req.send().await.map_err(|_| SourceError::new(source, SourceErrorKind::Transport))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(SourceError::new(source, SourceErrorKind::Status { status: status.as_u16() }));
        }
        resp.bytes().await.map(|b| b.to_vec()).map_err(|_| SourceError::new(source, SourceErrorKind::Transport))
    }

    pub async fn nsr_all_units(&self) -> Result<Vec<NsrListItem>, SourceError> {
        let url = format!("{}/v4/enheter", self.urls.nsr);
        let mut units = Vec::new();
        let mut page_count = None;
        for page in 1..=NSR_MAX_PAGES {
            let body = self
                .get(Source::Nsr, &url, &[("sidenummer", page.to_string()), ("antallperside", NSR_PAGE_SIZE.to_string())], false)
                .await?;
            let parsed = nsr::parse_list_page(&body)?;
            let count = *page_count.get_or_insert(parsed.page_count);
            if parsed.units.is_empty() {
                return Ok(units);
            }
            units.extend(parsed.units);
            if page >= count {
                return Ok(units);
            }
        }
        Err(SourceError::new(Source::Nsr, SourceErrorKind::TooManyPages))
    }

    pub async fn nsr_units_in(&self, municipality_number: &str) -> Result<Vec<NsrListItem>, SourceError> {
        let url = format!("{}/v4/enheter/kommune/{municipality_number}", self.urls.nsr);
        Ok(nsr::parse_list_page(&self.get(Source::Nsr, &url, &[], false).await?)?.units)
    }

    pub async fn nsr_unit(&self, orgnr: &str) -> Result<NsrUnit, SourceError> {
        let url = format!("{}/v4/enhet/{orgnr}", self.urls.nsr);
        nsr::parse_unit(&self.get(Source::Nsr, &url, &[], false).await?)
    }

    pub async fn municipalities(&self) -> Result<Vec<MunicipalityRecord>, SourceError> {
        let url = format!("{}/fylkerkommuner", self.urls.kartverket);
        kartverket::parse_municipalities(&self.get(Source::Kartverket, &url, &[], false).await?)
    }

    pub async fn code_changes(&self, from: Date, to: Date) -> Result<Vec<CodeChange>, SourceError> {
        let url = format!("{}/classifications/131/changes", self.urls.ssb);
        let query = [("from", from.to_string()), ("to", to.to_string())];
        ssb::parse_changes(&self.get(Source::Ssb, &url, &query, true).await?)
    }

    /// Streams the gzipped bulk file to `dest` (about 210 MB) without holding it in memory.
    pub async fn download_brreg_bulk(&self, dest: &Path) -> Result<u64, SourceError> {
        let url = format!("{}/enheter/lastned", self.urls.brreg);
        let transport = |_| SourceError::new(Source::Brreg, SourceErrorKind::Transport);
        let mut resp = self.http.get(&url).timeout(BRREG_TIMEOUT).send().await.map_err(transport)?;
        if !resp.status().is_success() {
            return Err(SourceError::new(Source::Brreg, SourceErrorKind::Status { status: resp.status().as_u16() }));
        }
        let io = |_| SourceError::new(Source::Brreg, SourceErrorKind::Io);
        let mut file = tokio::fs::File::create(dest).await.map_err(io)?;
        let mut written = 0u64;
        while let Some(chunk) = resp.chunk().await.map_err(transport)? {
            file.write_all(&chunk).await.map_err(io)?;
            written += chunk.len() as u64;
        }
        file.flush().await.map_err(io)?;
        Ok(written)
    }
}
```

The workspace `tokio` features must include `fs` and `io-util` for `tokio::fs` and
`AsyncWriteExt`. If they do not, add the features to the `tokio` line in this crate's
`[dependencies]`, e.g. `tokio = { workspace = true, features = ["fs", "io-util"] }`. Do not change
the workspace line.

- [ ] **Step 4: Run to verify pass**, via `cargo test -p fau-register-sources`, then the workspace, fmt and clippy.
  The `127.0.0.1:9` test relies on nothing listening on the discard port. If the environment
  answers there, bind a listener, take its port, drop it, and use that port instead.

- [ ] **Step 5: Commit**: `git commit -m "Add the register source client, tested against a local server (#3441)"`

---

## Self-Review

- **Spec coverage:**
  - §2.1 NSR list, detail and paging: Tasks 2 and 5;
  - §2.2 Kartverket and SSB: Task 3;
  - §2.3 scope over real records: Task 2's classification test;
  - §2.5 Brreg candidate set: Task 4;
  - §5.1 egress hosts: the production URLs in Task 5;
  - §5.3 missing fields fail loudly: Tasks 2 and 4;
  - §10 recorded fixtures, no network: all tasks;
  - Erik's contact-email decision: Task 4's DTO and test, and Task 1's redacted `Debug`.
- **Later parts:**
  - the sync planner (part 3), which is pure domain code over these values plus the current
    register snapshot, with the circuit breaker;
  - apply, `register_sync_runs` and the `fau register sync|export` CLI;
  - the Brreg matcher, submission lookups, search SQL and the review CLI.
- **Types:**
  - `NsrListItem` (Task 2) is used by Task 5;
  - `BrregStats` (Task 4) is used by Task 5's test;
  - `SourceError::parse` / `new` (Task 2) are used by Tasks 3–5.
