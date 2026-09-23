//! Migration 0003: the membership foundation's constraints, proven in SQL before any
//! Rust depends on them (flow spec §9). Every check and uniqueness constraint 0003
//! defines is exercised here (or in schema_review.rs, for the standing cross-cutting
//! guards) by a test that asserts its SQLSTATE and constraint name, with a valid
//! positive control alongside it where that is cheap. Where a constraint cannot be
//! violated independently of another, the test that covers both says so.

mod common;
use common::TestDb;
use uuid::Uuid;

fn sqlstate(err: &sqlx::Error) -> Option<String> {
    err.as_database_error()
        .and_then(|e| e.code())
        .map(|c| c.into_owned())
}

fn constraint_name(err: &sqlx::Error) -> Option<String> {
    err.as_database_error()
        .and_then(|e| e.constraint())
        .map(|c| c.to_owned())
}

async fn tenant(
    pool: &sqlx::PgPool,
    status: &str,
    school: Option<Uuid>,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query("insert into tenants (id, name, status, school_id) values ($1, 'FAU', $2, $3)")
        .bind(id)
        .bind(status)
        .bind(school)
        .execute(pool)
        .await
        .map(|_| id)
}

/// An active tenant with one membership and one role, for tables that reference them.
async fn seeded(pool: &sqlx::PgPool) -> (Uuid, Uuid, Uuid) {
    let t = tenant(pool, "active", None).await.unwrap();
    let (acc, m, r) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    sqlx::query("insert into accounts (id, email) values ($1, $2)")
        .bind(acc)
        .bind(format!("{acc}@example.test"))
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("insert into memberships (tenant_id, id, account_id) values ($1, $2, $3)")
        .bind(t)
        .bind(m)
        .bind(acc)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "insert into roles (tenant_id, id, name, capability_class) values ($1, $2, 'Leder', 'admin')",
    )
    .bind(t)
    .bind(r)
    .execute(pool)
    .await
    .unwrap();
    (t, m, r)
}

async fn invitation(
    pool: &sqlx::PgPool,
    t: Uuid,
    mode: &str,
    issued_by: Option<Uuid>,
    hash: Vec<u8>,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into invitations
           (tenant_id, id, token_hash, mode, recipient_email, issued_by, expires_at, created_at)
         values ($1, $2, $3, $4, 'ny@example.test', $5, now() + interval '14 days', now())",
    )
    .bind(t)
    .bind(id)
    .bind(hash)
    .bind(mode)
    .bind(issued_by)
    .execute(pool)
    .await
    .map(|_| id)
}

#[tokio::test]
async fn one_live_fau_per_school() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let school = Uuid::now_v7();

    tenant(&pool, "pending", Some(school)).await.unwrap();
    for status in ["pending", "active"] {
        let err = tenant(&pool, status, Some(school))
            .await
            .expect_err("a second live FAU for one school was accepted");
        assert_eq!(sqlstate(&err).as_deref(), Some("23505"), "{err:?}");
        assert_eq!(
            constraint_name(&err).as_deref(),
            Some("tenants_one_live_per_school")
        );
    }
    // A closed FAU does not block, and FAU-er without a school yet never collide.
    tenant(&pool, "closed", Some(school)).await.unwrap();
    tenant(&pool, "active", None).await.unwrap();
    tenant(&pool, "active", None).await.unwrap();
}

#[tokio::test]
async fn deleting_a_pending_tenant_takes_its_signup_row() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t = tenant(&pool, "pending", Some(Uuid::now_v7()))
        .await
        .unwrap();
    sqlx::query(
        "insert into tenant_signups
           (tenant_id, registrant_email, leader_email, admin_ends_on_exclusive, expires_at)
         values ($1, 'r@example.test', 'l@example.test', date '2027-10-01', now())",
    )
    .bind(t)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query("delete from tenants where id = $1")
        .bind(t)
        .execute(&pool)
        .await
        .expect("the runtime role deletes an expired pending tenant");
    let left: i64 = sqlx::query_scalar("select count(*) from tenant_signups")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn invitation_token_hashes_are_unique_sha256_digests() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t, m, _) = seeded(&pool).await;

    invitation(&pool, t, "normal", Some(m), vec![7; 32])
        .await
        .unwrap();
    let dup = invitation(&pool, t, "normal", Some(m), vec![7; 32])
        .await
        .expect_err("duplicate token hash accepted");
    assert_eq!(
        constraint_name(&dup).as_deref(),
        Some("invitations_token_hash_unique")
    );
    let short = invitation(&pool, t, "normal", Some(m), vec![1; 16])
        .await
        .expect_err("a 16-byte hash accepted");
    assert_eq!(
        constraint_name(&short).as_deref(),
        Some("invitation_token_hash_is_sha256")
    );
}

#[tokio::test]
async fn invitation_issuer_must_match_its_mode() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t, m, _) = seeded(&pool).await;

    invitation(&pool, t, "activation", None, vec![1; 32])
        .await
        .unwrap();
    let normal_without_issuer = invitation(&pool, t, "normal", None, vec![2; 32])
        .await
        .expect_err("a normal invitation without an issuer was accepted");
    assert_eq!(
        constraint_name(&normal_without_issuer).as_deref(),
        Some("invitation_issuer_matches_mode")
    );
    let activation_with_issuer = invitation(&pool, t, "activation", Some(m), vec![3; 32])
        .await
        .expect_err("an activation invitation with an issuer was accepted");
    assert_eq!(
        constraint_name(&activation_with_issuer).as_deref(),
        Some("invitation_issuer_matches_mode")
    );
    let handover_without_grant = invitation(&pool, t, "handover", Some(m), vec![4; 32])
        .await
        .expect_err("a handover invitation without a grant was accepted");
    assert_eq!(
        constraint_name(&handover_without_grant).as_deref(),
        Some("invitation_handover_has_grant")
    );
}

#[tokio::test]
async fn an_invitation_role_cannot_point_at_another_tenants_role() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t1, m1, r1) = seeded(&pool).await;
    let (_t2, _m2, r2) = seeded(&pool).await;
    let inv = invitation(&pool, t1, "normal", Some(m1), vec![5; 32])
        .await
        .unwrap();

    let insert = |role: Uuid| {
        sqlx::query(
            "insert into invitation_roles (tenant_id, invitation_id, role_id, starts_on, ends_on_exclusive)
             values ($1, $2, $3, date '2026-10-01', date '2027-10-01')",
        )
        .bind(t1)
        .bind(inv)
        .bind(role)
        .execute(&pool)
    };
    insert(r1).await.expect("a same-tenant role is accepted");
    let err = insert(r2)
        .await
        .expect_err("a cross-tenant role was accepted");
    assert_eq!(sqlstate(&err).as_deref(), Some("23503"), "{err:?}");
}

#[tokio::test]
async fn access_request_constraints() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t, _, _) = seeded(&pool).await;

    let insert = |email: &'static str, message: String| {
        sqlx::query(
            "insert into access_requests
               (tenant_id, id, kind, requester_email, invitee_email, message, created_on, created_at)
             values ($1, $2, 'access', $3, $3, $4, current_date, now())",
        )
        .bind(t)
        .bind(Uuid::now_v7())
        .bind(email)
        .bind(message)
        .execute(&pool)
    };
    insert("a@example.test", "ø".repeat(500)).await.unwrap();
    let dup = insert("a@example.test", "igjen".into())
        .await
        .expect_err("a second open request from one address was accepted");
    assert_eq!(
        constraint_name(&dup).as_deref(),
        Some("access_requests_one_open_per_address")
    );
    let long = insert("b@example.test", "ø".repeat(501))
        .await
        .expect_err("a 501-character message was accepted");
    assert_eq!(
        constraint_name(&long).as_deref(),
        Some("access_request_message_is_short")
    );

    let replacement_without_role = sqlx::query(
        "insert into access_requests
           (tenant_id, id, kind, requester_email, invitee_email, created_on, created_at)
         values ($1, $2, 'replacement', 'c@example.test', 'd@example.test', current_date, now())",
    )
    .bind(t)
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await
    .expect_err("a replacement without proposer and role was accepted");
    assert_eq!(
        constraint_name(&replacement_without_role).as_deref(),
        Some("replacement_names_proposer_and_role")
    );
}

#[tokio::test]
async fn one_handover_grant_per_source_assignment() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t, m, r) = seeded(&pool).await;
    let ra = Uuid::now_v7();
    sqlx::query(
        "insert into role_assignments (tenant_id, id, membership_id, role_id, starts_on, ends_on_exclusive)
         values ($1, $2, $3, $4, date '2026-10-01', date '2027-10-01')",
    )
    .bind(t)
    .bind(ra)
    .bind(m)
    .bind(r)
    .execute(&pool)
    .await
    .unwrap();

    let grant = || {
        sqlx::query(
            "insert into handover_grants (tenant_id, id, source_assignment_id, starts_on, ends_on_exclusive)
             values ($1, $2, $3, date '2027-10-01', date '2028-04-01')",
        )
        .bind(t)
        .bind(Uuid::now_v7())
        .bind(ra)
        .execute(&pool)
    };
    grant().await.unwrap();
    let err = grant()
        .await
        .expect_err("a second grant from one assignment was accepted");
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("handover_grants_one_per_source")
    );
}

#[tokio::test]
async fn a_school_rep_holds_the_seat_only_once_confirmed() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t, _, _) = seeded(&pool).await;

    let err = sqlx::query(
        "insert into recovery_contacts (tenant_id, holder, nomination_status, nominee_email)
         values ($1, 'school_rep', 'nominated', 'rektor@skole.example.test')",
    )
    .bind(t)
    .execute(&pool)
    .await
    .expect_err("an unconfirmed nominee took the seat");
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("recovery_school_rep_is_confirmed")
    );

    let unrecorded = sqlx::query(
        "insert into recovery_contacts (tenant_id, holder, nomination_status, nominee_email)
         values ($1, 'school_rep', 'confirmed', 'rektor@skole.example.test')",
    )
    .bind(t)
    .execute(&pool)
    .await
    .expect_err("a confirmation without its verification record was accepted");
    assert_eq!(
        constraint_name(&unrecorded).as_deref(),
        Some("recovery_confirmation_is_recorded")
    );

    sqlx::query("insert into recovery_contacts (tenant_id, holder) values ($1, 'ewb')")
        .bind(t)
        .execute(&pool)
        .await
        .expect("EWB in the seat with no nomination is the activation default");
}

#[tokio::test]
async fn audit_and_outbox_codes_are_constrained() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;

    let bad_action = sqlx::query(
        "insert into audit_events (id, actor_kind, action, subject_type, subject_id, occurred_at)
         values ($1, 'system', 'Kari ble lagt til', 'tenant', $1, now())",
    )
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await
    .expect_err("a sentence was accepted as an action code");
    assert_eq!(
        constraint_name(&bad_action).as_deref(),
        Some("audit_action_is_a_code")
    );

    let big_params = sqlx::query(
        "insert into outbox (id, template, recipient_email, params, created_at)
         values ($1, 'invitation.issued', 'x@example.test', jsonb_build_object('pad', repeat('x', 3000)), now())",
    )
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await
    .expect_err("an oversized parameter object was accepted");
    assert_eq!(
        constraint_name(&big_params).as_deref(),
        Some("outbox_params_are_small")
    );
}

#[tokio::test]
async fn the_contract_version_is_three() {
    let db = TestDb::migrated().await;
    let version: i32 = sqlx::query_scalar("select max(version) from schema_contract")
        .fetch_one(&db.app_pool().await)
        .await
        .unwrap();
    assert_eq!(version, 3);
}

// The tests below round out coverage of 0003's remaining check constraints -- every
// one not already exercised above or by schema_review.rs's guards -- each proving its
// own SQLSTATE (23514) and constraint name, with a valid positive control inserted
// first wherever that control is cheap to build.

#[tokio::test]
async fn invitation_recovery_names_seat_matches_its_mode() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t, _, _) = seeded(&pool).await;

    // Positive control: a recovery invitation names the seat that issued it.
    sqlx::query(
        "insert into invitations
           (tenant_id, id, token_hash, mode, recipient_email, recovery_holder, expires_at, created_at)
         values ($1, $2, $3, 'recovery', 'ny@example.test', 'ewb', now() + interval '14 days', now())",
    )
    .bind(t)
    .bind(Uuid::now_v7())
    .bind(vec![10u8; 32])
    .execute(&pool)
    .await
    .expect("a recovery invitation naming its seat is accepted");

    let recovery_without_seat = sqlx::query(
        "insert into invitations
           (tenant_id, id, token_hash, mode, recipient_email, expires_at, created_at)
         values ($1, $2, $3, 'recovery', 'ny@example.test', now() + interval '14 days', now())",
    )
    .bind(t)
    .bind(Uuid::now_v7())
    .bind(vec![11u8; 32])
    .execute(&pool)
    .await
    .expect_err("a recovery invitation without a seat was accepted");
    assert_eq!(sqlstate(&recovery_without_seat).as_deref(), Some("23514"));
    assert_eq!(
        constraint_name(&recovery_without_seat).as_deref(),
        Some("invitation_recovery_names_seat")
    );

    let non_recovery_with_seat = sqlx::query(
        "insert into invitations
           (tenant_id, id, token_hash, mode, recipient_email, recovery_holder, expires_at, created_at)
         values ($1, $2, $3, 'activation', 'ny@example.test', 'ewb', now() + interval '14 days', now())",
    )
    .bind(t)
    .bind(Uuid::now_v7())
    .bind(vec![12u8; 32])
    .execute(&pool)
    .await
    .expect_err("a non-recovery invitation naming a seat was accepted");
    assert_eq!(sqlstate(&non_recovery_with_seat).as_deref(), Some("23514"));
    assert_eq!(
        constraint_name(&non_recovery_with_seat).as_deref(),
        Some("invitation_recovery_names_seat")
    );
}

#[tokio::test]
async fn invitation_acceptance_fields_are_complete_together() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t, m, _) = seeded(&pool).await;

    // Positive control: accepted_at and accepted_membership_id set together.
    let id1 = invitation(&pool, t, "normal", Some(m), vec![20; 32])
        .await
        .unwrap();
    sqlx::query(
        "update invitations set accepted_at = now(), accepted_membership_id = $3
         where tenant_id = $1 and id = $2",
    )
    .bind(t)
    .bind(id1)
    .bind(m)
    .execute(&pool)
    .await
    .expect("accepted_at and accepted_membership_id set together is accepted");

    let id2 = invitation(&pool, t, "normal", Some(m), vec![21; 32])
        .await
        .unwrap();
    let half =
        sqlx::query("update invitations set accepted_at = now() where tenant_id = $1 and id = $2")
            .bind(t)
            .bind(id2)
            .execute(&pool)
            .await
            .expect_err("accepted_at without accepted_membership_id was accepted");
    assert_eq!(sqlstate(&half).as_deref(), Some("23514"));
    assert_eq!(
        constraint_name(&half).as_deref(),
        Some("invitation_acceptance_is_complete")
    );

    let id3 = invitation(&pool, t, "normal", Some(m), vec![22; 32])
        .await
        .unwrap();
    let other_half = sqlx::query(
        "update invitations set accepted_membership_id = $3 where tenant_id = $1 and id = $2",
    )
    .bind(t)
    .bind(id3)
    .bind(m)
    .execute(&pool)
    .await
    .expect_err("accepted_membership_id without accepted_at was accepted");
    assert_eq!(sqlstate(&other_half).as_deref(), Some("23514"));
    assert_eq!(
        constraint_name(&other_half).as_deref(),
        Some("invitation_acceptance_is_complete")
    );
}

#[tokio::test]
async fn an_invitation_cannot_be_both_accepted_and_revoked() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t, m, _) = seeded(&pool).await;

    // Positive control: revoking an unaccepted invitation is fine.
    let id1 = invitation(&pool, t, "normal", Some(m), vec![23; 32])
        .await
        .unwrap();
    sqlx::query("update invitations set revoked_at = now() where tenant_id = $1 and id = $2")
        .bind(t)
        .bind(id1)
        .execute(&pool)
        .await
        .expect("revoking an unaccepted invitation is accepted");

    let id2 = invitation(&pool, t, "normal", Some(m), vec![24; 32])
        .await
        .unwrap();
    sqlx::query(
        "update invitations set accepted_at = now(), accepted_membership_id = $3
         where tenant_id = $1 and id = $2",
    )
    .bind(t)
    .bind(id2)
    .bind(m)
    .execute(&pool)
    .await
    .expect("accepting it first is fine");

    let err =
        sqlx::query("update invitations set revoked_at = now() where tenant_id = $1 and id = $2")
            .bind(t)
            .bind(id2)
            .execute(&pool)
            .await
            .expect_err("an accepted invitation was also revoked");
    assert_eq!(sqlstate(&err).as_deref(), Some("23514"));
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("invitation_is_not_accepted_and_revoked")
    );
}

#[tokio::test]
async fn handover_period_must_be_non_empty() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t, m, r) = seeded(&pool).await;
    let ra1 = Uuid::now_v7();
    sqlx::query(
        "insert into role_assignments (tenant_id, id, membership_id, role_id, starts_on, ends_on_exclusive)
         values ($1, $2, $3, $4, date '2026-10-01', date '2027-10-01')",
    )
    .bind(t)
    .bind(ra1)
    .bind(m)
    .bind(r)
    .execute(&pool)
    .await
    .unwrap();

    // Positive control: a genuine window is accepted.
    sqlx::query(
        "insert into handover_grants (tenant_id, id, source_assignment_id, starts_on, ends_on_exclusive)
         values ($1, $2, $3, date '2027-10-01', date '2028-04-01')",
    )
    .bind(t)
    .bind(Uuid::now_v7())
    .bind(ra1)
    .execute(&pool)
    .await
    .expect("a non-empty handover period is accepted");

    // A second source assignment: handover_grants_one_per_source would otherwise
    // reject a second grant from ra1 before this constraint is even reached.
    let ra2 = Uuid::now_v7();
    sqlx::query(
        "insert into role_assignments (tenant_id, id, membership_id, role_id, starts_on, ends_on_exclusive)
         values ($1, $2, $3, $4, date '2026-10-01', date '2027-10-01')",
    )
    .bind(t)
    .bind(ra2)
    .bind(m)
    .bind(r)
    .execute(&pool)
    .await
    .unwrap();
    let err = sqlx::query(
        "insert into handover_grants (tenant_id, id, source_assignment_id, starts_on, ends_on_exclusive)
         values ($1, $2, $3, date '2028-04-01', date '2028-04-01')",
    )
    .bind(t)
    .bind(Uuid::now_v7())
    .bind(ra2)
    .execute(&pool)
    .await
    .expect_err("an empty handover period was accepted");
    assert_eq!(sqlstate(&err).as_deref(), Some("23514"));
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("handover_period_is_non_empty")
    );
}

#[tokio::test]
async fn access_request_proposed_period_must_be_non_empty() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t, m, r) = seeded(&pool).await;
    let ra = Uuid::now_v7();
    sqlx::query(
        "insert into role_assignments (tenant_id, id, membership_id, role_id, starts_on, ends_on_exclusive)
         values ($1, $2, $3, $4, date '2026-10-01', date '2027-10-01')",
    )
    .bind(t)
    .bind(ra)
    .bind(m)
    .bind(r)
    .execute(&pool)
    .await
    .unwrap();

    // Positive control: a genuine proposed period is accepted.
    sqlx::query(
        "insert into access_requests
           (tenant_id, id, kind, requester_email, invitee_email, requester_membership_id,
            replaced_assignment_id, proposed_starts_on, proposed_ends_on_exclusive,
            created_on, created_at)
         values ($1, $2, 'replacement', 'a@example.test', 'b@example.test', $3, $4,
                 date '2027-10-01', date '2028-10-01', current_date, now())",
    )
    .bind(t)
    .bind(Uuid::now_v7())
    .bind(m)
    .bind(ra)
    .execute(&pool)
    .await
    .expect("a non-empty proposed period is accepted");

    let err = sqlx::query(
        "insert into access_requests
           (tenant_id, id, kind, requester_email, invitee_email, requester_membership_id,
            replaced_assignment_id, proposed_starts_on, proposed_ends_on_exclusive,
            created_on, created_at)
         values ($1, $2, 'replacement', 'c@example.test', 'd@example.test', $3, $4,
                 date '2027-10-01', date '2027-10-01', current_date, now())",
    )
    .bind(t)
    .bind(Uuid::now_v7())
    .bind(m)
    .bind(ra)
    .execute(&pool)
    .await
    .expect_err("an empty proposed period was accepted");
    assert_eq!(sqlstate(&err).as_deref(), Some("23514"));
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("access_request_proposed_period_is_non_empty")
    );
}

#[tokio::test]
async fn access_request_closure_matches_its_status() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t, _, _) = seeded(&pool).await;

    // Positive control: a pending request has no closed_at.
    sqlx::query(
        "insert into access_requests (tenant_id, id, kind, requester_email, invitee_email, created_on, created_at)
         values ($1, $2, 'access', 'a@example.test', 'a@example.test', current_date, now())",
    )
    .bind(t)
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await
    .expect("a pending request without closed_at is accepted");

    let err = sqlx::query(
        "insert into access_requests
           (tenant_id, id, kind, requester_email, invitee_email, status, closed_at, created_on, created_at)
         values ($1, $2, 'access', 'b@example.test', 'b@example.test', 'pending', now(), current_date, now())",
    )
    .bind(t)
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await
    .expect_err("a pending request with closed_at was accepted");
    assert_eq!(sqlstate(&err).as_deref(), Some("23514"));
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("access_request_closure_matches_status")
    );
}

#[tokio::test]
async fn access_request_decider_matches_its_status() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t, m, _) = seeded(&pool).await;

    // Positive control: an approved request names its decider (closed_at is set too,
    // to satisfy access_request_closure_matches_status independently of this one).
    sqlx::query(
        "insert into access_requests
           (tenant_id, id, kind, requester_email, invitee_email, status, decided_by, closed_at, created_on, created_at)
         values ($1, $2, 'access', 'a@example.test', 'a@example.test', 'approved', $3, now(), current_date, now())",
    )
    .bind(t)
    .bind(Uuid::now_v7())
    .bind(m)
    .execute(&pool)
    .await
    .expect("an approved request naming its decider is accepted");

    let err = sqlx::query(
        "insert into access_requests
           (tenant_id, id, kind, requester_email, invitee_email, status, closed_at, created_on, created_at)
         values ($1, $2, 'access', 'b@example.test', 'b@example.test', 'approved', now(), current_date, now())",
    )
    .bind(t)
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await
    .expect_err("an approved request without a decider was accepted");
    assert_eq!(sqlstate(&err).as_deref(), Some("23514"));
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("access_request_decider_matches_status")
    );
}

#[tokio::test]
async fn invitation_role_period_must_be_non_empty() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t, m, r) = seeded(&pool).await;
    let inv = invitation(&pool, t, "normal", Some(m), vec![30; 32])
        .await
        .unwrap();

    // Positive control: a genuine period is accepted.
    sqlx::query(
        "insert into invitation_roles (tenant_id, invitation_id, role_id, starts_on, ends_on_exclusive)
         values ($1, $2, $3, date '2026-10-01', date '2027-10-01')",
    )
    .bind(t)
    .bind(inv)
    .bind(r)
    .execute(&pool)
    .await
    .expect("a non-empty invitation-role period is accepted");

    // A second role on the same tenant: (tenant_id, invitation_id, role_id) is the
    // primary key, and r is already used by the row above.
    let r2 = Uuid::now_v7();
    sqlx::query(
        "insert into roles (tenant_id, id, name, capability_class) values ($1, $2, 'Nestleder', 'member')",
    )
    .bind(t)
    .bind(r2)
    .execute(&pool)
    .await
    .unwrap();

    let err = sqlx::query(
        "insert into invitation_roles (tenant_id, invitation_id, role_id, starts_on, ends_on_exclusive)
         values ($1, $2, $3, date '2027-10-01', date '2027-10-01')",
    )
    .bind(t)
    .bind(inv)
    .bind(r2)
    .execute(&pool)
    .await
    .expect_err("an empty invitation-role period was accepted");
    assert_eq!(sqlstate(&err).as_deref(), Some("23514"));
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("invitation_role_period_is_non_empty")
    );
}

#[tokio::test]
async fn recovery_nominee_email_matches_nomination_status() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t1, _, _) = seeded(&pool).await;

    // Positive control: no nomination, no nominee.
    sqlx::query("insert into recovery_contacts (tenant_id, holder) values ($1, 'ewb')")
        .bind(t1)
        .execute(&pool)
        .await
        .expect("no nominee with nomination_status 'none' is accepted");

    // recovery_contacts is keyed on tenant_id alone, so the violating row needs its
    // own tenant.
    let (t2, _, _) = seeded(&pool).await;
    let err = sqlx::query(
        "insert into recovery_contacts (tenant_id, holder, nominee_email)
         values ($1, 'ewb', 'someone@example.test')",
    )
    .bind(t2)
    .execute(&pool)
    .await
    .expect_err("a nominee email with nomination_status 'none' was accepted");
    assert_eq!(sqlstate(&err).as_deref(), Some("23514"));
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("recovery_nominee_matches_status")
    );
}

#[tokio::test]
async fn audit_params_must_be_small() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;

    // Positive control: a small object is accepted.
    sqlx::query(
        "insert into audit_events (id, actor_kind, action, subject_type, subject_id, occurred_at, params)
         values ($1, 'system', 'tenant.signup_created', 'tenant', $1, now(), jsonb_build_object('a', 1))",
    )
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await
    .expect("a small params object is accepted");

    let err = sqlx::query(
        "insert into audit_events (id, actor_kind, action, subject_type, subject_id, occurred_at, params)
         values ($1, 'system', 'tenant.signup_created', 'tenant', $1, now(), jsonb_build_object('pad', repeat('x', 3000)))",
    )
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await
    .expect_err("an oversized params object was accepted");
    assert_eq!(sqlstate(&err).as_deref(), Some("23514"));
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("audit_params_are_small")
    );
}

#[tokio::test]
async fn outbox_template_must_be_a_code() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;

    // Positive control: a dotted action-code template is accepted.
    sqlx::query(
        "insert into outbox (id, template, recipient_email, created_at)
         values ($1, 'invitation.issued', 'x@example.test', now())",
    )
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await
    .expect("a code-shaped template is accepted");

    let err = sqlx::query(
        "insert into outbox (id, template, recipient_email, created_at)
         values ($1, 'Invitation Issued', 'y@example.test', now())",
    )
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await
    .expect_err("a sentence-shaped template was accepted");
    assert_eq!(sqlstate(&err).as_deref(), Some("23514"));
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("outbox_template_is_a_code")
    );
}

#[tokio::test]
async fn audit_subject_type_must_be_a_code() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;

    // Positive control: a lowercase single-word code is accepted.
    sqlx::query(
        "insert into audit_events (id, actor_kind, action, subject_type, subject_id, occurred_at)
         values ($1, 'system', 'tenant.signup_created', 'tenant', $1, now())",
    )
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await
    .expect("a code-shaped subject_type is accepted");

    let err = sqlx::query(
        "insert into audit_events (id, actor_kind, action, subject_type, subject_id, occurred_at)
         values ($1, 'system', 'tenant.signup_created', 'Tenant Type', $1, now())",
    )
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await
    .expect_err("a non-code subject_type was accepted");
    assert_eq!(sqlstate(&err).as_deref(), Some("23514"));
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("audit_subject_type_is_a_code")
    );
}
