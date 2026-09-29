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
//!
//! **Authorization** (groups design §3.3, #3501): `authorize` is the one function that
//! reads the database to decide what a membership may do with a resource. Every read path
//! and every change-stream delivery goes through it or through `fau_domain::authz::decide`
//! on the same facts.

mod access;
mod authz;
mod directory;
mod error;
mod events;
mod groups;
mod handover;
mod invitations;
mod names;
mod profile;
mod requests;
mod retention;
mod roles;
mod signup;
mod sql;
mod token;

pub use access::effective_access;
pub use authz::{authorize, read_transaction, Resource, Viewer};
pub use directory::{
    member_directory, DirectoryAddress, DirectoryPerson, DirectoryRead, DirectorySection,
};
pub use error::{ExistingFau, MembershipError};
pub use events::{Change, Hub, HubClock, Subscription, EVENTS_CHANNEL, SUBSCRIPTION_BUFFER};
pub use groups::{
    add_group_member, archive_group, create_group, get_group, list_group_members, list_groups,
    remove_group_member, rename_group, set_group_visibility, ArchiveGroup, CreateGroup,
    GroupBinding, GroupMemberChange, GroupMemberView, GroupView, RenameGroup, SetGroupVisibility,
    GROUP_NAME_AAD, GROUP_NAME_CIPHERTEXT_BYTES,
};
pub use handover::{create_handover_grants, recovery_grant_admin, RecoveryActor, RecoveryGrant};
pub use invitations::{
    accept_invitation, invitation_message, issue_invitation, prepare_acceptance, resend_invitation,
    withdraw_invitation, AcceptInvitation, AcceptanceTarget, Accepted, InvitationChange,
    InvitationMessage, InvitationMessageView, IssueInvitation, IssuedInvitation, OfferedRole,
    RoleChoice, INVITATION_MESSAGE_AAD,
};
pub use names::{member_names, MemberName};
pub use profile::{
    set_contact_email, set_display_name, MemberProfile, SetContactEmail, SetDisplayName,
    CONTACT_EMAIL_AAD, DISPLAY_NAME_AAD, MEMBER_FIELD_CIPHERTEXT_BYTES,
};
pub use requests::{
    access_request_message, approve_request, create_access_request, create_replacement_proposal,
    decline_request, lapse_requests, AccessRequestMessage, CreateAccessRequest,
    CreateReplacementProposal, RequestDecision, ACCESS_REQUEST_MESSAGE_AAD, MESSAGE_MAX_BYTES,
};
pub use retention::{clear_ended_profiles, erase_member_names};
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
