//! Access evaluation (§2.1, §4, §6.3, §6.4). Rights are the union of roles valid today,
//! re-evaluated on every request from the database, never cached from login.

use jiff::civil::Date;

use super::period::Period;
use super::vocabulary::CapabilityClass;

/// What a person may do in one FAU today. Ordered: `Admin` includes every `Member` right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capability {
    None,
    Member,
    Admin,
}

/// One role assignment as the rules need it. `capability` comes from the role row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssignmentView {
    pub capability: CapabilityClass,
    pub period: Period,
    pub revoked: bool,
}

impl AssignmentView {
    pub fn valid_on(&self, day: Date) -> bool {
        !self.revoked && self.period.contains(day)
    }
}

/// One handover grant (§6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrantView {
    pub period: Period,
    pub revoked: bool,
}

impl GrantView {
    pub fn valid_on(&self, day: Date) -> bool {
        !self.revoked && self.period.contains(day)
    }
}

/// The preconditions that come before any role is looked at. Each one alone removes all
/// access: an FAU that is not active, a disabled or unverified account, or a revoked
/// membership (#3412: "a revoked right gives no access through the revoked right").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Standing {
    pub tenant_active: bool,
    pub account_usable: bool,
    pub membership_active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Access {
    pub capability: Capability,
    /// A handover grant is valid today. It gives no document access (§6.3); the HTTP
    /// layer lets it reach only the handover operations.
    pub handover: bool,
    /// When `capability` is `None`, the earliest future start of a role, so the page can
    /// say when access begins (§4).
    pub next_start: Option<Date>,
}

impl Access {
    pub const NONE: Access = Access {
        capability: Capability::None,
        handover: false,
        next_start: None,
    };
}

pub fn evaluate_access(
    standing: Standing,
    assignments: &[AssignmentView],
    grants: &[GrantView],
    today: Date,
) -> Access {
    if !(standing.tenant_active && standing.account_usable && standing.membership_active) {
        return Access::NONE;
    }
    let capability = assignments
        .iter()
        .filter(|a| a.valid_on(today))
        .map(|a| match a.capability {
            CapabilityClass::Member => Capability::Member,
            CapabilityClass::Admin => Capability::Admin,
        })
        .max()
        .unwrap_or(Capability::None);
    let handover = grants.iter().any(|g| g.valid_on(today));
    let next_start = if capability == Capability::None {
        assignments
            .iter()
            .filter(|a| !a.revoked && a.period.starts_on() > today)
            .map(|a| a.period.starts_on())
            .min()
    } else {
        None
    };
    Access {
        capability,
        handover,
        next_start,
    }
}

/// The negation of §6.4's no-admin state: an `admin`-class role valid today or a handover
/// grant valid today. The caller passes only rows whose membership is active and whose
/// account is usable; a revoked person's rows do not count.
pub fn tenant_has_admin(assignments: &[AssignmentView], grants: &[GrantView], today: Date) -> bool {
    assignments
        .iter()
        .any(|a| a.capability == CapabilityClass::Admin && a.valid_on(today))
        || grants.iter().any(|g| g.valid_on(today))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    const OK: Standing = Standing {
        tenant_active: true,
        account_usable: true,
        membership_active: true,
    };

    fn role(capability: CapabilityClass, from: Date, to: Date) -> AssignmentView {
        AssignmentView {
            capability,
            period: Period::new(from, to).unwrap(),
            revoked: false,
        }
    }

    fn grant(from: Date, to: Date) -> GrantView {
        GrantView {
            period: Period::new(from, to).unwrap(),
            revoked: false,
        }
    }

    #[test]
    fn a_role_valid_tomorrow_grants_nothing_today() {
        let today = date(2026, 9, 23);
        let a = [role(
            CapabilityClass::Member,
            date(2026, 9, 24),
            date(2027, 9, 24),
        )];
        let access = evaluate_access(OK, &a, &[], today);
        assert_eq!(access.capability, Capability::None);
        assert_eq!(access.next_start, Some(date(2026, 9, 24)));
    }

    #[test]
    fn the_end_date_itself_grants_nothing() {
        let a = [role(
            CapabilityClass::Member,
            date(2026, 8, 1),
            date(2027, 8, 1),
        )];
        assert_eq!(
            evaluate_access(OK, &a, &[], date(2027, 7, 31)).capability,
            Capability::Member
        );
        assert_eq!(
            evaluate_access(OK, &a, &[], date(2027, 8, 1)).capability,
            Capability::None
        );
    }

    #[test]
    fn rights_are_the_union_of_valid_roles() {
        // #3412's example: admin ends, a member role continues.
        let a = [
            role(CapabilityClass::Admin, date(2026, 8, 1), date(2027, 8, 1)),
            role(CapabilityClass::Member, date(2026, 8, 1), date(2028, 1, 1)),
        ];
        assert_eq!(
            evaluate_access(OK, &a, &[], date(2027, 7, 31)).capability,
            Capability::Admin
        );
        assert_eq!(
            evaluate_access(OK, &a, &[], date(2027, 8, 1)).capability,
            Capability::Member
        );
    }

    #[test]
    fn a_revoked_role_grants_nothing() {
        let mut a = role(CapabilityClass::Admin, date(2026, 8, 1), date(2027, 8, 1));
        a.revoked = true;
        assert_eq!(
            evaluate_access(OK, &[a], &[], date(2026, 9, 23)),
            Access::NONE
        );
    }

    #[test]
    fn each_standing_precondition_removes_all_access() {
        let a = [role(
            CapabilityClass::Admin,
            date(2026, 8, 1),
            date(2027, 8, 1),
        )];
        let g = [grant(date(2026, 8, 1), date(2027, 2, 1))];
        for standing in [
            Standing {
                tenant_active: false,
                ..OK
            },
            Standing {
                account_usable: false,
                ..OK
            },
            Standing {
                membership_active: false,
                ..OK
            },
        ] {
            assert_eq!(
                evaluate_access(standing, &a, &g, date(2026, 9, 23)),
                Access::NONE
            );
        }
    }

    #[test]
    fn a_handover_grant_alone_gives_no_capability() {
        let a = [role(
            CapabilityClass::Admin,
            date(2026, 10, 1),
            date(2027, 10, 1),
        )];
        let g = [grant(date(2027, 10, 1), date(2028, 4, 1))];
        let access = evaluate_access(OK, &a, &g, date(2027, 11, 1));
        assert_eq!(access.capability, Capability::None);
        assert!(access.handover);
        assert_eq!(access.next_start, None);
    }

    #[test]
    fn a_handover_grant_ends_at_its_boundary_or_on_revocation() {
        let g = grant(date(2027, 10, 1), date(2028, 4, 1));
        assert!(evaluate_access(OK, &[], &[g], date(2028, 3, 31)).handover);
        assert!(!evaluate_access(OK, &[], &[g], date(2028, 4, 1)).handover);
        let revoked = GrantView { revoked: true, ..g };
        assert!(!evaluate_access(OK, &[], &[revoked], date(2027, 11, 1)).handover);
    }

    #[test]
    fn the_no_admin_predicate() {
        let today = date(2026, 9, 23);
        let member = role(CapabilityClass::Member, date(2026, 1, 1), date(2027, 1, 1));
        let admin = role(CapabilityClass::Admin, date(2026, 1, 1), date(2027, 1, 1));
        let future_admin = role(CapabilityClass::Admin, date(2026, 10, 1), date(2027, 10, 1));
        let g = grant(date(2026, 8, 1), date(2027, 2, 1));

        assert!(!tenant_has_admin(&[], &[], today));
        assert!(
            !tenant_has_admin(&[member], &[], today),
            "the class decides, not the name"
        );
        assert!(!tenant_has_admin(&[future_admin], &[], today));
        assert!(tenant_has_admin(&[admin], &[], today));
        assert!(
            tenant_has_admin(&[member], &[g], today),
            "a valid handover grant counts"
        );
    }
}
