//! Handover grants and the no-admin recovery path (flow spec §6.2–6.4; §10 "Handover
//! and the no-admin state").

mod common;
use common::membership::*;
use common::TestDb;
use fau_domain::membership::acceptance::AcceptanceRefusal;
use fau_domain::membership::vocabulary::CapabilityClass;
use fau_domain::time::Moment;
use fau_persistence::membership::{
    accept_invitation, approve_request, create_access_request, create_handover_grants,
    effective_access, grant_role, issue_invitation, recovery_grant_admin, revoke_membership,
    revoke_role_assignment, AcceptInvitation, CreateAccessRequest, GrantRole, IssueInvitation,
    MembershipError, OfferedRole, RecoveryActor, RecoveryGrant, RequestDecision, RevokeAssignment,
    RevokeMembership, RoleChoice,
};
use sqlx::PgPool;
use uuid::Uuid;

async fn grant_for(pool: &PgPool, assignment_id: Uuid) -> Option<(Uuid, String, String, bool)> {
    sqlx::query_as(
        "select id, starts_on::text, ends_on_exclusive::text, revoked_at is not null
           from handover_grants where source_assignment_id = $1",
    )
    .bind(assignment_id)
    .fetch_optional(pool)
    .await
    .unwrap()
}

fn accept(token: &str, acceptor: &str) -> AcceptInvitation {
    AcceptInvitation {
        token: token.to_owned(),
        acceptor: verified(acceptor),
        admin_end_override: None,
    }
}

#[tokio::test]
async fn a_naturally_ended_admin_role_gets_a_six_month_grant() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;

    assert_eq!(
        create_handover_grants(&pool, at("2027-09-30T10:00:00Z"))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        create_handover_grants(&pool, at("2027-10-01T10:00:00Z"))
            .await
            .unwrap(),
        1
    );
    let (_, starts, ends, revoked) = grant_for(&pool, fau.admin_assignment_id).await.unwrap();
    assert_eq!(
        (starts.as_str(), ends.as_str(), revoked),
        ("2027-10-01", "2028-04-01", false)
    );
    assert_eq!(
        create_handover_grants(&pool, at("2027-10-02T10:00:00Z"))
            .await
            .unwrap(),
        0,
        "one grant per source assignment"
    );
    assert_eq!(audit_count(&pool, "handover.granted").await, 1);
}

#[tokio::test]
async fn the_grant_boundary_truncates_to_the_end_of_february() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let short = add_member(
        &pool,
        &fau,
        "kort@example.test",
        RoleChoice::Existing(fau.admin_role_id),
        period(day(2026, 9, 23), day(2027, 8, 31)),
        t0,
    )
    .await;

    create_handover_grants(&pool, at("2027-09-15T10:00:00Z"))
        .await
        .unwrap();
    let (_, starts, ends, _) = grant_for(&pool, short.assignment_ids[0]).await.unwrap();
    assert_eq!(
        (starts.as_str(), ends.as_str()),
        ("2027-08-31", "2028-02-29"),
        "a late sweep still starts the window at the role's end"
    );
}

#[tokio::test]
async fn a_revoked_admin_role_gets_no_grant_and_revocation_ends_an_existing_one() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let revoked_early = add_member(
        &pool,
        &fau,
        "a@example.test",
        RoleChoice::Existing(fau.admin_role_id),
        period(day(2026, 9, 23), day(2027, 3, 1)),
        t0,
    )
    .await;
    let ends_naturally = add_member(
        &pool,
        &fau,
        "b@example.test",
        RoleChoice::Existing(fau.admin_role_id),
        period(day(2026, 9, 23), day(2027, 3, 1)),
        t0,
    )
    .await;
    revoke_role_assignment(
        &pool,
        RevokeAssignment {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            assignment_id: revoked_early.assignment_ids[0],
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .unwrap();

    let t1 = at("2027-03-01T10:00:00Z");
    assert_eq!(create_handover_grants(&pool, t1).await.unwrap(), 1);
    assert!(grant_for(&pool, revoked_early.assignment_ids[0])
        .await
        .is_none());
    assert!(grant_for(&pool, ends_naturally.assignment_ids[0])
        .await
        .is_some());

    revoke_role_assignment(
        &pool,
        RevokeAssignment {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            assignment_id: ends_naturally.assignment_ids[0],
            confirm_no_admin: false,
        },
        t1,
    )
    .await
    .unwrap();
    let (_, _, _, revoked) = grant_for(&pool, ends_naturally.assignment_ids[0])
        .await
        .unwrap();
    assert!(
        revoked,
        "revoking the role later ends the grant it produced"
    );
}

/// An FAU whose registrant's admin role ended on 2027-10-01 and whose grant exists; a
/// member role keeps the registrant in the FAU. Returns the FAU, the grant id and the
/// moment inside the window.
async fn outgoing_admin(pool: &PgPool) -> (Fau, Uuid, Moment) {
    let t0 = at(T0);
    let fau = active_fau(pool, "gammel@example.test", t0).await;
    grant_role(
        pool,
        GrantRole {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: fau.admin_membership_id,
            role: new_role("Medlem", CapabilityClass::Member),
            period: period(day(2026, 9, 23), day(2028, 9, 1)),
        },
        t0,
    )
    .await
    .unwrap();
    let inside = at("2027-11-01T10:00:00Z");
    create_handover_grants(pool, inside).await.unwrap();
    let (grant_id, ..) = grant_for(pool, fau.admin_assignment_id).await.unwrap();
    (fau, grant_id, inside)
}

fn handover_invite(
    fau: &Fau,
    grant_id: Uuid,
    recipient: &str,
    role: RoleChoice,
) -> IssueInvitation {
    IssueInvitation {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        recipient: email(recipient),
        roles: vec![OfferedRole {
            role,
            period: period(day(2027, 11, 1), day(2028, 10, 1)),
        }],
        handover_grant_id: Some(grant_id),
    }
}

#[tokio::test]
async fn an_outgoing_admin_brings_in_a_replacement_who_becomes_an_ordinary_admin() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let (fau, grant_id, inside) = outgoing_admin(&pool).await;

    let issued = issue_invitation(
        &pool,
        handover_invite(
            &fau,
            grant_id,
            "ny@example.test",
            RoleChoice::Existing(fau.admin_role_id),
        ),
        inside,
    )
    .await
    .unwrap();
    let mode: String = sqlx::query_scalar("select mode from invitations where id = $1")
        .bind(issued.invitation_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(mode, "handover");
    let accepted = accept_invitation(
        &pool,
        accept(issued.token.expose(), "ny@example.test"),
        inside,
    )
    .await
    .unwrap();

    // The new admin acts as any admin: here, granting a role.
    grant_role(
        &pool,
        GrantRole {
            tenant_id: fau.tenant_id,
            actor_membership_id: accepted.membership_id,
            membership_id: accepted.membership_id,
            role: new_role("Kasserer", CapabilityClass::Member),
            period: period(day(2027, 11, 1), day(2028, 10, 1)),
        },
        inside,
    )
    .await
    .expect("the replacement is an ordinary admin");
}

#[tokio::test]
async fn handover_allows_nothing_else() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let (fau, grant_id, inside) = outgoing_admin(&pool).await;

    // Not extending their own role.
    assert_eq!(
        grant_role(
            &pool,
            GrantRole {
                tenant_id: fau.tenant_id,
                actor_membership_id: fau.admin_membership_id,
                membership_id: fau.admin_membership_id,
                role: RoleChoice::Existing(fau.admin_role_id),
                period: period(day(2027, 11, 1), day(2028, 10, 1)),
            },
            inside,
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized
    );
    // Not inviting themselves.
    assert_eq!(
        issue_invitation(
            &pool,
            handover_invite(
                &fau,
                grant_id,
                "gammel@example.test",
                RoleChoice::Existing(fau.admin_role_id)
            ),
            inside,
        )
        .await
        .unwrap_err(),
        MembershipError::SelfInvitation
    );
    // Not editing the organisation.
    assert_eq!(
        issue_invitation(
            &pool,
            handover_invite(
                &fau,
                grant_id,
                "ny@example.test",
                new_role("Ny rolle", CapabilityClass::Member)
            ),
            inside,
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized
    );
    // Not issuing ordinary invitations, and not approving requests.
    assert_eq!(
        issue_invitation(
            &pool,
            IssueInvitation {
                handover_grant_id: None,
                ..handover_invite(
                    &fau,
                    grant_id,
                    "ny@example.test",
                    RoleChoice::Existing(fau.admin_role_id)
                )
            },
            inside,
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized
    );
    let request_id = create_access_request(
        &pool,
        CreateAccessRequest {
            tenant_id: fau.tenant_id,
            requester: verified("sporsmal@example.test"),
        },
        inside,
    )
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
                role: RoleChoice::Existing(fau.admin_role_id),
                period: period(day(2027, 11, 1), day(2028, 10, 1)),
            }],
            inside,
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized
    );
    // Not revoking anyone else's role.
    let member = add_member(
        &pool,
        &fau,
        "medlem@example.test",
        new_role("Kasserer", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2028, 9, 1)),
        at(T0),
    )
    .await;
    assert_eq!(
        revoke_role_assignment(
            &pool,
            RevokeAssignment {
                tenant_id: fau.tenant_id,
                actor_membership_id: fau.admin_membership_id,
                assignment_id: member.assignment_ids[0],
                confirm_no_admin: true,
            },
            inside,
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized
    );
    // Not after the boundary.
    assert_eq!(
        issue_invitation(
            &pool,
            handover_invite(
                &fau,
                grant_id,
                "ny@example.test",
                RoleChoice::Existing(fau.admin_role_id)
            ),
            at("2028-04-01T10:00:00Z"),
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized
    );
}

#[tokio::test]
async fn a_handover_invitation_dies_with_its_grant() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let (fau, grant_id, inside) = outgoing_admin(&pool).await;
    let issued = issue_invitation(
        &pool,
        handover_invite(
            &fau,
            grant_id,
            "ny@example.test",
            RoleChoice::Existing(fau.admin_role_id),
        ),
        inside,
    )
    .await
    .unwrap();
    sqlx::query("update handover_grants set revoked_at = now() where id = $1")
        .bind(grant_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();
    assert_eq!(
        accept_invitation(
            &pool,
            accept(issued.token.expose(), "ny@example.test"),
            inside
        )
        .await
        .unwrap_err(),
        MembershipError::Acceptance(AcceptanceRefusal::IssuerLacksAuthority)
    );
}

/// A handover issuer may re-send and withdraw their own invitation while the grant is
/// valid (spec §6.3).
#[tokio::test]
async fn a_handover_issuer_can_resend_and_withdraw_while_the_grant_is_valid() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let (fau, grant_id, inside) = outgoing_admin(&pool).await;
    let issued = issue_invitation(
        &pool,
        handover_invite(
            &fau,
            grant_id,
            "ny@example.test",
            RoleChoice::Existing(fau.admin_role_id),
        ),
        inside,
    )
    .await
    .unwrap();

    let resent = fau_persistence::membership::resend_invitation(
        &pool,
        fau_persistence::membership::InvitationChange {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            invitation_id: issued.invitation_id,
        },
        inside,
    )
    .await
    .expect("the issuer can resend while their grant is valid");
    assert_ne!(resent.token.expose(), issued.token.expose());

    fau_persistence::membership::withdraw_invitation(
        &pool,
        fau_persistence::membership::InvitationChange {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            invitation_id: issued.invitation_id,
        },
        inside,
    )
    .await
    .expect("the issuer can withdraw their own handover invitation");

    assert_eq!(
        accept_invitation(
            &pool,
            accept(resent.token.expose(), "ny@example.test"),
            inside
        )
        .await
        .unwrap_err(),
        MembershipError::Acceptance(AcceptanceRefusal::Revoked)
    );
}

/// After the handover grant has ended, the issuer may still withdraw their own pending
/// invitation (a controller ruling: removing a way in never needs standing authority),
/// but may no longer re-send it.
#[tokio::test]
async fn after_the_grant_ends_the_issuer_can_withdraw_but_not_resend() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let (fau, grant_id, inside) = outgoing_admin(&pool).await;
    let issued = issue_invitation(
        &pool,
        handover_invite(
            &fau,
            grant_id,
            "ny@example.test",
            RoleChoice::Existing(fau.admin_role_id),
        ),
        inside,
    )
    .await
    .unwrap();

    let after_boundary = at("2028-04-01T10:00:00Z");
    assert_eq!(
        fau_persistence::membership::resend_invitation(
            &pool,
            fau_persistence::membership::InvitationChange {
                tenant_id: fau.tenant_id,
                actor_membership_id: fau.admin_membership_id,
                invitation_id: issued.invitation_id,
            },
            after_boundary,
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized,
        "the grant has ended, so re-sending is no longer authorized"
    );

    fau_persistence::membership::withdraw_invitation(
        &pool,
        fau_persistence::membership::InvitationChange {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            invitation_id: issued.invitation_id,
        },
        after_boundary,
    )
    .await
    .expect("withdrawing only ever removes a way in, so it needs no standing authority");
}

/// An FAU in the no-admin state with one ordinary member left.
async fn without_admin(pool: &PgPool) -> Fau {
    let t0 = at(T0);
    let fau = active_fau(pool, "admin@example.test", t0).await;
    add_member(
        pool,
        &fau,
        "medlem@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 23), day(2027, 9, 1)),
        t0,
    )
    .await;
    revoke_membership(
        pool,
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
    fau
}

fn recovery(fau: &Fau, actor: RecoveryActor, recipient: &str, role: RoleChoice) -> RecoveryGrant {
    RecoveryGrant {
        tenant_id: fau.tenant_id,
        actor,
        recipient: email(recipient),
        role,
        period: period(day(2026, 9, 23), day(2027, 10, 1)),
    }
}

#[tokio::test]
async fn in_the_no_admin_state_the_recovery_contact_can_grant_admin() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = without_admin(&pool).await;

    let issued = recovery_grant_admin(
        &pool,
        recovery(
            &fau,
            RecoveryActor::Ewb,
            "ny-leder@example.test",
            RoleChoice::Existing(fau.admin_role_id),
        ),
        t0,
    )
    .await
    .unwrap();
    assert_eq!(audit_count(&pool, "recovery.admin_invited").await, 1);
    assert_eq!(
        outbox_count(&pool, "recovery.invitation_created", "medlem@example.test").await,
        1
    );
    assert_eq!(
        outbox_count(&pool, "recovery.invitation_created", "fau@ewb-solutions.as").await,
        1
    );

    accept_invitation(
        &pool,
        accept(issued.token.expose(), "ny-leder@example.test"),
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        outbox_count(&pool, "recovery.invitation_accepted", "medlem@example.test").await,
        1
    );
    assert_eq!(
        outbox_count(
            &pool,
            "recovery.invitation_accepted",
            "fau@ewb-solutions.as"
        )
        .await,
        1
    );

    // An admin exists again, so the recovery contact's power is gone.
    assert_eq!(
        recovery_grant_admin(
            &pool,
            recovery(
                &fau,
                RecoveryActor::Ewb,
                "tredje@example.test",
                RoleChoice::Existing(fau.admin_role_id)
            ),
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::NotInNoAdminState
    );
}

#[tokio::test]
async fn outside_the_no_admin_state_the_recovery_contact_cannot_grant_admin() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    assert_eq!(
        recovery_grant_admin(
            &pool,
            recovery(
                &fau,
                RecoveryActor::Ewb,
                "ny@example.test",
                RoleChoice::Existing(fau.admin_role_id)
            ),
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::NotInNoAdminState
    );
}

#[tokio::test]
async fn only_the_seat_holder_acts_and_never_for_itself() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = without_admin(&pool).await;
    let admin_role = || RoleChoice::Existing(fau.admin_role_id);
    // The FAU already has a member-class "Medlem" role (from `without_admin`), which
    // exercises `NotAdminRole` on a role recovery is at least allowed to *name*.
    // Offering a brand-new role is refused categorically, before capability is even
    // considered (fix round 1, Important #2; see `recovery_cannot_create_a_new_role`).
    let member_role_id: Uuid =
        sqlx::query_scalar("select id from roles where tenant_id = $1 and name = 'Medlem'")
            .bind(fau.tenant_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(
        recovery_grant_admin(
            &pool,
            recovery(
                &fau,
                RecoveryActor::SchoolRep(verified("rektor@skole.example.test")),
                "ny@example.test",
                admin_role(),
            ),
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized,
        "an unseated school representative has no power"
    );
    assert_eq!(
        recovery_grant_admin(
            &pool,
            recovery(
                &fau,
                RecoveryActor::Ewb,
                "fau@ewb-solutions.as",
                admin_role()
            ),
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::SelfInvitation
    );
    assert_eq!(
        recovery_grant_admin(
            &pool,
            recovery(
                &fau,
                RecoveryActor::Ewb,
                "ny@example.test",
                RoleChoice::Existing(member_role_id)
            ),
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::NotAdminRole
    );
}

#[tokio::test]
async fn with_no_members_left_recent_role_holders_are_told() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "sist@example.test", t0).await;
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

    recovery_grant_admin(
        &pool,
        recovery(
            &fau,
            RecoveryActor::Ewb,
            "ny@example.test",
            RoleChoice::Existing(fau.admin_role_id),
        ),
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        outbox_count(&pool, "recovery.invitation_created", "sist@example.test").await,
        1
    );
    assert_eq!(
        outbox_count(&pool, "recovery.invitation_created", "fau@ewb-solutions.as").await,
        1
    );
}

/// The seat may move between two recovery invitations being issued and one of them
/// being accepted (spec §6.4, §6.5). An invitation issued under a seat that has since
/// moved elsewhere is refused at acceptance, the same way a handover invitation dies
/// with its grant.
#[tokio::test]
async fn a_recovery_invitation_dies_when_the_seat_moves_to_someone_else() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = without_admin(&pool).await;

    let issued = recovery_grant_admin(
        &pool,
        recovery(
            &fau,
            RecoveryActor::Ewb,
            "ny-leder@example.test",
            RoleChoice::Existing(fau.admin_role_id),
        ),
        t0,
    )
    .await
    .unwrap();

    // The seat moves from EWB to a confirmed school representative before the
    // invitation is accepted.
    sqlx::query(
        "update recovery_contacts set
           holder = 'school_rep',
           nomination_status = 'confirmed',
           nominee_email = 'rektor@skole.example.test',
           domain_verified_at = now(),
           title_checked_by = 'erik',
           title_checked_at = now(),
           title_check_method = 'telephone'
         where tenant_id = $1",
    )
    .bind(fau.tenant_id)
    .execute(&db.admin_pool())
    .await
    .unwrap();

    assert_eq!(
        accept_invitation(
            &pool,
            accept(issued.token.expose(), "ny-leder@example.test"),
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::Acceptance(AcceptanceRefusal::IssuerLacksAuthority),
        "the invitation was issued under a seat that has since moved"
    );
}

// -- Fix round 1 regression tests --------------------------------------------------

/// Defence in depth against the sweep race (fix round 1, Critical #1): a grant whose
/// source assignment has since been revoked must confer no authority, even though the
/// grant row itself was never revoked. Manufactured directly via `admin_pool` --
/// "revoke the source, leave the grant unrevoked" is exactly the state the unlocked
/// sweep could once have produced; the locked sweep (tested separately below) means the
/// public API can no longer reach it, but the grant reads must not trust that either.
#[tokio::test]
async fn a_grant_whose_source_is_revoked_confers_no_authority() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;

    revoke_role_assignment(
        &pool,
        RevokeAssignment {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            assignment_id: fau.admin_assignment_id,
            confirm_no_admin: true,
        },
        t0,
    )
    .await
    .unwrap();

    let stray_grant_id = Uuid::now_v7();
    sqlx::query(
        "insert into handover_grants
           (tenant_id, id, source_assignment_id, starts_on, ends_on_exclusive)
         values ($1, $2, $3, $4::date, $5::date)",
    )
    .bind(fau.tenant_id)
    .bind(stray_grant_id)
    .bind(fau.admin_assignment_id)
    .bind("2026-09-23")
    .bind("2027-09-23")
    .execute(&db.admin_pool())
    .await
    .unwrap();

    // The stray grant gives the outgoing admin no handover authority: issuing under it
    // is refused just as if the grant did not exist.
    assert_eq!(
        issue_invitation(
            &pool,
            handover_invite(
                &fau,
                stray_grant_id,
                "ny@example.test",
                RoleChoice::Existing(fau.admin_role_id),
            ),
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized,
        "a grant sourced from a revoked assignment must not validate"
    );

    // Nor does the stray grant count toward the no-admin state: the recovery contact
    // can still act, because `AdminState` does not count it either.
    recovery_grant_admin(
        &pool,
        recovery(
            &fau,
            RecoveryActor::Ewb,
            "ny-leder@example.test",
            RoleChoice::Existing(fau.admin_role_id),
        ),
        t0,
    )
    .await
    .expect("a grant sourced from a revoked assignment must not count as an admin");
}

/// The ordinary, sequential half of Critical #1: an assignment revoked before the sweep
/// ever runs gets no grant, whether the sweep discovers this from its own outer scan or
/// re-confirms it under the tenant lock.
#[tokio::test]
async fn the_sweep_skips_an_assignment_revoked_before_it_runs() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;

    revoke_role_assignment(
        &pool,
        RevokeAssignment {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            assignment_id: fau.admin_assignment_id,
            confirm_no_admin: true,
        },
        t0,
    )
    .await
    .unwrap();

    assert_eq!(
        create_handover_grants(&pool, at("2027-10-01T10:00:00Z"))
            .await
            .unwrap(),
        0
    );
    assert!(grant_for(&pool, fau.admin_assignment_id).await.is_none());
    assert_eq!(audit_count(&pool, "handover.granted").await, 0);
}

/// The sweep skips an assignment whose membership has been revoked, even when the
/// assignment's own `revoked_at` stays null because it had already ended naturally
/// before the membership was revoked (Important #3, Minor #4: `USABLE_ACCOUNT`, not
/// just `ra.revoked_at`, gates the sweep).
#[tokio::test]
async fn the_sweep_skips_an_assignment_whose_membership_is_revoked() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;

    // The admin's own role has already ended; only then do they leave the FAU
    // entirely. Since final review I1, `revoke_membership` also revokes an ended admin
    // role whose handover window is still open, so the sweep now has two independent
    // reasons to skip it: the revoked assignment and the revoked membership.
    let after_role_end = at("2027-10-15T10:00:00Z");
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: fau.admin_membership_id,
            confirm_no_admin: false,
        },
        after_role_end,
    )
    .await
    .unwrap();

    assert_eq!(
        create_handover_grants(&pool, at("2027-11-01T10:00:00Z"))
            .await
            .unwrap(),
        0
    );
    assert!(grant_for(&pool, fau.admin_assignment_id).await.is_none());
    assert_eq!(audit_count(&pool, "handover.granted").await, 0);
}

/// The sweep skips a tenant that is not active (Important #3). Arranged directly via
/// `admin_pool`: no persistence function closes a tenant yet.
#[tokio::test]
async fn the_sweep_skips_non_active_tenants() {
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
        create_handover_grants(&pool, at("2027-10-01T10:00:00Z"))
            .await
            .unwrap(),
        0
    );
    assert!(grant_for(&pool, fau.admin_assignment_id).await.is_none());
    assert_eq!(audit_count(&pool, "handover.granted").await, 0);
}

/// The sweep skips a window that is already over by the time it runs at all -- a sweep
/// so late that even the six-month handover boundary has passed (Important #3).
#[tokio::test]
async fn the_sweep_skips_a_window_that_is_already_over() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;

    // The admin role ends 2027-10-01; its handover window is [2027-10-01, 2028-04-01).
    // The very first sweep only runs after that boundary has already passed.
    assert_eq!(
        create_handover_grants(&pool, at("2028-05-01T10:00:00Z"))
            .await
            .unwrap(),
        0
    );
    assert!(grant_for(&pool, fau.admin_assignment_id).await.is_none());
    assert_eq!(audit_count(&pool, "handover.granted").await, 0);
}

/// Recovery is refused while a valid handover grant exists, even with no acting admin:
/// the negation of the no-admin state is "no admin role valid today *and* no handover
/// grant valid today" (spec 6.4), so a grant alone keeps the FAU out of it.
#[tokio::test]
async fn recovery_is_refused_while_a_valid_handover_grant_exists() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let inside = at("2027-11-01T10:00:00Z");
    let fau = active_fau(&pool, "gammel@example.test", at(T0)).await;
    create_handover_grants(&pool, inside).await.unwrap();

    assert_eq!(
        recovery_grant_admin(
            &pool,
            recovery(
                &fau,
                RecoveryActor::Ewb,
                "ny@example.test",
                RoleChoice::Existing(fau.admin_role_id),
            ),
            inside,
        )
        .await
        .unwrap_err(),
        MembershipError::NotInNoAdminState,
        "a valid handover grant alone keeps the FAU out of the no-admin state"
    );
}

/// A confirmed school representative may act once seated; EWB is refused once it no
/// longer holds the seat; and a different address than the confirmed nominee has no
/// power, even while holding the `school_rep` seat kind (Important #3).
#[tokio::test]
async fn a_confirmed_school_rep_holds_the_seat_and_only_that_rep_may_act() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = without_admin(&pool).await;

    sqlx::query(
        "update recovery_contacts set
           holder = 'school_rep',
           nomination_status = 'confirmed',
           nominee_email = 'rektor@skole.example.test',
           domain_verified_at = now(),
           title_checked_by = 'erik',
           title_checked_at = now(),
           title_check_method = 'telephone'
         where tenant_id = $1",
    )
    .bind(fau.tenant_id)
    .execute(&db.admin_pool())
    .await
    .unwrap();

    assert_eq!(
        recovery_grant_admin(
            &pool,
            recovery(
                &fau,
                RecoveryActor::Ewb,
                "ny@example.test",
                RoleChoice::Existing(fau.admin_role_id),
            ),
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized,
        "EWB no longer holds the seat"
    );
    assert_eq!(
        recovery_grant_admin(
            &pool,
            recovery(
                &fau,
                RecoveryActor::SchoolRep(verified("annen@skole.example.test")),
                "ny@example.test",
                RoleChoice::Existing(fau.admin_role_id),
            ),
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized,
        "a different address than the confirmed nominee has no power"
    );
    recovery_grant_admin(
        &pool,
        recovery(
            &fau,
            RecoveryActor::SchoolRep(verified("rektor@skole.example.test")),
            "ny@example.test",
            RoleChoice::Existing(fau.admin_role_id),
        ),
        t0,
    )
    .await
    .expect("the confirmed nominee holds the seat");
}

/// End to end: a recovery invitation is refused at acceptance once an admin has
/// appeared since it was issued, through a second recovery invitation being accepted
/// first -- the ordinary `tenant_has_admin_today` path, exercised through the public
/// API rather than the domain unit test alone.
#[tokio::test]
async fn a_recovery_invitation_is_refused_at_acceptance_once_an_admin_has_appeared() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = without_admin(&pool).await;

    let issued = recovery_grant_admin(
        &pool,
        recovery(
            &fau,
            RecoveryActor::Ewb,
            "ny-leder@example.test",
            RoleChoice::Existing(fau.admin_role_id),
        ),
        t0,
    )
    .await
    .unwrap();

    let second = recovery_grant_admin(
        &pool,
        recovery(
            &fau,
            RecoveryActor::Ewb,
            "annen-leder@example.test",
            RoleChoice::Existing(fau.admin_role_id),
        ),
        t0,
    )
    .await
    .unwrap();
    accept_invitation(
        &pool,
        accept(second.token.expose(), "annen-leder@example.test"),
        t0,
    )
    .await
    .unwrap();

    assert_eq!(
        accept_invitation(
            &pool,
            accept(issued.token.expose(), "ny-leder@example.test"),
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::Acceptance(AcceptanceRefusal::IssuerLacksAuthority),
        "an admin now exists, so the earlier recovery invitation no longer carries authority"
    );
}

/// Recovery cannot create a new role (Important #2): its one power is granting an
/// existing role, never editing the organisation.
#[tokio::test]
async fn recovery_cannot_create_a_new_role() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = without_admin(&pool).await;

    assert_eq!(
        recovery_grant_admin(
            &pool,
            recovery(
                &fau,
                RecoveryActor::Ewb,
                "ny@example.test",
                new_role("Ny leder", CapabilityClass::Admin),
            ),
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized,
        "the recovery contact may only offer an existing role, never create one"
    );
}

/// Final review I1: an admin role that ends on the very day its membership is revoked,
/// before the sweep has given it a grant, must not come back to life as a handover grant
/// when the same person is later invited back as a plain member. `ensure_membership`
/// reopens the old membership row, so the ended assignment would otherwise be an
/// unrevoked admin-class role behind a usable membership -- exactly the sweep's input.
#[tokio::test]
async fn a_removed_admin_does_not_regain_handover_through_reopen_and_the_sweep() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let other = add_member(
        &pool,
        &fau,
        "admin2@example.test",
        RoleChoice::Existing(fau.admin_role_id),
        period(day(2026, 9, 23), day(2028, 10, 1)),
        t0,
    )
    .await;

    // 1-2. The registrant's admin role ends on D = 2027-10-01; the same day, before any
    // sweep, the other admin removes them.
    let d = at("2027-10-01T08:00:00Z");
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: other.membership_id,
            membership_id: fau.admin_membership_id,
            confirm_no_admin: false,
        },
        d,
    )
    .await
    .unwrap();

    // 3. Invited back as a plain member, and accepted.
    let issued = issue_invitation(
        &pool,
        IssueInvitation {
            tenant_id: fau.tenant_id,
            actor_membership_id: other.membership_id,
            recipient: email("admin@example.test"),
            roles: vec![OfferedRole {
                role: new_role("Medlem", CapabilityClass::Member),
                period: period(day(2027, 10, 1), day(2028, 9, 1)),
            }],
            handover_grant_id: None,
        },
        d,
    )
    .await
    .unwrap();
    let back = accept_invitation(
        &pool,
        accept(issued.token.expose(), "admin@example.test"),
        d,
    )
    .await
    .unwrap();
    assert_eq!(back.membership_id, fau.admin_membership_id, "reopened");

    // 4. The sweep runs, that day and the next.
    let sweep_d = at("2027-10-01T22:00:00Z");
    assert_eq!(create_handover_grants(&pool, sweep_d).await.unwrap(), 0);
    let next = at("2027-10-02T10:00:00Z");
    assert_eq!(create_handover_grants(&pool, next).await.unwrap(), 0);

    // 5. No grant, and no handover reported.
    assert!(grant_for(&pool, fau.admin_assignment_id).await.is_none());
    let access = effective_access(&pool, fau.admin_account_id, fau.tenant_id, next)
        .await
        .unwrap();
    assert!(!access.handover);
    assert_eq!(audit_count(&pool, "handover.granted").await, 0);
}

/// Final review M7: the recovery contact's grant creates an invitation, which a frozen
/// FAU refuses like every other issuance.
#[tokio::test]
async fn recovery_is_refused_on_a_frozen_fau() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = without_admin(&pool).await;
    sqlx::query("update tenants set frozen_at = now() where id = $1")
        .bind(fau.tenant_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();
    assert_eq!(
        recovery_grant_admin(
            &pool,
            recovery(
                &fau,
                RecoveryActor::Ewb,
                "ny-leder@example.test",
                RoleChoice::Existing(fau.admin_role_id),
            ),
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::TenantFrozen
    );
}
