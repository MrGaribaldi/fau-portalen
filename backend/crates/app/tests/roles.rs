//! Granting and revoking roles and memberships, and the last-admin safeguard (flow
//! spec §7; §10 "The last-admin safeguard requires confirmation in each of these cases").

mod common;
use common::membership::*;
use common::TestDb;
use fau_domain::membership::vocabulary::CapabilityClass;
use fau_persistence::membership::{
    grant_role, revoke_membership, revoke_role_assignment, GrantRole, MembershipError,
    RevokeAssignment, RevokeMembership, RoleChoice,
};
use sqlx::PgPool;
use uuid::Uuid;

async fn revoked(pool: &PgPool, table: &str, id: Uuid) -> bool {
    sqlx::query_scalar(&format!(
        "select revoked_at is not null from {table} where id = $1"
    ))
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn an_admin_grants_a_role_to_a_member() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let member = add_member(
        &pool,
        &fau,
        "medlem@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;

    let grant = |actor| GrantRole {
        tenant_id: fau.tenant_id,
        actor_membership_id: actor,
        membership_id: member.membership_id,
        role: new_role("Kasserer", CapabilityClass::Member),
        period: period(day(2026, 10, 1), day(2027, 10, 1)),
    };
    assert_eq!(
        grant_role(&pool, grant(member.membership_id), t0)
            .await
            .unwrap_err(),
        MembershipError::NotAuthorized,
        "a member cannot grant, not even to themselves"
    );
    let id = grant_role(&pool, grant(fau.admin_membership_id), t0)
        .await
        .unwrap();
    let granted_by: Option<Uuid> =
        sqlx::query_scalar("select granted_by from role_assignments where id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(granted_by, Some(fau.admin_membership_id));
    assert_eq!(audit_count(&pool, "role.granted").await, 1);
}

#[tokio::test]
async fn a_revoked_membership_cannot_be_granted_a_role() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let member = add_member(
        &pool,
        &fau,
        "medlem@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: member.membership_id,
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .unwrap();
    let err = grant_role(
        &pool,
        GrantRole {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: member.membership_id,
            role: new_role("Kasserer", CapabilityClass::Member),
            period: period(day(2026, 10, 1), day(2027, 10, 1)),
        },
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::MembershipRevoked);
}

#[tokio::test]
async fn an_admin_revokes_a_role_at_once() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let member = add_member(
        &pool,
        &fau,
        "medlem@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    let revoke = RevokeAssignment {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        assignment_id: member.assignment_ids[0],
        confirm_no_admin: false,
    };
    revoke_role_assignment(&pool, revoke, t0).await.unwrap();
    assert!(revoked(&pool, "role_assignments", member.assignment_ids[0]).await);
    assert_eq!(
        revoke_role_assignment(&pool, revoke, t0).await.unwrap_err(),
        MembershipError::AssignmentAlreadyRevoked
    );
}

#[tokio::test]
async fn a_member_cannot_revoke_someone_elses_role() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let member = add_member(
        &pool,
        &fau,
        "medlem@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    let err = revoke_role_assignment(
        &pool,
        RevokeAssignment {
            tenant_id: fau.tenant_id,
            actor_membership_id: member.membership_id,
            assignment_id: fau.admin_assignment_id,
            confirm_no_admin: true,
        },
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::NotAuthorized);
}

/// Controller ruling (fix round 1, item 6): a member may revoke their own role -- step
/// down from a single role -- without being an admin.
#[tokio::test]
async fn a_member_can_revoke_their_own_role() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let member = add_member(
        &pool,
        &fau,
        "medlem@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    revoke_role_assignment(
        &pool,
        RevokeAssignment {
            tenant_id: fau.tenant_id,
            actor_membership_id: member.membership_id,
            assignment_id: member.assignment_ids[0],
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .expect("a member may step down from their own role");
    assert!(revoked(&pool, "role_assignments", member.assignment_ids[0]).await);
}

/// Fix round 1, item 1: authority before state. A non-admin's target is looked up
/// scoped to their own membership, so an id nobody holds is indistinguishable from one
/// that belongs to somebody else -- both come back `NotAuthorized`, never
/// `UnknownAssignment`.
#[tokio::test]
async fn a_member_targeting_an_unknown_assignment_gets_not_authorized() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let member = add_member(
        &pool,
        &fau,
        "medlem@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    let err = revoke_role_assignment(
        &pool,
        RevokeAssignment {
            tenant_id: fau.tenant_id,
            actor_membership_id: member.membership_id,
            assignment_id: Uuid::now_v7(),
            confirm_no_admin: true,
        },
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(
        err,
        MembershipError::NotAuthorized,
        "a non-admin learns nothing about whether the id exists"
    );
}

/// Fix round 1, item 1: a non-admin targeting someone else's assignment gets
/// `NotAuthorized` whether or not it is already revoked -- never
/// `AssignmentAlreadyRevoked`, which would reveal its state to someone with no claim on
/// it.
#[tokio::test]
async fn a_member_targeting_someone_elses_already_revoked_assignment_gets_not_authorized() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let target = add_member(
        &pool,
        &fau,
        "medlem@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    let outsider = add_member(
        &pool,
        &fau,
        "utenfor@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    revoke_role_assignment(
        &pool,
        RevokeAssignment {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            assignment_id: target.assignment_ids[0],
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .unwrap();

    let err = revoke_role_assignment(
        &pool,
        RevokeAssignment {
            tenant_id: fau.tenant_id,
            actor_membership_id: outsider.membership_id,
            assignment_id: target.assignment_ids[0],
            confirm_no_admin: true,
        },
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(
        err,
        MembershipError::NotAuthorized,
        "a non-admin learns nothing about someone else's assignment, revoked or not"
    );
}

/// Fix round 1, item 2: the same admin gate applies to `revoke_membership` -- a plain
/// member cannot revoke someone else's membership.
#[tokio::test]
async fn a_member_cannot_revoke_someone_elses_membership() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let a = add_member(
        &pool,
        &fau,
        "a@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    let b = add_member(
        &pool,
        &fau,
        "b@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    let err = revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: a.membership_id,
            membership_id: b.membership_id,
            confirm_no_admin: true,
        },
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::NotAuthorized);
    assert!(!revoked(&pool, "memberships", b.membership_id).await);
}

/// Fix round 1, item 7: `grant_role` refuses a target whose account is not usable,
/// using the same `USABLE_ACCOUNT` condition the last-admin safeguard relies on.
#[tokio::test]
async fn grant_role_refuses_a_target_with_a_disabled_account() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let admin_pool = db.admin_pool();
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let member = add_member(
        &pool,
        &fau,
        "medlem@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    // The runtime cannot disable an account itself -- superuser pool (fix round 1, item 5).
    sqlx::query("update accounts set disabled_at = now() where id = $1")
        .bind(member.account_id)
        .execute(&admin_pool)
        .await
        .unwrap();

    let err = grant_role(
        &pool,
        GrantRole {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: member.membership_id,
            role: new_role("Kasserer", CapabilityClass::Member),
            period: period(day(2026, 10, 1), day(2027, 10, 1)),
        },
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::AccountDisabled);
}

#[tokio::test]
async fn revoking_oneself_as_the_only_admin_needs_confirmation() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let revoke = |confirm_no_admin| RevokeAssignment {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        assignment_id: fau.admin_assignment_id,
        confirm_no_admin,
    };

    assert_eq!(
        revoke_role_assignment(&pool, revoke(false), t0)
            .await
            .unwrap_err(),
        MembershipError::WouldLeaveNoAdmin
    );
    assert!(!revoked(&pool, "role_assignments", fau.admin_assignment_id).await);
    revoke_role_assignment(&pool, revoke(true), t0)
        .await
        .unwrap();
    assert!(revoked(&pool, "role_assignments", fau.admin_assignment_id).await);
    let flagged: bool = sqlx::query_scalar(
        "select (params->>'left_no_admin')::boolean from audit_events where action = 'role.revoked'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(flagged);
}

#[tokio::test]
async fn leaving_as_the_only_admin_needs_confirmation() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let leave = |confirm_no_admin| RevokeMembership {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        membership_id: fau.admin_membership_id,
        confirm_no_admin,
    };

    assert_eq!(
        revoke_membership(&pool, leave(false), t0)
            .await
            .unwrap_err(),
        MembershipError::WouldLeaveNoAdmin
    );
    revoke_membership(&pool, leave(true), t0).await.unwrap();
    assert!(revoked(&pool, "memberships", fau.admin_membership_id).await);
    assert!(
        revoked(&pool, "role_assignments", fau.admin_assignment_id).await,
        "leaving ends the roles too"
    );
    assert_eq!(audit_count(&pool, "membership.left").await, 1);
}

#[tokio::test]
async fn revoking_the_only_other_admin_needs_confirmation() {
    // The registrant's own role ends 2027-10-01. The second admin would have carried
    // the FAU until 2028-06-01; revoking them leaves no admin from 2027-10-01.
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let other = add_member(
        &pool,
        &fau,
        "admin2@example.test",
        RoleChoice::Existing(fau.admin_role_id),
        period(day(2026, 9, 23), day(2028, 6, 1)),
        t0,
    )
    .await;
    let revoke = |confirm_no_admin| RevokeMembership {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        membership_id: other.membership_id,
        confirm_no_admin,
    };

    assert_eq!(
        revoke_membership(&pool, revoke(false), t0)
            .await
            .unwrap_err(),
        MembershipError::WouldLeaveNoAdmin
    );
    revoke_membership(&pool, revoke(true), t0).await.unwrap();
    assert_eq!(audit_count(&pool, "membership.revoked").await, 1);
}

#[tokio::test]
async fn revoking_an_admin_whose_term_is_covered_needs_no_confirmation() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let other = add_member(
        &pool,
        &fau,
        "admin2@example.test",
        RoleChoice::Existing(fau.admin_role_id),
        period(day(2026, 9, 23), day(2027, 10, 1)),
        t0,
    )
    .await;
    revoke_role_assignment(
        &pool,
        RevokeAssignment {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            assignment_id: other.assignment_ids[0],
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .expect("the remaining admin covers the same term");
}

/// Review carry-over 1: `AdminState`'s queries filter on `USABLE_ACCOUNT` and on
/// `revoked_at is null`, so a revoked membership does not count as "an admin" for the
/// safeguard even while its role assignment row itself is still unrevoked. Sabotaging
/// the membership row directly (no persistence function reaches this shape -- revoking
/// a membership through `revoke_membership` revokes its role assignments too) is the
/// only way to construct it, the same way `handover_only_actor` in `requests.rs` does.
#[tokio::test]
async fn a_revoked_membership_with_an_unrevoked_admin_assignment_is_not_an_admin() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let admin_pool = db.admin_pool();
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let other = add_member(
        &pool,
        &fau,
        "admin2@example.test",
        RoleChoice::Existing(fau.admin_role_id),
        period(day(2026, 9, 23), day(2028, 6, 1)),
        t0,
    )
    .await;
    // No persistence function reaches this shape -- arranged directly with the
    // superuser pool, per the global constraints (fix round 1, item 5).
    sqlx::query("update memberships set revoked_at = now() where tenant_id = $1 and id = $2")
        .bind(fau.tenant_id)
        .bind(other.membership_id)
        .execute(&admin_pool)
        .await
        .unwrap();
    assert!(
        !revoked(&pool, "role_assignments", other.assignment_ids[0]).await,
        "the assignment itself stays unrevoked; only the membership was sabotaged"
    );

    let err = revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: fau.admin_membership_id,
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(
        err,
        MembershipError::WouldLeaveNoAdmin,
        "the revoked membership's admin role does not cover the FAU"
    );
}

/// Review carry-over 1: a disabled account's admin role does not count either.
#[tokio::test]
async fn a_disabled_accounts_admin_role_is_not_an_admin() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let admin_pool = db.admin_pool();
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let other = add_member(
        &pool,
        &fau,
        "admin2@example.test",
        RoleChoice::Existing(fau.admin_role_id),
        period(day(2026, 9, 23), day(2028, 6, 1)),
        t0,
    )
    .await;
    // The runtime cannot disable an account itself -- superuser pool (fix round 1, item 5).
    sqlx::query("update accounts set disabled_at = now() where id = $1")
        .bind(other.account_id)
        .execute(&admin_pool)
        .await
        .unwrap();

    let err = revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: fau.admin_membership_id,
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::WouldLeaveNoAdmin);
}

/// Review carry-over 2: Task 6's `ensure_membership` reopens a revoked membership
/// without touching its role assignments, so `revoke_membership` must revoke the open
/// assignments itself -- otherwise a person invited back would find their old roles
/// still valid alongside the newly offered one.
#[tokio::test]
async fn rejoining_after_revocation_brings_back_only_the_newly_offered_roles() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let first = add_member(
        &pool,
        &fau,
        "medlem@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: first.membership_id,
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .unwrap();
    assert!(revoked(&pool, "role_assignments", first.assignment_ids[0]).await);

    let second = add_member(
        &pool,
        &fau,
        "medlem@example.test",
        new_role("Kasserer", CapabilityClass::Member),
        period(day(2026, 10, 1), day(2027, 10, 1)),
        t0,
    )
    .await;
    assert_eq!(
        second.membership_id, first.membership_id,
        "the same person rejoins the same membership row"
    );
    assert!(
        revoked(&pool, "role_assignments", first.assignment_ids[0]).await,
        "the old role does not come back"
    );
    assert!(
        !revoked(&pool, "role_assignments", second.assignment_ids[0]).await,
        "only the newly offered role is valid"
    );
    assert!(!revoked(&pool, "memberships", second.membership_id).await);
}

/// Builds a membership whose only admin-class role assignment has already ended, and a
/// still-valid handover grant sourced from it (spec 6.2 -- the grant's window starts
/// exactly where the assignment's ends). No persistence function creates a grant yet
/// (that is a scheduled sweep, out of scope here per the global constraints), so the
/// shape is arranged directly, row-for-row against 0003's schema, with the superuser
/// pool -- the same convention `requests.rs`'s `handover_only_actor` uses for the
/// identical shape (fix round 1, item 4).
async fn ended_admin_with_live_grant(
    admin_pool: &PgPool,
    fau: &Fau,
    address: &str,
) -> (Uuid, Uuid, Uuid) {
    let account_id = Uuid::now_v7();
    sqlx::query("insert into accounts (id, email, verified_at) values ($1, $2, now())")
        .bind(account_id)
        .bind(address)
        .execute(admin_pool)
        .await
        .unwrap();
    let membership_id = Uuid::now_v7();
    sqlx::query("insert into memberships (tenant_id, id, account_id) values ($1, $2, $3)")
        .bind(fau.tenant_id)
        .bind(membership_id)
        .bind(account_id)
        .execute(admin_pool)
        .await
        .unwrap();
    let assignment_id = Uuid::now_v7();
    sqlx::query(
        "insert into role_assignments
           (tenant_id, id, membership_id, role_id, starts_on, ends_on_exclusive)
         values ($1, $2, $3, $4, $5::date, $6::date)",
    )
    .bind(fau.tenant_id)
    .bind(assignment_id)
    .bind(membership_id)
    .bind(fau.admin_role_id)
    .bind("2026-01-01")
    .bind("2026-09-01")
    .execute(admin_pool)
    .await
    .unwrap();
    let grant_id = Uuid::now_v7();
    sqlx::query(
        "insert into handover_grants (tenant_id, id, source_assignment_id, starts_on, ends_on_exclusive)
         values ($1, $2, $3, $4::date, $5::date)",
    )
    .bind(fau.tenant_id)
    .bind(grant_id)
    .bind(assignment_id)
    .bind("2026-09-01")
    .bind("2027-03-01")
    .execute(admin_pool)
    .await
    .unwrap();
    (membership_id, assignment_id, grant_id)
}

/// Review carry-over 3 / #3412, spec 6.2: revoking an assignment revokes any handover
/// grant already derived from it, so the safeguard is not fooled by a grant surviving
/// past the role it came from. The source assignment has already ended and the grant is
/// the live thing actually covering the FAU today (fix round 1, item 4's realistic
/// fixture) -- the registrant's own role covers the same window, so no confirmation is
/// needed either.
#[tokio::test]
async fn revoking_an_assignment_revokes_its_handover_grant_too() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let admin_pool = db.admin_pool();
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let (_, assignment_id, grant_id) =
        ended_admin_with_live_grant(&admin_pool, &fau, "avtroppende@example.test").await;

    revoke_role_assignment(
        &pool,
        RevokeAssignment {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            assignment_id,
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .expect("the registrant's own role covers the FAU throughout the grant's remaining term");
    assert!(revoked(&pool, "role_assignments", assignment_id).await);
    assert!(revoked(&pool, "handover_grants", grant_id).await);
}

/// Fix round 1, item 3: `revoke_membership` revokes a handover grant derived from any
/// assignment the membership holds, even one that had already ended before the
/// membership itself was revoked.
#[tokio::test]
async fn revoke_membership_revokes_a_handover_grant_from_an_already_ended_role() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let admin_pool = db.admin_pool();
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let (membership_id, _, grant_id) =
        ended_admin_with_live_grant(&admin_pool, &fau, "avtroppende@example.test").await;

    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id,
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .expect("the registrant's own role covers the FAU throughout the grant's remaining term");
    assert!(revoked(&pool, "memberships", membership_id).await);
    assert!(revoked(&pool, "handover_grants", grant_id).await);
}

#[tokio::test]
async fn leaving_one_fau_leaves_the_other_untouched() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let first = active_fau(&pool, "begge@example.test", t0).await;
    let second = active_fau(&pool, "begge@example.test", t0).await;
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: first.tenant_id,
            actor_membership_id: first.admin_membership_id,
            membership_id: first.admin_membership_id,
            confirm_no_admin: true,
        },
        t0,
    )
    .await
    .unwrap();
    assert!(!revoked(&pool, "memberships", second.admin_membership_id).await);
    assert!(!revoked(&pool, "role_assignments", second.admin_assignment_id).await);
}

async fn set_tenant(db: &TestDb, tenant_id: Uuid, sql: &str) {
    sqlx::query(sql)
        .bind(tenant_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();
}

/// Final review M7: granting adds rights, so a frozen FAU refuses it.
#[tokio::test]
async fn a_frozen_fau_refuses_granting_a_role() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let member = add_member(
        &pool,
        &fau,
        "medlem@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    set_tenant(
        &db,
        fau.tenant_id,
        "update tenants set frozen_at = now() where id = $1",
    )
    .await;
    assert_eq!(
        grant_role(
            &pool,
            GrantRole {
                tenant_id: fau.tenant_id,
                actor_membership_id: fau.admin_membership_id,
                membership_id: member.membership_id,
                role: new_role("Kasserer", CapabilityClass::Member),
                period: period(day(2026, 10, 1), day(2027, 10, 1)),
            },
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::TenantFrozen
    );
}

/// Final review M7: `require_open` reports a closed FAU as `TenantNotActive`, checked
/// here through `grant_role`.
#[tokio::test]
async fn a_closed_fau_refuses_granting_a_role_as_not_active() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    set_tenant(
        &db,
        fau.tenant_id,
        "update tenants set status = 'closed' where id = $1",
    )
    .await;
    assert_eq!(
        grant_role(
            &pool,
            GrantRole {
                tenant_id: fau.tenant_id,
                actor_membership_id: fau.admin_membership_id,
                membership_id: fau.admin_membership_id,
                role: new_role("Kasserer", CapabilityClass::Member),
                period: period(day(2026, 10, 1), day(2027, 10, 1)),
            },
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::TenantNotActive
    );
}

/// Final review M7, controller ruling: revocation only reduces rights, so a frozen FAU
/// allows revoking a role and revoking a membership.
#[tokio::test]
async fn a_frozen_fau_still_allows_revocation() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let first = add_member(
        &pool,
        &fau,
        "en@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    let second = add_member(
        &pool,
        &fau,
        "to@example.test",
        new_role("Kasserer", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    set_tenant(
        &db,
        fau.tenant_id,
        "update tenants set frozen_at = now() where id = $1",
    )
    .await;

    revoke_role_assignment(
        &pool,
        RevokeAssignment {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            assignment_id: first.assignment_ids[0],
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .expect("revoking a role is allowed on a frozen FAU");
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: second.membership_id,
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .expect("revoking a membership is allowed on a frozen FAU");
    assert!(revoked(&pool, "role_assignments", first.assignment_ids[0]).await);
    assert!(revoked(&pool, "memberships", second.membership_id).await);
}

/// Final review M7: two admins each revoke the other's admin assignment at the same
/// moment, without confirming. `lock_tenant` serialises them: the first succeeds, and
/// the second finds its actor no longer an admin. The FAU is never left with no admin
/// without confirmation. Repeated, so an interleaving that slips past the lock is
/// likely to show.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_admins_revoking_each_other_at_once_never_leave_the_fau_without_an_admin() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    for round in 0..10 {
        let fau = active_fau(&pool, &format!("a{round}@example.test"), t0).await;
        let other = add_member(
            &pool,
            &fau,
            &format!("b{round}@example.test"),
            RoleChoice::Existing(fau.admin_role_id),
            period(day(2026, 9, 23), day(2027, 10, 1)),
            t0,
        )
        .await;
        let revoke = |actor: Uuid, assignment_id: Uuid| {
            let pool = pool.clone();
            let tenant_id = fau.tenant_id;
            tokio::spawn(async move {
                revoke_role_assignment(
                    &pool,
                    RevokeAssignment {
                        tenant_id,
                        actor_membership_id: actor,
                        assignment_id,
                        confirm_no_admin: false,
                    },
                    t0,
                )
                .await
            })
        };
        let a = revoke(fau.admin_membership_id, other.assignment_ids[0]);
        let b = revoke(other.membership_id, fau.admin_assignment_id);
        let results = [a.await.unwrap(), b.await.unwrap()];

        let ok = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(
            ok, 1,
            "round {round}: exactly one revocation succeeds under serialisation"
        );
        for r in &results {
            if let Err(e) = r {
                assert!(
                    matches!(
                        e,
                        MembershipError::NotAuthorized | MembershipError::WouldLeaveNoAdmin
                    ),
                    "round {round}: unexpected {e:?}"
                );
            }
        }
        let admins_left: i64 = sqlx::query_scalar(
            "select count(*) from role_assignments ra join roles r on r.id = ra.role_id
              where ra.tenant_id = $1 and r.capability_class = 'admin' and ra.revoked_at is null",
        )
        .bind(fau.tenant_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(admins_left >= 1, "round {round}: no admin left");
    }
}
