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

use std::ops::RangeInclusive;

use fau_crypto::Ciphertext;
use sqlx::PgConnection;
use uuid::Uuid;

use super::error::MembershipError;

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
