//! The membership foundation's transactions (#3413 flow spec; #3417, #3418). Each public
//! function is one transaction that writes its state change together with its audit
//! entry and, where the spec says so, its outbox message (spec 2.6).
//!
//! Every function takes a [`fau_domain::time::Moment`]: rule-deciding timestamps and
//! "today" come from the caller, never from the database clock.

mod error;
mod handover;
mod invitations;
mod requests;
mod roles;
mod signup;
mod sql;
mod token;

pub use error::{ExistingFau, MembershipError};
pub use handover::{create_handover_grants, recovery_grant_admin, RecoveryActor, RecoveryGrant};
pub use invitations::{
    accept_invitation, issue_invitation, resend_invitation, withdraw_invitation, AcceptInvitation,
    Accepted, InvitationChange, IssueInvitation, IssuedInvitation, OfferedRole, RoleChoice,
};
pub use requests::{
    approve_request, create_access_request, create_replacement_proposal, decline_request,
    lapse_requests, CreateAccessRequest, CreateReplacementProposal, RequestDecision,
};
pub use roles::{
    grant_role, revoke_membership, revoke_role_assignment, GrantRole, RevokeAssignment,
    RevokeMembership,
};
pub use signup::{
    activate_tenant, create_pending_tenant, expire_pending_tenants, Activated, Activation,
    PendingSignup, PendingTenant,
};
pub use token::InvitationToken;
