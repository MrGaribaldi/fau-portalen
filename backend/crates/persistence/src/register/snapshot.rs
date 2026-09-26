//! The register as the planner sees it (docs/school-register-design.md §5.2; the part 3
//! handover's "Building the snapshot"). Every query orders by id, so a plan and its diff are
//! reproducible, and every attribute reads back exactly as stored: an empty string stays an
//! empty string and a null stays `None`, or every run would report `UpdateAttributes`.

use std::collections::BTreeMap;

use fau_domain::register::source::OfficialName;
use fau_domain::register::sync::{
    MunicipalitySnapshot, RegisterSnapshot, SchoolAttributes, SchoolSnapshot,
};
use sqlx::PgConnection;
use uuid::Uuid;

use super::error::RegisterError;
use super::sql;

#[derive(sqlx::FromRow)]
struct MunicipalityRow {
    id: Uuid,
    number: Option<String>,
    name: String,
    official_name: Option<String>,
    county_number: String,
    county_name: String,
    slug: String,
    status: String,
    source: String,
}

#[derive(sqlx::FromRow)]
struct SchoolRow {
    id: Uuid,
    municipality_id: Uuid,
    origin: String,
    orgnr: Option<String>,
    register_name: Option<String>,
    display_name: String,
    display_name_curated: bool,
    slug: Option<String>,
    verification: String,
    status: String,
    in_scope: bool,
    scope_override: Option<bool>,
    has_live_fau: bool,
    ownership: Option<String>,
    grade_from: Option<i16>,
    grade_to: Option<i16>,
    register_language: Option<String>,
    website: Option<String>,
    street_address: Option<String>,
    postcode: Option<String>,
    post_town: Option<String>,
}

/// Reads the whole register. Call it inside a REPEATABLE READ transaction, so its six queries
/// see one snapshot.
///
/// A municipality's `number` is its current one (`valid_until is null`). A dissolved
/// municipality has none, so it carries its most recent number; the planner never resolves a
/// dissolved row's number.
pub async fn load_snapshot(
    conn: &mut PgConnection,
) -> Result<RegisterSnapshot<Uuid>, RegisterError> {
    let municipality_rows: Vec<MunicipalityRow> = sqlx::query_as(
        "select m.id,
                (select n.number from municipality_numbers n
                  where n.municipality_id = m.id
                  order by n.valid_until is null desc, n.valid_from desc
                  limit 1) as number,
                m.name, m.official_name, m.county_number, m.county_name, m.slug, m.status,
                m.source
           from municipalities m
          order by m.id",
    )
    .fetch_all(&mut *conn)
    .await?;

    let name_rows: Vec<(Uuid, String, String, i32)> = sqlx::query_as(
        "select municipality_id, name, language, priority from municipality_names
          order by municipality_id, priority, language, name",
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut names: BTreeMap<Uuid, Vec<OfficialName>> = BTreeMap::new();
    for (id, name, language, priority) in name_rows {
        names.entry(id).or_default().push(OfficialName {
            name,
            language,
            priority: u8::try_from(priority).map_err(|_| RegisterError::Decode)?,
        });
    }

    let mut municipalities = Vec::with_capacity(municipality_rows.len());
    for r in municipality_rows {
        municipalities.push(MunicipalitySnapshot {
            id: r.id,
            number: r.number.ok_or(RegisterError::Decode)?,
            name: r.name,
            official_name: r.official_name,
            county_number: r.county_number,
            county_name: r.county_name,
            slug: r.slug,
            status: sql::municipality_status(&r.status)?,
            source: sql::municipality_source(&r.source)?,
            names: names.remove(&r.id).unwrap_or_default(),
        });
    }

    let school_rows: Vec<SchoolRow> = sqlx::query_as(
        "select s.id, s.municipality_id, s.origin, s.orgnr, s.register_name, s.display_name,
                s.display_name_curated, s.slug, s.verification, s.status, s.in_scope,
                s.scope_override,
                exists (select 1 from tenants t
                         where t.school_id = s.id and t.status in ('pending', 'active'))
                  as has_live_fau,
                s.ownership, s.grade_from, s.grade_to, s.register_language, s.website,
                s.street_address, s.postcode, s.post_town
           from schools s
          order by s.id",
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut schools = Vec::with_capacity(school_rows.len());
    for r in school_rows {
        schools.push(SchoolSnapshot {
            id: r.id,
            municipality_id: r.municipality_id,
            origin: sql::origin(&r.origin)?,
            orgnr: r.orgnr,
            register_name: r.register_name,
            display_name: r.display_name,
            display_name_curated: r.display_name_curated,
            slug: r.slug,
            verification: sql::verification(&r.verification)?,
            status: sql::school_status(&r.status)?,
            in_scope: r.in_scope,
            scope_override: r.scope_override,
            has_live_fau: r.has_live_fau,
            attributes: SchoolAttributes {
                ownership: sql::ownership(r.ownership.as_deref())?,
                grade_from: r.grade_from,
                grade_to: r.grade_to,
                language: r.register_language,
                website: r.website,
                street_address: r.street_address,
                postcode: r.postcode,
                post_town: r.post_town,
            },
        });
    }

    let school_orgnr_history: Vec<(String, Uuid)> = sqlx::query_as(
        "select orgnr, school_id from school_orgnr_history order by school_id, orgnr",
    )
    .fetch_all(&mut *conn)
    .await?;
    let school_slug_history: Vec<(Uuid, String, Uuid)> = sqlx::query_as(
        "select municipality_id, slug, school_id from school_slug_history
          order by school_id, valid_from, municipality_id, slug",
    )
    .fetch_all(&mut *conn)
    .await?;
    let municipality_slug_history: Vec<(String, Uuid)> = sqlx::query_as(
        "select slug, municipality_id from municipality_slug_history
          order by municipality_id, valid_from, slug",
    )
    .fetch_all(&mut *conn)
    .await?;

    Ok(RegisterSnapshot {
        municipalities,
        schools,
        school_orgnr_history,
        school_slug_history,
        municipality_slug_history,
    })
}

/// No municipality and no school: what `--seed` requires and a plain sync refuses.
pub async fn register_is_empty(conn: &mut PgConnection) -> Result<bool, RegisterError> {
    Ok(sqlx::query_scalar(
        "select not exists (select 1 from municipalities) and not exists (select 1 from schools)",
    )
    .fetch_one(&mut *conn)
    .await?)
}
