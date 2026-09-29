//! How long the directory fields live (groups design §4.2 as amended by Erik's D3, 28
//! September 2026; §10; #3502): neither the name nor the contact address outlives the
//! membership, history shows an ended membership as its role and years, and an erasure
//! shows "Tidligere medlem".

mod common;
use common::groups::*;
use common::membership::*;
use common::TestDb;
use fau_crypto::Ciphertext;
use fau_domain::directory::history::HeldRole;
use fau_domain::membership::period::Period;
use fau_domain::membership::vocabulary::{CapabilityClass, Visibility};
use fau_persistence::membership::*;
use jiff::civil::Date;
use sqlx::PgPool;
use uuid::Uuid;

fn envelope(marker: u8) -> Ciphertext {
    let mut v = vec![1u8, marker];
    v.extend_from_slice(&[0u8; 40]);
    Ciphertext::from_stored(v)
}

async fn join(pool: &PgPool, fau: &Fau, address: &str, p: Period) -> Accepted {
    add_member(
        pool,
        fau,
        address,
        new_role("Medlem", CapabilityClass::Member),
        p,
        at(T0),
    )
    .await
}

async fn with_contact(pool: &PgPool, fau: &Fau, m: Uuid, marker: u8) {
    set_contact_email(
        pool,
        SetContactEmail {
            tenant_id: fau.tenant_id,
            membership_id: m,
            encrypted_contact_email: Some(envelope(marker)),
        },
        at(T0),
    )
    .await
    .unwrap();
}

async fn fields(pool: &PgPool, m: Uuid) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    sqlx::query_as(
        "select encrypted_display_name, encrypted_contact_email from memberships where id = $1",
    )
    .bind(m)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn name(pool: &PgPool, fau: &Fau, m: Uuid, at_: &str) -> MemberName {
    let viewer = Viewer {
        tenant_id: fau.tenant_id,
        membership_id: fau.admin_membership_id,
    };
    let names = member_names(pool, viewer, &[m], at(at_)).await.unwrap();
    assert_eq!(names.len(), 1);
    names.into_iter().next().unwrap().1
}

/// "Medlem", held from `from` up to (not including) `until`.
fn medlem(from: Date, until: Date) -> MemberName {
    MemberName::Ended(vec![HeldRole {
        name: "Medlem".into(),
        class: CapabilityClass::Member,
        from,
        until,
    }])
}

/// Mutation checks: drop `ra.tenant_id = m.tenant_id` from `membership_ended` and Kari's
/// running role in FAU B keeps her FAU A fields; drop `ra.revoked_at is null` and the
/// stepped-down member keeps theirs; make `member_names` ignore `ended` and the read before
/// the sweep returns the name.
#[tokio::test]
async fn the_sweep_clears_the_name_and_address_once_no_role_runs_or_is_still_to_come() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let a = active_fau(&pool, "admin-a@example.test", t0).await;
    let b = active_fau(&pool, "admin-b@example.test", t0).await;
    let short = period(day(2026, 9, 1), day(2026, 10, 1));
    let long = period(day(2026, 9, 1), day(2027, 9, 1));

    let ends = join(&pool, &a, "ends@example.test", short)
        .await
        .membership_id;
    // A name and no contact address: the sweep takes the name all the same.
    let quiet = join(&pool, &a, "quiet@example.test", short)
        .await
        .membership_id;
    let renewed = join(&pool, &a, "renewed@example.test", short)
        .await
        .membership_id;
    grant_role(
        &pool,
        GrantRole {
            tenant_id: a.tenant_id,
            actor_membership_id: a.admin_membership_id,
            membership_id: renewed,
            role: new_role("Neste år", CapabilityClass::Member),
            period: period(day(2026, 11, 1), day(2027, 11, 1)),
        },
        t0,
    )
    .await
    .unwrap();
    // The same person in two FAU-er: her role in A ends, her role in B runs on.
    let kari_a = join(&pool, &a, "kari@example.test", short)
        .await
        .membership_id;
    let kari_b = join(&pool, &b, "kari@example.test", long)
        .await
        .membership_id;
    // A role revoked early counts as ended.
    let stepped_down = join(&pool, &a, "down@example.test", long)
        .await
        .membership_id;
    for (fau, m, marker) in [
        (&a, ends, 1),
        (&a, renewed, 2),
        (&a, kari_a, 3),
        (&b, kari_b, 4),
        (&a, stepped_down, 5),
    ] {
        with_contact(&pool, fau, m, marker).await;
    }
    revoke(&pool, &a, live_assignment(&pool, stepped_down).await, t0).await;

    // While the short roles run, the sweep clears only the one whose role was revoked, and
    // history shows it as the role it held until the revocation day (T0, 23 September).
    assert_eq!(
        clear_ended_profiles(&pool, at("2026-09-30T10:00:00Z"))
            .await
            .unwrap(),
        1
    );
    assert_eq!(fields(&pool, stepped_down).await, (None, None));
    assert_eq!(
        name(&pool, &a, stepped_down, "2026-09-30T10:00:00Z").await,
        medlem(day(2026, 9, 1), day(2026, 9, 23))
    );
    assert!(fields(&pool, ends).await.0.is_some());

    // The day the short roles end, history already shows the role, before any sweep.
    let after = at("2026-10-01T10:00:00Z");
    assert!(fields(&pool, ends).await.0.is_some(), "not swept yet");
    assert_eq!(
        name(&pool, &a, ends, "2026-10-01T10:00:00Z").await,
        medlem(day(2026, 9, 1), day(2026, 10, 1))
    );

    // Then the sweep clears every membership without a running or future role.
    assert_eq!(clear_ended_profiles(&pool, after).await.unwrap(), 3);
    for gone in [ends, quiet, kari_a] {
        assert_eq!(fields(&pool, gone).await, (None, None));
    }
    assert_eq!(
        fields(&pool, renewed).await,
        (
            Some(placeholder_envelope().as_bytes().to_vec()),
            Some(envelope(2).as_bytes().to_vec())
        ),
        "a role still to come keeps both"
    );
    assert_eq!(
        fields(&pool, kari_b).await,
        (
            Some(placeholder_envelope().as_bytes().to_vec()),
            Some(envelope(4).as_bytes().to_vec())
        ),
        "another FAU's membership is its own"
    );
    assert_eq!(
        name(&pool, &a, renewed, "2026-10-01T10:00:00Z").await,
        MemberName::Named(placeholder_envelope())
    );
    // Idempotent.
    assert_eq!(clear_ended_profiles(&pool, after).await.unwrap(), 0);

    let causes: Vec<String> = sqlx::query_scalar(
        "select params->>'cause' from audit_events
          where action = 'membership.profile_cleared' and actor_kind = 'system'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(causes, ["membership_ended"; 4]);
}

#[tokio::test]
async fn erasure_shows_former_member_in_every_fau_even_after_the_membership_ended() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let a = active_fau(&pool, "admin-a@example.test", t0).await;
    let b = active_fau(&pool, "admin-b@example.test", t0).await;
    let year = period(day(2026, 9, 1), day(2027, 9, 1));
    let in_a = join(&pool, &a, "kari@example.test", year).await;
    let in_b = join(
        &pool,
        &b,
        "kari@example.test",
        period(day(2026, 9, 1), day(2026, 10, 1)),
    )
    .await;
    let ola = join(&pool, &a, "ola@example.test", year)
        .await
        .membership_id;
    with_contact(&pool, &a, in_a.membership_id, 1).await;
    // Before the erasure, Kari's ended membership in B shows its role and years.
    assert_eq!(
        name(&pool, &b, in_b.membership_id, "2026-10-02T10:00:00Z").await,
        medlem(day(2026, 9, 1), day(2026, 10, 1))
    );

    assert_eq!(
        erase_member_names(&pool, in_a.account_id, t0)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        name(&pool, &a, in_a.membership_id, T0).await,
        MemberName::Former
    );
    assert_eq!(
        name(&pool, &b, in_b.membership_id, "2026-10-02T10:00:00Z").await,
        MemberName::Former,
        "not even the role and year"
    );
    assert_eq!(fields(&pool, in_a.membership_id).await, (None, None));
    assert_eq!(
        name(&pool, &a, ola, T0).await,
        MemberName::Named(placeholder_envelope()),
        "nobody else's name"
    );
    assert_eq!(
        erase_member_names(&pool, in_a.account_id, t0)
            .await
            .unwrap(),
        0
    );
    let audits: Vec<String> = sqlx::query_scalar(
        "select params::text from audit_events where action = 'membership.name_erased'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(audits, ["{}", "{}"]);

    // A correction cannot bring the name back, and neither can a new acceptance.
    assert_eq!(
        set_display_name(
            &pool,
            SetDisplayName {
                tenant_id: a.tenant_id,
                actor_membership_id: a.admin_membership_id,
                membership_id: in_a.membership_id,
                encrypted_display_name: envelope(9),
            },
            t0,
        )
        .await,
        Err(MembershipError::MembershipErased)
    );
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: a.tenant_id,
            actor_membership_id: a.admin_membership_id,
            membership_id: in_a.membership_id,
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .unwrap();
    let token = issue_invitation(
        &pool,
        IssueInvitation {
            tenant_id: a.tenant_id,
            actor_membership_id: a.admin_membership_id,
            recipient: email("kari@example.test"),
            roles: vec![OfferedRole {
                role: new_role("Medlem", CapabilityClass::Member),
                period: year,
            }],
            handover_grant_id: None,
            message: None,
        },
        t0,
    )
    .await
    .unwrap()
    .token
    .expose()
    .to_owned();
    let profile = profile_for(&pool, &token, "kari@example.test", t0).await;
    assert_eq!(profile.membership_id, in_a.membership_id);
    assert_eq!(
        accept_invitation(
            &pool,
            AcceptInvitation {
                token,
                acceptor: verified("kari@example.test"),
                admin_end_override: None,
                profile,
            },
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::MembershipErased
    );
}

#[tokio::test]
async fn names_for_history_are_for_members_and_admins_of_that_fau_only() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let a = active_fau(&pool, "admin-a@example.test", t0).await;
    let b = active_fau(&pool, "admin-b@example.test", t0).await;
    let year = period(day(2026, 9, 1), day(2027, 9, 1));
    let kari = join(&pool, &a, "kari@example.test", year)
        .await
        .membership_id;
    // Active with no standing yet: every role is still to come, so the name applies.
    let next = join(
        &pool,
        &a,
        "neste@example.test",
        period(day(2026, 10, 1), day(2027, 9, 1)),
    )
    .await
    .membership_id;
    let in_b = join(&pool, &b, "ola@example.test", year)
        .await
        .membership_id;
    let group = seed_group(&pool, &a, Visibility::Open).await;
    let guest = add_guest(&pool, &a, "gjest@example.test", group, year, t0)
        .await
        .membership_id;

    let viewer = |m: Uuid| Viewer {
        tenant_id: a.tenant_id,
        membership_id: m,
    };
    // Another FAU's id, an unknown id and a repeat are left out.
    let names = member_names(
        &pool,
        viewer(kari),
        &[kari, in_b, Uuid::now_v7(), kari, next],
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        names,
        [
            (kari, MemberName::Named(placeholder_envelope())),
            (next, MemberName::Named(placeholder_envelope()))
        ]
    );
    assert_eq!(
        member_names(&pool, viewer(guest), &[kari], t0).await,
        Err(MembershipError::NotAuthorized)
    );
    assert_eq!(
        member_names(&pool, viewer(in_b), &[kari], t0).await,
        Err(MembershipError::NotAuthorized),
        "a viewer from another FAU"
    );
}
