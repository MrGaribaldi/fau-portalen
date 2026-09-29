//! How long the directory fields live (groups design §4.2 as amended by Erik's D3, 28
//! September 2026; #3502).
//!
//! - **Neither field outlives the membership.** A display name is valid only while the
//!   membership is active; once it has ended (`membership_ended`: revoked, or no role
//!   assignment still running or yet to start) the name goes, exactly like the contact
//!   address. History then shows the membership as its role and year, computed at read time
//!   (`member_names`), so there is no name history to keep.
//! - Revocation clears both fields in the same statement (`revoke_membership`, held by
//!   migration 0008's two `*_only_while_current` checks). A membership whose roles simply ran
//!   out is not revoked, so [`clear_ended_profiles`], a daily sweep, clears it. Until the
//!   sweep runs, nothing shows the name: `member_names` decides "ended" at read time, and the
//!   directory lists only people with standing today.
//! - **An Article 17 erasure** removes both fields from every membership the account holds
//!   and marks them erased ([`erase_member_names`]). History then shows "Tidligere medlem",
//!   not even the role and year. It is the storage step of #3426's erasure flow, which
//!   decides who asks for it and what else goes. Database backups keep the old ciphertext
//!   until they age out; only deleting the FAU shreds it (key-service design §3.1).
//! - **Deleting the FAU** shreds its record key, and with it every name and address.

use fau_domain::time::Moment;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::error::MembershipError;
use super::profile::membership_ended;
use super::sql::{date_param, lock_tenant, ts_param, write_audit, Audit};

/// The daily sweep: clears the name and the contact address of every membership that has
/// ended, each clearing audited. Per FAU, under that FAU's lock, and re-checked under it, so
/// a role granted concurrently is never swept past. It clears at once, without the
/// account's three-month grace: that grace protects re-recognition of the account, and a
/// returning member states their name again when they accept. Returns how many memberships
/// it cleared.
pub async fn clear_ended_profiles(pool: &PgPool, at: Moment) -> Result<u64, MembershipError> {
    let today = date_param(at.today());
    let tenants: Vec<Uuid> = sqlx::query_scalar(&format!(
        "select distinct m.tenant_id from memberships m
          where (m.encrypted_display_name is not null or m.encrypted_contact_email is not null)
            and {}
          order by m.tenant_id",
        membership_ended("$1")
    ))
    .bind(&today)
    .fetch_all(pool)
    .await?;
    let mut cleared = 0;
    for tenant_id in tenants {
        let mut tx = pool.begin().await?;
        lock_tenant(&mut tx, tenant_id).await?;
        let ids: Vec<Uuid> = sqlx::query_scalar(&format!(
            "update memberships m
                set encrypted_display_name = null, encrypted_contact_email = null
              where m.tenant_id = $1
                and (m.encrypted_display_name is not null or m.encrypted_contact_email is not null)
                and {}
             returning m.id",
            membership_ended("$2")
        ))
        .bind(tenant_id)
        .bind(&today)
        .fetch_all(&mut *tx)
        .await?;
        for id in &ids {
            write_audit(
                &mut tx,
                at,
                Audit::system(
                    tenant_id,
                    "membership.profile_cleared",
                    "membership",
                    *id,
                    json!({ "cause": "membership_ended" }),
                ),
            )
            .await?;
        }
        tx.commit().await?;
        cleared += ids.len() as u64;
    }
    Ok(cleared)
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
                    encrypted_display_name = null, encrypted_contact_email = null
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
