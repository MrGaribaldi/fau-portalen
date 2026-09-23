//! Invitations (spec 5.1): issue, re-send, withdraw and accept.

use std::collections::HashSet;
use std::fmt;

use fau_domain::email::{Email, VerifiedEmail};
use fau_domain::membership::acceptance::{check_acceptance, AcceptanceSnapshot};
use fau_domain::membership::period::Period;
use fau_domain::membership::rules::{invitation_expiry, validate_admin_end};
use fau_domain::membership::vocabulary::{
    CapabilityClass, InvitationMode, RecoveryHolder, RoleName,
};
use fau_domain::time::Moment;
use jiff::civil::Date;
use jiff::Timestamp;
use serde_json::json;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::error::MembershipError;
use super::sql::{
    date_param, enqueue, ensure_membership, from_micros, handover_grant_valid, insert_assignment,
    is_admin_today, lock_tenant, membership_usable, period_from, recovery_notice_recipients,
    require_admin, require_open, ts_param, upsert_verified_account, usable_member_email,
    write_audit, ActorKind, AdminState, Audit,
};
use super::token::{hash_token, looks_like_token, InvitationToken};

/// What an issuer offers: an existing role, or a new one created with the invitation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoleChoice {
    Existing(Uuid),
    New {
        name: RoleName,
        capability: CapabilityClass,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferedRole {
    pub role: RoleChoice,
    pub period: Period,
}

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

/// Resolves offered roles to role ids, creating new ones, and rejects an empty list, a
/// duplicate, an unknown role or a period that has already ended.
pub(crate) async fn resolve_roles(
    conn: &mut PgConnection,
    at: Moment,
    tenant_id: Uuid,
    actor_membership_id: Option<Uuid>,
    offered: &[OfferedRole],
) -> Result<Vec<(Uuid, CapabilityClass, Period)>, MembershipError> {
    if offered.is_empty() {
        return Err(MembershipError::NoRolesOffered);
    }
    let mut seen = HashSet::new();
    let mut resolved = Vec::with_capacity(offered.len());
    for o in offered {
        if o.period.has_ended_by(at.today()) {
            return Err(MembershipError::PeriodAlreadyEnded);
        }
        let (role_id, capability) = match &o.role {
            RoleChoice::Existing(id) => {
                let class: Option<String> = sqlx::query_scalar(
                    "select capability_class from roles where tenant_id = $1 and id = $2",
                )
                .bind(tenant_id)
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?;
                let class = class.ok_or(MembershipError::UnknownRole)?;
                (
                    *id,
                    CapabilityClass::from_code(&class).ok_or_else(MembershipError::decode)?,
                )
            }
            RoleChoice::New { name, capability } => {
                let id = Uuid::now_v7();
                sqlx::query(
                    "insert into roles (tenant_id, id, name, capability_class) values ($1, $2, $3, $4)",
                )
                .bind(tenant_id)
                .bind(id)
                .bind(name.as_str())
                .bind(capability.code())
                .execute(&mut *conn)
                .await?;
                write_audit(
                    conn,
                    at,
                    Audit {
                        tenant_id: Some(tenant_id),
                        actor_kind: if actor_membership_id.is_some() {
                            ActorKind::Member
                        } else {
                            ActorKind::System
                        },
                        actor_account_id: None,
                        actor_membership_id,
                        action: "role.created",
                        subject_type: "role",
                        subject_id: id,
                        params: json!({ "capability_class": capability.code() }),
                    },
                )
                .await?;
                (id, *capability)
            }
        };
        if !seen.insert(role_id) {
            return Err(MembershipError::DuplicateRole);
        }
        resolved.push((role_id, capability, o.period));
    }
    Ok(resolved)
}

#[derive(Debug, Clone)]
pub struct IssueInvitation {
    pub tenant_id: Uuid,
    pub actor_membership_id: Uuid,
    pub recipient: Email,
    pub roles: Vec<OfferedRole>,
    /// `Some` issues a `handover` invitation under the actor's own handover grant
    /// (spec 6.3); `None` issues a `normal` one and requires an admin role valid today.
    pub handover_grant_id: Option<Uuid>,
}

/// Issues an invitation (spec 5.1, 6.3).
///
/// **Gate:** the caller must already have passed #3414's second-factor and freshness
/// gate; the HTTP layer enforces it (#3417). This function verifies authority in the
/// database -- an admin role valid today, or the actor's own handover grant valid today
/// -- and nothing about the session.
///
/// Under a handover grant the actor may offer only existing roles (creating a role
/// edits the organisation, which handover does not allow) and may not invite their own
/// address (no extending one's own role).
pub async fn issue_invitation(
    pool: &PgPool,
    req: IssueInvitation,
    at: Moment,
) -> Result<IssuedInvitation, MembershipError> {
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, req.tenant_id).await?;
    require_open(&state)?;

    let mode = match req.handover_grant_id {
        None => {
            require_admin(&mut tx, req.tenant_id, req.actor_membership_id, at.today()).await?;
            InvitationMode::Normal
        }
        Some(grant_id) => {
            let valid = handover_grant_valid(
                &mut tx,
                req.tenant_id,
                grant_id,
                req.actor_membership_id,
                at.today(),
            )
            .await?;
            if !valid {
                return Err(MembershipError::NotAuthorized);
            }
            if req
                .roles
                .iter()
                .any(|r| matches!(r.role, RoleChoice::New { .. }))
            {
                return Err(MembershipError::NotAuthorized);
            }
            let own = usable_member_email(&mut tx, req.tenant_id, req.actor_membership_id)
                .await?
                .ok_or(MembershipError::NotAuthorized)?;
            if own == req.recipient.as_str() {
                return Err(MembershipError::SelfInvitation);
            }
            InvitationMode::Handover
        }
    };

    let roles = resolve_roles(
        &mut tx,
        at,
        req.tenant_id,
        Some(req.actor_membership_id),
        &req.roles,
    )
    .await?;
    let issued = insert_invitation(
        &mut tx,
        at,
        NewInvitation {
            tenant_id: req.tenant_id,
            mode,
            recipient: req.recipient,
            issued_by: Some(req.actor_membership_id),
            handover_grant_id: req.handover_grant_id,
            access_request_id: None,
            recovery_holder: None,
            roles: roles.into_iter().map(|(id, _, p)| (id, p)).collect(),
            actor_kind: ActorKind::Member,
            actor_membership_id: Some(req.actor_membership_id),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(issued)
}

struct InvitationRow {
    mode: InvitationMode,
    recipient_email: String,
    issued_by: Option<Uuid>,
    handover_grant_id: Option<Uuid>,
    expires_at: Timestamp,
    accepted: bool,
    revoked: bool,
}

async fn lock_invitation(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    invitation_id: Uuid,
    token_hash: Option<&[u8]>,
) -> Result<Option<InvitationRow>, MembershipError> {
    type Row = (String, String, Option<Uuid>, Option<Uuid>, i64, bool, bool);
    let row: Option<Row> = sqlx::query_as(
        "select mode, recipient_email, issued_by, handover_grant_id,
                (extract(epoch from expires_at) * 1000000)::bigint,
                accepted_at is not null, revoked_at is not null
           from invitations
          where tenant_id = $1 and id = $2 and ($3::bytea is null or token_hash = $3)
          for update",
    )
    .bind(tenant_id)
    .bind(invitation_id)
    .bind(token_hash)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((mode, recipient_email, issued_by, handover_grant_id, expires_us, accepted, revoked)) =
        row
    else {
        return Ok(None);
    };
    Ok(Some(InvitationRow {
        mode: InvitationMode::from_code(&mode).ok_or_else(MembershipError::decode)?,
        recipient_email,
        issued_by,
        handover_grant_id,
        expires_at: from_micros(expires_us)?,
        accepted,
        revoked,
    }))
}

/// Re-sending or withdrawing a pending invitation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvitationChange {
    pub tenant_id: Uuid,
    pub actor_membership_id: Uuid,
    pub invitation_id: Uuid,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChangeKind {
    Resend,
    Withdraw,
}

/// Any admin valid today may re-send or withdraw any pending invitation. Otherwise the
/// issuer may: a handover issuer while their grant is valid (spec 6.3), a normal issuer
/// only to withdraw (spec 5.1: "the issuer or any admin can withdraw"). Withdrawing does
/// not need current authority at all, only that the actor was the issuer and their own
/// membership is still usable: a normal issuer who has since lost admin, or a handover
/// issuer whose grant has since ended, may still withdraw their own invitation -- the
/// same "removing a way in never needs standing authority" reasoning that lets
/// [`withdraw_invitation`] run on a frozen FAU.
///
/// Takes `inv` as `Option` and runs before the caller has decided whether the
/// invitation exists or is still pending, so an unauthorized actor always gets
/// [`MembershipError::NotAuthorized`], never a hint about the invitation's state
/// (controller ruling, fix round 1): an admin passes on identity alone; anyone else
/// needs the invitation row to prove they were its issuer, so a missing invitation
/// falls through to `NotAuthorized` rather than revealing "not found".
async fn authorize_change(
    conn: &mut PgConnection,
    change: &InvitationChange,
    inv: Option<&InvitationRow>,
    today: Date,
    kind: ChangeKind,
) -> Result<(), MembershipError> {
    let (tenant_id, actor) = (change.tenant_id, change.actor_membership_id);
    if is_admin_today(conn, tenant_id, actor, today).await? {
        return Ok(());
    }
    let Some(inv) = inv else {
        return Err(MembershipError::NotAuthorized);
    };
    if inv.issued_by != Some(actor) {
        return Err(MembershipError::NotAuthorized);
    }
    let allowed = match (inv.mode, inv.handover_grant_id) {
        (InvitationMode::Handover, Some(grant_id)) => match kind {
            ChangeKind::Resend => {
                handover_grant_valid(conn, tenant_id, grant_id, actor, today).await?
            }
            ChangeKind::Withdraw => membership_usable(conn, tenant_id, actor).await?,
        },
        (InvitationMode::Normal, _) => {
            kind == ChangeKind::Withdraw && membership_usable(conn, tenant_id, actor).await?
        }
        _ => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(MembershipError::NotAuthorized)
    }
}

/// Re-sends a pending invitation: a new token replaces the old one, which stops
/// working at once, and the 14 days restart (spec 5.1).
///
/// **Gate:** as [`issue_invitation`].
pub async fn resend_invitation(
    pool: &PgPool,
    change: InvitationChange,
    at: Moment,
) -> Result<IssuedInvitation, MembershipError> {
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, change.tenant_id).await?;
    let inv = lock_invitation(&mut tx, change.tenant_id, change.invitation_id, None).await?;
    // Authority first: an unauthorized actor gets `NotAuthorized` whatever the
    // invitation's state, never a hint that it doesn't exist or is already settled.
    authorize_change(
        &mut tx,
        &change,
        inv.as_ref(),
        at.today(),
        ChangeKind::Resend,
    )
    .await?;
    require_open(&state)?;
    let inv = inv.ok_or(MembershipError::UnknownInvitation)?;
    if inv.accepted || inv.revoked {
        return Err(MembershipError::InvitationNotPending);
    }

    let token = InvitationToken::generate()?;
    let expires_at = invitation_expiry(at.now());
    // The old token's hash is gone the instant this commits, so accepting it afterwards
    // finds no row at all and reports `UnknownInvitation`, never a "replaced" refusal.
    sqlx::query(
        "update invitations set token_hash = $3, expires_at = $4::timestamptz
          where tenant_id = $1 and id = $2",
    )
    .bind(change.tenant_id)
    .bind(change.invitation_id)
    .bind(token.hash())
    .bind(ts_param(expires_at))
    .execute(&mut *tx)
    .await?;
    write_audit(
        &mut tx,
        at,
        Audit::member(
            change.tenant_id,
            change.actor_membership_id,
            "invitation.resent",
            "invitation",
            change.invitation_id,
            json!({ "mode": inv.mode.code() }),
        ),
    )
    .await?;
    enqueue(
        &mut tx,
        at,
        "invitation.issued",
        &inv.recipient_email,
        json!({
            "tenant_id": change.tenant_id,
            "invitation_id": change.invitation_id,
            "mode": inv.mode.code(),
        }),
    )
    .await?;
    tx.commit().await?;
    Ok(IssuedInvitation {
        invitation_id: change.invitation_id,
        token,
        expires_at,
    })
}

/// Withdraws a pending invitation (spec 5.1). Allowed while the FAU is frozen: it only
/// ever removes a way in.
///
/// **Gate:** as [`issue_invitation`].
pub async fn withdraw_invitation(
    pool: &PgPool,
    change: InvitationChange,
    at: Moment,
) -> Result<(), MembershipError> {
    let mut tx = pool.begin().await?;
    lock_tenant(&mut tx, change.tenant_id).await?;
    let inv = lock_invitation(&mut tx, change.tenant_id, change.invitation_id, None).await?;
    // Authority first: see the comment on `authorize_change`.
    authorize_change(
        &mut tx,
        &change,
        inv.as_ref(),
        at.today(),
        ChangeKind::Withdraw,
    )
    .await?;
    let inv = inv.ok_or(MembershipError::UnknownInvitation)?;
    if inv.accepted || inv.revoked {
        return Err(MembershipError::InvitationNotPending);
    }

    sqlx::query(
        "update invitations set revoked_at = $3::timestamptz where tenant_id = $1 and id = $2",
    )
    .bind(change.tenant_id)
    .bind(change.invitation_id)
    .bind(ts_param(at.now()))
    .execute(&mut *tx)
    .await?;
    write_audit(
        &mut tx,
        at,
        Audit::member(
            change.tenant_id,
            change.actor_membership_id,
            "invitation.withdrawn",
            "invitation",
            change.invitation_id,
            json!({ "mode": inv.mode.code() }),
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

#[derive(Clone)]
pub struct AcceptInvitation {
    /// The raw token from the link.
    pub token: String,
    /// The address the person just verified with a passcode (#3417).
    pub acceptor: VerifiedEmail,
    /// The leader's own choice of end date for the admin role an `activation`
    /// invitation offers (spec 3.4.3, 3.5), within the same 1–24 month range. Refused on
    /// any other mode.
    pub admin_end_override: Option<Date>,
}

/// Hand-written so `token` never reaches a log line the way a derived `Debug` would.
impl fmt::Debug for AcceptInvitation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AcceptInvitation")
            .field("token", &"[redacted]")
            .field("acceptor", &self.acceptor)
            .field("admin_end_override", &self.admin_end_override)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accepted {
    pub tenant_id: Uuid,
    pub account_id: Uuid,
    pub membership_id: Uuid,
    pub assignment_ids: Vec<Uuid>,
}

/// Accepts an invitation in one transaction (spec 5.1): every check in
/// `fau_domain::membership::acceptance`, then the account and membership (created or
/// reused), the role assignments, the acceptance mark and the audit entry. A recovery
/// invitation also notifies every current member (spec 6.4.4).
///
/// Opening the link calls nothing here; only the explicit "Bli med" does.
pub async fn accept_invitation(
    pool: &PgPool,
    req: AcceptInvitation,
    at: Moment,
) -> Result<Accepted, MembershipError> {
    if !looks_like_token(&req.token) {
        return Err(MembershipError::UnknownInvitation);
    }
    let hash = hash_token(&req.token);
    let mut tx = pool.begin().await?;

    let found: Option<(Uuid, Uuid)> =
        sqlx::query_as("select tenant_id, id from invitations where token_hash = $1")
            .bind(&hash)
            .fetch_optional(&mut *tx)
            .await?;
    let (tenant_id, invitation_id) = found.ok_or(MembershipError::UnknownInvitation)?;
    // Lock order everywhere is tenant, then invitation. The hash is re-checked under the
    // lock, in case a re-send replaced it between the two reads.
    let state = lock_tenant(&mut tx, tenant_id).await?;
    let inv = lock_invitation(&mut tx, tenant_id, invitation_id, Some(&hash))
        .await?
        .ok_or(MembershipError::UnknownInvitation)?;

    let acceptor_email = req.acceptor.email().as_str().to_owned();
    let account_disabled: Option<bool> =
        sqlx::query_scalar("select disabled_at is not null from accounts where email = $1")
            .bind(&acceptor_email)
            .fetch_optional(&mut *tx)
            .await?;

    let issuer_admin_today = match (inv.mode, inv.issued_by) {
        (InvitationMode::Normal, Some(issuer)) => {
            is_admin_today(&mut tx, tenant_id, issuer, at.today()).await?
        }
        _ => false,
    };
    let handover_grant_valid_today = match (inv.mode, inv.handover_grant_id, inv.issued_by) {
        (InvitationMode::Handover, Some(grant_id), Some(issuer)) => {
            handover_grant_valid(&mut tx, tenant_id, grant_id, issuer, at.today()).await?
        }
        _ => false,
    };
    let tenant_has_admin_today = if inv.mode == InvitationMode::Recovery {
        AdminState::load(&mut tx, tenant_id)
            .await?
            .has_admin(at.today())
    } else {
        true
    };

    let snapshot = AcceptanceSnapshot {
        mode: inv.mode,
        expires_at: inv.expires_at,
        accepted: inv.accepted,
        revoked: inv.revoked,
        recipient: Email::parse(&inv.recipient_email).map_err(|_| MembershipError::decode())?,
        acceptor: req.acceptor.clone(),
        acceptor_account_disabled: account_disabled.unwrap_or(false),
        tenant_status: state.status,
        tenant_frozen: state.frozen,
        issuer_admin_today,
        handover_grant_valid_today,
        tenant_has_admin_today,
    };
    check_acceptance(&snapshot, at.now()).map_err(MembershipError::Acceptance)?;

    let rows: Vec<(Uuid, String, String, String)> = sqlx::query_as(
        "select ir.role_id, r.capability_class,
                to_char(ir.starts_on, 'YYYY-MM-DD'), to_char(ir.ends_on_exclusive, 'YYYY-MM-DD')
           from invitation_roles ir
           join roles r on r.tenant_id = ir.tenant_id and r.id = ir.role_id
          where ir.tenant_id = $1 and ir.invitation_id = $2
          order by ir.role_id",
    )
    .bind(tenant_id)
    .bind(invitation_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut roles = Vec::with_capacity(rows.len());
    for (role_id, class, starts, ends) in rows {
        let class = CapabilityClass::from_code(&class).ok_or_else(MembershipError::decode)?;
        roles.push((role_id, class, period_from(&starts, &ends)?));
    }

    if let Some(end) = req.admin_end_override {
        if inv.mode != InvitationMode::Activation {
            return Err(MembershipError::OverrideNotAllowed);
        }
        validate_admin_end(at.today(), end).map_err(MembershipError::AdminEnd)?;
        for (_, class, period) in roles.iter_mut() {
            if *class == CapabilityClass::Admin {
                *period = Period::new(period.starts_on(), end)
                    .map_err(|_| MembershipError::EmptyPeriod)?;
            }
        }
    }

    // Recovery notices go to the members as they stood before this acceptance.
    let recovery_recipients = if inv.mode == InvitationMode::Recovery {
        recovery_notice_recipients(&mut tx, tenant_id, at.today()).await?
    } else {
        Vec::new()
    };

    let (account_id, _) = upsert_verified_account(&mut tx, &acceptor_email, at).await?;
    let (membership_id, reused) = ensure_membership(&mut tx, tenant_id, account_id).await?;
    let mut assignment_ids = Vec::with_capacity(roles.len());
    for (role_id, _, period) in &roles {
        assignment_ids.push(
            insert_assignment(
                &mut tx,
                tenant_id,
                membership_id,
                *role_id,
                *period,
                inv.issued_by,
            )
            .await?,
        );
    }
    sqlx::query(
        "update invitations set accepted_at = $3::timestamptz, accepted_membership_id = $4
          where tenant_id = $1 and id = $2",
    )
    .bind(tenant_id)
    .bind(invitation_id)
    .bind(ts_param(at.now()))
    .bind(membership_id)
    .execute(&mut *tx)
    .await?;
    write_audit(
        &mut tx,
        at,
        Audit {
            tenant_id: Some(tenant_id),
            actor_kind: ActorKind::Member,
            actor_account_id: Some(account_id),
            actor_membership_id: Some(membership_id),
            action: "invitation.accepted",
            subject_type: "invitation",
            subject_id: invitation_id,
            params: json!({
                "mode": inv.mode.code(),
                "membership_reused": reused,
                "assignment_count": assignment_ids.len(),
            }),
        },
    )
    .await?;
    for recipient in &recovery_recipients {
        enqueue(
            &mut tx,
            at,
            "recovery.invitation_accepted",
            recipient,
            json!({ "tenant_id": tenant_id, "invitation_id": invitation_id }),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Accepted {
        tenant_id,
        account_id,
        membership_id,
        assignment_ids,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every other struct in this module either carries no raw secret (`IssueInvitation`,
    /// `InvitationChange`, `OfferedRole`, `RoleChoice`, `Accepted`) or already redacts one
    /// through its own type (`IssuedInvitation`'s `InvitationToken`). `AcceptInvitation`
    /// is the one place a raw token string crosses this module's boundary.
    #[test]
    fn accept_invitation_debug_redacts_the_token() {
        let token = "a".repeat(64);
        let req = AcceptInvitation {
            token: token.clone(),
            acceptor: VerifiedEmail::from_provider(Email::parse("ny@example.test").unwrap()),
            admin_end_override: None,
        };
        let debug = format!("{req:?}");
        assert!(!debug.contains(&token));
        assert!(debug.contains("[redacted]"));
    }
}
