//! A member's directory fields (groups design §4.1; #3502): the display name, required when
//! an invitation is accepted or an FAU activated, and an optional contact address.
//!
//! **Both are content.** The caller encrypts each under the FAU's record key
//! (`fau_crypto::Unit::Record`) with `Aad::new(tenant_id, DISPLAY_NAME_AAD.0,
//! DISPLAY_NAME_AAD.1, membership_id)` or the same with [`CONTACT_EMAIL_AAD`]. This module
//! stores and returns the ciphertext only, and never writes a name or an address to audit,
//! to a NOTIFY or to a log.
//!
//! **The contact address is the member's own statement.** It is not verified, and no system
//! mail is ever sent to it: login, invitations and recovery keep using the account address.
//! So only the member sets it. An admin may correct a name, not an address.
//!
//! **Neither field outlives the membership** (Erik's D3, 28 September 2026). Once a
//! membership has ended ([`membership_ended`]) it holds no name, so no edit gives it one;
//! history shows it as its role and year instead (`member_names`).
//!
//! **Order** (as in the rest of `membership`): tenant state, then authority, then row state.

use std::ops::RangeInclusive;

use fau_crypto::Ciphertext;
use fau_domain::membership::access::Capability;
use fau_domain::time::Moment;
use serde_json::json;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::access::membership_access;
use super::error::MembershipError;
use super::sql::{date_param, is_admin_today, lock_tenant, require_open, write_audit, Audit};

/// The associated data a display name is encrypted with, with the tenant and the
/// membership's id.
pub const DISPLAY_NAME_AAD: (&str, &str) = ("memberships", "encrypted_display_name");

/// The associated data a contact address is encrypted with, with the tenant and the
/// membership's id.
pub const CONTACT_EMAIL_AAD: (&str, &str) = ("memberships", "encrypted_contact_email");

/// The envelope sizes either field may have, the same octet bounds as migration 0008's
/// checks: fau-crypto's 41 bytes of version, nonce and tag around 1..=471 bytes. A name
/// (`DisplayName::MAX_CHARS` = 100 characters, at most 400 bytes) and an address
/// (`Email::MAX_LEN` = 254 bytes) both fit; the rest is headroom, not a second limit.
pub const MEMBER_FIELD_CIPHERTEXT_BYTES: RangeInclusive<usize> = 42..=512;

/// What a person gives when they join: their name, and optionally a contact address. Both
/// are bound to `membership_id`, which comes from `prepare_acceptance` (or, on activation,
/// is chosen fresh by the caller).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberProfile {
    pub membership_id: Uuid,
    pub encrypted_display_name: Ciphertext,
    pub encrypted_contact_email: Option<Ciphertext>,
}

fn well_formed(ct: &Ciphertext) -> bool {
    MEMBER_FIELD_CIPHERTEXT_BYTES.contains(&ct.len()) && ct.as_bytes()[0] == 1
}

pub(crate) fn check_display_name(ct: &Ciphertext) -> Result<(), MembershipError> {
    if well_formed(ct) {
        Ok(())
    } else {
        Err(MembershipError::DisplayNameMalformed)
    }
}

pub(crate) fn check_contact_email(ct: &Ciphertext) -> Result<(), MembershipError> {
    if well_formed(ct) {
        Ok(())
    } else {
        Err(MembershipError::ContactEmailMalformed)
    }
}

/// The id must be a UUIDv7, the same as every other id this schema mints (fix round 1,
/// Q10): `Uuid::nil()` and every other version fail this too, since `get_version` only
/// returns `Some(Version::SortRand)` for one.
pub(crate) fn check_membership_id(id: Uuid) -> Result<(), MembershipError> {
    if id.get_version() == Some(uuid::Version::SortRand) {
        Ok(())
    } else {
        Err(MembershipError::MembershipIdMalformed)
    }
}

impl MemberProfile {
    pub(crate) fn check(&self) -> Result<(), MembershipError> {
        check_membership_id(self.membership_id)?;
        check_display_name(&self.encrypted_display_name)?;
        if let Some(ct) = &self.encrypted_contact_email {
            check_contact_email(ct)?;
        }
        Ok(())
    }
}

/// Writes the profile onto a membership that has just been created, reopened, or was
/// already current -- `already_current`, from `ensure_membership`'s third return value.
///
/// The display name always takes the accepted profile's value: an acceptance always states
/// one (`check` requires it), and it is the name that applies from now on.
///
/// The contact address does too, *unless* `already_current` is true and the new profile
/// carries none: an already-active, already-named member reached by a second invitation --
/// a handover to a sitting member, or recovery -- must not have their stored address
/// silently wiped just because that invitation's acceptance carried no address of its own
/// (fix round 1, Q10). A freshly created row, or one just reopened from revoked, is never
/// `already_current`, and has no address to keep either way (D3 cleared it on revocation),
/// so there the profile's value -- `None` or not -- is written as given.
pub(crate) async fn write_profile(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    profile: &MemberProfile,
    already_current: bool,
) -> Result<(), MembershipError> {
    sqlx::query(
        "update memberships
            set encrypted_display_name = $3,
                encrypted_contact_email = case
                  when $5 and $4 is null then encrypted_contact_email
                  else $4
                end
          where tenant_id = $1 and id = $2",
    )
    .bind(tenant_id)
    .bind(profile.membership_id)
    .bind(profile.encrypted_display_name.as_bytes())
    .bind(
        profile
            .encrypted_contact_email
            .as_ref()
            .map(|c| c.as_bytes()),
    )
    .bind(already_current)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct SetDisplayName {
    pub tenant_id: Uuid,
    pub actor_membership_id: Uuid,
    /// Equal to `actor_membership_id` when a member edits their own name.
    pub membership_id: Uuid,
    pub encrypted_display_name: Ciphertext,
}

/// SQL: membership `m` has ended on the date bound as `date_param` (for example `$3`). It
/// is revoked, or none of its role assignments is still running or yet to start (plan R8).
/// A handover grant does not keep a membership going: it is a recovery right, not a role.
///
/// The one definition that editing ([`set_display_name`]), the retention sweep
/// (`clear_ended_profiles`) and history (`member_names`) share (D3). `revoked_at` is tested
/// on its own too: `revoke_membership` revokes the running and future assignments with the
/// membership, but a row revoked any other way must still count as ended.
pub(crate) fn membership_ended(date_param: &str) -> String {
    format!(
        "(m.revoked_at is not null
          or not exists (select 1 from role_assignments ra
                          where ra.tenant_id = m.tenant_id and ra.membership_id = m.id
                            and ra.revoked_at is null
                            and ra.ends_on_exclusive > {date_param}::date))"
    )
}

/// A member edits their own name, or an admin corrects the name of anyone whose membership
/// is still active (§4.1). Refused while the FAU is frozen, for a name an Article 17
/// erasure removed (`MembershipErased`), and for a membership that has ended
/// (`MembershipEnded`): under D3 an ended membership holds no name, and history shows its
/// role and year instead. A membership whose roles are all still to come is active.
///
/// **Authority before state:** a member naming anyone but themselves gets `NotAuthorized`
/// whether or not the id exists; only an admin learns `UnknownMembership` or
/// `MembershipEnded`.
pub async fn set_display_name(
    pool: &PgPool,
    req: SetDisplayName,
    at: Moment,
) -> Result<(), MembershipError> {
    check_display_name(&req.encrypted_display_name)?;
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, req.tenant_id).await?;
    require_open(&state)?;
    let own = req.actor_membership_id == req.membership_id;
    let by_admin = if own {
        let access =
            membership_access(&mut tx, req.tenant_id, req.actor_membership_id, at.today()).await?;
        if access.capability == Capability::None {
            return Err(MembershipError::NotAuthorized);
        }
        false
    } else {
        if !is_admin_today(&mut tx, req.tenant_id, req.actor_membership_id, at.today()).await? {
            return Err(MembershipError::NotAuthorized);
        }
        true
    };
    let row: Option<(bool, bool)> = sqlx::query_as(&format!(
        "select m.name_erased_at is not null, {}
           from memberships m
          where m.tenant_id = $1 and m.id = $2 for update",
        membership_ended("$3")
    ))
    .bind(req.tenant_id)
    .bind(req.membership_id)
    .bind(date_param(at.today()))
    .fetch_optional(&mut *tx)
    .await?;
    match row {
        None => return Err(MembershipError::UnknownMembership),
        Some((true, _)) => return Err(MembershipError::MembershipErased),
        Some((false, true)) => return Err(MembershipError::MembershipEnded),
        Some((false, false)) => {}
    }
    sqlx::query(
        "update memberships set encrypted_display_name = $3 where tenant_id = $1 and id = $2",
    )
    .bind(req.tenant_id)
    .bind(req.membership_id)
    .bind(req.encrypted_display_name.as_bytes())
    .execute(&mut *tx)
    .await?;
    write_audit(
        &mut tx,
        at,
        Audit::member(
            req.tenant_id,
            req.actor_membership_id,
            "membership.display_name_changed",
            "membership",
            req.membership_id,
            json!({ "by_admin": by_admin }),
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct SetContactEmail {
    pub tenant_id: Uuid,
    /// Only the member themself: the address is their own statement (§4.1).
    pub membership_id: Uuid,
    /// `None` clears it, and the login address is shown instead.
    pub encrypted_contact_email: Option<Ciphertext>,
}

/// A member sets or clears their own contact address. Setting one needs an open FAU;
/// clearing one reduces what others see, so it is allowed while frozen. Either needs
/// standing today: someone whose roles have ended has no address to show, and the
/// retention sweep clears it.
///
/// Refused for a name an Article 17 erasure removed (`MembershipErased`), the same as
/// `set_display_name`: without this check, writing (even clearing) hits migration 0008's
/// `memberships_erasure_leaves_nothing` check constraint as a raw database error instead of
/// a typed refusal (controller ruling, Task 5 review).
pub async fn set_contact_email(
    pool: &PgPool,
    req: SetContactEmail,
    at: Moment,
) -> Result<(), MembershipError> {
    if let Some(ct) = &req.encrypted_contact_email {
        check_contact_email(ct)?;
    }
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, req.tenant_id).await?;
    if req.encrypted_contact_email.is_some() {
        require_open(&state)?;
    }
    let access = membership_access(&mut tx, req.tenant_id, req.membership_id, at.today()).await?;
    if access.capability == Capability::None {
        return Err(MembershipError::NotAuthorized);
    }
    let erased: bool = sqlx::query_scalar(
        "select name_erased_at is not null from memberships where tenant_id = $1 and id = $2",
    )
    .bind(req.tenant_id)
    .bind(req.membership_id)
    .fetch_one(&mut *tx)
    .await?;
    if erased {
        return Err(MembershipError::MembershipErased);
    }
    sqlx::query(
        "update memberships set encrypted_contact_email = $3 where tenant_id = $1 and id = $2",
    )
    .bind(req.tenant_id)
    .bind(req.membership_id)
    .bind(req.encrypted_contact_email.as_ref().map(|c| c.as_bytes()))
    .execute(&mut *tx)
    .await?;
    write_audit(
        &mut tx,
        at,
        Audit::member(
            req.tenant_id,
            req.membership_id,
            "membership.contact_email_changed",
            "membership",
            req.membership_id,
            json!({ "cleared": req.encrypted_contact_email.is_none() }),
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}
