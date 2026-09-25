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
        // After the 1507 split is resolved, Brattvåg barneskole's NSR number 1580 resolves
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
