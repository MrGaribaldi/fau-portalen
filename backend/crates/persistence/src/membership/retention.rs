//! How long the directory fields live (groups design §4.2 as amended by Erik's D3, 28
//! September 2026; #3502).
//!
//! - **Ended, the fields are kept hidden for the member's chosen period** (#3511,
//!   docs/member-retention-design.md, Erik's M1-M3 of 29 September 2026). When a
//!   membership ends (`membership_ended`: revoked, or no role assignment still running or
//!   yet to start) its name and address stay on the row until `profile_retained_until`:
//!   the day it ended (`ended_on_for`, plan Ruling R2) plus the account's `retention_months`,
//!   exclusive. A period of none clears at once, as under D3. Nobody else sees a retained
//!   name: `member_names` and the directory decide "ended" at read time, and history shows
//!   the membership as its role and year.
//! - Revocation stamps the date, or clears for a period of none, in the same statement
//!   (`revoke_membership`, held by migration 0009's two `*_only_while_current_or_retained`
//!   checks). A membership whose roles simply ran out is not revoked, so
//!   [`clear_ended_profiles`], a daily sweep, notices it: it stamps an end not yet noticed
//!   ([`settle_profile`]) and clears a period that is over.
//! - **Group memberships end with the membership, however it ends** (Erik's M6). A
//!   revocation removes the hand-added `group_members` rows at once; a natural end is
//!   caught by the sweep, or on return by [`reopen_profile`], whichever comes first.
//! - **An Article 17 erasure** removes both fields from every membership the account holds
//!   and marks them erased ([`erase_member_names`]). History then shows "Tidligere medlem",
//!   not even the role and year. It is the storage step of #3426's erasure flow, which
//!   decides who asks for it and what else goes. Database backups keep the old ciphertext
//!   until they age out; only deleting the FAU shreds it (key-service design §3.1).
//! - **Deleting the FAU** shreds its record key, and with it every name and address.

use fau_domain::directory::history::ended_on;
use fau_domain::membership::retention::{keep_until, RetentionMonths};
use fau_domain::time::{oslo_today, Moment};
use jiff::civil::Date;
use serde_json::json;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::error::MembershipError;
use super::groups::remove_from_all_groups;
use super::names::held_roles;
use super::profile::membership_ended;
use super::sql::{date_param, from_micros, lock_tenant, parse_date, ts_param, write_audit, Audit};

/// Where a membership's name and address stand after [`settle_profile`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Settled {
    /// The membership is active; its fields, if any, are its own.
    Active,
    /// Ended, and within its period: kept, hidden, until `profile_retained_until`.
    Retained,
    /// Ended and past its period (or with a period of none): cleared just now.
    Cleared,
    /// Nothing to keep: no field, or an erasure.
    Absent,
}

/// The account's chosen period, read through the membership.
pub(crate) async fn account_retention(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
) -> Result<RetentionMonths, MembershipError> {
    let months: i32 = sqlx::query_scalar(
        "select a.retention_months from memberships m join accounts a on a.id = m.account_id
          where m.tenant_id = $1 and m.id = $2",
    )
    .bind(tenant_id)
    .bind(membership_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(MembershipError::UnknownMembership)?;
    RetentionMonths::from_months(months).ok_or_else(MembershipError::decode)
}

/// The day an ended membership ended (plan Ruling R2; final review I2), the one end date
/// every retention path uses: the latest day any of its roles stopped being held. A
/// membership that held no role at all -- revoked, or its only roles revoked, before any
/// started -- ended on the Oslo date of its latest assignment revocation (Erik, 29
/// September 2026: "use the revocation date"): `revoke_membership` revokes every live
/// assignment at the same moment, so that revocation always marks the end, and a member
/// who leaves after their last booked role was revoked had already ended then.
/// `memberships.revoked_at` is only the fallback for a membership with no assignment rows.
/// Never later than `today`; `today` only when none of those exist. A fact of the past,
/// so recomputing it never moves a period.
pub(crate) async fn ended_on_for(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
    today: Date,
) -> Result<Date, MembershipError> {
    let held = held_roles(conn, tenant_id, &[membership_id])
        .await?
        .remove(&membership_id)
        .unwrap_or_default();
    if !held.is_empty() {
        return Ok(ended_on(&held, today));
    }
    let revoked_us: Option<i64> = sqlx::query_scalar(
        "select (extract(epoch from coalesce(
                    (select max(ra.revoked_at) from role_assignments ra
                      where ra.tenant_id = m.tenant_id and ra.membership_id = m.id),
                    m.revoked_at
                )) * 1000000)::bigint
           from memberships m where m.tenant_id = $1 and m.id = $2",
    )
    .bind(tenant_id)
    .bind(membership_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(MembershipError::UnknownMembership)?;
    Ok(match revoked_us {
        Some(us) => oslo_today(from_micros(us)?).min(today),
        None => today,
    })
}

/// The date an ended membership's fields go: the stamped one, or, for an end not yet
/// noticed, `keep_until` from the day it ended ([`ended_on_for`]). `None`: clear now.
pub(crate) async fn effective_keep_until(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
    stamped: Option<&str>,
    today: Date,
) -> Result<Option<Date>, MembershipError> {
    if let Some(d) = stamped {
        let until = parse_date(d)?;
        return Ok((today < until).then_some(until));
    }
    let months = account_retention(conn, tenant_id, membership_id).await?;
    let ended = ended_on_for(conn, tenant_id, membership_id, today).await?;
    Ok(keep_until(ended, months, today))
}

/// Why an ended membership's fields are cleared now (plan Ruling R9): only an unstamped
/// end under a period of none is `membership_ended`; a stamped period, or any other
/// setting, means the period was already over, `retention_ended`.
pub(crate) async fn clearing_cause(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
    stamped: bool,
) -> Result<&'static str, MembershipError> {
    Ok(
        if stamped
            || account_retention(conn, tenant_id, membership_id).await? != RetentionMonths::None
        {
            "retention_ended"
        } else {
            "membership_ended"
        },
    )
}

/// Clears both fields and the date, audited with `cause`. The caller holds the lock.
pub(crate) async fn clear_profile(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
    at: Moment,
    cause: &'static str,
) -> Result<(), MembershipError> {
    sqlx::query(
        "update memberships
            set encrypted_display_name = null, encrypted_contact_email = null,
                profile_retained_until = null
          where tenant_id = $1 and id = $2",
    )
    .bind(tenant_id)
    .bind(membership_id)
    .execute(&mut *conn)
    .await?;
    audit_profile_cleared(conn, tenant_id, membership_id, at, cause).await
}

/// `membership.profile_cleared` {cause}, as the system (plan Ruling R9).
pub(crate) async fn audit_profile_cleared(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
    at: Moment,
    cause: &'static str,
) -> Result<(), MembershipError> {
    write_audit(
        conn,
        at,
        Audit::system(
            tenant_id,
            "membership.profile_cleared",
            "membership",
            membership_id,
            json!({ "cause": cause }),
        ),
    )
    .await
}

/// `membership.profile_retained` {until}, as the system, when an end is first stamped.
pub(crate) async fn audit_profile_retained(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
    at: Moment,
    until: Date,
) -> Result<(), MembershipError> {
    write_audit(
        conn,
        at,
        Audit::system(
            tenant_id,
            "membership.profile_retained",
            "membership",
            membership_id,
            json!({ "until": date_param(until) }),
        ),
    )
    .await
}

type SettleRow = (bool, bool, Option<String>, bool);

/// Decides, under the tenant lock the caller holds, whether an ended membership keeps its
/// fields: stamps `profile_retained_until` the first time an end is noticed (audited
/// `membership.profile_retained`), and clears once the period is over (`retention_ended`)
/// or when it had none (`membership_ended`). An active membership is left alone.
///
/// A natural end that the sweep reaches only after its period was already over (a
/// 3-month period and a sweep that has not run for 4 months) is `retention_ended`: only a
/// period of none gives `membership_ended`.
pub(crate) async fn settle_profile(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
    at: Moment,
) -> Result<Settled, MembershipError> {
    let today = at.today();
    let row: Option<SettleRow> = sqlx::query_as(&format!(
        "select m.name_erased_at is not null,
                (m.encrypted_display_name is not null or m.encrypted_contact_email is not null),
                to_char(m.profile_retained_until, 'YYYY-MM-DD'),
                {}
           from memberships m
          where m.tenant_id = $1 and m.id = $2 for update",
        membership_ended("$3")
    ))
    .bind(tenant_id)
    .bind(membership_id)
    .bind(date_param(today))
    .fetch_optional(&mut *conn)
    .await?;
    let (erased, has_fields, stamped, ended) = row.ok_or(MembershipError::UnknownMembership)?;
    if erased || !has_fields {
        return Ok(Settled::Absent);
    }
    if !ended {
        return Ok(Settled::Active);
    }
    let until =
        effective_keep_until(conn, tenant_id, membership_id, stamped.as_deref(), today).await?;
    match (until, stamped.is_some()) {
        (Some(_), true) => Ok(Settled::Retained),
        (Some(until), false) => {
            sqlx::query(
                "update memberships set profile_retained_until = $3::date
                  where tenant_id = $1 and id = $2",
            )
            .bind(tenant_id)
            .bind(membership_id)
            .bind(date_param(until))
            .execute(&mut *conn)
            .await?;
            audit_profile_retained(conn, tenant_id, membership_id, at, until).await?;
            Ok(Settled::Retained)
        }
        (None, stamped) => {
            let cause = clearing_cause(conn, tenant_id, membership_id, stamped).await?;
            clear_profile(conn, tenant_id, membership_id, at, cause).await?;
            Ok(Settled::Cleared)
        }
    }
}

/// A returner (#3511 §3, plan Ruling R4): settles the profile -- an expired period is
/// cleared, a retained one kept -- then makes the row current again in one statement
/// (`revoked_at` and `profile_retained_until` null together, as migration 0009's checks
/// require), and audits `membership.profile_restored` when something was retained. The
/// caller holds the tenant lock and adds the role in the same transaction, after this.
///
/// A membership that had ended first leaves every group it was added to by hand (Erik's
/// M6): a returner starts with no groups, and a guest reaches only the group its role
/// names. This is decided on `membership_ended`, not on what `settle_profile` returns,
/// since a row with no field (`Absent`) can still be ended and hold group rows. An active
/// member (`grant_role` reopens those too) keeps their groups.
pub(crate) async fn reopen_profile(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
    at: Moment,
) -> Result<Settled, MembershipError> {
    let ended: bool = sqlx::query_scalar(&format!(
        "select {} from memberships m where m.tenant_id = $1 and m.id = $2",
        membership_ended("$3")
    ))
    .bind(tenant_id)
    .bind(membership_id)
    .bind(date_param(at.today()))
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(MembershipError::UnknownMembership)?;
    if ended {
        remove_from_all_groups(conn, tenant_id, None, membership_id, "membership_ended", at)
            .await?;
    }
    let settled = settle_profile(conn, tenant_id, membership_id, at).await?;
    sqlx::query(
        "update memberships set revoked_at = null, profile_retained_until = null
          where tenant_id = $1 and id = $2",
    )
    .bind(tenant_id)
    .bind(membership_id)
    .execute(&mut *conn)
    .await?;
    if settled == Settled::Retained {
        write_audit(
            conn,
            at,
            Audit::system(
                tenant_id,
                "membership.profile_restored",
                "membership",
                membership_id,
                json!({}),
            ),
        )
        .await?;
    }
    Ok(settled)
}

/// The daily sweep (#3511): settles every ended membership that still holds a field and
/// whose period is not known to be running: an end not yet noticed is stamped, and an
/// expired period is cleared. It also ends the hand-added group memberships of every
/// ended membership (Erik's M6), audited as the system with cause `membership_ended`. Per
/// FAU, under that FAU's lock, taken once for both passes, each row re-decided under it,
/// so a role granted concurrently is never swept past. Returns how many profiles it
/// cleared; group removals are not counted.
///
/// Runs on a frozen FAU too: it only reduces what is kept, a system privacy action.
pub async fn clear_ended_profiles(pool: &PgPool, at: Moment) -> Result<u64, MembershipError> {
    let today = date_param(at.today());
    let candidates = |tenant_filter: &str| {
        format!(
            "select {cols} from memberships m
              where {tenant_filter}
                (m.encrypted_display_name is not null or m.encrypted_contact_email is not null)
                and (m.profile_retained_until is null or m.profile_retained_until <= $1::date)
                and {ended}",
            cols = if tenant_filter.is_empty() {
                "distinct m.tenant_id"
            } else {
                "m.id"
            },
            ended = membership_ended("$1"),
        )
    };
    let in_groups = |tenant_filter: &str| {
        format!(
            "select {cols} from memberships m
              where {tenant_filter}
                exists (select 1 from group_members gm
                         where gm.tenant_id = m.tenant_id and gm.membership_id = m.id
                           and gm.removed_at is null)
                and {ended}",
            cols = if tenant_filter.is_empty() {
                "distinct m.tenant_id"
            } else {
                "m.id"
            },
            ended = membership_ended("$1"),
        )
    };
    let mut tenants: Vec<Uuid> = sqlx::query_scalar(&candidates(""))
        .bind(&today)
        .fetch_all(pool)
        .await?;
    tenants.extend(
        sqlx::query_scalar::<_, Uuid>(&in_groups(""))
            .bind(&today)
            .fetch_all(pool)
            .await?,
    );
    tenants.sort_unstable();
    tenants.dedup();
    let mut cleared = 0;
    for tenant_id in tenants {
        let mut tx = pool.begin().await?;
        lock_tenant(&mut tx, tenant_id).await?;
        let ids: Vec<Uuid> =
            sqlx::query_scalar(&(candidates("m.tenant_id = $2 and") + " order by m.id"))
                .bind(&today)
                .bind(tenant_id)
                .fetch_all(&mut *tx)
                .await?;
        for id in ids {
            if settle_profile(&mut tx, tenant_id, id, at).await? == Settled::Cleared {
                cleared += 1;
            }
        }
        let ids: Vec<Uuid> =
            sqlx::query_scalar(&(in_groups("m.tenant_id = $2 and") + " order by m.id"))
                .bind(&today)
                .bind(tenant_id)
                .fetch_all(&mut *tx)
                .await?;
        for id in ids {
            remove_from_all_groups(&mut tx, tenant_id, None, id, "membership_ended", at).await?;
        }
        tx.commit().await?;
    }
    Ok(cleared)
}

/// The member's own setting (#3511 §4, plan Ruling R8, overridden in part by fix round
/// 1's ruling below): how long a membership of theirs is remembered after it ends.
/// `account_id` is the verified login's account (#3417's session), so a member only ever
/// sets their own.
///
/// First settles every membership the account still holds a field on, in every FAU,
/// under the setting that is about to be replaced (fix round 1, Important 1): an
/// unstamped natural end -- no revocation, and no sweep since the roles ran out, so
/// `profile_retained_until` is still null -- is stamped or cleared exactly as
/// [`settle_profile`] would do it on its own, under the *old* months. Without this step a
/// longer new setting could revive a period that had already run out under the old one,
/// and a period of none would leave an unswept natural end's fields in place instead of
/// clearing them at once. An active membership settles as `Active` and is left alone.
///
/// Then writes the new setting on the account, and recalculates every period still
/// stamped, in every FAU, from the day it ended: a longer setting extends a period that
/// is still running, and a shorter one shortens it or clears it at once if the new period
/// is already over (`retention_shortened`). **Every row reaching this step is guaranteed
/// not yet over under the old setting** -- the settle-first step above already cleared
/// any that were (as `retention_ended` or `membership_ended`), so there is no "already
/// past its stamped date" case left to check here; the two clearing causes above are the
/// only ones this function itself audits. Each moved period is audited as
/// `membership.retention_changed` by the member.
///
/// Locks: the FAU-er are locked in id order *before* the account row is written -- the
/// same order `accept_invitation` uses (tenant lock, then the account row), not the other
/// way round. As for `erase_member_names`, the tenant list is read before the locks; a
/// membership created meanwhile in another FAU is active and has no period yet, so the
/// new setting applies when it ends. A membership that ends concurrently in an FAU not on
/// that pre-read list -- for instance one accepted into after this transaction's read --
/// settles under whatever `retention_months` is visible to it at that moment, so it can
/// be stamped under the old value if that happens before this transaction commits.
///
/// Runs on a frozen FAU too, with no `require_open`, and may then extend a running period
/// (Erik, 29 September 2026, recorded in docs/planning-decisions.md's "#3511 built"): it is
/// the member's own account-wide setting over their own fields, it grants nothing, it never
/// revives a period already over, and crypto-shredding on the FAU's deletion destroys
/// everything regardless of the date.
pub async fn set_retention_months(
    pool: &PgPool,
    account_id: Uuid,
    months: RetentionMonths,
    at: Moment,
) -> Result<(), MembershipError> {
    let today = at.today();
    let mut tx = pool.begin().await?;
    let tenants: Vec<Uuid> = sqlx::query_scalar(
        "select distinct tenant_id from memberships
          where account_id = $1 and name_erased_at is null
            and (encrypted_display_name is not null or encrypted_contact_email is not null)
          order by tenant_id",
    )
    .bind(account_id)
    .fetch_all(&mut *tx)
    .await?;
    for tenant_id in &tenants {
        lock_tenant(&mut tx, *tenant_id).await?;
    }

    // Settle each such membership under the old setting, still in the accounts row,
    // before that row is overwritten below (Important 1).
    for tenant_id in &tenants {
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "select id from memberships
              where tenant_id = $1 and account_id = $2 and name_erased_at is null
                and (encrypted_display_name is not null or encrypted_contact_email is not null)
              order by id",
        )
        .bind(tenant_id)
        .bind(account_id)
        .fetch_all(&mut *tx)
        .await?;
        for id in ids {
            settle_profile(&mut tx, *tenant_id, id, at).await?;
        }
    }

    let found: Option<Uuid> =
        sqlx::query_scalar("update accounts set retention_months = $2 where id = $1 returning id")
            .bind(account_id)
            .bind(months.months())
            .fetch_optional(&mut *tx)
            .await?;
    found.ok_or(MembershipError::UnknownAccount)?;

    for tenant_id in tenants {
        let rows: Vec<(Uuid, String)> = sqlx::query_as(
            "select id, to_char(profile_retained_until, 'YYYY-MM-DD') from memberships
              where tenant_id = $1 and account_id = $2 and profile_retained_until is not null
              order by id",
        )
        .bind(tenant_id)
        .bind(account_id)
        .fetch_all(&mut *tx)
        .await?;
        for (id, stamped) in rows {
            // Once a membership has ended, its end date is a fact of the past, so
            // `ended_on_for` recomputed here is the same day as at stamping: the last held
            // role's end, else the latest role revocation (not a later leave), else the
            // membership's revocation.
            let ended = ended_on_for(&mut tx, tenant_id, id, today).await?;
            match keep_until(ended, months, today) {
                None => clear_profile(&mut tx, tenant_id, id, at, "retention_shortened").await?,
                Some(until) if date_param(until) != stamped => {
                    sqlx::query(
                        "update memberships set profile_retained_until = $3::date
                          where tenant_id = $1 and id = $2",
                    )
                    .bind(tenant_id)
                    .bind(id)
                    .bind(date_param(until))
                    .execute(&mut *tx)
                    .await?;
                    write_audit(
                        &mut tx,
                        at,
                        Audit::member(
                            tenant_id,
                            id,
                            "membership.retention_changed",
                            "membership",
                            id,
                            json!({ "until": date_param(until) }),
                        ),
                    )
                    .await?;
                }
                Some(_) => {}
            }
        }
    }
    tx.commit().await?;
    Ok(())
}

/// Erases the account's name and contact address from every FAU it belongs to, in one
/// transaction, locking those FAU-er in id order, and marks each membership erased, an
/// ended one included: the marker is what makes history show "Tidligere medlem" instead of
/// the role and year. Idempotent: an already-erased membership is left as it is. Each
/// erasure is audited, without the name. Returns how many memberships it erased.
///
/// An erased membership cannot be accepted into again (`MembershipErased`): a new name on
/// the old row would re-attach history to it. #3426 decides how an erased person rejoins.
///
/// **The tenant list is read before anything is locked** (fix round 1, controller ruling
/// Q12, item 4): the `select tenant_id from memberships where account_id = $1` above runs
/// before the loop's `lock_tenant`. A membership created for this account in a FAU not yet
/// in that list -- an acceptance racing this call -- is not covered by this erasure: it
/// keeps its name. So the caller (#3426) must make an erasure exclusive with an acceptance
/// for the same account, not just serialise per FAU as this function does.
pub async fn erase_member_names(
    pool: &PgPool,
    account_id: Uuid,
    at: Moment,
) -> Result<u64, MembershipError> {
    let mut tx = pool.begin().await?;
    let tenants: Vec<Uuid> = sqlx::query_scalar(
        "select tenant_id from memberships where account_id = $1 order by tenant_id",
    )
    .bind(account_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut erased = 0;
    for tenant_id in tenants {
        lock_tenant(&mut tx, tenant_id).await?;
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "update memberships
                set name_erased_at = $3::timestamptz,
                    encrypted_display_name = null, encrypted_contact_email = null,
                    profile_retained_until = null
              where tenant_id = $1 and account_id = $2 and name_erased_at is null
             returning id",
        )
        .bind(tenant_id)
        .bind(account_id)
        .bind(ts_param(at.now()))
        .fetch_all(&mut *tx)
        .await?;
        for id in &ids {
            write_audit(
                &mut tx,
                at,
                Audit::system(
                    tenant_id,
                    "membership.name_erased",
                    "membership",
                    *id,
                    json!({}),
                ),
            )
            .await?;
        }
        erased += ids.len() as u64;
    }
    tx.commit().await?;
    Ok(erased)
}
