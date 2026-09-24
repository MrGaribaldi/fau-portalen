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

use fau_domain::membership::access::{
    removal_leaves_no_admin, tenant_has_admin, AssignmentView, GrantView,
};
use fau_domain::membership::period::Period;
use fau_domain::membership::rules::{recent_holder_window_start, EWB_OVERSIGHT_ADDRESS};
use fau_domain::membership::vocabulary::{CapabilityClass, TenantStatus};
use fau_domain::time::Moment;
use jiff::civil::Date;
use jiff::Timestamp;
use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use super::error::MembershipError;

/// The two conditions [`USABLE_ACCOUNT`] conjoins, named separately for callers that
/// need them apart -- `access::effective_access` reads "the membership is not revoked"
/// and "the account is verified and not disabled" as two distinct `Standing` fields
/// rather than one combined boolean (fix round 1, task 10). Kept equal to
/// `USABLE_ACCOUNT`'s own text by `usable_account_is_the_conjunction_of_its_two_parts`
/// below, since Rust has no stable way to build one `const &str` out of others.
pub(crate) const MEMBERSHIP_NOT_REVOKED: &str = "m.revoked_at is null";
pub(crate) const ACCOUNT_USABLE: &str = "a.disabled_at is null and a.verified_at is not null";

/// The condition every "usable membership" query shares: the membership itself is not
/// revoked, and the account behind it is verified and not disabled. Kept in one place
/// so every query that filters on it agrees -- the handover-grant queries once drifted
/// from the role-assignment ones by omitting the verified check. `pub(crate)` so the
/// sweep in `handover.rs` shares it too (fix round 1, Minor #4).
pub(crate) const USABLE_ACCOUNT: &str =
    "m.revoked_at is null and a.disabled_at is null and a.verified_at is not null";

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
/// membership mutation takes this lock first (the documented exceptions are listed in
/// the module doc of `membership`), which serialises role changes within one
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
    Member,
    System,
    Registrant,
    Requester,
    RecoveryEwb,
    RecoverySchoolRep,
}

impl ActorKind {
    /// Every variant, in the order `audit_events.actor_kind`'s check constraint lists
    /// them. `all_lists_every_variant` below keeps it complete.
    pub(crate) const ALL: &'static [ActorKind] = &[
        ActorKind::Member,
        ActorKind::System,
        ActorKind::Registrant,
        ActorKind::Requester,
        ActorKind::RecoveryEwb,
        ActorKind::RecoverySchoolRep,
    ];

    pub(crate) fn code(self) -> &'static str {
        match self {
            ActorKind::Member => "member",
            ActorKind::System => "system",
            ActorKind::Registrant => "registrant",
            ActorKind::Requester => "requester",
            ActorKind::RecoveryEwb => "recovery_ewb",
            ActorKind::RecoverySchoolRep => "recovery_school_rep",
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
    /// An entry by a member acting through their membership.
    pub(crate) fn member(
        tenant_id: Uuid,
        membership_id: Uuid,
        action: &'static str,
        subject_type: &'static str,
        subject_id: Uuid,
        params: Value,
    ) -> Self {
        Self {
            tenant_id: Some(tenant_id),
            actor_kind: ActorKind::Member,
            actor_account_id: None,
            actor_membership_id: Some(membership_id),
            action,
            subject_type,
            subject_id,
            params,
        }
    }

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
/// never a token and never free text. `tenant_id` is the FAU the message is about, and
/// `None` only for a global message (`outbox_tenant_scoped_unless_global` names which
/// templates may be global), so FAU deletion and erasure can find queued mail by column.
pub(crate) async fn enqueue(
    conn: &mut PgConnection,
    at: Moment,
    tenant_id: Option<Uuid>,
    template: &'static str,
    recipient_email: &str,
    params: Value,
) -> Result<(), MembershipError> {
    sqlx::query(
        "insert into outbox (id, tenant_id, template, recipient_email, params, created_at)
         values ($1, $2, $3, $4, $5::jsonb, $6::timestamptz)",
    )
    .bind(Uuid::now_v7())
    .bind(tenant_id)
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

pub(crate) fn period_from(starts: &str, ends: &str) -> Result<Period, MembershipError> {
    Period::new(parse_date(starts)?, parse_date(ends)?).map_err(|_| MembershipError::decode())
}

/// Active and not frozen: the state in which invitations and requests may be created.
pub(crate) fn require_open(state: &TenantState) -> Result<(), MembershipError> {
    if state.status != TenantStatus::Active {
        return Err(MembershipError::TenantNotActive);
    }
    if state.frozen {
        return Err(MembershipError::TenantFrozen);
    }
    Ok(())
}

/// Whether `membership_id` holds an admin-class role valid today, through a membership
/// that is not revoked and an account that is verified and not disabled.
pub(crate) async fn is_admin_today(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
    today: Date,
) -> Result<bool, MembershipError> {
    let sql = format!(
        "select exists (
           select 1
             from memberships m
             join accounts a          on a.id = m.account_id
             join role_assignments ra on ra.tenant_id = m.tenant_id and ra.membership_id = m.id
             join roles r             on r.tenant_id = ra.tenant_id and r.id = ra.role_id
            where m.tenant_id = $1 and m.id = $2
              and {USABLE_ACCOUNT}
              and ra.revoked_at is null and r.capability_class = 'admin'
              and ra.starts_on <= $3::date and ra.ends_on_exclusive > $3::date)"
    );
    Ok(sqlx::query_scalar(&sql)
        .bind(tenant_id)
        .bind(membership_id)
        .bind(date_param(today))
        .fetch_one(&mut *conn)
        .await?)
}

pub(crate) async fn require_admin(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
    today: Date,
) -> Result<(), MembershipError> {
    if is_admin_today(conn, tenant_id, membership_id, today).await? {
        Ok(())
    } else {
        Err(MembershipError::NotAuthorized)
    }
}

/// The membership's revocation state and whether its account is usable (verified and
/// not disabled), read under `for update`. `None` when no such membership exists.
/// `grant_role`'s target check: a revoked membership and an unusable account are
/// reported as distinct, typed refusals rather than collapsed into one.
pub(crate) async fn membership_and_account_state(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
) -> Result<Option<(bool, bool)>, MembershipError> {
    let sql = format!(
        "select m.revoked_at is not null, ({USABLE_ACCOUNT})
           from memberships m join accounts a on a.id = m.account_id
          where m.tenant_id = $1 and m.id = $2 for update"
    );
    Ok(sqlx::query_as(&sql)
        .bind(tenant_id)
        .bind(membership_id)
        .fetch_optional(&mut *conn)
        .await?)
}

/// Whether the membership is not revoked and its account is verified and not disabled.
pub(crate) async fn membership_usable(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
) -> Result<bool, MembershipError> {
    let sql = format!(
        "select exists (
           select 1 from memberships m join accounts a on a.id = m.account_id
            where m.tenant_id = $1 and m.id = $2
              and {USABLE_ACCOUNT})"
    );
    Ok(sqlx::query_scalar(&sql)
        .bind(tenant_id)
        .bind(membership_id)
        .fetch_one(&mut *conn)
        .await?)
}

/// The account address behind a usable membership, or `None`.
pub(crate) async fn usable_member_email(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
) -> Result<Option<String>, MembershipError> {
    let sql = format!(
        "select a.email from memberships m join accounts a on a.id = m.account_id
          where m.tenant_id = $1 and m.id = $2
            and {USABLE_ACCOUNT}"
    );
    Ok(sqlx::query_scalar(&sql)
        .bind(tenant_id)
        .bind(membership_id)
        .fetch_optional(&mut *conn)
        .await?)
}

/// Whether `grant_id` is valid today and still belongs to `membership_id`, through a
/// membership that is not revoked and an account that is not disabled (spec 6.3).
///
/// **Defence in depth (fix round 1, Critical #1):** also requires the grant's own
/// source assignment to be unrevoked. The sweep that creates grants now locks the
/// tenant first, so a grant should never outlive a revocation of its source, but this
/// read does not trust that invariant either.
pub(crate) async fn handover_grant_valid(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    grant_id: Uuid,
    membership_id: Uuid,
    today: Date,
) -> Result<bool, MembershipError> {
    let sql = format!(
        "select exists (
           select 1
             from handover_grants g
             join role_assignments ra on ra.tenant_id = g.tenant_id and ra.id = g.source_assignment_id
             join memberships m       on m.tenant_id = ra.tenant_id and m.id = ra.membership_id
             join accounts a          on a.id = m.account_id
            where g.tenant_id = $1 and g.id = $2 and ra.membership_id = $3
              and g.revoked_at is null and ra.revoked_at is null
              and g.starts_on <= $4::date and g.ends_on_exclusive > $4::date
              and {USABLE_ACCOUNT})"
    );
    Ok(sqlx::query_scalar(&sql)
        .bind(tenant_id)
        .bind(grant_id)
        .bind(membership_id)
        .bind(date_param(today))
        .fetch_one(&mut *conn)
        .await?)
}

pub(crate) struct AdminAssignmentRow {
    pub(crate) id: Uuid,
    pub(crate) membership_id: Uuid,
    pub(crate) view: AssignmentView,
}

pub(crate) struct GrantRow {
    pub(crate) source_assignment_id: Uuid,
    pub(crate) membership_id: Uuid,
    pub(crate) view: GrantView,
}

/// Every admin-class assignment and handover grant held through a usable membership:
/// the inputs to the no-admin predicate (spec 6.4), with ids so the last-admin safeguard
/// can ask "and without this one?".
pub(crate) struct AdminState {
    pub(crate) assignments: Vec<AdminAssignmentRow>,
    pub(crate) grants: Vec<GrantRow>,
}

impl AdminState {
    pub(crate) async fn load(
        conn: &mut PgConnection,
        tenant_id: Uuid,
    ) -> Result<Self, MembershipError> {
        let assignments_sql = format!(
            "select ra.id, ra.membership_id,
                    to_char(ra.starts_on, 'YYYY-MM-DD'), to_char(ra.ends_on_exclusive, 'YYYY-MM-DD'),
                    ra.revoked_at is not null
               from role_assignments ra
               join roles r       on r.tenant_id = ra.tenant_id and r.id = ra.role_id
               join memberships m on m.tenant_id = ra.tenant_id and m.id = ra.membership_id
               join accounts a    on a.id = m.account_id
              where ra.tenant_id = $1 and r.capability_class = 'admin'
                and {USABLE_ACCOUNT}"
        );
        let rows: Vec<(Uuid, Uuid, String, String, bool)> = sqlx::query_as(&assignments_sql)
            .bind(tenant_id)
            .fetch_all(&mut *conn)
            .await?;
        let mut assignments = Vec::with_capacity(rows.len());
        for (id, membership_id, starts, ends, revoked) in rows {
            assignments.push(AdminAssignmentRow {
                id,
                membership_id,
                view: AssignmentView {
                    capability: CapabilityClass::Admin,
                    period: period_from(&starts, &ends)?,
                    revoked,
                },
            });
        }

        // Defence in depth (fix round 1, Critical #1): `ra.revoked_at is null` as well,
        // so a grant can never confer admin coverage once its source assignment is
        // revoked, whether or not the grant row itself was also revoked.
        let grants_sql = format!(
            "select g.source_assignment_id, ra.membership_id,
                    to_char(g.starts_on, 'YYYY-MM-DD'), to_char(g.ends_on_exclusive, 'YYYY-MM-DD'),
                    g.revoked_at is not null
               from handover_grants g
               join role_assignments ra on ra.tenant_id = g.tenant_id and ra.id = g.source_assignment_id
               join memberships m       on m.tenant_id = ra.tenant_id and m.id = ra.membership_id
               join accounts a          on a.id = m.account_id
              where g.tenant_id = $1 and ra.revoked_at is null and {USABLE_ACCOUNT}"
        );
        let rows: Vec<(Uuid, Uuid, String, String, bool)> = sqlx::query_as(&grants_sql)
            .bind(tenant_id)
            .fetch_all(&mut *conn)
            .await?;
        let mut grants = Vec::with_capacity(rows.len());
        for (source_assignment_id, membership_id, starts, ends, revoked) in rows {
            grants.push(GrantRow {
                source_assignment_id,
                membership_id,
                view: GrantView {
                    period: period_from(&starts, &ends)?,
                    revoked,
                },
            });
        }
        Ok(Self {
            assignments,
            grants,
        })
    }

    pub(crate) fn has_admin(&self, today: Date) -> bool {
        let assignments: Vec<AssignmentView> = self.assignments.iter().map(|r| r.view).collect();
        let grants: Vec<GrantView> = self.grants.iter().map(|r| r.view).collect();
        tenant_has_admin(&assignments, &grants, today)
    }
}

/// The last-admin safeguard (spec 7). `removed(assignment_id, membership_id)` names the
/// assignments an action ends; a grant goes with its source assignment. Refuses, unless
/// confirmed, an action that `removal_leaves_no_admin` says would leave the FAU without
/// an admin; returns whether it does, for the audit entry.
pub(crate) fn check_last_admin(
    state: &AdminState,
    today: Date,
    confirm_no_admin: bool,
    removed: impl Fn(Uuid, Uuid) -> bool,
) -> Result<bool, MembershipError> {
    let (mut kept_a, mut gone_a) = (Vec::new(), Vec::new());
    for r in &state.assignments {
        if removed(r.id, r.membership_id) {
            gone_a.push(r.view);
        } else {
            kept_a.push(r.view);
        }
    }
    let (mut kept_g, mut gone_g) = (Vec::new(), Vec::new());
    for r in &state.grants {
        if removed(r.source_assignment_id, r.membership_id) {
            gone_g.push(r.view);
        } else {
            kept_g.push(r.view);
        }
    }
    let leaves_none = removal_leaves_no_admin(&kept_a, &kept_g, &gone_a, &gone_g, today);
    if leaves_none && !confirm_no_admin {
        return Err(MembershipError::WouldLeaveNoAdmin);
    }
    Ok(leaves_none)
}

/// Addresses of every current member: a usable membership with any role valid today.
pub(crate) async fn current_member_emails(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    today: Date,
) -> Result<Vec<String>, MembershipError> {
    member_emails(conn, tenant_id, today, false).await
}

async fn member_emails(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    today: Date,
    admins_only: bool,
) -> Result<Vec<String>, MembershipError> {
    let sql = format!(
        "select distinct a.email
           from memberships m
           join accounts a          on a.id = m.account_id
           join role_assignments ra on ra.tenant_id = m.tenant_id and ra.membership_id = m.id
           join roles r             on r.tenant_id = ra.tenant_id and r.id = ra.role_id
          where m.tenant_id = $1
            and {USABLE_ACCOUNT}
            and ra.revoked_at is null
            and ra.starts_on <= $2::date and ra.ends_on_exclusive > $2::date
            and ($3 = false or r.capability_class = 'admin')
          order by a.email"
    );
    Ok(sqlx::query_scalar(&sql)
        .bind(tenant_id)
        .bind(date_param(today))
        .bind(admins_only)
        .fetch_all(&mut *conn)
        .await?)
}

/// Recovery notices (spec 6.4.4, ADR-003 decision 10): every current member; when none
/// remain, everyone who held a role in the past 24 months. EWB is always added, as the
/// notified second party for every recovery on every FAU (ADR-003 decision 8).
pub(crate) async fn recovery_notice_recipients(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    today: Date,
) -> Result<Vec<String>, MembershipError> {
    let mut recipients = current_member_emails(conn, tenant_id, today).await?;
    if recipients.is_empty() {
        recipients = sqlx::query_scalar(
            "select distinct a.email
               from role_assignments ra
               join memberships m on m.tenant_id = ra.tenant_id and m.id = ra.membership_id
               join accounts a    on a.id = m.account_id
              where ra.tenant_id = $1 and a.disabled_at is null
                and ra.starts_on <= $2::date and ra.ends_on_exclusive > $3::date
              order by a.email",
        )
        .bind(tenant_id)
        .bind(date_param(today))
        .bind(date_param(recent_holder_window_start(today)))
        .fetch_all(&mut *conn)
        .await?;
    }
    if !recipients.iter().any(|e| e == EWB_OVERSIGHT_ADDRESS) {
        recipients.push(EWB_OVERSIGHT_ADDRESS.to_owned());
    }
    Ok(recipients)
}

/// Addresses of everyone holding an admin-class role valid today.
pub(crate) async fn admin_emails(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    today: Date,
) -> Result<Vec<String>, MembershipError> {
    member_emails(conn, tenant_id, today, true).await
}

/// The codes `audit_events.actor_kind` accepts, in `ActorKind::ALL` order. Exposed only
/// so the schema-agreement test (`membership_schema.rs`) can compare them with the live
/// check constraint; `ActorKind` itself stays internal.
pub fn audit_actor_kind_codes() -> Vec<&'static str> {
    ActorKind::ALL.iter().map(|k| k.code()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guards against `USABLE_ACCOUNT` drifting apart from its two named conjuncts
    /// (fix round 1, task 10): a caller like `access::effective_access` that needs
    /// them apart reads `MEMBERSHIP_NOT_REVOKED` and `ACCOUNT_USABLE` rather than
    /// retyping the SQL, so this test is what keeps all three in sync.
    /// `ActorKind::ALL` names every variant: adding one without listing it fails to
    /// compile here (the exhaustive match) or fails the count.
    #[test]
    fn all_lists_every_variant() {
        let count = |k: ActorKind| match k {
            ActorKind::Member
            | ActorKind::System
            | ActorKind::Registrant
            | ActorKind::Requester
            | ActorKind::RecoveryEwb
            | ActorKind::RecoverySchoolRep => 1,
        };
        assert_eq!(ActorKind::ALL.iter().map(|k| count(*k)).sum::<usize>(), 6);
        let mut codes: Vec<_> = ActorKind::ALL.iter().map(|k| k.code()).collect();
        codes.dedup();
        assert_eq!(codes.len(), 6);
    }

    #[test]
    fn usable_account_is_the_conjunction_of_its_two_parts() {
        assert_eq!(
            format!("{MEMBERSHIP_NOT_REVOKED} and {ACCOUNT_USABLE}"),
            USABLE_ACCOUNT
        );
    }
}
