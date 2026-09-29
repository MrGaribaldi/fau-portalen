//! Group management (groups design §3.1 and §3.3): admin-only, audited, announced on the
//! change stream. Names arrive encrypted; these tests use an envelope-shaped placeholder.

mod common;
use std::sync::Arc;
use std::time::Duration;

use common::groups::*;
use common::membership::*;
use common::TestDb;
use fau_crypto::Ciphertext;
use fau_domain::authz::{Action, Denied};
use fau_domain::membership::vocabulary::{CapabilityClass, Visibility};
use fau_persistence::membership::{
    add_group_member, archive_group, authorize, create_group, remove_group_member, rename_group,
    revoke_membership, set_group_visibility, ArchiveGroup, Change, CreateGroup, GroupBinding,
    GroupMemberChange, Hub, MembershipError, RenameGroup, Resource, RevokeMembership,
    SetGroupVisibility, Subscription, Viewer, EVENTS_CHANNEL,
};
use sqlx::postgres::PgListener;
use sqlx::PgPool;
use uuid::Uuid;

fn name() -> Ciphertext {
    Ciphertext::from_stored(placeholder_name())
}

fn create(
    tenant_id: Uuid,
    actor: Uuid,
    visibility: Visibility,
    binding: Option<GroupBinding>,
) -> CreateGroup {
    CreateGroup {
        tenant_id,
        actor_membership_id: actor,
        group_id: Uuid::now_v7(),
        encrypted_name: name(),
        visibility,
        binding,
    }
}

fn change(fau: &Fau, actor: Uuid, group_id: Uuid, membership_id: Uuid) -> GroupMemberChange {
    GroupMemberChange {
        tenant_id: fau.tenant_id,
        actor_membership_id: actor,
        group_id,
        membership_id,
    }
}

fn visibility(fau: &Fau, group_id: Uuid, visibility: Visibility) -> SetGroupVisibility {
    SetGroupVisibility {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        group_id,
        visibility,
    }
}

fn rename(fau: &Fau, group_id: Uuid) -> RenameGroup {
    RenameGroup {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        group_id,
        encrypted_name: name(),
    }
}

fn archive(fau: &Fau, group_id: Uuid) -> ArchiveGroup {
    ArchiveGroup {
        tenant_id: fau.tenant_id,
        actor_membership_id: fau.admin_membership_id,
        group_id,
    }
}

async fn params(pool: &PgPool, action: &str) -> Vec<serde_json::Value> {
    let rows: Vec<String> = sqlx::query_scalar(
        "select params::text from audit_events where action = $1 order by occurred_at, id",
    )
    .bind(action)
    .fetch_all(pool)
    .await
    .unwrap();
    rows.iter()
        .map(|r| serde_json::from_str(r).unwrap())
        .collect()
}

async fn member(pool: &PgPool, fau: &Fau, address: &str) -> Uuid {
    add_member(
        pool,
        fau,
        address,
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 1), day(2027, 9, 1)),
        at(T0),
    )
    .await
    .membership_id
}

#[tokio::test]
async fn an_admin_creates_a_group_with_its_ciphertext_and_an_audit_entry() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    let req = create(
        fau.tenant_id,
        fau.admin_membership_id,
        Visibility::Closed,
        None,
    );
    let id = create_group(&pool, req.clone(), at(T0)).await.unwrap();
    assert_eq!(id, req.group_id, "the id the name's AAD is bound to");

    let (stored, vis): (Vec<u8>, String) =
        sqlx::query_as("select encrypted_name, visibility from groups where id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored, placeholder_name());
    assert_eq!(vis, "closed");
    assert_eq!(
        params(&pool, "group.created").await,
        [serde_json::json!({ "visibility": "closed", "unit_id": null, "cohort_id": null })],
        "ids and codes only, never the name"
    );
}

#[tokio::test]
async fn only_an_admin_manages_groups_and_a_hidden_group_stays_hidden() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let t0 = at(T0);
    for who in ["member_in", "guest_in", "none_in"] {
        assert_eq!(
            create_group(
                &pool,
                create(w.fau.tenant_id, w.membership(who), Visibility::Open, None),
                t0
            )
            .await
            .unwrap_err(),
            MembershipError::NotAuthorized,
            "{who}"
        );
    }
    let someone = w.membership("member_out");
    // A member who can see the open group may not manage it...
    assert_eq!(
        add_group_member(
            &pool,
            change(&w.fau, w.membership("member_in"), w.open, someone),
            t0
        )
        .await
        .unwrap_err(),
        MembershipError::NotAuthorized
    );
    // ...and one who cannot see the closed group learns nothing about it, exactly as for an
    // id that does not exist.
    for group in [w.closed, Uuid::now_v7()] {
        assert_eq!(
            add_group_member(
                &pool,
                change(&w.fau, w.membership("member_out"), group, someone),
                t0
            )
            .await
            .unwrap_err(),
            MembershipError::UnknownGroup
        );
    }
    assert_eq!(
        add_group_member(
            &pool,
            change(&w.fau, w.fau.admin_membership_id, Uuid::now_v7(), someone),
            t0
        )
        .await
        .unwrap_err(),
        MembershipError::UnknownGroup,
        "an admin naming an unknown group"
    );
}

#[tokio::test]
async fn a_group_binds_only_to_a_unit_or_cohort_of_its_own_fau() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let other = active_fau(&pool, "annen@example.test", t0).await;
    let unit = seed_unit(&pool, fau.tenant_id).await;
    let foreign_cohort = seed_cohort(&pool, other.tenant_id).await;
    let admin = fau.admin_membership_id;

    let bound = create_group(
        &pool,
        create(
            fau.tenant_id,
            admin,
            Visibility::Open,
            Some(GroupBinding::Unit(unit)),
        ),
        t0,
    )
    .await
    .unwrap();
    let stored: Option<Uuid> = sqlx::query_scalar("select unit_id from groups where id = $1")
        .bind(bound)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, Some(unit));
    assert_eq!(
        create_group(
            &pool,
            create(
                fau.tenant_id,
                admin,
                Visibility::Open,
                Some(GroupBinding::Unit(Uuid::now_v7()))
            ),
            t0
        )
        .await
        .unwrap_err(),
        MembershipError::UnknownUnit
    );
    assert_eq!(
        create_group(
            &pool,
            create(
                fau.tenant_id,
                admin,
                Visibility::Open,
                Some(GroupBinding::Cohort(foreign_cohort))
            ),
            t0
        )
        .await
        .unwrap_err(),
        MembershipError::UnknownCohort
    );
}

#[tokio::test]
async fn a_malformed_name_is_refused_before_the_database() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    for bad in [vec![1u8; 10], vec![2u8; 60], vec![1u8; 513]] {
        let mut req = create(
            fau.tenant_id,
            fau.admin_membership_id,
            Visibility::Open,
            None,
        );
        req.encrypted_name = Ciphertext::from_stored(bad);
        assert_eq!(
            create_group(&pool, req, at(T0)).await.unwrap_err(),
            MembershipError::GroupNameMalformed
        );
    }
}

#[tokio::test]
async fn members_are_added_once_and_removed_softly() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let admin = fau.admin_membership_id;
    let group = seed_group(&pool, &fau, Visibility::Closed).await;
    let m = member(&pool, &fau, "m@example.test").await;

    add_group_member(&pool, change(&fau, admin, group, m), t0)
        .await
        .unwrap();
    assert_eq!(
        add_group_member(&pool, change(&fau, admin, group, m), t0)
            .await
            .unwrap_err(),
        MembershipError::AlreadyInGroup
    );
    remove_group_member(&pool, change(&fau, admin, group, m), t0)
        .await
        .unwrap();
    assert_eq!(
        remove_group_member(&pool, change(&fau, admin, group, m), t0)
            .await
            .unwrap_err(),
        MembershipError::NotInGroup
    );
    add_group_member(&pool, change(&fau, admin, group, m), t0)
        .await
        .unwrap();
    assert_eq!(
        count(
            &pool,
            &format!("select count(*) from group_members where group_id = '{group}'")
        )
        .await,
        2,
        "history keeps the removed row"
    );
    assert_eq!(audit_count(&pool, "group.member_added").await, 2);
    assert_eq!(
        params(&pool, "group.member_removed").await,
        [serde_json::json!({ "membership_id": m })]
    );

    assert_eq!(
        add_group_member(&pool, change(&fau, admin, group, Uuid::now_v7()), t0)
            .await
            .unwrap_err(),
        MembershipError::UnknownMembership
    );
    let leaver = member(&pool, &fau, "borte@example.test").await;
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: leaver,
            membership_id: leaver,
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        add_group_member(&pool, change(&fau, admin, group, leaver), t0)
            .await
            .unwrap_err(),
        MembershipError::MembershipRevoked
    );
}

/// Erik, 29 September 2026: a membership whose roles ran out grants nothing and the sweep
/// would remove a group row for it (M6), so adding it is refused with
/// `MembershipEndedInviteAsGuest` -- the screen offers a guest invitation instead. Nothing
/// is written or audited. A non-admin still gets `NotAuthorized` first: authority before
/// row state.
///
/// Mutation check: drop the ended check in `add_group_member` and the admin's add succeeds.
#[tokio::test]
async fn adding_a_member_whose_roles_ran_out_offers_a_guest_invitation() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    let admin = fau.admin_membership_id;
    let group = seed_group(&pool, &fau, Visibility::Open).await;
    let ran_out = add_member(
        &pool,
        &fau,
        "ferdig@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 1), day(2026, 10, 1)),
        at(T0),
    )
    .await
    .membership_id;
    let other = member(&pool, &fau, "annen@example.test").await;
    let later = at("2026-10-15T10:00:00Z");

    assert_eq!(
        add_group_member(&pool, change(&fau, other, group, ran_out), later)
            .await
            .unwrap_err(),
        MembershipError::NotAuthorized
    );
    assert_eq!(
        add_group_member(&pool, change(&fau, admin, group, ran_out), later)
            .await
            .unwrap_err(),
        MembershipError::MembershipEndedInviteAsGuest
    );
    assert_eq!(
        count(
            &pool,
            &format!("select count(*) from group_members where membership_id = '{ran_out}'")
        )
        .await,
        0
    );
    assert_eq!(audit_count(&pool, "group.member_added").await, 0);
}

/// Controls for the refusal above: an active member is added, and so is a member whose
/// only role is still to start -- not ended, so not refused.
#[tokio::test]
async fn an_active_member_or_one_whose_roles_start_later_is_added() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    let admin = fau.admin_membership_id;
    let group = seed_group(&pool, &fau, Visibility::Open).await;
    let active = member(&pool, &fau, "aktiv@example.test").await;
    let future = add_member(
        &pool,
        &fau,
        "senere@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 11, 1), day(2027, 11, 1)),
        at(T0),
    )
    .await
    .membership_id;
    for m in [active, future] {
        add_group_member(&pool, change(&fau, admin, group, m), at(T0))
            .await
            .unwrap();
    }
    assert_eq!(audit_count(&pool, "group.member_added").await, 2);
}

#[tokio::test]
async fn closing_opening_renaming_and_archiving() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let admin = fau.admin_membership_id;
    let group = seed_group(&pool, &fau, Visibility::Open).await;
    let m = member(&pool, &fau, "m@example.test").await;
    add_group_member(&pool, change(&fau, admin, group, m), t0)
        .await
        .unwrap();

    set_group_visibility(&pool, visibility(&fau, group, Visibility::Closed), t0)
        .await
        .unwrap();
    set_group_visibility(&pool, visibility(&fau, group, Visibility::Closed), t0)
        .await
        .unwrap();
    assert_eq!(
        audit_count(&pool, "group.closed").await,
        1,
        "a no-op writes nothing"
    );
    set_group_visibility(&pool, visibility(&fau, group, Visibility::Open), t0)
        .await
        .unwrap();
    assert_eq!(audit_count(&pool, "group.opened").await, 1);

    let mut renamed = rename(&fau, group);
    let mut bytes = placeholder_name();
    bytes[1] = 7;
    renamed.encrypted_name = Ciphertext::from_stored(bytes.clone());
    rename_group(&pool, renamed, t0).await.unwrap();
    let stored: Vec<u8> = sqlx::query_scalar("select encrypted_name from groups where id = $1")
        .bind(group)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, bytes);
    assert_eq!(
        params(&pool, "group.renamed").await,
        [serde_json::json!({})]
    );

    archive_group(&pool, archive(&fau, group), t0)
        .await
        .unwrap();
    assert_eq!(audit_count(&pool, "group.archived").await, 1);
    let other = member(&pool, &fau, "ny@example.test").await;
    let outsider = Viewer {
        tenant_id: fau.tenant_id,
        membership_id: other,
    };
    let mut conn = pool.acquire().await.unwrap();
    assert_eq!(
        authorize(
            &mut conn,
            outsider,
            Resource::Group(group),
            Action::Read,
            t0
        )
        .await
        .unwrap(),
        Ok(()),
        "an archived open group is still readable by a non-member"
    );

    // Closing reduces access, so an archived group may still be closed (ruling P13): audited
    // and announced like any other visibility change, and hidden from non-members after.
    let mut listener = PgListener::connect_with(&pool).await.unwrap();
    listener.listen(EVENTS_CHANNEL).await.unwrap();
    set_group_visibility(&pool, visibility(&fau, group, Visibility::Closed), t0)
        .await
        .expect("an archived group may be closed");
    assert_eq!(audit_count(&pool, "group.closed").await, 2);
    let announced = tokio::time::timeout(Duration::from_secs(5), listener.recv())
        .await
        .expect("an announcement within five seconds")
        .unwrap();
    assert_eq!(
        Change::parse(announced.payload()),
        Some((fau.tenant_id, Change::Changed(Resource::Group(group))))
    );
    assert_eq!(
        authorize(
            &mut conn,
            outsider,
            Resource::Group(group),
            Action::Read,
            t0
        )
        .await
        .unwrap(),
        Err(Denied::Hidden),
        "once closed, the archived group is hidden from a non-member"
    );

    for err in [
        rename_group(&pool, rename(&fau, group), t0)
            .await
            .unwrap_err(),
        set_group_visibility(&pool, visibility(&fau, group, Visibility::Open), t0)
            .await
            .unwrap_err(),
        add_group_member(&pool, change(&fau, admin, group, other), t0)
            .await
            .unwrap_err(),
        archive_group(&pool, archive(&fau, group), t0)
            .await
            .unwrap_err(),
    ] {
        assert_eq!(err, MembershipError::GroupArchived);
    }
    remove_group_member(&pool, change(&fau, admin, group, m), t0)
        .await
        .expect("an archived group still loses members");
}

#[tokio::test]
async fn a_frozen_fau_refuses_additions_and_allows_reductions() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let admin = fau.admin_membership_id;
    let group = seed_group(&pool, &fau, Visibility::Open).await;
    let m = member(&pool, &fau, "m@example.test").await;
    let n = member(&pool, &fau, "n@example.test").await;
    add_group_member(&pool, change(&fau, admin, group, m), t0)
        .await
        .unwrap();
    sqlx::query("update tenants set frozen_at = $1::timestamptz where id = $2")
        .bind(T0)
        .bind(fau.tenant_id)
        .execute(&pool)
        .await
        .unwrap();

    for err in [
        create_group(
            &pool,
            create(fau.tenant_id, admin, Visibility::Open, None),
            t0,
        )
        .await
        .unwrap_err(),
        rename_group(&pool, rename(&fau, group), t0)
            .await
            .unwrap_err(),
        add_group_member(&pool, change(&fau, admin, group, n), t0)
            .await
            .unwrap_err(),
    ] {
        assert_eq!(err, MembershipError::TenantFrozen);
    }
    set_group_visibility(&pool, visibility(&fau, group, Visibility::Closed), t0)
        .await
        .expect("closing reduces access");
    assert_eq!(
        set_group_visibility(&pool, visibility(&fau, group, Visibility::Open), t0)
            .await
            .unwrap_err(),
        MembershipError::TenantFrozen
    );
    remove_group_member(&pool, change(&fau, admin, group, m), t0)
        .await
        .expect("removal reduces access");
    archive_group(&pool, archive(&fau, group), t0)
        .await
        .expect("archiving reduces access");
}

#[tokio::test]
async fn revoking_a_membership_removes_it_from_every_group_for_good() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let w = world(&pool).await;
    let gone = w.membership("member_in");
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: w.fau.tenant_id,
            actor_membership_id: w.fau.admin_membership_id,
            membership_id: gone,
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        count(&pool, &format!("select count(*) from group_members where membership_id = '{gone}' and removed_at is null")).await,
        0
    );
    let removed = params(&pool, "group.member_removed").await;
    assert_eq!(removed.len(), 2, "open and closed");
    assert!(removed.iter().all(|p| p["cause"] == "membership_revoked"));

    // Re-invited, the same membership row reopens, but the closed group is not handed back.
    let back = add_member(
        &pool,
        &w.fau,
        "member-in@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 1), day(2027, 9, 1)),
        t0,
    )
    .await;
    assert_eq!(back.membership_id, gone);
    let mut conn = pool.acquire().await.unwrap();
    assert_eq!(
        authorize(
            &mut conn,
            w.viewer("member_in"),
            Resource::GroupContent(w.closed),
            Action::Read,
            t0
        )
        .await
        .unwrap(),
        Err(Denied::Hidden)
    );
}

async fn next(sub: &mut Subscription) -> Option<Resource> {
    tokio::time::timeout(Duration::from_secs(5), sub.recv())
        .await
        .expect("a delivery or a close within five seconds")
}

#[tokio::test]
async fn group_changes_reach_the_stream_and_a_removal_closes_it() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let w = world(&pool).await;
    let hub = Hub::start(
        db.app_pool().await,
        Arc::new(|| T0.parse::<jiff::Timestamp>().unwrap()),
    )
    .await
    .unwrap();
    let mut admin = hub.subscribe(w.viewer("admin_out")).await.unwrap();
    let mut outsider = hub.subscribe(w.viewer("member_out")).await.unwrap();
    let mut insider = hub.subscribe(w.viewer("member_in")).await.unwrap();

    let secret = create_group(
        &pool,
        create(
            w.fau.tenant_id,
            w.fau.admin_membership_id,
            Visibility::Closed,
            None,
        ),
        t0,
    )
    .await
    .unwrap();
    assert_eq!(next(&mut admin).await, Some(Resource::Group(secret)));

    remove_group_member(
        &pool,
        change(
            &w.fau,
            w.fau.admin_membership_id,
            w.open,
            w.membership("member_in"),
        ),
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        next(&mut insider).await,
        None,
        "removal from a group closes the stream"
    );

    assert_eq!(
        next(&mut outsider).await,
        Some(Resource::Group(w.open)),
        "the outsider drains the removal's own broadcast before the rename"
    );

    rename_group(&pool, rename(&w.fau, w.open), t0)
        .await
        .unwrap();
    assert_eq!(
        next(&mut outsider).await,
        Some(Resource::Group(w.open)),
        "the member outside the new closed group heard only of the open one"
    );
}
