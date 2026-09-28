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

impl MemberProfile {
    pub(crate) fn check(&self) -> Result<(), MembershipError> {
        check_display_name(&self.encrypted_display_name)?;
        if let Some(ct) = &self.encrypted_contact_email {
            check_contact_email(ct)?;
        }
        Ok(())
    }
}

/// Writes both fields onto a membership that is not revoked (the caller has just created
/// or reopened it). A re-invited former member's row holds no name by then (D3: revocation
/// and the `clear_ended_profiles` sweep cleared it), so the new acceptance states the name
/// that applies from now on; one whose roles ran out before the sweep reached them has
/// theirs replaced.
pub(crate) async fn write_profile(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    profile: &MemberProfile,
) -> Result<(), MembershipError> {
    sqlx::query(
        "update memberships set encrypted_display_name = $3, encrypted_contact_email = $4
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
    .execute(&mut *conn)
    .await?;
    Ok(())
}
