//! The outbox's tenant scoping (final review, controller ruling: `outbox.tenant_id`).
//! Every message about an FAU carries that FAU's id in its own column, so deleting an
//! FAU (ADR-003 decision 7) and an Article 17 erasure can find its queued mail without
//! parsing `params`. The only global template is the signup collision copy to EWB,
//! which -- like its audit entry -- belongs to no tenant.

mod common;
use std::collections::BTreeSet;

use common::membership::*;
use common::TestDb;
use fau_domain::membership::vocabulary::CapabilityClass;
use fau_persistence::membership::{
    accept_invitation, create_access_request, create_pending_tenant, decline_request,
    lapse_requests, recovery_grant_admin, revoke_membership, AcceptInvitation, CreateAccessRequest,
    MembershipError, RecoveryActor, RecoveryGrant, RequestDecision, RevokeMembership, RoleChoice,
};
use uuid::Uuid;

/// Every template the membership foundation writes, and whether it is tenant-scoped.
const TEMPLATES: &[(&str, bool)] = &[
    ("tenant.activated", true),
    ("invitation.issued", true),
    ("request.received", true),
    ("request.declined", true),
    ("request.lapsed", true),
    ("recovery.invitation_created", true),
    ("recovery.invitation_accepted", true),
    ("signup.collision", false),
];

#[tokio::test]
async fn every_tenant_scoped_template_writes_its_tenant_id() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);

    // tenant.activated (and invitation.issued, via add_member).
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let taken_school: Option<Uuid> =
        sqlx::query_scalar("select school_id from tenants where id = $1")
            .bind(fau.tenant_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    // signup.collision.
    assert!(matches!(
        create_pending_tenant(
            &pool,
            signup(
                taken_school.unwrap(),
                "annen@example.test",
                "annen@example.test",
                t0
            ),
            t0
        )
        .await,
        Err(MembershipError::SchoolTaken(_))
    ));
    add_member(
        &pool,
        &fau,
        "medlem@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;

    // request.received and request.declined.
    let request = |address: &str| CreateAccessRequest {
        tenant_id: fau.tenant_id,
        requester: verified(address),
        message: None,
    };
    let declined = create_access_request(&pool, request("nei@example.test"), t0)
        .await
        .unwrap();
    decline_request(
        &pool,
        RequestDecision {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            request_id: declined,
        },
        t0,
    )
    .await
    .unwrap();
    // request.lapsed.
    create_access_request(&pool, request("glemt@example.test"), t0)
        .await
        .unwrap();
    assert_eq!(
        lapse_requests(&pool, at("2026-10-23T10:00:00Z"))
            .await
            .unwrap(),
        1
    );

    // recovery.invitation_created and recovery.invitation_accepted.
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: fau.admin_membership_id,
            confirm_no_admin: true,
        },
        t0,
    )
    .await
    .unwrap();
    let issued = recovery_grant_admin(
        &pool,
        RecoveryGrant {
            tenant_id: fau.tenant_id,
            actor: RecoveryActor::Ewb,
            recipient: email("ny-leder@example.test"),
            role: RoleChoice::Existing(fau.admin_role_id),
            period: period(day(2026, 9, 23), day(2027, 10, 1)),
        },
        t0,
    )
    .await
    .unwrap();
    accept_invitation(
        &pool,
        AcceptInvitation {
            token: issued.token.expose().to_owned(),
            acceptor: verified("ny-leder@example.test"),
            admin_end_override: None,
        },
        t0,
    )
    .await
    .unwrap();

    let rows: Vec<(String, Option<Uuid>, Option<String>)> =
        sqlx::query_as("select template, tenant_id, params->>'tenant_id' from outbox")
            .fetch_all(&pool)
            .await
            .unwrap();
    let seen: BTreeSet<&str> = rows.iter().map(|(t, _, _)| t.as_str()).collect();
    let expected: BTreeSet<&str> = TEMPLATES.iter().map(|(t, _)| *t).collect();
    assert_eq!(seen, expected, "the scenario reaches every template");
    for (template, tenant_id, param) in &rows {
        let scoped = TEMPLATES.iter().any(|(t, s)| t == template && *s);
        if scoped {
            assert_eq!(*tenant_id, Some(fau.tenant_id), "{template}");
            assert_eq!(
                param.as_deref(),
                Some(fau.tenant_id.to_string().as_str()),
                "{template}: params agree with the column"
            );
        } else {
            assert_eq!(*tenant_id, None, "{template} is global");
        }
    }
}

/// The schema backs the same rule: a tenant-scoped template cannot be queued without
/// its tenant, and the global collision copy can.
#[tokio::test]
async fn the_schema_refuses_a_tenant_scoped_message_without_a_tenant() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let insert = |template: &'static str| {
        sqlx::query(
            "insert into outbox (id, template, recipient_email, created_at)
             values ($1, $2, 'x@example.test', now())",
        )
        .bind(Uuid::now_v7())
        .bind(template)
    };
    for (template, scoped) in TEMPLATES {
        let result = insert(template).execute(&pool).await;
        if *scoped {
            let err = result.expect_err(template);
            assert_eq!(
                err.as_database_error().and_then(|e| e.constraint()),
                Some("outbox_tenant_scoped_unless_global"),
                "{template}"
            );
        } else {
            result.expect(template);
        }
    }
}
