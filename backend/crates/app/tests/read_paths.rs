//! The #3418 read-path audit (groups design §3.3, last bullet; #3501). The membership
//! foundation predates guests, so every path that read "any valid role" as "a member" was
//! checked. These tests pin the places where a guest would otherwise have counted as an
//! FAU-wide member.
//!
//! Controller ruling P4: routing `access_request_message` through `authorize` also changes
//! behaviour for admins, not only guests. `authorize` reads standing through
//! `membership_access`, which requires `tenants.status = 'active'`; `require_admin` did not
//! check tenant status at all. So an admin of a `closed` or `pending` FAU, who previously
//! got the message, now gets `NotAuthorized` like anyone else with no current standing. A frozen FAU stays `active`
//! with `frozen_at` set, so an admin's read continues through a freeze -- only `status`
//! changes the outcome.

mod common;
use common::groups::*;
use common::membership::*;
use common::TestDb;
use fau_domain::membership::access::Capability;
use fau_domain::membership::vocabulary::{CapabilityClass, Visibility};
use fau_persistence::membership::{
    access_request_message, create_access_request, effective_access, recovery_grant_admin,
    revoke_membership, AccessRequestMessage, CreateAccessRequest, MembershipError, RecoveryActor,
    RecoveryGrant, RevokeMembership, RoleChoice,
};
use sqlx::PgPool;
use uuid::Uuid;

fn ct(s: &str) -> fau_crypto::MessageCiphertext {
    fau_crypto::MessageCiphertext::new(format!("vault:v1:{s}")).unwrap()
}

async fn admin_leaves(pool: &PgPool, fau: &Fau) {
    revoke_membership(
        pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: fau.admin_membership_id,
            confirm_no_admin: true,
        },
        at(T0),
    )
    .await
    .unwrap();
}

async fn recover(pool: &PgPool, fau: &Fau) {
    recovery_grant_admin(
        pool,
        RecoveryGrant {
            tenant_id: fau.tenant_id,
            actor: RecoveryActor::Ewb,
            recipient: email("ny-leder@example.test"),
            role: RoleChoice::Existing(fau.admin_role_id),
            period: period(day(2026, 9, 23), day(2027, 10, 1)),
        },
        at(T0),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_guest_is_a_guest_and_cannot_read_an_access_request_message() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let group = seed_group(&pool, &fau, Visibility::Open).await;
    let guest = add_guest(
        &pool,
        &fau,
        "gjest@example.test",
        group,
        period(day(2026, 9, 1), day(2027, 9, 1)),
        t0,
    )
    .await;
    assert_eq!(
        effective_access(&pool, guest.account_id, fau.tenant_id, t0)
            .await
            .unwrap()
            .capability,
        Capability::Guest
    );

    let request_id = Uuid::now_v7();
    create_access_request(
        &pool,
        CreateAccessRequest {
            tenant_id: fau.tenant_id,
            requester: verified("ny@example.test"),
            message: Some(AccessRequestMessage {
                request_id,
                ciphertext: ct("c2VhbGVk"),
            }),
        },
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        access_request_message(
            &pool,
            fau.tenant_id,
            request_id,
            fau.admin_membership_id,
            t0
        )
        .await
        .unwrap(),
        Some(ct("c2VhbGVk"))
    );
    for request in [request_id, Uuid::now_v7()] {
        assert_eq!(
            access_request_message(&pool, fau.tenant_id, request, guest.membership_id, t0)
                .await
                .unwrap_err(),
            MembershipError::NotAuthorized,
            "authority before the row, and a guest has none"
        );
    }
}

#[tokio::test]
async fn guests_are_not_told_about_a_recovery() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let year = period(day(2026, 9, 23), day(2027, 9, 1));
    add_member(
        &pool,
        &fau,
        "medlem@example.test",
        new_role("Medlem", CapabilityClass::Member),
        year,
        t0,
    )
    .await;
    let group = seed_group(&pool, &fau, Visibility::Open).await;
    add_guest(&pool, &fau, "gjest@example.test", group, year, t0).await;
    admin_leaves(&pool, &fau).await;

    recover(&pool, &fau).await;
    assert_eq!(
        outbox_count(&pool, "recovery.invitation_created", "medlem@example.test").await,
        1
    );
    assert_eq!(
        outbox_count(&pool, "recovery.invitation_created", "gjest@example.test").await,
        0
    );
}

#[tokio::test]
async fn with_no_members_left_the_fallback_skips_guests_too() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let group = seed_group(&pool, &fau, Visibility::Open).await;
    add_guest(
        &pool,
        &fau,
        "gjest@example.test",
        group,
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    admin_leaves(&pool, &fau).await;

    recover(&pool, &fau).await;
    // The guest is no FAU-wide member, so nobody current remains: the 24-month fallback
    // reaches the admin who left, and still not the guest.
    assert_eq!(
        outbox_count(&pool, "recovery.invitation_created", "admin@example.test").await,
        1
    );
    assert_eq!(
        outbox_count(&pool, "recovery.invitation_created", "gjest@example.test").await,
        0
    );
}

/// Controller ruling P4: `access_request_message` now runs `authorize`, which requires
/// `tenants.status = 'active'` (through `membership_access`). `require_admin` never checked
/// tenant status, so this is a real behaviour change for admins, arranged directly on
/// `admin_pool` the way `access.rs`'s `a_non_active_tenant_gives_no_access` does -- no
/// persistence function closes or un-activates a tenant.
#[tokio::test]
async fn an_admin_of_a_non_active_fau_is_refused_the_message_but_a_frozen_one_still_reads() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;

    let request_id = Uuid::now_v7();
    create_access_request(
        &pool,
        CreateAccessRequest {
            tenant_id: fau.tenant_id,
            requester: verified("ny@example.test"),
            message: Some(AccessRequestMessage {
                request_id,
                ciphertext: ct("c2VhbGVk"),
            }),
        },
        t0,
    )
    .await
    .unwrap();

    // Frozen but still `active`: reads continue (ADR-003 decision 7a), so the admin still
    // reads the message.
    sqlx::query("update tenants set frozen_at = now() where id = $1")
        .bind(fau.tenant_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();
    assert_eq!(
        access_request_message(
            &pool,
            fau.tenant_id,
            request_id,
            fau.admin_membership_id,
            t0
        )
        .await
        .unwrap(),
        Some(ct("c2VhbGVk")),
        "a freeze stops writes, not reads"
    );
    sqlx::query("update tenants set frozen_at = null where id = $1")
        .bind(fau.tenant_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();

    // `closed`: no current standing at all, so the admin is refused like anyone else.
    sqlx::query("update tenants set status = 'closed' where id = $1")
        .bind(fau.tenant_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();
    assert_eq!(
        access_request_message(
            &pool,
            fau.tenant_id,
            request_id,
            fau.admin_membership_id,
            t0
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized,
        "a closed FAU's former admin has no standing to read the message"
    );

    // `pending`: same outcome, cheap to arrange on the same tenant row -- `membership_access`
    // treats every non-active status alike.
    sqlx::query("update tenants set status = 'pending' where id = $1")
        .bind(fau.tenant_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();
    assert_eq!(
        access_request_message(
            &pool,
            fau.tenant_id,
            request_id,
            fau.admin_membership_id,
            t0
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized,
        "a pending FAU's admin has no standing yet either"
    );
}
