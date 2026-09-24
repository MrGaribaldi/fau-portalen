//! The one error type every membership transaction returns. Typed, so #3417's HTTP layer
//! can map each variant to an `ErrorCode` without parsing text; and its `Display` is a
//! fixed phrase per variant that never carries an address, a name, a token or a message.

use fau_domain::membership::acceptance::AcceptanceRefusal;
use fau_domain::membership::requests::{MessageError, ReplacementDateError, RequestLimit};
use fau_domain::membership::rules::AdminEndError;

use crate::pool::safe_error_kind;

/// Which state the FAU that already holds a school is in (spec 3.2). The only thing a
/// colliding registrant learns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExistingFau {
    Pending,
    Active,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MembershipError {
    // Input that only makes sense against today's date or the database.
    #[error("admin end date out of range")]
    AdminEnd(AdminEndError),
    #[error("period is empty")]
    EmptyPeriod,
    #[error("period has already ended")]
    PeriodAlreadyEnded,
    #[error("no roles offered")]
    NoRolesOffered,
    #[error("a role is offered twice")]
    DuplicateRole,

    // Signup and activation.
    #[error("the school already has an FAU")]
    SchoolTaken(ExistingFau),
    #[error("too many pending signups for this address")]
    TooManyPendingSignups,
    #[error("the signup has expired")]
    SignupExpired,
    #[error("the verified address does not match the signup")]
    RegistrantMismatch,
    /// The one-live-FAU-per-school insert lost a race, and the retry that follows
    /// (see `signup::create_pending_tenant`) also failed to either place the row or
    /// find a live competitor to report as a collision. Distinct from
    /// [`MembershipError::decode`]'s "the database gave us something we could not
    /// parse": this is "the database's state kept changing under us", a resolvable
    /// concurrency condition, not corrupted data or a schema drift.
    #[error("the signup could not be placed after a concurrent change")]
    SignupRaceUnresolved,

    // The FAU.
    #[error("FAU not found")]
    UnknownTenant,
    #[error("the FAU is not pending")]
    TenantNotPending,
    #[error("the FAU is not active")]
    TenantNotActive,
    #[error("the FAU is frozen")]
    TenantFrozen,

    // Authority.
    #[error("the actor lacks authority for this action")]
    NotAuthorized,
    #[error("an invitation to oneself is refused")]
    SelfInvitation,
    #[error("the account is disabled")]
    AccountDisabled,

    // Invitations.
    #[error("invitation not found")]
    UnknownInvitation,
    #[error("the invitation is no longer pending")]
    InvitationNotPending,
    #[error("the invitation was refused")]
    Acceptance(AcceptanceRefusal),
    #[error("an end-date change is not allowed for this invitation")]
    OverrideNotAllowed,

    // Roles and memberships.
    #[error("role not found")]
    UnknownRole,
    #[error("the role is not an admin role")]
    NotAdminRole,
    #[error("membership not found")]
    UnknownMembership,
    #[error("the membership is revoked")]
    MembershipRevoked,
    #[error("role assignment not found")]
    UnknownAssignment,
    #[error("the role assignment is already revoked")]
    AssignmentAlreadyRevoked,
    #[error("the role is not held today")]
    RoleNotHeldToday,
    #[error("the action would leave the FAU without an administrator")]
    WouldLeaveNoAdmin,
    #[error("the FAU has an administrator")]
    NotInNoAdminState,

    // Requests.
    #[error("replacement dates are invalid")]
    ReplacementDates(ReplacementDateError),
    #[error("request limit reached")]
    RequestLimit(RequestLimit),
    #[error("the message is too long")]
    MessageTooLong,
    #[error("the message contains a control character")]
    MessageControlCharacter,
    #[error("request not found")]
    UnknownRequest,
    #[error("the request is no longer pending")]
    RequestNotPending,

    // Infrastructure.
    #[error("the operating system's random source failed")]
    Randomness,
    /// `pool::safe_error_kind`'s fixed description: a SQLSTATE or a fixed word, never the
    /// driver's own message, which can quote bound values.
    #[error("database error ({0})")]
    Database(String),
}

impl From<sqlx::Error> for MembershipError {
    fn from(e: sqlx::Error) -> Self {
        MembershipError::Database(safe_error_kind(&e))
    }
}

/// Maps the domain's message-normalisation refusal onto its own variant, so later tasks
/// use `?`/`.into()` instead of collapsing both cases into one with `map_err`.
impl From<MessageError> for MembershipError {
    fn from(e: MessageError) -> Self {
        match e {
            MessageError::TooLong => MembershipError::MessageTooLong,
            MessageError::ControlCharacter => MembershipError::MessageControlCharacter,
        }
    }
}

impl MembershipError {
    /// A value read back from the database did not decode: a bug or a schema drift, not
    /// user input. Fixed text, like every other variant.
    pub(crate) fn decode() -> Self {
        MembershipError::Database("decode error".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_fixed_text() {
        assert_eq!(
            MembershipError::SchoolTaken(ExistingFau::Active).to_string(),
            "the school already has an FAU"
        );
        assert_eq!(
            MembershipError::Acceptance(AcceptanceRefusal::EmailMismatch).to_string(),
            "the invitation was refused"
        );
    }

    #[test]
    fn a_database_error_carries_only_the_sqlstate() {
        let e: MembershipError = crate::pool::test_support::database_error("23505").into();
        assert_eq!(e, MembershipError::Database("sqlstate 23505".to_owned()));
        assert_eq!(e.to_string(), "database error (sqlstate 23505)");
    }

    #[test]
    fn message_errors_map_to_their_own_variant() {
        assert_eq!(
            MembershipError::from(MessageError::TooLong),
            MembershipError::MessageTooLong
        );
        assert_eq!(
            MembershipError::from(MessageError::ControlCharacter),
            MembershipError::MessageControlCharacter
        );
    }
}
