//! The membership foundation's transactions (#3413 flow spec; #3417, #3418). Each public
//! function is one transaction that writes its state change together with its audit
//! entry and, where the spec says so, its outbox message (spec 2.6).
//!
//! Every function takes a [`fau_domain::time::Moment`]: rule-deciding timestamps and
//! "today" come from the caller, never from the database clock.

mod error;
mod invitations;
mod signup;
mod sql;
mod token;

pub use error::{ExistingFau, MembershipError};
pub use invitations::IssuedInvitation;
pub use signup::{
    activate_tenant, create_pending_tenant, expire_pending_tenants, Activated, Activation,
    PendingSignup, PendingTenant,
};
pub use token::InvitationToken;
