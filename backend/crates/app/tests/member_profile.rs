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
        let target = prepare_acceptance(&pool, &token, &verified(address))
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

    // Invited back: prepare names the old membership, which acceptance reopens.
    let token = invite(&pool, &fau, "kari@example.test").await;
    let target = prepare_acceptance(&pool, &token, &verified("kari@example.test"))
        .await
        .unwrap();
    assert_eq!(target.membership_id, first.membership_id);

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
    let target = prepare_acceptance(&pool, &token, &verified("kari@example.test"))
        .await
        .unwrap();
    assert_eq!(target.tenant_id, a.tenant_id);
    assert_ne!(
        target.membership_id, in_b.membership_id,
        "a membership in another FAU is never reused"
    );
    let again = prepare_acceptance(&pool, &token, &verified("kari@example.test"))
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
            prepare_acceptance(&pool, token, &verified(reader))
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
    let target = prepare_acceptance(&pool, &token, &verified("kari@example.test"))
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

/// Both fields go with the membership (D3). Without the clearing in `revoke_membership`,
/// migration 0008's `memberships_display_name_only_while_current` or
/// `memberships_contact_email_only_while_current` refuses the revocation outright.
#[tokio::test]
async fn revoking_a_membership_clears_its_name_and_contact_address() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let token = invite(&pool, &fau, "kari@example.test").await;
    let target = prepare_acceptance(&pool, &token, &verified("kari@example.test"))
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
                encrypted_display_name: envelope(3),
                encrypted_contact_email: Some(envelope(4)),
            },
        },
        t0,
    )
    .await
    .unwrap()
    .membership_id;
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: kari,
            membership_id: kari,
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .unwrap();
    assert_eq!(fields(&pool, kari).await, (None, None));
}
