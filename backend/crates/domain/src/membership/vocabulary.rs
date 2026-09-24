//! The closed sets the membership model is built from, with the exact codes the database
//! check constraints in migrations 0002 and 0003 accept. `code()` and `from_code()` are
//! the only translation between the two.

use std::fmt;

/// The privilege a role grants. The class decides, never the role's name (§2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityClass {
    Member,
    Admin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TenantStatus {
    Pending,
    Active,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvitationMode {
    /// Issued by an admin valid today.
    Normal,
    /// The leader invitation written by activation (§3.4.3); no issuing membership.
    Activation,
    /// A replacement invitation from an outgoing admin's handover grant (§6.3).
    Handover,
    /// Issued by the recovery contact in the no-admin state (§6.4); no issuing membership.
    Recovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    Access,
    Replacement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestStatus {
    Pending,
    Approved,
    Declined,
    Withdrawn,
    Lapsed,
}

/// Who holds an FAU's recovery-contact seat (§6.5, ADR-003 decision 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryHolder {
    Ewb,
    SchoolRep,
}

macro_rules! codes {
    ($ty:ty { $($variant:ident => $code:literal),+ $(,)? }) => {
        impl $ty {
            pub const ALL: &'static [$ty] = &[$(<$ty>::$variant),+];

            pub fn code(self) -> &'static str {
                match self { $(<$ty>::$variant => $code),+ }
            }

            pub fn from_code(code: &str) -> Option<Self> {
                match code { $($code => Some(<$ty>::$variant),)+ _ => None }
            }
        }
    };
}

codes!(CapabilityClass { Member => "member", Admin => "admin" });
codes!(TenantStatus { Pending => "pending", Active => "active", Closed => "closed" });
codes!(InvitationMode {
    Normal => "normal",
    Activation => "activation",
    Handover => "handover",
    Recovery => "recovery",
});
codes!(RequestKind { Access => "access", Replacement => "replacement" });
codes!(RequestStatus {
    Pending => "pending",
    Approved => "approved",
    Declined => "declined",
    Withdrawn => "withdrawn",
    Lapsed => "lapsed",
});
codes!(RecoveryHolder { Ewb => "ewb", SchoolRep => "school_rep" });

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameError {
    Empty,
    TooLong,
    ControlCharacter,
}

fn validate_name(raw: &str, max_chars: usize) -> Result<String, NameError> {
    let s = raw.trim();
    if s.is_empty() {
        return Err(NameError::Empty);
    }
    if s.chars().count() > max_chars {
        return Err(NameError::TooLong);
    }
    if s.chars().any(char::is_control) {
        return Err(NameError::ControlCharacter);
    }
    Ok(s.to_owned())
}

/// A role's display name, as an admin typed it ("Leder", "Kasserer"). Free text, so it
/// is never written to audit parameters; audit carries the role's id instead. `Debug`
/// is redacted, like `Email`'s.
#[derive(Clone, PartialEq, Eq)]
pub struct RoleName(String);

impl RoleName {
    pub const MAX_CHARS: usize = 100;

    pub fn parse(raw: &str) -> Result<Self, NameError> {
        validate_name(raw, Self::MAX_CHARS).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An FAU's name. Pre-filled from the school name and editable (§3.1.2). `Debug` is
/// redacted, like `Email`'s: the name is free text the registrant typed.
#[derive(Clone, PartialEq, Eq)]
pub struct FauName(String);

impl FauName {
    pub const MAX_CHARS: usize = 200;

    pub fn parse(raw: &str) -> Result<Self, NameError> {
        validate_name(raw, Self::MAX_CHARS).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for RoleName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RoleName([redacted])")
    }
}

impl fmt::Debug for FauName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FauName([redacted])")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! round_trips {
        ($name:ident, $ty:ty) => {
            #[test]
            fn $name() {
                for v in <$ty>::ALL {
                    assert_eq!(<$ty>::from_code(v.code()), Some(*v));
                }
                assert_eq!(<$ty>::from_code("unknown"), None);
            }
        };
    }

    round_trips!(capability_class_round_trips, CapabilityClass);
    round_trips!(tenant_status_round_trips, TenantStatus);
    round_trips!(invitation_mode_round_trips, InvitationMode);
    round_trips!(request_kind_round_trips, RequestKind);
    round_trips!(request_status_round_trips, RequestStatus);
    round_trips!(recovery_holder_round_trips, RecoveryHolder);

    #[test]
    fn codes_match_the_database_check_constraints() {
        // The literal sets from 0002 and 0003; changing either side must change both.
        assert_eq!(
            CapabilityClass::ALL
                .iter()
                .map(|v| v.code())
                .collect::<Vec<_>>(),
            ["member", "admin"]
        );
        assert_eq!(
            TenantStatus::ALL
                .iter()
                .map(|v| v.code())
                .collect::<Vec<_>>(),
            ["pending", "active", "closed"]
        );
        assert_eq!(
            InvitationMode::ALL
                .iter()
                .map(|v| v.code())
                .collect::<Vec<_>>(),
            ["normal", "activation", "handover", "recovery"]
        );
        assert_eq!(
            RequestKind::ALL
                .iter()
                .map(|v| v.code())
                .collect::<Vec<_>>(),
            ["access", "replacement"]
        );
        assert_eq!(
            RequestStatus::ALL
                .iter()
                .map(|v| v.code())
                .collect::<Vec<_>>(),
            ["pending", "approved", "declined", "withdrawn", "lapsed"]
        );
        assert_eq!(
            RecoveryHolder::ALL
                .iter()
                .map(|v| v.code())
                .collect::<Vec<_>>(),
            ["ewb", "school_rep"]
        );
    }

    /// Final review M8: role and FAU names are free text an admin or registrant typed,
    /// so `Debug` redacts them the same way `Email`'s does -- a derived `Debug` on any
    /// struct holding one (`PendingSignup`, `RoleChoice`, ...) would otherwise print it.
    #[test]
    fn names_are_redacted_in_debug() {
        let role = RoleName::parse("Kasserer Kari").unwrap();
        let fau = FauName::parse("Nordre skole FAU").unwrap();
        assert_eq!(format!("{role:?}"), "RoleName([redacted])");
        assert_eq!(format!("{fau:?}"), "FauName([redacted])");
        assert!(!format!("{:?}", Some(&role)).contains("Kari"));
        assert!(!format!("{fau:#?}").contains("Nordre"));
    }

    #[test]
    fn names_are_trimmed_and_bounded() {
        assert_eq!(RoleName::parse("  Leder ").unwrap().as_str(), "Leder");
        assert_eq!(RoleName::parse("   "), Err(NameError::Empty));
        assert_eq!(
            RoleName::parse(&"æ".repeat(101)),
            Err(NameError::TooLong),
            "the bound counts characters, not bytes"
        );
        assert!(RoleName::parse(&"æ".repeat(100)).is_ok());
        assert_eq!(RoleName::parse("Le\nder"), Err(NameError::ControlCharacter));
        assert_eq!(
            FauName::parse("Nordre Skole FAU").unwrap().as_str(),
            "Nordre Skole FAU"
        );
        assert_eq!(FauName::parse(&"x".repeat(201)), Err(NameError::TooLong));
    }
}
