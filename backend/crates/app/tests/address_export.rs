//! Handing over a selection's addresses for "Skriv e-post" or "Kopier adresser" (groups
//! design §4.3, §4.4, §10; #3502): each person once, only people the viewer's scope lists,
//! and an audit entry that holds the count and the scope, never the addresses.

mod common;
use common::groups::*;
use common::membership::*;
use common::TestDb;
use fau_domain::membership::vocabulary::Visibility;
use fau_persistence::membership::*;
use sqlx::PgPool;
use uuid::Uuid;

async fn exported(pool: &PgPool) -> i64 {
    count(
        pool,
        "select count(*) from audit_events where action = 'directory.addresses_exported'",
    )
    .await
}

async fn last_params(pool: &PgPool) -> String {
    sqlx::query_scalar(
        "select params::text from audit_events where action = 'directory.addresses_exported'
          order by occurred_at desc, id desc limit 1",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

fn export(purpose: ExportPurpose, scope: ExportScope, ids: &[Uuid]) -> AddressExport {
    AddressExport {
        purpose,
        scope,
        membership_ids: ids.to_vec(),
    }
}

#[tokio::test]
async fn each_person_comes_back_once_and_the_audit_holds_only_the_count_and_the_scope() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let (admin, member, guest) = (
        w.membership("admin_in"),
        w.membership("member_in"),
        w.membership("guest_in"),
    );

    // Admin ticked in two groups: once, in first-selected order.
    let got = export_addresses(
        &pool,
        w.viewer("member_out"),
        export(
            ExportPurpose::Mailto,
            ExportScope::All,
            &[admin, member, admin, guest],
        ),
        at(T0),
    )
    .await
    .unwrap();
    let ids: Vec<Uuid> = got.iter().map(|r| r.membership_id).collect();
    assert_eq!(ids, [admin, member, guest]);
    assert_eq!(
        got[2].address,
        DirectoryAddress::Login(email("guest-in@example.test")),
        "a member sees a guest's address inside a group they can read"
    );

    let params = last_params(&pool).await;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&params).unwrap(),
        serde_json::json!({
            "purpose": "mailto", "recipient_count": 3, "scope": "all", "group_id": null
        })
    );
    assert!(!params.contains('@'), "no address");
    for id in [admin, member, guest] {
        assert!(!params.contains(&id.to_string()), "nobody's id");
    }

    export_addresses(
        &pool,
        w.viewer("member_out"),
        export(ExportPurpose::Copy, ExportScope::Group(w.open), &[guest]),
        at(T0),
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&last_params(&pool).await).unwrap(),
        serde_json::json!({
            "purpose": "copy", "recipient_count": 1, "scope": "group", "group_id": w.open
        })
    );
    assert_eq!(exported(&pool).await, 2);
}

/// Every refusal writes nothing, and comes in a fixed order, so no answer depends on
/// whether a hidden id exists. Mutation check: skip the scope check and the `member_out` /
/// `closed` row passes; check memberships before the scope and the unknown-group rows
/// answer `UnknownMembership`.
#[tokio::test]
async fn a_selection_outside_what_the_viewer_sees_is_refused_and_writes_nothing() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let other_fau = active_fau(&pool, "admin-b@example.test", at(T0)).await;
    let m = |n: &str| w.membership(n);
    let unknown = Uuid::now_v7();
    let rows: Vec<(&str, ExportScope, Vec<Uuid>, MembershipError)> = vec![
        (
            "member_out",
            ExportScope::All,
            vec![],
            MembershipError::EmptySelection,
        ),
        // Final review M5: input validation comes before authority, so a viewer with no
        // standing and an empty selection learns only that the selection is empty.
        (
            "none_in",
            ExportScope::All,
            vec![],
            MembershipError::EmptySelection,
        ),
        (
            "none_in",
            ExportScope::All,
            vec![m("admin_in")],
            MembershipError::NotAuthorized,
        ),
        (
            "guest_in",
            ExportScope::Fau,
            vec![m("admin_in")],
            MembershipError::NotAuthorized,
        ),
        (
            "member_out",
            ExportScope::Group(w.closed),
            vec![m("member_in")],
            MembershipError::UnknownGroup,
        ),
        (
            "member_out",
            ExportScope::Group(unknown),
            vec![m("member_in")],
            MembershipError::UnknownGroup,
        ),
        (
            "member_out",
            ExportScope::Group(w.other),
            vec![unknown],
            MembershipError::UnknownGroup,
        ),
        // guest_out is in `other`, which member_out cannot read.
        (
            "member_out",
            ExportScope::All,
            vec![m("guest_out")],
            MembershipError::UnknownMembership,
        ),
        (
            "member_out",
            ExportScope::Group(w.open),
            vec![m("guest_in"), m("member_out")],
            MembershipError::UnknownMembership,
        ),
        // A guest never reaches the FAU-wide section's people.
        (
            "guest_in",
            ExportScope::All,
            vec![m("member_out")],
            MembershipError::UnknownMembership,
        ),
        (
            "guest_in",
            ExportScope::All,
            vec![unknown],
            MembershipError::UnknownMembership,
        ),
        (
            "admin_out",
            ExportScope::All,
            vec![other_fau.admin_membership_id],
            MembershipError::UnknownMembership,
        ),
        // A person with no standing today is not listed, even though still in the group.
        (
            "admin_out",
            ExportScope::Group(w.open),
            vec![m("none_in")],
            MembershipError::UnknownMembership,
        ),
    ];
    for (viewer, scope, ids, want) in rows {
        let got = export_addresses(
            &pool,
            w.viewer(viewer),
            export(ExportPurpose::Copy, scope, &ids),
            at(T0),
        )
        .await;
        assert_eq!(got, Err(want.clone()), "{viewer} {scope:?}");
    }
    assert_eq!(exported(&pool).await, 0);

    // The same person from another FAU's viewer: refused as having no standing here.
    let crossed = Viewer {
        tenant_id: w.fau.tenant_id,
        membership_id: other_fau.admin_membership_id,
    };
    assert_eq!(
        export_addresses(
            &pool,
            crossed,
            export(ExportPurpose::Copy, ExportScope::All, &[m("admin_in")]),
            at(T0),
        )
        .await,
        Err(MembershipError::NotAuthorized)
    );
    assert_eq!(
        exported(&pool).await,
        0,
        "the crossed-tenant call wrote nothing"
    );
}

#[tokio::test]
async fn a_frozen_fau_still_hands_over_addresses() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    sqlx::query("update tenants set frozen_at = now() where id = $1")
        .bind(w.fau.tenant_id)
        .execute(&pool)
        .await
        .unwrap();
    let got = export_addresses(
        &pool,
        w.viewer("member_in"),
        export(
            ExportPurpose::Mailto,
            ExportScope::Fau,
            &[w.membership("admin_out")],
        ),
        at(T0),
    )
    .await
    .unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(exported(&pool).await, 1);
    // Final review M5: the FAU-wide scope is audited as "fau".
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&last_params(&pool).await).unwrap(),
        serde_json::json!({
            "purpose": "mailto", "recipient_count": 1, "scope": "fau", "group_id": null
        })
    );
}

/// Final review M5: the positive control for a guest. `guest_in` exports from `closed`, a
/// group they are in, and gets the one member they picked, audited with the group's id.
#[tokio::test]
async fn a_guest_exports_from_their_own_group() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let member = w.membership("member_in");
    let got = export_addresses(
        &pool,
        w.viewer("guest_in"),
        export(ExportPurpose::Copy, ExportScope::Group(w.closed), &[member]),
        at(T0),
    )
    .await
    .unwrap();
    assert_eq!(
        got,
        [ExportedRecipient {
            membership_id: member,
            address: DirectoryAddress::Login(email("member-in@example.test")),
        }]
    );
    assert_eq!(exported(&pool).await, 1);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&last_params(&pool).await).unwrap(),
        serde_json::json!({
            "purpose": "copy", "recipient_count": 1, "scope": "group", "group_id": w.closed
        })
    );
}

/// Q4 (controller ruling): a guest reaching for a group outside their own groups is
/// refused exactly as an unknown group, and nothing is audited. `guest_in` is a member of
/// `open` (their guest role) and `closed` (hand-added), but never `other`.
#[tokio::test]
async fn a_guest_outside_the_group_is_refused_and_writes_nothing() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let got = export_addresses(
        &pool,
        w.viewer("guest_in"),
        export(
            ExportPurpose::Copy,
            ExportScope::Group(w.other),
            &[w.membership("guest_out")],
        ),
        at(T0),
    )
    .await;
    assert_eq!(got, Err(MembershipError::UnknownGroup));
    assert_eq!(exported(&pool).await, 0);
}

/// Q4 (controller ruling): a viewer whose account is disabled has no standing at all, so
/// the refusal comes before the scope is ever read, and nothing is audited.
#[tokio::test]
async fn a_disabled_account_is_refused_and_writes_nothing() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    sqlx::query(
        "update accounts set disabled_at = now()
          where id = (select account_id from memberships where tenant_id = $1 and id = $2)",
    )
    .bind(w.fau.tenant_id)
    .bind(w.membership("member_in"))
    .execute(&pool)
    .await
    .unwrap();
    let got = export_addresses(
        &pool,
        w.viewer("member_in"),
        export(
            ExportPurpose::Copy,
            ExportScope::All,
            &[w.membership("admin_in")],
        ),
        at(T0),
    )
    .await;
    assert_eq!(got, Err(MembershipError::NotAuthorized));
    assert_eq!(exported(&pool).await, 0);
}

/// Q6 (controller ruling, spec §4.1): the directory shows the present, so an archived
/// group is not a valid export scope -- refused exactly as an unknown group, even for the
/// admin who could manage it, and even naming someone who was a real member. Positive
/// control: the same request against the group before it is archived succeeds.
#[tokio::test]
async fn an_archived_group_is_refused_as_unknown_and_writes_nothing() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let admin = w.viewer("admin_in");
    let before = export_addresses(
        &pool,
        admin,
        export(
            ExportPurpose::Copy,
            ExportScope::Group(w.open),
            &[w.membership("member_in")],
        ),
        at(T0),
    )
    .await;
    assert!(
        before.is_ok(),
        "positive control: open, unarchived, succeeds"
    );
    assert_eq!(exported(&pool).await, 1);

    archive_group(
        &pool,
        ArchiveGroup {
            tenant_id: w.fau.tenant_id,
            actor_membership_id: w.fau.admin_membership_id,
            group_id: w.open,
        },
        at(T0),
    )
    .await
    .unwrap();

    let after = export_addresses(
        &pool,
        admin,
        export(
            ExportPurpose::Copy,
            ExportScope::Group(w.open),
            &[w.membership("member_in")],
        ),
        at(T0),
    )
    .await;
    assert_eq!(after, Err(MembershipError::UnknownGroup));
    assert_eq!(
        exported(&pool).await,
        1,
        "the archived attempt wrote nothing"
    );
}

/// A group id from another FAU is refused exactly as an unknown group: the tenant
/// boundary hides it the same way a made-up id does.
#[tokio::test]
async fn a_group_id_from_another_fau_is_refused_as_unknown() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let other = active_fau(&pool, "admin-c@example.test", at(T0)).await;
    let foreign_group = seed_group(&pool, &other, Visibility::Open).await;

    let got = export_addresses(
        &pool,
        w.viewer("member_out"),
        export(
            ExportPurpose::Copy,
            ExportScope::Group(foreign_group),
            &[w.membership("member_in")],
        ),
        at(T0),
    )
    .await;
    assert_eq!(got, Err(MembershipError::UnknownGroup));
    assert_eq!(exported(&pool).await, 0);
}
