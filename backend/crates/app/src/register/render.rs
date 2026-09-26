//! `--dry-run`'s output: the plan as JSON, with ops, reviews and counts, and never an
//! address. A created school's attributes are left out, and an attribute update names the
//! fields it changes rather than their values.

use fau_domain::register::sync::{
    AbortReason, MunicipalityOp, Ref, RegisterSnapshot, ReviewItem, RunKind, SchoolAttributes,
    SchoolOp, SlugChange, SyncOutcome, SyncPlan,
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

/// What a dry run prints. `stopped` is the plan the mass-change breaker aborted, printed in
/// full with the abort so the operator can see what tripped it (final review, item 4);
/// `accepted` is the abort `--accept-mass-change` overrode, printed next to the plan it let
/// through.
pub(super) fn plan_json(
    kind: RunKind,
    outcome: &SyncOutcome<Uuid>,
    snapshot: &RegisterSnapshot<Uuid>,
    stopped: Option<&SyncPlan<Uuid>>,
    accepted: Option<&AbortReason>,
) -> Value {
    let kind = match kind {
        RunKind::Seed => "seed",
        RunKind::Sync => "sync",
    };
    let ops = |out: &mut Value, plan: &SyncPlan<Uuid>| {
        out["municipality_ops"] = plan.municipality_ops.iter().map(municipality_op).collect();
        out["school_ops"] = plan
            .school_ops
            .iter()
            .map(|op| school_op(op, snapshot))
            .collect();
        out["reviews"] = plan.reviews.iter().map(review).collect();
    };
    match outcome {
        SyncOutcome::NoChange => json!({ "kind": kind, "outcome": "no_change" }),
        SyncOutcome::Abort { reason, counts } => {
            let mut out = json!({
                "kind": kind, "outcome": "abort", "abort_reason": abort_reason_text(reason),
                "counts": counts_json(counts),
            });
            if let Some(plan) = stopped {
                ops(&mut out, plan);
            }
            out
        }
        SyncOutcome::Apply(plan) => {
            let mut out = json!({
                "kind": kind, "outcome": "apply", "counts": counts_json(&plan.counts),
            });
            if let Some(reason) = accepted {
                out["mass_change_accepted"] = json!(true);
                out["abort_reason"] = json!(abort_reason_text(reason));
            }
            ops(&mut out, plan);
            out
        }
    }
}
