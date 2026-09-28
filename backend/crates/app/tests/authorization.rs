//! The authorization matrix (groups design §3.3 and §10): viewer class × group visibility ×
//! in or out of the group × resource type, through the one function that reads the
//! database. **A new resource type must add its rows to `MATRIX`.**

mod common;
use common::groups::*;
use common::membership::*;
use common::TestDb;
use fau_domain::authz::{Action, Decision, Denied};
use fau_domain::membership::vocabulary::{CapabilityClass, Visibility};
use fau_persistence::membership::{authorize, grant_role, GrantRole, Resource, RoleChoice, Viewer};
use sqlx::PgPool;
use uuid::Uuid;

/// Allowed, Forbidden, Hidden (answered as not found), No access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum O {
    A,
    F,
    H,
    N,
}
use O::*;

fn outcome(d: Decision) -> O {
    match d {
        Ok(()) => A,
        Err(Denied::Forbidden) => F,
        Err(Denied::Hidden) => H,
        Err(Denied::NoAccess) => N,
    }
}

#[derive(Debug, Clone, Copy)]
enum R {
    /// FAU-wide content (audience null).
    Fau,
    OpenContent,
    ClosedContent,
    OpenGroup,
    ClosedGroup,
    /// A group id that does not exist: must look exactly like a hidden one.
    UnknownGroup,
}

/// (viewer, resource, read, write, manage). Every viewer in `VIEWERS`, every resource type.
#[rustfmt::skip]
const MATRIX: &[(&str, R, O, O, O)] = &[
    // FAU-wide content: members and admins, never guests.
    ("admin_in",   R::Fau, A, A, A),
    ("admin_out",  R::Fau, A, A, A),
    ("member_in",  R::Fau, A, A, F),
    ("member_out", R::Fau, A, A, F),
    ("guest_in",   R::Fau, H, H, H),
    ("guest_out",  R::Fau, H, H, H),
    ("none_in",    R::Fau, N, N, N),
    ("none_out",   R::Fau, N, N, N),
    // Content in an open group: members read; only those in it write.
    ("admin_in",   R::OpenContent, A, A, A),
    ("admin_out",  R::OpenContent, A, A, A),
    ("member_in",  R::OpenContent, A, A, F),
    ("member_out", R::OpenContent, A, F, F),
    ("guest_in",   R::OpenContent, A, A, F),
    ("guest_out",  R::OpenContent, H, H, H),
    ("none_in",    R::OpenContent, N, N, N),
    ("none_out",   R::OpenContent, N, N, N),
    // Content in a closed group: only those in it, and admins.
    ("admin_in",   R::ClosedContent, A, A, A),
    ("admin_out",  R::ClosedContent, A, A, A),
    ("member_in",  R::ClosedContent, A, A, F),
    ("member_out", R::ClosedContent, H, H, H),
    ("guest_in",   R::ClosedContent, A, A, F),
    ("guest_out",  R::ClosedContent, H, H, H),
    ("none_in",    R::ClosedContent, N, N, N),
    ("none_out",   R::ClosedContent, N, N, N),
    // An open group itself: writing to it is managing it, which is admin-only.
    ("admin_in",   R::OpenGroup, A, A, A),
    ("admin_out",  R::OpenGroup, A, A, A),
    ("member_in",  R::OpenGroup, A, F, F),
    ("member_out", R::OpenGroup, A, F, F),
    ("guest_in",   R::OpenGroup, A, F, F),
    ("guest_out",  R::OpenGroup, H, H, H),
    ("none_in",    R::OpenGroup, N, N, N),
    ("none_out",   R::OpenGroup, N, N, N),
    // A closed group itself: its existence and name are hidden from those outside it.
    ("admin_in",   R::ClosedGroup, A, A, A),
    ("admin_out",  R::ClosedGroup, A, A, A),
    ("member_in",  R::ClosedGroup, A, F, F),
    ("member_out", R::ClosedGroup, H, H, H),
    ("guest_in",   R::ClosedGroup, A, F, F),
    ("guest_out",  R::ClosedGroup, H, H, H),
    ("none_in",    R::ClosedGroup, N, N, N),
    ("none_out",   R::ClosedGroup, N, N, N),
    // An unknown id: indistinguishable from a hidden group, even for an admin.
    ("admin_in",   R::UnknownGroup, H, H, H),
    ("admin_out",  R::UnknownGroup, H, H, H),
    ("member_in",  R::UnknownGroup, H, H, H),
    ("member_out", R::UnknownGroup, H, H, H),
    ("guest_in",   R::UnknownGroup, H, H, H),
    ("guest_out",  R::UnknownGroup, H, H, H),
    ("none_in",    R::UnknownGroup, N, N, N),
    ("none_out",   R::UnknownGroup, N, N, N),
];

async fn check(pool: &PgPool, viewer: Viewer, resource: Resource, action: Action, at_: &str) -> O {
    let mut conn = pool.acquire().await.unwrap();
    outcome(
        authorize(&mut conn, viewer, resource, action, at(at_))
            .await
            .unwrap(),
    )
}

#[tokio::test]
async fn the_authorization_matrix() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let unknown = Uuid::now_v7();

    // Every viewer appears once per resource type.
    for r in [
        "Fau",
        "OpenContent",
        "ClosedContent",
        "OpenGroup",
        "ClosedGroup",
        "UnknownGroup",
    ] {
        let mut viewers: Vec<&str> = MATRIX
            .iter()
            .filter(|row| format!("{:?}", row.1) == r)
            .map(|row| row.0)
            .collect();
        viewers.sort_unstable();
        let mut all = VIEWERS.to_vec();
        all.sort_unstable();
        assert_eq!(viewers, all, "{r}");
    }

    let mut failures = Vec::new();
    for &(viewer, r, read, write, manage) in MATRIX {
        let resource = match r {
            R::Fau => Resource::Fau,
            R::OpenContent => Resource::GroupContent(w.open),
            R::ClosedContent => Resource::GroupContent(w.closed),
            R::OpenGroup => Resource::Group(w.open),
            R::ClosedGroup => Resource::Group(w.closed),
            R::UnknownGroup => Resource::Group(unknown),
        };
        for (action, want) in [
            (Action::Read, read),
            (Action::Write, write),
            (Action::Manage, manage),
        ] {
            let got = check(&pool, w.viewer(viewer), resource, action, T0).await;
            if got != want {
                failures.push(format!(
                    "{viewer} {r:?} {action:?}: want {want:?}, got {got:?}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[tokio::test]
async fn an_archived_group_is_read_only_but_still_managed() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    sqlx::query("update groups set archived_at = $1::timestamptz where id = $2")
        .bind(T0)
        .bind(w.open)
        .execute(&pool)
        .await
        .unwrap();
    let content = Resource::GroupContent(w.open);
    assert_eq!(
        check(&pool, w.viewer("admin_out"), content, Action::Read, T0).await,
        A
    );
    assert_eq!(
        check(&pool, w.viewer("admin_out"), content, Action::Write, T0).await,
        F
    );
    assert_eq!(
        check(&pool, w.viewer("member_in"), content, Action::Write, T0).await,
        F
    );
    assert_eq!(
        check(&pool, w.viewer("guest_in"), content, Action::Read, T0).await,
        A
    );
    assert_eq!(
        check(
            &pool,
            w.viewer("admin_out"),
            Resource::Group(w.open),
            Action::Manage,
            T0
        )
        .await,
        A
    );
}

#[tokio::test]
async fn a_bound_group_follows_roles_on_its_unit_or_cohort_on_the_current_date() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let unit = seed_unit(&pool, fau.tenant_id).await;
    let cohort = seed_cohort(&pool, fau.tenant_id).await;
    let unit_role = seed_role(
        &pool,
        fau.tenant_id,
        CapabilityClass::Member,
        None,
        Some(unit),
        None,
    )
    .await;
    let cohort_role = seed_role(
        &pool,
        fau.tenant_id,
        CapabilityClass::Member,
        None,
        None,
        Some(cohort),
    )
    .await;
    let by_unit = seed_bound_group(&pool, &fau, Visibility::Closed, Some(unit), None).await;
    let by_cohort = seed_bound_group(&pool, &fau, Visibility::Closed, None, Some(cohort)).await;

    // A member for two years, contact parent on the unit until New Year.
    let parent = add_member(
        &pool,
        &fau,
        "kontakt@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 1), day(2028, 9, 1)),
        t0,
    )
    .await;
    grant_role(
        &pool,
        GrantRole {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: parent.membership_id,
            role: RoleChoice::Existing(unit_role),
            period: period(day(2026, 9, 1), day(2027, 1, 1)),
        },
        t0,
    )
    .await
    .unwrap();
    let v = Viewer {
        tenant_id: fau.tenant_id,
        membership_id: parent.membership_id,
    };
    let read = Action::Read;
    assert_eq!(
        check(&pool, v, Resource::GroupContent(by_unit), read, T0).await,
        A
    );
    assert_eq!(
        check(&pool, v, Resource::GroupContent(by_cohort), read, T0).await,
        H
    );
    // On 1 January the unit role has ended; no job ran, and the group no longer holds them.
    let new_year = "2026-12-31T23:00:00Z";
    assert_eq!(
        check(&pool, v, Resource::GroupContent(by_unit), read, new_year).await,
        H
    );
    assert_eq!(
        check(&pool, v, Resource::Fau, read, new_year).await,
        A,
        "still a member"
    );

    // Link `unit` and `cohort` in unit_cohorts, so the "no traversal" assertions below are
    // actually exercised: without this row, a broken ROLE_FOLLOWS_GROUP that joined through
    // unit_cohorts would find no match either, and the assertions could never fail.
    sqlx::query(
        "insert into unit_cohorts (tenant_id, unit_id, cohort_id, grade_level) values ($1, $2, $3, 3)",
    )
    .bind(fau.tenant_id)
    .bind(unit)
    .bind(cohort)
    .execute(&pool)
    .await
    .unwrap();
    // Reverse of the check below: the unit-role holder still does not reach the
    // cohort-bound group now that the link exists (Ruling R6 -- no traversal through
    // unit_cohorts).
    assert_eq!(
        check(&pool, v, Resource::GroupContent(by_cohort), read, T0).await,
        H,
        "a unit role must not reach a cohort-bound group via unit_cohorts"
    );

    // A cohort role reaches the cohort's group, not the unit's: there is no traversal
    // through unit_cohorts (Ruling R6).
    let cohort_parent = add_member(
        &pool,
        &fau,
        "kull@example.test",
        RoleChoice::Existing(cohort_role),
        period(day(2026, 9, 1), day(2027, 9, 1)),
        t0,
    )
    .await;
    let c = Viewer {
        tenant_id: fau.tenant_id,
        membership_id: cohort_parent.membership_id,
    };
    assert_eq!(
        check(&pool, c, Resource::GroupContent(by_cohort), read, T0).await,
        A
    );
    assert_eq!(
        check(&pool, c, Resource::GroupContent(by_unit), read, T0).await,
        H
    );

    // A bound group can still take a hand-added member.
    seed_group_member(&pool, &fau, by_unit, cohort_parent.membership_id).await;
    assert_eq!(
        check(&pool, c, Resource::GroupContent(by_unit), read, T0).await,
        A
    );

    // ASSIGNMENT_VALID_ON_3's `ra.revoked_at is null`: a member's unit role, once revoked,
    // no longer reaches the unit-bound group, even though their (separate) member role is
    // still live and keeps their FAU-wide standing.
    let revoked_parent = add_member(
        &pool,
        &fau,
        "revoked-unit-role@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 1), day(2028, 9, 1)),
        t0,
    )
    .await;
    grant_role(
        &pool,
        GrantRole {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: revoked_parent.membership_id,
            role: RoleChoice::Existing(unit_role),
            period: period(day(2026, 9, 1), day(2027, 9, 1)),
        },
        t0,
    )
    .await
    .unwrap();
    let unit_assignment: Uuid = sqlx::query_scalar(
        "select id from role_assignments
          where membership_id = $1 and role_id = $2 and revoked_at is null",
    )
    .bind(revoked_parent.membership_id)
    .bind(unit_role)
    .fetch_one(&pool)
    .await
    .unwrap();
    revoke(&pool, &fau, unit_assignment, t0).await;
    let rv = Viewer {
        tenant_id: fau.tenant_id,
        membership_id: revoked_parent.membership_id,
    };
    assert_eq!(
        check(&pool, rv, Resource::GroupContent(by_unit), read, T0).await,
        H,
        "a revoked unit role must not reach the unit-bound group"
    );
    assert_eq!(
        check(&pool, rv, Resource::Fau, read, T0).await,
        A,
        "the member role is untouched by revoking the unit role"
    );

    // ASSIGNMENT_VALID_ON_3's `ra.starts_on <= $3::date`: a unit role that has not started
    // yet grants nothing today, and starts to on its first valid day.
    let future_parent = add_member(
        &pool,
        &fau,
        "future-unit-role@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 1), day(2028, 9, 1)),
        t0,
    )
    .await;
    grant_role(
        &pool,
        GrantRole {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: future_parent.membership_id,
            role: RoleChoice::Existing(unit_role),
            period: period(day(2027, 1, 1), day(2027, 9, 1)),
        },
        t0,
    )
    .await
    .unwrap();
    let fv = Viewer {
        tenant_id: fau.tenant_id,
        membership_id: future_parent.membership_id,
    };
    assert_eq!(
        check(&pool, fv, Resource::GroupContent(by_unit), read, T0).await,
        H,
        "the unit role has not started yet"
    );
    assert_eq!(
        check(
            &pool,
            fv,
            Resource::GroupContent(by_unit),
            read,
            "2027-01-02T00:00:00Z"
        )
        .await,
        A,
        "the unit role has started"
    );
}

/// A viewer from a different FAU is not "outside the group": their own tenant has no such
/// group at all, so the group's own tenant scoping (`g.tenant_id = $1`) must refuse them
/// exactly as it would an unknown id -- not fall through to `viewer_in_group` on someone
/// else's rows.
#[tokio::test]
async fn a_viewer_from_another_tenant_gets_no_access() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;

    let t0 = at(T0);
    let year = period(day(2026, 9, 1), day(2027, 9, 1));
    let fau_b = active_fau(&pool, "admin-b@example.test", t0).await;
    let member_b = add_member(
        &pool,
        &fau_b,
        "member-b@example.test",
        new_role("Medlem", CapabilityClass::Member),
        year,
        t0,
    )
    .await
    .membership_id;

    let viewer_b = Viewer {
        tenant_id: fau_b.tenant_id,
        membership_id: member_b,
    };
    for resource in [
        Resource::Group(w.open),
        Resource::GroupContent(w.open),
        Resource::GroupContent(w.closed),
    ] {
        assert_eq!(
            check(&pool, viewer_b, resource, Action::Read, T0).await,
            H,
            "{resource:?}"
        );
    }

    // A viewer whose tenant_id and membership_id name different FAUs has no standing in
    // the claimed tenant at all -- the membership lookup itself must find nothing.
    let mismatched = Viewer {
        tenant_id: w.fau.tenant_id,
        membership_id: member_b,
    };
    assert_eq!(
        check(&pool, mismatched, Resource::Fau, Action::Read, T0).await,
        N
    );
}

#[tokio::test]
async fn a_revocation_takes_effect_on_the_next_call() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let v = w.viewer("member_in");
    let closed = Resource::GroupContent(w.closed);
    assert_eq!(check(&pool, v, closed, Action::Read, T0).await, A);

    sqlx::query(
        "update group_members set removed_at = $1::timestamptz where group_id = $2 and membership_id = $3",
    )
    .bind(T0)
    .bind(w.closed)
    .bind(v.membership_id)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(check(&pool, v, closed, Action::Read, T0).await, H);

    revoke(
        &pool,
        &w.fau,
        live_assignment(&pool, v.membership_id).await,
        at(T0),
    )
    .await;
    assert_eq!(check(&pool, v, Resource::Fau, Action::Read, T0).await, N);
}
