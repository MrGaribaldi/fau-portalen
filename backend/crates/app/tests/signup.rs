//! Signup, activation and pending expiry (flow spec §3, §10 "Signup and activation").

mod common;
use common::membership::*;
use common::TestDb;
use fau_domain::membership::rules::AdminEndError;
use fau_persistence::membership::{
    activate_tenant, create_pending_tenant, expire_pending_tenants, Activation, ExistingFau,
    MembershipError,
};
use uuid::Uuid;

#[tokio::test]
async fn signup_creates_a_pending_fau() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);

    let pending = create_pending_tenant(
        &pool,
        signup(Uuid::now_v7(), "reg@example.test", "leder@example.test", t0),
        t0,
    )
    .await
    .unwrap();

    assert_eq!(pending.expires_at, "2026-09-30T10:00:00Z".parse().unwrap());
    let status: String = sqlx::query_scalar("select status from tenants where id = $1")
        .bind(pending.tenant_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "pending");
    assert_eq!(count(&pool, "select count(*) from tenant_signups").await, 1);
    assert_eq!(audit_count(&pool, "tenant.signup_created").await, 1);
    assert_eq!(
        count(&pool, "select count(*) from memberships").await,
        0,
        "the registrant has no data rights before verifying (spec 3.3)"
    );
}

#[tokio::test]
async fn a_second_signup_for_a_school_is_refused_and_copied_to_ewb() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let school = Uuid::now_v7();

    let first = create_pending_tenant(
        &pool,
        signup(school, "a@example.test", "a@example.test", t0),
        t0,
    )
    .await
    .unwrap();
    let err = create_pending_tenant(
        &pool,
        signup(school, "b@example.test", "b@example.test", t0),
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::SchoolTaken(ExistingFau::Pending));

    activate_tenant(
        &pool,
        Activation {
            tenant_id: first.tenant_id,
            registrant: verified("a@example.test"),
        },
        t0,
    )
    .await
    .unwrap();
    let err = create_pending_tenant(
        &pool,
        signup(school, "c@example.test", "c@example.test", t0),
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::SchoolTaken(ExistingFau::Active));

    // The refusal names nobody: the error carries only the state, and its text is fixed.
    assert_eq!(err.to_string(), "the school already has an FAU");
    // Both collisions reached Erik, and both are audited, although neither created a row.
    assert_eq!(
        outbox_count(&pool, "signup.collision", "fau@ewb-solutions.as").await,
        2
    );
    assert_eq!(audit_count(&pool, "tenant.signup_collision").await, 2);
    assert_eq!(count(&pool, "select count(*) from tenants").await, 1);
    // Both collisions name which FAU already held the school, so a later render can
    // tell the two collisions on the same school apart -- an id, never personal data.
    let existing_ids: Vec<Option<String>> = sqlx::query_scalar(
        "select params->>'existing_tenant_id' from audit_events
          where action = 'tenant.signup_collision' order by occurred_at",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        existing_ids,
        vec![Some(first.tenant_id.to_string()); 2],
        "both collisions name the FAU that already held the school"
    );
}

#[tokio::test]
async fn one_address_holds_at_most_three_pending_faus() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    for _ in 0..3 {
        create_pending_tenant(
            &pool,
            signup(Uuid::now_v7(), "r@example.test", "r@example.test", t0),
            t0,
        )
        .await
        .unwrap();
    }
    let err = create_pending_tenant(
        &pool,
        signup(Uuid::now_v7(), "r@example.test", "r@example.test", t0),
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::TooManyPendingSignups);
    // Another address is unaffected.
    create_pending_tenant(
        &pool,
        signup(Uuid::now_v7(), "s@example.test", "s@example.test", t0),
        t0,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn the_admin_end_date_must_be_one_to_twenty_four_months_away() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let mut s = signup(Uuid::now_v7(), "r@example.test", "r@example.test", t0);
    s.admin_ends_on_exclusive = day(2026, 10, 22);
    assert_eq!(
        create_pending_tenant(&pool, s.clone(), t0)
            .await
            .unwrap_err(),
        MembershipError::AdminEnd(AdminEndError::TooSoon)
    );
    s.admin_ends_on_exclusive = day(2028, 9, 24);
    assert_eq!(
        create_pending_tenant(&pool, s, t0).await.unwrap_err(),
        MembershipError::AdminEnd(AdminEndError::TooLate)
    );
}

#[tokio::test]
async fn a_pending_fau_expires_after_seven_days_and_frees_the_school() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let school = Uuid::now_v7();
    let pending = create_pending_tenant(
        &pool,
        signup(school, "r@example.test", "r@example.test", t0),
        t0,
    )
    .await
    .unwrap();

    assert_eq!(
        expire_pending_tenants(&pool, at("2026-09-30T09:59:59Z"))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        expire_pending_tenants(&pool, at("2026-09-30T10:00:00Z"))
            .await
            .unwrap(),
        1
    );

    assert_eq!(count(&pool, "select count(*) from tenants").await, 0);
    assert_eq!(count(&pool, "select count(*) from tenant_signups").await, 0);
    assert_eq!(audit_count(&pool, "tenant.signup_expired").await, 1);
    let later = at("2026-10-01T10:00:00Z");
    create_pending_tenant(
        &pool,
        signup(school, "ny@example.test", "ny@example.test", later),
        later,
    )
    .await
    .expect("the school is free again");
    assert_eq!(
        activate_tenant(
            &pool,
            Activation {
                tenant_id: pending.tenant_id,
                registrant: verified("r@example.test"),
            },
            later,
        )
        .await
        .unwrap_err(),
        MembershipError::UnknownTenant
    );
}

#[tokio::test]
async fn an_expired_pending_fau_does_not_hold_its_school_before_the_sweep() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let school = Uuid::now_v7();
    let pending = create_pending_tenant(
        &pool,
        signup(school, "r@example.test", "r@example.test", t0),
        t0,
    )
    .await
    .unwrap();

    let late = at("2026-09-30T10:00:00Z");
    assert_eq!(
        activate_tenant(
            &pool,
            Activation {
                tenant_id: pending.tenant_id,
                registrant: verified("r@example.test"),
            },
            late,
        )
        .await
        .unwrap_err(),
        MembershipError::SignupExpired
    );
    let fresh = create_pending_tenant(
        &pool,
        signup(school, "ny@example.test", "ny@example.test", late),
        late,
    )
    .await
    .expect("the stale pending FAU is expired inline");

    // The stale row is really gone, not just no longer blocking the school, and its
    // expiry was audited on this inline path exactly as the scheduled sweep would.
    let old_tenant_still_exists: bool =
        sqlx::query_scalar("select exists(select 1 from tenants where id = $1)")
            .bind(pending.tenant_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!old_tenant_still_exists);
    assert_ne!(fresh.tenant_id, pending.tenant_id);
    assert_eq!(audit_count(&pool, "tenant.signup_expired").await, 1);
}

#[tokio::test]
async fn activation_writes_everything_together() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let pending = create_pending_tenant(
        &pool,
        signup(Uuid::now_v7(), "reg@example.test", "leder@example.test", t0),
        t0,
    )
    .await
    .unwrap();

    let activated = activate_tenant(
        &pool,
        Activation {
            tenant_id: pending.tenant_id,
            registrant: verified("reg@example.test"),
        },
        t0,
    )
    .await
    .unwrap();

    let status: String = sqlx::query_scalar("select status from tenants where id = $1")
        .bind(pending.tenant_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "active");
    let verified_at_set: bool =
        sqlx::query_scalar("select verified_at is not null from accounts where id = $1")
            .bind(activated.account_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(verified_at_set);

    let (name, class, starts, ends): (String, String, String, String) = sqlx::query_as(
        "select r.name, r.capability_class, ra.starts_on::text, ra.ends_on_exclusive::text
           from role_assignments ra join roles r on r.tenant_id = ra.tenant_id and r.id = ra.role_id
          where ra.id = $1",
    )
    .bind(activated.admin_assignment_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (
            name.as_str(),
            class.as_str(),
            starts.as_str(),
            ends.as_str()
        ),
        ("Administrator", "admin", "2026-09-23", "2027-10-01")
    );

    let leader = activated
        .leader_invitation
        .expect("the leader is a different address");
    let (mode, issued_by_null, recipient): (String, bool, String) = sqlx::query_as(
        "select mode, issued_by is null, recipient_email from invitations where id = $1",
    )
    .bind(leader.invitation_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((mode.as_str(), issued_by_null), ("activation", true));
    assert_eq!(recipient, "leder@example.test");
    let (offered_role, offered_end): (uuid::Uuid, String) = sqlx::query_as(
        "select role_id, ends_on_exclusive::text from invitation_roles where invitation_id = $1",
    )
    .bind(leader.invitation_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(offered_role, activated.admin_role_id);
    assert_eq!(
        offered_end, "2027-10-01",
        "the leader is offered the same end date"
    );

    let holder: String =
        sqlx::query_scalar("select holder from recovery_contacts where tenant_id = $1")
            .bind(pending.tenant_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(holder, "ewb");
    assert_eq!(count(&pool, "select count(*) from tenant_signups").await, 0);
    assert_eq!(audit_count(&pool, "tenant.activated").await, 1);
    assert_eq!(
        outbox_count(&pool, "tenant.activated", "fau@ewb-solutions.as").await,
        1
    );
    assert_eq!(
        outbox_count(&pool, "invitation.issued", "leder@example.test").await,
        1
    );
    // Verifying the registrant verified nobody else.
    assert_eq!(
        count(
            &pool,
            "select count(*) from accounts where email = 'leder@example.test'"
        )
        .await,
        0
    );
}

#[tokio::test]
async fn activation_skips_the_leader_invitation_when_the_leader_is_the_registrant() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    active_fau(&pool, "solo@example.test", at(T0)).await;
    assert_eq!(count(&pool, "select count(*) from invitations").await, 0);
    assert_eq!(count(&pool, "select count(*) from memberships").await, 1);
}

#[tokio::test]
async fn activation_is_all_or_nothing() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let pending = create_pending_tenant(
        &pool,
        signup(Uuid::now_v7(), "reg@example.test", "leder@example.test", t0),
        t0,
    )
    .await
    .unwrap();
    // Sabotage the fourth step: a recovery seat already exists, so activation's own
    // insert fails after the status, account, membership, role and invitation writes.
    sqlx::query("insert into recovery_contacts (tenant_id, holder) values ($1, 'ewb')")
        .bind(pending.tenant_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();

    let err = activate_tenant(
        &pool,
        Activation {
            tenant_id: pending.tenant_id,
            registrant: verified("reg@example.test"),
        },
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::Database("sqlstate 23505".to_owned()));

    let status: String = sqlx::query_scalar("select status from tenants where id = $1")
        .bind(pending.tenant_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "pending");
    for table in [
        "accounts",
        "memberships",
        "roles",
        "role_assignments",
        "invitations",
        "invitation_roles",
        "outbox",
    ] {
        assert_eq!(
            count(&pool, &format!("select count(*) from {table}")).await,
            0,
            "{table} kept a row from a failed activation"
        );
    }
    assert_eq!(audit_count(&pool, "tenant.activated").await, 0);
    // The leader invitation is written (and audited) as the third-to-last step, before
    // the sabotaged recovery-contact insert -- proving the rollback undoes that too, not
    // just the final failing statement. A count of 0 alone for "tenant.activated" would
    // hold even without a real transaction, since that entry is the very last write.
    assert_eq!(audit_count(&pool, "invitation.issued").await, 0);
    assert_eq!(count(&pool, "select count(*) from tenant_signups").await, 1);
}

#[tokio::test]
async fn activation_requires_the_registrants_own_address() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let pending = create_pending_tenant(
        &pool,
        signup(Uuid::now_v7(), "reg@example.test", "reg@example.test", t0),
        t0,
    )
    .await
    .unwrap();
    let err = activate_tenant(
        &pool,
        Activation {
            tenant_id: pending.tenant_id,
            registrant: verified("someone-else@example.test"),
        },
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::RegistrantMismatch);
}

#[tokio::test]
async fn a_registrant_with_an_account_elsewhere_keeps_one_account() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let first = active_fau(&pool, "same@example.test", at(T0)).await;
    let second = active_fau(&pool, "same@example.test", at(T0)).await;
    assert_eq!(first.admin_account_id, second.admin_account_id);
    assert_eq!(count(&pool, "select count(*) from accounts").await, 1);
    assert_eq!(count(&pool, "select count(*) from memberships").await, 2);
}

#[tokio::test]
async fn expired_but_unswept_signups_do_not_count_toward_the_pending_limit() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    for _ in 0..3 {
        create_pending_tenant(
            &pool,
            signup(Uuid::now_v7(), "r@example.test", "r@example.test", t0),
            t0,
        )
        .await
        .unwrap();
    }
    // Past every one of those three signups' 7-day expiry. None of the calls below
    // touches any of their schools, so the rows are still there, just not swept yet.
    let later = at("2026-10-01T00:00:00Z");
    create_pending_tenant(
        &pool,
        signup(Uuid::now_v7(), "r@example.test", "r@example.test", later),
        later,
    )
    .await
    .expect("expired-but-unswept signups do not count toward the pending limit");
}

#[tokio::test]
async fn only_one_of_several_concurrent_signups_for_one_school_succeeds() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let school = Uuid::now_v7();

    let mut handles = Vec::new();
    for i in 0..8 {
        let pool = pool.clone();
        let address = format!("r{i}@example.test");
        handles.push(tokio::spawn(async move {
            create_pending_tenant(&pool, signup(school, &address, &address, t0), t0).await
        }));
    }

    let (mut ok, mut taken) = (0, 0);
    for handle in handles {
        match handle.await.expect("task did not panic") {
            Ok(_) => ok += 1,
            Err(MembershipError::SchoolTaken(ExistingFau::Pending)) => taken += 1,
            Err(other) => panic!("unexpected error from a concurrent signup: {other:?}"),
        }
    }
    assert_eq!(
        ok, 1,
        "exactly one concurrent signup places the school's FAU"
    );
    assert_eq!(taken, 7);
    assert_eq!(count(&pool, "select count(*) from tenants").await, 1);
}

#[tokio::test]
async fn only_three_of_several_concurrent_signups_for_one_address_succeed() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let address = "many@example.test";

    let mut handles = Vec::new();
    for _ in 0..5 {
        let pool = pool.clone();
        let school = Uuid::now_v7();
        handles.push(tokio::spawn(async move {
            create_pending_tenant(&pool, signup(school, address, address, t0), t0).await
        }));
    }

    let (mut ok, mut too_many) = (0, 0);
    for handle in handles {
        match handle.await.expect("task did not panic") {
            Ok(_) => ok += 1,
            Err(MembershipError::TooManyPendingSignups) => too_many += 1,
            Err(other) => panic!("unexpected error from a concurrent signup: {other:?}"),
        }
    }
    assert_eq!(
        ok, 3,
        "the advisory lock serialises signups from one address"
    );
    assert_eq!(too_many, 2);
}

#[tokio::test]
async fn activating_an_already_active_fau_is_refused() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let pending = create_pending_tenant(
        &pool,
        signup(Uuid::now_v7(), "reg@example.test", "reg@example.test", t0),
        t0,
    )
    .await
    .unwrap();
    activate_tenant(
        &pool,
        Activation {
            tenant_id: pending.tenant_id,
            registrant: verified("reg@example.test"),
        },
        t0,
    )
    .await
    .unwrap();

    let err = activate_tenant(
        &pool,
        Activation {
            tenant_id: pending.tenant_id,
            registrant: verified("reg@example.test"),
        },
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::TenantNotPending);
}

#[tokio::test]
async fn activating_a_frozen_fau_is_refused() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let pending = create_pending_tenant(
        &pool,
        signup(Uuid::now_v7(), "reg@example.test", "reg@example.test", t0),
        t0,
    )
    .await
    .unwrap();
    // The freeze flow itself is out of scope (ADR-003 7a); arranged directly as
    // superuser, the way a frozen FAU would look once that flow exists.
    sqlx::query("update tenants set frozen_at = $1::timestamptz where id = $2")
        .bind(T0)
        .bind(pending.tenant_id)
        .execute(&db.admin_pool())
        .await
        .unwrap();

    let err = activate_tenant(
        &pool,
        Activation {
            tenant_id: pending.tenant_id,
            registrant: verified("reg@example.test"),
        },
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::TenantFrozen);

    let status: String = sqlx::query_scalar("select status from tenants where id = $1")
        .bind(pending.tenant_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "pending", "a refused activation changes nothing");
}

#[tokio::test]
async fn activating_with_a_disabled_account_is_refused() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let pending = create_pending_tenant(
        &pool,
        signup(Uuid::now_v7(), "reg@example.test", "reg@example.test", t0),
        t0,
    )
    .await
    .unwrap();
    // The account already exists (e.g. from an earlier membership elsewhere) and was
    // disabled; activation's own account upsert must find it, not create a fresh one.
    sqlx::query("insert into accounts (id, email, disabled_at) values ($1, $2, $3::timestamptz)")
        .bind(Uuid::now_v7())
        .bind("reg@example.test")
        .bind(T0)
        .execute(&db.admin_pool())
        .await
        .unwrap();

    let err = activate_tenant(
        &pool,
        Activation {
            tenant_id: pending.tenant_id,
            registrant: verified("reg@example.test"),
        },
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::AccountDisabled);

    let status: String = sqlx::query_scalar("select status from tenants where id = $1")
        .bind(pending.tenant_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "pending", "a refused activation changes nothing");
    assert_eq!(count(&pool, "select count(*) from memberships").await, 0);
}
