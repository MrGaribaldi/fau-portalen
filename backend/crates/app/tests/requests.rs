//! Access requests and replacement proposals (flow spec §5.2–5.4).

mod common;
use common::membership::*;
use common::TestDb;
use fau_domain::membership::requests::{ReplacementDateError, RequestLimit};
use fau_domain::membership::vocabulary::CapabilityClass;
use fau_persistence::membership::{
    accept_invitation, approve_request, create_access_request, create_replacement_proposal,
    decline_request, lapse_requests, revoke_membership, AcceptInvitation, CreateAccessRequest,
    CreateReplacementProposal, MembershipError, OfferedRole, RequestDecision, RevokeMembership,
    RoleChoice,
};
use sqlx::PgPool;
use uuid::Uuid;

fn access(fau: &Fau, requester: &str) -> CreateAccessRequest {
    CreateAccessRequest {
        tenant_id: fau.tenant_id,
        requester: verified(requester),
    }
}

async fn status(pool: &PgPool, request_id: Uuid) -> String {
    sqlx::query_scalar("select status from access_requests where id = $1")
        .bind(request_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn an_access_request_reaches_the_admins_and_names_nobody() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;

    let id = create_access_request(&pool, access(&fau, "ny@example.test"), t0)
        .await
        .unwrap();

    assert_eq!(status(&pool, id).await, "pending");
    assert_eq!(
        outbox_count(&pool, "request.received", "admin@example.test").await,
        1
    );
    let sealed_message: Option<Vec<u8>> =
        sqlx::query_scalar("select sealed_message from access_requests where id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        sealed_message.is_none(),
        "no message field until the key service exists: nothing is stored as plaintext \
         (ADR-003 decision 6)"
    );
    let in_audit: i64 = sqlx::query_scalar(
        "select count(*) from audit_events where params::text like '%ny@example.test%'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        in_audit, 0,
        "the requester's address never reaches audit parameters"
    );
    let params: String =
        sqlx::query_scalar("select params::text from outbox where template = 'request.received'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        !params.contains("ny@example.test"),
        "the requester's address never reaches the outbox: the admin is not told who asked"
    );
}

#[tokio::test]
async fn one_open_request_per_address_and_five_per_fau_per_day() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    // Both instants fall on the same UTC calendar date (23 September) but on either
    // side of Oslo midnight (22:00 UTC in September's daylight time): 21:30Z is still
    // 23 September in Oslo, 22:30Z is already 24 September there. The daily limit must
    // follow the Oslo date, not the UTC one.
    let t0 = at("2026-09-23T21:30:00Z");
    let fau = active_fau(&pool, "admin@example.test", t0).await;

    create_access_request(&pool, access(&fau, "a@example.test"), t0)
        .await
        .unwrap();
    assert_eq!(
        create_access_request(&pool, access(&fau, "a@example.test"), t0)
            .await
            .unwrap_err(),
        MembershipError::RequestLimit(RequestLimit::OpenRequestExists)
    );
    for n in 0..4 {
        create_access_request(&pool, access(&fau, &format!("b{n}@example.test")), t0)
            .await
            .unwrap();
    }
    assert_eq!(
        create_access_request(&pool, access(&fau, "c@example.test"), t0)
            .await
            .unwrap_err(),
        MembershipError::RequestLimit(RequestLimit::TenantDailyLimit)
    );
    let tomorrow = at("2026-09-23T22:30:00Z");
    create_access_request(&pool, access(&fau, "c@example.test"), tomorrow)
        .await
        .expect("the daily limit resets on the next Oslo date");
}

#[tokio::test]
async fn approving_issues_a_normal_invitation_linked_to_the_request() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let request_id = create_access_request(&pool, access(&fau, "ny@example.test"), t0)
        .await
        .unwrap();

    let issued = approve_request(
        &pool,
        RequestDecision {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            request_id,
        },
        vec![OfferedRole {
            role: new_role("Medlem", CapabilityClass::Member),
            period: period(day(2026, 9, 23), day(2027, 9, 1)),
        }],
        t0,
    )
    .await
    .unwrap();

    assert_eq!(status(&pool, request_id).await, "approved");
    let (mode, link): (String, Option<Uuid>) =
        sqlx::query_as("select mode, access_request_id from invitations where id = $1")
            .bind(issued.invitation_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((mode.as_str(), link), ("normal", Some(request_id)));
    accept_invitation(
        &pool,
        AcceptInvitation {
            token: issued.token.expose().to_owned(),
            acceptor: verified("ny@example.test"),
            admin_end_override: None,
        },
        t0,
    )
    .await
    .unwrap();
    assert_eq!(audit_count(&pool, "request.approved").await, 1);
}

#[tokio::test]
async fn declining_tells_the_requester_and_nothing_more() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let request_id = create_access_request(&pool, access(&fau, "ny@example.test"), t0)
        .await
        .unwrap();
    let decision = RequestDecision {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        request_id,
    };
    decline_request(&pool, decision.clone(), t0).await.unwrap();

    assert_eq!(status(&pool, request_id).await, "declined");
    assert_eq!(
        outbox_count(&pool, "request.declined", "ny@example.test").await,
        1
    );
    let params: String =
        sqlx::query_scalar("select params::text from outbox where template = 'request.declined'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!params.contains(&fau.admin_membership_id.to_string()));
    assert_eq!(
        decline_request(&pool, decision, t0).await.unwrap_err(),
        MembershipError::RequestNotPending
    );
}

#[tokio::test]
async fn only_an_admin_decides() {
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
    let request_id = create_access_request(&pool, access(&fau, "ny@example.test"), t0)
        .await
        .unwrap();
    let err = decline_request(
        &pool,
        RequestDecision {
            tenant_id: fau.tenant_id,
            actor_membership_id: member.membership_id,
            request_id,
        },
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::NotAuthorized);
}

#[tokio::test]
async fn an_unauthorised_actor_is_refused_even_for_an_unknown_request() {
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
    // The request id does not exist at all; an unauthorised actor still gets
    // `NotAuthorized`, never a hint that it doesn't exist.
    let err = decline_request(
        &pool,
        RequestDecision {
            tenant_id: fau.tenant_id,
            actor_membership_id: member.membership_id,
            request_id: Uuid::now_v7(),
        },
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::NotAuthorized);
}

#[tokio::test]
async fn only_current_admins_receive_the_request_received_notice() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    add_member(
        &pool,
        &fau,
        "medlem@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;

    create_access_request(&pool, access(&fau, "ny@example.test"), t0)
        .await
        .unwrap();

    assert_eq!(
        outbox_count(&pool, "request.received", "admin@example.test").await,
        1
    );
    assert_eq!(
        outbox_count(&pool, "request.received", "medlem@example.test").await,
        0,
        "a plain member is not an admin and is not told about the request"
    );
}

/// Builds a membership whose only admin-class role assignment has already ended, and a
/// still-valid handover grant sourced from it. Row-for-row against 0003's schema, since
/// no persistence function can create this shape: `issue_invitation` refuses a period
/// that has already ended, so the only way to reach it is to arrange it directly, the
/// way the standing conventions arrange a frozen tenant.
async fn handover_only_actor(pool: &PgPool, fau: &Fau, address: &str) -> Uuid {
    let account_id = Uuid::now_v7();
    sqlx::query("insert into accounts (id, email, verified_at) values ($1, $2, now())")
        .bind(account_id)
        .bind(address)
        .execute(pool)
        .await
        .unwrap();
    let membership_id = Uuid::now_v7();
    sqlx::query("insert into memberships (tenant_id, id, account_id) values ($1, $2, $3)")
        .bind(fau.tenant_id)
        .bind(membership_id)
        .bind(account_id)
        .execute(pool)
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
    .execute(pool)
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
    .execute(pool)
    .await
    .unwrap();
    membership_id
}

#[tokio::test]
async fn approving_requires_an_admin_role_today_not_merely_a_handover_grant() {
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
    let handover_only =
        handover_only_actor(&db.admin_pool(), &fau, "avtroppende@example.test").await;

    let roles = || {
        vec![OfferedRole {
            role: new_role("Medlem", CapabilityClass::Member),
            period: period(day(2026, 9, 23), day(2027, 9, 1)),
        }]
    };

    let request_id = create_access_request(&pool, access(&fau, "ny@example.test"), t0)
        .await
        .unwrap();
    assert_eq!(
        approve_request(
            &pool,
            RequestDecision {
                tenant_id: fau.tenant_id,
                actor_membership_id: member.membership_id,
                request_id,
            },
            roles(),
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized,
        "a plain member is not an admin"
    );
    assert_eq!(
        approve_request(
            &pool,
            RequestDecision {
                tenant_id: fau.tenant_id,
                actor_membership_id: handover_only,
                request_id,
            },
            roles(),
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized,
        "a handover grant authorizes issuing, but approving a request needs an admin role today"
    );
}

#[tokio::test]
async fn approving_a_request_that_is_already_decided_is_refused() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let decide = |request_id| RequestDecision {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        request_id,
    };
    let roles = || {
        vec![OfferedRole {
            role: new_role("Medlem", CapabilityClass::Member),
            period: period(day(2026, 9, 23), day(2027, 9, 1)),
        }]
    };

    let approved_id = create_access_request(&pool, access(&fau, "ny1@example.test"), t0)
        .await
        .unwrap();
    approve_request(&pool, decide(approved_id), roles(), t0)
        .await
        .unwrap();
    assert_eq!(
        approve_request(&pool, decide(approved_id), roles(), t0)
            .await
            .unwrap_err(),
        MembershipError::RequestNotPending,
        "approving an already-approved request is refused"
    );

    let declined_id = create_access_request(&pool, access(&fau, "ny2@example.test"), t0)
        .await
        .unwrap();
    decline_request(&pool, decide(declined_id), t0)
        .await
        .unwrap();
    assert_eq!(
        approve_request(&pool, decide(declined_id), roles(), t0)
            .await
            .unwrap_err(),
        MembershipError::RequestNotPending,
        "approving an already-declined request is refused"
    );
}

#[tokio::test]
async fn a_frozen_fau_refuses_approving_and_proposing() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let kasserer = add_member(
        &pool,
        &fau,
        "kasserer@example.test",
        new_role("Kasserer", CapabilityClass::Member),
        period(day(2026, 9, 1), day(2027, 9, 1)),
        t0,
    )
    .await;
    let request_id = create_access_request(&pool, access(&fau, "ny@example.test"), t0)
        .await
        .unwrap();
    sqlx::query("update tenants set frozen_at = now() where id = $1")
        .bind(fau.tenant_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();

    assert_eq!(
        approve_request(
            &pool,
            RequestDecision {
                tenant_id: fau.tenant_id,
                actor_membership_id: fau.admin_membership_id,
                request_id,
            },
            vec![OfferedRole {
                role: new_role("Medlem", CapabilityClass::Member),
                period: period(day(2026, 9, 23), day(2027, 9, 1)),
            }],
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::TenantFrozen
    );
    assert_eq!(
        create_replacement_proposal(
            &pool,
            CreateReplacementProposal {
                tenant_id: fau.tenant_id,
                proposer_membership_id: kasserer.membership_id,
                replaced_assignment_id: kasserer.assignment_ids[0],
                successor: email("etterfolger@example.test"),
                starts_on: day(2027, 9, 1),
                ends_on_exclusive: day(2028, 9, 1),
            },
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::TenantFrozen
    );

    // Final review M7, controller ruling: declining closes a pending item and grants
    // nothing, so a frozen FAU still allows it.
    decline_request(
        &pool,
        RequestDecision {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            request_id,
        },
        t0,
    )
    .await
    .expect("declining is allowed on a frozen FAU");
    assert_eq!(status(&pool, request_id).await, "declined");
}

#[tokio::test]
async fn an_unhandled_request_lapses_after_thirty_days() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let request_id = create_access_request(&pool, access(&fau, "ny@example.test"), t0)
        .await
        .unwrap();

    assert_eq!(
        lapse_requests(&pool, at("2026-10-22T10:00:00Z"))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        lapse_requests(&pool, at("2026-10-23T10:00:00Z"))
            .await
            .unwrap(),
        1
    );
    assert_eq!(status(&pool, request_id).await, "lapsed");
    assert_eq!(
        outbox_count(&pool, "request.lapsed", "ny@example.test").await,
        1
    );
    assert_eq!(audit_count(&pool, "request.lapsed").await, 1);
}

#[tokio::test]
async fn lapse_leaves_decided_requests_untouched_however_old() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let decide = |request_id| RequestDecision {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        request_id,
    };

    let approved_id = create_access_request(&pool, access(&fau, "ny1@example.test"), t0)
        .await
        .unwrap();
    approve_request(
        &pool,
        decide(approved_id),
        vec![OfferedRole {
            role: new_role("Medlem", CapabilityClass::Member),
            period: period(day(2026, 9, 23), day(2027, 9, 1)),
        }],
        t0,
    )
    .await
    .unwrap();
    let declined_id = create_access_request(&pool, access(&fau, "ny2@example.test"), t0)
        .await
        .unwrap();
    decline_request(&pool, decide(declined_id), t0)
        .await
        .unwrap();

    // Long past the 30-day lapse window; only ever-pending requests are eligible.
    let long_after = at("2026-12-31T10:00:00Z");
    assert_eq!(lapse_requests(&pool, long_after).await.unwrap(), 0);
    assert_eq!(status(&pool, approved_id).await, "approved");
    assert_eq!(status(&pool, declined_id).await, "declined");
}

#[tokio::test]
async fn a_member_proposes_a_successor_for_their_own_role() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let kasserer = add_member(
        &pool,
        &fau,
        "kasserer@example.test",
        new_role("Kasserer", CapabilityClass::Member),
        period(day(2026, 9, 1), day(2027, 9, 1)),
        t0,
    )
    .await;
    let proposal = |starts, ends| CreateReplacementProposal {
        tenant_id: fau.tenant_id,
        proposer_membership_id: kasserer.membership_id,
        replaced_assignment_id: kasserer.assignment_ids[0],
        successor: email("etterfolger@example.test"),
        starts_on: starts,
        ends_on_exclusive: ends,
    };

    assert_eq!(
        create_replacement_proposal(&pool, proposal(day(2026, 9, 22), day(2028, 9, 1)), t0)
            .await
            .unwrap_err(),
        MembershipError::ReplacementDates(ReplacementDateError::StartsInPast)
    );
    let request_id =
        create_replacement_proposal(&pool, proposal(day(2027, 9, 1), day(2028, 9, 1)), t0)
            .await
            .unwrap();
    assert_eq!(
        create_replacement_proposal(&pool, proposal(day(2027, 9, 1), day(2028, 9, 1)), t0)
            .await
            .unwrap_err(),
        MembershipError::RequestLimit(RequestLimit::OpenRequestExists),
        "a proposal counts against its proposer"
    );
    let still_held: bool =
        sqlx::query_scalar("select revoked_at is null from role_assignments where id = $1")
            .bind(kasserer.assignment_ids[0])
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(still_held, "proposing does not end the proposer's role");

    // The role is the proposer's own, so the successor holds the same position.
    let role_id: Uuid = sqlx::query_scalar("select role_id from role_assignments where id = $1")
        .bind(kasserer.assignment_ids[0])
        .fetch_one(&pool)
        .await
        .unwrap();
    let issued = approve_request(
        &pool,
        RequestDecision {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            request_id,
        },
        vec![OfferedRole {
            role: RoleChoice::Existing(role_id),
            period: period(day(2027, 9, 1), day(2028, 9, 1)),
        }],
        t0,
    )
    .await
    .unwrap();
    let recipient: String =
        sqlx::query_scalar("select recipient_email from invitations where id = $1")
            .bind(issued.invitation_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(recipient, "etterfolger@example.test");
}

#[tokio::test]
async fn a_member_may_propose_only_for_a_role_they_hold_today() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let future = add_member(
        &pool,
        &fau,
        "snart@example.test",
        new_role("Sekretær", CapabilityClass::Member),
        period(day(2026, 10, 1), day(2027, 10, 1)),
        t0,
    )
    .await;
    let other = add_member(
        &pool,
        &fau,
        "annen@example.test",
        new_role("Kasserer", CapabilityClass::Member),
        period(day(2026, 9, 1), day(2027, 9, 1)),
        t0,
    )
    .await;
    let proposal = |proposer: Uuid, assignment: Uuid| CreateReplacementProposal {
        tenant_id: fau.tenant_id,
        proposer_membership_id: proposer,
        replaced_assignment_id: assignment,
        successor: email("etterfolger@example.test"),
        starts_on: day(2027, 10, 1),
        ends_on_exclusive: day(2028, 10, 1),
    };

    assert_eq!(
        create_replacement_proposal(
            &pool,
            proposal(future.membership_id, future.assignment_ids[0]),
            t0
        )
        .await
        .unwrap_err(),
        MembershipError::RoleNotHeldToday
    );
    assert_eq!(
        create_replacement_proposal(
            &pool,
            proposal(future.membership_id, other.assignment_ids[0]),
            t0
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized
    );
}

/// Final review M2: the proposer's assignment is looked up within their own membership
/// only, so an unknown id and someone else's id are indistinguishable.
#[tokio::test]
async fn a_proposal_naming_an_unknown_or_foreign_assignment_reveals_nothing() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let me = add_member(
        &pool,
        &fau,
        "meg@example.test",
        new_role("Sekretær", CapabilityClass::Member),
        period(day(2026, 9, 1), day(2027, 10, 1)),
        t0,
    )
    .await;
    let other = add_member(
        &pool,
        &fau,
        "annen@example.test",
        new_role("Kasserer", CapabilityClass::Member),
        period(day(2026, 9, 1), day(2027, 9, 1)),
        t0,
    )
    .await;
    for assignment in [Uuid::now_v7(), other.assignment_ids[0]] {
        assert_eq!(
            create_replacement_proposal(
                &pool,
                CreateReplacementProposal {
                    tenant_id: fau.tenant_id,
                    proposer_membership_id: me.membership_id,
                    replaced_assignment_id: assignment,
                    successor: email("etterfolger@example.test"),
                    starts_on: day(2027, 10, 1),
                    ends_on_exclusive: day(2028, 10, 1),
                },
                t0
            )
            .await
            .unwrap_err(),
            MembershipError::NotAuthorized
        );
    }
}

/// Final review M9: revoking a membership withdraws the member's own pending proposal
/// and any pending access request from their address, with an audit entry for each;
/// other people's requests are untouched.
#[tokio::test]
async fn revoking_a_membership_withdraws_the_members_pending_requests() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;

    // A proposal by one member ...
    let proposer = add_member(
        &pool,
        &fau,
        "kasserer@example.test",
        new_role("Kasserer", CapabilityClass::Member),
        period(day(2026, 9, 1), day(2027, 9, 1)),
        t0,
    )
    .await;
    let proposal = create_replacement_proposal(
        &pool,
        CreateReplacementProposal {
            tenant_id: fau.tenant_id,
            proposer_membership_id: proposer.membership_id,
            replaced_assignment_id: proposer.assignment_ids[0],
            successor: email("etterfolger@example.test"),
            starts_on: day(2027, 9, 1),
            ends_on_exclusive: day(2028, 9, 1),
        },
        t0,
    )
    .await
    .unwrap();
    // ... an access request another member sent before they were invited in ...
    let early = create_access_request(&pool, access(&fau, "tidlig@example.test"), t0)
        .await
        .unwrap();
    let early_member = add_member(
        &pool,
        &fau,
        "tidlig@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 1), day(2027, 9, 1)),
        t0,
    )
    .await;
    // ... and an outsider's request, which must survive.
    let outsider = create_access_request(&pool, access(&fau, "utenfor@example.test"), t0)
        .await
        .unwrap();

    for membership_id in [proposer.membership_id, early_member.membership_id] {
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
        .unwrap();
    }

    for id in [proposal, early] {
        assert_eq!(status(&pool, id).await, "withdrawn");
        let (closed, decided): (bool, bool) = sqlx::query_as(
            "select closed_at is not null, decided_by is not null from access_requests where id = $1",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(closed && !decided);
    }
    assert_eq!(status(&pool, outsider).await, "pending");
    assert_eq!(audit_count(&pool, "request.withdrawn").await, 2);
    let subjects: Vec<Uuid> = sqlx::query_scalar(
        "select subject_id from audit_events where action = 'request.withdrawn' order by subject_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let mut expected = vec![proposal, early];
    expected.sort();
    assert_eq!(subjects, expected);
}

#[tokio::test]
async fn a_frozen_fau_takes_no_requests() {
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
        create_access_request(&pool, access(&fau, "ny@example.test"), t0)
            .await
            .unwrap_err(),
        MembershipError::TenantFrozen
    );
}

#[tokio::test]
async fn request_structs_never_print_addresses_in_debug() {
    let request = CreateAccessRequest {
        tenant_id: Uuid::now_v7(),
        requester: verified("hemmelig@example.test"),
    };
    let debug = format!("{request:?}");
    assert!(!debug.contains("hemmelig@example.test"));

    let proposal = CreateReplacementProposal {
        tenant_id: Uuid::now_v7(),
        proposer_membership_id: Uuid::now_v7(),
        replaced_assignment_id: Uuid::now_v7(),
        successor: email("hemmelig-etterfolger@example.test"),
        starts_on: day(2027, 9, 1),
        ends_on_exclusive: day(2028, 9, 1),
    };
    let debug = format!("{proposal:?}");
    assert!(!debug.contains("hemmelig-etterfolger@example.test"));
}

/// Coverage gap (audit section 5): `UnknownRequest`, reached only once the actor's
/// admin authority is already established -- a three-line lookup a random
/// `Uuid::now_v7()` cannot match, on both decision paths.
#[tokio::test]
async fn an_admin_deciding_an_unknown_request_gets_unknown_request() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;

    let decision = |request_id| RequestDecision {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        request_id,
    };

    let err = approve_request(&pool, decision(Uuid::now_v7()), vec![], t0)
        .await
        .unwrap_err();
    assert_eq!(err, MembershipError::UnknownRequest, "approve_request");

    let err = decline_request(&pool, decision(Uuid::now_v7()), t0)
        .await
        .unwrap_err();
    assert_eq!(err, MembershipError::UnknownRequest, "decline_request");
}
