//! Signup, activation and pending expiry (spec 3.2–3.4).

use fau_domain::email::{Email, VerifiedEmail};
use fau_domain::membership::period::Period;
use fau_domain::membership::rules::{
    pending_signup_expiry, validate_admin_end, BOOTSTRAP_ADMIN_ROLE_NAME, EWB_OVERSIGHT_ADDRESS,
    MAX_PENDING_SIGNUPS_PER_ADDRESS,
};
use fau_domain::membership::vocabulary::{
    CapabilityClass, FauName, InvitationMode, RecoveryHolder, TenantStatus,
};
use fau_domain::time::Moment;
use jiff::civil::Date;
use jiff::Timestamp;
use serde_json::json;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::error::{ExistingFau, MembershipError};
use super::invitations::{insert_invitation, IssuedInvitation, NewInvitation};
use super::sql::{
    date_param, enqueue, ensure_membership, from_micros, insert_assignment, lock_tenant,
    parse_date, ts_param, upsert_verified_account, write_audit, ActorKind, Audit,
};

/// The signup form (spec 3.1), already parsed by the caller. `school_id` is a bare uuid
/// until #3441's register gives it a foreign key.
#[derive(Debug, Clone)]
pub struct PendingSignup {
    pub school_id: Uuid,
    pub fau_name: FauName,
    pub registrant_email: Email,
    pub leader_email: Email,
    /// "Til hvilken dato er du valgt?", as an exclusive end.
    pub admin_ends_on_exclusive: Date,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingTenant {
    pub tenant_id: Uuid,
    pub expires_at: Timestamp,
}

/// Creates an FAU in `pending` (spec 3.3).
///
/// A school that already has a pending or active FAU is refused with
/// [`MembershipError::SchoolTaken`], which says which of the two and nothing else; the
/// refusal still commits a copy to EWB's outbox and a global audit entry (spec 3.2).
/// One address may hold at most three pending FAU-er.
pub async fn create_pending_tenant(
    pool: &PgPool,
    signup: PendingSignup,
    at: Moment,
) -> Result<PendingTenant, MembershipError> {
    validate_admin_end(at.today(), signup.admin_ends_on_exclusive)
        .map_err(MembershipError::AdminEnd)?;

    let mut tx = pool.begin().await?;
    // Serialise signups from one address, so the three-pending limit holds when two
    // forms are submitted at once. Released at commit or rollback.
    sqlx::query("select pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(signup.registrant_email.as_str())
        .execute(&mut *tx)
        .await?;
    // An expired pending FAU the sweep has not reached yet must not hold the school.
    expire_due(&mut tx, at, Some(signup.school_id)).await?;

    if let Some((existing_tenant_id, existing)) =
        live_tenant_for_school(&mut tx, signup.school_id).await?
    {
        return record_collision(tx, at, signup.school_id, existing_tenant_id, existing).await;
    }

    // Only signups the sweep has not yet reached count against the limit: an expired
    // row that a concurrent caller hasn't swept away yet must not block a new signup
    // from the same address (it holds no data rights and is about to be deleted).
    let pending: i64 = sqlx::query_scalar(
        "select count(*) from tenant_signups s join tenants t on t.id = s.tenant_id
          where t.status = 'pending' and s.registrant_email = $1
            and s.expires_at > $2::timestamptz",
    )
    .bind(signup.registrant_email.as_str())
    .bind(ts_param(at.now()))
    .fetch_one(&mut *tx)
    .await?;
    if pending >= MAX_PENDING_SIGNUPS_PER_ADDRESS {
        return Err(MembershipError::TooManyPendingSignups);
    }

    // At most two attempts: the initial insert, and one retry. A lost race is resolved
    // by re-reading the school's live tenant (below); the loop only continues to a
    // second attempt when that re-read finds nothing, meaning the transaction we
    // conflicted with rolled back between the insert and the re-read, so the school
    // may be free again -- a transient, resolvable race, not a decode failure.
    let mut tenant_id = Uuid::now_v7();
    let mut placed = false;
    for attempt in 1..=2 {
        if attempt > 1 {
            tenant_id = Uuid::now_v7();
        }
        let inserted = try_insert_tenant(&mut tx, tenant_id, &signup).await?;
        if inserted.is_some() {
            placed = true;
            break;
        }
        // Lost a race with a concurrent signup for the same school since the check above.
        if let Some((existing_tenant_id, existing)) =
            live_tenant_for_school(&mut tx, signup.school_id).await?
        {
            return record_collision(tx, at, signup.school_id, existing_tenant_id, existing).await;
        }
        if attempt == 2 {
            return Err(MembershipError::SignupRaceUnresolved);
        }
    }
    debug_assert!(placed);

    let expires_at = pending_signup_expiry(at.now());
    sqlx::query(
        "insert into tenant_signups
           (tenant_id, registrant_email, leader_email, admin_ends_on_exclusive, expires_at)
         values ($1, $2, $3, $4::date, $5::timestamptz)",
    )
    .bind(tenant_id)
    .bind(signup.registrant_email.as_str())
    .bind(signup.leader_email.as_str())
    .bind(date_param(signup.admin_ends_on_exclusive))
    .bind(ts_param(expires_at))
    .execute(&mut *tx)
    .await?;

    write_audit(
        &mut tx,
        at,
        Audit {
            tenant_id: Some(tenant_id),
            actor_kind: ActorKind::Registrant,
            actor_account_id: None,
            actor_membership_id: None,
            action: "tenant.signup_created",
            subject_type: "tenant",
            subject_id: tenant_id,
            params: json!({ "school_id": signup.school_id }),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(PendingTenant {
        tenant_id,
        expires_at,
    })
}

/// Attempts to place `tenant_id` as the school's live FAU, returning `None` when a
/// concurrent live tenant already holds the school (the partial unique index's
/// `on conflict ... do nothing`).
async fn try_insert_tenant(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    signup: &PendingSignup,
) -> Result<Option<Uuid>, MembershipError> {
    Ok(sqlx::query_scalar(
        "insert into tenants (id, name, status, school_id) values ($1, $2, 'pending', $3)
         on conflict (school_id) where status in ('pending', 'active') and school_id is not null
         do nothing
         returning id",
    )
    .bind(tenant_id)
    .bind(signup.fau_name.as_str())
    .bind(signup.school_id)
    .fetch_optional(&mut *conn)
    .await?)
}

/// The school's current live tenant, if any: its id (for the collision payload, spec
/// 3.2) and which state it is in.
async fn live_tenant_for_school(
    conn: &mut PgConnection,
    school_id: Uuid,
) -> Result<Option<(Uuid, ExistingFau)>, MembershipError> {
    let row: Option<(Uuid, String)> = sqlx::query_as(
        "select id, status from tenants where school_id = $1 and status in ('pending', 'active')",
    )
    .bind(school_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.and_then(|(id, status)| {
        let existing = match TenantStatus::from_code(&status) {
            Some(TenantStatus::Active) => ExistingFau::Active,
            Some(TenantStatus::Pending) => ExistingFau::Pending,
            _ => return None,
        };
        Some((id, existing))
    }))
}

/// Commits the collision copy to Erik and its audit entry, then returns the refusal.
/// `existing_tenant_id` names which FAU already holds the school, so a later render can
/// tell the two collisions on the same school apart -- an id, not personal data.
async fn record_collision(
    mut tx: sqlx::Transaction<'_, sqlx::Postgres>,
    at: Moment,
    school_id: Uuid,
    existing_tenant_id: Uuid,
    existing: ExistingFau,
) -> Result<PendingTenant, MembershipError> {
    let status = match existing {
        ExistingFau::Active => "active",
        ExistingFau::Pending => "pending",
    };
    enqueue(
        &mut tx,
        at,
        "signup.collision",
        EWB_OVERSIGHT_ADDRESS,
        json!({
            "school_id": school_id,
            "existing_status": status,
            "existing_tenant_id": existing_tenant_id,
        }),
    )
    .await?;
    write_audit(
        &mut tx,
        at,
        Audit {
            tenant_id: None,
            actor_kind: ActorKind::Registrant,
            actor_account_id: None,
            actor_membership_id: None,
            action: "tenant.signup_collision",
            subject_type: "school",
            subject_id: school_id,
            params: json!({
                "existing_status": status,
                "existing_tenant_id": existing_tenant_id,
            }),
        },
    )
    .await?;
    tx.commit().await?;
    Err(MembershipError::SchoolTaken(existing))
}

/// Deletes pending FAU-er whose signup has expired (spec 3.3: deleted, not closed, since
/// they never became FAU-er), optionally only for one school. Returns how many.
async fn expire_due(
    conn: &mut PgConnection,
    at: Moment,
    school_id: Option<Uuid>,
) -> Result<u64, MembershipError> {
    let expired: Vec<Uuid> = sqlx::query_scalar(
        "delete from tenants
          where status = 'pending'
            and id in (select tenant_id from tenant_signups where expires_at <= $1::timestamptz)
            and ($2::uuid is null or school_id = $2)
         returning id",
    )
    .bind(ts_param(at.now()))
    .bind(school_id)
    .fetch_all(&mut *conn)
    .await?;
    for tenant_id in &expired {
        write_audit(
            conn,
            at,
            Audit::system(
                *tenant_id,
                "tenant.signup_expired",
                "tenant",
                *tenant_id,
                json!({}),
            ),
        )
        .await?;
    }
    Ok(expired.len() as u64)
}

/// The scheduled sweep: deletes every expired pending FAU, freeing its school.
pub async fn expire_pending_tenants(pool: &PgPool, at: Moment) -> Result<u64, MembershipError> {
    let mut tx = pool.begin().await?;
    let n = expire_due(&mut tx, at, None).await?;
    tx.commit().await?;
    Ok(n)
}

#[derive(Debug, Clone)]
pub struct Activation {
    pub tenant_id: Uuid,
    /// The registrant's address, just verified with a Hanko passcode (#3417).
    pub registrant: VerifiedEmail,
}

#[derive(Debug)]
pub struct Activated {
    pub account_id: Uuid,
    pub membership_id: Uuid,
    pub admin_role_id: Uuid,
    pub admin_assignment_id: Uuid,
    /// `None` when the leader address is the registrant's own (spec 3.4.3).
    pub leader_invitation: Option<IssuedInvitation>,
}

/// Activates a pending FAU in one transaction (spec 3.4): status, the registrant's
/// account, membership and admin role, the leader invitation, the EWB recovery seat,
/// audit, and the outbox copy to EWB -- all or nothing.
///
/// The leader invitation is exempt from #3414's gate (decision 11); so is activation.
pub async fn activate_tenant(
    pool: &PgPool,
    activation: Activation,
    at: Moment,
) -> Result<Activated, MembershipError> {
    let tenant_id = activation.tenant_id;
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, tenant_id).await?;
    if state.status != TenantStatus::Pending {
        return Err(MembershipError::TenantNotPending);
    }
    // A frozen FAU takes no writes and issues no invitations (ADR-003 decision 7a), and
    // activation does both.
    if state.frozen {
        return Err(MembershipError::TenantFrozen);
    }
    let (registrant_email, leader_email, admin_end, expires_us): (String, String, String, i64) =
        sqlx::query_as(
            "select registrant_email, leader_email,
                    to_char(admin_ends_on_exclusive, 'YYYY-MM-DD'),
                    (extract(epoch from expires_at) * 1000000)::bigint
               from tenant_signups where tenant_id = $1",
        )
        .bind(tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(MembershipError::TenantNotPending)?;
    if at.now() >= from_micros(expires_us)? {
        return Err(MembershipError::SignupExpired);
    }
    if registrant_email != activation.registrant.email().as_str() {
        return Err(MembershipError::RegistrantMismatch);
    }
    let period = Period::new(at.today(), parse_date(&admin_end)?)
        .map_err(|_| MembershipError::EmptyPeriod)?;

    sqlx::query("update tenants set status = 'active' where id = $1")
        .bind(tenant_id)
        .execute(&mut *tx)
        .await?;

    let (account_id, disabled) = upsert_verified_account(&mut tx, &registrant_email, at).await?;
    if disabled {
        return Err(MembershipError::AccountDisabled);
    }
    let (membership_id, _) = ensure_membership(&mut tx, tenant_id, account_id).await?;

    let admin_role_id = Uuid::now_v7();
    sqlx::query(
        "insert into roles (tenant_id, id, name, capability_class) values ($1, $2, $3, $4)",
    )
    .bind(tenant_id)
    .bind(admin_role_id)
    .bind(BOOTSTRAP_ADMIN_ROLE_NAME)
    .bind(CapabilityClass::Admin.code())
    .execute(&mut *tx)
    .await?;
    let admin_assignment_id = insert_assignment(
        &mut tx,
        tenant_id,
        membership_id,
        admin_role_id,
        period,
        None,
    )
    .await?;

    let leader_invitation = if leader_email != registrant_email {
        let leader = Email::parse(&leader_email).map_err(|_| MembershipError::decode())?;
        Some(
            insert_invitation(
                &mut tx,
                at,
                NewInvitation {
                    tenant_id,
                    mode: InvitationMode::Activation,
                    recipient: leader,
                    issued_by: None,
                    handover_grant_id: None,
                    access_request_id: None,
                    recovery_holder: None,
                    roles: vec![(admin_role_id, period)],
                    actor_kind: ActorKind::Registrant,
                    actor_membership_id: Some(membership_id),
                },
            )
            .await?,
        )
    } else {
        None
    };

    sqlx::query("insert into recovery_contacts (tenant_id, holder) values ($1, $2)")
        .bind(tenant_id)
        .bind(RecoveryHolder::Ewb.code())
        .execute(&mut *tx)
        .await?;
    // Pending-only data: the registrant is now an account, the leader an invitation.
    sqlx::query("delete from tenant_signups where tenant_id = $1")
        .bind(tenant_id)
        .execute(&mut *tx)
        .await?;

    write_audit(
        &mut tx,
        at,
        Audit {
            tenant_id: Some(tenant_id),
            actor_kind: ActorKind::Registrant,
            actor_account_id: Some(account_id),
            actor_membership_id: Some(membership_id),
            action: "tenant.activated",
            subject_type: "tenant",
            subject_id: tenant_id,
            params: json!({
                "admin_assignment_id": admin_assignment_id,
                "leader_invited": leader_invitation.is_some(),
            }),
        },
    )
    .await?;
    enqueue(
        &mut tx,
        at,
        "tenant.activated",
        EWB_OVERSIGHT_ADDRESS,
        json!({ "tenant_id": tenant_id }),
    )
    .await?;
    tx.commit().await?;

    Ok(Activated {
        account_id,
        membership_id,
        admin_role_id,
        admin_assignment_id,
        leader_invitation,
    })
}
