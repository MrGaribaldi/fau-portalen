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

/// The part of `details` that tells apart items sharing a kind and null references: the
/// municipality number for `unknown_municipality_number`, and the reason plus the number or
/// old code for `municipality_split_or_merge`. `(municipality_number, reason, number_or_code)`.
fn discriminator<Id>(item: &ReviewItem<Id>) -> (Option<&str>, Option<&str>, Option<&str>) {
    let detail = |key: &str| {
        item.details
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value.as_str())
    };
    match item.kind {
        ReviewKind::UnknownMunicipalityNumber => (detail("municipality_number"), None, None),
        ReviewKind::MunicipalitySplitOrMerge => (
            None,
            detail("reason"),
            detail("number").or_else(|| detail("old_code")),
        ),
        _ => (None, None, None),
    }
}

/// Writes every item no open item already covers. An item is covered when an open item has
/// the same kind, school, other school and municipality, and the same [`discriminator`].
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
        let (number, reason, code) = discriminator(item);
        let open: bool = sqlx::query_scalar(
            "select exists (
               select 1 from register_review_items
                where resolved_at is null and kind = $1
                  and school_id is not distinct from $2
                  and other_school_id is not distinct from $3
                  and municipality_id is not distinct from $4
                  and ($5::text is null or details->>'municipality_number' = $5)
                  and ($6::text is null or details->>'reason' = $6)
                  and ($7::text is null
                       or coalesce(details->>'number', details->>'old_code') = $7))",
        )
        .bind(item.kind.code())
        .bind(school)
        .bind(other_school)
        .bind(municipality)
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
