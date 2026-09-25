//! The SQL applier (docs/school-register-design.md §5.2-5.3; the part 3 plan's "Handover to
//! part 4", whose executable form is `fau_domain::register::sync::testkit::apply`). Every op
//! runs as its own statements inside the caller's transaction, so 0004's constraints check
//! the register after each one, exactly as the test applier's `check_invariants` does.

use std::collections::{BTreeSet, HashMap};

use fau_domain::register::search::search_text;
use fau_domain::register::source::OfficialName;
use fau_domain::register::sync::{Counts, MunicipalityOp, Ref, SchoolOp, SyncPlan};
use fau_domain::time::Moment;
use jiff::civil::Date;
use sqlx::PgConnection;
use uuid::Uuid;

use super::error::RegisterError;
use super::reviews::{write_reviews, NewReview};
use super::school_ops::{apply_school_op, link_successor, refresh_school_search_text};
use super::sql::{date_param, ts_param};

/// What an applied plan wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedCounts {
    /// The planner's own counts, as recorded on the run.
    pub counts: Counts,
    /// How many ops ran.
    pub ops: usize,
    /// The review items written, in plan order.
    pub new_reviews: Vec<NewReview>,
    /// Review items skipped because an open item already has their key.
    pub deduplicated_reviews: usize,
}

impl AppliedCounts {
    /// No op ran and every review deduplicated away: the Handover records such a run as
    /// `no_change`, like a `NoChange` outcome, so the caller rolls it back.
    pub fn wrote_nothing(&self) -> bool {
        self.ops == 0 && self.new_reviews.is_empty()
    }
}

/// The fresh UUIDv7 of every `Ref::New`, allocated before any op runs, so a `Close` can name
/// a successor that a later `Create` makes. Municipalities first, then schools, each in op
/// order: ids are time-ordered, so new rows sort after every existing one, as the test
/// applier's `max + 1` ids do.
pub(super) struct NewIds {
    municipalities: HashMap<u32, Uuid>,
    schools: HashMap<u32, Uuid>,
}

impl NewIds {
    fn allocate(plan: &SyncPlan<Uuid>) -> Self {
        let municipalities = plan
            .municipality_ops
            .iter()
            .filter_map(|op| match op {
                MunicipalityOp::Create { new, .. } => Some((*new, Uuid::now_v7())),
                _ => None,
            })
            .collect();
        let schools = plan
            .school_ops
            .iter()
            .filter_map(|op| match op {
                SchoolOp::Create { new, .. } => Some((*new, Uuid::now_v7())),
                _ => None,
            })
            .collect();
        NewIds {
            municipalities,
            schools,
        }
    }

    pub(super) fn municipality(&self, r: &Ref<Uuid>) -> Result<Uuid, RegisterError> {
        resolve(&self.municipalities, r)
    }

    pub(super) fn school(&self, r: &Ref<Uuid>) -> Result<Uuid, RegisterError> {
        resolve(&self.schools, r)
    }
}

fn resolve(new: &HashMap<u32, Uuid>, r: &Ref<Uuid>) -> Result<Uuid, RegisterError> {
    match r {
        Ref::Existing(id) => Ok(*id),
        Ref::New(n) => new.get(n).copied().ok_or(RegisterError::UnknownRow),
    }
}

/// Applies `plan` inside the caller's transaction, as the Handover orders it: every
/// municipality op, then every school op, each in plan order; then the successor links, since
/// a `Close` can name a school a later `Create` makes; then the review items, deduplicated
/// against open ones.
pub async fn apply_plan(
    conn: &mut PgConnection,
    plan: &SyncPlan<Uuid>,
    at: Moment,
) -> Result<AppliedCounts, RegisterError> {
    let ids = NewIds::allocate(plan);

    let mut touched = BTreeSet::new();
    for op in &plan.municipality_ops {
        touched.insert(apply_municipality_op(conn, op, &ids, at).await?);
    }
    for id in touched {
        refresh_municipality_search_text(conn, id).await?;
    }

    let mut renamed = BTreeSet::new();
    let mut successors = Vec::new();
    for op in &plan.school_ops {
        match op {
            SchoolOp::Rename { id, .. } => {
                renamed.insert(*id);
            }
            SchoolOp::Close {
                id,
                successor: Some(successor),
                ..
            } => successors.push((*id, ids.school(successor)?)),
            _ => {}
        }
        apply_school_op(conn, op, &ids, at).await?;
    }
    for (closed, successor) in successors {
        link_successor(conn, closed, successor).await?;
    }
    for id in renamed {
        refresh_school_search_text(conn, id).await?;
    }

    let (new_reviews, deduplicated_reviews) = write_reviews(conn, &plan.reviews, &ids, at).await?;
    Ok(AppliedCounts {
        counts: plan.counts,
        ops: plan.municipality_ops.len() + plan.school_ops.len(),
        new_reviews,
        deduplicated_reviews,
    })
}

/// One municipality op. Returns the id it touched.
async fn apply_municipality_op(
    conn: &mut PgConnection,
    op: &MunicipalityOp<Uuid>,
    ids: &NewIds,
    at: Moment,
) -> Result<Uuid, RegisterError> {
    let now = ts_param(at.now());
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
        } => {
            let id = ids.municipality(&Ref::New(*new))?;
            sqlx::query(
                "insert into municipalities
                   (id, name, official_name, county_number, county_name, slug, status, source,
                    search_text, created_at, updated_at)
                 values ($1, $2, $3, $4, $5, $6, 'active', $7, '', $8::timestamptz,
                         $8::timestamptz)",
            )
            .bind(id)
            .bind(name)
            .bind(official_name)
            .bind(county_number)
            .bind(county_name)
            .bind(slug)
            .bind(source.code())
            .bind(&now)
            .execute(&mut *conn)
            .await?;
            // The number's true start is unknown: it is valid from the day we first saw it.
            sqlx::query(
                "insert into municipality_numbers (municipality_id, number, valid_from)
                 values ($1, $2, $3::date)",
            )
            .bind(id)
            .bind(number)
            .bind(date_param(at.today()))
            .execute(&mut *conn)
            .await?;
            insert_names(conn, id, names).await?;
            Ok(id)
        }
        MunicipalityOp::Renumber {
            id,
            from,
            to,
            valid_from,
            name,
            old_slug,
            new_slug,
        } => {
            // A renumber always moves the slug: the new one starts with the new number.
            municipality_slug_history(conn, *id, old_slug, at).await?;
            let updated = sqlx::query(
                "update municipalities
                    set slug = $2, name = coalesce($3, name), updated_at = $4::timestamptz
                  where id = $1",
            )
            .bind(id)
            .bind(new_slug)
            .bind(name)
            .bind(&now)
            .execute(&mut *conn)
            .await?;
            expect_one(updated.rows_affected())?;
            renumber(conn, *id, from, to, *valid_from).await?;
            Ok(*id)
        }
        MunicipalityOp::Rename {
            id,
            name,
            old_slug,
            new_slug,
        } => {
            // A case-only change keeps the slug and writes no history (§6).
            if old_slug != new_slug {
                municipality_slug_history(conn, *id, old_slug, at).await?;
            }
            let updated = sqlx::query(
                "update municipalities set name = $2, slug = $3, updated_at = $4::timestamptz
                  where id = $1",
            )
            .bind(id)
            .bind(name)
            .bind(new_slug)
            .bind(&now)
            .execute(&mut *conn)
            .await?;
            expect_one(updated.rows_affected())?;
            Ok(*id)
        }
        MunicipalityOp::UpdateDetails {
            id,
            official_name,
            county_number,
            county_name,
            names,
        } => {
            let updated = sqlx::query(
                "update municipalities
                    set official_name = $2, county_number = $3, county_name = $4,
                        updated_at = $5::timestamptz
                  where id = $1",
            )
            .bind(id)
            .bind(official_name)
            .bind(county_number)
            .bind(county_name)
            .bind(&now)
            .execute(&mut *conn)
            .await?;
            expect_one(updated.rows_affected())?;
            // Replaced whole: Kartverket may drop a name, and fau_register may delete (0004).
            sqlx::query("delete from municipality_names where municipality_id = $1")
                .bind(id)
                .execute(&mut *conn)
                .await?;
            insert_names(conn, *id, names).await?;
            Ok(*id)
        }
    }
}

async fn insert_names(
    conn: &mut PgConnection,
    id: Uuid,
    names: &[OfficialName],
) -> Result<(), RegisterError> {
    for n in names {
        sqlx::query(
            "insert into municipality_names (municipality_id, name, language, priority)
             values ($1, $2, $3, $4)",
        )
        .bind(id)
        .bind(&n.name)
        .bind(&n.language)
        .bind(i32::from(n.priority))
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// Closes the current number at `valid_from` and opens the new one from the same date. A
/// number first seen on or after the change's date (a seed on the day of a renumber) cannot
/// close before it opened, so it closes the day after it opened instead.
async fn renumber(
    conn: &mut PgConnection,
    id: Uuid,
    from: &str,
    to: &str,
    valid_from: Date,
) -> Result<(), RegisterError> {
    let opened = sqlx::query(
        "with closed as (
           update municipality_numbers
              set valid_until = greatest($3::date, valid_from + 1)
            where municipality_id = $1 and number = $2 and valid_until is null
            returning valid_until)
         insert into municipality_numbers (municipality_id, number, valid_from)
         select $1, $4, valid_until from closed",
    )
    .bind(id)
    .bind(from)
    .bind(date_param(valid_from))
    .bind(to)
    .execute(&mut *conn)
    .await?;
    expect_one(opened.rows_affected())
}

/// Moves `old_slug` into history, valid from the end of the municipality's last history row
/// or else from its creation. Skipped when that interval is empty: nobody saw the slug live,
/// e.g. the middle slug of two renumbers in one run (the Handover's guard).
async fn municipality_slug_history(
    conn: &mut PgConnection,
    id: Uuid,
    old_slug: &str,
    at: Moment,
) -> Result<(), RegisterError> {
    sqlx::query(
        "insert into municipality_slug_history (slug, municipality_id, valid_from, valid_until)
         select $1, m.id,
                coalesce((select max(h.valid_until) from municipality_slug_history h
                           where h.municipality_id = m.id),
                         m.created_at),
                $3::timestamptz
           from municipalities m
          where m.id = $2
            and coalesce((select max(h.valid_until) from municipality_slug_history h
                           where h.municipality_id = m.id),
                         m.created_at) < $3::timestamptz",
    )
    .bind(old_slug)
    .bind(id)
    .bind(ts_param(at.now()))
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// §7: every name a municipality answers to, the Norwegian one first, folded for search.
async fn refresh_municipality_search_text(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<(), RegisterError> {
    let name: String = sqlx::query_scalar("select name from municipalities where id = $1")
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
    let names: Vec<String> = sqlx::query_scalar(
        "select name from municipality_names where municipality_id = $1
          order by priority, language, name",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?;
    let text = search_text(std::iter::once(name.as_str()).chain(names.iter().map(String::as_str)));
    sqlx::query("update municipalities set search_text = $2 where id = $1")
        .bind(id)
        .bind(text)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// An op names exactly one row; anything else is a plan against a register it did not read.
pub(super) fn expect_one(rows: u64) -> Result<(), RegisterError> {
    if rows == 1 {
        Ok(())
    } else {
        Err(RegisterError::UnknownRow)
    }
}
