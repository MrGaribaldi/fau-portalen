//! How long the directory fields live (groups design §4.2 as amended by Erik's D3, 28
//! September 2026; §10; #3502): neither the name nor the contact address outlives the
//! membership, history shows an ended membership as its role and years, and an erasure
//! shows "Tidligere medlem".

mod common;
use common::groups::*;
use common::membership::*;
use common::TestDb;
use fau_crypto::Ciphertext;
use fau_domain::directory::history::{ActivePeriod, HeldRole};
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

async fn assignment_count(pool: &PgPool, m: Uuid) -> i64 {
    sqlx::query_scalar("select count(*) from role_assignments where membership_id = $1")
        .bind(m)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn membership_revoked(pool: &PgPool, m: Uuid) -> bool {
    sqlx::query_scalar("select revoked_at is not null from memberships where id = $1")
        .bind(m)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Whether exactly one invitation to `recipient` is still pending -- unaccepted.
async fn invitation_is_still_pending(pool: &PgPool, recipient: &str) -> bool {
    let count: i64 = sqlx::query_scalar(
        "select count(*) from invitations where recipient_email = $1 and accepted_at is null",
    )
    .bind(recipient)
    .fetch_one(pool)
    .await
    .unwrap();
    count == 1
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

/// Sets the account's retention period (#3511). Call it after the account exists (its
/// first acceptance); before that the update sets nothing.
async fn set_months_by_sql(pool: &PgPool, address: &str, months: i32) {
    sqlx::query("update accounts set retention_months = $2 where email = $1")
        .bind(address)
        .bind(months)
        .execute(pool)
        .await
        .unwrap();
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

/// Mutation checks: drop `ra.revoked_at is null` from `membership_ended` and the
/// stepped-down member keeps their fields; make `member_names` ignore `ended` and the read
/// before the sweep returns the name.
#[tokio::test]
async fn the_sweep_clears_the_name_and_address_once_no_role_runs_or_is_still_to_come_with_a_period_of_none(
) {
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
    // #3511: with a period of none, every end clears at once, as under D3.
    for address in [
        "ends@example.test",
        "quiet@example.test",
        "renewed@example.test",
        "kari@example.test",
        "down@example.test",
    ] {
        set_months_by_sql(&pool, address, 0).await;
    }
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
    // #3511: the short role ran out and the next one has not started, so this membership
    // has an earlier period too -- `Returned`, not `Named` (M1).
    assert_eq!(
        name(&pool, &a, renewed, "2026-10-01T10:00:00Z").await,
        MemberName::Returned {
            name: placeholder_envelope(),
            period: ActivePeriod {
                since: day(2026, 10, 1),
                earlier: vec![HeldRole {
                    name: "Medlem".into(),
                    class: CapabilityClass::Member,
                    from: day(2026, 9, 1),
                    until: day(2026, 10, 1),
                }],
            },
        }
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

    // Fix round 1 (controller ruling Q12, carry-in c): the failed acceptance below must
    // seat nothing. Capture the state before the attempt, so the assertions after it are
    // a real "unchanged", not just "still revoked/erased" read from the docs.
    let assignments_before = assignment_count(&pool, in_a.membership_id).await;
    assert!(
        membership_revoked(&pool, in_a.membership_id).await,
        "revoked, before"
    );
    assert!(
        invitation_is_still_pending(&pool, "kari@example.test").await,
        "unaccepted, before"
    );

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

    assert_eq!(
        assignment_count(&pool, in_a.membership_id).await,
        assignments_before,
        "no role assignment was seated"
    );
    assert!(
        membership_revoked(&pool, in_a.membership_id).await,
        "still revoked"
    );
    assert!(
        invitation_is_still_pending(&pool, "kari@example.test").await,
        "still unaccepted"
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

/// Fix round 1 (controller ruling Q12, carry-in a): `member_names`' `authorize` call
/// refuses a viewer whose own standing has lapsed, exactly as it refuses a guest or a
/// viewer from another FAU (the sibling test above). Each case names the mutation it
/// catches; the revoked and ran-out cases were demonstrated failing under a temporary
/// mutation (recorded in the fix-round report), then reverted -- neither mutation is left
/// in the tree.
#[tokio::test]
async fn member_names_refuses_a_viewer_whose_own_standing_has_lapsed() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let a = active_fau(&pool, "admin-a@example.test", t0).await;
    let year = period(day(2026, 9, 1), day(2027, 9, 1));

    // Revoked via `revoke_membership` (leaving), which revokes both the membership row
    // and the assignment itself -- deliberately redundant (Task 9's pattern, `access.rs`):
    // either alone still blocks access through the other. So no single-line mutation
    // defeats this case; it takes both at once: drop `standing.membership_active` from
    // `evaluate_access`'s guard *and* `!self.revoked` from `AssignmentView::valid_on`
    // (`fau_domain::membership::access`), and only then does the revoked viewer's own,
    // still-unexpired role period make their capability look current again.
    let revoked = join(&pool, &a, "revoked@example.test", year)
        .await
        .membership_id;
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: a.tenant_id,
            actor_membership_id: revoked,
            membership_id: revoked,
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        member_names(
            &pool,
            Viewer {
                tenant_id: a.tenant_id,
                membership_id: revoked
            },
            &[revoked],
            t0,
        )
        .await,
        Err(MembershipError::NotAuthorized),
        "revoked"
    );

    // Ran out: read after the only role's period ends, with the membership itself never
    // revoked.
    //
    // Mutation check: drop `self.period.contains(day)` from `AssignmentView::valid_on`,
    // and this fails: a role that ended keeps looking current forever.
    let ran_out = join(
        &pool,
        &a,
        "ranout@example.test",
        period(day(2026, 9, 1), day(2026, 10, 1)),
    )
    .await
    .membership_id;
    assert_eq!(
        member_names(
            &pool,
            Viewer {
                tenant_id: a.tenant_id,
                membership_id: ran_out
            },
            &[ran_out],
            at("2026-10-01T10:00:00Z"),
        )
        .await,
        Err(MembershipError::NotAuthorized),
        "ran out"
    );

    // Disabled account: standing and a running role otherwise, but the account itself is
    // unusable.
    //
    // Mutation check: drop `a.disabled_at is null` from `ACCOUNT_USABLE`
    // (`fau_persistence::membership::sql`), and this fails: a disabled account's viewer
    // reads on regardless.
    let disabled = join(&pool, &a, "disabled@example.test", year).await;
    sqlx::query("update accounts set disabled_at = now() where id = $1")
        .bind(disabled.account_id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        member_names(
            &pool,
            Viewer {
                tenant_id: a.tenant_id,
                membership_id: disabled.membership_id
            },
            &[disabled.membership_id],
            t0,
        )
        .await,
        Err(MembershipError::NotAuthorized),
        "disabled account"
    );
}

/// D3 cheap check (fix round 1, item 5): a revocation timestamped 23:30 UTC on 31
/// December is already 00:30 on 1 January in Oslo (winter, UTC+1), and `member_names`
/// must cap the held role at the Oslo date (`fau_domain::time::oslo_today`), not the UTC
/// one. A wrong, UTC-only conversion would report `until` as 31 December instead.
#[tokio::test]
async fn a_revocation_at_the_new_year_uses_the_oslo_date_not_the_utc_one() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let a = active_fau(&pool, "admin-a@example.test", t0).await;
    let m = join(
        &pool,
        &a,
        "nyttaar@example.test",
        period(day(2026, 1, 1), day(2028, 1, 1)),
    )
    .await
    .membership_id;
    revoke(
        &pool,
        &a,
        live_assignment(&pool, m).await,
        at("2026-12-31T23:30:00Z"),
    )
    .await;
    assert_eq!(
        name(&pool, &a, m, "2027-06-01T10:00:00Z").await,
        medlem(day(2026, 1, 1), day(2027, 1, 1)),
        "the Oslo date (1 January), not the UTC one (31 December)"
    );
}

/// D3 cheap check (fix round 1, item 5): an assignment revoked before it ever began held
/// nothing. `member_names`' `from < until` guard drops it, so a membership whose only
/// role never actually started shows "Tidligere medlem", the same as one that was never
/// given a role at all -- never an empty-but-present role list.
#[tokio::test]
async fn an_assignment_revoked_before_it_began_holds_nothing() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let a = active_fau(&pool, "admin-a@example.test", t0).await;
    let m = join(
        &pool,
        &a,
        "aldri@example.test",
        period(day(2027, 1, 1), day(2028, 1, 1)),
    )
    .await
    .membership_id;
    revoke(&pool, &a, live_assignment(&pool, m).await, t0).await;
    assert_eq!(
        name(&pool, &a, m, T0).await,
        MemberName::Former,
        "never held a role"
    );
}

async fn profile_cleared_audits(pool: &PgPool, m: Uuid) -> Vec<(String, String, String)> {
    sqlx::query_as(
        "select actor_kind, subject_type, params::text from audit_events
          where action = 'membership.profile_cleared' and subject_id = $1",
    )
    .bind(m)
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn grant(pool: &PgPool, fau: &Fau, m: Uuid, p: Period, at_: &str) {
    grant_role(
        pool,
        GrantRole {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: m,
            role: new_role("Nytt verv", CapabilityClass::Member),
            period: p,
        },
        at(at_),
    )
    .await
    .unwrap();
}

/// Final review I1 (controller ruling Q13): a membership whose roles simply ran out is not
/// revoked, so `grant_role` may reach it before the daily sweep has. With a period of none
/// (#3511) it holds neither field any more, so the grant clears both in its own
/// transaction, audited the way the sweep audits, and the returning member states their
/// name again. A retained profile is restored instead (`member_retention.rs`).
///
/// Mutation check: drop the `reopen_profile` call from `grant_role` and the old name and
/// address become valid again on the new role.
#[tokio::test]
async fn granting_a_role_to_an_ended_membership_clears_what_the_sweep_has_not_with_a_period_of_none(
) {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin-a@example.test", at(T0)).await;
    let m = join(
        &pool,
        &a,
        "tilbake@example.test",
        period(day(2026, 9, 1), day(2026, 10, 1)),
    )
    .await
    .membership_id;
    with_contact(&pool, &a, m, 1).await;
    set_months_by_sql(&pool, "tilbake@example.test", 0).await;
    let later = "2026-10-05T10:00:00Z";
    assert_eq!(
        fields(&pool, m).await,
        (
            Some(placeholder_envelope().as_bytes().to_vec()),
            Some(envelope(1).as_bytes().to_vec())
        ),
        "ended, but not swept"
    );

    grant(
        &pool,
        &a,
        m,
        period(day(2026, 10, 5), day(2027, 10, 5)),
        later,
    )
    .await;

    assert_eq!(fields(&pool, m).await, (None, None));
    assert_eq!(
        profile_cleared_audits(&pool, m).await,
        [(
            "system".to_owned(),
            "membership".to_owned(),
            r#"{"cause": "membership_ended"}"#.to_owned()
        )]
    );
}

/// The control for the test above: an extra role for a member who is still active leaves
/// both fields alone and writes no clearing.
///
/// Mutation check: clear unconditionally in `grant_role` and this fails.
#[tokio::test]
async fn granting_an_extra_role_to_an_active_member_keeps_both_fields() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let a = active_fau(&pool, "admin-a@example.test", at(T0)).await;
    let m = join(
        &pool,
        &a,
        "aktiv@example.test",
        period(day(2026, 9, 1), day(2027, 9, 1)),
    )
    .await
    .membership_id;
    with_contact(&pool, &a, m, 1).await;

    grant(
        &pool,
        &a,
        m,
        period(day(2026, 10, 5), day(2027, 10, 5)),
        "2026-10-05T10:00:00Z",
    )
    .await;

    assert_eq!(
        fields(&pool, m).await,
        (
            Some(placeholder_envelope().as_bytes().to_vec()),
            Some(envelope(1).as_bytes().to_vec())
        )
    );
    assert!(profile_cleared_audits(&pool, m).await.is_empty());
}
