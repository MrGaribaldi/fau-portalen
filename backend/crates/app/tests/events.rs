//! The per-FAU change stream (groups design §3.3 and §10, "SSE"): no delivery reaches a
//! viewer who may not read the resource, a guest beside a closed group included, and
//! revocation closes a live stream. `Subscription::recv` returning `None` is what ends an
//! SSE body (Ruling R1).

mod common;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::groups::*;
use common::membership::*;
use common::TestDb;
use fau_persistence::membership::{
    revoke_membership, Change, Hub, HubClock, MembershipError, Resource, RevokeMembership,
    Subscription, EVENTS_CHANNEL,
};
use jiff::Timestamp;
use sqlx::PgPool;
use uuid::Uuid;

/// A hub on its own pool, with a clock the test can move, starting at T0.
async fn hub(db: &TestDb) -> (Hub, Arc<Mutex<Timestamp>>) {
    let now = Arc::new(Mutex::new(T0.parse::<Timestamp>().unwrap()));
    let read = now.clone();
    let clock: HubClock = Arc::new(move || *read.lock().unwrap());
    (
        Hub::start(db.app_pool().await, clock)
            .await
            .expect("start the hub"),
        now,
    )
}

async fn next(sub: &mut Subscription) -> Option<Resource> {
    tokio::time::timeout(Duration::from_secs(5), sub.recv())
        .await
        .expect("a delivery or a close within five seconds")
}

async fn announce(pool: &PgPool, tenant: Uuid, change: Change) {
    sqlx::query("select pg_notify($1, $2)")
        .bind(EVENTS_CHANNEL)
        .bind(change.payload(tenant))
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_change_reaches_only_the_viewers_who_may_read_it() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let (hub, _) = hub(&db).await;
    let mut admin = hub.subscribe(w.viewer("admin_out")).await.unwrap();
    let mut member = hub.subscribe(w.viewer("member_out")).await.unwrap();
    let mut guest_in = hub.subscribe(w.viewer("guest_in")).await.unwrap();
    let mut guest_out = hub.subscribe(w.viewer("guest_out")).await.unwrap();
    let t = w.fau.tenant_id;
    let changed = |g| Change::Changed(Resource::Group(g));

    for g in [w.closed, w.open, w.other] {
        announce(&pool, t, changed(g)).await;
    }
    for g in [w.closed, w.open, w.other] {
        assert_eq!(
            next(&mut admin).await,
            Some(Resource::Group(g)),
            "admins hear everything"
        );
    }
    assert_eq!(next(&mut member).await, Some(Resource::Group(w.open)));
    assert_eq!(next(&mut guest_in).await, Some(Resource::Group(w.closed)));
    assert_eq!(next(&mut guest_in).await, Some(Resource::Group(w.open)));
    assert_eq!(next(&mut guest_out).await, Some(Resource::Group(w.other)));

    // Nothing unauthorized is queued behind those. The next change each viewer may read is
    // the next thing it hears.
    announce(&pool, t, changed(w.open)).await;
    assert_eq!(
        next(&mut member).await,
        Some(Resource::Group(w.open)),
        "not `other`"
    );
    assert_eq!(
        next(&mut guest_in).await,
        Some(Resource::Group(w.open)),
        "not `other`"
    );
    announce(&pool, t, changed(w.other)).await;
    assert_eq!(
        next(&mut guest_out).await,
        Some(Resource::Group(w.other)),
        "a guest beside two closed groups heard nothing about them, or about the open one"
    );
}

#[tokio::test]
async fn revoking_a_role_closes_the_stream() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let (hub, _) = hub(&db).await;
    let mut member = hub.subscribe(w.viewer("member_in")).await.unwrap();
    let mut bystander = hub.subscribe(w.viewer("member_out")).await.unwrap();
    assert_eq!(hub.subscriber_count(), 2);

    revoke(
        &pool,
        &w.fau,
        live_assignment(&pool, w.membership("member_in")).await,
        at(T0),
    )
    .await;
    assert_eq!(
        next(&mut member).await,
        None,
        "the revoked member's stream closes"
    );

    announce(
        &pool,
        w.fau.tenant_id,
        Change::Changed(Resource::Group(w.open)),
    )
    .await;
    assert_eq!(
        next(&mut bystander).await,
        Some(Resource::Group(w.open)),
        "other streams stay open"
    );
    assert_eq!(hub.subscriber_count(), 1);
    assert_eq!(
        hub.subscribe(w.viewer("member_in")).await.unwrap_err(),
        MembershipError::NotAuthorized,
        "without a valid role a new stream is refused"
    );
}

#[tokio::test]
async fn revoking_a_membership_closes_the_stream() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let (hub, _) = hub(&db).await;
    let mut leaver = hub.subscribe(w.viewer("guest_in")).await.unwrap();
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: w.fau.tenant_id,
            actor_membership_id: w.membership("guest_in"),
            membership_id: w.membership("guest_in"),
            confirm_no_admin: false,
        },
        at(T0),
    )
    .await
    .unwrap();
    assert_eq!(next(&mut leaver).await, None);
}

#[tokio::test]
async fn a_stream_whose_role_has_ended_closes_at_its_next_delivery() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let (hub, now) = hub(&db).await;
    // member_out's role ends on 1 September 2027; the registrant's admin role on 1 October.
    let mut member = hub.subscribe(w.viewer("member_out")).await.unwrap();
    let mut admin = hub.subscribe(w.viewer("admin_in")).await.unwrap();
    *now.lock().unwrap() = "2027-09-01T10:00:00Z".parse().unwrap();

    announce(
        &pool,
        w.fau.tenant_id,
        Change::Changed(Resource::Group(w.open)),
    )
    .await;
    assert_eq!(
        next(&mut member).await,
        None,
        "no standing any more: closed, not merely filtered"
    );
    assert_eq!(next(&mut admin).await, Some(Resource::Group(w.open)));
}

#[tokio::test]
async fn a_guest_may_subscribe_and_a_person_without_standing_may_not() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let (hub, _) = hub(&db).await;
    let _guest = hub
        .subscribe(w.viewer("guest_out"))
        .await
        .expect("a guest has standing");
    for gone in ["none_in", "none_out"] {
        assert_eq!(
            hub.subscribe(w.viewer(gone)).await.unwrap_err(),
            MembershipError::NotAuthorized,
            "{gone}"
        );
    }
    assert_eq!(
        hub.subscriber_count(),
        1,
        "a refused subscription leaves nothing behind"
    );
}
