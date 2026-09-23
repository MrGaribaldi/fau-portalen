//! Invitations (spec 5.1): issue, re-send, withdraw and accept.

use fau_domain::email::Email;
use fau_domain::membership::period::Period;
use fau_domain::membership::rules::invitation_expiry;
use fau_domain::membership::vocabulary::{InvitationMode, RecoveryHolder};
use fau_domain::time::Moment;
use jiff::Timestamp;
use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

use super::error::MembershipError;
use super::sql::{date_param, enqueue, ts_param, write_audit, ActorKind, Audit};
use super::token::InvitationToken;

/// A newly issued (or re-sent) invitation. `token` is the only copy of the raw token.
#[derive(Debug)]
pub struct IssuedInvitation {
    pub invitation_id: Uuid,
    pub token: InvitationToken,
    pub expires_at: Timestamp,
}

pub(crate) struct NewInvitation {
    pub(crate) tenant_id: Uuid,
    pub(crate) mode: InvitationMode,
    pub(crate) recipient: Email,
    pub(crate) issued_by: Option<Uuid>,
    pub(crate) handover_grant_id: Option<Uuid>,
    pub(crate) access_request_id: Option<Uuid>,
    pub(crate) recovery_holder: Option<RecoveryHolder>,
    pub(crate) roles: Vec<(Uuid, Period)>,
    pub(crate) actor_kind: ActorKind,
    pub(crate) actor_membership_id: Option<Uuid>,
}

/// Writes an invitation, its roles, its audit entry and its outbox message. Every path
/// that issues an invitation -- activation, an admin, a handover grant, an approved
/// request, the recovery contact -- goes through here, inside the caller's transaction.
pub(crate) async fn insert_invitation(
    conn: &mut PgConnection,
    at: Moment,
    new: NewInvitation,
) -> Result<IssuedInvitation, MembershipError> {
    let token = InvitationToken::generate()?;
    let invitation_id = Uuid::now_v7();
    let expires_at = invitation_expiry(at.now());
    sqlx::query(
        "insert into invitations
           (tenant_id, id, token_hash, mode, recipient_email, issued_by, handover_grant_id,
            access_request_id, recovery_holder, expires_at, created_at)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::timestamptz, $11::timestamptz)",
    )
    .bind(new.tenant_id)
    .bind(invitation_id)
    .bind(token.hash())
    .bind(new.mode.code())
    .bind(new.recipient.as_str())
    .bind(new.issued_by)
    .bind(new.handover_grant_id)
    .bind(new.access_request_id)
    .bind(new.recovery_holder.map(RecoveryHolder::code))
    .bind(ts_param(expires_at))
    .bind(ts_param(at.now()))
    .execute(&mut *conn)
    .await?;
    for (role_id, period) in &new.roles {
        sqlx::query(
            "insert into invitation_roles
               (tenant_id, invitation_id, role_id, starts_on, ends_on_exclusive)
             values ($1, $2, $3, $4::date, $5::date)",
        )
        .bind(new.tenant_id)
        .bind(invitation_id)
        .bind(role_id)
        .bind(date_param(period.starts_on()))
        .bind(date_param(period.ends_on_exclusive()))
        .execute(&mut *conn)
        .await?;
    }
    write_audit(
        conn,
        at,
        Audit {
            tenant_id: Some(new.tenant_id),
            actor_kind: new.actor_kind,
            actor_account_id: None,
            actor_membership_id: new.actor_membership_id,
            action: "invitation.issued",
            subject_type: "invitation",
            subject_id: invitation_id,
            params: json!({ "mode": new.mode.code(), "role_count": new.roles.len() }),
        },
    )
    .await?;
    enqueue(
        conn,
        at,
        "invitation.issued",
        new.recipient.as_str(),
        json!({
            "tenant_id": new.tenant_id,
            "invitation_id": invitation_id,
            "mode": new.mode.code(),
        }),
    )
    .await?;
    Ok(IssuedInvitation {
        invitation_id,
        token,
        expires_at,
    })
}
