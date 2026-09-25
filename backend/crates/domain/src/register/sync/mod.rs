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

use crate::register::source::NsrUnit;
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

    // Normalise the units before any planning, so create order, `Ref::New` numbering and slug
    // minting are independent of the caller's order. Sorted by orgnr and de-duplicated, keeping
    // the first occurrence (the caller merges two sources, so the same orgnr can appear twice).
    let mut units: Vec<NsrUnit> = inputs.units.to_vec();
    units.sort_by(|a, b| a.orgnr.cmp(&b.orgnr));
    units.dedup_by(|a, b| a.orgnr == b.orgnr);
    let inputs = &SyncInputs {
        units: &units,
        ..*inputs
    };

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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use jiff::civil::date;

    use super::testkit::{
        apply, change, fixture_records, fixture_units, hosle, inputs, municipality, ntg, record,
        school, stange_new, stange_old, unit,
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

    /// Plans, applies, and checks that the very same inputs then change nothing. Also returns
    /// every closed school's resolved successor (closed id -> successor id).
    fn apply_and_rerun(
        snapshot: &RegisterSnapshot<u32>,
        records: &[MunicipalityRecord],
        changes: &[CodeChange],
        units: &[NsrUnit],
        kind: RunKind,
    ) -> (SyncPlan<u32>, RegisterSnapshot<u32>, BTreeMap<u32, u32>) {
        let plan = applied(super::plan(
            snapshot,
            &inputs(records, changes, units, kind),
        ));
        let (after, successors) = apply(snapshot, &plan);
        assert_eq!(
            super::plan(&after, &inputs(records, changes, units, RunKind::Sync)),
            SyncOutcome::NoChange,
            "a second run on the same sources writes nothing"
        );
        (plan, after, successors)
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
        let (plan, after, _) = apply_and_rerun(
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
        let (plan, after, _) =
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
        let (plan, after, _) = apply_and_rerun(&snapshot, &[baerum], &[], &units, RunKind::Sync);
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
        let (plan, after, successors) =
            apply_and_rerun(&snapshot, &[stange], &[], &units, RunKind::Sync);
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
        assert_eq!(
            successors.get(&old.id),
            Some(&new.id),
            "the closed school's resolved successor is the new school"
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

    /// Controller ruling: `plan()` normalises `inputs.units` by orgnr before planning, so the
    /// caller's order (and duplicates, which the caller's merge of two sources can produce)
    /// never affect the outcome. A realistic mix: several new schools including two sharing a
    /// name in one municipality, one rename and one close, plus enough unchanged bystanders to
    /// stay under the circuit breaker. The duplicate orgnr carries two different names (as a
    /// stale second source read might), so the test also pins down *which* one the planner
    /// keeps: whichever is first in the caller's own order, in both orderings tried here.
    #[test]
    fn unit_order_and_duplicates_do_not_affect_the_outcome() {
        let baerum = record("3201", "Bærum", "32", "Akershus");
        let (bystander_schools, bystander_units) = bystanders(1, "3201", 60);
        let hosle_renamed = NsrUnit {
            name: "Hosle barneskole".into(),
            ..hosle()
        };
        let ntg_closed = inactive(ntg());
        let a = unit("974000001", "Ås skole", "3201");
        // Same orgnr as `a`, but a different name: the caller's merge of "every active
        // grunnskole" with "every orgnr the register holds" can read the same unit twice from
        // sources that disagree, e.g. a stale name from one of them.
        let a_stale = NsrUnit {
            name: "Ås skole (feilregistrert)".into(),
            ..a.clone()
        };
        let b = unit("974000002", "Ås skole", "3201");
        let mut units_in_order = vec![
            hosle_renamed.clone(),
            ntg_closed.clone(),
            a.clone(),
            b.clone(),
        ];
        units_in_order.extend(bystander_units.iter().cloned());
        // A different caller order. `a` still comes before `a_stale` here, as it must in both
        // orderings for the two plans to agree on which one wins; everything else is shuffled.
        let mut units_reordered = vec![
            b.clone(),
            a.clone(),
            a_stale.clone(),
            ntg_closed.clone(),
            hosle_renamed.clone(),
        ];
        units_reordered.extend(bystander_units.iter().rev().cloned());

        let mut schools = vec![
            school(10, 1, &hosle(), "hosle-skole"),
            school(11, 1, &ntg(), "ntg-baerum"),
        ];
        schools.extend(bystander_schools);
        let snapshot = RegisterSnapshot {
            municipalities: vec![municipality(1, &baerum)],
            schools,
            ..RegisterSnapshot::default()
        };

        let plan_a = applied(super::plan(
            &snapshot,
            &inputs(
                std::slice::from_ref(&baerum),
                &[],
                &units_in_order,
                RunKind::Sync,
            ),
        ));
        let plan_b = applied(super::plan(
            &snapshot,
            &inputs(
                std::slice::from_ref(&baerum),
                &[],
                &units_reordered,
                RunKind::Sync,
            ),
        ));
        assert_eq!(plan_a, plan_b);

        let (after_a, successors_a) = apply(&snapshot, &plan_a);
        let (after_b, successors_b) = apply(&snapshot, &plan_b);
        assert_eq!(after_a, after_b);
        assert_eq!(successors_a, successors_b);

        // The first-supplied `a` wins over the later `a_stale`, in both orderings.
        let created_name = |after: &RegisterSnapshot<u32>| {
            after
                .schools
                .iter()
                .find(|s| s.orgnr.as_deref() == Some("974000001"))
                .expect("Ås skole is created")
                .register_name
                .clone()
        };
        assert_eq!(created_name(&after_a), Some(a.name.clone()));
        assert_eq!(created_name(&after_b), Some(a.name.clone()));
    }

    /// Controller ruling: an out-of-scope close is applied as the part 4 handover says — the
    /// closed school's `in_scope` also goes false, not just its status.
    #[test]
    fn an_out_of_scope_close_sets_in_scope_false() {
        let baerum = record("3201", "Bærum", "32", "Akershus");
        let (bystander_schools, bystander_units) = bystanders(1, "3201", 60);
        let mut schools = vec![school(10, 1, &hosle(), "hosle-skole")];
        schools.extend(bystander_schools);
        let snapshot = RegisterSnapshot {
            municipalities: vec![municipality(1, &baerum)],
            schools,
            ..RegisterSnapshot::default()
        };
        let adult = NsrUnit {
            category_ids: vec!["10".into()],
            ..hosle()
        };
        let mut units = vec![adult];
        units.extend(bystander_units);

        let plan = applied(super::plan(
            &snapshot,
            &inputs(std::slice::from_ref(&baerum), &[], &units, RunKind::Sync),
        ));
        assert_eq!(plan.counts.schools_closed, 1);
        let (after, _) = apply(&snapshot, &plan);
        let closed = after
            .schools
            .iter()
            .find(|s| s.id == 10)
            .expect("hosle is still in the snapshot, closed");
        assert_eq!(closed.status, SchoolStatus::Closed);
        assert!(
            !closed.in_scope,
            "an out-of-scope close must clear in_scope"
        );
    }

    /// §5.3's breaker only fires on a Sync with `active > 0`: a snapshot with no active Listed
    /// or Verified school must not abort even when every op it does produce would, by raw
    /// percentage, look enormous against zero.
    #[test]
    fn no_active_schools_does_not_trip_the_breaker() {
        let baerum = record("3201", "Bærum", "32", "Akershus");
        let held = SchoolSnapshot {
            verification: Verification::Held,
            slug: None,
            ..school(10, 1, &hosle(), "hosle-skole")
        };
        let snapshot = RegisterSnapshot {
            municipalities: vec![municipality(1, &baerum)],
            schools: vec![held],
            ..RegisterSnapshot::default()
        };
        let renamed_hosle = NsrUnit {
            name: "Hosle barneskole".into(),
            ..hosle()
        };
        let a = unit("974000001", "Ås skole", "3201");
        let outcome = super::plan(
            &snapshot,
            &inputs(
                std::slice::from_ref(&baerum),
                &[],
                &[renamed_hosle, a],
                RunKind::Sync,
            ),
        );
        assert!(
            matches!(outcome, SyncOutcome::Apply(_)),
            "expected a plan with no active schools to apply, got {outcome:?}"
        );
    }
}
