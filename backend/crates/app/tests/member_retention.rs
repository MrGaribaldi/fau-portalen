//! Remembering a former member for a chosen period (#3511, docs/member-retention-design.md
//! §6). Ending keeps the name and address hidden for the account's `retention_months`;
//! the sweep clears them after; others see role and year throughout.

mod common;
use common::groups::{live_assignment, revoke, seed_group, seed_group_member};
use common::membership::*;
use common::TestDb;
use fau_crypto::Ciphertext;
use fau_domain::membership::period::Period;
use fau_domain::membership::retention::RetentionMonths;
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

async fn account_of(pool: &PgPool, m: Uuid) -> Uuid {
    sqlx::query_scalar("select account_id from memberships where id = $1")
        .bind(m)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Spec §6 "changing the setting shortens an active retention", plan Ruling R8: shorter
/// moves the date, none clears at once, longer extends a running period, and an already
/// expired one is never revived. Across FAU-er.
#[tokio::test]
async fn the_setting_recalculates_every_running_period_and_never_revives_an_expired_one() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin-a@example.test", at(T0)).await;
    let b = active_fau(&pool, "admin-b@example.test", at(T0)).await;
    let in_a = join(&pool, &a, "kari@example.test", year()).await;
    let in_b = join(&pool, &b, "kari@example.test", year()).await;
    leave(&pool, &a, in_a, "2026-10-01T10:00:00Z").await; // until 2027-01-01
    leave(&pool, &b, in_b, "2026-10-01T10:00:00Z").await;
    let kari = account_of(&pool, in_a).await;
    let nov = at("2026-11-01T10:00:00Z");

    set_retention_months(&pool, kari, RetentionMonths::TwentyFour, nov)
        .await
        .unwrap();
    assert_eq!(
        state(&pool, in_a).await.2.as_deref(),
        Some("2028-10-01"),
        "extended"
    );
    assert_eq!(
        state(&pool, in_b).await.2.as_deref(),
        Some("2028-10-01"),
        "every FAU"
    );
    assert!(audits(&pool, in_a)
        .await
        .iter()
        .any(|(a, p)| a == "membership.retention_changed" && p.contains("2028-10-01")));

    set_retention_months(&pool, kari, RetentionMonths::Three, nov)
        .await
        .unwrap();
    assert_eq!(
        state(&pool, in_a).await.2.as_deref(),
        Some("2027-01-01"),
        "shortened"
    );

    // One month after leaving, a period of three is running; none clears it now.
    set_retention_months(&pool, kari, RetentionMonths::None, nov)
        .await
        .unwrap();
    assert_eq!(state(&pool, in_a).await, (false, false, None));
    assert!(audits(&pool, in_a)
        .await
        .iter()
        .any(|(a, p)| a == "membership.profile_cleared" && p.contains("retention_shortened")));
    let months: i32 = sqlx::query_scalar("select retention_months from accounts where id = $1")
        .bind(kari)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(months, 0);
}

/// Mutation check: skip the settle-under-the-old-setting step (fix round 1, Important 1)
/// and this already-revoked, already-stamped row is extended to 2028 instead of cleared --
/// `settle_profile` is what notices the stamped date is already past `today` and clears
/// it before the new setting is ever applied.
#[tokio::test]
async fn an_expired_period_is_cleared_not_extended() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let m = join(&pool, &a, "late@example.test", year()).await;
    leave(&pool, &a, m, "2026-10-01T10:00:00Z").await; // until 2027-01-01, no sweep since
    let acc = account_of(&pool, m).await;
    set_retention_months(
        &pool,
        acc,
        RetentionMonths::TwentyFour,
        at("2027-02-01T10:00:00Z"),
    )
    .await
    .unwrap();
    assert_eq!(state(&pool, m).await, (false, false, None));
    assert_eq!(
        audits(&pool, m).await.last().unwrap(),
        &(
            "membership.profile_cleared".to_owned(),
            r#"{"cause": "retention_ended"}"#.to_owned()
        )
    );

    // Calling again with the same value touches a row with no fields left to settle, so
    // it writes no second `retention_changed` audit.
    let before = audits(&pool, m).await.len();
    set_retention_months(
        &pool,
        acc,
        RetentionMonths::TwentyFour,
        at("2027-02-02T10:00:00Z"),
    )
    .await
    .unwrap();
    assert_eq!(audits(&pool, m).await.len(), before, "no new audit");
}

/// Fix round 1, Important 1: an unstamped natural end -- no revocation, and no sweep run
/// since the roles ran out, so `profile_retained_until` is still null -- must settle
/// under the *old* setting before the new one is written, so a period already over under
/// the old setting is never revived by a longer new one.
///
/// Mutation check: skip the settle-under-the-old-setting step and this membership, left
/// untouched by the old (tenant-list) query, keeps its name instead of being cleared.
#[tokio::test]
async fn an_unstamped_natural_end_already_over_is_settled_before_the_new_setting_applies() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let m = join(
        &pool,
        &a,
        "unswept@example.test",
        period(day(2026, 9, 1), day(2026, 10, 1)),
    )
    .await;
    set_months_by_sql(&pool, "unswept@example.test", 0).await;
    let acc = account_of(&pool, m).await;
    // The role ran out 2026-10-01; two weeks later, with the old setting of none, the
    // period is already over. No sweep has run.
    set_retention_months(
        &pool,
        acc,
        RetentionMonths::TwentyFour,
        at("2026-10-15T10:00:00Z"),
    )
    .await
    .unwrap();
    assert_eq!(state(&pool, m).await, (false, false, None));
}

/// The reverse of the case above: an unstamped natural end still within its old (default,
/// three-month) period is stamped by the settle-first step, then the new setting of none
/// clears it at once. Since the row was still running, not already expired, the clear is
/// `retention_shortened` (the settle step's own `retention_ended`/`membership_ended`
/// causes are for an already-over period, which this is not).
#[tokio::test]
async fn an_unstamped_natural_end_within_its_old_period_is_stamped_then_shortened_to_cleared() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let m = join(
        &pool,
        &a,
        "default@example.test",
        period(day(2026, 9, 1), day(2026, 10, 1)),
    )
    .await;
    // retention_months stays the account's default of three; no sweep has run since the
    // natural end on 2026-10-01.
    let acc = account_of(&pool, m).await;
    set_retention_months(
        &pool,
        acc,
        RetentionMonths::None,
        at("2026-10-15T10:00:00Z"),
    )
    .await
    .unwrap();
    assert_eq!(state(&pool, m).await, (false, false, None));
    assert_eq!(
        audits(&pool, m).await.last().unwrap(),
        &(
            "membership.profile_cleared".to_owned(),
            r#"{"cause": "retention_shortened"}"#.to_owned()
        )
    );
}

fn booked() -> Period {
    period(day(2026, 11, 1), day(2027, 11, 1))
}

fn count_of(audits: &[(String, String)], action: &str) -> usize {
    audits.iter().filter(|(a, _)| a == action).count()
}

/// Final review I2: a membership revoked before its first role started held no role, so
/// its end is the day it was revoked, not whatever day the period is next recalculated.
/// A setting call with the same value a month later moves nothing and audits nothing.
///
/// Mutation check: fall back to `today` for a membership that held no role (the old
/// `ended_on(&[], today)`) and the date moves to 2027-02-01 with a `retention_changed`.
#[tokio::test]
async fn a_membership_that_held_no_role_is_not_extended_by_a_recalculation() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let m = join(&pool, &a, "early@example.test", booked()).await;
    leave(&pool, &a, m, "2026-10-01T10:00:00Z").await;
    assert_eq!(state(&pool, m).await.2.as_deref(), Some("2027-01-01"));
    let acc = account_of(&pool, m).await;
    set_retention_months(
        &pool,
        acc,
        RetentionMonths::Three,
        at("2026-11-01T10:00:00Z"),
    )
    .await
    .unwrap();
    assert_eq!(
        state(&pool, m).await.2.as_deref(),
        Some("2027-01-01"),
        "unchanged"
    );
    assert_eq!(
        count_of(&audits(&pool, m).await, "membership.retention_changed"),
        0
    );
}

/// Final review I2: revoking a membership whose end the sweep already stamped keeps the
/// stamp and writes no second `profile_retained`. Covers a natural end, and a membership
/// whose only role was revoked before it started (no role held: its end is that role's
/// revocation day).
///
/// Mutation check: re-stamp in `revoke_membership` instead of keeping the stamp and both
/// get a second `profile_retained`; fall back to `today` for no role held and the sweep
/// stamps the second one 2027-01-15.
#[tokio::test]
async fn revoking_a_stamped_natural_end_keeps_the_stamp() {
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
    let never_started = join(&pool, &a, "never@example.test", booked()).await;
    revoke(
        &pool,
        &a,
        live_assignment(&pool, never_started).await,
        at("2026-10-01T10:00:00Z"),
    )
    .await;
    clear_ended_profiles(&pool, at("2026-10-15T10:00:00Z"))
        .await
        .unwrap();
    for m in [ran_out, never_started] {
        assert_eq!(
            state(&pool, m).await.2.as_deref(),
            Some("2027-01-01"),
            "stamped from the day it ended"
        );
    }
    for m in [ran_out, never_started] {
        leave(&pool, &a, m, "2026-10-20T10:00:00Z").await;
        assert_eq!(
            state(&pool, m).await,
            (true, false, Some("2027-01-01".into()))
        );
        assert_eq!(
            count_of(&audits(&pool, m).await, "membership.profile_retained"),
            1,
            "one stamp"
        );
    }
}

/// Erik, 29 September 2026 ("use the revocation date"): a membership that held no role
/// ended on the day its last booked role was revoked, even if the member leaves later.
/// The sweep stamps that day plus 3 months, the later leave keeps the stamp, and a
/// recalculation with the same setting a month on moves nothing and audits nothing.
///
/// Mutation check: put `m.revoked_at` first in `ended_on_for`'s coalesce again and the
/// recalculation takes the leave day, moving the date to 2027-01-20 with a
/// `retention_changed`.
#[tokio::test]
async fn a_role_revoked_before_it_started_marks_the_end_even_after_a_later_leave() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let m = join(&pool, &a, "never@example.test", booked()).await;
    revoke(
        &pool,
        &a,
        live_assignment(&pool, m).await,
        at("2026-10-01T10:00:00Z"),
    )
    .await;
    clear_ended_profiles(&pool, at("2026-10-15T10:00:00Z"))
        .await
        .unwrap();
    assert_eq!(state(&pool, m).await.2.as_deref(), Some("2027-01-01"));
    leave(&pool, &a, m, "2026-10-20T10:00:00Z").await;
    assert_eq!(state(&pool, m).await.2.as_deref(), Some("2027-01-01"));
    let acc = account_of(&pool, m).await;
    set_retention_months(
        &pool,
        acc,
        RetentionMonths::Three,
        at("2026-11-20T10:00:00Z"),
    )
    .await
    .unwrap();
    assert_eq!(
        state(&pool, m).await.2.as_deref(),
        Some("2027-01-01"),
        "the role's revocation day, not the leave day"
    );
    assert_eq!(
        count_of(&audits(&pool, m).await, "membership.retention_changed"),
        0
    );
}

#[tokio::test]
async fn an_unknown_account_is_refused() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    assert_eq!(
        set_retention_months(&pool, Uuid::now_v7(), RetentionMonths::Six, at(T0)).await,
        Err(MembershipError::UnknownAccount)
    );
}

// ---- Task 9 (Erik's M6): an ended membership leaves every group it was added to by hand.

/// The membership's hand-added group rows still in force.
async fn hand_groups(pool: &PgPool, m: Uuid) -> Vec<Uuid> {
    sqlx::query_scalar(
        "select group_id from group_members
          where membership_id = $1 and removed_at is null order by group_id",
    )
    .bind(m)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// `group.member_removed` audits naming the membership: (actor kind, cause).
async fn group_removals(pool: &PgPool, m: Uuid) -> Vec<(String, String)> {
    sqlx::query_as(
        "select actor_kind, params->>'cause' from audit_events
          where action = 'group.member_removed' and params->>'membership_id' = $1
          order by occurred_at, id",
    )
    .bind(m.to_string())
    .fetch_all(pool)
    .await
    .unwrap()
}

fn short() -> Period {
    period(day(2026, 9, 1), day(2026, 10, 1))
}

async fn grant(pool: &PgPool, fau: &Fau, m: Uuid, role: RoleChoice, p: Period, at_: &str) {
    grant_role(
        pool,
        GrantRole {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: m,
            role,
            period: p,
        },
        at(at_),
    )
    .await
    .unwrap();
}

/// M6: roles that ran out end the hand-added group memberships at the next sweep, audited
/// as the system with cause `membership_ended`. An active member of the same group stays.
///
/// Mutation check: drop the sweep's group pass in `clear_ended_profiles`, and the row
/// stays.
#[tokio::test]
async fn a_member_whose_roles_ran_out_leaves_their_groups_in_the_sweep() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let y = seed_group(&pool, &a, Visibility::Closed).await;
    let ran_out = join(&pool, &a, "ranout@example.test", short()).await;
    let stays = join(&pool, &a, "stays@example.test", year()).await;
    for m in [ran_out, stays] {
        seed_group_member(&pool, &a, y, m).await;
    }

    clear_ended_profiles(&pool, at("2026-10-15T10:00:00Z"))
        .await
        .unwrap();
    assert_eq!(hand_groups(&pool, ran_out).await, Vec::<Uuid>::new());
    assert_eq!(
        group_removals(&pool, ran_out).await,
        [("system".to_owned(), "membership_ended".to_owned())]
    );
    assert_eq!(
        hand_groups(&pool, stays).await,
        [y],
        "the control keeps its row"
    );
    assert_eq!(group_removals(&pool, stays).await, []);

    // Idempotent: a second sweep has nothing left to remove.
    clear_ended_profiles(&pool, at("2026-10-16T10:00:00Z"))
        .await
        .unwrap();
    assert_eq!(group_removals(&pool, ran_out).await.len(), 1);
}

/// M6: a guest who comes back before any sweep reaches only the group its role names,
/// not the closed group it was once added to by hand.
///
/// Mutation check: remove the group removal from `reopen_profile`, and Y comes back.
#[tokio::test]
async fn a_guest_returning_before_any_sweep_reaches_only_the_guest_group() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let y = seed_group(&pool, &a, Visibility::Closed).await;
    let x = seed_group(&pool, &a, Visibility::Closed).await;
    let m = join(&pool, &a, "guest@example.test", short()).await;
    seed_group_member(&pool, &a, y, m).await;

    let guest = RoleChoice::New {
        name: RoleName::parse("Gjest").unwrap(),
        capability: CapabilityClass::Guest,
        group_id: Some(x),
    };
    grant(
        &pool,
        &a,
        m,
        guest,
        period(day(2026, 10, 15), day(2027, 1, 15)),
        "2026-10-15T10:00:00Z",
    )
    .await;
    let viewer = Viewer {
        tenant_id: a.tenant_id,
        membership_id: m,
    };
    let groups = list_groups(&pool, viewer, at("2026-10-16T10:00:00Z"))
        .await
        .unwrap();
    assert_eq!(groups.iter().map(|g| g.group_id).collect::<Vec<_>>(), [x]);
    assert_eq!(hand_groups(&pool, m).await, Vec::<Uuid>::new());
    assert_eq!(
        group_removals(&pool, m).await,
        [("system".to_owned(), "membership_ended".to_owned())]
    );
}

/// M6: a return through an invitation starts with no groups. The open group is still
/// listed, as any member sees an open group, but never as the viewer's own.
///
/// Mutation check: remove the group removal from `reopen_profile`, and both come back as
/// `viewer_in_group`.
#[tokio::test]
async fn returning_by_invitation_starts_with_no_groups() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let closed = seed_group(&pool, &a, Visibility::Closed).await;
    let open = seed_group(&pool, &a, Visibility::Open).await;
    let m = join(&pool, &a, "back@example.test", short()).await;
    for g in [closed, open] {
        seed_group_member(&pool, &a, g, m).await;
    }

    let (_, again) = reinvite(
        &pool,
        &a,
        "back@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 11, 1), day(2027, 11, 1)),
        "2026-11-01T10:00:00Z",
    )
    .await;
    assert_eq!(again, m);
    assert_eq!(hand_groups(&pool, m).await, Vec::<Uuid>::new());
    assert_eq!(group_removals(&pool, m).await.len(), 2);
    let viewer = Viewer {
        tenant_id: a.tenant_id,
        membership_id: m,
    };
    let groups = list_groups(&pool, viewer, at("2026-11-02T10:00:00Z"))
        .await
        .unwrap();
    assert!(
        groups.iter().all(|g| !g.viewer_in_group),
        "no group is the returner's own"
    );
    assert!(
        groups.iter().any(|g| g.group_id == open),
        "open stays visible"
    );
}

/// Control for M6: `grant_role` reopens an active member too; that member keeps their
/// hand-added groups and no removal is written.
///
/// Mutation check: remove groups in `reopen_profile` unconditionally (not only when the
/// membership has ended), and the row goes.
#[tokio::test]
async fn an_active_member_keeps_their_groups_when_granted_another_role() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin@example.test", at(T0)).await;
    let y = seed_group(&pool, &a, Visibility::Closed).await;
    let m = join(&pool, &a, "active@example.test", year()).await;
    seed_group_member(&pool, &a, y, m).await;
    grant(
        &pool,
        &a,
        m,
        new_role("Kasserer", CapabilityClass::Member),
        period(day(2026, 10, 1), day(2027, 10, 1)),
        "2026-10-01T10:00:00Z",
    )
    .await;
    assert_eq!(hand_groups(&pool, m).await, [y]);
    assert_eq!(group_removals(&pool, m).await, []);
}
