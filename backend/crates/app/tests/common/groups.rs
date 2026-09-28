//! Group fixtures for the integration tests. Groups, hand-added members, guest roles and
//! school structure are written straight to the database, so a test of `authorize` or of
//! the group transactions does not depend on the code it exercises. People still join
//! through the invitation transactions (`add_member`), as everywhere else.

use fau_domain::membership::period::Period;
use fau_domain::membership::vocabulary::{CapabilityClass, Visibility};
use fau_domain::time::Moment;
use fau_persistence::membership::{
    revoke_role_assignment, Accepted, RevokeAssignment, RoleChoice, Viewer,
};
use sqlx::PgPool;
use uuid::Uuid;

use super::membership::*;

/// Every fixture row is dated T0, so a transaction at T0 or later may remove it
/// (`group_member_removed_after_added`).
const FIXTURE_AT: &str = T0;

/// Shaped like fau-crypto's envelope (version byte 1, then 41 bytes), so
/// `groups_name_is_an_envelope` accepts it. Never decrypted.
pub fn placeholder_name() -> Vec<u8> {
    let mut v = vec![1u8];
    v.extend_from_slice(&[0u8; 41]);
    v
}

pub async fn seed_group(pool: &PgPool, fau: &Fau, visibility: Visibility) -> Uuid {
    seed_bound_group(pool, fau, visibility, None, None).await
}

pub async fn seed_bound_group(
    pool: &PgPool,
    fau: &Fau,
    visibility: Visibility,
    unit: Option<Uuid>,
    cohort: Option<Uuid>,
) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into groups (tenant_id, id, encrypted_name, visibility, unit_id, cohort_id, created_by, created_at)
         values ($1, $2, $3, $4, $5, $6, $7, $8::timestamptz)",
    )
    .bind(fau.tenant_id)
    .bind(id)
    .bind(placeholder_name())
    .bind(visibility.code())
    .bind(unit)
    .bind(cohort)
    .bind(fau.admin_membership_id)
    .bind(FIXTURE_AT)
    .execute(pool)
    .await
    .expect("seed a group");
    id
}

pub async fn seed_group_member(pool: &PgPool, fau: &Fau, group: Uuid, membership: Uuid) {
    sqlx::query(
        "insert into group_members (tenant_id, id, group_id, membership_id, added_by, added_at)
         values ($1, $2, $3, $4, $5, $6::timestamptz)",
    )
    .bind(fau.tenant_id)
    .bind(Uuid::now_v7())
    .bind(group)
    .bind(membership)
    .bind(fau.admin_membership_id)
    .bind(FIXTURE_AT)
    .execute(pool)
    .await
    .expect("seed a group member");
}

pub async fn seed_role(
    pool: &PgPool,
    tenant: Uuid,
    class: CapabilityClass,
    group: Option<Uuid>,
    unit: Option<Uuid>,
    cohort: Option<Uuid>,
) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into roles (tenant_id, id, name, capability_class, group_id, unit_id, cohort_id)
         values ($1, $2, 'Fixture', $3, $4, $5, $6)",
    )
    .bind(tenant)
    .bind(id)
    .bind(class.code())
    .bind(group)
    .bind(unit)
    .bind(cohort)
    .execute(pool)
    .await
    .expect("seed a role");
    id
}

/// A unit in a school year of its own, so names never collide.
pub async fn seed_unit(pool: &PgPool, tenant: Uuid) -> Uuid {
    let (year, unit) = (Uuid::now_v7(), Uuid::now_v7());
    sqlx::query(
        "insert into school_years (tenant_id, id, name, starts_on, ends_on_exclusive)
         values ($1, $2, '2026/27', '2026-08-01', '2027-08-01')",
    )
    .bind(tenant)
    .bind(year)
    .execute(pool)
    .await
    .expect("seed a school year");
    sqlx::query(
        "insert into organization_units (tenant_id, id, school_year_id, kind, name)
         values ($1, $2, $3, 'grade', $4)",
    )
    .bind(tenant)
    .bind(unit)
    .bind(year)
    .bind(format!("unit-{unit}"))
    .execute(pool)
    .await
    .expect("seed a unit");
    unit
}

pub async fn seed_cohort(pool: &PgPool, tenant: Uuid) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query("insert into cohorts (tenant_id, id, name) values ($1, $2, $3)")
        .bind(tenant)
        .bind(id)
        .bind(format!("cohort-{id}"))
        .execute(pool)
        .await
        .expect("seed a cohort");
    id
}

/// `address` joins as a guest of `group`, through a real invitation.
pub async fn add_guest(
    pool: &PgPool,
    fau: &Fau,
    address: &str,
    group: Uuid,
    period: Period,
    at: Moment,
) -> Accepted {
    let role = seed_role(
        pool,
        fau.tenant_id,
        CapabilityClass::Guest,
        Some(group),
        None,
        None,
    )
    .await;
    add_member(pool, fau, address, RoleChoice::Existing(role), period, at).await
}

/// The membership's oldest unrevoked role assignment.
pub async fn live_assignment(pool: &PgPool, membership: Uuid) -> Uuid {
    sqlx::query_scalar(
        "select id from role_assignments where membership_id = $1 and revoked_at is null order by id limit 1",
    )
    .bind(membership)
    .fetch_one(pool)
    .await
    .expect("a live assignment")
}

/// The FAU's admin revokes one assignment.
pub async fn revoke(pool: &PgPool, fau: &Fau, assignment_id: Uuid, at: Moment) {
    revoke_role_assignment(
        pool,
        RevokeAssignment {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            assignment_id,
            confirm_no_admin: false,
        },
        at,
    )
    .await
    .expect("revoke");
}

/// The eight viewers the matrix runs over: every capability class, in and out of the
/// groups. `none_*` held a member role that was revoked, so they have no standing.
pub const VIEWERS: [&str; 8] = [
    "admin_in",
    "admin_out",
    "member_in",
    "member_out",
    "guest_in",
    "guest_out",
    "none_in",
    "none_out",
];

/// One FAU with three groups and eight viewers, at T0 (23 September 2026).
///
/// - `open` and `closed`: `admin_in`, `member_in` and `none_in` are added to both by hand.
///   `guest_in` holds a guest role naming `open` and is added to `closed` by hand, so both
///   ways into a group are exercised.
/// - `other`: a closed group whose only member is `guest_out`, by its guest role.
/// - `admin_out` and `member_out` are in no group.
pub struct World {
    pub fau: Fau,
    pub open: Uuid,
    pub closed: Uuid,
    pub other: Uuid,
    members: Vec<(&'static str, Uuid)>,
}

impl World {
    pub fn membership(&self, name: &str) -> Uuid {
        self.members
            .iter()
            .find(|(n, _)| *n == name)
            .unwrap_or_else(|| panic!("no viewer named {name}"))
            .1
    }

    pub fn viewer(&self, name: &str) -> Viewer {
        Viewer {
            tenant_id: self.fau.tenant_id,
            membership_id: self.membership(name),
        }
    }
}

async fn plain_member(pool: &PgPool, fau: &Fau, address: &str, year: Period, t0: Moment) -> Uuid {
    add_member(
        pool,
        fau,
        address,
        new_role("Medlem", CapabilityClass::Member),
        year,
        t0,
    )
    .await
    .membership_id
}

pub async fn world(pool: &PgPool) -> World {
    let t0 = at(T0);
    let fau = active_fau(pool, "admin@example.test", t0).await;
    let open = seed_group(pool, &fau, Visibility::Open).await;
    let closed = seed_group(pool, &fau, Visibility::Closed).await;
    let other = seed_group(pool, &fau, Visibility::Closed).await;
    let year = period(day(2026, 9, 1), day(2027, 9, 1));

    let admin_in = fau.admin_membership_id;
    let admin_out = add_member(
        pool,
        &fau,
        "admin-out@example.test",
        RoleChoice::Existing(fau.admin_role_id),
        year,
        t0,
    )
    .await
    .membership_id;
    let member_in = plain_member(pool, &fau, "member-in@example.test", year, t0).await;
    let member_out = plain_member(pool, &fau, "member-out@example.test", year, t0).await;
    let none_in = plain_member(pool, &fau, "none-in@example.test", year, t0).await;
    let none_out = plain_member(pool, &fau, "none-out@example.test", year, t0).await;
    let guest_in = add_guest(pool, &fau, "guest-in@example.test", open, year, t0)
        .await
        .membership_id;
    let guest_out = add_guest(pool, &fau, "guest-out@example.test", other, year, t0)
        .await
        .membership_id;

    for m in [admin_in, member_in, none_in] {
        seed_group_member(pool, &fau, open, m).await;
        seed_group_member(pool, &fau, closed, m).await;
    }
    seed_group_member(pool, &fau, closed, guest_in).await;
    for m in [none_in, none_out] {
        revoke(pool, &fau, live_assignment(pool, m).await, t0).await;
    }

    World {
        members: vec![
            ("admin_in", admin_in),
            ("admin_out", admin_out),
            ("member_in", member_in),
            ("member_out", member_out),
            ("guest_in", guest_in),
            ("guest_out", guest_out),
            ("none_in", none_in),
            ("none_out", none_out),
        ],
        fau,
        open,
        closed,
        other,
    }
}
