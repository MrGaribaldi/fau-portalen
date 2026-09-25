//! Builders for the planner's tests. The values mirror the recorded fixtures in
//! crates/register-sources/tests/fixtures/ (see its README); the domain cannot depend on
//! that crate, so they are written out by hand.

use std::collections::{BTreeMap, HashMap, HashSet};

use jiff::civil::Date;
use jiff::Timestamp;

use crate::register::slug::municipality_slug;
use crate::register::source::{
    CodeChange, MunicipalityRecord, NsrAddress, NsrClosure, NsrUnit, OfficialName,
};
use crate::time::Moment;

use super::types::{
    ClosureReason, CreateVerification, MunicipalityOp, MunicipalitySnapshot, MunicipalitySource,
    MunicipalityStatus, Origin, Ref, RegisterSnapshot, RunKind, SchoolAttributes, SchoolOp,
    SchoolSnapshot, SchoolStatus, SlugChange, SyncInputs, SyncPlan, Verification,
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

/// Applies a plan to a snapshot as the persistence applier (part 4) must: every `Ref::New`
/// gets its id before any op runs, so a `Close` can name a successor created after it; then
/// the ops run in order. A close moves the slug to history and clears it, sets `in_scope =
/// false` when the reason is `OutOfScope`, and resolves its successor. The executable
/// specification the SQL applier has to match.
///
/// Returns the applied snapshot plus every closed school's resolved successor id (closed id ->
/// successor id), i.e. what `schools.successor_id` would hold. This is tracked test-side rather
/// than added to `SchoolSnapshot`: the plan's "Building the snapshot" list (what the applier
/// must read back for the planner) never mentions `successor_id`, because the planner itself
/// never reads a school's successor back from the snapshot — only `testkit::apply`'s callers
/// need it, to check the handover contract.
pub(super) fn apply(
    snapshot: &RegisterSnapshot<u32>,
    plan: &SyncPlan<u32>,
) -> (RegisterSnapshot<u32>, BTreeMap<u32, u32>) {
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
    // Closed school id -> its plan's successor `Ref`, resolved and validated only once every
    // `Create` op below has run: a `Close` can name a successor that is created later in this
    // same plan (see the doc comment above), so the successor row is not yet in `out.schools`
    // while `school_ops` is still being applied.
    let mut pending_successors: Vec<(u32, Ref<u32>)> = Vec::new();

    for op in &plan.school_ops {
        match op {
            SchoolOp::Close {
                id,
                reason,
                successor,
                ..
            } => {
                let s = school_mut(&mut out, *id);
                s.status = SchoolStatus::Closed;
                if *reason == ClosureReason::OutOfScope {
                    s.in_scope = false;
                }
                let released = s.slug.take().map(|slug| (s.municipality_id, slug, *id));
                out.school_slug_history.extend(released);
                if let Some(successor) = successor {
                    pending_successors.push((*id, *successor));
                }
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

    // Now that every `Create` has run, resolve and validate each closed school's successor:
    // the handover contract (part 4) is that it names a school that exists, is not Held, and
    // shares the closed school's municipality.
    let mut successors = BTreeMap::new();
    for (closed_id, successor) in pending_successors {
        let successor_id = match successor {
            Ref::Existing(id) => {
                assert!(
                    out.schools.iter().any(|s| s.id == id),
                    "successor {id} names no school in the snapshot"
                );
                id
            }
            Ref::New(n) => *new_schools.get(&n).unwrap_or_else(|| {
                panic!("successor names Ref::New({n}), which no Create op in this plan makes")
            }),
        };
        let closed_municipality = school_ref(&out, closed_id).municipality_id;
        let successor_school = school_ref(&out, successor_id);
        assert!(
            successor_school.verification != Verification::Held,
            "successor {successor_id} is a held row"
        );
        assert_eq!(
            successor_school.municipality_id, closed_municipality,
            "successor {successor_id} is not in the closed school {closed_id}'s municipality"
        );
        successors.insert(closed_id, successor_id);
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
    (out, successors)
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

fn school_ref(out: &RegisterSnapshot<u32>, id: u32) -> &SchoolSnapshot<u32> {
    out.schools
        .iter()
        .find(|s| s.id == id)
        .expect("the plan names an existing school")
}
