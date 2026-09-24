//! The per-request access check (spec 2.1, 4; #3417 calls it on every request).

use fau_domain::membership::access::{
    evaluate_access, Access, AssignmentView, GrantView, Standing,
};
use fau_domain::membership::vocabulary::CapabilityClass;
use fau_domain::time::Moment;
use sqlx::PgPool;
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
    // Reads the two `Standing` fields that `USABLE_ACCOUNT` normally checks together
    // from its own named conjuncts, rather than retyping the SQL by hand (fix round 1,
    // task 10): `sql::tests::usable_account_is_the_conjunction_of_its_two_parts` keeps
    // all three in sync.
    let sql = format!(
        "select t.status = 'active',
                {ACCOUNT_USABLE},
                m.id,
                coalesce({MEMBERSHIP_NOT_REVOKED}, false)
           from tenants t
           cross join accounts a
           left join memberships m on m.tenant_id = t.id and m.account_id = a.id
          where t.id = $1 and a.id = $2"
    );
    let standing: Option<(bool, bool, Option<Uuid>, bool)> = sqlx::query_as(&sql)
        .bind(tenant_id)
        .bind(account_id)
        .fetch_optional(&mut *tx)
        .await?;
    let Some((tenant_active, account_usable, Some(membership_id), membership_active)) = standing
    else {
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
    .fetch_all(&mut *tx)
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
    .fetch_all(&mut *tx)
    .await?;
    let mut grants = Vec::with_capacity(rows.len());
    for (starts, ends, revoked) in rows {
        grants.push(GrantView {
            period: period_from(&starts, &ends)?,
            revoked,
        });
    }
    tx.commit().await?;

    Ok(evaluate_access(
        Standing {
            tenant_active,
            account_usable,
            membership_active,
        },
        &assignments,
        &grants,
        at.today(),
    ))
}
