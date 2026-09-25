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
    // slug before any rename can take it (§4.5). Every candidate is assigned at once, by a
    // global best match rather than in input order.
    let placements = place_all(
        &candidates,
        snapshot,
        &mut closings,
        &mut fau_closings,
        &mut reviews,
    );
    for (i, placement) in placements.iter().enumerate() {
        if let Placement::Successor(Some(slug)) = placement {
            book.take(candidates[i].municipality, slug, Ref::New(new_number(i)));
        }
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

/// Which predecessor a (candidate, predecessor) match refers to.
enum PredKind {
    Closing(usize),
    Fau(usize),
}

/// §4.5: assign every candidate a re-registration, a submission match, or a plain listing, all
/// at once. Every (candidate, predecessor) pair at or above
/// `REREGISTRATION_THRESHOLD`, in the same municipality, competes globally: sorted by
/// similarity descending, then the candidate's orgnr, then — for a tie on both — the FAU
/// predecessor before a non-FAU one (the existing preference, since a human sees it either
/// way), then the predecessor's own orgnr and id. Assigning in that order and taking each candidate's
/// and each closing predecessor's *first* (i.e. best) match makes predecessor assignment
/// independent of input order; `plan()` normalises `inputs.units`'s order for everything else
/// (create order, `Ref::New` numbering and slug minting).
///
/// A closing (non-FAU) predecessor gives its slug and successor link to its single best match
/// and closes as `Merged`; any other candidate that also matched it gets nothing from it (one
/// successor per closing school). An FAU predecessor is never closed (D8), so it is never
/// "used up": every candidate that matches one, at or above the threshold, is held and
/// reviewed on its own account, even when a different, higher-priority candidate already
/// claimed the same FAU predecessor (a split against a live FAU is several reviews, not one).
fn place_all<Id: RowId>(
    candidates: &[Candidate<'_, Id>],
    snapshot: &RegisterSnapshot<Id>,
    closings: &mut [Closing<'_, Id>],
    fau_closings: &mut [FauClosing<'_, Id>],
    reviews: &mut Vec<ReviewItem<Id>>,
) -> Vec<Placement> {
    let in_municipality = |cand: &Candidate<'_, Id>, municipality_id: Id| {
        Ref::Existing(municipality_id) == cand.municipality
    };

    let mut matches: Vec<(usize, PredKind, f32)> = Vec::new();
    for (i, cand) in candidates.iter().enumerate() {
        let name = cand.unit.name.as_str();
        for (j, c) in closings.iter().enumerate() {
            if c.reregistrable && in_municipality(cand, c.school.municipality_id) {
                let sim = name_similarity(c.school, name);
                if sim >= REREGISTRATION_THRESHOLD {
                    matches.push((i, PredKind::Closing(j), sim));
                }
            }
        }
        for (k, f) in fau_closings.iter().enumerate() {
            if f.reregistrable && in_municipality(cand, f.school.municipality_id) {
                let sim = name_similarity(f.school, name);
                if sim >= REREGISTRATION_THRESHOLD {
                    matches.push((i, PredKind::Fau(k), sim));
                }
            }
        }
    }

    let pred_key = |pred: &PredKind| -> (&str, Id) {
        let school = match *pred {
            PredKind::Closing(j) => closings[j].school,
            PredKind::Fau(k) => fau_closings[k].school,
        };
        (school.orgnr.as_deref().unwrap_or(""), school.id)
    };
    matches.sort_by(|a, b| {
        b.2.partial_cmp(&a.2)
            .expect("similarity is always finite")
            .then_with(|| candidates[a.0].unit.orgnr.cmp(&candidates[b.0].unit.orgnr))
            .then_with(|| {
                let a_fau = matches!(a.1, PredKind::Fau(_));
                let b_fau = matches!(b.1, PredKind::Fau(_));
                b_fau.cmp(&a_fau)
            })
            .then_with(|| pred_key(&a.1).cmp(&pred_key(&b.1)))
    });

    let mut placement: Vec<Option<Placement>> = (0..candidates.len()).map(|_| None).collect();
    let mut closing_claimed: BTreeSet<usize> = BTreeSet::new();
    for (unit_i, pred, sim) in matches {
        if placement[unit_i].is_some() {
            continue;
        }
        match pred {
            PredKind::Closing(j) => {
                if closing_claimed.contains(&j) {
                    continue;
                }
                closing_claimed.insert(j);
                let me = Ref::New(new_number(unit_i));
                closings[j].successor = Some(me);
                // §4.5: a re-registration's predecessor closes as merged, whatever NSR's code.
                closings[j].reason = ClosureReason::Merged;
                placement[unit_i] = Some(Placement::Successor(closings[j].school.slug.clone()));
            }
            PredKind::Fau(k) => {
                let cand = &candidates[unit_i];
                let me = Ref::New(new_number(unit_i));
                let f = &mut fau_closings[k];
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
                placement[unit_i] = Some(Placement::HeldReregistration);
            }
        }
    }

    // Submission matching, for every candidate that got no predecessor above (§4.5). Only a
    // submission that has no orgnr yet can be the unit.
    for (i, cand) in candidates.iter().enumerate() {
        if placement[i].is_some() {
            continue;
        }
        let name = cand.unit.name.as_str();
        let me = Ref::New(new_number(i));
        let submitted = best(
            snapshot.schools.iter().filter(|s| {
                s.origin == Origin::Submitted
                    && s.orgnr.is_none()
                    && s.status == SchoolStatus::Active
                    && matches!(
                        s.verification,
                        Verification::Pending | Verification::Verified
                    )
                    && in_municipality(cand, s.municipality_id)
            }),
            |s| name_similarity(s, name),
            SUBMISSION_MATCH_THRESHOLD,
        );
        placement[i] = Some(if let Some((p, sim)) = submitted {
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
            Placement::HeldSubmission
        } else {
            Placement::Listed
        });
    }

    placement
        .into_iter()
        .map(|p| p.expect("every candidate got a placement above"))
        .collect()
}

/// How alike a school and a unit name are: the better of the display name and the register
/// name, so a curated display name (D5) does not hide the school from its own NSR name.
fn name_similarity<Id>(school: &SchoolSnapshot<Id>, name: &str) -> f32 {
    let display = similarity(&school.display_name, name);
    school
        .register_name
        .as_deref()
        .map_or(display, |register| display.max(similarity(register, name)))
}

/// The most similar school at or above `threshold`; the lowest id on a tie, so the result
/// never depends on the snapshot's row order.
fn best<'a, Id: RowId>(
    items: impl Iterator<Item = &'a SchoolSnapshot<Id>>,
    score: impl Fn(&SchoolSnapshot<Id>) -> f32,
    threshold: f32,
) -> Option<(&'a SchoolSnapshot<Id>, f32)> {
    let mut found: Option<(&SchoolSnapshot<Id>, f32)> = None;
    for item in items {
        let s = score(item);
        let better = found
            .as_ref()
            .is_none_or(|(b, best)| s > *best || (s == *best && item.id < b.id));
        if s >= threshold && better {
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
            _ if moving || school.display_name_curated => None,
            Some(old) => slugify(&unit.name)
                .ok()
                .filter(|base| base != old)
                .map(|base| book.mint(here, &base, post_town, me))
                .filter(|new| new != old)
                .map(|new| SlugChange {
                    old: Some(old.clone()),
                    new,
                }),
            // A school without a slug gets its first one once its name is sluggable.
            None => slugify(&unit.name).ok().map(|base| SlugChange {
                old: None,
                new: book.mint(here, &base, post_town, me),
            }),
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
                old: Some(old.clone()),
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
        address, at, fixture_records, fixture_units, gran_canaria, hosle, inputs, school,
        stange_new, stange_old, ts, unit,
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
                    old: Some("hosle-skole".into()),
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
                    old: Some("brattvaag-barneskule".into()),
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

    fn stange_create(
        new: u32,
        slug: Option<&str>,
        verification: CreateVerification,
    ) -> SchoolOp<u32> {
        SchoolOp::Create {
            new,
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

    /// A second unit that might be "Stange ungdomsskole" under yet another number: a split.
    /// similarity("Stange ungdomsskole", "Stange ungdomsskole nord") is 0.80, above the
    /// threshold but below the exact name's 1.00.
    fn stange_nord() -> NsrUnit {
        unit("933181996", "Stange ungdomsskole nord", "3413")
    }

    fn stange_nord_create(
        new: u32,
        slug: Option<&str>,
        verification: CreateVerification,
    ) -> SchoolOp<u32> {
        SchoolOp::Create {
            new,
            municipality: Ref::Existing(20),
            orgnr: "933181996".into(),
            register_name: "Stange ungdomsskole nord".into(),
            display_name: "Stange ungdomsskole nord".into(),
            slug: slug.map(str::to_owned),
            verification,
            in_scope: true,
            attributes: SchoolAttributes::from_unit(&stange_nord()),
            source_changed_at: None,
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
                stange_create(0, Some("stange-ungdomsskole"), CreateVerification::Listed),
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
            [stange_create(0, None, CreateVerification::Held)],
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
                0,
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
                stange_create(0, Some("stange-ungdomsskole"), CreateVerification::Listed),
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
            assert!(
                matches!(
                    plan.ops[0],
                    SchoolOp::Close {
                        reason: ClosureReason::Merged,
                        ..
                    }
                ),
                "{code}: a close with a successor is always Merged (§4.5): {:?}",
                plan.ops
            );
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
                stange_create(0, Some("stange-ungdomsskole"), CreateVerification::Listed),
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
                    0,
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

    // A split (several new units matching one closing predecessor) must resolve to a single
    // global best match, not to whichever unit the input lists first.

    #[test]
    fn a_split_assigns_the_slug_to_the_best_match_not_the_first_seen() {
        // similarity("Stange ungdomsskole", "Stange ungdomsskole nord") is 0.80: above the
        // threshold, but the exact name's 1.00 must win the slug and successor link, whichever
        // unit `inputs.units` lists first.
        for nord_first in [true, false] {
            let units = if nord_first {
                vec![stange_old(), stange_nord(), stange_new()]
            } else {
                vec![stange_old(), stange_new(), stange_nord()]
            };
            let (nord_new, exact_new) = if nord_first { (0, 1) } else { (1, 0) };
            let plan = run(&with_schools(vec![stange_row(false)]), &units, &stange());
            assert_eq!(
                plan.ops[0],
                SchoolOp::Close {
                    id: 30,
                    reason: ClosureReason::Merged,
                    closed_on: date(2024, 8, 25),
                    successor: Some(Ref::New(exact_new)),
                },
                "nord_first={nord_first}: {:?}",
                plan.ops
            );
            assert!(
                plan.ops.contains(&stange_create(
                    exact_new,
                    Some("stange-ungdomsskole"),
                    CreateVerification::Listed
                )),
                "nord_first={nord_first}: the exact name takes the old slug: {:?}",
                plan.ops
            );
            assert!(
                plan.ops.contains(&stange_nord_create(
                    nord_new,
                    Some("stange-ungdomsskole-nord"),
                    CreateVerification::Listed
                )),
                "nord_first={nord_first}: nord gets its own fresh slug: {:?}",
                plan.ops
            );
            assert_eq!(plan.ops.len(), 3, "nord_first={nord_first}: {:?}", plan.ops);
            assert!(plan.reviews.is_empty(), "{:?}", plan.reviews);
        }
    }

    #[test]
    fn a_split_with_an_fau_holds_every_matching_unit_for_review() {
        // Same split, but the old school has a live FAU: it is never closed, and both new
        // units are their own possible re-registration, not just the best match.
        for nord_first in [true, false] {
            let units = if nord_first {
                vec![stange_old(), stange_nord(), stange_new()]
            } else {
                vec![stange_old(), stange_new(), stange_nord()]
            };
            let (nord_new, exact_new) = if nord_first { (0, 1) } else { (1, 0) };
            let plan = run(&with_schools(vec![stange_row(true)]), &units, &stange());
            assert!(
                !plan
                    .ops
                    .iter()
                    .any(|op| matches!(op, SchoolOp::Close { .. })),
                "nord_first={nord_first}: an FAU school is never closed: {:?}",
                plan.ops
            );
            assert!(
                plan.ops
                    .contains(&stange_create(exact_new, None, CreateVerification::Held)),
                "nord_first={nord_first}: {:?}",
                plan.ops
            );
            assert!(
                plan.ops.contains(&stange_nord_create(
                    nord_new,
                    None,
                    CreateVerification::Held
                )),
                "nord_first={nord_first}: {:?}",
                plan.ops
            );
            assert_eq!(plan.ops.len(), 2, "nord_first={nord_first}: {:?}", plan.ops);

            let review = |other: u32, new_orgnr: &str, new_name: &str, sim: &str| ReviewItem {
                kind: ReviewKind::PossibleReregistration,
                school: Some(Ref::Existing(30)),
                other_school: Some(Ref::New(other)),
                municipality: Some(Ref::Existing(20)),
                details: vec![
                    ("old_orgnr", "975270920".into()),
                    ("new_orgnr", new_orgnr.into()),
                    ("old_name", "Stange ungdomsskole".into()),
                    ("new_name", new_name.into()),
                    ("similarity", sim.into()),
                ],
            };
            assert_eq!(
                plan.reviews.len(),
                2,
                "nord_first={nord_first}: {:?}",
                plan.reviews
            );
            assert!(plan.reviews.contains(&review(
                exact_new,
                "933181995",
                "Stange ungdomsskole",
                "1.00"
            )));
            assert!(plan.reviews.contains(&review(
                nord_new,
                "933181996",
                "Stange ungdomsskole nord",
                "0.80"
            )));
        }
    }

    /// A closing school named "Fagerborg skole", for the equal-similarity tie test: the new
    /// unit matches both at 1.00, so only the predecessor's orgnr breaks the tie.
    fn fagerborg(id: u32, orgnr: &str, slug: &str) -> SchoolSnapshot<u32> {
        SchoolSnapshot {
            has_live_fau: false,
            ..school(id, 20, &unit(orgnr, "Fagerborg skole", "3413"), slug)
        }
    }

    #[test]
    fn a_tie_in_similarity_goes_to_the_lower_orgnr_predecessor() {
        let closed_a = closed(
            unit("100000002", "Fagerborg skole", "3413"),
            "F",
            "2024-08-25T01:15:10.91Z",
        );
        let closed_b = closed(
            unit("100000001", "Fagerborg skole", "3413"),
            "F",
            "2024-08-25T01:15:10.91Z",
        );
        let new_unit = unit("900000050", "Fagerborg skole", "3413");
        let cases = [
            (
                vec![
                    fagerborg(60, "100000002", "fagerborg-a"),
                    fagerborg(61, "100000001", "fagerborg-b"),
                ],
                vec![closed_a.clone(), closed_b.clone(), new_unit.clone()],
            ),
            (
                vec![
                    fagerborg(61, "100000001", "fagerborg-b"),
                    fagerborg(60, "100000002", "fagerborg-a"),
                ],
                vec![closed_b.clone(), closed_a.clone(), new_unit.clone()],
            ),
        ];
        for (schools, units) in cases {
            let plan = run(&with_schools(schools), &units, &stange());
            let successor_of = |id: u32| {
                plan.ops.iter().find_map(|op| match op {
                    SchoolOp::Close {
                        id: closed_id,
                        successor,
                        ..
                    } if *closed_id == id => Some(*successor),
                    _ => None,
                })
            };
            assert_eq!(
                successor_of(61),
                Some(Some(Ref::New(0))),
                "the lower orgnr (100000001, id 61) wins: {:?}",
                plan.ops
            );
            assert_eq!(
                successor_of(60),
                Some(None),
                "the higher orgnr gets no successor: {:?}",
                plan.ops
            );
            assert_eq!(plan.ops.len(), 3, "{:?}", plan.ops);
        }
    }

    #[test]
    fn a_similarity_of_exactly_the_threshold_still_reregisters() {
        // similarity("Oksenøya", "Oksenøya skole") is 9/15 = 0.6 exactly: proves the planner
        // uses `>=`, not `>`.
        let old = SchoolSnapshot {
            has_live_fau: false,
            ..school(70, 1, &unit("200000001", "Oksenøya", "3201"), "oksenoya")
        };
        let old_unit = closed(
            unit("200000001", "Oksenøya", "3201"),
            "F",
            "2024-08-25T01:15:10.91Z",
        );
        let new_unit = unit("200000002", "Oksenøya skole", "3201");
        let plan = run(
            &with_schools(vec![old]),
            &[old_unit, new_unit.clone()],
            &baerum(),
        );
        assert_eq!(
            plan.ops,
            [
                SchoolOp::Close {
                    id: 70,
                    reason: ClosureReason::Merged,
                    closed_on: date(2024, 8, 25),
                    successor: Some(Ref::New(0)),
                },
                SchoolOp::Create {
                    new: 0,
                    municipality: Ref::Existing(1),
                    orgnr: "200000002".into(),
                    register_name: "Oksenøya skole".into(),
                    display_name: "Oksenøya skole".into(),
                    slug: Some("oksenoya".into()),
                    verification: CreateVerification::Listed,
                    in_scope: true,
                    attributes: SchoolAttributes::from_unit(&new_unit),
                    source_changed_at: None,
                },
            ]
        );
        assert!(plan.reviews.is_empty(), "{:?}", plan.reviews);
    }
    #[test]
    fn a_school_renamed_and_moved_in_one_run() {
        // Brattvåg barneskule is renamed in NSR and its number 1580 now resolves to Haram (2).
        let brattvaag = unit("974585715", "Brattvåg barneskule", "1580");
        let snapshot = with_schools(vec![school(10, 1, &brattvaag, "brattvaag-barneskule")]);
        let state = state(&[("1508", Ref::Existing(1)), ("1580", Ref::Existing(2))]);
        let renamed = NsrUnit {
            name: "Brattvåg skule".into(),
            ..brattvaag
        };
        let plan = run(&snapshot, &[renamed], &state);
        assert_eq!(
            plan.ops,
            [
                SchoolOp::Rename {
                    id: 10,
                    register_name: "Brattvåg skule".into(),
                    display_name: Some("Brattvåg skule".into()),
                    slug: None,
                },
                SchoolOp::Move {
                    id: 10,
                    from: 1,
                    to: Ref::Existing(2),
                    slug: Some(SlugChange {
                        old: Some("brattvaag-barneskule".into()),
                        new: "brattvaag-skule".into(),
                    }),
                },
            ],
            "the Rename carries no slug; the Move re-mints from the new display name"
        );
    }

    #[test]
    fn an_fau_predecessor_wins_a_tie_with_a_non_fau_one() {
        // Two closed "Stange ungdomsskole" rows, one with an FAU; the new unit matches both at
        // 1.00. The FAU one is the match, so the unit is held and the other closes alone.
        let fau = SchoolSnapshot {
            has_live_fau: true,
            ..school(30, 20, &stange_old(), "stange-ungdomsskole")
        };
        let other_unit = NsrUnit {
            orgnr: "975270921".into(),
            ..stange_old()
        };
        let other = school(31, 20, &other_unit, "stange-ungdomsskole-stange");
        for schools in [
            vec![fau.clone(), other.clone()],
            vec![other.clone(), fau.clone()],
        ] {
            let plan = run(
                &with_schools(schools),
                &[stange_old(), other_unit.clone(), stange_new()],
                &stange(),
            );
            assert_eq!(
                plan.ops,
                [
                    SchoolOp::Close {
                        id: 31,
                        reason: ClosureReason::Merged,
                        closed_on: date(2024, 8, 25),
                        successor: None,
                    },
                    stange_create(0, None, CreateVerification::Held),
                ]
            );
            assert_eq!(plan.reviews.len(), 1, "{:?}", plan.reviews);
            assert_eq!(plan.reviews[0].kind, ReviewKind::PossibleReregistration);
            assert_eq!(plan.reviews[0].school, Some(Ref::Existing(30)));
        }
    }

    #[test]
    fn a_submitted_school_that_already_has_an_orgnr_pulls_in_no_unit() {
        let matched = SchoolSnapshot {
            orgnr: Some("900000099".into()),
            ..fornebu_submission(Verification::Verified, 1)
        };
        let fornebu = unit("900000011", "Fornebu skole", "3201");
        let plan = run(&with_schools(vec![matched]), &[fornebu], &baerum());
        assert!(plan.reviews.is_empty(), "{:?}", plan.reviews);
        assert_eq!(
            slugs(&plan),
            [(
                "900000011".into(),
                Ref::Existing(1),
                Some("fornebu-skole".into())
            )]
        );
    }

    #[test]
    fn a_curated_display_name_does_not_hide_a_reregistration() {
        let curated = "Fjellhaug u-skole (gamle)";
        assert!(similarity(curated, "Stange ungdomsskole") < REREGISTRATION_THRESHOLD);
        for has_live_fau in [false, true] {
            let row = SchoolSnapshot {
                display_name: curated.into(),
                display_name_curated: true,
                ..stange_row(has_live_fau)
            };
            let plan = run(
                &with_schools(vec![row]),
                &[stange_old(), stange_new()],
                &stange(),
            );
            if has_live_fau {
                assert_eq!(plan.ops, [stange_create(0, None, CreateVerification::Held)]);
                assert_eq!(plan.reviews[0].kind, ReviewKind::PossibleReregistration);
                assert_eq!(plan.reviews[0].details[4], ("similarity", "1.00".into()));
            } else {
                assert_eq!(
                    plan.ops[0],
                    SchoolOp::Close {
                        id: 30,
                        reason: ClosureReason::Merged,
                        closed_on: date(2024, 8, 25),
                        successor: Some(Ref::New(0)),
                    }
                );
            }
        }
    }

    #[test]
    fn a_submission_match_also_reads_the_register_name() {
        let curated = "Nesøya (FAU)";
        assert!(similarity(curated, "Fornebu skole") < SUBMISSION_MATCH_THRESHOLD);
        let submission = SchoolSnapshot {
            display_name: curated.into(),
            display_name_curated: true,
            register_name: Some("Fornebu".into()),
            ..fornebu_submission(Verification::Verified, 1)
        };
        let fornebu = unit("900000011", "Fornebu skole", "3201");
        let plan = run(&with_schools(vec![submission]), &[fornebu], &baerum());
        assert_eq!(plan.reviews.len(), 1, "{:?}", plan.reviews);
        assert_eq!(plan.reviews[0].kind, ReviewKind::PossibleSubmissionMatch);
        assert_eq!(plan.reviews[0].details[3], ("similarity", "0.57".into()));
    }

    #[test]
    fn a_listed_school_without_a_slug_gets_one_when_a_rename_makes_it_sluggable() {
        let unsluggable = unit("974552124", "Школа", "3201");
        let row = SchoolSnapshot {
            slug: None,
            ..school(10, 1, &unsluggable, "unused")
        };
        let renamed = NsrUnit {
            name: "Hosle skole".into(),
            ..unsluggable
        };
        let plan = run(&with_schools(vec![row]), &[renamed], &baerum());
        assert_eq!(
            plan.ops,
            [SchoolOp::Rename {
                id: 10,
                register_name: "Hosle skole".into(),
                display_name: Some("Hosle skole".into()),
                slug: Some(SlugChange {
                    old: None,
                    new: "hosle-skole".into(),
                }),
            }]
        );
    }

    #[test]
    fn best_breaks_a_tie_by_id_not_snapshot_order() {
        let a = SchoolSnapshot {
            id: 41,
            ..fornebu_submission(Verification::Pending, 1)
        };
        let b = fornebu_submission(Verification::Pending, 1);
        let fornebu = unit("900000011", "Fornebu skole", "3201");
        let mut schools = vec![a, b];
        let forward = run(
            &with_schools(schools.clone()),
            std::slice::from_ref(&fornebu),
            &baerum(),
        );
        schools.reverse();
        let reversed = run(&with_schools(schools), &[fornebu], &baerum());
        assert_eq!(forward, reversed);
        assert_eq!(forward.reviews[0].school, Some(Ref::Existing(40)));
    }

    #[test]
    fn reversing_the_snapshot_schools_changes_nothing() {
        let mut schools = vec![
            stange_row(false),
            fagerborg(60, "100000002", "fagerborg-a"),
            fagerborg(61, "100000001", "fagerborg-b"),
            SchoolSnapshot {
                id: 41,
                municipality_id: 20,
                ..fornebu_submission(Verification::Pending, 20)
            },
            fornebu_submission(Verification::Verified, 20),
        ];
        let units = [
            stange_old(),
            stange_new(),
            closed(
                unit("100000002", "Fagerborg skole", "3413"),
                "F",
                "2024-08-25T01:15:10.91Z",
            ),
            closed(
                unit("100000001", "Fagerborg skole", "3413"),
                "S",
                "2024-08-25T01:15:10.91Z",
            ),
            unit("900000050", "Fagerborg skole", "3413"),
            unit("900000011", "Fornebu skole", "3413"),
        ];
        let forward = run(&with_schools(schools.clone()), &units, &stange());
        schools.reverse();
        let reversed = run(&with_schools(schools), &units, &stange());
        assert_eq!(forward, reversed);
        assert!(
            !forward.reviews.is_empty() && forward.ops.len() > 3,
            "{forward:?}"
        );
    }
}
