//! The per-request access check (flow spec §2.1, §4; §10 "Access").

mod common;
use common::membership::*;
use common::TestDb;
use fau_domain::membership::access::{Access, Capability};
use fau_domain::membership::vocabulary::CapabilityClass;
use fau_persistence::membership::{
    create_handover_grants, effective_access, revoke_role_assignment, RevokeAssignment,
};
use uuid::Uuid;

#[tokio::test]
async fn the_registrant_is_an_admin_until_the_chosen_end_date() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;

    let during = effective_access(&pool, fau.admin_account_id, fau.tenant_id, at(T0))
        .await
        .unwrap();
    assert_eq!(during.capability, Capability::Admin);
    let last_day = at("2027-09-30T21:59:59Z");
    assert_eq!(
        effective_access(&pool, fau.admin_account_id, fau.tenant_id, last_day)
            .await
            .unwrap()
            .capability,
        Capability::Admin
    );
    let midnight = at("2027-09-30T22:00:00Z");
    assert_eq!(
        effective_access(&pool, fau.admin_account_id, fau.tenant_id, midnight)
            .await
            .unwrap(),
        Access::NONE,
        "access ends at Oslo midnight whether or not any job has run"
    );
}

#[tokio::test]
async fn a_role_valid_tomorrow_grants_nothing_today() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let member = add_member(
        &pool,
        &fau,
        "snart@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 24), day(2027, 9, 1)),
        t0,
    )
    .await;

    let today = effective_access(&pool, member.account_id, fau.tenant_id, t0)
        .await
        .unwrap();
    assert_eq!(today.capability, Capability::None);
    assert_eq!(today.next_start, Some(day(2026, 9, 24)));
    let tomorrow = effective_access(
        &pool,
        member.account_id,
        fau.tenant_id,
        at("2026-09-24T10:00:00Z"),
    )
    .await
    .unwrap();
    assert_eq!(tomorrow.capability, Capability::Member);
}

#[tokio::test]
async fn losing_the_last_valid_role_removes_access_on_the_next_check() {
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
    assert_eq!(
        effective_access(&pool, member.account_id, fau.tenant_id, t0)
            .await
            .unwrap()
            .capability,
        Capability::Member
    );

    revoke_role_assignment(
        &pool,
        RevokeAssignment {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            assignment_id: member.assignment_ids[0],
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        effective_access(&pool, member.account_id, fau.tenant_id, t0)
            .await
            .unwrap(),
        Access::NONE
    );
}

#[tokio::test]
async fn a_handover_grant_alone_is_reported_without_capability() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    let inside = at("2027-10-01T10:00:00Z");
    create_handover_grants(&pool, inside).await.unwrap();

    let access = effective_access(&pool, fau.admin_account_id, fau.tenant_id, inside)
        .await
        .unwrap();
    assert_eq!(
        access.capability,
        Capability::None,
        "no document access (spec 6.3)"
    );
    assert!(access.handover);
}

#[tokio::test]
async fn nothing_is_revealed_about_other_faus_or_unknown_ids() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let a = active_fau(&pool, "a@example.test", t0).await;
    let b = active_fau(&pool, "b@example.test", t0).await;

    for (account, tenant) in [
        (a.admin_account_id, b.tenant_id),
        (Uuid::now_v7(), a.tenant_id),
        (a.admin_account_id, Uuid::now_v7()),
    ] {
        assert_eq!(
            effective_access(&pool, account, tenant, t0).await.unwrap(),
            Access::NONE
        );
    }
}

#[tokio::test]
async fn a_disabled_account_has_no_access_and_a_frozen_fau_keeps_it() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;

    sqlx::query("update tenants set frozen_at = now() where id = $1")
        .bind(fau.tenant_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();
    assert_eq!(
        effective_access(&pool, fau.admin_account_id, fau.tenant_id, t0)
            .await
            .unwrap()
            .capability,
        Capability::Admin,
        "reads continue during a freeze (ADR-003 decision 7a)"
    );

    sqlx::query("update accounts set disabled_at = now() where id = $1")
        .bind(fau.admin_account_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();
    assert_eq!(
        effective_access(&pool, fau.admin_account_id, fau.tenant_id, t0)
            .await
            .unwrap(),
        Access::NONE
    );
}

// -- Fix round 1 regression tests --------------------------------------------------

/// A tenant that is not active gives no access, whatever state it is in -- `closed` or
/// `pending`, not just the frozen case already covered above. Arranged directly via
/// `admin_pool`, as `handover_recovery.rs`'s `the_sweep_skips_non_active_tenants` does:
/// no persistence function closes a tenant or un-activates one.
#[tokio::test]
async fn a_non_active_tenant_gives_no_access() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;

    sqlx::query("update tenants set status = 'closed' where id = $1")
        .bind(fau.tenant_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();
    assert_eq!(
        effective_access(&pool, fau.admin_account_id, fau.tenant_id, t0)
            .await
            .unwrap(),
        Access::NONE,
        "a closed tenant grants nothing to its former admin"
    );

    sqlx::query("update tenants set status = 'pending' where id = $1")
        .bind(fau.tenant_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();
    assert_eq!(
        effective_access(&pool, fau.admin_account_id, fau.tenant_id, t0)
            .await
            .unwrap(),
        Access::NONE,
        "a pending tenant grants nothing either"
    );
}

/// Defence in depth (fix round 1, task 10 review): a handover grant whose source role
/// assignment has since been revoked must report no handover, even when the grant row
/// itself was never revoked and the revocation did not go through
/// `revoke_role_assignment`'s normal cascade. Manufactured directly via `admin_pool`,
/// the same way `handover_recovery.rs`'s
/// `a_grant_whose_source_is_revoked_confers_no_authority` arranges it, except here the
/// grant is created first through the real sweep and the assignment is revoked
/// afterwards, so the grant row is left with `revoked_at` still null.
#[tokio::test]
async fn a_grant_whose_source_assignment_is_revoked_without_cascade_reports_no_handover() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let inside = at("2027-10-01T10:00:00Z");
    assert_eq!(create_handover_grants(&pool, inside).await.unwrap(), 1);

    // The source assignment is revoked with a bare update, not through
    // `revoke_role_assignment`, so the grant's own `revoked_at` stays null -- exactly
    // the state the SQL filter, not the domain layer, must catch.
    sqlx::query("update role_assignments set revoked_at = now() where tenant_id = $1 and id = $2")
        .bind(fau.tenant_id)
        .bind(fau.admin_assignment_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();
    let grant_revoked: bool = sqlx::query_scalar(
        "select revoked_at is not null from handover_grants where tenant_id = $1 and source_assignment_id = $2",
    )
    .bind(fau.tenant_id)
    .bind(fau.admin_assignment_id)
    .fetch_one(&db.admin_pool())
    .await
    .unwrap();
    assert!(
        !grant_revoked,
        "the grant row itself must still be unrevoked for this test to prove anything"
    );

    let access = effective_access(&pool, fau.admin_account_id, fau.tenant_id, inside)
        .await
        .unwrap();
    assert!(
        !access.handover,
        "a grant sourced from a revoked assignment must not be reported as a live handover"
    );
}
