//! The per-request access check (spec 2.1, 4; #3417 calls it on every request), and its
//! per-membership form, which `authorize` and the change stream share.
//!
//! A capability is standing, not permission. `Capability::Guest` reaches only its own
//! groups, and even a member does not read a closed group they are not in. So resource
//! reads ask `authorize` (groups design §3.3), and nothing may read `capability >= Member`
//! as "may read everything" (the #3418 read-path audit, #3501).

use fau_domain::membership::access::{
    evaluate_access, Access, AssignmentView, GrantView, Standing,
};
use fau_domain::membership::vocabulary::CapabilityClass;
use fau_domain::time::Moment;
use jiff::civil::Date;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::error::MembershipError;
use super::sql::{period_from, ACCOUNT_USABLE, MEMBERSHIP_NOT_REVOKED};

/// What `account_id` may do in `tenant_id` today, read fresh from the database. An
/// unknown account, tenant or membership is simply no access -- the answer never
/// reveals which of them is missing (spec 2.7).
pub async fn effective_access(
    pool: &PgPool,
    account_id: Uuid,
    tenant_id: Uuid,
    at: Moment,
) -> Result<Access, MembershipError> {
    let mut tx = pool.begin().await?;
    // One snapshot for every read (final review M3): at READ COMMITTED each statement
    // sees its own snapshot, so a revocation committing between them could pair a
    // membership's old standing with its new assignments. REPEATABLE READ fixes the
    // snapshot at the first query; READ ONLY because nothing here writes. It must be the
    // transaction's first statement.
    sqlx::query("set transaction isolation level repeatable read, read only")
        .execute(&mut *tx)
        .await?;
    let membership_id: Option<Uuid> =
        sqlx::query_scalar("select id from memberships where tenant_id = $1 and account_id = $2")
            .bind(tenant_id)
            .bind(account_id)
            .fetch_optional(&mut *tx)
            .await?;
    let access = match membership_id {
        Some(id) => membership_access(&mut tx, tenant_id, id, at.today()).await?,
        None => Access::NONE,
    };
    tx.commit().await?;
    Ok(access)
}

/// What `membership_id` may do in `tenant_id` on `today`, on the caller's connection, so
/// it shares the caller's transaction and snapshot. An unknown membership is no access.
pub(crate) async fn membership_access(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    membership_id: Uuid,
    today: Date,
) -> Result<Access, MembershipError> {
    // The two `Standing` fields that `USABLE_ACCOUNT` normally checks together, read from
    // its own named conjuncts rather than retyped by hand (fix round 1, task 10):
    // `sql::tests::usable_account_is_the_conjunction_of_its_two_parts` keeps all three
    // in sync.
    let sql = format!(
        "select t.status = 'active', ({ACCOUNT_USABLE}), {MEMBERSHIP_NOT_REVOKED}
           from memberships m
           join tenants t  on t.id = m.tenant_id
           join accounts a on a.id = m.account_id
          where m.tenant_id = $1 and m.id = $2"
    );
    let standing: Option<(bool, bool, bool)> = sqlx::query_as(&sql)
        .bind(tenant_id)
        .bind(membership_id)
        .fetch_optional(&mut *conn)
        .await?;
    let Some((tenant_active, account_usable, membership_active)) = standing else {
        return Ok(Access::NONE);
    };

    let rows: Vec<(String, String, String, bool)> = sqlx::query_as(
        "select r.capability_class,
                to_char(ra.starts_on, 'YYYY-MM-DD'), to_char(ra.ends_on_exclusive, 'YYYY-MM-DD'),
                ra.revoked_at is not null
           from role_assignments ra
           join roles r on r.tenant_id = ra.tenant_id and r.id = ra.role_id
          where ra.tenant_id = $1 and ra.membership_id = $2",
    )
    .bind(tenant_id)
    .bind(membership_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut assignments = Vec::with_capacity(rows.len());
    for (class, starts, ends, revoked) in rows {
        assignments.push(AssignmentView {
            capability: CapabilityClass::from_code(&class).ok_or_else(MembershipError::decode)?,
            period: period_from(&starts, &ends)?,
            revoked,
        });
    }

    // Defence in depth (Task 9's pattern in `sql.rs`, `AdminState::load` and
    // `handover_grant_valid`): a grant whose source assignment has since been revoked
    // is ignored outright, not just marked `revoked` on the view. Revoking an
    // assignment already cascades to revoke any grant it produced
    // (`revoke_role_assignment`), so this is a second, independent check rather than
    // the only one.
    let rows: Vec<(String, String, bool)> = sqlx::query_as(
        "select to_char(g.starts_on, 'YYYY-MM-DD'), to_char(g.ends_on_exclusive, 'YYYY-MM-DD'),
                g.revoked_at is not null
           from handover_grants g
           join role_assignments ra on ra.tenant_id = g.tenant_id and ra.id = g.source_assignment_id
          where g.tenant_id = $1 and ra.membership_id = $2 and ra.revoked_at is null",
    )
    .bind(tenant_id)
    .bind(membership_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut grants = Vec::with_capacity(rows.len());
    for (starts, ends, revoked) in rows {
        grants.push(GrantView {
            period: period_from(&starts, &ends)?,
            revoked,
        });
    }

    Ok(evaluate_access(
        Standing {
            tenant_active,
            account_usable,
            membership_active,
        },
        &assignments,
        &grants,
        today,
    ))
}
