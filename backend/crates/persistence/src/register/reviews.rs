//! Review items (docs/school-register-design.md §4.3), written with deduplication against
//! open ones. The planner cannot see open items, so without this each weekly run would raise
//! every unresolved item again (the Handover's "Reviews").

use fau_domain::register::sync::{ReviewItem, ReviewKind};
use fau_domain::time::Moment;
use serde_json::{Map, Value};
use sqlx::PgConnection;
use uuid::Uuid;

use super::apply::NewIds;
use super::error::RegisterError;
use super::sql::ts_param;

/// A review item this run wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewReview {
    pub id: Uuid,
    pub kind: ReviewKind,
}

/// The part of `details` that tells apart items sharing a kind and null references (the
/// Handover's "Reviews"). For a discriminated kind, an absent value is a value of its own:
/// compared with `is not distinct from`, so it never matches a present one (final review,
/// item 6). A kind without a discriminator compares nothing here.
enum Discriminator<'a> {
    None,
    /// `unknown_municipality_number`: the municipality number.
    MunicipalityNumber(Option<&'a str>),
    /// `municipality_split_or_merge`: the reason, and the number or else the old code.
    ReasonAndCode(Option<&'a str>, Option<&'a str>),
}

fn discriminator<Id>(item: &ReviewItem<Id>) -> Discriminator<'_> {
    let detail = |key: &str| {
        item.details
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value.as_str())
    };
    match item.kind {
        ReviewKind::UnknownMunicipalityNumber => {
            Discriminator::MunicipalityNumber(detail("municipality_number"))
        }
        ReviewKind::MunicipalitySplitOrMerge => Discriminator::ReasonAndCode(
            detail("reason"),
            detail("number").or_else(|| detail("old_code")),
        ),
        _ => Discriminator::None,
    }
}

/// Writes every item no open item already covers. An item is covered when an open item has
/// the same kind, school, other school and municipality, and the same [`discriminator`],
/// absent values included.
/// Returns the items written, in plan order, and how many were skipped.
pub(super) async fn write_reviews(
    conn: &mut PgConnection,
    items: &[ReviewItem<Uuid>],
    ids: &NewIds,
    at: Moment,
) -> Result<(Vec<NewReview>, usize), RegisterError> {
    let mut written = Vec::new();
    let mut skipped = 0;
    for item in items {
        let school = item.school.as_ref().map(|r| ids.school(r)).transpose()?;
        let other_school = item
            .other_school
            .as_ref()
            .map(|r| ids.school(r))
            .transpose()?;
        let municipality = item
            .municipality
            .as_ref()
            .map(|r| ids.municipality(r))
            .transpose()?;
        // `$5` says which discriminator applies; a kind without one skips both comparisons.
        let (which, number, reason, code) = match discriminator(item) {
            Discriminator::None => ("none", None, None, None),
            Discriminator::MunicipalityNumber(number) => ("number", number, None, None),
            Discriminator::ReasonAndCode(reason, code) => ("reason_and_code", None, reason, code),
        };
        let open: bool = sqlx::query_scalar(
            "select exists (
               select 1 from register_review_items
                where resolved_at is null and kind = $1
                  and school_id is not distinct from $2
                  and other_school_id is not distinct from $3
                  and municipality_id is not distinct from $4
                  and ($5 <> 'number'
                       or details->>'municipality_number' is not distinct from $6::text)
                  and ($5 <> 'reason_and_code'
                       or (details->>'reason' is not distinct from $7::text
                           and coalesce(details->>'number', details->>'old_code')
                               is not distinct from $8::text)))",
        )
        .bind(item.kind.code())
        .bind(school)
        .bind(other_school)
        .bind(municipality)
        .bind(which)
        .bind(number)
        .bind(reason)
        .bind(code)
        .fetch_one(&mut *conn)
        .await?;
        if open {
            skipped += 1;
            continue;
        }
        let details: Map<String, Value> = item
            .details
            .iter()
            .map(|(k, v)| ((*k).to_owned(), Value::from(v.as_str())))
            .collect();
        let id = Uuid::now_v7();
        sqlx::query(
            "insert into register_review_items
               (id, kind, school_id, other_school_id, municipality_id, details, created_at)
             values ($1, $2, $3, $4, $5, $6::jsonb, $7::timestamptz)",
        )
        .bind(id)
        .bind(item.kind.code())
        .bind(school)
        .bind(other_school)
        .bind(municipality)
        .bind(Value::Object(details).to_string())
        .bind(ts_param(at.now()))
        .execute(&mut *conn)
        .await?;
        written.push(NewReview {
            id,
            kind: item.kind,
        });
    }
    Ok((written, skipped))
}
