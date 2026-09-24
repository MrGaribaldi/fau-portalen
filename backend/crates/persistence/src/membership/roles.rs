//! Granting and revoking roles and memberships, with the last-admin safeguard (spec 7).

use fau_domain::membership::period::Period;
use fau_domain::membership::rules::handover_period;
use fau_domain::time::Moment;
use serde_json::json;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::error::MembershipError;
use super::invitations::{resolve_roles, OfferedRole, RoleChoice};
use super::sql::{
    check_last_admin, date_param, insert_assignment, is_admin_today, lock_tenant,
    membership_and_account_state, parse_date, require_admin, require_open, ts_param, write_audit,
    AdminState, Audit,
};

#[derive(Debug, Clone)]
pub struct GrantRole {
    pub tenant_id: Uuid,
    pub actor_membership_id: Uuid,
    pub membership_id: Uuid,
    pub role: RoleChoice,
    pub period: Period,
}

/// Gives an existing member a role for a period.
///
/// **Gate:** the caller must already have passed #3414's second-factor and freshness
/// gate (#3417). Only an admin valid today may grant; a handover grant does not suffice,
/// which is also what stops an outgoing admin extending their own role (spec 6.3).
pub async fn grant_role(
    pool: &PgPool,
    req: GrantRole,
    at: Moment,
) -> Result<Uuid, MembershipError> {
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, req.tenant_id).await?;
    require_open(&state)?;
    require_admin(&mut tx, req.tenant_id, req.actor_membership_id, at.today()).await?;

    let target = membership_and_account_state(&mut tx, req.tenant_id, req.membership_id).await?;
    match target {
        None => return Err(MembershipError::UnknownMembership),
        Some((true, _)) => return Err(MembershipError::MembershipRevoked),
        Some((false, false)) => return Err(MembershipError::AccountDisabled),
        Some((false, true)) => {}
    }

    let resolved = resolve_roles(
        &mut tx,
        at,
        req.tenant_id,
        Some(req.actor_membership_id),
        &[OfferedRole {
            role: req.role,
            period: req.period,
        }],
    )
    .await?;
    let (role_id, capability, period) = resolved[0];
    let assignment_id = insert_assignment(
        &mut tx,
        req.tenant_id,
        req.membership_id,
        role_id,
        period,
        Some(req.actor_membership_id),
    )
    .await?;
    write_audit(
        &mut tx,
        at,
        Audit::member(
            req.tenant_id,
            req.actor_membership_id,
            "role.granted",
            "role_assignment",
            assignment_id,
            json!({
                "membership_id": req.membership_id,
                "role_id": role_id,
                "capability_class": capability.code(),
                "starts_on": date_param(period.starts_on()),
                "ends_on_exclusive": date_param(period.ends_on_exclusive()),
            }),
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(assignment_id)
}

#[derive(Debug, Clone, Copy)]
pub struct RevokeAssignment {
    pub tenant_id: Uuid,
    pub actor_membership_id: Uuid,
    pub assignment_id: Uuid,
    /// The explicit confirmation of "FAU-et får da ingen administrator" (spec 7).
    /// Without it, a revocation that would leave no admin is refused.
    pub confirm_no_admin: bool,
}

/// Revokes one role assignment, at once, together with any handover grant it already
/// produced (#3412, spec 6.2). Creates no handover grant (spec 7).
///
/// An admin valid today may revoke any assignment; anyone may revoke their own -- a
/// member may step down from a single role without being an admin.
///
/// No `require_open` here: revoking only ever reduces rights, so this is allowed on a
/// frozen FAU too -- the same reasoning as `withdraw_invitation` and `decline_request`
/// (controller ruling, fix round 1).
///
/// **Authority before state** (controller ruling, fix round 1): admin-ness is decided
/// first, and a non-admin's target is looked up scoped to their own membership, so an
/// unknown id or someone else's assignment -- revoked or not -- both come back as
/// [`MembershipError::NotAuthorized`], never a hint about whether the id exists or is
/// already settled. Only an admin, who may target anyone, can learn
/// [`MembershipError::UnknownAssignment`] or [`MembershipError::AssignmentAlreadyRevoked`].
///
/// **Gate:** as [`grant_role`].
pub async fn revoke_role_assignment(
    pool: &PgPool,
    req: RevokeAssignment,
    at: Moment,
) -> Result<(), MembershipError> {
    let mut tx = pool.begin().await?;
    lock_tenant(&mut tx, req.tenant_id).await?;
    let admin = is_admin_today(&mut tx, req.tenant_id, req.actor_membership_id, at.today()).await?;
    let row: Option<(Uuid, bool)> = sqlx::query_as(
        "select membership_id, revoked_at is not null from role_assignments
          where tenant_id = $1 and id = $2 and ($3 or membership_id = $4) for update",
    )
    .bind(req.tenant_id)
    .bind(req.assignment_id)
    .bind(admin)
    .bind(req.actor_membership_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (holder, revoked) = match row {
        Some(r) => r,
        None if admin => return Err(MembershipError::UnknownAssignment),
        None => return Err(MembershipError::NotAuthorized),
    };
    if revoked {
        return Err(MembershipError::AssignmentAlreadyRevoked);
    }

    let admins = AdminState::load(&mut tx, req.tenant_id).await?;
    let leaves_none = check_last_admin(&admins, at.today(), req.confirm_no_admin, |id, _| {
        id == req.assignment_id
    })?;

    let now = ts_param(at.now());
    sqlx::query(
        "update role_assignments set revoked_at = $3::timestamptz where tenant_id = $1 and id = $2",
    )
    .bind(req.tenant_id)
    .bind(req.assignment_id)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "update handover_grants set revoked_at = $3::timestamptz
          where tenant_id = $1 and source_assignment_id = $2 and revoked_at is null",
    )
    .bind(req.tenant_id)
    .bind(req.assignment_id)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    write_audit(
        &mut tx,
        at,
        Audit::member(
            req.tenant_id,
            req.actor_membership_id,
            "role.revoked",
            "role_assignment",
            req.assignment_id,
            json!({ "membership_id": holder, "left_no_admin": leaves_none }),
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub struct RevokeMembership {
    pub tenant_id: Uuid,
    pub actor_membership_id: Uuid,
    /// Equal to `actor_membership_id` when a member leaves (spec 7).
    pub membership_id: Uuid,
    pub confirm_no_admin: bool,
}

/// Revokes a whole membership: an admin removing someone, or a member leaving. Ends
/// every current and future role the membership holds, every already-ended admin role
/// whose handover window is still open (so a later re-invite cannot revive it, final
/// review I1), and every handover grant derived from any of its roles, at once. The
/// member's own pending requests and proposals are withdrawn with it. Other FAU-er the
/// account belongs to are untouched (spec 7, ADR-003 decision 6a).
///
/// No `require_open` here: revoking only ever reduces rights, so this is allowed on a
/// frozen FAU too -- the same reasoning as `withdraw_invitation` and `decline_request`
/// (controller ruling, fix round 1).
///
/// **Authority before state** (controller ruling, fix round 1): when the actor is not
/// leaving themselves, admin authority is checked before the target membership is
/// looked up at all, so a non-admin acting on someone else's membership always gets
/// [`MembershipError::NotAuthorized`], never a hint about whether it exists or is
/// already revoked.
///
/// **Gate:** as [`grant_role`] when an admin removes someone. Leaving is not a
/// privileged admin action and needs no gate.
pub async fn revoke_membership(
    pool: &PgPool,
    req: RevokeMembership,
    at: Moment,
) -> Result<(), MembershipError> {
    let mut tx = pool.begin().await?;
    lock_tenant(&mut tx, req.tenant_id).await?;
    let leaving = req.actor_membership_id == req.membership_id;
    if !leaving {
        require_admin(&mut tx, req.tenant_id, req.actor_membership_id, at.today()).await?;
    }
    let target: Option<bool> = sqlx::query_scalar(
        "select revoked_at is not null from memberships where tenant_id = $1 and id = $2 for update",
    )
    .bind(req.tenant_id)
    .bind(req.membership_id)
    .fetch_optional(&mut *tx)
    .await?;
    match target {
        None => return Err(MembershipError::UnknownMembership),
        Some(true) => return Err(MembershipError::MembershipRevoked),
        Some(false) => {}
    }

    let admins = AdminState::load(&mut tx, req.tenant_id).await?;
    let leaves_none = check_last_admin(&admins, at.today(), req.confirm_no_admin, |_, m| {
        m == req.membership_id
    })?;

    let now = ts_param(at.now());
    sqlx::query(
        "update memberships set revoked_at = $3::timestamptz where tenant_id = $1 and id = $2",
    )
    .bind(req.tenant_id)
    .bind(req.membership_id)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "update role_assignments set revoked_at = $3::timestamptz
          where tenant_id = $1 and membership_id = $2
            and revoked_at is null and ends_on_exclusive > $4::date",
    )
    .bind(req.tenant_id)
    .bind(req.membership_id)
    .bind(&now)
    .bind(date_param(at.today()))
    .execute(&mut *tx)
    .await?;
    revoke_ended_admin_roles_with_open_window(&mut tx, req.tenant_id, req.membership_id, at)
        .await?;
    sqlx::query(
        "update handover_grants g set revoked_at = $3::timestamptz
           from role_assignments ra
          where g.tenant_id = $1 and ra.tenant_id = g.tenant_id
            and ra.id = g.source_assignment_id and ra.membership_id = $2
            and g.revoked_at is null",
    )
    .bind(req.tenant_id)
    .bind(req.membership_id)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    withdraw_pending_requests(
        &mut tx,
        req.tenant_id,
        req.actor_membership_id,
        req.membership_id,
        at,
    )
    .await?;
    write_audit(
        &mut tx,
        at,
        Audit::member(
            req.tenant_id,
            req.actor_membership_id,
            if leaving {
                "membership.left"
            } else {
                "membership.revoked"
            },
            "membership",
            req.membership_id,
            json!({ "left_no_admin": leaves_none }),
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// The second half of `revoke_membership`'s assignment cascade (final review I1).
///
/// An admin-class assignment that has already ended naturally still matters while its
/// handover window is open: the sweep gives it a grant if it has none yet. Left
/// unrevoked, such an assignment sits behind a revoked membership only until someone
/// invites the person back -- `ensure_membership` reopens the same row, and the next
/// sweep would then hand a removed admin handover authority.
///
/// So this revokes every unrevoked, already-ended admin-class assignment of the
/// membership whose handover window (`handover_period`, the one authoritative
/// computation) is not over today. The rule is decided in Rust per row rather than
/// approximated in SQL, because the window's end is truncated at month ends. Together
/// with the running-or-future update before it, the revoked set is exactly the
/// assignments that could still confer anything; older assignments are inert history
/// and keep their natural-end record. An existing grant from one of these assignments
/// is revoked by the grant cascade that follows, as before.
async fn revoke_ended_admin_roles_with_open_window(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
    at: Moment,
) -> Result<(), MembershipError> {
    let today = at.today();
    let ended: Vec<(Uuid, String)> = sqlx::query_as(
        "select ra.id, to_char(ra.ends_on_exclusive, 'YYYY-MM-DD')
           from role_assignments ra
           join roles r on r.tenant_id = ra.tenant_id and r.id = ra.role_id
          where ra.tenant_id = $1 and ra.membership_id = $2
            and ra.revoked_at is null and r.capability_class = 'admin'
            and ra.ends_on_exclusive <= $3::date",
    )
    .bind(tenant_id)
    .bind(membership_id)
    .bind(date_param(today))
    .fetch_all(&mut *conn)
    .await?;
    let mut open = Vec::new();
    for (id, ends) in ended {
        if handover_period(parse_date(&ends)?).is_some_and(|w| !w.has_ended_by(today)) {
            open.push(id);
        }
    }
    if open.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "update role_assignments set revoked_at = $3::timestamptz
          where tenant_id = $1 and id = any($2)",
    )
    .bind(tenant_id)
    .bind(&open)
    .bind(ts_param(at.now()))
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Withdraws the revoked member's own pending items (final review M9): replacement
/// proposals they made, and any access request still pending from their account's
/// address -- sent before they were invited in, say. A removed member's proposal must
/// not stay in front of the admins as if they still spoke for the role. `withdrawn`
/// carries no decider (`access_request_decider_matches_status`) and needs `closed_at`
/// (`access_request_closure_matches_status`). Each is audited, attributed to the actor
/// who revoked the membership. Nobody is emailed: the member knows they left or were
/// removed.
async fn withdraw_pending_requests(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    actor_membership_id: Uuid,
    membership_id: Uuid,
    at: Moment,
) -> Result<(), MembershipError> {
    let withdrawn: Vec<Uuid> = sqlx::query_scalar(
        "update access_requests ar set status = 'withdrawn', closed_at = $3::timestamptz
          where ar.tenant_id = $1 and ar.status = 'pending'
            and (ar.requester_membership_id = $2
                 or ar.requester_email = (select a.email
                                            from memberships m join accounts a on a.id = m.account_id
                                           where m.tenant_id = $1 and m.id = $2))
         returning ar.id",
    )
    .bind(tenant_id)
    .bind(membership_id)
    .bind(ts_param(at.now()))
    .fetch_all(&mut *conn)
    .await?;
    for request_id in withdrawn {
        write_audit(
            conn,
            at,
            Audit::member(
                tenant_id,
                actor_membership_id,
                "request.withdrawn",
                "access_request",
                request_id,
                json!({ "cause": "membership_revoked", "membership_id": membership_id }),
            ),
        )
        .await?;
    }
    Ok(())
}
