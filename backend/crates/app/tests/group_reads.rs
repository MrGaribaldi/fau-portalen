//! Group reads (groups design §3.3): a list holds exactly the groups `authorize` lets the
//! viewer read (Ruling R11), a hidden group is not found, and a member list follows the
//! group's own visibility (R22).

mod common;
use common::groups::*;
use common::membership::*;
use common::TestDb;
use fau_crypto::Ciphertext;
use fau_domain::authz::Action;
use fau_domain::membership::period::Period;
use fau_domain::membership::vocabulary::{CapabilityClass, Visibility};
use fau_persistence::membership::{
    authorize, get_group, list_group_members, list_groups, GroupMemberView, MembershipError,
    Resource, RoleChoice, Viewer,
};
use sqlx::PgPool;
use uuid::Uuid;

#[tokio::test]
async fn a_list_holds_exactly_what_authorize_lets_the_viewer_read() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let t0 = at(T0);
    let mut conn = pool.acquire().await.unwrap();
    for name in VIEWERS {
        let viewer = w.viewer(name);
        let listed = list_groups(&pool, viewer, t0).await;
        if name.starts_with("none") {
            assert_eq!(
                listed.unwrap_err(),
                MembershipError::NotAuthorized,
                "{name}"
            );
            continue;
        }
        let mut listed: Vec<Uuid> = listed.unwrap().iter().map(|g| g.group_id).collect();
        listed.sort_unstable();
        let mut readable = Vec::new();
        for g in [w.open, w.closed, w.other] {
            if authorize(&mut conn, viewer, Resource::Group(g), Action::Read, t0)
                .await
                .unwrap()
                .is_ok()
            {
                readable.push(g);
            }
        }
        readable.sort_unstable();
        assert_eq!(listed, readable, "{name}");
    }

    assert_eq!(
        ids(&pool, &w, "admin_out").await,
        sorted(vec![w.open, w.closed, w.other])
    );
    assert_eq!(ids(&pool, &w, "member_out").await, vec![w.open]);
    assert_eq!(
        ids(&pool, &w, "guest_in").await,
        sorted(vec![w.open, w.closed])
    );
    assert_eq!(ids(&pool, &w, "guest_out").await, vec![w.other]);
}

async fn ids(pool: &PgPool, w: &World, name: &str) -> Vec<Uuid> {
    let listed = list_groups(pool, w.viewer(name), at(T0)).await.unwrap();
    sorted(listed.iter().map(|g| g.group_id).collect())
}

fn sorted(mut v: Vec<Uuid>) -> Vec<Uuid> {
    v.sort_unstable();
    v
}

async fn join(pool: &PgPool, fau: &Fau, address: &str, role: RoleChoice, p: Period) -> Uuid {
    add_member(pool, fau, address, role, p, at(T0))
        .await
        .membership_id
}

#[tokio::test]
async fn a_hidden_group_is_not_found_and_a_visible_one_carries_its_facts() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let t0 = at(T0);
    for group in [w.closed, Uuid::now_v7()] {
        assert_eq!(
            get_group(&pool, w.viewer("member_out"), group, t0)
                .await
                .unwrap_err(),
            MembershipError::UnknownGroup
        );
    }
    let seen = get_group(&pool, w.viewer("guest_in"), w.closed, t0)
        .await
        .unwrap();
    assert_eq!(seen.group_id, w.closed);
    assert_eq!(
        seen.encrypted_name,
        Ciphertext::from_stored(placeholder_name())
    );
    assert_eq!(seen.visibility, Visibility::Closed);
    assert!(!seen.archived && seen.binding.is_none() && seen.viewer_in_group);
    let admin_view = get_group(&pool, w.viewer("admin_out"), w.closed, t0)
        .await
        .unwrap();
    assert!(
        !admin_view.viewer_in_group,
        "an admin reads a group without being in it"
    );
}

#[tokio::test]
async fn a_member_list_shows_role_holders_and_hand_added_members_with_standing() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let unit = seed_unit(&pool, fau.tenant_id).await;
    let unit_role = seed_role(
        &pool,
        fau.tenant_id,
        CapabilityClass::Member,
        None,
        Some(unit),
        None,
    )
    .await;
    let group = seed_bound_group(&pool, &fau, Visibility::Closed, Some(unit), None).await;
    let year = period(day(2026, 9, 1), day(2027, 9, 1));
    let by_role = || RoleChoice::Existing(unit_role);

    let holder = join(&pool, &fau, "rolle@example.test", by_role(), year).await;
    let by_hand = join(
        &pool,
        &fau,
        "hand@example.test",
        new_role("Medlem", CapabilityClass::Member),
        year,
    )
    .await;
    let both = join(&pool, &fau, "begge@example.test", by_role(), year).await;
    let short = join(
        &pool,
        &fau,
        "kort@example.test",
        by_role(),
        period(day(2026, 9, 1), day(2026, 10, 1)),
    )
    .await;
    let revoked = join(&pool, &fau, "borte@example.test", by_role(), year).await;
    seed_group_member(&pool, &fau, group, by_hand).await;
    seed_group_member(&pool, &fau, group, both).await;
    revoke(&pool, &fau, live_assignment(&pool, revoked).await, t0).await;

    let admin = Viewer {
        tenant_id: fau.tenant_id,
        membership_id: fau.admin_membership_id,
    };
    let mut want = vec![
        GroupMemberView {
            membership_id: holder,
            added_by_hand: false,
            through_role: true,
        },
        GroupMemberView {
            membership_id: by_hand,
            added_by_hand: true,
            through_role: false,
        },
        GroupMemberView {
            membership_id: both,
            added_by_hand: true,
            through_role: true,
        },
        GroupMemberView {
            membership_id: short,
            added_by_hand: false,
            through_role: true,
        },
    ];
    want.sort_by_key(|m| m.membership_id);
    assert_eq!(
        list_group_members(&pool, admin, group, t0).await.unwrap(),
        want
    );

    // On 1 October the short role has ended: no job ran, and the list no longer holds it.
    want.retain(|m| m.membership_id != short);
    assert_eq!(
        list_group_members(&pool, admin, group, at("2026-10-01T10:00:00Z"))
            .await
            .unwrap(),
        want
    );

    // A member outside the closed group cannot see who is in it, or that it exists.
    let outsider = Viewer {
        tenant_id: fau.tenant_id,
        membership_id: by_hand,
    };
    sqlx::query("update group_members set removed_at = $1::timestamptz where membership_id = $2")
        .bind(T0)
        .bind(by_hand)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        list_group_members(&pool, outsider, group, t0)
            .await
            .unwrap_err(),
        MembershipError::UnknownGroup
    );
}
