//! Names for rendering history (groups design §4.2 as amended by Erik's D3, 28 September
//! 2026; #3502): an author, a minute-taker, an audit line. Every renderer reads them from
//! here, so the retention rules hold everywhere at once:
//! - an active membership has its name;
//! - an ended one (`membership_ended`, decided now, at read time, so a sweep that has not
//!   run yet never lets a name through) is shown as a role and its years, from the roles it
//!   actually held (`fau_domain::directory::history::role_label`);
//! - an Article 17 erasure is "Tidligere medlem", with neither name nor role;
//! - an active membership with an earlier period is `Returned`: its name for the current
//!   period only (#3511, M1).

use std::collections::HashMap;

use fau_crypto::Ciphertext;
use fau_domain::authz::Action;
use fau_domain::directory::history::{active_period, ActivePeriod, HeldRole};
use fau_domain::membership::vocabulary::CapabilityClass;
use fau_domain::time::{oslo_today, Moment};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::authz::{authorize, denied, read_transaction, Resource, Viewer};
use super::error::MembershipError;
use super::profile::membership_ended;
use super::sql::{date_param, from_micros, parse_date};

/// A membership as history shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberName {
    /// An active membership's name, encrypted under the record key with `DISPLAY_NAME_AAD`
    /// and the membership's id.
    Named(Ciphertext),
    /// An active membership created before migration 0008, which has no name.
    Unnamed,
    /// An active membership that had an earlier period (#3511, Erik's M1): its name,
    /// encrypted as for `Named`, for events on or after `period.since`, and role and year
    /// for anything earlier (`period.label_on`). Others never see the returner's name on
    /// their old contributions. Never produced by the directory.
    ///
    /// **Rendering contract** (final review I4, binding on #3417 and every renderer): the
    /// server resolves each event to exactly one of the decrypted name or the role label
    /// from `period.label_on(event_date)`, and sends only that. For an event shown by role
    /// it never sends this variant, `period.earlier`, or the author's membership id to a
    /// viewer: together they link the returner to their earlier contributions, which M1
    /// forbids.
    Returned {
        name: Ciphertext,
        period: ActivePeriod,
    },
    /// The membership has ended (D3): the roles it held, never empty, ordered by start. The
    /// renderer picks one for the event's date with `role_label` and renders it through the
    /// catalogue ("Leder 2025–2026").
    Ended(Vec<HeldRole>),
    /// An Article 17 erasure, or an ended membership that never held a role. Bokmål source
    /// string for the catalogue (#3439): "Tidligere medlem". Never listed by the directory.
    Former,
}

type MembershipRow = (Uuid, Option<Vec<u8>>, bool, bool);
type AssignmentRow = (Uuid, String, String, String, String, Option<i64>);

/// How `membership_ids` in the viewer's FAU appear in history (see [`MemberName`]). Ids not
/// in this FAU are left out, and a repeat comes back once. Members and admins only: a
/// guest's history rendering is decided with the first feature that shows a guest history
/// (#3503), so a guest gets `NotAuthorized`.
pub async fn member_names(
    pool: &PgPool,
    viewer: Viewer,
    membership_ids: &[Uuid],
    at: Moment,
) -> Result<Vec<(Uuid, MemberName)>, MembershipError> {
    let mut tx = read_transaction(pool).await?;
    authorize(&mut tx, viewer, Resource::Fau, Action::Read, at)
        .await?
        .map_err(|d| denied(d, MembershipError::NotAuthorized))?;
    let rows: Vec<MembershipRow> = sqlx::query_as(&format!(
        "select m.id, m.encrypted_display_name, m.name_erased_at is not null, {}
           from memberships m
          where m.tenant_id = $1 and m.id = any($2)",
        membership_ended("$3")
    ))
    .bind(viewer.tenant_id)
    .bind(membership_ids)
    .bind(date_param(at.today()))
    .fetch_all(&mut *tx)
    .await?;
    let wanted: Vec<Uuid> = rows
        .iter()
        .filter(|(_, _, erased, _)| !erased)
        .map(|(id, ..)| *id)
        .collect();
    let mut held = held_roles(&mut tx, viewer.tenant_id, &wanted).await?;
    tx.commit().await?;

    let mut out: Vec<(Uuid, MemberName)> = Vec::with_capacity(rows.len());
    for id in membership_ids {
        if out.iter().any(|(m, _)| m == id) {
            continue;
        }
        let Some((_, name, erased, ended)) = rows.iter().find(|r| r.0 == *id) else {
            continue;
        };
        let shown = match (erased, ended) {
            (true, _) => MemberName::Former,
            (false, true) => match held.remove(id) {
                Some(roles) => MemberName::Ended(roles),
                None => MemberName::Former,
            },
            (false, false) => {
                let period = active_period(held.get(id).map_or(&[][..], Vec::as_slice), at.today());
                match name {
                    Some(n) if !period.earlier.is_empty() => MemberName::Returned {
                        name: Ciphertext::from_stored(n.clone()),
                        period,
                    },
                    Some(n) => MemberName::Named(Ciphertext::from_stored(n.clone())),
                    None => MemberName::Unnamed,
                }
            }
        };
        out.push((*id, shown));
    }
    Ok(out)
}

/// The roles each membership in `ids` actually held, over the days held (an early
/// revocation ends a span on its Oslo date), non-empty spans only, ordered by start.
/// Shared by history (`member_names`) and retention (`settle_profile`, `revoke_membership`),
/// so "when did it end" and "what did it hold" read the same facts.
pub(crate) async fn held_roles(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<HeldRole>>, MembershipError> {
    let assignments: Vec<AssignmentRow> = sqlx::query_as(
        "select ra.membership_id, r.name, r.capability_class,
                to_char(ra.starts_on, 'YYYY-MM-DD'), to_char(ra.ends_on_exclusive, 'YYYY-MM-DD'),
                (extract(epoch from ra.revoked_at) * 1000000)::bigint
           from role_assignments ra
           join roles r on r.tenant_id = ra.tenant_id and r.id = ra.role_id
          where ra.tenant_id = $1 and ra.membership_id = any($2)
          order by ra.membership_id, ra.starts_on, ra.id",
    )
    .bind(tenant_id)
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    let mut held: HashMap<Uuid, Vec<HeldRole>> = HashMap::new();
    for (membership, name, class, starts, ends, revoked_us) in assignments {
        let from = parse_date(&starts)?;
        let mut until = parse_date(&ends)?;
        if let Some(us) = revoked_us {
            until = until.min(oslo_today(from_micros(us)?));
        }
        if from < until {
            held.entry(membership).or_default().push(HeldRole {
                name,
                class: CapabilityClass::from_code(&class).ok_or_else(MembershipError::decode)?,
                from,
                until,
            });
        }
    }
    Ok(held)
}
