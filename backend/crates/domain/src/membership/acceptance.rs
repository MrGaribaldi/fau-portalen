//! Invitation acceptance (§5.1). Persistence locks the invitation, reads everything the
//! rule needs into an [`AcceptanceSnapshot`], and calls [`check_acceptance`]; the rule
//! itself never touches the database, so every failure case is a unit test here.

use jiff::Timestamp;

use super::vocabulary::{InvitationMode, TenantStatus};
use crate::email::{Email, VerifiedEmail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptanceSnapshot {
    pub mode: InvitationMode,
    pub expires_at: Timestamp,
    pub accepted: bool,
    pub revoked: bool,
    pub recipient: Email,
    /// The address the provider verified for the person clicking "Bli med". The type
    /// carries the verification, so "email is verified" is not a runtime check.
    pub acceptor: VerifiedEmail,
    pub acceptor_account_disabled: bool,
    pub tenant_status: TenantStatus,
    pub tenant_frozen: bool,
    /// `normal` mode: the issuing membership holds an admin role valid today.
    pub issuer_admin_today: bool,
    /// `handover` mode: the linked grant is valid today and still belongs to the issuer.
    pub handover_grant_valid_today: bool,
    /// `recovery` mode: the FAU has an admin today, which ends the recovery contact's
    /// authority (§6.4).
    pub tenant_has_admin_today: bool,
}

/// Why an acceptance was refused. Each maps to its own message in #3422, and none of
/// them names another person (§5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptanceRefusal {
    Revoked,
    AlreadyAccepted,
    Expired,
    EmailMismatch,
    AccountDisabled,
    TenantNotActive,
    TenantFrozen,
    IssuerLacksAuthority,
}

/// Every check in §5.1, in a fixed order: the token's own state first, then the person,
/// then the FAU, then the issuer's authority.
pub fn check_acceptance(s: &AcceptanceSnapshot, now: Timestamp) -> Result<(), AcceptanceRefusal> {
    if s.revoked {
        return Err(AcceptanceRefusal::Revoked);
    }
    if s.accepted {
        return Err(AcceptanceRefusal::AlreadyAccepted);
    }
    if now >= s.expires_at {
        return Err(AcceptanceRefusal::Expired);
    }
    if &s.recipient != s.acceptor.email() {
        return Err(AcceptanceRefusal::EmailMismatch);
    }
    if s.acceptor_account_disabled {
        return Err(AcceptanceRefusal::AccountDisabled);
    }
    if s.tenant_status != TenantStatus::Active {
        return Err(AcceptanceRefusal::TenantNotActive);
    }
    if s.tenant_frozen {
        return Err(AcceptanceRefusal::TenantFrozen);
    }
    let authorised = match s.mode {
        InvitationMode::Normal => s.issuer_admin_today,
        InvitationMode::Handover => s.handover_grant_valid_today,
        InvitationMode::Recovery => !s.tenant_has_admin_today,
        // Completes the signup form (decision 11): the registrant's own authority is not
        // re-checked, only the token, the address and the FAU.
        InvitationMode::Activation => true,
    };
    if !authorised {
        return Err(AcceptanceRefusal::IssuerLacksAuthority);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    fn email(s: &str) -> Email {
        Email::parse(s).unwrap()
    }

    fn valid(mode: InvitationMode) -> AcceptanceSnapshot {
        AcceptanceSnapshot {
            mode,
            expires_at: ts("2026-10-07T10:00:00Z"),
            accepted: false,
            revoked: false,
            recipient: email("ny@example.test"),
            acceptor: VerifiedEmail::from_provider(email("ny@example.test")),
            acceptor_account_disabled: false,
            tenant_status: TenantStatus::Active,
            tenant_frozen: false,
            issuer_admin_today: true,
            handover_grant_valid_today: true,
            tenant_has_admin_today: false,
        }
    }

    const NOW: &str = "2026-09-30T10:00:00Z";

    #[test]
    fn a_valid_invitation_is_accepted_in_every_mode() {
        for mode in InvitationMode::ALL {
            assert_eq!(check_acceptance(&valid(*mode), ts(NOW)), Ok(()), "{mode:?}");
        }
    }

    #[test]
    fn expired_used_and_revoked_tokens_are_refused() {
        let s = valid(InvitationMode::Normal);
        assert_eq!(
            check_acceptance(&s, ts("2026-10-07T10:00:00Z")),
            Err(AcceptanceRefusal::Expired),
            "the expiry instant itself is already expired"
        );
        let used = AcceptanceSnapshot {
            accepted: true,
            ..s.clone()
        };
        assert_eq!(
            check_acceptance(&used, ts(NOW)),
            Err(AcceptanceRefusal::AlreadyAccepted)
        );
        let revoked = AcceptanceSnapshot { revoked: true, ..s };
        assert_eq!(
            check_acceptance(&revoked, ts(NOW)),
            Err(AcceptanceRefusal::Revoked)
        );
    }

    #[test]
    fn revocation_is_reported_before_expiry() {
        let s = AcceptanceSnapshot {
            revoked: true,
            ..valid(InvitationMode::Normal)
        };
        assert_eq!(
            check_acceptance(&s, ts("2027-01-01T00:00:00Z")),
            Err(AcceptanceRefusal::Revoked)
        );
    }

    #[test]
    fn a_different_address_is_refused() {
        let s = AcceptanceSnapshot {
            acceptor: VerifiedEmail::from_provider(email("annen@example.test")),
            ..valid(InvitationMode::Normal)
        };
        assert_eq!(
            check_acceptance(&s, ts(NOW)),
            Err(AcceptanceRefusal::EmailMismatch)
        );
    }

    /// A wrong-address acceptor never reveals FAU-internal state (frozen, not active):
    /// the person check runs, and fails, before the FAU check ever does. Guards against a
    /// leak through the refusal reason.
    #[test]
    fn a_mismatched_email_is_reported_before_a_frozen_or_inactive_tenant() {
        let not_active = AcceptanceSnapshot {
            acceptor: VerifiedEmail::from_provider(email("annen@example.test")),
            tenant_status: TenantStatus::Closed,
            ..valid(InvitationMode::Normal)
        };
        assert_eq!(
            check_acceptance(&not_active, ts(NOW)),
            Err(AcceptanceRefusal::EmailMismatch)
        );
        let frozen = AcceptanceSnapshot {
            acceptor: VerifiedEmail::from_provider(email("annen@example.test")),
            tenant_frozen: true,
            ..valid(InvitationMode::Normal)
        };
        assert_eq!(
            check_acceptance(&frozen, ts(NOW)),
            Err(AcceptanceRefusal::EmailMismatch)
        );
    }

    #[test]
    fn a_disabled_account_is_refused() {
        let s = AcceptanceSnapshot {
            acceptor_account_disabled: true,
            ..valid(InvitationMode::Normal)
        };
        assert_eq!(
            check_acceptance(&s, ts(NOW)),
            Err(AcceptanceRefusal::AccountDisabled)
        );
    }

    #[test]
    fn an_fau_that_is_not_active_or_is_frozen_is_refused() {
        for status in [TenantStatus::Pending, TenantStatus::Closed] {
            let s = AcceptanceSnapshot {
                tenant_status: status,
                ..valid(InvitationMode::Normal)
            };
            assert_eq!(
                check_acceptance(&s, ts(NOW)),
                Err(AcceptanceRefusal::TenantNotActive)
            );
        }
        let frozen = AcceptanceSnapshot {
            tenant_frozen: true,
            ..valid(InvitationMode::Normal)
        };
        assert_eq!(
            check_acceptance(&frozen, ts(NOW)),
            Err(AcceptanceRefusal::TenantFrozen)
        );
    }

    #[test]
    fn the_issuer_must_still_have_authority() {
        let normal = AcceptanceSnapshot {
            issuer_admin_today: false,
            ..valid(InvitationMode::Normal)
        };
        assert_eq!(
            check_acceptance(&normal, ts(NOW)),
            Err(AcceptanceRefusal::IssuerLacksAuthority)
        );
        let handover = AcceptanceSnapshot {
            handover_grant_valid_today: false,
            ..valid(InvitationMode::Handover)
        };
        assert_eq!(
            check_acceptance(&handover, ts(NOW)),
            Err(AcceptanceRefusal::IssuerLacksAuthority)
        );
        let recovery = AcceptanceSnapshot {
            tenant_has_admin_today: true,
            ..valid(InvitationMode::Recovery)
        };
        assert_eq!(
            check_acceptance(&recovery, ts(NOW)),
            Err(AcceptanceRefusal::IssuerLacksAuthority)
        );
    }

    #[test]
    fn an_activation_invitation_needs_no_issuer_authority() {
        let s = AcceptanceSnapshot {
            issuer_admin_today: false,
            handover_grant_valid_today: false,
            tenant_has_admin_today: true,
            ..valid(InvitationMode::Activation)
        };
        assert_eq!(check_acceptance(&s, ts(NOW)), Ok(()));
    }
}
