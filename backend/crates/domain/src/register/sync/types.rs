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
    /// The visiting address taken as a whole when it has a street; otherwise the whole postal
    /// address. Never mixes fields between the two (controller ruling, 25 September 2026).
    pub fn from_unit(unit: &NsrUnit) -> Self {
        let address = if unit.visiting.street.is_some() {
            &unit.visiting
        } else {
            &unit.postal
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
            street_address: address.street.clone(),
            postcode: address.postcode.clone(),
            post_town: address.post_town.clone(),
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
    /// A Norwegian-name change in the same run is folded in: `name` is the new name when it
    /// changed, and `new_slug` is minted once from the new number and the name the municipality
    /// ends up with. No separate `Rename` follows for it in the same plan.
    Renumber {
        id: Id,
        from: String,
        to: String,
        valid_from: Date,
        name: Option<String>,
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

    /// Controller ruling, 25 September 2026: the visiting address is taken as a whole when it
    /// has a street; a present postal field is never used to fill a gap in it.
    #[test]
    fn attributes_keep_the_visiting_address_even_with_a_gap() {
        let unit = NsrUnit {
            visiting: NsrAddress {
                street: Some("Bispeveien 73".into()),
                postcode: Some("1362".into()),
                post_town: None,
            },
            ..hosle()
        };
        let a = SchoolAttributes::from_unit(&unit);
        assert_eq!(a.street_address.as_deref(), Some("Bispeveien 73"));
        assert_eq!(a.postcode.as_deref(), Some("1362"));
        assert_eq!(
            a.post_town, None,
            "postal's post town must not fill the gap"
        );
    }

    /// The other half of the same ruling: no street in the visiting address at all falls back
    /// to the whole postal address, not field by field.
    #[test]
    fn attributes_fall_back_to_the_postal_address_as_a_whole() {
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
        assert_eq!(a.postcode.as_deref(), Some("1304"));
        assert_eq!(a.post_town.as_deref(), Some("SANDVIKA"));
    }
}
