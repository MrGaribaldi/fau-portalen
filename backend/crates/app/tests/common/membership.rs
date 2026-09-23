//! Builders for the membership integration tests. Every fixture goes through the
//! persistence functions themselves, so a fixture that works is evidence too.

use fau_domain::email::{Email, VerifiedEmail};
use fau_domain::membership::period::Period;
use fau_domain::membership::rules::default_admin_end;
use fau_domain::membership::vocabulary::FauName;
use fau_domain::time::Moment;
use fau_persistence::membership::{
    activate_tenant, create_pending_tenant, Activation, PendingSignup,
};
use jiff::civil::Date;
use sqlx::PgPool;
use uuid::Uuid;

/// The instant most tests start at: 23 September 2026, 12:00 in Oslo. The default admin
/// end date from here is 2027-10-01.
pub const T0: &str = "2026-09-23T10:00:00Z";

pub fn at(rfc3339: &str) -> Moment {
    Moment::at(rfc3339.parse().expect("an RFC 3339 timestamp"))
}

pub fn email(s: &str) -> Email {
    Email::parse(s).expect("a valid test address")
}

pub fn verified(s: &str) -> VerifiedEmail {
    VerifiedEmail::from_provider(email(s))
}

pub fn day(year: i16, month: i8, d: i8) -> Date {
    jiff::civil::date(year, month, d)
}

pub fn period(from: Date, to: Date) -> Period {
    Period::new(from, to).expect("a non-empty test period")
}

pub fn signup(school_id: Uuid, registrant: &str, leader: &str, at: Moment) -> PendingSignup {
    PendingSignup {
        school_id,
        fau_name: FauName::parse("Nordre skole FAU").unwrap(),
        registrant_email: email(registrant),
        leader_email: email(leader),
        admin_ends_on_exclusive: default_admin_end(at.today()),
    }
}

/// An active FAU whose registrant is its only admin.
pub struct Fau {
    pub tenant_id: Uuid,
    pub admin_account_id: Uuid,
    pub admin_membership_id: Uuid,
    pub admin_role_id: Uuid,
    pub admin_assignment_id: Uuid,
}

pub async fn active_fau(pool: &PgPool, registrant: &str, at: Moment) -> Fau {
    let pending =
        create_pending_tenant(pool, signup(Uuid::now_v7(), registrant, registrant, at), at)
            .await
            .expect("signup");
    let activated = activate_tenant(
        pool,
        Activation {
            tenant_id: pending.tenant_id,
            registrant: verified(registrant),
        },
        at,
    )
    .await
    .expect("activation");
    Fau {
        tenant_id: pending.tenant_id,
        admin_account_id: activated.account_id,
        admin_membership_id: activated.membership_id,
        admin_role_id: activated.admin_role_id,
        admin_assignment_id: activated.admin_assignment_id,
    }
}

pub async fn count(pool: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.expect(sql)
}

pub async fn outbox_count(pool: &PgPool, template: &str, recipient: &str) -> i64 {
    sqlx::query_scalar("select count(*) from outbox where template = $1 and recipient_email = $2")
        .bind(template)
        .bind(recipient)
        .fetch_one(pool)
        .await
        .unwrap()
}

pub async fn audit_count(pool: &PgPool, action: &str) -> i64 {
    sqlx::query_scalar("select count(*) from audit_events where action = $1")
        .bind(action)
        .fetch_one(pool)
        .await
        .unwrap()
}
