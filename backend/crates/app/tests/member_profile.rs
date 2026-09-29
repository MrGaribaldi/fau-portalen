//! The directory fields enter the system (groups design §4.1, §8's #3418 row; #3502): a
//! display name is required when an invitation is accepted or an FAU activated, a contact
//! address is optional, and both are bound to the membership `prepare_acceptance` names.
//! Under D3 both leave with the membership: revocation clears them.

mod common;
use common::membership::*;
use common::TestDb;
use fau_crypto::Ciphertext;
use fau_domain::membership::vocabulary::CapabilityClass;
use fau_persistence::membership::*;
use sqlx::PgPool;
use uuid::Uuid;

/// An envelope whose bytes say which one it is, so a test can tell two apart.
fn envelope(marker: u8) -> Ciphertext {
    let mut v = vec![1u8, marker];
    v.extend_from_slice(&[0u8; 40]);
    Ciphertext::from_stored(v)
}

fn year() -> fau_domain::membership::period::Period {
    period(day(2026, 9, 1), day(2027, 9, 1))
}

async fn invite(pool: &PgPool, fau: &Fau, address: &str) -> String {
    issue_invitation(
        pool,
        IssueInvitation {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            recipient: email(address),
            roles: vec![OfferedRole {
                role: new_role("Medlem", CapabilityClass::Member),
                period: year(),
            }],
            handover_grant_id: None,
            message: None,
        },
        at(T0),
    )
    .await
    .unwrap()
    .token
    .expose()
    .to_owned()
}

async fn fields(pool: &PgPool, membership: Uuid) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    sqlx::query_as(
        "select encrypted_display_name, encrypted_contact_email from memberships where id = $1",
    )
    .bind(membership)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn pending_invitations(pool: &PgPool, recipient: &str) -> i64 {
    sqlx::query_scalar(
        "select count(*) from invitations where recipient_email = $1 and accepted_at is null",
    )
    .bind(recipient)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn acceptance_stores_the_name_and_the_optional_address_on_the_prepared_membership() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;

    for (address, contact) in [
        ("kari@example.test", Some(envelope(7))),
        ("ola@example.test", None),
    ] {
        let token = invite(&pool, &fau, address).await;
        let target = prepare_acceptance(&pool, &token, &verified(address), at(T0))
            .await
            .unwrap();
        assert_eq!(target.tenant_id, fau.tenant_id);
        let accepted = accept_invitation(
            &pool,
            AcceptInvitation {
                token,
                acceptor: verified(address),
                admin_end_override: None,
                profile: MemberProfile {
                    membership_id: target.membership_id,
                    encrypted_display_name: envelope(3),
                    encrypted_contact_email: contact.clone(),
                },
            },
            at(T0),
        )
        .await
        .unwrap();
        assert_eq!(accepted.membership_id, target.membership_id, "{address}");
        let (name, stored_contact) = fields(&pool, accepted.membership_id).await;
        assert_eq!(name.as_deref(), Some(envelope(3).as_bytes()), "{address}");
        assert_eq!(
            stored_contact.as_deref(),
            contact.as_ref().map(|c| c.as_bytes()),
            "{address}"
        );
    }
}

#[tokio::test]
async fn activation_stores_the_registrants_name() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let school_id = school(&pool, "activation-name").await;
    let pending = create_pending_tenant(
        &pool,
        signup(school_id, "reg@example.test", "reg@example.test", t0),
        t0,
    )
    .await
    .unwrap();
    let chosen = Uuid::now_v7();
    let activated = activate_tenant(
        &pool,
        Activation {
            tenant_id: pending.tenant_id,
            registrant: verified("reg@example.test"),
            profile: MemberProfile {
                membership_id: chosen,
                encrypted_display_name: envelope(5),
                encrypted_contact_email: Some(envelope(6)),
            },
        },
        t0,
    )
    .await
    .unwrap();
    assert_eq!(activated.membership_id, chosen);
    let (name, contact) = fields(&pool, chosen).await;
    assert_eq!(name.as_deref(), Some(envelope(5).as_bytes()));
    assert_eq!(contact.as_deref(), Some(envelope(6).as_bytes()));
}

#[tokio::test]
async fn a_malformed_field_is_refused_before_anything_is_written() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    let token = invite(&pool, &fau, "kari@example.test").await;
    let short = Ciphertext::from_stored(vec![1u8; 41]);
    let wrong_version = Ciphertext::from_stored(vec![2u8; 60]);
    for (profile, want) in [
        (
            MemberProfile {
                encrypted_display_name: short.clone(),
                ..fresh_profile()
            },
            MembershipError::DisplayNameMalformed,
        ),
        (
            MemberProfile {
                encrypted_display_name: wrong_version.clone(),
                ..fresh_profile()
            },
            MembershipError::DisplayNameMalformed,
        ),
        (
            MemberProfile {
                encrypted_contact_email: Some(Ciphertext::from_stored(vec![1u8; 513])),
                ..fresh_profile()
            },
            MembershipError::ContactEmailMalformed,
        ),
    ] {
        let err = accept_invitation(
            &pool,
            AcceptInvitation {
                token: token.clone(),
                acceptor: verified("kari@example.test"),
                admin_end_override: None,
                profile,
            },
            at(T0),
        )
        .await
        .unwrap_err();
        assert_eq!(err, want);
        assert_eq!(pending_invitations(&pool, "kari@example.test").await, 1);
    }
    assert_eq!(
        count(
            &pool,
            "select count(*) from accounts where email = 'kari@example.test'"
        )
        .await,
        0,
        "no account was created"
    );
}

/// Fix round 1, item 2 (controller ruling Q10): `membership_id` must be a UUIDv7, like
/// every other id this schema mints. `Uuid::nil()` and a v4-shaped id are both refused, the
/// same DisplayName-style check-then-error pattern as the ciphertext fields.
#[tokio::test]
async fn a_non_uuidv7_membership_id_is_refused_before_anything_is_written() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    let token = invite(&pool, &fau, "kari@example.test").await;
    let v4_shaped = Uuid::from_u128(0x0199_0000_0000_4000_8000_0000_0000_0301);
    for bad_id in [Uuid::nil(), v4_shaped] {
        let err = accept_invitation(
            &pool,
            AcceptInvitation {
                token: token.clone(),
                acceptor: verified("kari@example.test"),
                admin_end_override: None,
                profile: MemberProfile {
                    membership_id: bad_id,
                    ..fresh_profile()
                },
            },
            at(T0),
        )
        .await
        .unwrap_err();
        assert_eq!(err, MembershipError::MembershipIdMalformed, "{bad_id}");
        assert_eq!(pending_invitations(&pool, "kari@example.test").await, 1);
    }
}

// Task 5: passes a revoked member through re-acceptance; under #3511 the revocation keeps
// the name for the default period, so this fails until Task 5 settles re-acceptance.
#[tokio::test]
async fn a_profile_for_another_membership_is_refused() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let first = add_member(
        &pool,
        &fau,
        "kari@example.test",
        new_role("Medlem", CapabilityClass::Member),
        year(),
        t0,
    )
    .await;
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: first.membership_id,
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        fields(&pool, first.membership_id).await,
        (None, None),
        "revocation took the name (D3)"
    );

    // Invited back: prepare names the old membership, which acceptance reopens. Revoked,
    // it is not existing_current (controller ruling, Task 5 review: the same shared
    // `membership_ended` check `ensure_membership` and editing use).
    //
    // Note (fix round 1, Q11): this does not exercise `membership_ended`'s
    // `revoked_at is not null` arm on its own -- `revoke_membership` above also revokes
    // every running and future role assignment, so the `not exists (...)` arm alone
    // already reports this row ended. The arm that can fail only through `revoked_at` is
    // covered by `a_membership_revoked_by_other_means_still_counts_as_ended`.
    let token = invite(&pool, &fau, "kari@example.test").await;
    let target = prepare_acceptance(&pool, &token, &verified("kari@example.test"), t0)
        .await
        .unwrap();
    assert_eq!(target.membership_id, first.membership_id);
    assert!(!target.existing_current, "revoked, so not existing_current");

    // A profile encrypted for any other id would be undecryptable on that row: refused,
    // and the membership stays revoked.
    let err = accept_invitation(
        &pool,
        AcceptInvitation {
            token: token.clone(),
            acceptor: verified("kari@example.test"),
            admin_end_override: None,
            profile: fresh_profile(),
        },
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::AcceptanceTargetChanged);
    assert_eq!(pending_invitations(&pool, "kari@example.test").await, 1);
    let revoked: bool =
        sqlx::query_scalar("select revoked_at is not null from memberships where id = $1")
            .bind(first.membership_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(revoked);

    // Positive control: the prepared id is accepted, and the reopened row is named again.
    accept_invitation(
        &pool,
        AcceptInvitation {
            token,
            acceptor: verified("kari@example.test"),
            admin_end_override: None,
            profile: MemberProfile {
                membership_id: target.membership_id,
                encrypted_display_name: envelope(9),
                encrypted_contact_email: None,
            },
        },
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        fields(&pool, first.membership_id).await.0.as_deref(),
        Some(envelope(9).as_bytes())
    );
}

/// Controller ruling Q11 (Task 5 fix round 1): every revoked row `a_profile_for_another_
/// membership_is_refused` builds went through `revoke_membership`, which also revokes the
/// running and future role assignments -- so `membership_ended`'s `not exists (...)` arm
/// alone already reports it ended, and the `m.revoked_at is not null` arm on its own had
/// no test that could fail if it were dropped. This revokes by direct SQL instead, with a
/// live role assignment left in place, so only the `revoked_at` arm can catch it.
///
/// Mutation check: drop the `m.revoked_at is not null or` arm from `membership_ended`,
/// and this test fails on both assertions -- `existing_current` comes back `true`
/// (the running assignment makes `not exists (...)` false too), and `set_display_name`
/// succeeds instead of returning `MembershipEnded`.
#[tokio::test]
async fn a_membership_revoked_by_other_means_still_counts_as_ended() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let kari = add_member(
        &pool,
        &fau,
        "kari@example.test",
        new_role("Medlem", CapabilityClass::Member),
        year(),
        t0,
    )
    .await
    .membership_id;

    // Revoked by hand, not through `revoke_membership`: its role assignment is left
    // running, exactly as D3 requires when a row is "revoked any other way" (0008's
    // checks: revoked_at is not null or both fields are null).
    sqlx::query(
        "update memberships
            set revoked_at = now(), encrypted_display_name = null, encrypted_contact_email = null
          where id = $1",
    )
    .bind(kari)
    .execute(&pool)
    .await
    .unwrap();

    let token = invite(&pool, &fau, "kari@example.test").await;
    let target = prepare_acceptance(&pool, &token, &verified("kari@example.test"), t0)
        .await
        .unwrap();
    assert_eq!(target.membership_id, kari);
    assert!(
        !target.existing_current,
        "revoked by hand, so not existing_current"
    );

    assert_eq!(
        set_display_name(
            &pool,
            SetDisplayName {
                tenant_id: fau.tenant_id,
                actor_membership_id: fau.admin_membership_id,
                membership_id: kari,
                encrypted_display_name: envelope(9),
            },
            t0,
        )
        .await,
        Err(MembershipError::MembershipEnded)
    );
}

/// Fix round 1, Q11 (minor): `existing_current` must not report an erased row as one
/// whose name the caller may prefill, even though it is neither revoked nor "ended" by
/// role assignment. There is no erasure transaction yet (a later task's scope), so this
/// sets `name_erased_at` by direct SQL, with both fields left null as migration 0008's
/// `memberships_erasure_leaves_nothing` requires.
///
/// Mutation check: drop `and m.name_erased_at is null` from `prepare_acceptance`'s
/// query, and this fails: an unrevoked, currently-assigned but erased row reports
/// `existing_current = true`.
#[tokio::test]
async fn an_erased_but_unrevoked_membership_is_not_existing_current() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let kari = add_member(
        &pool,
        &fau,
        "kari@example.test",
        new_role("Medlem", CapabilityClass::Member),
        year(),
        t0,
    )
    .await
    .membership_id;
    sqlx::query(
        "update memberships
            set name_erased_at = now(), encrypted_display_name = null, encrypted_contact_email = null
          where id = $1",
    )
    .bind(kari)
    .execute(&pool)
    .await
    .unwrap();

    let token = invite(&pool, &fau, "kari@example.test").await;
    let target = prepare_acceptance(&pool, &token, &verified("kari@example.test"), t0)
        .await
        .unwrap();
    assert_eq!(target.membership_id, kari, "the erased row is still reused");
    assert!(!target.existing_current, "erased, so not existing_current");
}

#[tokio::test]
async fn prepare_names_only_this_faus_membership_and_only_for_the_recipient() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let a = active_fau(&pool, "admin-a@example.test", t0).await;
    let b = active_fau(&pool, "admin-b@example.test", t0).await;
    // Kari is already a member of B; A invites her.
    let in_b = add_member(
        &pool,
        &b,
        "kari@example.test",
        new_role("Medlem", CapabilityClass::Member),
        year(),
        t0,
    )
    .await;
    let token = invite(&pool, &a, "kari@example.test").await;
    let target = prepare_acceptance(&pool, &token, &verified("kari@example.test"), t0)
        .await
        .unwrap();
    assert_eq!(target.tenant_id, a.tenant_id);
    assert_ne!(
        target.membership_id, in_b.membership_id,
        "a membership in another FAU is never reused"
    );
    let again = prepare_acceptance(&pool, &token, &verified("kari@example.test"), t0)
        .await
        .unwrap();
    assert_ne!(again.membership_id, target.membership_id, "fresh each time");

    // Every other failure looks the same.
    for (token, reader) in [
        (token.as_str(), "ola@example.test"),
        ("not-a-token", "kari@example.test"),
        (&"a".repeat(64)[..], "kari@example.test"),
    ] {
        assert_eq!(
            prepare_acceptance(&pool, token, &verified(reader), t0)
                .await
                .unwrap_err(),
            MembershipError::UnknownInvitation,
            "{reader}"
        );
    }
}

#[tokio::test]
async fn prepare_answers_for_an_expired_invitation_so_acceptance_can_say_why() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    let token = invite(&pool, &fau, "kari@example.test").await;
    let late = at("2027-06-01T10:00:00Z");
    let target = prepare_acceptance(&pool, &token, &verified("kari@example.test"), at(T0))
        .await
        .unwrap();
    let err = accept_invitation(
        &pool,
        AcceptInvitation {
            token,
            acceptor: verified("kari@example.test"),
            admin_end_override: None,
            profile: MemberProfile {
                membership_id: target.membership_id,
                ..fresh_profile()
            },
        },
        late,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, MembershipError::Acceptance(_)), "{err:?}");
}

/// Fix round 1, item 1(b) (controller ruling Q10): `existing_current` tells the caller
/// whether the membership `prepare_acceptance` names is already active and already has a
/// name -- so a fresh acceptee is `false`, and an active member reached by a second
/// invitation is `true`.
#[tokio::test]
async fn prepare_reports_existing_current_only_for_an_active_named_membership() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;

    // Negative control: never a member here.
    let fresh_token = invite(&pool, &fau, "ny@example.test").await;
    let fresh = prepare_acceptance(&pool, &fresh_token, &verified("ny@example.test"), t0)
        .await
        .unwrap();
    assert!(!fresh.existing_current, "never joined");

    // Kari accepts once, so her membership is active and named.
    let token = invite(&pool, &fau, "kari@example.test").await;
    let target = prepare_acceptance(&pool, &token, &verified("kari@example.test"), t0)
        .await
        .unwrap();
    assert!(!target.existing_current, "not yet a member");
    accept_invitation(
        &pool,
        AcceptInvitation {
            token,
            acceptor: verified("kari@example.test"),
            admin_end_override: None,
            profile: MemberProfile {
                membership_id: target.membership_id,
                encrypted_display_name: envelope(1),
                encrypted_contact_email: None,
            },
        },
        t0,
    )
    .await
    .unwrap();

    // A second invitation to the same address now prepares against an active, named row.
    let second_token = invite(&pool, &fau, "kari@example.test").await;
    let second = prepare_acceptance(&pool, &second_token, &verified("kari@example.test"), t0)
        .await
        .unwrap();
    assert_eq!(second.membership_id, target.membership_id);
    assert!(second.existing_current, "active and named");
}

/// Controller ruling, Task 5 review: `already_current` only means "no address to keep
/// unstated" -- it never means "keep the old address regardless". A second invitation to
/// an already-active member that states a fresh address replaces the stored one.
///
/// Mutation check: change `write_profile`'s case condition from `$5 and $4 is null` to
/// bare `$5`, and this fails: the fresh address (9) would be silently dropped in favour of
/// the one already stored (2).
#[tokio::test]
async fn an_already_current_membership_stating_a_fresh_address_has_it_replace_the_stored_one() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let token = invite(&pool, &fau, "kari@example.test").await;
    let target = prepare_acceptance(&pool, &token, &verified("kari@example.test"), t0)
        .await
        .unwrap();
    let kari = accept_invitation(
        &pool,
        AcceptInvitation {
            token,
            acceptor: verified("kari@example.test"),
            admin_end_override: None,
            profile: MemberProfile {
                membership_id: target.membership_id,
                encrypted_display_name: envelope(1),
                encrypted_contact_email: Some(envelope(2)),
            },
        },
        t0,
    )
    .await
    .unwrap()
    .membership_id;

    // A second invitation reaches the same, still-active member, and states a new
    // address this time.
    let second_token = invite(&pool, &fau, "kari@example.test").await;
    let second = prepare_acceptance(&pool, &second_token, &verified("kari@example.test"), t0)
        .await
        .unwrap();
    assert!(second.existing_current, "active and named");
    accept_invitation(
        &pool,
        AcceptInvitation {
            token: second_token,
            acceptor: verified("kari@example.test"),
            admin_end_override: None,
            profile: MemberProfile {
                membership_id: second.membership_id,
                encrypted_display_name: envelope(3),
                encrypted_contact_email: Some(envelope(9)),
            },
        },
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        fields(&pool, kari).await,
        (
            Some(envelope(3).as_bytes().to_vec()),
            Some(envelope(9).as_bytes().to_vec())
        ),
        "the stated address replaces the stored one"
    );
}

/// Controller ruling, Task 5 review: a membership whose roles simply ran out -- not
/// revoked, but the retention sweep hasn't reached it yet -- is `already_current = false`
/// too, the same as a revoked one. Re-invited the same day, before the sweep, it holds no
/// address to keep.
///
/// Mutation check: revert `ensure_membership`'s third return value to `!was_revoked`
/// (dropping the shared `membership_ended` check), and this fails: the stale address (2)
/// survives the second acceptance instead of being dropped.
#[tokio::test]
async fn a_membership_whose_roles_ran_out_reports_existing_current_false_and_drops_its_address() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let issued = issue_invitation(
        &pool,
        IssueInvitation {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            recipient: email("kari@example.test"),
            roles: vec![OfferedRole {
                role: new_role("Medlem", CapabilityClass::Member),
                period: period(day(2026, 9, 1), day(2026, 10, 1)),
            }],
            handover_grant_id: None,
            message: None,
        },
        t0,
    )
    .await
    .unwrap();
    let token = issued.token.expose().to_owned();
    let target = prepare_acceptance(&pool, &token, &verified("kari@example.test"), t0)
        .await
        .unwrap();
    let kari = accept_invitation(
        &pool,
        AcceptInvitation {
            token,
            acceptor: verified("kari@example.test"),
            admin_end_override: None,
            profile: MemberProfile {
                membership_id: target.membership_id,
                encrypted_display_name: envelope(1),
                encrypted_contact_email: Some(envelope(2)),
            },
        },
        t0,
    )
    .await
    .unwrap()
    .membership_id;

    // Her only role ends 2026-10-01. The same day, ahead of any sweep, she is invited
    // back to a fresh role.
    let after = at("2026-10-01T09:00:00Z");
    let issued_again = issue_invitation(
        &pool,
        IssueInvitation {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            recipient: email("kari@example.test"),
            roles: vec![OfferedRole {
                role: new_role("Medlem", CapabilityClass::Member),
                period: period(day(2026, 10, 1), day(2027, 9, 1)),
            }],
            handover_grant_id: None,
            message: None,
        },
        after,
    )
    .await
    .unwrap();
    let token_again = issued_again.token.expose().to_owned();
    let target_again =
        prepare_acceptance(&pool, &token_again, &verified("kari@example.test"), after)
            .await
            .unwrap();
    assert_eq!(target_again.membership_id, kari, "the same row is reused");
    assert!(
        !target_again.existing_current,
        "roles had run out, ahead of the sweep"
    );

    accept_invitation(
        &pool,
        AcceptInvitation {
            token: token_again,
            acceptor: verified("kari@example.test"),
            admin_end_override: None,
            profile: MemberProfile {
                membership_id: target_again.membership_id,
                encrypted_display_name: envelope(3),
                encrypted_contact_email: None,
            },
        },
        after,
    )
    .await
    .unwrap();
    assert_eq!(
        fields(&pool, kari).await,
        (Some(envelope(3).as_bytes().to_vec()), None),
        "an ended membership's old address is not kept, even though the row was never revoked"
    );
}

/// Accepts kari's invitation with name `envelope(3)` and address `envelope(4)`.
async fn kari_with_both_fields(pool: &PgPool, fau: &Fau) -> Uuid {
    let t0 = at(T0);
    let token = invite(pool, fau, "kari@example.test").await;
    let target = prepare_acceptance(pool, &token, &verified("kari@example.test"), t0)
        .await
        .unwrap();
    accept_invitation(
        pool,
        AcceptInvitation {
            token,
            acceptor: verified("kari@example.test"),
            admin_end_override: None,
            profile: MemberProfile {
                membership_id: target.membership_id,
                encrypted_display_name: envelope(3),
                encrypted_contact_email: Some(envelope(4)),
            },
        },
        t0,
    )
    .await
    .unwrap()
    .membership_id
}

async fn leave_at_t0(pool: &PgPool, fau: &Fau, m: Uuid) {
    revoke_membership(
        pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: m,
            membership_id: m,
            confirm_no_admin: false,
        },
        at(T0),
    )
    .await
    .unwrap();
}

async fn retained_until(pool: &PgPool, m: Uuid) -> Option<String> {
    sqlx::query_scalar(
        "select to_char(profile_retained_until, 'YYYY-MM-DD') from memberships where id = $1",
    )
    .bind(m)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// With a period of none (#3511), both fields go with the membership, as under D3.
/// Without the clearing in `revoke_membership`, migration 0009's
/// `memberships_*_only_while_current_or_retained` checks refuse the revocation outright.
#[tokio::test]
async fn revoking_a_membership_with_a_period_of_none_clears_both_fields() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    let kari = kari_with_both_fields(&pool, &fau).await;
    sqlx::query("update accounts set retention_months = 0 where email = 'kari@example.test'")
        .execute(&pool)
        .await
        .unwrap();
    leave_at_t0(&pool, &fau, kari).await;
    assert_eq!(fields(&pool, kari).await, (None, None));
    assert_eq!(retained_until(&pool, kari).await, None);
}

/// The default period (3 months, #3511 M2) keeps both fields, hidden, from the day the
/// membership ended (T0, 23 September 2026) until 23 December 2026.
#[tokio::test]
async fn revoking_a_membership_keeps_both_fields_for_the_default_period() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    let kari = kari_with_both_fields(&pool, &fau).await;
    leave_at_t0(&pool, &fau, kari).await;
    assert_eq!(
        fields(&pool, kari).await,
        (
            Some(envelope(3).as_bytes().to_vec()),
            Some(envelope(4).as_bytes().to_vec())
        )
    );
    assert_eq!(
        retained_until(&pool, kari).await.as_deref(),
        Some("2026-12-23")
    );
}
