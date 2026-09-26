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

/// An active municipality as this plan leaves it: a renumber changes its number and slug, and
/// its name when Kartverket renamed it in the same run.
struct Working<'a, Id> {
    row: &'a MunicipalitySnapshot<Id>,
    number: String,
    name: String,
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
                    name: row.name.clone(),
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

    // Controller ruling: a chain of renumbers (5001 -> 1234 -> 5501) must apply in the order it
    // actually happened, not in old-code order, or an intermediate code can be processed before
    // the change that creates it. A group's position is its earliest `occurred_on`, ties broken
    // by old code for a stable order.
    let mut ordered_groups: Vec<(&str, Vec<&CodeChange>)> = groups.into_iter().collect();
    ordered_groups.sort_by(|(a_code, a_changes), (b_code, b_changes)| {
        let a_min = a_changes
            .iter()
            .map(|c| c.occurred_on)
            .min()
            .expect("a group is never empty");
        let b_min = b_changes
            .iter()
            .map(|c| c.occurred_on)
            .min()
            .expect("a group is never empty");
        a_min.cmp(&b_min).then_with(|| a_code.cmp(b_code))
    });

    let records_by_number: BTreeMap<&str, &MunicipalityRecord> = inputs
        .municipalities
        .iter()
        .map(|r| (r.number.as_str(), r))
        .collect();
    let mut blocked_numbers: BTreeSet<String> = BTreeSet::new();
    for (old, changes) in &ordered_groups {
        let targets: BTreeMap<&str, &str> = changes
            .iter()
            .map(|c| (c.new_code.as_str(), c.new_name.as_str()))
            .collect();
        let holder = match by_number.get(*old) {
            Some(Ref::Existing(id)) => Some(*id),
            _ => None,
        };
        // Already resolved: nobody holds the old code and every target is held. Part 4 reads
        // SSB over a fixed lookback, so an applied renumber or a split an operator resolved
        // keeps appearing and must not block anything.
        if holder.is_none() && targets.keys().all(|t| by_number.contains_key(*t)) {
            continue;
        }
        let one_to_one = match targets.keys().next() {
            Some(to) if targets.len() == 1 && gone_groups_per_target[to] == 1 => Some(*to),
            _ => None,
        };
        let reason = match (one_to_one, holder) {
            // Nobody holds the old code, but another group in the feed changes something into
            // it: a chain this run cannot order (same day, or dated backwards).
            (_, None) if gone_groups_per_target.contains_key(*old) => "unresolved_chain",
            // Never held: there is nothing to renumber.
            (Some(_), None) => continue,
            (Some(to), Some(id)) => {
                let w = &working[&id];
                if w.row.source == MunicipalitySource::Manual {
                    continue;
                }
                // Kartverket's name for the new number, when it lists it; otherwise unchanged.
                let name = records_by_number
                    .get(to)
                    .map_or(w.name.as_str(), |r| r.norwegian_name.as_str());
                if blocked_numbers.contains(to) || blocked_numbers.contains(*old) {
                    "unresolved_chain"
                } else if by_number.contains_key(to) {
                    "target_in_use"
                } else if let Ok(new_slug) = municipality_slug(to, name) {
                    let renamed = (name != w.name).then(|| name.to_owned());
                    ops.push(MunicipalityOp::Renumber {
                        id,
                        from: (*old).to_owned(),
                        to: to.to_owned(),
                        valid_from: changes[0].occurred_on,
                        name: renamed.clone(),
                        old_slug: w.slug.clone(),
                        new_slug: new_slug.clone(),
                    });
                    by_number.remove(*old);
                    by_number.insert(to.to_owned(), Ref::Existing(id));
                    let w = working.get_mut(&id).expect("the holder is a working row");
                    w.number = to.to_owned();
                    w.slug = new_slug;
                    if let Some(name) = renamed {
                        w.name = name;
                    }
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
        let (codes_key, names_key) = if reason == "unresolved_chain" {
            ("new_code", "new_name")
        } else {
            ("new_codes", "new_names")
        };
        reviews.push(ReviewItem {
            kind: ReviewKind::MunicipalitySplitOrMerge,
            school: None,
            other_school: None,
            municipality: holder.map(Ref::Existing),
            details: vec![
                ("reason", reason.to_owned()),
                ("old_code", (*old).to_owned()),
                ("old_name", changes[0].old_name.clone()),
                (codes_key, join(targets.keys().copied())),
                (names_key, join(targets.values().copied())),
            ],
        });
    }

    // Kartverket against the municipality holding each number. Sorted by number first, so
    // `Create`'s order and `new` indices never depend on the API's own ordering.
    let mut sorted_records: Vec<&MunicipalityRecord> = inputs.municipalities.iter().collect();
    sorted_records.sort_by(|a, b| a.number.cmp(&b.number));
    let mut next_new = 0u32;
    for r in sorted_records {
        if blocked_numbers.contains(&r.number) {
            continue;
        }
        match by_number.get(&r.number) {
            Some(Ref::Existing(id)) => {
                let w = &working[id];
                if w.row.source == MunicipalitySource::Manual {
                    continue;
                }
                if r.norwegian_name != w.name {
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
                    ("name", w.name.clone()),
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
                    name: None,
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
    fn a_merger_into_a_surviving_code_is_target_in_use() {
        // 5002 B is gone from Kartverket, folded into 5001 A, which is still current: the
        // target number is already held by another active municipality.
        let a = municipality(1, &record("5001", "A", "50", "Femtylke"));
        let b = municipality(2, &record("5002", "B", "50", "Femtylke"));
        let records = [record("5001", "A", "50", "Femtylke")];
        let changes = [change("5002", "B", "5001", "A", date(2024, 1, 1))];
        let plan = run(
            &snapshot(vec![a, b]),
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
                municipality: Some(Ref::Existing(2)),
                details: vec![
                    ("reason", "target_in_use".into()),
                    ("old_code", "5002".into()),
                    ("old_name", "B".into()),
                    ("new_codes", "5001".into()),
                    ("new_names", "A".into()),
                ],
            }]
        );
        assert_eq!(
            plan.state.blocked_numbers,
            BTreeSet::from(["5001".into(), "5002".into()])
        );
        assert_eq!(plan.state.blocked_ids, BTreeSet::from([1, 2]));
    }

    #[test]
    fn a_chain_of_renumbers_is_applied_in_date_order_not_code_order() {
        // Controller ruling: a chain must apply in the order it actually happened. Code order
        // would process "1234" before "5001" (lexically smaller), which is backwards here.
        let testby = municipality(1, &record("5001", "Testby", "50", "Testfylke"));
        let records = [record("5501", "Testby", "50", "Testfylke")];
        let changes = [
            change("1234", "Testby", "5501", "Testby", date(2025, 1, 1)),
            change("5001", "Testby", "1234", "Testby", date(2024, 1, 1)),
        ];
        let plan = run(
            &snapshot(vec![testby]),
            &records,
            &changes,
            &[],
            RunKind::Sync,
        );
        assert_eq!(
            plan.ops,
            [
                MunicipalityOp::Renumber {
                    id: 1,
                    from: "5001".into(),
                    to: "1234".into(),
                    valid_from: date(2024, 1, 1),
                    name: None,
                    old_slug: "5001-testby".into(),
                    new_slug: "1234-testby".into(),
                },
                MunicipalityOp::Renumber {
                    id: 1,
                    from: "1234".into(),
                    to: "5501".into(),
                    valid_from: date(2025, 1, 1),
                    name: None,
                    old_slug: "1234-testby".into(),
                    new_slug: "5501-testby".into(),
                },
            ]
        );
        assert!(
            plan.reviews.is_empty(),
            "no unknown_number, no absent_from_kartverket: {:?}",
            plan.reviews
        );
        assert_eq!(
            plan.state.by_number,
            BTreeMap::from([("5501".to_owned(), Ref::Existing(1))])
        );
    }

    #[test]
    fn kartverket_records_are_sorted_before_planning() {
        let baerum = record("3201", "Bærum", "32", "Akershus");
        let stange = record("3413", "Stange", "34", "Innlandet");
        let forward = run(
            &RegisterSnapshot::default(),
            &[baerum.clone(), stange.clone()],
            &[],
            &[],
            RunKind::Seed,
        );
        let reversed = run(
            &RegisterSnapshot::default(),
            &[stange, baerum],
            &[],
            &[],
            RunKind::Seed,
        );
        assert!(!forward.ops.is_empty());
        assert_eq!(
            forward.ops, reversed.ops,
            "Create order and indices must not depend on API order"
        );
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
    fn a_resolved_split_neither_reviews_nor_blocks() {
        // The 2024 1507 split, once an operator has resolved it: 1508 and 1580 exist, 1507
        // is held by no active municipality. The fixed SSB lookback keeps showing the split.
        let aalesund = municipality(1, &record("1508", "Ålesund", "15", "Møre og Romsdal"));
        let haram = municipality(2, &record("1580", "Haram", "15", "Møre og Romsdal"));
        let records = [
            record("1508", "Ålesund", "15", "Møre og Romsdal"),
            record("1580", "Haram", "15", "Møre og Romsdal"),
        ];
        let changes = [
            change("1507", "Ålesund", "1508", "Ålesund", date(2024, 1, 1)),
            change("1507", "Ålesund", "1580", "Haram", date(2024, 1, 1)),
        ];
        let plan = run(
            &snapshot(vec![aalesund, haram]),
            &records,
            &changes,
            &[],
            RunKind::Sync,
        );
        assert!(plan.ops.is_empty(), "{:?}", plan.ops);
        assert!(plan.reviews.is_empty(), "{:?}", plan.reviews);
        assert!(plan.state.blocked_numbers.is_empty());
        assert!(plan.state.blocked_ids.is_empty());
    }

    #[test]
    fn a_resolved_merger_neither_reviews_nor_blocks_but_an_open_one_still_does() {
        // 5055 Heim exists; 1571 Halsa is gone, 5011 Hemne is still held.
        let heim = municipality(1, &record("5055", "Heim", "50", "Trøndelag"));
        let hemne = municipality(2, &record("5011", "Hemne", "50", "Trøndelag"));
        let records = [record("5055", "Heim", "50", "Trøndelag")];
        let changes = [
            change("1571", "Halsa", "5055", "Heim", date(2020, 1, 1)),
            change("5011", "Hemne", "5055", "Heim", date(2020, 1, 1)),
        ];
        let plan = run(
            &snapshot(vec![heim.clone()]),
            &records,
            &changes,
            &[],
            RunKind::Sync,
        );
        assert!(plan.ops.is_empty() && plan.reviews.is_empty(), "{plan:?}");
        assert!(plan.state.blocked_numbers.is_empty());

        let plan = run(
            &snapshot(vec![heim, hemne]),
            &records,
            &changes,
            &[],
            RunKind::Sync,
        );
        let reasons: Vec<_> = plan
            .reviews
            .iter()
            .map(|r| (r.details[0].1.as_str(), r.details[1].1.as_str()))
            .collect();
        assert_eq!(reasons, [("merge", "5011")], "1571's group is resolved");
    }

    #[test]
    fn a_renumber_and_a_rename_in_one_run_are_one_renumber() {
        let kristiansund =
            municipality(1, &record("1503", "Kristiansund", "15", "Møre og Romsdal"));
        let records = [record("1599", "Nyby", "15", "Møre og Romsdal")];
        let changes = [change(
            "1503",
            "Kristiansund",
            "1599",
            "Nyby",
            date(2027, 1, 1),
        )];
        let plan = run(
            &snapshot(vec![kristiansund]),
            &records,
            &changes,
            &[],
            RunKind::Sync,
        );
        assert_eq!(
            plan.ops,
            [
                MunicipalityOp::Renumber {
                    id: 1,
                    from: "1503".into(),
                    to: "1599".into(),
                    valid_from: date(2027, 1, 1),
                    name: Some("Nyby".into()),
                    old_slug: "1503-kristiansund".into(),
                    new_slug: "1599-nyby".into(),
                },
                MunicipalityOp::UpdateDetails {
                    id: 1,
                    official_name: Some("Nyby".into()),
                    county_number: "15".into(),
                    county_name: "Møre og Romsdal".into(),
                    names: no_names("Nyby"),
                },
            ],
            "no separate Rename"
        );
        assert!(plan.reviews.is_empty(), "{:?}", plan.reviews);
    }

    #[test]
    fn a_same_day_descending_chain_is_reviewed_and_blocked() {
        // Both on one day, so code order puts 1234 -> 5501 before 5001 -> 1234: the group for
        // 1234 has no holder yet, but 1234 is the target of another group in the feed.
        let testby = municipality(1, &record("5001", "Testby", "50", "Testfylke"));
        let records = [record("5501", "Testby", "50", "Testfylke")];
        let d = date(2024, 1, 1);
        let changes = [
            change("1234", "Testby", "5501", "Testby", d),
            change("5001", "Testby", "1234", "Testby", d),
        ];
        let plan = run(
            &snapshot(vec![testby]),
            &records,
            &changes,
            &[],
            RunKind::Sync,
        );
        assert!(
            plan.ops.is_empty(),
            "no renumber into a blocked code: {:?}",
            plan.ops
        );
        let chain = |municipality, old: &str, new: &str| ReviewItem {
            kind: ReviewKind::MunicipalitySplitOrMerge,
            school: None,
            other_school: None,
            municipality,
            details: vec![
                ("reason", "unresolved_chain".into()),
                ("old_code", old.into()),
                ("old_name", "Testby".into()),
                ("new_code", new.into()),
                ("new_name", "Testby".into()),
            ],
        };
        assert_eq!(
            plan.reviews,
            [
                chain(None, "1234", "5501"),
                chain(Some(Ref::Existing(1)), "5001", "1234"),
            ]
        );
        assert_eq!(
            plan.state.blocked_numbers,
            BTreeSet::from(["1234".into(), "5001".into(), "5501".into()])
        );
        assert_eq!(plan.state.blocked_ids, BTreeSet::from([1]));
    }

    #[test]
    fn an_applied_chain_is_skipped_on_the_next_run() {
        // The fixed lookback shows the chain again once 5501 holds it: nothing to do.
        let testby = municipality(1, &record("5501", "Testby", "50", "Testfylke"));
        let records = [record("5501", "Testby", "50", "Testfylke")];
        for d2 in [date(2024, 1, 1), date(2025, 1, 1)] {
            let changes = [
                change("1234", "Testby", "5501", "Testby", d2),
                change("5001", "Testby", "1234", "Testby", date(2024, 1, 1)),
            ];
            let plan = run(
                &snapshot(vec![testby.clone()]),
                &records,
                &changes,
                &[],
                RunKind::Sync,
            );
            assert!(plan.ops.is_empty() && plan.reviews.is_empty(), "{plan:?}");
            assert!(plan.state.blocked_numbers.is_empty());
        }
    }

    /// Part 4 de-duplicates review items on (`kind`, `school`, `other_school`,
    /// `municipality`) plus a discriminator from `details`: `reason` and `number` or
    /// `old_code` for this kind. Every reason the municipality planner raises carries them.
    #[test]
    fn every_split_or_merge_item_carries_the_dedup_discriminator() {
        let d = date(2024, 1, 1);
        let m = |id, n: &str, name: &str| municipality(id, &record(n, name, "50", "F"));
        type Scenario = (
            Vec<MunicipalitySnapshot<u32>>,
            Vec<MunicipalityRecord>,
            Vec<CodeChange>,
            RunKind,
        );
        let scenarios: Vec<Scenario> = vec![
            // split
            (
                vec![m(1, "1507", "A")],
                vec![
                    record("1508", "A", "50", "F"),
                    record("1580", "B", "50", "F"),
                ],
                vec![
                    change("1507", "A", "1508", "A", d),
                    change("1507", "A", "1580", "B", d),
                ],
                RunKind::Sync,
            ),
            // merge
            (
                vec![m(1, "1571", "A"), m(2, "5011", "B")],
                vec![record("5055", "C", "50", "F")],
                vec![
                    change("1571", "A", "5055", "C", d),
                    change("5011", "B", "5055", "C", d),
                ],
                RunKind::Sync,
            ),
            // target_in_use
            (
                vec![m(1, "5001", "A"), m(2, "5002", "B")],
                vec![record("5001", "A", "50", "F")],
                vec![change("5002", "B", "5001", "A", d)],
                RunKind::Sync,
            ),
            // unsluggable renumber, unsluggable record, unknown_number, absent_from_kartverket
            (
                vec![m(1, "5001", "A"), m(2, "5003", "C")],
                vec![
                    record("5002", "Школа", "50", "F"),
                    record("5009", "Школа", "50", "F"),
                    record("5010", "D", "50", "F"),
                ],
                vec![change("5001", "A", "5002", "Школа", d)],
                RunKind::Sync,
            ),
            // unsluggable on a seed
            (
                vec![],
                vec![record("5009", "Школа", "50", "F")],
                vec![],
                RunKind::Seed,
            ),
            // unresolved_chain
            (
                vec![m(1, "5001", "A")],
                vec![record("5501", "A", "50", "F")],
                vec![
                    change("1234", "A", "5501", "A", d),
                    change("5001", "A", "1234", "A", d),
                ],
                RunKind::Sync,
            ),
        ];
        let mut reasons = BTreeSet::new();
        for (rows, records, changes, kind) in scenarios {
            let plan = run(&snapshot(rows), &records, &changes, &[], kind);
            for item in &plan.reviews {
                assert_eq!(item.kind, ReviewKind::MunicipalitySplitOrMerge);
                let key = |k: &str| item.details.iter().any(|(name, _)| *name == k);
                assert!(key("reason"), "{item:?}");
                assert!(key("number") || key("old_code"), "{item:?}");
                reasons.insert(item.details[0].1.clone());
            }
        }
        assert_eq!(
            reasons,
            BTreeSet::from(
                [
                    "absent_from_kartverket",
                    "merge",
                    "split",
                    "target_in_use",
                    "unknown_number",
                    "unresolved_chain",
                    "unsluggable",
                ]
                .map(str::to_owned)
            ),
            "every reason is exercised"
        );
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
