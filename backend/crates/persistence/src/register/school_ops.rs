//! The school half of the SQL applier: one function per `SchoolOp`, as the Handover and
//! `testkit::apply` define them.

use fau_domain::register::search::search_text;
use fau_domain::register::sync::{ClosureReason, Ref, SchoolAttributes, SchoolOp, SlugChange};
use fau_domain::time::Moment;
use sqlx::PgConnection;
use uuid::Uuid;

use super::apply::{expect_one, NewIds};
use super::error::RegisterError;
use super::sql::{date_param, ts_param};

pub(super) async fn apply_school_op(
    conn: &mut PgConnection,
    op: &SchoolOp<Uuid>,
    ids: &NewIds,
    at: Moment,
) -> Result<(), RegisterError> {
    let now = ts_param(at.now());
    match op {
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
            source_changed_at,
        } => {
            // Origin `register`, no FAU link and, for a held row, no slug (the planner never
            // gives it one, and `schools_slug_needs_verification` would refuse it).
            let a = attributes;
            sqlx::query(
                "insert into schools
                   (id, municipality_id, origin, display_name, register_name, slug,
                    verification, orgnr, ownership, grade_from, grade_to, register_language,
                    website, street_address, postcode, post_town, in_scope, status,
                    source_changed_at, last_seen_in_source_at, search_text, created_at,
                    updated_at)
                 values ($1, $2, 'register', $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13,
                         $14, $15, $16, 'active', $17::timestamptz, $18::timestamptz, $19,
                         $18::timestamptz, $18::timestamptz)",
            )
            .bind(ids.school(&Ref::New(*new))?)
            .bind(ids.municipality(municipality)?)
            .bind(display_name)
            .bind(register_name)
            .bind(slug)
            .bind(verification.code())
            .bind(orgnr)
            .bind(a.ownership.map(|o| o.code()))
            .bind(a.grade_from)
            .bind(a.grade_to)
            .bind(&a.language)
            .bind(&a.website)
            .bind(&a.street_address)
            .bind(&a.postcode)
            .bind(&a.post_town)
            .bind(in_scope)
            .bind(source_changed_at.map(ts_param))
            .bind(&now)
            .bind(search_text([display_name.as_str(), register_name.as_str()]))
            .execute(&mut *conn)
            .await?;
        }
        SchoolOp::Rename {
            id,
            register_name,
            display_name,
            slug,
        } => {
            // `old: None` is a slugless school getting its first slug: no history.
            if let Some(SlugChange { old: Some(old), .. }) = slug {
                school_slug_history(conn, *id, None, old, at).await?;
            }
            let updated = sqlx::query(
                "update schools
                    set register_name = $2, display_name = coalesce($3, display_name),
                        slug = coalesce($4, slug), updated_at = $5::timestamptz
                  where id = $1",
            )
            .bind(id)
            .bind(register_name)
            .bind(display_name)
            .bind(slug.as_ref().map(|c| c.new.as_str()))
            .bind(&now)
            .execute(&mut *conn)
            .await?;
            expect_one(updated.rows_affected())?;
        }
        SchoolOp::UpdateAttributes { id, attributes } => {
            update_attributes(conn, *id, attributes, &now).await?;
        }
        SchoolOp::Move { id, from, to, slug } => {
            // The old slug's history row is keyed on the old municipality (§4.2), even when
            // the text stays the same.
            if let Some(SlugChange { old: Some(old), .. }) = slug {
                school_slug_history(conn, *id, Some(*from), old, at).await?;
            }
            let updated = sqlx::query(
                "update schools
                    set municipality_id = $2, slug = coalesce($3, slug),
                        updated_at = $4::timestamptz
                  where id = $1",
            )
            .bind(id)
            .bind(ids.municipality(to)?)
            .bind(slug.as_ref().map(|c| c.new.as_str()))
            .bind(&now)
            .execute(&mut *conn)
            .await?;
            expect_one(updated.rows_affected())?;
        }
        SchoolOp::Close {
            id,
            reason,
            closed_on,
            ..
        } => {
            // The slug goes to history and is cleared, so `schools_slug_current` lets a
            // successor take it (ADR-002 rule 3). The successor is linked after every op.
            let slug: Option<String> = sqlx::query_scalar("select slug from schools where id = $1")
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?
                .ok_or(RegisterError::UnknownRow)?;
            if let Some(slug) = slug {
                school_slug_history(conn, *id, None, &slug, at).await?;
            }
            let updated = sqlx::query(
                "update schools
                    set status = 'closed', closed_on = $2::date, closure_reason = $3,
                        slug = null, in_scope = in_scope and not $4,
                        updated_at = $5::timestamptz
                  where id = $1",
            )
            .bind(id)
            .bind(date_param(*closed_on))
            .bind(reason.code())
            .bind(*reason == ClosureReason::OutOfScope)
            .bind(&now)
            .execute(&mut *conn)
            .await?;
            expect_one(updated.rows_affected())?;
        }
    }
    Ok(())
}

async fn update_attributes(
    conn: &mut PgConnection,
    id: Uuid,
    a: &SchoolAttributes,
    now: &str,
) -> Result<(), RegisterError> {
    let updated = sqlx::query(
        "update schools
            set ownership = $2, grade_from = $3, grade_to = $4, register_language = $5,
                website = $6, street_address = $7, postcode = $8, post_town = $9,
                updated_at = $10::timestamptz
          where id = $1",
    )
    .bind(id)
    .bind(a.ownership.map(|o| o.code()))
    .bind(a.grade_from)
    .bind(a.grade_to)
    .bind(&a.language)
    .bind(&a.website)
    .bind(&a.street_address)
    .bind(&a.postcode)
    .bind(&a.post_town)
    .bind(now)
    .execute(&mut *conn)
    .await?;
    expect_one(updated.rows_affected())
}

/// Moves `slug` into history under `municipality` (the school's current one when `None`),
/// valid from the end of the school's last history row, or else from when it got a slug:
/// `verified_at` for a verified school, `created_at` for one created listed. Skipped when that
/// interval is empty, as for municipalities.
async fn school_slug_history(
    conn: &mut PgConnection,
    id: Uuid,
    municipality: Option<Uuid>,
    slug: &str,
    at: Moment,
) -> Result<(), RegisterError> {
    sqlx::query(
        "insert into school_slug_history (municipality_id, slug, school_id, valid_from, valid_until)
         select coalesce($2, s.municipality_id), $3, s.id, f.valid_from, $4::timestamptz
           from schools s
          cross join lateral (
                select coalesce((select max(h.valid_until) from school_slug_history h
                                  where h.school_id = s.id),
                                s.verified_at, s.created_at) as valid_from) f
          where s.id = $1 and f.valid_from < $4::timestamptz",
    )
    .bind(id)
    .bind(municipality)
    .bind(slug)
    .bind(ts_param(at.now()))
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `schools.successor_id`, set once every `Create` has run.
pub(super) async fn link_successor(
    conn: &mut PgConnection,
    closed: Uuid,
    successor: Uuid,
) -> Result<(), RegisterError> {
    let updated = sqlx::query("update schools set successor_id = $2 where id = $1")
        .bind(closed)
        .bind(successor)
        .execute(&mut *conn)
        .await?;
    expect_one(updated.rows_affected())
}

/// §7: a school is found by its display name and its NSR name.
pub(super) async fn refresh_school_search_text(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<(), RegisterError> {
    let (display_name, register_name): (String, Option<String>) =
        sqlx::query_as("select display_name, register_name from schools where id = $1")
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
    let text = search_text(std::iter::once(display_name.as_str()).chain(register_name.as_deref()));
    sqlx::query("update schools set search_text = $2 where id = $1")
        .bind(id)
        .bind(text)
        .execute(&mut *conn)
        .await?;
    Ok(())
}
