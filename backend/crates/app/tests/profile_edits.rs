//! Editing the directory fields (groups design §4.1; #3502): a member edits their own name,
//! an admin corrects that of anyone still active, and only the member sets their contact
//! address. An ended membership takes no name (D3). Every refusal a non-admin can reach is
//! the same `NotAuthorized`, whatever the target is.

mod common;
use common::groups::*;
use common::membership::*;
use common::TestDb;
use fau_crypto::Ciphertext;
use fau_domain::membership::vocabulary::CapabilityClass;
use fau_persistence::membership::*;
use sqlx::PgPool;
use uuid::Uuid;

fn envelope(marker: u8) -> Ciphertext {
    let mut v = vec![1u8, marker];
    v.extend_from_slice(&[0u8; 40]);
    Ciphertext::from_stored(v)
}

fn year() -> fau_domain::membership::period::Period {
    period(day(2026, 9, 1), day(2027, 9, 1))
}

async fn member(pool: &PgPool, fau: &Fau, address: &str) -> Uuid {
    add_member(
        pool,
        fau,
        address,
        new_role("Medlem", CapabilityClass::Member),
        year(),
        at(T0),
    )
    .await
    .membership_id
}

async fn name_of(pool: &PgPool, m: Uuid) -> Option<Vec<u8>> {
    sqlx::query_scalar("select encrypted_display_name from memberships where id = $1")
        .bind(m)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn contact_of(pool: &PgPool, m: Uuid) -> Option<Vec<u8>> {
    sqlx::query_scalar("select encrypted_contact_email from memberships where id = $1")
        .bind(m)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn rename(
    pool: &PgPool,
    tenant_id: Uuid,
    actor: Uuid,
    target: Uuid,
    marker: u8,
) -> Result<(), MembershipError> {
    set_display_name(
        pool,
        SetDisplayName {
            tenant_id,
            actor_membership_id: actor,
            membership_id: target,
            encrypted_display_name: envelope(marker),
        },
        at(T0),
    )
    .await
}

async fn last_audit(pool: &PgPool, action: &str) -> serde_json::Value {
    let params: String = sqlx::query_scalar(
        "select params::text from audit_events where action = $1 order by occurred_at desc, id desc limit 1",
    )
    .bind(action)
    .fetch_one(pool)
    .await
    .unwrap();
    serde_json::from_str(&params).unwrap()
}

#[tokio::test]
async fn a_member_edits_their_own_name_and_the_audit_holds_no_name() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    let kari = member(&pool, &fau, "kari@example.test").await;
    rename(&pool, fau.tenant_id, kari, kari, 42).await.unwrap();
    assert_eq!(
        name_of(&pool, kari).await.as_deref(),
        Some(envelope(42).as_bytes())
    );
    assert_eq!(
        last_audit(&pool, "membership.display_name_changed").await,
        serde_json::json!({ "by_admin": false })
    );
}

/// Mutation check: drop the `membership_ended` test from `set_display_name`, and the
/// revoked row, retained under #3511, is renamed to `envelope(9)`, as is the ran-out row.
#[tokio::test]
async fn an_admin_corrects_an_active_members_name_and_an_ended_membership_takes_none() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    let kari = member(&pool, &fau, "kari@example.test").await;
    rename(&pool, fau.tenant_id, fau.admin_membership_id, kari, 8)
        .await
        .unwrap();
    assert_eq!(
        name_of(&pool, kari).await.as_deref(),
        Some(envelope(8).as_bytes())
    );
    assert_eq!(
        last_audit(&pool, "membership.display_name_changed").await,
        serde_json::json!({ "by_admin": true })
    );

    // Active before it has begun: every role is still to come.
    let next_year = add_member(
        &pool,
        &fau,
        "neste@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 10, 1), day(2027, 9, 1)),
        at(T0),
    )
    .await
    .membership_id;
    rename(&pool, fau.tenant_id, fau.admin_membership_id, next_year, 6)
        .await
        .unwrap();

    // Revoked: a correction is refused. Under #3511 the name is kept, hidden, for the
    // default period, and the refused correction leaves it untouched.
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: kari,
            confirm_no_admin: false,
        },
        at(T0),
    )
    .await
    .unwrap();
    assert_eq!(
        rename(&pool, fau.tenant_id, fau.admin_membership_id, kari, 9).await,
        Err(MembershipError::MembershipEnded)
    );
    assert_eq!(
        name_of(&pool, kari).await.as_deref(),
        Some(envelope(8).as_bytes())
    );

    // Ran out, and the sweep has not reached it yet: the old name stays untouched until
    // it does, and no new one is accepted.
    let month = add_member(
        &pool,
        &fau,
        "maaned@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 1), day(2026, 10, 1)),
        at(T0),
    )
    .await
    .membership_id;
    let before = name_of(&pool, month).await;
    assert_eq!(
        set_display_name(
            &pool,
            SetDisplayName {
                tenant_id: fau.tenant_id,
                actor_membership_id: fau.admin_membership_id,
                membership_id: month,
                encrypted_display_name: envelope(9),
            },
            at("2026-10-01T10:00:00Z"),
        )
        .await,
        Err(MembershipError::MembershipEnded)
    );
    assert_eq!(name_of(&pool, month).await, before);

    // Only an admin learns that an id does not exist.
    assert_eq!(
        rename(
            &pool,
            fau.tenant_id,
            fau.admin_membership_id,
            Uuid::now_v7(),
            8
        )
        .await,
        Err(MembershipError::UnknownMembership)
    );
}

/// Mutation check: drop the `is_admin_today` branch, or the standing check on the own
/// path, and one of these rows passes.
#[tokio::test]
async fn nobody_else_may_edit_a_name_and_every_refusal_looks_the_same() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let other = active_fau(&pool, "admin-b@example.test", t0).await;
    let kari = member(&pool, &fau, "kari@example.test").await;
    let ola = member(&pool, &fau, "ola@example.test").await;
    let group = seed_group(
        &pool,
        &fau,
        fau_domain::membership::vocabulary::Visibility::Open,
    )
    .await;
    let guest = add_guest(&pool, &fau, "gjest@example.test", group, year(), t0)
        .await
        .membership_id;
    let ended = member(&pool, &fau, "ferdig@example.test").await;
    revoke(&pool, &fau, live_assignment(&pool, ended).await, t0).await;
    let outsider = member(&pool, &other, "utenfor@example.test").await;

    let before = name_of(&pool, kari).await;
    for (actor, target, why) in [
        (ola, kari, "a member, on another member"),
        (guest, kari, "a guest, on a member"),
        (ola, Uuid::now_v7(), "a member, on an unknown id"),
        (ola, outsider, "a member, on another FAU's membership"),
        (ended, ended, "no standing, on themself"),
        (outsider, kari, "another FAU's member, on this FAU's"),
    ] {
        assert_eq!(
            rename(&pool, fau.tenant_id, actor, target, 99).await,
            Err(MembershipError::NotAuthorized),
            "{why}"
        );
    }
    assert_eq!(name_of(&pool, kari).await, before);

    // Another FAU's admin cannot reach into this one, either way round.
    assert_eq!(
        rename(&pool, fau.tenant_id, other.admin_membership_id, kari, 99).await,
        Err(MembershipError::NotAuthorized)
    );
    assert_eq!(
        rename(&pool, other.tenant_id, other.admin_membership_id, kari, 99).await,
        Err(MembershipError::UnknownMembership)
    );
    assert_eq!(name_of(&pool, kari).await, before);

    // A guest may edit their own name.
    rename(&pool, fau.tenant_id, guest, guest, 5).await.unwrap();
}

#[tokio::test]
async fn a_disabled_account_edits_nothing() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    let kari = add_member(
        &pool,
        &fau,
        "kari@example.test",
        new_role("Medlem", CapabilityClass::Member),
        year(),
        at(T0),
    )
    .await;
    sqlx::query("update accounts set disabled_at = now() where id = $1")
        .bind(kari.account_id)
        .execute(&pool)
        .await
        .unwrap();
    let m = kari.membership_id;
    assert_eq!(
        rename(&pool, fau.tenant_id, m, m, 1).await,
        Err(MembershipError::NotAuthorized)
    );
    assert_eq!(
        set_contact_email(
            &pool,
            SetContactEmail {
                tenant_id: fau.tenant_id,
                membership_id: m,
                encrypted_contact_email: Some(envelope(1)),
            },
            at(T0),
        )
        .await,
        Err(MembershipError::NotAuthorized)
    );
}

#[tokio::test]
async fn only_the_member_sets_or_clears_their_contact_address() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    let kari = member(&pool, &fau, "kari@example.test").await;
    let set = |m: Uuid, v: Option<Ciphertext>| {
        let pool = pool.clone();
        async move {
            set_contact_email(
                &pool,
                SetContactEmail {
                    tenant_id: fau.tenant_id,
                    membership_id: m,
                    encrypted_contact_email: v,
                },
                at(T0),
            )
            .await
        }
    };
    set(kari, Some(envelope(4))).await.unwrap();
    assert_eq!(
        contact_of(&pool, kari).await.as_deref(),
        Some(envelope(4).as_bytes())
    );
    assert_eq!(
        last_audit(&pool, "membership.contact_email_changed").await,
        serde_json::json!({ "cleared": false })
    );
    set(kari, None).await.unwrap();
    assert_eq!(contact_of(&pool, kari).await, None);
    assert_eq!(
        last_audit(&pool, "membership.contact_email_changed").await,
        serde_json::json!({ "cleared": true })
    );
    assert_eq!(
        set(Uuid::now_v7(), Some(envelope(4))).await,
        Err(MembershipError::NotAuthorized)
    );
    assert_eq!(
        set(kari, Some(Ciphertext::from_stored(vec![1u8; 41]))).await,
        Err(MembershipError::ContactEmailMalformed)
    );
}

#[tokio::test]
async fn a_frozen_fau_takes_no_new_name_or_address_but_lets_one_be_cleared() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    let kari = member(&pool, &fau, "kari@example.test").await;
    set_contact_email(
        &pool,
        SetContactEmail {
            tenant_id: fau.tenant_id,
            membership_id: kari,
            encrypted_contact_email: Some(envelope(4)),
        },
        at(T0),
    )
    .await
    .unwrap();
    sqlx::query("update tenants set frozen_at = now() where id = $1")
        .bind(fau.tenant_id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        rename(&pool, fau.tenant_id, kari, kari, 1).await,
        Err(MembershipError::TenantFrozen)
    );
    let write = |v: Option<Ciphertext>| {
        let pool = pool.clone();
        async move {
            set_contact_email(
                &pool,
                SetContactEmail {
                    tenant_id: fau.tenant_id,
                    membership_id: kari,
                    encrypted_contact_email: v,
                },
                at(T0),
            )
            .await
        }
    };
    assert_eq!(
        write(Some(envelope(5))).await,
        Err(MembershipError::TenantFrozen)
    );
    write(None).await.unwrap();
    assert_eq!(contact_of(&pool, kari).await, None);
}

/// Q4 (pre-flight scan gap): the existing `nobody_else…` test's `ended` actor only had
/// its *role assignment* revoked; this shows the same refusal when the *membership
/// itself* is revoked outright (`revoke_membership`), which also clears the name.
///
/// Mutation check: skip the own-path's `membership_access` standing check (e.g. replace
/// it with a bare `actor == target` test), and this test fails: the assertion expects
/// `NotAuthorized`, but the row-state check that runs afterwards still catches the
/// revoked row and returns `MembershipEnded` instead -- a different, and here wrong,
/// refusal.
#[tokio::test]
async fn a_revoked_membership_cannot_edit_its_own_name() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let kari = member(&pool, &fau, "kari@example.test").await;
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: kari,
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        rename(&pool, fau.tenant_id, kari, kari, 1).await,
        Err(MembershipError::NotAuthorized)
    );
}

/// Q4 (pre-flight scan gap): `SetContactEmail` carries no actor field -- the caller is
/// trusted (at the HTTP layer) to always name their own membership -- so the standing
/// checks that matter here are cross-tenant, revoked and ended, not "someone else's
/// membership". All three must come back `NotAuthorized`.
///
/// Mutation check: drop `membership_access`'s `m.tenant_id = $1` join condition, or its
/// `capability == None` test, and one of these three rows returns `Ok` instead.
#[tokio::test]
async fn set_contact_email_refuses_a_cross_tenant_revoked_or_ended_membership() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let other = active_fau(&pool, "admin-b@example.test", t0).await;
    let outsider = member(&pool, &other, "utenfor@example.test").await;

    let ended = member(&pool, &fau, "ferdig@example.test").await;
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: ended,
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .unwrap();

    let ran_out = add_member(
        &pool,
        &fau,
        "maaned@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 1), day(2026, 10, 1)),
        t0,
    )
    .await
    .membership_id;

    async fn set(
        pool: &PgPool,
        tenant_id: Uuid,
        m: Uuid,
        at_moment: fau_domain::time::Moment,
    ) -> Result<(), MembershipError> {
        set_contact_email(
            pool,
            SetContactEmail {
                tenant_id,
                membership_id: m,
                encrypted_contact_email: Some(envelope(1)),
            },
            at_moment,
        )
        .await
    }

    // Cross-tenant: another FAU's membership id, under this FAU's tenant_id.
    assert_eq!(
        set(&pool, fau.tenant_id, outsider, t0).await,
        Err(MembershipError::NotAuthorized),
        "cross-tenant"
    );
    // Revoked.
    assert_eq!(
        set(&pool, fau.tenant_id, ended, t0).await,
        Err(MembershipError::NotAuthorized),
        "revoked"
    );
    // Ended: roles ran out, ahead of the sweep.
    assert_eq!(
        set(&pool, fau.tenant_id, ran_out, at("2026-10-01T10:00:00Z")).await,
        Err(MembershipError::NotAuthorized),
        "ended"
    );
}

/// Controller ruling (Task 5 review): `set_contact_email` had no `name_erased_at` check,
/// so writing to (or clearing) an erased membership hit migration 0008's
/// `memberships_erasure_leaves_nothing` check constraint -- a raw database error -- instead
/// of the typed refusal every other write on an erased row returns.
///
/// Mutation check: drop the `name_erased_at` check from `set_contact_email`, and setting a
/// value below returns `MembershipError::Database(..)` (sqlstate 23514, migration 0008's
/// check constraint) instead of `MembershipErased`.
#[tokio::test]
async fn set_contact_email_on_an_erased_membership_is_refused_before_the_database_constraint() {
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
    .await;
    assert_eq!(
        erase_member_names(&pool, kari.account_id, t0)
            .await
            .unwrap(),
        1
    );
    let set = |v: Option<Ciphertext>| {
        let pool = pool.clone();
        async move {
            set_contact_email(
                &pool,
                SetContactEmail {
                    tenant_id: fau.tenant_id,
                    membership_id: kari.membership_id,
                    encrypted_contact_email: v,
                },
                t0,
            )
            .await
        }
    };
    assert_eq!(
        set(Some(envelope(1))).await,
        Err(MembershipError::MembershipErased),
        "setting a value"
    );
    assert_eq!(
        set(None).await,
        Err(MembershipError::MembershipErased),
        "clearing to None"
    );
    assert_eq!(contact_of(&pool, kari.membership_id).await, None);
}
