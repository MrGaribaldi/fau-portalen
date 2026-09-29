//! How long the directory fields live (groups design §4.2 as amended by Erik's D3, 28
//! September 2026; #3502).
//!
//! - **Ended, the fields are kept hidden for the member's chosen period** (#3511,
//!   docs/member-retention-design.md, Erik's M1-M3 of 29 September 2026). When a
//!   membership ends (`membership_ended`: revoked, or no role assignment still running or
//!   yet to start) its name and address stay on the row until `profile_retained_until`:
//!   the day it ended (`ended_on`, plan Ruling R2) plus the account's `retention_months`,
//!   exclusive. A period of none clears at once, as under D3. Nobody else sees a retained
//!   name: `member_names` and the directory decide "ended" at read time, and history shows
//!   the membership as its role and year.
//! - Revocation stamps the date, or clears for a period of none, in the same statement
//!   (`revoke_membership`, held by migration 0009's two `*_only_while_current_or_retained`
//!   checks). A membership whose roles simply ran out is not revoked, so
//!   [`clear_ended_profiles`], a daily sweep, notices it: it stamps an end not yet noticed
//!   ([`settle_profile`]) and clears a period that is over.
//! - **An Article 17 erasure** removes both fields from every membership the account holds
//!   and marks them erased ([`erase_member_names`]). History then shows "Tidligere medlem",
//!   not even the role and year. It is the storage step of #3426's erasure flow, which
//!   decides who asks for it and what else goes. Database backups keep the old ciphertext
//!   until they age out; only deleting the FAU shreds it (key-service design §3.1).
//! - **Deleting the FAU** shreds its record key, and with it every name and address.

use fau_domain::directory::history::ended_on;
use fau_domain::membership::retention::{keep_until, RetentionMonths};
use fau_domain::time::Moment;
use jiff::civil::Date;
use serde_json::json;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::error::MembershipError;
use super::names::held_roles;
use super::profile::membership_ended;
use super::sql::{date_param, lock_tenant, parse_date, ts_param, write_audit, Audit};

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

/// The date an ended membership's fields go: the stamped one, or, for an end not yet
/// noticed, `keep_until` from the day it ended (plan Ruling R2). `None`: clear now.
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
    let held = held_roles(conn, tenant_id, &[membership_id])
        .await?
        .remove(&membership_id)
        .unwrap_or_default();
    Ok(keep_until(ended_on(&held, today), months, today))
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
            .await?;
            Ok(Settled::Retained)
        }
        (None, stamped) => {
            let cause = if stamped
                || account_retention(conn, tenant_id, membership_id).await? != RetentionMonths::None
            {
                "retention_ended"
            } else {
                "membership_ended"
            };
            clear_profile(conn, tenant_id, membership_id, at, cause).await?;
            Ok(Settled::Cleared)
        }
    }
}

/// A returner (#3511 §3, plan Ruling R4): settles the profile -- an expired period is
/// cleared, a retained one kept -- then makes the row current again in one statement
/// (`revoked_at` and `profile_retained_until` null together, as migration 0009's checks
/// require), and audits `membership.profile_restored` when something was retained. The
/// caller holds the tenant lock and adds the role in the same transaction.
pub(crate) async fn reopen_profile(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
    at: Moment,
) -> Result<Settled, MembershipError> {
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
/// expired period is cleared. Per FAU, under that FAU's lock, each row re-decided under
/// it, so a role granted concurrently is never swept past. Returns how many it cleared.
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
    let tenants: Vec<Uuid> = sqlx::query_scalar(&(candidates("") + " order by m.tenant_id"))
        .bind(&today)
        .fetch_all(pool)
        .await?;
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
            // Once a membership has ended, no role span changes, so `ended_on` recomputed
            // here from held roles is the same day as at stamping.
            let held = held_roles(&mut tx, tenant_id, &[id])
                .await?
                .remove(&id)
                .unwrap_or_default();
            match keep_until(ended_on(&held, today), months, today) {
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
