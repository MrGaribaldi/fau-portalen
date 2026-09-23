//! Shared pieces of every membership transaction: date and timestamp binding, the tenant
//! lock, authority checks, the admin state behind the last-admin safeguard, recipient
//! lists, and the audit and outbox writers.
//!
//! **Dates and timestamps cross the boundary as text.** sqlx 0.8 has no jiff support,
//! and adding `chrono` or `time` next to jiff would give the workspace two date types.
//! So a `Date` is bound as `'2027-10-01'` with a `::date` cast and read back with
//! `to_char(col, 'YYYY-MM-DD')`, and a `Timestamp` is bound as RFC 3339 with a
//! `::timestamptz` cast and read back as microseconds since the epoch. Rule-deciding
//! timestamps always come from the caller's `Moment`, never from SQL `now()`, so a test
//! can move time.

use fau_domain::membership::period::Period;
use fau_domain::membership::vocabulary::TenantStatus;
use fau_domain::time::Moment;
use jiff::civil::Date;
use jiff::Timestamp;
use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use super::error::MembershipError;

pub(crate) fn date_param(d: Date) -> String {
    d.to_string()
}

pub(crate) fn ts_param(t: Timestamp) -> String {
    t.to_string()
}

pub(crate) fn parse_date(s: &str) -> Result<Date, MembershipError> {
    s.parse().map_err(|_| MembershipError::decode())
}

pub(crate) fn from_micros(us: i64) -> Result<Timestamp, MembershipError> {
    Timestamp::from_microsecond(us).map_err(|_| MembershipError::decode())
}

pub(crate) struct TenantState {
    pub(crate) status: TenantStatus,
    pub(crate) frozen: bool,
}

/// Locks the tenant row for the rest of the transaction and returns its state. Every
/// membership mutation takes this lock first, which serialises role changes within one
/// FAU: two admins revoking each other at the same moment cannot both pass the
/// last-admin safeguard. `for no key update` does not block foreign-key checks from
/// other transactions inserting rows that reference the tenant.
pub(crate) async fn lock_tenant(
    conn: &mut PgConnection,
    tenant_id: Uuid,
) -> Result<TenantState, MembershipError> {
    let row: Option<(String, bool)> = sqlx::query_as(
        "select status, frozen_at is not null from tenants where id = $1 for no key update",
    )
    .bind(tenant_id)
    .fetch_optional(&mut *conn)
    .await?;
    let (status, frozen) = row.ok_or(MembershipError::UnknownTenant)?;
    let status = TenantStatus::from_code(&status).ok_or_else(MembershipError::decode)?;
    Ok(TenantState { status, frozen })
}

/// Who performed an audited action. Matches `audit_events.actor_kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActorKind {
    System,
    Registrant,
}

impl ActorKind {
    fn code(self) -> &'static str {
        match self {
            ActorKind::System => "system",
            ActorKind::Registrant => "registrant",
        }
    }
}

/// One audit entry. `params` holds ids, codes, dates and flags only (checked by
/// `audit_params_are_small` in size; by review in content).
pub(crate) struct Audit {
    pub(crate) tenant_id: Option<Uuid>,
    pub(crate) actor_kind: ActorKind,
    pub(crate) actor_account_id: Option<Uuid>,
    pub(crate) actor_membership_id: Option<Uuid>,
    pub(crate) action: &'static str,
    pub(crate) subject_type: &'static str,
    pub(crate) subject_id: Uuid,
    pub(crate) params: Value,
}

impl Audit {
    /// An entry written by a scheduled sweep rather than a person.
    pub(crate) fn system(
        tenant_id: Uuid,
        action: &'static str,
        subject_type: &'static str,
        subject_id: Uuid,
        params: Value,
    ) -> Self {
        Self {
            tenant_id: Some(tenant_id),
            actor_kind: ActorKind::System,
            actor_account_id: None,
            actor_membership_id: None,
            action,
            subject_type,
            subject_id,
            params,
        }
    }
}

pub(crate) async fn write_audit(
    conn: &mut PgConnection,
    at: Moment,
    e: Audit,
) -> Result<(), MembershipError> {
    sqlx::query(
        "insert into audit_events
           (id, tenant_id, actor_kind, actor_account_id, actor_membership_id,
            action, subject_type, subject_id, occurred_at, params)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9::timestamptz, $10::jsonb)",
    )
    .bind(Uuid::now_v7())
    .bind(e.tenant_id)
    .bind(e.actor_kind.code())
    .bind(e.actor_account_id)
    .bind(e.actor_membership_id)
    .bind(e.action)
    .bind(e.subject_type)
    .bind(e.subject_id)
    .bind(ts_param(at.now()))
    .bind(e.params.to_string())
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Queues one notification. `params` carries ids and codes for the renderer to look up;
/// never a token and never free text.
pub(crate) async fn enqueue(
    conn: &mut PgConnection,
    at: Moment,
    template: &'static str,
    recipient_email: &str,
    params: Value,
) -> Result<(), MembershipError> {
    sqlx::query(
        "insert into outbox (id, template, recipient_email, params, created_at)
         values ($1, $2, $3, $4::jsonb, $5::timestamptz)",
    )
    .bind(Uuid::now_v7())
    .bind(template)
    .bind(recipient_email)
    .bind(params.to_string())
    .bind(ts_param(at.now()))
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Finds or creates the account for a verified address, marking it verified if it was
/// not. Returns the account id and whether it is disabled.
pub(crate) async fn upsert_verified_account(
    conn: &mut PgConnection,
    email: &str,
    at: Moment,
) -> Result<(Uuid, bool), MembershipError> {
    Ok(sqlx::query_as(
        "insert into accounts (id, email, verified_at) values ($1, $2, $3::timestamptz)
         on conflict (email) do update
           set verified_at = coalesce(accounts.verified_at, excluded.verified_at)
         returning id, disabled_at is not null",
    )
    .bind(Uuid::now_v7())
    .bind(email)
    .bind(ts_param(at.now()))
    .fetch_one(&mut *conn)
    .await?)
}

/// Finds or creates the account's membership in the tenant. A revoked membership is
/// reopened rather than duplicated (one membership per account per FAU, #3412); the
/// revocation stays in the audit log. Returns the id and whether it already existed.
pub(crate) async fn ensure_membership(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    account_id: Uuid,
) -> Result<(Uuid, bool), MembershipError> {
    let existing: Option<Uuid> = sqlx::query_scalar(
        "select id from memberships where tenant_id = $1 and account_id = $2 for update",
    )
    .bind(tenant_id)
    .bind(account_id)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(id) = existing {
        sqlx::query("update memberships set revoked_at = null where tenant_id = $1 and id = $2")
            .bind(tenant_id)
            .bind(id)
            .execute(&mut *conn)
            .await?;
        return Ok((id, true));
    }
    let id = Uuid::now_v7();
    sqlx::query("insert into memberships (tenant_id, id, account_id) values ($1, $2, $3)")
        .bind(tenant_id)
        .bind(id)
        .bind(account_id)
        .execute(&mut *conn)
        .await?;
    Ok((id, false))
}

/// Inserts a role assignment and returns its id.
pub(crate) async fn insert_assignment(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
    role_id: Uuid,
    period: Period,
    granted_by: Option<Uuid>,
) -> Result<Uuid, MembershipError> {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into role_assignments
           (tenant_id, id, membership_id, role_id, starts_on, ends_on_exclusive, granted_by)
         values ($1, $2, $3, $4, $5::date, $6::date, $7)",
    )
    .bind(tenant_id)
    .bind(id)
    .bind(membership_id)
    .bind(role_id)
    .bind(date_param(period.starts_on()))
    .bind(date_param(period.ends_on_exclusive()))
    .bind(granted_by)
    .execute(&mut *conn)
    .await?;
    Ok(id)
}
