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
