//! Remembering a former member for a chosen period (#3511, docs/member-retention-design.md
//! §6). Ending keeps the name and address hidden for the account's `retention_months`;
//! the sweep clears them after; others see role and year throughout.

mod common;
use common::groups::seed_group;
use common::membership::*;
use common::TestDb;
use fau_crypto::Ciphertext;
use fau_domain::membership::period::Period;
use fau_domain::membership::vocabulary::{CapabilityClass, RoleName, Visibility};
use fau_persistence::membership::*;
use sqlx::PgPool;
use uuid::Uuid;

fn envelope(marker: u8) -> Ciphertext {
    let mut v = vec![1u8, marker];
    v.extend_from_slice(&[0u8; 40]);
    Ciphertext::from_stored(v)
}

async fn join(pool: &PgPool, fau: &Fau, address: &str, p: Period) -> Uuid {
    add_member(
        pool,
        fau,
        address,
        new_role("Medlem", CapabilityClass::Member),
        p,
        at(T0),
    )
    .await
    .membership_id
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

/// (name present, address present, profile_retained_until as text)
async fn state(pool: &PgPool, m: Uuid) -> (bool, bool, Option<String>) {
    sqlx::query_as(
        "select encrypted_display_name is not null, encrypted_contact_email is not null,
                to_char(profile_retained_until, 'YYYY-MM-DD')
           from memberships where id = $1",
    )
    .bind(m)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn set_months_by_sql(pool: &PgPool, address: &str, months: i32) {
    sqlx::query("update accounts set retention_months = $2 where email = $1")
        .bind(address)
        .bind(months)
        .execute(pool)
        .await
        .unwrap();
}

async fn leave(pool: &PgPool, fau: &Fau, m: Uuid, at_: &str) {
    revoke_membership(
        pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: m,
            membership_id: m,
            confirm_no_admin: false,
        },
        at(at_),
    )
    .await
    .unwrap();
}

async fn audits(pool: &PgPool, m: Uuid) -> Vec<(String, String)> {
    sqlx::query_as(
        "select action, params::text from audit_events
          where subject_id = $1
            and action in ('membership.profile_retained', 'membership.profile_cleared',
                           'membership.profile_restored', 'membership.retention_changed')
          order by occurred_at, id",
    )
    .bind(m)
    .fetch_all(pool)
    .await
    .unwrap()
}

fn year() -> Period {
    period(day(2026, 9, 1), day(2027, 9, 1))
}

/// Spec §6 "ending with each period value". Leaving on 2026-10-01 stamps
/// `ended_on + months`, or clears at once for 0.
///
/// Mutation check: clear unconditionally in `revoke_membership` (0008's behaviour) and
/// every non-zero row fails.
#[tokio::test]
async fn leaving_keeps_both_fields_for_the_chosen_period_or_clears_at_once_for_none() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    for (months, expected) in [
        (0, None),
        (3, Some("2027-01-01")),
        (6, Some("2027-04-01")),
        (12, Some("2027-10-01")),
        (24, Some("2028-10-01")),
    ] {
        let address = format!("p{months}@example.test");
        let m = join(&pool, &a, &address, year()).await;
        with_contact(&pool, &a, m, 1).await;
        set_months_by_sql(&pool, &address, months).await;
        leave(&pool, &a, m, "2026-10-01T10:00:00Z").await;
        let kept = expected.is_some();
        assert_eq!(
            state(&pool, m).await,
            (kept, kept, expected.map(str::to_owned)),
            "{months} months"
        );
        let expected_audit = match expected {
            Some(d) => (
                "membership.profile_retained".to_owned(),
                format!(r#"{{"until": "{d}"}}"#),
            ),
            None => (
                "membership.profile_cleared".to_owned(),
                r#"{"cause": "membership_ended"}"#.to_owned(),
            ),
        };
        assert_eq!(audits(&pool, m).await, [expected_audit], "{months} months");
    }
}

/// Spec §6 "the sweep before and after the period ends", and a natural end: roles that
/// ran out are stamped by the first sweep, from the day they ran out, not the sweep's day.
///
/// Mutation check: stamp from `at.today()` instead of `ended_on` and the natural-end
/// date comes out 2027-01-15.
#[tokio::test]
async fn the_sweep_stamps_a_natural_end_from_the_day_it_ended_and_clears_after_the_period() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let ran_out = join(
        &pool,
        &a,
        "ranout@example.test",
        period(day(2026, 9, 1), day(2026, 10, 1)),
    )
    .await;
    let left = join(&pool, &a, "left@example.test", year()).await;
    with_contact(&pool, &a, ran_out, 1).await;
    leave(&pool, &a, left, "2026-10-01T10:00:00Z").await;

    // Two weeks after the natural end: the sweep stamps, clears nothing.
    assert_eq!(
        clear_ended_profiles(&pool, at("2026-10-15T10:00:00Z"))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        state(&pool, ran_out).await,
        (true, true, Some("2027-01-01".into()))
    );
    // The day before the period ends: still kept, and idempotent.
    assert_eq!(
        clear_ended_profiles(&pool, at("2026-12-31T10:00:00Z"))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        state(&pool, left).await,
        (true, false, Some("2027-01-01".into()))
    );
    // The day it ends: both cleared, with the date.
    assert_eq!(
        clear_ended_profiles(&pool, at("2027-01-01T10:00:00Z"))
            .await
            .unwrap(),
        2
    );
    for m in [ran_out, left] {
        assert_eq!(state(&pool, m).await, (false, false, None));
        assert_eq!(
            audits(&pool, m).await.last().unwrap(),
            &(
                "membership.profile_cleared".to_owned(),
                r#"{"cause": "retention_ended"}"#.to_owned()
            )
        );
    }
    assert_eq!(
        clear_ended_profiles(&pool, at("2027-01-02T10:00:00Z"))
            .await
            .unwrap(),
        0
    );
}

/// Spec §6 "retention_months = 0": a natural end with a period of none is cleared by the
/// first sweep, as under D3.
#[tokio::test]
async fn a_period_of_none_is_cleared_by_the_first_sweep() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let m = join(
        &pool,
        &a,
        "none@example.test",
        period(day(2026, 9, 1), day(2026, 10, 1)),
    )
    .await;
    set_months_by_sql(&pool, "none@example.test", 0).await;
    assert_eq!(
        clear_ended_profiles(&pool, at("2026-10-01T10:00:00Z"))
            .await
            .unwrap(),
        1
    );
    assert_eq!(state(&pool, m).await, (false, false, None));
}

/// M1 while retained: others see role and year. Neither the directory nor `member_names`
/// gives the retained name.
///
/// Mutation check: make `member_names` return `Named` for a row with a name regardless of
/// `ended` and the second assertion fails.
#[tokio::test]
async fn while_retained_others_see_only_role_and_year() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let m = join(&pool, &a, "kept@example.test", year()).await;
    leave(&pool, &a, m, "2026-10-01T10:00:00Z").await;
    assert_eq!(state(&pool, m).await.2.as_deref(), Some("2027-01-01"));
    let viewer = Viewer {
        tenant_id: a.tenant_id,
        membership_id: a.admin_membership_id,
    };
    let later = at("2026-11-01T10:00:00Z");
    let names = member_names(&pool, viewer, &[m], later).await.unwrap();
    assert!(
        matches!(names[0].1, MemberName::Ended(_)),
        "{:?}",
        names[0].1
    );
    let dir = member_directory(&pool, viewer, later).await.unwrap();
    assert!(dir.people.iter().all(|p| p.membership_id != m));
}

/// Spec §6 "erasure during retention": everything at once, and the date with it.
#[tokio::test]
async fn an_erasure_during_retention_clears_everything_at_once() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let m = join(&pool, &a, "erase@example.test", year()).await;
    with_contact(&pool, &a, m, 1).await;
    leave(&pool, &a, m, "2026-10-01T10:00:00Z").await;
    let account: Uuid = sqlx::query_scalar("select account_id from memberships where id = $1")
        .bind(m)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        erase_member_names(&pool, account, at("2026-10-02T10:00:00Z"))
            .await
            .unwrap(),
        1
    );
    assert_eq!(state(&pool, m).await, (false, false, None));
}

/// Spec §6 "cross-tenant isolation": one person in two FAU-er. Leaving A retains in A only;
/// the sweep that clears A leaves B's active membership alone.
///
/// Mutation check: drop `m.tenant_id = $2` from the sweep's per-tenant select and A's pass
/// picks up B's ended row (`ola_b`), which `settle_profile` cannot find under A's id: the
/// sweep fails. An active row is never a candidate, so `ola_b` is what makes the filter
/// observable; B's active fields must survive as well.
#[tokio::test]
async fn retention_is_per_fau() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin-a@example.test", at(T0)).await;
    let b = active_fau(&pool, "admin-b@example.test", at(T0)).await;
    let in_a = join(&pool, &a, "kari@example.test", year()).await;
    let in_b = join(&pool, &b, "kari@example.test", year()).await;
    with_contact(&pool, &b, in_b, 2).await;
    let ola_b = join(
        &pool,
        &b,
        "ola@example.test",
        period(day(2026, 9, 1), day(2026, 10, 1)),
    )
    .await;
    leave(&pool, &a, in_a, "2026-10-01T10:00:00Z").await;
    assert_eq!(state(&pool, in_a).await.2.as_deref(), Some("2027-01-01"));
    assert_eq!(state(&pool, in_b).await, (true, true, None));
    assert_eq!(
        clear_ended_profiles(&pool, at("2027-01-01T10:00:00Z"))
            .await
            .unwrap(),
        2
    );
    assert_eq!(state(&pool, in_a).await, (false, false, None));
    assert_eq!(state(&pool, ola_b).await, (false, false, None));
    assert_eq!(state(&pool, in_b).await, (true, true, None));
}

/// Plan Ruling R9: a revocation that finds the period already over (roles ran out on
/// 2026-10-01, the default 3 months ended 2027-01-01, and no sweep ran) clears with
/// `retention_ended`, as `settle_profile` would. Only a period of none is `membership_ended`.
///
/// Mutation check: always write `membership_ended` in `revoke_membership` and this fails.
#[tokio::test]
async fn a_revocation_after_the_period_is_over_clears_with_retention_ended() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let m = join(
        &pool,
        &a,
        "late@example.test",
        period(day(2026, 9, 1), day(2026, 10, 1)),
    )
    .await;
    with_contact(&pool, &a, m, 1).await;
    leave(&pool, &a, m, "2027-02-01T10:00:00Z").await;
    assert_eq!(state(&pool, m).await, (false, false, None));
    assert_eq!(
        audits(&pool, m).await,
        [(
            "membership.profile_cleared".to_owned(),
            r#"{"cause": "retention_ended"}"#.to_owned()
        )]
    );
}

async fn reinvite(
    pool: &PgPool,
    fau: &Fau,
    address: &str,
    role: RoleChoice,
    p: Period,
    at_: &str,
) -> (bool, Uuid) {
    let issued = issue_invitation(
        pool,
        IssueInvitation {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            recipient: email(address),
            roles: vec![OfferedRole { role, period: p }],
            handover_grant_id: None,
            message: None,
        },
        at(at_),
    )
    .await
    .unwrap();
    let token = issued.token.expose().to_owned();
    let target = prepare_acceptance(pool, &token, &verified(address), at(at_))
        .await
        .unwrap();
    let accepted = accept_invitation(
        pool,
        AcceptInvitation {
            token,
            acceptor: verified(address),
            admin_end_override: None,
            // A returner states a name again; the address is left unstated.
            profile: MemberProfile {
                membership_id: target.membership_id,
                encrypted_display_name: envelope(9),
                encrypted_contact_email: None,
            },
        },
        at(at_),
    )
    .await
    .unwrap();
    (target.existing_current, accepted.membership_id)
}

async fn contact(pool: &PgPool, m: Uuid) -> Option<Vec<u8>> {
    sqlx::query_scalar("select encrypted_contact_email from memberships where id = $1")
        .bind(m)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Spec §6 "a return within the period": the same row, recognised (`existing_current`),
/// the retained address kept, the date cleared, `profile_restored` audited. Returning as
/// a guest, the person sees only their own group (spec §6 "a guest who returns").
///
/// Mutation checks: drop the `settle` in `reopen_profile` and an expired return keeps the
/// old address (the next test); drop the `profile_retained_until = null` and 0009's
/// checks are fine but the returner stays stamped ("no longer retained" below). Report
/// the retained row as not current in `prepare_acceptance` or `ensure_membership` and
/// `recognised` fails.
#[tokio::test]
async fn a_guest_returning_within_the_period_is_recognised_and_sees_only_their_group() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let m = join(&pool, &a, "back@example.test", year()).await;
    with_contact(&pool, &a, m, 1).await;
    leave(&pool, &a, m, "2026-10-01T10:00:00Z").await;

    let group = seed_group(&pool, &a, Visibility::Open).await;
    // A second open group the guest must not reach, so "only their own" can fail.
    let _other = seed_group(&pool, &a, Visibility::Open).await;
    let guest = RoleChoice::New {
        name: RoleName::parse("Gjest").unwrap(),
        capability: CapabilityClass::Guest,
        group_id: Some(group),
    };
    let (recognised, again) = reinvite(
        &pool,
        &a,
        "back@example.test",
        guest,
        period(day(2026, 11, 1), day(2027, 2, 1)),
        "2026-11-01T10:00:00Z",
    )
    .await;
    assert!(recognised, "within the period: existing_current");
    assert_eq!(again, m, "the same row");
    assert_eq!(
        contact(&pool, m).await,
        Some(envelope(1).as_bytes().to_vec()),
        "address kept"
    );
    assert_eq!(state(&pool, m).await.2, None, "no longer retained");
    assert!(audits(&pool, m)
        .await
        .iter()
        .any(|(a, _)| a == "membership.profile_restored"));
    // The sweep after the old period's end leaves an active returner alone.
    clear_ended_profiles(&pool, at("2027-01-05T10:00:00Z"))
        .await
        .unwrap();
    assert_eq!(state(&pool, m).await, (true, true, None));
    // A guest reaches only their own group.
    let viewer = Viewer {
        tenant_id: a.tenant_id,
        membership_id: m,
    };
    let groups = list_groups(&pool, viewer, at("2026-11-02T10:00:00Z"))
        .await
        .unwrap();
    assert_eq!(
        groups.iter().map(|g| g.group_id).collect::<Vec<_>>(),
        [group]
    );
}

/// Spec §6 "a return after the period": expired first, so nothing comes back and the
/// person is not `existing_current`.
#[tokio::test]
async fn returning_after_the_period_is_a_fresh_start() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let m = join(&pool, &a, "late@example.test", year()).await;
    with_contact(&pool, &a, m, 1).await;
    leave(&pool, &a, m, "2026-10-01T10:00:00Z").await;
    // No sweep has run since the period ended on 2027-01-01.
    let (recognised, again) = reinvite(
        &pool,
        &a,
        "late@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2027, 2, 1), day(2028, 2, 1)),
        "2027-02-01T10:00:00Z",
    )
    .await;
    assert!(!recognised);
    assert_eq!(again, m);
    assert_eq!(contact(&pool, m).await, None, "the expired address is gone");
    assert!(audits(&pool, m)
        .await
        .iter()
        .any(|(a, p)| a == "membership.profile_cleared" && p.contains("retention_ended")));
}

/// `grant_role` to a membership whose roles ran out: restore if retained, clear if expired
/// (spec §3, replacing #3502's clear_if_ended).
#[tokio::test]
async fn a_grant_restores_a_retained_profile_and_clears_an_expired_one() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let short = period(day(2026, 9, 1), day(2026, 10, 1));
    let kept = join(&pool, &a, "kept@example.test", short).await;
    let gone = join(&pool, &a, "gone@example.test", short).await;
    for m in [kept, gone] {
        with_contact(&pool, &a, m, 1).await;
    }
    let grant_at = |m: Uuid, at_: &'static str, p: Period| {
        let pool = pool.clone();
        let tenant_id = a.tenant_id;
        let admin = a.admin_membership_id;
        async move {
            grant_role(
                &pool,
                GrantRole {
                    tenant_id,
                    actor_membership_id: admin,
                    membership_id: m,
                    role: new_role("Nytt verv", CapabilityClass::Member),
                    period: p,
                },
                at(at_),
            )
            .await
            .unwrap();
        }
    };
    grant_at(
        kept,
        "2026-11-01T10:00:00Z",
        period(day(2026, 11, 1), day(2027, 11, 1)),
    )
    .await;
    assert_eq!(state(&pool, kept).await, (true, true, None));
    grant_at(
        gone,
        "2027-02-01T10:00:00Z",
        period(day(2027, 2, 1), day(2028, 2, 1)),
    )
    .await;
    assert_eq!(state(&pool, gone).await, (false, false, None));
}

/// M1 and spec §6 "history labels for events from an earlier active period after a
/// return": the returner is `Returned`; an event from the old period labels as the role
/// and year, an event from the new one as the name. A member with one unbroken period
/// stays `Named`.
///
/// Mutation check: return `Named` for every active row (0008's behaviour) and the first
/// assertion fails.
#[tokio::test]
async fn after_a_return_old_events_keep_role_and_year_and_new_ones_show_the_name() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let m = join(&pool, &a, "back@example.test", year()).await;
    leave(&pool, &a, m, "2026-10-01T10:00:00Z").await;
    reinvite(
        &pool,
        &a,
        "back@example.test",
        new_role("Sekretær", CapabilityClass::Member),
        period(day(2026, 11, 1), day(2027, 11, 1)),
        "2026-11-01T10:00:00Z",
    )
    .await;
    let viewer = Viewer {
        tenant_id: a.tenant_id,
        membership_id: a.admin_membership_id,
    };
    let names = member_names(
        &pool,
        viewer,
        &[m, a.admin_membership_id],
        at("2026-11-02T10:00:00Z"),
    )
    .await
    .unwrap();
    let MemberName::Returned { name, period: p } = &names[0].1 else {
        panic!("{:?}", names[0].1)
    };
    assert_eq!(name, &envelope(9));
    assert_eq!(p.since, day(2026, 11, 1));
    let old = p.label_on(day(2026, 9, 15)).expect("role and year");
    assert_eq!(
        (old.role.as_str(), old.first_year, old.last_year),
        ("Medlem", 2026, 2026)
    );
    assert_eq!(
        p.label_on(day(2026, 11, 1)),
        None,
        "the new period shows the name"
    );
    assert!(
        matches!(names[1].1, MemberName::Named(_)),
        "one unbroken period"
    );
}
