//! The authorization rule (groups design §3.3). One pure function decides what a viewer may
//! do with a resource, from facts read from the database in the same transaction as the
//! operation. `fau_persistence::membership::authorize` reads those facts for one resource.
//! List reads load the same facts per row with the same SQL and call [`decide`] too, so
//! the rule exists exactly once.
//!
//! Security note: this defends against mistakes and casual misuse by people with
//! legitimate access (CLAUDE.md's threat-model boundary). Encryption keys stay per FAU and
//! per document (spec §3.4), so a guest's restriction is authorization, not cryptography.

use crate::membership::access::Capability;
use crate::membership::vocabulary::Visibility;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Seeing that the resource exists, and reading it.
    Read,
    /// Posting, voting and editing content (§3.3).
    Write,
    /// Creating, closing and archiving groups, adding members and inviting guests. Admin
    /// only in the MVP (§3.3; a group lead role is deferred).
    Manage,
}

/// What the database says about a group, for one viewer, today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupFacts {
    pub visibility: Visibility,
    pub archived: bool,
    /// A current member: added by hand and not removed, or holding a role valid today that
    /// the group follows (its guest role, its unit or its cohort; §3.1).
    pub viewer_in_group: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Content addressed to the whole FAU, `group_id` null (§3.2): members and admins,
    /// never guests.
    Fau,
    /// The group itself: that it exists, its name, its member list.
    Group(GroupFacts),
    /// Content whose audience is the group (§3.2).
    GroupContent(GroupFacts),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Denied {
    /// No valid standing in the FAU at all. A live stream for this viewer closes.
    NoAccess,
    /// The viewer may not read the resource, so for them it does not exist: answered
    /// exactly as an unknown id (§3.3's closed-group ruling, generalised).
    Hidden,
    /// The viewer may read the resource but not do this to it.
    Forbidden,
}

pub type Decision = Result<(), Denied>;

pub fn decide(capability: Capability, target: Target, action: Action) -> Decision {
    let can_read = match (capability, target) {
        (Capability::None, _) => return Err(Denied::NoAccess),
        (Capability::Admin, _) => true,
        (Capability::Member, Target::Fau) => true,
        (Capability::Guest, Target::Fau) => false,
        (Capability::Member, Target::Group(g) | Target::GroupContent(g)) => {
            g.visibility == Visibility::Open || g.viewer_in_group
        }
        (Capability::Guest, Target::Group(g) | Target::GroupContent(g)) => g.viewer_in_group,
    };
    if !can_read {
        return Err(Denied::Hidden);
    }
    let admin = capability == Capability::Admin;
    let allowed = match (action, target) {
        (Action::Read, _) => true,
        (Action::Manage, _) | (Action::Write, Target::Group(_)) => admin,
        // Only admins and members read FAU-wide content, and both may write it.
        (Action::Write, Target::Fau) => true,
        (Action::Write, Target::GroupContent(g)) => !g.archived && (admin || g.viewer_in_group),
    };
    if allowed {
        Ok(())
    } else {
        Err(Denied::Forbidden)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::membership::access::Capability::{self, Admin, Guest, Member};
    use crate::membership::vocabulary::Visibility::{Closed, Open};

    const A: Decision = Ok(());
    const F: Decision = Err(Denied::Forbidden);
    const H: Decision = Err(Denied::Hidden);

    fn facts(visibility: Visibility, viewer_in_group: bool) -> GroupFacts {
        GroupFacts {
            visibility,
            archived: false,
            viewer_in_group,
        }
    }

    fn all(c: Capability, t: Target) -> [Decision; 3] {
        [
            decide(c, t, Action::Read),
            decide(c, t, Action::Write),
            decide(c, t, Action::Manage),
        ]
    }

    #[test]
    fn spec_table_for_content() {
        // (capability, visibility or FAU-wide, in the group, [read, write, manage])
        let rows: &[(Capability, Option<Visibility>, bool, [Decision; 3])] = &[
            (Admin, None, false, [A, A, A]),
            (Member, None, false, [A, A, F]),
            (Guest, None, false, [H, H, H]),
            (Admin, Some(Open), true, [A, A, A]),
            (Admin, Some(Open), false, [A, A, A]),
            (Admin, Some(Closed), true, [A, A, A]),
            (Admin, Some(Closed), false, [A, A, A]),
            (Member, Some(Open), true, [A, A, F]),
            (Member, Some(Open), false, [A, F, F]),
            (Member, Some(Closed), true, [A, A, F]),
            (Member, Some(Closed), false, [H, H, H]),
            (Guest, Some(Open), true, [A, A, F]),
            (Guest, Some(Open), false, [H, H, H]),
            (Guest, Some(Closed), true, [A, A, F]),
            (Guest, Some(Closed), false, [H, H, H]),
        ];
        for &(c, v, inside, want) in rows {
            let t = match v {
                None => Target::Fau,
                Some(v) => Target::GroupContent(facts(v, inside)),
            };
            assert_eq!(all(c, t), want, "{c:?} {v:?} in={inside}");
        }
    }

    #[test]
    fn writing_to_a_group_record_is_managing_it() {
        for v in [Open, Closed] {
            assert_eq!(all(Admin, Target::Group(facts(v, false))), [A, A, A]);
            assert_eq!(all(Member, Target::Group(facts(v, true))), [A, F, F]);
            assert_eq!(all(Guest, Target::Group(facts(v, true))), [A, F, F]);
        }
        assert_eq!(all(Member, Target::Group(facts(Open, false))), [A, F, F]);
        assert_eq!(all(Member, Target::Group(facts(Closed, false))), [H, H, H]);
        assert_eq!(all(Guest, Target::Group(facts(Open, false))), [H, H, H]);
    }

    #[test]
    fn no_standing_outranks_everything() {
        let targets = [
            Target::Fau,
            Target::Group(facts(Open, true)),
            Target::GroupContent(facts(Closed, true)),
        ];
        for t in targets {
            for a in [Action::Read, Action::Write, Action::Manage] {
                assert_eq!(decide(Capability::None, t, a), Err(Denied::NoAccess));
            }
        }
    }

    #[test]
    fn an_archived_group_is_read_only_for_everyone_but_still_managed() {
        let archived = GroupFacts {
            visibility: Open,
            archived: true,
            viewer_in_group: true,
        };
        assert_eq!(all(Admin, Target::GroupContent(archived)), [A, F, A]);
        assert_eq!(all(Member, Target::GroupContent(archived)), [A, F, F]);
        assert_eq!(all(Guest, Target::GroupContent(archived)), [A, F, F]);
        assert_eq!(all(Admin, Target::Group(archived)), [A, A, A]);
    }
}
