//! The membership foundation's transactions (#3413 flow spec; #3417, #3418). Each public
//! function is one transaction that writes its state change together with its audit
//! entry and, where the spec says so, its outbox message (spec 2.6).
//!
//! Every function takes a [`fau_domain::time::Moment`]: rule-deciding timestamps and
//! "today" come from the caller, never from the database clock.
//!
//! **Ordering rule** (controller ruling, final review M6): authority before row state;
//! tenant state (frozen/closed) may be checked first, because members can see it
//! anyway. So an actor without authority gets `NotAuthorized` whether the invitation,
//! request, assignment or membership they named exists, is already settled, or not --
//! but may learn from `TenantFrozen` or `TenantNotActive` that the FAU is frozen or
//! closed.
//!
//! **Locking rule:** every mutation of a tenant's membership state takes `lock_tenant`
//! first, which serialises role changes, the last-admin safeguard and the handover
//! sweep within one FAU. The exceptions are documented where they occur:
//! `create_pending_tenant` (no tenant exists yet), `expire_pending_tenants` and
//! `lapse_requests` (row locks and a READ COMMITTED re-check suffice). The read-only
//! `effective_access` takes no lock and reads one REPEATABLE READ snapshot instead.

mod access;
mod error;
mod handover;
mod invitations;
mod requests;
mod roles;
mod signup;
mod sql;
mod token;

pub use access::effective_access;
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
pub use sql::audit_actor_kind_codes;
pub use token::InvitationToken;
