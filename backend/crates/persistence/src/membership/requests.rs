//! Access requests and replacement proposals (spec 5.2–5.4): one table, one approval
//! path, the same statuses. The requester never learns who the admins are.

use std::fmt;

use fau_domain::email::{Email, VerifiedEmail};
use fau_domain::membership::period::Period;
use fau_domain::membership::requests::{
    check_replacement_dates, check_request_limits, lapse_cutoff, normalise_message,
};
use fau_domain::membership::vocabulary::{InvitationMode, RequestKind};
use fau_domain::time::Moment;
use jiff::civil::Date;
use serde_json::json;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::error::MembershipError;
use super::invitations::{
    insert_invitation, resolve_roles, IssuedInvitation, NewInvitation, OfferedRole,
};
use super::sql::{
    admin_emails, date_param, enqueue, lock_tenant, parse_date, require_admin, require_open,
    ts_param, usable_member_email, write_audit, ActorKind, Audit,
};

#[derive(Clone)]
pub struct CreateAccessRequest {
    pub tenant_id: Uuid,
    /// Confirmed with a Hanko passcode first (spec 3.2), so nobody can make the portal
    /// email an FAU's admins from an address they do not control.
    pub requester: VerifiedEmail,
    pub message: Option<String>,
}

/// Hand-written so the message never reaches a log line the way a derived `Debug` would
/// (`requester`'s own `Debug` already redacts the address).
impl fmt::Debug for CreateAccessRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreateAccessRequest")
            .field("tenant_id", &self.tenant_id)
            .field("requester", &self.requester)
            .field("message", &self.message.as_ref().map(|_| "[redacted]"))
            .finish()
    }
}

/// Creates an access request (spec 5.2) and emails every current admin.
pub async fn create_access_request(
    pool: &PgPool,
    req: CreateAccessRequest,
    at: Moment,
) -> Result<Uuid, MembershipError> {
    let message = normalise_message(req.message.as_deref())?;
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, req.tenant_id).await?;
    require_open(&state)?;
    let requester = req.requester.email().as_str();
    check_limits(&mut tx, req.tenant_id, requester, at.today()).await?;

    let request_id = insert_request(
        &mut tx,
        at,
        NewRequest {
            tenant_id: req.tenant_id,
            kind: RequestKind::Access,
            requester_email: requester,
            invitee_email: requester,
            requester_membership_id: None,
            replaced_assignment_id: None,
            proposed: None,
            message: message.as_deref(),
        },
    )
    .await?;
    notify_admins(&mut tx, at, req.tenant_id, request_id, RequestKind::Access).await?;
    write_audit(
        &mut tx,
        at,
        Audit {
            tenant_id: Some(req.tenant_id),
            actor_kind: ActorKind::Requester,
            actor_account_id: None,
            actor_membership_id: None,
            action: "request.created",
            subject_type: "access_request",
            subject_id: request_id,
            params: json!({ "kind": RequestKind::Access.code() }),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(request_id)
}

#[derive(Clone)]
pub struct CreateReplacementProposal {
    pub tenant_id: Uuid,
    pub proposer_membership_id: Uuid,
    /// One of the proposer's own role assignments, valid today (spec 5.3).
    pub replaced_assignment_id: Uuid,
    pub successor: Email,
    pub starts_on: Date,
    pub ends_on_exclusive: Date,
    pub message: Option<String>,
}

/// Hand-written so the message never reaches a log line the way a derived `Debug` would
/// (`successor`'s own `Debug` already redacts the address).
impl fmt::Debug for CreateReplacementProposal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreateReplacementProposal")
            .field("tenant_id", &self.tenant_id)
            .field("proposer_membership_id", &self.proposer_membership_id)
            .field("replaced_assignment_id", &self.replaced_assignment_id)
            .field("successor", &self.successor)
            .field("starts_on", &self.starts_on)
            .field("ends_on_exclusive", &self.ends_on_exclusive)
            .field("message", &self.message.as_ref().map(|_| "[redacted]"))
            .finish()
    }
}

/// Creates a replacement proposal (spec 5.3). The role's name and class are the
/// proposer's own role's and cannot be changed here; an admin may change them on
/// approval. Proposing does not end the proposer's role. Counts against the proposer's
/// address for the one-open-request limit.
pub async fn create_replacement_proposal(
    pool: &PgPool,
    req: CreateReplacementProposal,
    at: Moment,
) -> Result<Uuid, MembershipError> {
    let message = normalise_message(req.message.as_deref())?;
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, req.tenant_id).await?;
    require_open(&state)?;
    let proposer_email = usable_member_email(&mut tx, req.tenant_id, req.proposer_membership_id)
        .await?
        .ok_or(MembershipError::NotAuthorized)?;

    // Scoped to the proposer's own membership (final review M2): an unknown id and
    // someone else's assignment both come back as `NotAuthorized`, so a proposal never
    // reveals whether an assignment id exists.
    let assignment: Option<(String, String, bool)> = sqlx::query_as(
        "select to_char(starts_on, 'YYYY-MM-DD'), to_char(ends_on_exclusive, 'YYYY-MM-DD'),
                revoked_at is not null
           from role_assignments where tenant_id = $1 and id = $2 and membership_id = $3",
    )
    .bind(req.tenant_id)
    .bind(req.replaced_assignment_id)
    .bind(req.proposer_membership_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (starts, ends, revoked) = assignment.ok_or(MembershipError::NotAuthorized)?;
    let held = Period::new(parse_date(&starts)?, parse_date(&ends)?)
        .map_err(|_| MembershipError::decode())?;
    if revoked || !held.contains(at.today()) {
        return Err(MembershipError::RoleNotHeldToday);
    }
    let proposed = check_replacement_dates(at.today(), req.starts_on, req.ends_on_exclusive)
        .map_err(MembershipError::ReplacementDates)?;
    check_limits(&mut tx, req.tenant_id, &proposer_email, at.today()).await?;

    let request_id = insert_request(
        &mut tx,
        at,
        NewRequest {
            tenant_id: req.tenant_id,
            kind: RequestKind::Replacement,
            requester_email: &proposer_email,
            invitee_email: req.successor.as_str(),
            requester_membership_id: Some(req.proposer_membership_id),
            replaced_assignment_id: Some(req.replaced_assignment_id),
            proposed: Some(proposed),
            message: message.as_deref(),
        },
    )
    .await?;
    notify_admins(
        &mut tx,
        at,
        req.tenant_id,
        request_id,
        RequestKind::Replacement,
    )
    .await?;
    write_audit(
        &mut tx,
        at,
        Audit::member(
            req.tenant_id,
            req.proposer_membership_id,
            "request.created",
            "access_request",
            request_id,
            json!({
                "kind": RequestKind::Replacement.code(),
                "replaced_assignment_id": req.replaced_assignment_id,
            }),
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(request_id)
}

async fn check_limits(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    requester_email: &str,
    today: Date,
) -> Result<(), MembershipError> {
    let open: i64 = sqlx::query_scalar(
        "select count(*) from access_requests
          where tenant_id = $1 and requester_email = $2 and status = 'pending'",
    )
    .bind(tenant_id)
    .bind(requester_email)
    .fetch_one(&mut *conn)
    .await?;
    let created_today: i64 = sqlx::query_scalar(
        "select count(*) from access_requests where tenant_id = $1 and created_on = $2::date",
    )
    .bind(tenant_id)
    .bind(date_param(today))
    .fetch_one(&mut *conn)
    .await?;
    check_request_limits(open, created_today).map_err(MembershipError::RequestLimit)
}

struct NewRequest<'a> {
    tenant_id: Uuid,
    kind: RequestKind,
    requester_email: &'a str,
    invitee_email: &'a str,
    requester_membership_id: Option<Uuid>,
    replaced_assignment_id: Option<Uuid>,
    proposed: Option<Period>,
    message: Option<&'a str>,
}

async fn insert_request(
    conn: &mut PgConnection,
    at: Moment,
    r: NewRequest<'_>,
) -> Result<Uuid, MembershipError> {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into access_requests
           (tenant_id, id, kind, requester_email, invitee_email, requester_membership_id,
            replaced_assignment_id, proposed_starts_on, proposed_ends_on_exclusive, message,
            created_on, created_at)
         values ($1, $2, $3, $4, $5, $6, $7, $8::date, $9::date, $10, $11::date, $12::timestamptz)",
    )
    .bind(r.tenant_id)
    .bind(id)
    .bind(r.kind.code())
    .bind(r.requester_email)
    .bind(r.invitee_email)
    .bind(r.requester_membership_id)
    .bind(r.replaced_assignment_id)
    .bind(r.proposed.map(|p| date_param(p.starts_on())))
    .bind(r.proposed.map(|p| date_param(p.ends_on_exclusive())))
    .bind(r.message)
    .bind(date_param(at.today()))
    .bind(ts_param(at.now()))
    .execute(&mut *conn)
    .await?;
    Ok(id)
}

/// One `request.received` message per current admin. An FAU with no admin gets none;
/// the request waits, and lapses if nobody arrives to handle it.
async fn notify_admins(
    conn: &mut PgConnection,
    at: Moment,
    tenant_id: Uuid,
    request_id: Uuid,
    kind: RequestKind,
) -> Result<(), MembershipError> {
    for admin in admin_emails(conn, tenant_id, at.today()).await? {
        enqueue(
            conn,
            at,
            Some(tenant_id),
            "request.received",
            &admin,
            json!({ "tenant_id": tenant_id, "request_id": request_id, "kind": kind.code() }),
        )
        .await?;
    }
    Ok(())
}

/// A decision on a pending request.
#[derive(Debug, Clone)]
pub struct RequestDecision {
    pub tenant_id: Uuid,
    pub actor_membership_id: Uuid,
    pub request_id: Uuid,
}

struct RequestRow {
    requester_email: String,
    invitee_email: String,
}

async fn lock_pending_request(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    request_id: Uuid,
) -> Result<RequestRow, MembershipError> {
    let row: Option<(String, String, String)> = sqlx::query_as(
        "select status, requester_email, invitee_email from access_requests
          where tenant_id = $1 and id = $2 for update",
    )
    .bind(tenant_id)
    .bind(request_id)
    .fetch_optional(&mut *conn)
    .await?;
    let (status, requester_email, invitee_email) = row.ok_or(MembershipError::UnknownRequest)?;
    if status != "pending" {
        return Err(MembershipError::RequestNotPending);
    }
    Ok(RequestRow {
        requester_email,
        invitee_email,
    })
}

/// Approves a pending request by issuing a normal invitation to its invitee with the
/// roles the admin chose (spec 5.2.2, 5.3), linked to the request.
///
/// **Gate:** as `issue_invitation`. Only an admin valid today may approve; a handover
/// grant does not suffice (spec 6.3). Authority is checked before the request's state,
/// so an unauthorised actor gets `NotAuthorized` whatever that state is (or whether the
/// request exists at all) -- the same principle as `resend_invitation` and
/// `withdraw_invitation`.
pub async fn approve_request(
    pool: &PgPool,
    decision: RequestDecision,
    roles: Vec<OfferedRole>,
    at: Moment,
) -> Result<IssuedInvitation, MembershipError> {
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, decision.tenant_id).await?;
    require_open(&state)?;
    require_admin(
        &mut tx,
        decision.tenant_id,
        decision.actor_membership_id,
        at.today(),
    )
    .await?;
    let request = lock_pending_request(&mut tx, decision.tenant_id, decision.request_id).await?;
    let invitee = Email::parse(&request.invitee_email).map_err(|_| MembershipError::decode())?;

    let roles = resolve_roles(
        &mut tx,
        at,
        decision.tenant_id,
        Some(decision.actor_membership_id),
        &roles,
    )
    .await?;
    let issued = insert_invitation(
        &mut tx,
        at,
        NewInvitation {
            tenant_id: decision.tenant_id,
            mode: InvitationMode::Normal,
            recipient: invitee,
            issued_by: Some(decision.actor_membership_id),
            handover_grant_id: None,
            access_request_id: Some(decision.request_id),
            recovery_holder: None,
            roles: roles.into_iter().map(|(id, _, p)| (id, p)).collect(),
            actor_kind: ActorKind::Member,
            actor_membership_id: Some(decision.actor_membership_id),
        },
    )
    .await?;
    close_request(&mut tx, at, &decision, "approved").await?;
    tx.commit().await?;
    Ok(issued)
}

/// Declines a pending request. The requester is told only that it was not approved
/// (spec 5.2.3).
///
/// **Gate:** as `issue_invitation`. Authority is checked before the request's state, as
/// in [`approve_request`].
pub async fn decline_request(
    pool: &PgPool,
    decision: RequestDecision,
    at: Moment,
) -> Result<(), MembershipError> {
    let mut tx = pool.begin().await?;
    // No `require_open` here: closing a pending item grants nothing, so this is allowed
    // on a frozen FAU too -- the same reasoning as `withdraw_invitation` (controller
    // ruling, fix round 1).
    lock_tenant(&mut tx, decision.tenant_id).await?;
    require_admin(
        &mut tx,
        decision.tenant_id,
        decision.actor_membership_id,
        at.today(),
    )
    .await?;
    let request = lock_pending_request(&mut tx, decision.tenant_id, decision.request_id).await?;
    close_request(&mut tx, at, &decision, "declined").await?;
    enqueue(
        &mut tx,
        at,
        Some(decision.tenant_id),
        "request.declined",
        &request.requester_email,
        json!({ "tenant_id": decision.tenant_id, "request_id": decision.request_id }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn close_request(
    conn: &mut PgConnection,
    at: Moment,
    decision: &RequestDecision,
    status: &'static str,
) -> Result<(), MembershipError> {
    sqlx::query(
        "update access_requests set status = $3, decided_by = $4, closed_at = $5::timestamptz
          where tenant_id = $1 and id = $2",
    )
    .bind(decision.tenant_id)
    .bind(decision.request_id)
    .bind(status)
    .bind(decision.actor_membership_id)
    .bind(ts_param(at.now()))
    .execute(&mut *conn)
    .await?;
    let action = if status == "approved" {
        "request.approved"
    } else {
        "request.declined"
    };
    write_audit(
        conn,
        at,
        Audit::member(
            decision.tenant_id,
            decision.actor_membership_id,
            action,
            "access_request",
            decision.request_id,
            json!({}),
        ),
    )
    .await
}

/// The scheduled sweep: every request pending for 30 days lapses, and its requester is
/// told the same as for a decline (spec 5.2.4). Returns how many.
///
/// **No `lock_tenant`** (final review M5), deliberately: lapsing closes pending items
/// across every tenant in one statement and grants nothing, so it needs no tenant-wide
/// serialisation -- no admin state, role or last-admin check depends on it. The
/// `update` takes each request row's own row lock, so it never waits on a tenant lock
/// either. A concurrent `approve_request` or `decline_request` that already holds the
/// row (`lock_pending_request`'s `for update`) makes it wait; at READ COMMITTED the
/// update then re-checks `status = 'pending'` against the committed row and skips a
/// request that was decided meanwhile. The reverse order is covered the same way:
/// a decision arriving after the lapse finds the row no longer pending.
pub async fn lapse_requests(pool: &PgPool, at: Moment) -> Result<u64, MembershipError> {
    let mut tx = pool.begin().await?;
    let lapsed: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
        "update access_requests set status = 'lapsed', closed_at = $1::timestamptz
          where status = 'pending' and created_on <= $2::date
         returning tenant_id, id, requester_email",
    )
    .bind(ts_param(at.now()))
    .bind(date_param(lapse_cutoff(at.today())))
    .fetch_all(&mut *tx)
    .await?;
    for (tenant_id, request_id, requester_email) in &lapsed {
        enqueue(
            &mut tx,
            at,
            Some(*tenant_id),
            "request.lapsed",
            requester_email,
            json!({ "tenant_id": tenant_id, "request_id": request_id }),
        )
        .await?;
        write_audit(
            &mut tx,
            at,
            Audit::system(
                *tenant_id,
                "request.lapsed",
                "access_request",
                *request_id,
                json!({}),
            ),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(lapsed.len() as u64)
}
