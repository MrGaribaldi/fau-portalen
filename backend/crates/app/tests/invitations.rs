//! Invitations: issue, re-send, withdraw, accept (flow spec §5.1, §3.5; §10
//! "Invitations and requests").

mod common;
use common::membership::*;
use common::TestDb;
use fau_domain::membership::acceptance::AcceptanceRefusal;
use fau_domain::membership::rules::AdminEndError;
use fau_domain::membership::vocabulary::CapabilityClass;
use fau_persistence::membership::{
    accept_invitation, activate_tenant, create_pending_tenant, issue_invitation, resend_invitation,
    withdraw_invitation, AcceptInvitation, Activation, InvitationChange, IssueInvitation,
    IssuedInvitation, MembershipError, OfferedRole, RoleChoice,
};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

async fn invite(
    pool: &PgPool,
    fau: &Fau,
    actor: Uuid,
    recipient: &str,
    role: RoleChoice,
    t: fau_domain::time::Moment,
) -> Result<IssuedInvitation, MembershipError> {
    issue_invitation(
        pool,
        IssueInvitation {
            tenant_id: fau.tenant_id,
            actor_membership_id: actor,
            recipient: email(recipient),
            roles: vec![OfferedRole {
                role,
                period: period(day(2026, 9, 23), day(2027, 9, 1)),
            }],
            handover_grant_id: None,
        },
        t,
    )
    .await
}

fn accept(token: &str, acceptor: &str) -> AcceptInvitation {
    AcceptInvitation {
        token: token.to_owned(),
        acceptor: verified(acceptor),
        admin_end_override: None,
    }
}

#[tokio::test]
async fn an_admin_invites_and_the_recipient_accepts() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;

    let issued = invite(
        &pool,
        &fau,
        fau.admin_membership_id,
        "ny@example.test",
        new_role("Kasserer", CapabilityClass::Member),
        t0,
    )
    .await
    .unwrap();
    assert_eq!(issued.expires_at, "2026-10-07T10:00:00Z".parse().unwrap());

    // Only the hash is stored, and neither the outbox nor the audit log holds the token.
    let digest = Sha256::digest(issued.token.expose().as_bytes()).to_vec();
    assert_eq!(count(&pool, "select count(*) from invitations").await, 1);
    let stored: Vec<u8> = sqlx::query_scalar("select token_hash from invitations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, digest);
    let leaked: i64 = sqlx::query_scalar(
        "select (select count(*) from outbox where params::text like '%' || $1 || '%')
              + (select count(*) from audit_events where params::text like '%' || $1 || '%')",
    )
    .bind(issued.token.expose())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(leaked, 0);
    assert_eq!(
        outbox_count(&pool, "invitation.issued", "ny@example.test").await,
        1
    );
    assert_eq!(
        count(&pool, "select count(*) from memberships").await,
        1,
        "issuing grants nothing until the recipient accepts"
    );

    let accepted = accept_invitation(&pool, accept(issued.token.expose(), "ny@example.test"), t0)
        .await
        .unwrap();
    assert_eq!(accepted.tenant_id, fau.tenant_id);
    assert_eq!(accepted.assignment_ids.len(), 1);
    let granted_by: Option<Uuid> =
        sqlx::query_scalar("select granted_by from role_assignments where id = $1")
            .bind(accepted.assignment_ids[0])
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(granted_by, Some(fau.admin_membership_id));
    assert_eq!(audit_count(&pool, "invitation.accepted").await, 1);
}

#[tokio::test]
async fn only_an_admin_valid_today_may_invite() {
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

    let err = invite(
        &pool,
        &fau,
        member.membership_id,
        "ny@example.test",
        new_role("Medlem", CapabilityClass::Member),
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::NotAuthorized);
}

#[tokio::test]
async fn acceptance_refuses_an_expired_used_or_withdrawn_token() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let role = || new_role("Medlem", CapabilityClass::Member);

    let expired = invite(
        &pool,
        &fau,
        fau.admin_membership_id,
        "a@example.test",
        role(),
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        accept_invitation(
            &pool,
            accept(expired.token.expose(), "a@example.test"),
            at("2026-10-07T10:00:00Z")
        )
        .await
        .unwrap_err(),
        MembershipError::Acceptance(AcceptanceRefusal::Expired)
    );

    let used = invite(
        &pool,
        &fau,
        fau.admin_membership_id,
        "b@example.test",
        role(),
        t0,
    )
    .await
    .unwrap();
    accept_invitation(&pool, accept(used.token.expose(), "b@example.test"), t0)
        .await
        .unwrap();
    assert_eq!(
        accept_invitation(&pool, accept(used.token.expose(), "b@example.test"), t0)
            .await
            .unwrap_err(),
        MembershipError::Acceptance(AcceptanceRefusal::AlreadyAccepted)
    );

    let withdrawn = invite(
        &pool,
        &fau,
        fau.admin_membership_id,
        "c@example.test",
        role(),
        t0,
    )
    .await
    .unwrap();
    withdraw_invitation(
        &pool,
        InvitationChange {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            invitation_id: withdrawn.invitation_id,
        },
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        accept_invitation(
            &pool,
            accept(withdrawn.token.expose(), "c@example.test"),
            t0
        )
        .await
        .unwrap_err(),
        MembershipError::Acceptance(AcceptanceRefusal::Revoked)
    );
    assert_eq!(
        accept_invitation(&pool, accept(&"0".repeat(64), "c@example.test"), t0)
            .await
            .unwrap_err(),
        MembershipError::UnknownInvitation
    );
    assert_eq!(
        accept_invitation(&pool, accept("not-a-token", "c@example.test"), t0)
            .await
            .unwrap_err(),
        MembershipError::UnknownInvitation
    );
}

#[tokio::test]
async fn acceptance_refuses_a_different_address() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let issued = invite(
        &pool,
        &fau,
        fau.admin_membership_id,
        "ny@example.test",
        new_role("Medlem", CapabilityClass::Member),
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        accept_invitation(
            &pool,
            accept(issued.token.expose(), "annen@example.test"),
            t0
        )
        .await
        .unwrap_err(),
        MembershipError::Acceptance(AcceptanceRefusal::EmailMismatch)
    );
}

#[tokio::test]
async fn acceptance_refuses_a_frozen_or_closed_fau() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let issued = invite(
        &pool,
        &fau,
        fau.admin_membership_id,
        "ny@example.test",
        new_role("Medlem", CapabilityClass::Member),
        t0,
    )
    .await
    .unwrap();

    sqlx::query("update tenants set frozen_at = now() where id = $1")
        .bind(fau.tenant_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();
    assert_eq!(
        accept_invitation(&pool, accept(issued.token.expose(), "ny@example.test"), t0)
            .await
            .unwrap_err(),
        MembershipError::Acceptance(AcceptanceRefusal::TenantFrozen)
    );
    assert_eq!(
        invite(
            &pool,
            &fau,
            fau.admin_membership_id,
            "to@example.test",
            new_role("Medlem", CapabilityClass::Member),
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::TenantFrozen,
        "a frozen FAU issues no invitations (ADR-003 decision 7a)"
    );

    sqlx::query("update tenants set frozen_at = null, status = 'closed' where id = $1")
        .bind(fau.tenant_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();
    assert_eq!(
        accept_invitation(&pool, accept(issued.token.expose(), "ny@example.test"), t0)
            .await
            .unwrap_err(),
        MembershipError::Acceptance(AcceptanceRefusal::TenantNotActive)
    );
}

#[tokio::test]
async fn acceptance_refuses_when_the_issuer_has_lost_authority() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let issued = invite(
        &pool,
        &fau,
        fau.admin_membership_id,
        "ny@example.test",
        new_role("Medlem", CapabilityClass::Member),
        t0,
    )
    .await
    .unwrap();

    sqlx::query("update role_assignments set revoked_at = now() where id = $1")
        .bind(fau.admin_assignment_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();
    assert_eq!(
        accept_invitation(&pool, accept(issued.token.expose(), "ny@example.test"), t0)
            .await
            .unwrap_err(),
        MembershipError::Acceptance(AcceptanceRefusal::IssuerLacksAuthority)
    );
}

#[tokio::test]
async fn resending_replaces_the_token_and_restarts_the_fourteen_days() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let first = invite(
        &pool,
        &fau,
        fau.admin_membership_id,
        "ny@example.test",
        new_role("Medlem", CapabilityClass::Member),
        t0,
    )
    .await
    .unwrap();

    let t10 = at("2026-10-03T10:00:00Z");
    let second = resend_invitation(
        &pool,
        InvitationChange {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            invitation_id: first.invitation_id,
        },
        t10,
    )
    .await
    .unwrap();
    assert_eq!(second.invitation_id, first.invitation_id);
    assert_eq!(second.expires_at, "2026-10-17T10:00:00Z".parse().unwrap());
    assert_eq!(
        outbox_count(&pool, "invitation.issued", "ny@example.test").await,
        2
    );

    let later = at("2026-10-10T10:00:00Z");
    assert_eq!(
        accept_invitation(
            &pool,
            accept(first.token.expose(), "ny@example.test"),
            later
        )
        .await
        .unwrap_err(),
        MembershipError::UnknownInvitation,
        "the old token stops working"
    );
    accept_invitation(
        &pool,
        accept(second.token.expose(), "ny@example.test"),
        later,
    )
    .await
    .expect("the new token works past the old expiry");
}

#[tokio::test]
async fn the_issuer_or_any_admin_may_withdraw_and_nobody_else() {
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
    let issued = invite(
        &pool,
        &fau,
        fau.admin_membership_id,
        "ny@example.test",
        new_role("Medlem", CapabilityClass::Member),
        t0,
    )
    .await
    .unwrap();

    let change = |actor| InvitationChange {
        tenant_id: fau.tenant_id,
        actor_membership_id: actor,
        invitation_id: issued.invitation_id,
    };
    assert_eq!(
        withdraw_invitation(&pool, change(member.membership_id), t0)
            .await
            .unwrap_err(),
        MembershipError::NotAuthorized
    );
    withdraw_invitation(&pool, change(fau.admin_membership_id), t0)
        .await
        .unwrap();
    assert_eq!(
        withdraw_invitation(&pool, change(fau.admin_membership_id), t0)
            .await
            .unwrap_err(),
        MembershipError::InvitationNotPending
    );
    assert_eq!(audit_count(&pool, "invitation.withdrawn").await, 1);
}

#[tokio::test]
async fn the_leader_accepts_and_may_adjust_the_end_date_within_range() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let pending = create_pending_tenant(
        &pool,
        signup(
            school(&pool, "school-1").await,
            "reg@example.test",
            "leder@example.test",
            t0,
        ),
        t0,
    )
    .await
    .unwrap();
    let activated = activate_tenant(
        &pool,
        Activation {
            tenant_id: pending.tenant_id,
            registrant: verified("reg@example.test"),
        },
        t0,
    )
    .await
    .unwrap();
    let token = activated.leader_invitation.unwrap().token;
    let with_end = |end| AcceptInvitation {
        token: token.expose().to_owned(),
        acceptor: verified("leder@example.test"),
        admin_end_override: Some(end),
    };

    let t2 = at("2026-09-25T10:00:00Z");
    assert_eq!(
        accept_invitation(&pool, with_end(day(2028, 9, 26)), t2)
            .await
            .unwrap_err(),
        MembershipError::AdminEnd(AdminEndError::TooLate)
    );
    let accepted = accept_invitation(&pool, with_end(day(2028, 6, 1)), t2)
        .await
        .unwrap();
    let (starts, ends, granted_by_null): (String, String, bool) = sqlx::query_as(
        "select starts_on::text, ends_on_exclusive::text, granted_by is null
           from role_assignments where id = $1",
    )
    .bind(accepted.assignment_ids[0])
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (starts.as_str(), ends.as_str(), granted_by_null),
        ("2026-09-23", "2028-06-01", true)
    );
}

#[tokio::test]
async fn only_an_activation_invitation_accepts_an_end_date_change() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let issued = invite(
        &pool,
        &fau,
        fau.admin_membership_id,
        "ny@example.test",
        RoleChoice::Existing(fau.admin_role_id),
        t0,
    )
    .await
    .unwrap();
    let err = accept_invitation(
        &pool,
        AcceptInvitation {
            token: issued.token.expose().to_owned(),
            acceptor: verified("ny@example.test"),
            admin_end_override: Some(day(2027, 12, 1)),
        },
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::OverrideNotAllowed);
}

#[tokio::test]
async fn an_account_from_another_fau_is_reused() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let first = active_fau(&pool, "leder@example.test", t0).await;
    let second = active_fau(&pool, "admin@example.test", t0).await;
    let accepted = add_member(
        &pool,
        &second,
        "leder@example.test",
        RoleChoice::Existing(second.admin_role_id),
        period(day(2026, 9, 23), day(2027, 10, 1)),
        t0,
    )
    .await;
    assert_eq!(accepted.account_id, first.admin_account_id);
    assert_eq!(count(&pool, "select count(*) from accounts").await, 2);
}

#[tokio::test]
async fn a_revoked_member_who_is_invited_again_gets_the_same_membership_back() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let first = add_member(
        &pool,
        &fau,
        "tilbake@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    sqlx::query("update memberships set revoked_at = now() where id = $1")
        .bind(first.membership_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();

    let again = add_member(
        &pool,
        &fau,
        "tilbake@example.test",
        new_role("Sekretær", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    assert_eq!(again.membership_id, first.membership_id);
    let revoked: bool =
        sqlx::query_scalar("select revoked_at is not null from memberships where id = $1")
            .bind(again.membership_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!revoked);
}

#[tokio::test]
async fn offered_roles_are_validated() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let issue = |roles: Vec<OfferedRole>| {
        issue_invitation(
            &pool,
            IssueInvitation {
                tenant_id: fau.tenant_id,
                actor_membership_id: fau.admin_membership_id,
                recipient: email("ny@example.test"),
                roles,
                handover_grant_id: None,
            },
            t0,
        )
    };
    let current = period(day(2026, 9, 23), day(2027, 9, 1));

    assert_eq!(
        issue(vec![]).await.unwrap_err(),
        MembershipError::NoRolesOffered
    );
    assert_eq!(
        issue(vec![OfferedRole {
            role: RoleChoice::Existing(Uuid::now_v7()),
            period: current,
        }])
        .await
        .unwrap_err(),
        MembershipError::UnknownRole
    );
    assert_eq!(
        issue(vec![
            OfferedRole {
                role: RoleChoice::Existing(fau.admin_role_id),
                period: current,
            },
            OfferedRole {
                role: RoleChoice::Existing(fau.admin_role_id),
                period: current,
            },
        ])
        .await
        .unwrap_err(),
        MembershipError::DuplicateRole
    );
    assert_eq!(
        issue(vec![OfferedRole {
            role: RoleChoice::Existing(fau.admin_role_id),
            period: period(day(2025, 9, 1), day(2026, 9, 23)),
        }])
        .await
        .unwrap_err(),
        MembershipError::PeriodAlreadyEnded
    );
}

#[tokio::test]
async fn a_normal_issuer_who_has_lost_admin_can_withdraw_but_not_resend() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let issued = invite(
        &pool,
        &fau,
        fau.admin_membership_id,
        "ny@example.test",
        new_role("Medlem", CapabilityClass::Member),
        t0,
    )
    .await
    .unwrap();

    sqlx::query("update role_assignments set revoked_at = now() where id = $1")
        .bind(fau.admin_assignment_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();

    let change = InvitationChange {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        invitation_id: issued.invitation_id,
    };
    assert_eq!(
        resend_invitation(&pool, change, t0).await.unwrap_err(),
        MembershipError::NotAuthorized,
        "resend needs current authority, not just having been the issuer"
    );
    withdraw_invitation(&pool, change, t0).await.expect(
        "withdrawing needs only that the actor was the issuer and is still a usable member",
    );
}

#[tokio::test]
async fn a_non_admin_who_is_not_the_issuer_cannot_resend_or_withdraw() {
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
    let issued = invite(
        &pool,
        &fau,
        fau.admin_membership_id,
        "ny@example.test",
        new_role("Medlem", CapabilityClass::Member),
        t0,
    )
    .await
    .unwrap();

    let change = InvitationChange {
        tenant_id: fau.tenant_id,
        actor_membership_id: member.membership_id,
        invitation_id: issued.invitation_id,
    };
    assert_eq!(
        resend_invitation(&pool, change, t0).await.unwrap_err(),
        MembershipError::NotAuthorized
    );
    assert_eq!(
        withdraw_invitation(&pool, change, t0).await.unwrap_err(),
        MembershipError::NotAuthorized
    );
}

#[tokio::test]
async fn resend_is_refused_while_frozen_but_withdraw_still_succeeds() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let issued = invite(
        &pool,
        &fau,
        fau.admin_membership_id,
        "ny@example.test",
        new_role("Medlem", CapabilityClass::Member),
        t0,
    )
    .await
    .unwrap();

    sqlx::query("update tenants set frozen_at = now() where id = $1")
        .bind(fau.tenant_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();

    let change = InvitationChange {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        invitation_id: issued.invitation_id,
    };
    assert_eq!(
        resend_invitation(&pool, change, t0).await.unwrap_err(),
        MembershipError::TenantFrozen
    );
    withdraw_invitation(&pool, change, t0)
        .await
        .expect("withdrawing only ever removes a way in, so it works on a frozen FAU");
}

#[tokio::test]
async fn resend_of_a_withdrawn_or_accepted_invitation_is_refused() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;

    let withdrawn = invite(
        &pool,
        &fau,
        fau.admin_membership_id,
        "a@example.test",
        new_role("Medlem", CapabilityClass::Member),
        t0,
    )
    .await
    .unwrap();
    let change = InvitationChange {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        invitation_id: withdrawn.invitation_id,
    };
    withdraw_invitation(&pool, change, t0).await.unwrap();
    assert_eq!(
        resend_invitation(&pool, change, t0).await.unwrap_err(),
        MembershipError::InvitationNotPending
    );

    let accepted = invite(
        &pool,
        &fau,
        fau.admin_membership_id,
        "b@example.test",
        new_role("Medlem", CapabilityClass::Member),
        t0,
    )
    .await
    .unwrap();
    accept_invitation(&pool, accept(accepted.token.expose(), "b@example.test"), t0)
        .await
        .unwrap();
    let change = InvitationChange {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        invitation_id: accepted.invitation_id,
    };
    assert_eq!(
        resend_invitation(&pool, change, t0).await.unwrap_err(),
        MembershipError::InvitationNotPending
    );
}

#[tokio::test]
async fn concurrent_acceptance_of_the_same_token_succeeds_exactly_once() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let issued = invite(
        &pool,
        &fau,
        fau.admin_membership_id,
        "ny@example.test",
        new_role("Medlem", CapabilityClass::Member),
        t0,
    )
    .await
    .unwrap();
    let token = issued.token.expose().to_owned();

    let (r1, r2) = tokio::join!(
        accept_invitation(&pool, accept(&token, "ny@example.test"), t0),
        accept_invitation(&pool, accept(&token, "ny@example.test"), t0),
    );
    let outcomes = [r1, r2];
    let successes = outcomes.iter().filter(|r| r.is_ok()).count();
    assert_eq!(successes, 1, "{outcomes:?}");
    let failure = outcomes.into_iter().find(Result::is_err).unwrap();
    assert_eq!(
        failure.unwrap_err(),
        MembershipError::Acceptance(AcceptanceRefusal::AlreadyAccepted)
    );

    assert_eq!(
        count(&pool, "select count(*) from memberships").await,
        2,
        "the admin plus exactly one new member -- the loser created nothing"
    );
    let assignments: i64 = sqlx::query_scalar(
        "select count(*) from role_assignments where tenant_id = $1 and membership_id in
           (select id from memberships where tenant_id = $1 and account_id in
             (select id from accounts where email = 'ny@example.test'))",
    )
    .bind(fau.tenant_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(assignments, 1, "exactly one set of roles was granted");
}
