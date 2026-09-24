//! Handover grants (spec 6.2) and the recovery contact's admin grant (spec 6.4).

use fau_domain::email::{Email, VerifiedEmail};
use fau_domain::membership::period::Period;
use fau_domain::membership::rules::{handover_period, EWB_OVERSIGHT_ADDRESS};
use fau_domain::membership::vocabulary::{
    CapabilityClass, InvitationMode, RecoveryHolder, TenantStatus,
};
use fau_domain::time::Moment;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::error::MembershipError;
use super::invitations::{
    insert_invitation, resolve_roles, IssuedInvitation, NewInvitation, OfferedRole, RoleChoice,
};
use super::sql::{
    date_param, enqueue, lock_tenant, parse_date, recovery_notice_recipients, require_open,
    write_audit, ActorKind, AdminState, Audit, USABLE_ACCOUNT,
};

/// The scheduled sweep: gives every naturally ended admin role its handover grant
/// (spec 6.2). A role that was revoked gets none. Idempotent -- a source assignment has
/// at most one grant -- and late-safe: a grant created days after the role ended still
/// runs from the role's end, and a window already over is skipped. Returns how many
/// grants were created.
///
/// Runs one tenant per transaction, taking `lock_tenant` first, the same rule every
/// other membership mutation follows (fix round 1, Critical #1). Without that lock, a
/// revocation could commit between this function reading its candidates and inserting
/// their grants: the revocation's own cascade (`revoke_role_assignment` ending any
/// grant sourced from the assignment it revokes) would find nothing yet to end, because
/// the grant did not exist when the revocation ran, and the sweep would then insert a
/// live grant behind a source that is already revoked. Locking the tenant first closes
/// the window: `revoke_role_assignment` and `revoke_membership` both take the same lock
/// before touching anything, so either they commit first and the sweep's own,
/// under-the-lock candidate read already reflects the revocation, or the sweep holds
/// the lock and they wait for it to finish.
///
/// The outer scan below that decides which tenants to visit is a plain, unlocked read:
/// it only narrows which tenants are worth locking at all, and every condition it
/// checks is re-checked, authoritatively, under that tenant's own lock before anything
/// is written.
pub async fn create_handover_grants(pool: &PgPool, at: Moment) -> Result<u64, MembershipError> {
    let tenant_ids: Vec<Uuid> = sqlx::query_scalar(
        "select distinct ra.tenant_id
           from role_assignments ra
           join roles r on r.tenant_id = ra.tenant_id and r.id = ra.role_id
          where r.capability_class = 'admin'
            and ra.revoked_at is null
            and ra.ends_on_exclusive <= $1::date
            and not exists (select 1 from handover_grants g
                             where g.tenant_id = ra.tenant_id and g.source_assignment_id = ra.id)",
    )
    .bind(date_param(at.today()))
    .fetch_all(pool)
    .await?;

    let mut created = 0;
    for tenant_id in tenant_ids {
        created += create_tenant_handover_grants(pool, tenant_id, at).await?;
    }
    Ok(created)
}

/// One tenant's share of the sweep, inside its own locked transaction. See
/// `create_handover_grants` for why the lock comes first.
async fn create_tenant_handover_grants(
    pool: &PgPool,
    tenant_id: Uuid,
    at: Moment,
) -> Result<u64, MembershipError> {
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, tenant_id).await?;
    // Re-checked under the lock: a tenant closed (or not yet active) between the outer
    // scan and here gets no grant.
    if state.status != TenantStatus::Active {
        tx.commit().await?;
        return Ok(0);
    }

    // Re-read, under the lock, rather than trusting the outer scan's candidates: an
    // assignment revoked or a membership/account made unusable since the outer scan
    // must not get a grant (fix round 1, Critical #1 and Minor #4). `USABLE_ACCOUNT`
    // covers "membership not revoked" and "account usable" in one place.
    let sql = format!(
        "select ra.id, to_char(ra.ends_on_exclusive, 'YYYY-MM-DD')
           from role_assignments ra
           join roles r       on r.tenant_id = ra.tenant_id and r.id = ra.role_id
           join memberships m on m.tenant_id = ra.tenant_id and m.id = ra.membership_id
           join accounts a    on a.id = m.account_id
          where ra.tenant_id = $1 and r.capability_class = 'admin'
            and ra.revoked_at is null and {USABLE_ACCOUNT}
            and ra.ends_on_exclusive <= $2::date
            and not exists (select 1 from handover_grants g
                             where g.tenant_id = ra.tenant_id and g.source_assignment_id = ra.id)"
    );
    let candidates: Vec<(Uuid, String)> = sqlx::query_as(&sql)
        .bind(tenant_id)
        .bind(date_param(at.today()))
        .fetch_all(&mut *tx)
        .await?;

    let mut created = 0;
    for (assignment_id, ends) in candidates {
        let Some(window) = handover_period(parse_date(&ends)?) else {
            continue;
        };
        if window.has_ended_by(at.today()) {
            continue;
        }
        let grant_id: Option<Uuid> = sqlx::query_scalar(
            "insert into handover_grants
               (tenant_id, id, source_assignment_id, starts_on, ends_on_exclusive)
             values ($1, $2, $3, $4::date, $5::date)
             on conflict (tenant_id, source_assignment_id) do nothing
             returning id",
        )
        .bind(tenant_id)
        .bind(Uuid::now_v7())
        .bind(assignment_id)
        .bind(date_param(window.starts_on()))
        .bind(date_param(window.ends_on_exclusive()))
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(grant_id) = grant_id {
            write_audit(
                &mut tx,
                at,
                Audit::system(
                    tenant_id,
                    "handover.granted",
                    "handover_grant",
                    grant_id,
                    json!({
                        "source_assignment_id": assignment_id,
                        "ends_on_exclusive": date_param(window.ends_on_exclusive()),
                    }),
                ),
            )
            .await?;
            created += 1;
        }
    }
    tx.commit().await?;
    Ok(created)
}

/// Who is acting as the recovery contact. How EWB's operator and a school
/// representative authenticate is #3417's; this names which seat they claim.
#[derive(Debug, Clone)]
pub enum RecoveryActor {
    Ewb,
    SchoolRep(VerifiedEmail),
}

#[derive(Debug, Clone)]
pub struct RecoveryGrant {
    pub tenant_id: Uuid,
    pub actor: RecoveryActor,
    pub recipient: Email,
    /// Must resolve to an `admin`-class role.
    pub role: RoleChoice,
    pub period: Period,
}

/// The recovery contact's one power in the no-admin state (spec 6.4, decision 10):
/// issues a `recovery` invitation offering an admin-class role. Refused unless the
/// actor holds the seat and the FAU has no admin today. The recovery contact cannot
/// invite itself (ADR-003 decision 8).
///
/// Writes a permanent audit entry and a notice to every current member -- or, with no
/// members left, to recent role-holders -- and always to EWB as the second party. The
/// same notice goes out again on acceptance (`accept_invitation`). The 14-day login
/// banner is read from these audit entries by the UI (#3422).
///
/// **Gate:** #3414's step-up re-authentication applies (ADR-003 decision 10) and is the
/// HTTP layer's (#3417).
pub async fn recovery_grant_admin(
    pool: &PgPool,
    req: RecoveryGrant,
    at: Moment,
) -> Result<IssuedInvitation, MembershipError> {
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, req.tenant_id).await?;
    require_open(&state)?;

    let seat: Option<(String, Option<String>)> = sqlx::query_as(
        "select holder, nominee_email from recovery_contacts where tenant_id = $1 for update",
    )
    .bind(req.tenant_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (holder, nominee) = seat.ok_or(MembershipError::NotAuthorized)?;
    let holder = RecoveryHolder::from_code(&holder).ok_or_else(MembershipError::decode)?;
    let (actor_kind, own_address) = match (&req.actor, holder) {
        (RecoveryActor::Ewb, RecoveryHolder::Ewb) => {
            (ActorKind::RecoveryEwb, EWB_OVERSIGHT_ADDRESS.to_owned())
        }
        (RecoveryActor::SchoolRep(email), RecoveryHolder::SchoolRep)
            if nominee.as_deref() == Some(email.email().as_str()) =>
        {
            (
                ActorKind::RecoverySchoolRep,
                email.email().as_str().to_owned(),
            )
        }
        _ => return Err(MembershipError::NotAuthorized),
    };
    // The recovery contact's one power is granting an existing role (ADR-003 decision
    // 8); creating one edits the organisation, which is beyond it -- the same
    // restriction `issue_invitation` places on a handover grant (fix round 1,
    // Important #2).
    if matches!(req.role, RoleChoice::New { .. }) {
        return Err(MembershipError::NotAuthorized);
    }
    if req.recipient.as_str() == own_address {
        return Err(MembershipError::SelfInvitation);
    }
    if AdminState::load(&mut tx, req.tenant_id)
        .await?
        .has_admin(at.today())
    {
        return Err(MembershipError::NotInNoAdminState);
    }

    let resolved = resolve_roles(
        &mut tx,
        at,
        req.tenant_id,
        None,
        &[OfferedRole {
            role: req.role,
            period: req.period,
        }],
    )
    .await?;
    let (role_id, capability, period) = resolved[0];
    if capability != CapabilityClass::Admin {
        return Err(MembershipError::NotAdminRole);
    }

    let recipients = recovery_notice_recipients(&mut tx, req.tenant_id, at.today()).await?;
    let issued = insert_invitation(
        &mut tx,
        at,
        NewInvitation {
            tenant_id: req.tenant_id,
            mode: InvitationMode::Recovery,
            recipient: req.recipient,
            issued_by: None,
            handover_grant_id: None,
            access_request_id: None,
            recovery_holder: Some(holder),
            roles: vec![(role_id, period)],
            actor_kind,
            actor_membership_id: None,
        },
    )
    .await?;
    write_audit(
        &mut tx,
        at,
        Audit {
            tenant_id: Some(req.tenant_id),
            actor_kind,
            actor_account_id: None,
            actor_membership_id: None,
            action: "recovery.admin_invited",
            subject_type: "invitation",
            subject_id: issued.invitation_id,
            params: json!({ "holder": holder.code(), "role_id": role_id }),
        },
    )
    .await?;
    for recipient in &recipients {
        enqueue(
            &mut tx,
            at,
            "recovery.invitation_created",
            recipient,
            json!({ "tenant_id": req.tenant_id, "invitation_id": issued.invitation_id }),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(issued)
}
