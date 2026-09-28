//! Groups (groups design §3.1 and §3.3; #3501): arbitrary groups inside an FAU, open by
//! default and closable, optionally bound to a unit or a cohort so that their membership
//! follows roles.
//!
//! Managing groups is admin-only in the MVP. Every check goes through `authorize`, and
//! every change is audited and announced on the change stream in the same transaction.
//!
//! **Names are content.** The caller encrypts a name under the FAU's record key
//! (`fau_crypto::Unit::Record`) with `Aad::new(tenant_id, GROUP_NAME_AAD.0,
//! GROUP_NAME_AAD.1, group_id)` before calling in. This module stores and returns the
//! ciphertext only, and never writes a name to audit, to a NOTIFY or to a log.
//!
//! **Order** (as in the rest of `membership`): tenant state, then authority, then row state.
//! A frozen FAU refuses the additive actions (create, rename, open, add a member) and
//! allows the reducing ones (close, archive, remove a member), the line #3418 drew (Ruling
//! R14). An actor who may not see a group gets `UnknownGroup`, exactly as for an id that
//! does not exist.

use std::ops::RangeInclusive;

use fau_crypto::Ciphertext;
use fau_domain::authz::Action;
use fau_domain::membership::vocabulary::Visibility;
use fau_domain::time::Moment;
use serde_json::{json, Value};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::authz::{authorize, denied, Resource, Viewer};
use super::error::MembershipError;
use super::events::{notify, Change};
use super::sql::{
    lock_tenant, membership_and_account_state, require_open, ts_param, write_audit, Audit,
};

/// The associated data a group name is encrypted with, with the tenant and the group's id.
pub const GROUP_NAME_AAD: (&str, &str) = ("groups", "encrypted_name");

/// fau-crypto's envelope around a 1..=`GroupName::MAX_CHARS` name: 41 bytes of version,
/// nonce and tag, plus 1 to 400 bytes of UTF-8. The same bounds as the database's
/// `groups_name_is_an_envelope`.
pub const GROUP_NAME_CIPHERTEXT_BYTES: RangeInclusive<usize> = 42..=512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupBinding {
    Unit(Uuid),
    Cohort(Uuid),
}

#[derive(Debug, Clone)]
pub struct CreateGroup {
    pub tenant_id: Uuid,
    pub actor_membership_id: Uuid,
    /// Chosen by the caller (UUIDv7) before encrypting, so the name's AAD binds to it.
    pub group_id: Uuid,
    pub encrypted_name: Ciphertext,
    pub visibility: Visibility,
    pub binding: Option<GroupBinding>,
}

#[derive(Debug, Clone)]
pub struct RenameGroup {
    pub tenant_id: Uuid,
    pub actor_membership_id: Uuid,
    pub group_id: Uuid,
    pub encrypted_name: Ciphertext,
}

#[derive(Debug, Clone, Copy)]
pub struct SetGroupVisibility {
    pub tenant_id: Uuid,
    pub actor_membership_id: Uuid,
    pub group_id: Uuid,
    pub visibility: Visibility,
}

#[derive(Debug, Clone, Copy)]
pub struct ArchiveGroup {
    pub tenant_id: Uuid,
    pub actor_membership_id: Uuid,
    pub group_id: Uuid,
}

#[derive(Debug, Clone, Copy)]
pub struct GroupMemberChange {
    pub tenant_id: Uuid,
    pub actor_membership_id: Uuid,
    pub group_id: Uuid,
    pub membership_id: Uuid,
}

fn check_name(name: &Ciphertext) -> Result<(), MembershipError> {
    if GROUP_NAME_CIPHERTEXT_BYTES.contains(&name.len()) && name.as_bytes()[0] == 1 {
        Ok(())
    } else {
        Err(MembershipError::GroupNameMalformed)
    }
}

fn audit(
    tenant_id: Uuid,
    actor: Uuid,
    action: &'static str,
    group_id: Uuid,
    params: Value,
) -> Audit {
    Audit::member(tenant_id, actor, action, "group", group_id, params)
}

/// Authority to manage the group, then its archived flag under a row lock.
async fn manage(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    actor: Uuid,
    group_id: Uuid,
    at: Moment,
) -> Result<bool, MembershipError> {
    let viewer = Viewer {
        tenant_id,
        membership_id: actor,
    };
    authorize(conn, viewer, Resource::Group(group_id), Action::Manage, at)
        .await?
        .map_err(|d| denied(d, MembershipError::UnknownGroup))?;
    Ok(sqlx::query_scalar(
        "select archived_at is not null from groups where tenant_id = $1 and id = $2 for update",
    )
    .bind(tenant_id)
    .bind(group_id)
    .fetch_one(&mut *conn)
    .await?)
}

async fn require_exists(
    conn: &mut PgConnection,
    table: &'static str,
    tenant_id: Uuid,
    id: Uuid,
    missing: MembershipError,
) -> Result<(), MembershipError> {
    let sql = format!("select exists (select 1 from {table} where tenant_id = $1 and id = $2)");
    let found: bool = sqlx::query_scalar(&sql)
        .bind(tenant_id)
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
    if found {
        Ok(())
    } else {
        Err(missing)
    }
}

/// Creates a group (§3.1). Admin only; refused while the FAU is frozen.
pub async fn create_group(
    pool: &PgPool,
    req: CreateGroup,
    at: Moment,
) -> Result<Uuid, MembershipError> {
    check_name(&req.encrypted_name)?;
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, req.tenant_id).await?;
    require_open(&state)?;
    let viewer = Viewer {
        tenant_id: req.tenant_id,
        membership_id: req.actor_membership_id,
    };
    authorize(&mut tx, viewer, Resource::Fau, Action::Manage, at)
        .await?
        .map_err(|d| denied(d, MembershipError::NotAuthorized))?;
    let (unit_id, cohort_id) = match req.binding {
        None => (None, None),
        Some(GroupBinding::Unit(id)) => {
            require_exists(
                &mut tx,
                "organization_units",
                req.tenant_id,
                id,
                MembershipError::UnknownUnit,
            )
            .await?;
            (Some(id), None)
        }
        Some(GroupBinding::Cohort(id)) => {
            require_exists(
                &mut tx,
                "cohorts",
                req.tenant_id,
                id,
                MembershipError::UnknownCohort,
            )
            .await?;
            (None, Some(id))
        }
    };
    sqlx::query(
        "insert into groups
           (tenant_id, id, encrypted_name, visibility, unit_id, cohort_id, created_by, created_at)
         values ($1, $2, $3, $4, $5, $6, $7, $8::timestamptz)",
    )
    .bind(req.tenant_id)
    .bind(req.group_id)
    .bind(req.encrypted_name.as_bytes())
    .bind(req.visibility.code())
    .bind(unit_id)
    .bind(cohort_id)
    .bind(req.actor_membership_id)
    .bind(ts_param(at.now()))
    .execute(&mut *tx)
    .await?;
    write_audit(
        &mut tx,
        at,
        audit(
            req.tenant_id,
            req.actor_membership_id,
            "group.created",
            req.group_id,
            json!({ "visibility": req.visibility.code(), "unit_id": unit_id, "cohort_id": cohort_id }),
        ),
    )
    .await?;
    notify(
        &mut tx,
        req.tenant_id,
        Change::Changed(Resource::Group(req.group_id)),
    )
    .await?;
    tx.commit().await?;
    Ok(req.group_id)
}

/// Replaces a group's encrypted name. Admin only; refused while frozen or archived.
pub async fn rename_group(
    pool: &PgPool,
    req: RenameGroup,
    at: Moment,
) -> Result<(), MembershipError> {
    check_name(&req.encrypted_name)?;
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, req.tenant_id).await?;
    require_open(&state)?;
    if manage(
        &mut tx,
        req.tenant_id,
        req.actor_membership_id,
        req.group_id,
        at,
    )
    .await?
    {
        return Err(MembershipError::GroupArchived);
    }
    sqlx::query("update groups set encrypted_name = $3 where tenant_id = $1 and id = $2")
        .bind(req.tenant_id)
        .bind(req.group_id)
        .bind(req.encrypted_name.as_bytes())
        .execute(&mut *tx)
        .await?;
    write_audit(
        &mut tx,
        at,
        audit(
            req.tenant_id,
            req.actor_membership_id,
            "group.renamed",
            req.group_id,
            json!({}),
        ),
    )
    .await?;
    notify(
        &mut tx,
        req.tenant_id,
        Change::Changed(Resource::Group(req.group_id)),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Closes or opens a group (D11). Closing reduces access, so it is allowed while the FAU is
/// frozen; opening is not. Setting the current visibility again writes nothing.
pub async fn set_group_visibility(
    pool: &PgPool,
    req: SetGroupVisibility,
    at: Moment,
) -> Result<(), MembershipError> {
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, req.tenant_id).await?;
    if req.visibility == Visibility::Open {
        require_open(&state)?;
    }
    if manage(
        &mut tx,
        req.tenant_id,
        req.actor_membership_id,
        req.group_id,
        at,
    )
    .await?
    {
        return Err(MembershipError::GroupArchived);
    }
    let current: String =
        sqlx::query_scalar("select visibility from groups where tenant_id = $1 and id = $2")
            .bind(req.tenant_id)
            .bind(req.group_id)
            .fetch_one(&mut *tx)
            .await?;
    if current == req.visibility.code() {
        tx.commit().await?;
        return Ok(());
    }
    sqlx::query("update groups set visibility = $3 where tenant_id = $1 and id = $2")
        .bind(req.tenant_id)
        .bind(req.group_id)
        .bind(req.visibility.code())
        .execute(&mut *tx)
        .await?;
    let action = match req.visibility {
        Visibility::Closed => "group.closed",
        Visibility::Open => "group.opened",
    };
    write_audit(
        &mut tx,
        at,
        audit(
            req.tenant_id,
            req.actor_membership_id,
            action,
            req.group_id,
            json!({}),
        ),
    )
    .await?;
    notify(
        &mut tx,
        req.tenant_id,
        Change::Changed(Resource::Group(req.group_id)),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Archives a group, one way (Ruling R13): it stays readable as history and takes no
/// writes. Allowed while frozen.
pub async fn archive_group(
    pool: &PgPool,
    req: ArchiveGroup,
    at: Moment,
) -> Result<(), MembershipError> {
    let mut tx = pool.begin().await?;
    lock_tenant(&mut tx, req.tenant_id).await?;
    if manage(
        &mut tx,
        req.tenant_id,
        req.actor_membership_id,
        req.group_id,
        at,
    )
    .await?
    {
        return Err(MembershipError::GroupArchived);
    }
    sqlx::query("update groups set archived_at = $3::timestamptz where tenant_id = $1 and id = $2")
        .bind(req.tenant_id)
        .bind(req.group_id)
        .bind(ts_param(at.now()))
        .execute(&mut *tx)
        .await?;
    write_audit(
        &mut tx,
        at,
        audit(
            req.tenant_id,
            req.actor_membership_id,
            "group.archived",
            req.group_id,
            json!({}),
        ),
    )
    .await?;
    notify(
        &mut tx,
        req.tenant_id,
        Change::Changed(Resource::Group(req.group_id)),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Adds a member by hand: anyone with a usable membership, a guest or a member of a bound
/// group's unit included (§3.1). Admin only; refused while frozen or archived.
pub async fn add_group_member(
    pool: &PgPool,
    req: GroupMemberChange,
    at: Moment,
) -> Result<(), MembershipError> {
    let mut tx = pool.begin().await?;
    let state = lock_tenant(&mut tx, req.tenant_id).await?;
    require_open(&state)?;
    if manage(
        &mut tx,
        req.tenant_id,
        req.actor_membership_id,
        req.group_id,
        at,
    )
    .await?
    {
        return Err(MembershipError::GroupArchived);
    }
    match membership_and_account_state(&mut tx, req.tenant_id, req.membership_id).await? {
        None => return Err(MembershipError::UnknownMembership),
        Some((true, _)) => return Err(MembershipError::MembershipRevoked),
        Some((false, false)) => return Err(MembershipError::AccountDisabled),
        Some((false, true)) => {}
    }
    let already: bool = sqlx::query_scalar(
        "select exists (select 1 from group_members
                         where tenant_id = $1 and group_id = $2 and membership_id = $3
                           and removed_at is null)",
    )
    .bind(req.tenant_id)
    .bind(req.group_id)
    .bind(req.membership_id)
    .fetch_one(&mut *tx)
    .await?;
    if already {
        return Err(MembershipError::AlreadyInGroup);
    }
    sqlx::query(
        "insert into group_members (tenant_id, id, group_id, membership_id, added_by, added_at)
         values ($1, $2, $3, $4, $5, $6::timestamptz)",
    )
    .bind(req.tenant_id)
    .bind(Uuid::now_v7())
    .bind(req.group_id)
    .bind(req.membership_id)
    .bind(req.actor_membership_id)
    .bind(ts_param(at.now()))
    .execute(&mut *tx)
    .await?;
    write_audit(
        &mut tx,
        at,
        audit(
            req.tenant_id,
            req.actor_membership_id,
            "group.member_added",
            req.group_id,
            json!({ "membership_id": req.membership_id }),
        ),
    )
    .await?;
    notify(
        &mut tx,
        req.tenant_id,
        Change::Changed(Resource::Group(req.group_id)),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Removes a hand-added member, softly. Membership through a role is ended on the role,
/// not here (`NotInGroup`). Allowed while frozen and on an archived group. Closes the
/// removed member's live streams (Ruling R3).
pub async fn remove_group_member(
    pool: &PgPool,
    req: GroupMemberChange,
    at: Moment,
) -> Result<(), MembershipError> {
    let mut tx = pool.begin().await?;
    lock_tenant(&mut tx, req.tenant_id).await?;
    manage(
        &mut tx,
        req.tenant_id,
        req.actor_membership_id,
        req.group_id,
        at,
    )
    .await?;
    let row: Option<Uuid> = sqlx::query_scalar(
        "select id from group_members
          where tenant_id = $1 and group_id = $2 and membership_id = $3 and removed_at is null
          for update",
    )
    .bind(req.tenant_id)
    .bind(req.group_id)
    .bind(req.membership_id)
    .fetch_optional(&mut *tx)
    .await?;
    let row = row.ok_or(MembershipError::NotInGroup)?;
    sqlx::query(
        "update group_members set removed_at = $3::timestamptz where tenant_id = $1 and id = $2",
    )
    .bind(req.tenant_id)
    .bind(row)
    .bind(ts_param(at.now()))
    .execute(&mut *tx)
    .await?;
    write_audit(
        &mut tx,
        at,
        audit(
            req.tenant_id,
            req.actor_membership_id,
            "group.member_removed",
            req.group_id,
            json!({ "membership_id": req.membership_id }),
        ),
    )
    .await?;
    notify(
        &mut tx,
        req.tenant_id,
        Change::AccessRevoked {
            membership_id: req.membership_id,
        },
    )
    .await?;
    notify(
        &mut tx,
        req.tenant_id,
        Change::Changed(Resource::Group(req.group_id)),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// `revoke_membership`'s cascade (Ruling R15): the membership leaves every group it was
/// added to by hand, each removal audited. Without it, a re-invite (which reopens the same
/// membership row) would hand a removed person their closed groups back. Runs inside the
/// caller's transaction, under its tenant lock.
pub(crate) async fn remove_from_all_groups(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    actor_membership_id: Uuid,
    membership_id: Uuid,
    at: Moment,
) -> Result<(), MembershipError> {
    let groups: Vec<Uuid> = sqlx::query_scalar(
        "update group_members set removed_at = $3::timestamptz
          where tenant_id = $1 and membership_id = $2 and removed_at is null
         returning group_id",
    )
    .bind(tenant_id)
    .bind(membership_id)
    .bind(ts_param(at.now()))
    .fetch_all(&mut *conn)
    .await?;
    for group_id in groups {
        write_audit(
            conn,
            at,
            audit(
                tenant_id,
                actor_membership_id,
                "group.member_removed",
                group_id,
                json!({ "membership_id": membership_id, "cause": "membership_revoked" }),
            ),
        )
        .await?;
        notify(conn, tenant_id, Change::Changed(Resource::Group(group_id))).await?;
    }
    Ok(())
}
