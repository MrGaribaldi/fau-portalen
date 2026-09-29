//! The guest member type (groups design §3.1, §8's #3418 row): an admin invites a guest to
//! one group through a guest role naming it. The guest reaches that group and nothing
//! FAU-wide; a handover grant cannot invite one.

mod common;
use common::groups::*;
use common::membership::*;
use common::TestDb;
use fau_domain::authz::{Action, Denied};
use fau_domain::membership::access::Capability;
use fau_domain::membership::vocabulary::{CapabilityClass, RoleName, Visibility};
use fau_persistence::membership::{
    accept_invitation, archive_group, authorize, create_handover_grants, effective_access,
    grant_role, issue_invitation, resend_invitation, AcceptInvitation, ArchiveGroup, GrantRole,
    InvitationChange, IssueInvitation, MembershipError, OfferedRole, Resource, RoleChoice, Viewer,
};
use sqlx::PgPool;
use uuid::Uuid;

fn guest_role(group: Option<Uuid>) -> RoleChoice {
    RoleChoice::New {
        name: RoleName::parse("Gjest").unwrap(),
        capability: CapabilityClass::Guest,
        group_id: group,
    }
}

fn invite(fau: &Fau, role: RoleChoice) -> IssueInvitation {
    IssueInvitation {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        recipient: email("gjest@example.test"),
        roles: vec![OfferedRole {
            role,
            period: period(day(2026, 9, 23), day(2027, 9, 1)),
        }],
        handover_grant_id: None,
        message: None,
    }
}

async fn decision(
    pool: &PgPool,
    viewer: Viewer,
    resource: Resource,
    action: Action,
) -> Result<(), Denied> {
    let mut conn = pool.acquire().await.unwrap();
    authorize(&mut conn, viewer, resource, action, at(T0))
        .await
        .unwrap()
}

#[tokio::test]
async fn an_admin_invites_a_guest_to_one_group() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let group = seed_group(&pool, &fau, Visibility::Closed).await;
    let other = seed_group(&pool, &fau, Visibility::Open).await;
    let guest = add_member(
        &pool,
        &fau,
        "gjest@example.test",
        guest_role(Some(group)),
        period(day(2026, 9, 23), day(2027, 9, 1)),
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
    let v = Viewer {
        tenant_id: fau.tenant_id,
        membership_id: guest.membership_id,
    };
    assert_eq!(
        decision(&pool, v, Resource::GroupContent(group), Action::Read).await,
        Ok(())
    );
    assert_eq!(
        decision(&pool, v, Resource::GroupContent(group), Action::Write).await,
        Ok(())
    );
    assert_eq!(
        decision(&pool, v, Resource::Fau, Action::Read).await,
        Err(Denied::Hidden)
    );
    assert_eq!(
        decision(&pool, v, Resource::GroupContent(other), Action::Read).await,
        Err(Denied::Hidden),
        "not even an open group the guest was not invited to"
    );

    let (class, named): (String, Option<Uuid>) = sqlx::query_as(
        "select r.capability_class, r.group_id from role_assignments ra
           join roles r on r.tenant_id = ra.tenant_id and r.id = ra.role_id
          where ra.membership_id = $1",
    )
    .bind(guest.membership_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((class.as_str(), named), ("guest", Some(group)));
    let created: String = sqlx::query_scalar(
        "select params::text from audit_events where action = 'role.created' and subject_id = (
           select role_id from role_assignments where membership_id = $1)",
    )
    .bind(guest.membership_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let created: serde_json::Value = serde_json::from_str(&created).unwrap();
    assert_eq!(created["capability_class"], "guest");
    assert_eq!(created["group_id"], group.to_string());
}

#[tokio::test]
async fn a_guest_role_names_a_group_and_only_a_guest_role_may() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let group = seed_group(&pool, &fau, Visibility::Open).await;
    let roles_before = count(&pool, "select count(*) from roles").await;
    let member_naming_group = RoleChoice::New {
        name: RoleName::parse("Medlem").unwrap(),
        capability: CapabilityClass::Member,
        group_id: Some(group),
    };
    for role in [guest_role(None), member_naming_group] {
        assert_eq!(
            issue_invitation(&pool, invite(&fau, role), t0)
                .await
                .unwrap_err(),
            MembershipError::RoleGroupMismatch
        );
    }
    assert_eq!(
        count(&pool, "select count(*) from roles").await,
        roles_before,
        "nothing written"
    );
    assert_eq!(
        count(
            &pool,
            "select count(*) from invitations where mode = 'normal'"
        )
        .await,
        0
    );
}

#[tokio::test]
async fn a_guest_role_cannot_name_an_unknown_or_archived_group() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    assert_eq!(
        issue_invitation(&pool, invite(&fau, guest_role(Some(Uuid::now_v7()))), t0)
            .await
            .unwrap_err(),
        MembershipError::UnknownGroup
    );

    let group = seed_group(&pool, &fau, Visibility::Open).await;
    let existing = seed_role(
        &pool,
        fau.tenant_id,
        CapabilityClass::Guest,
        Some(group),
        None,
        None,
    )
    .await;
    archive_group(
        &pool,
        ArchiveGroup {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            group_id: group,
        },
        t0,
    )
    .await
    .unwrap();
    for role in [guest_role(Some(group)), RoleChoice::Existing(existing)] {
        assert_eq!(
            issue_invitation(&pool, invite(&fau, role), t0)
                .await
                .unwrap_err(),
            MembershipError::GroupArchived
        );
    }
}

#[tokio::test]
async fn an_admin_may_grant_a_member_a_guest_role_too() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let group = seed_group(&pool, &fau, Visibility::Closed).await;
    let member = add_member(
        &pool,
        &fau,
        "m@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 1), day(2027, 9, 1)),
        t0,
    )
    .await;
    let v = Viewer {
        tenant_id: fau.tenant_id,
        membership_id: member.membership_id,
    };
    assert_eq!(
        decision(&pool, v, Resource::GroupContent(group), Action::Read).await,
        Err(Denied::Hidden)
    );
    grant_role(
        &pool,
        GrantRole {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: member.membership_id,
            role: guest_role(Some(group)),
            period: period(day(2026, 9, 23), day(2027, 9, 1)),
        },
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        decision(&pool, v, Resource::GroupContent(group), Action::Write).await,
        Ok(())
    );
    assert_eq!(
        effective_access(&pool, member.account_id, fau.tenant_id, t0)
            .await
            .unwrap()
            .capability,
        Capability::Member,
        "rights are the union: still a member FAU-wide"
    );
}

#[tokio::test]
async fn a_handover_grant_cannot_invite_a_guest() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    // An outgoing admin who stays a member, inside their six-month handover window.
    let fau = active_fau(&pool, "gammel@example.test", t0).await;
    grant_role(
        &pool,
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
    let group = seed_group(&pool, &fau, Visibility::Open).await;
    let guest = seed_role(
        &pool,
        fau.tenant_id,
        CapabilityClass::Guest,
        Some(group),
        None,
        None,
    )
    .await;
    let inside = at("2027-11-01T10:00:00Z");
    create_handover_grants(&pool, inside).await.unwrap();
    let grant_id: Uuid =
        sqlx::query_scalar("select id from handover_grants where source_assignment_id = $1")
            .bind(fau.admin_assignment_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    let handover = |role| IssueInvitation {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        recipient: email("ny@example.test"),
        roles: vec![OfferedRole {
            role,
            period: period(day(2027, 11, 1), day(2028, 10, 1)),
        }],
        handover_grant_id: Some(grant_id),
        message: None,
    };
    assert_eq!(
        issue_invitation(&pool, handover(RoleChoice::Existing(guest)), inside)
            .await
            .unwrap_err(),
        MembershipError::NotAuthorized,
        "inviting guests is admin-only (Ruling R17)"
    );
    issue_invitation(
        &pool,
        handover(RoleChoice::Existing(fau.admin_role_id)),
        inside,
    )
    .await
    .expect("the handover still offers the admin role it exists for");
}

#[tokio::test]
async fn a_handover_holder_offering_an_existing_guest_role_on_an_archived_group_gets_not_authorized(
) {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "gammel@example.test", t0).await;
    grant_role(
        &pool,
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
    let group = seed_group(&pool, &fau, Visibility::Open).await;
    let guest = seed_role(
        &pool,
        fau.tenant_id,
        CapabilityClass::Guest,
        Some(group),
        None,
        None,
    )
    .await;
    archive_group(
        &pool,
        ArchiveGroup {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            group_id: group,
        },
        t0,
    )
    .await
    .unwrap();
    let inside = at("2027-11-01T10:00:00Z");
    create_handover_grants(&pool, inside).await.unwrap();
    let grant_id: Uuid =
        sqlx::query_scalar("select id from handover_grants where source_assignment_id = $1")
            .bind(fau.admin_assignment_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    // Not `GroupArchived`: a handover holder is never authorized to offer a guest role at
    // all (Ruling R17), so the archived group's state must not leak through the error
    // (Ruling P12 fix round 1).
    assert_eq!(
        issue_invitation(
            &pool,
            IssueInvitation {
                tenant_id: fau.tenant_id,
                actor_membership_id: fau.admin_membership_id,
                recipient: email("ny@example.test"),
                roles: vec![OfferedRole {
                    role: RoleChoice::Existing(guest),
                    period: period(day(2027, 11, 1), day(2028, 10, 1)),
                }],
                handover_grant_id: Some(grant_id),
                message: None,
            },
            inside,
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized
    );
}

#[tokio::test]
async fn a_non_guest_roles_audit_names_no_group() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let member = add_member(
        &pool,
        &fau,
        "member@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 1), day(2027, 9, 1)),
        t0,
    )
    .await;
    let created: String = sqlx::query_scalar(
        "select params::text from audit_events where action = 'role.created' and subject_id = (
           select role_id from role_assignments where membership_id = $1)",
    )
    .bind(member.membership_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let created: serde_json::Value = serde_json::from_str(&created).unwrap();
    assert_eq!(created["capability_class"], "member");
    assert_eq!(created["group_id"], serde_json::Value::Null);
}

#[tokio::test]
async fn a_guest_role_cannot_name_another_tenants_group() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let other_fau = active_fau(&pool, "other-admin@example.test", t0).await;
    let foreign_group = seed_group(&pool, &other_fau, Visibility::Open).await;
    assert_eq!(
        issue_invitation(&pool, invite(&fau, guest_role(Some(foreign_group))), t0)
            .await
            .unwrap_err(),
        MembershipError::UnknownGroup,
        "a group id scoped to another tenant is unknown, not merely archived"
    );
}

#[tokio::test]
async fn accepting_a_guest_invitation_whose_group_was_archived_meanwhile_is_refused() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let group = seed_group(&pool, &fau, Visibility::Open).await;
    let issued = issue_invitation(&pool, invite(&fau, guest_role(Some(group))), t0)
        .await
        .unwrap();
    archive_group(
        &pool,
        ArchiveGroup {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            group_id: group,
        },
        t0,
    )
    .await
    .unwrap();

    let memberships_before = count(&pool, "select count(*) from memberships").await;
    let assignments_before = count(&pool, "select count(*) from role_assignments").await;
    assert_eq!(
        accept_invitation(
            &pool,
            AcceptInvitation {
                token: issued.token.expose().to_owned(),
                acceptor: verified("gjest@example.test"),
                admin_end_override: None,
                profile: fresh_profile(),
            },
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::GroupArchived
    );
    assert_eq!(
        count(&pool, "select count(*) from memberships").await,
        memberships_before,
        "no membership is seated"
    );
    assert_eq!(
        count(&pool, "select count(*) from role_assignments").await,
        assignments_before,
        "no role assignment is seated"
    );
}

#[tokio::test]
async fn resending_a_guest_invitation_whose_group_was_archived_meanwhile_is_refused() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let group = seed_group(&pool, &fau, Visibility::Open).await;
    let issued = issue_invitation(&pool, invite(&fau, guest_role(Some(group))), t0)
        .await
        .unwrap();
    archive_group(
        &pool,
        ArchiveGroup {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            group_id: group,
        },
        t0,
    )
    .await
    .unwrap();

    assert_eq!(
        resend_invitation(
            &pool,
            InvitationChange {
                tenant_id: fau.tenant_id,
                actor_membership_id: fau.admin_membership_id,
                invitation_id: issued.invitation_id,
            },
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::GroupArchived,
        "resending must not re-arm a guest invitation for another 14 days"
    );
}
