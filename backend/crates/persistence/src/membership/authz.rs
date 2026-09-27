//! The one authorization function (groups design §3.3). Every read path, every group
//! mutation and every change-stream delivery asks [`authorize`]. It reads the database on
//! each call -- the viewer's standing today and the facts about the resource -- and
//! applies `fau_domain::authz::decide`, so a revocation takes effect on the next call.
//!
//! List reads (`groups::list_groups`) read the same facts with the same SQL
//! ([`in_group_sql`]) in one snapshot and call `decide` per row (Ruling R11). A test holds
//! the two equal.
//!
//! **Adding a resource type** (#3419's folders and documents, #3503's threads, ...)
//! means:
//! - a new [`Resource`] variant, resolved here to a `Target`, usually through its row's
//!   `group_id` and [`Resource::audience`];
//! - new rows in `tests/authorization.rs`.

use fau_domain::authz::{decide, Action, Decision, Denied, GroupFacts, Target};
use fau_domain::membership::access::Capability;
use fau_domain::membership::vocabulary::Visibility;
use fau_domain::time::Moment;
use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::access::membership_access;
use super::error::MembershipError;
use super::sql::date_param;

/// Who is asking: a membership in one FAU. #3417 resolves the session's account to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Viewer {
    pub tenant_id: Uuid,
    pub membership_id: Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Resource {
    /// FAU-wide content (audience null, §3.2), and the FAU itself for `Manage`: creating
    /// a group.
    Fau,
    /// A group itself: that it exists, its name, its member list.
    Group(Uuid),
    /// Content whose audience is the group.
    GroupContent(Uuid),
}

impl Resource {
    /// The audience of a resource that carries one nullable `group_id` (§3.2): null is
    /// FAU-wide. Threads, events, polls and folders resolve through this.
    pub fn audience(group_id: Option<Uuid>) -> Self {
        match group_id {
            Some(id) => Resource::GroupContent(id),
            None => Resource::Fau,
        }
    }
}

/// A role puts its holder into group `g` when it names the group (a guest role) or sits
/// on the unit or cohort the group is bound to (§3.1). Literal matches only: no traversal
/// through `unit_cohorts` (Ruling R6). `r` is `roles`.
pub(crate) const ROLE_FOLLOWS_GROUP: &str =
    "(r.group_id = g.id or r.unit_id = g.unit_id or r.cohort_id = g.cohort_id)";

/// Assignment `ra` is valid on the date bound as `$3`.
pub(crate) const ASSIGNMENT_VALID_ON_3: &str =
    "ra.revoked_at is null and ra.starts_on <= $3::date and ra.ends_on_exclusive > $3::date";

/// Whether membership `$2` is a current member of the group row aliased `g` on date `$3`:
/// added by hand and not removed, or holding a role valid on `$3` that the group follows.
/// Derived, never copied, so turnover needs no job (§3.1).
pub(crate) fn in_group_sql() -> String {
    format!(
        "(exists (select 1 from group_members gm
                   where gm.tenant_id = g.tenant_id and gm.group_id = g.id
                     and gm.membership_id = $2 and gm.removed_at is null)
          or exists (select 1 from role_assignments ra
                       join roles r on r.tenant_id = ra.tenant_id and r.id = ra.role_id
                      where ra.tenant_id = g.tenant_id and ra.membership_id = $2
                        and {ASSIGNMENT_VALID_ON_3}
                        and {ROLE_FOLLOWS_GROUP}))"
    )
}

/// Decides `action` on `resource` for `viewer`, reading the database on `conn`. Mutations
/// call it inside their own transaction, after `lock_tenant`. Read paths call it inside
/// [`read_transaction`], so the decision and the read see one snapshot.
///
/// The error is only ever a database failure. A refusal is the inner `Err(Denied)`:
/// - `NoAccess`: the viewer has no standing in the FAU;
/// - `Hidden`: the viewer may not read the resource, or the resource does not exist, and
///   the two look the same;
/// - `Forbidden`: the viewer may read the resource but not do this to it.
pub async fn authorize(
    conn: &mut PgConnection,
    viewer: Viewer,
    resource: Resource,
    action: Action,
    at: Moment,
) -> Result<Decision, MembershipError> {
    let access =
        membership_access(conn, viewer.tenant_id, viewer.membership_id, at.today()).await?;
    if access.capability == Capability::None {
        return Ok(Err(Denied::NoAccess));
    }
    let target = match resource {
        Resource::Fau => Target::Fau,
        Resource::Group(id) => match group_facts(conn, viewer, id, at).await? {
            Some(facts) => Target::Group(facts),
            None => return Ok(Err(Denied::Hidden)),
        },
        Resource::GroupContent(id) => match group_facts(conn, viewer, id, at).await? {
            Some(facts) => Target::GroupContent(facts),
            None => return Ok(Err(Denied::Hidden)),
        },
    };
    Ok(decide(access.capability, target, action))
}

async fn group_facts(
    conn: &mut PgConnection,
    viewer: Viewer,
    group_id: Uuid,
    at: Moment,
) -> Result<Option<GroupFacts>, MembershipError> {
    let sql = format!(
        "select g.visibility, g.archived_at is not null, {}
           from groups g
          where g.tenant_id = $1 and g.id = $4",
        in_group_sql()
    );
    let row: Option<(String, bool, bool)> = sqlx::query_as(&sql)
        .bind(viewer.tenant_id)
        .bind(viewer.membership_id)
        .bind(date_param(at.today()))
        .bind(group_id)
        .fetch_optional(&mut *conn)
        .await?;
    match row {
        None => Ok(None),
        Some((visibility, archived, viewer_in_group)) => Ok(Some(GroupFacts {
            visibility: Visibility::from_code(&visibility).ok_or_else(MembershipError::decode)?,
            archived,
            viewer_in_group,
        })),
    }
}

/// A snapshot for a read path: REPEATABLE READ, READ ONLY, so the authorization and the
/// read see the same database state (the reasoning of final review M3 in
/// `effective_access`).
pub async fn read_transaction(
    pool: &PgPool,
) -> Result<Transaction<'static, Postgres>, MembershipError> {
    let mut tx = pool.begin().await?;
    sqlx::query("set transaction isolation level repeatable read, read only")
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_audience_column_maps_to_a_resource() {
        let g = Uuid::now_v7();
        assert_eq!(Resource::audience(None), Resource::Fau);
        assert_eq!(Resource::audience(Some(g)), Resource::GroupContent(g));
    }

    #[test]
    fn in_group_is_built_from_the_shared_fragments() {
        let sql = in_group_sql();
        assert!(sql.contains(ROLE_FOLLOWS_GROUP) && sql.contains(ASSIGNMENT_VALID_ON_3));
    }
}
