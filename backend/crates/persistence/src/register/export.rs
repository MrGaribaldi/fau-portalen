//! `fau register export` (docs/school-register-design.md §9): the register's school identity
//! for #3431, which keys every prospect row on our school UUID and never edits school data.
//! Only pickable schools (§7) are listed: active, listed or verified, and effectively in
//! scope. Pending, held, rejected, closed and out-of-scope rows are never outreach targets.

use sqlx::PgConnection;
use uuid::Uuid;

use super::error::RegisterError;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct ExportRow {
    pub school_id: Uuid,
    pub orgnr: Option<String>,
    pub municipality_number: String,
    pub display_name: String,
    /// `/fau/<municipality slug>/<school slug>`, or `/s/<uuid>` for a school without a slug.
    pub path: String,
    /// The linked Brreg FAU (§4.6). Empty until part 5's matcher links one.
    pub fau_orgnr: Option<String>,
}

/// Every pickable school, by municipality number and then id: a stable order that never
/// sorts by a name (§7, #3439).
pub async fn export_rows(conn: &mut PgConnection) -> Result<Vec<ExportRow>, RegisterError> {
    Ok(sqlx::query_as(
        "select s.id as school_id, s.orgnr, n.number as municipality_number, s.display_name,
                case when s.slug is null then '/s/' || s.id::text
                     else '/fau/' || m.slug || '/' || s.slug end as path,
                (select l.fau_orgnr from school_fau_links l
                  where l.school_id = s.id and l.state = 'linked') as fau_orgnr
           from schools s
           join municipalities m on m.id = s.municipality_id
           join municipality_numbers n on n.municipality_id = m.id and n.valid_until is null
          where s.status = 'active' and s.verification in ('listed', 'verified')
            and coalesce(s.scope_override, s.in_scope)
          order by n.number, s.id",
    )
    .fetch_all(&mut *conn)
    .await?)
}
