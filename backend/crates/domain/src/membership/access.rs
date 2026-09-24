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

/// Spec 7's last-admin safeguard: whether ending `removed_*` would leave a day, from
/// `today` on, on which the FAU has no admin although the removed rows would have given
/// it one. Judging only today would make "revoking the only other admin" impossible to
/// catch -- the revoking admin is valid today by definition -- so the check runs over
/// the removed rows' remaining term: an admin whose own role ends in December revoking
/// the admin who would have carried the FAU until next October leaves a gap.
///
/// Coverage can only drop where a remaining row ends, so checking the first remaining
/// day of each removed row and every remaining end inside it is exhaustive. Handover
/// grants that do not exist yet (they are created when a role ends) are not predicted.
pub fn removal_leaves_no_admin(
    remaining_assignments: &[AssignmentView],
    remaining_grants: &[GrantView],
    removed_assignments: &[AssignmentView],
    removed_grants: &[GrantView],
    today: Date,
) -> bool {
    let covered = |day: Date| tenant_has_admin(remaining_assignments, remaining_grants, day);
    let remaining_ends: Vec<Date> = remaining_assignments
        .iter()
        .filter(|a| !a.revoked && a.capability == CapabilityClass::Admin)
        .map(|a| a.period.ends_on_exclusive())
        .chain(
            remaining_grants
                .iter()
                .filter(|g| !g.revoked)
                .map(|g| g.period.ends_on_exclusive()),
        )
        .collect();
    let removed = removed_assignments
        .iter()
        .filter(|a| !a.revoked && a.capability == CapabilityClass::Admin)
        .map(|a| a.period)
        .chain(
            removed_grants
                .iter()
                .filter(|g| !g.revoked)
                .map(|g| g.period),
        );
    for period in removed {
        if period.has_ended_by(today) {
            continue;
        }
        let first = period.starts_on().max(today);
        if !covered(first) {
            return true;
        }
        if remaining_ends
            .iter()
            .any(|&end| first < end && end < period.ends_on_exclusive() && !covered(end))
        {
            return true;
        }
    }
    false
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

    #[test]
    fn removing_the_sole_admin_leaves_none() {
        let today = date(2026, 9, 23);
        let only = role(CapabilityClass::Admin, date(2026, 9, 1), date(2027, 10, 1));
        assert!(removal_leaves_no_admin(&[], &[], &[only], &[], today));
    }

    #[test]
    fn removing_one_of_two_equal_admins_leaves_one() {
        let today = date(2026, 9, 23);
        let a = role(CapabilityClass::Admin, date(2026, 9, 1), date(2027, 10, 1));
        let b = role(CapabilityClass::Admin, date(2026, 9, 1), date(2027, 10, 1));
        assert!(!removal_leaves_no_admin(&[a], &[], &[b], &[], today));
    }

    #[test]
    fn removing_the_admin_who_outlasts_you_leaves_a_gap() {
        // Spec 7's "an admin revoking the only other admin".
        let today = date(2026, 9, 23);
        let short = role(CapabilityClass::Admin, date(2026, 1, 1), date(2026, 12, 1));
        let long = role(CapabilityClass::Admin, date(2026, 1, 1), date(2027, 10, 1));
        assert!(removal_leaves_no_admin(&[short], &[], &[long], &[], today));

        let successor = role(CapabilityClass::Admin, date(2026, 12, 1), date(2027, 10, 1));
        assert!(!removal_leaves_no_admin(
            &[short, successor],
            &[],
            &[long],
            &[],
            today
        ));
    }

    #[test]
    fn removing_an_ended_or_member_role_changes_nothing() {
        let today = date(2026, 9, 23);
        let ended = role(CapabilityClass::Admin, date(2025, 9, 1), date(2026, 9, 1));
        let member = role(CapabilityClass::Member, date(2026, 9, 1), date(2027, 9, 1));
        assert!(!removal_leaves_no_admin(
            &[],
            &[],
            &[ended, member],
            &[],
            today
        ));
    }

    #[test]
    fn removing_a_future_admin_with_nobody_else_then_leaves_a_gap() {
        let today = date(2026, 9, 23);
        let current = role(CapabilityClass::Admin, date(2026, 1, 1), date(2026, 12, 1));
        let next = role(CapabilityClass::Admin, date(2026, 12, 1), date(2027, 12, 1));
        assert!(removal_leaves_no_admin(
            &[current],
            &[],
            &[next],
            &[],
            today
        ));
    }

    #[test]
    fn a_remaining_handover_grant_counts_as_coverage() {
        let today = date(2026, 9, 23);
        let g = grant(date(2026, 8, 1), date(2027, 2, 1));
        let admin = role(CapabilityClass::Admin, date(2026, 9, 1), date(2026, 12, 1));
        assert!(!removal_leaves_no_admin(&[], &[g], &[admin], &[], today));
        assert!(removal_leaves_no_admin(&[], &[], &[], &[g], today));
    }
}
