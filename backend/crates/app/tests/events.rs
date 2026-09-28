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
    Subscription, EVENTS_CHANNEL, SUBSCRIPTION_BUFFER,
};
use jiff::Timestamp;
use sqlx::postgres::PgPoolOptions;
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
async fn revoking_a_role_discards_whatever_was_already_buffered() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let (hub, _) = hub(&db).await;
    let mut bystander = hub.subscribe(w.viewer("admin_out")).await.unwrap();
    let mut member = hub.subscribe(w.viewer("member_in")).await.unwrap();

    // Two changes the member can see (it is in both `open` and `closed`), left unread:
    // buffered in its channel, not drained, while the bystander reads along.
    announce(
        &pool,
        w.fau.tenant_id,
        Change::Changed(Resource::Group(w.open)),
    )
    .await;
    announce(
        &pool,
        w.fau.tenant_id,
        Change::Changed(Resource::Group(w.closed)),
    )
    .await;
    assert_eq!(next(&mut bystander).await, Some(Resource::Group(w.open)));
    assert_eq!(next(&mut bystander).await, Some(Resource::Group(w.closed)));

    revoke(
        &pool,
        &w.fau,
        live_assignment(&pool, w.membership("member_in")).await,
        at(T0),
    )
    .await;

    announce(
        &pool,
        w.fau.tenant_id,
        Change::Changed(Resource::Group(w.open)),
    )
    .await;
    assert_eq!(next(&mut bystander).await, Some(Resource::Group(w.open)));

    assert_eq!(
        next(&mut member).await,
        None,
        "revocation discards whatever was already buffered, rather than delivering it first"
    );
}

/// The other half of fix round 1's I1: a viewer whose standing has ended (so dispatch
/// answers `NoAccess`) is closed without first reading what was already buffered, exactly
/// as `revoking_a_role_discards_whatever_was_already_buffered` proves for `AccessRevoked`.
#[tokio::test]
async fn a_stream_whose_standing_ends_discards_whatever_was_already_buffered() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let (hub, now) = hub(&db).await;
    // member_out's role ends on 1 September 2027; the registrant's admin role on 1 October.
    // Subscribed in this order because dispatch visits a tenant's subscribers in
    // subscription order: the member is decided before the bystander is sent anything.
    let mut member = hub.subscribe(w.viewer("member_out")).await.unwrap();
    let mut bystander = hub.subscribe(w.viewer("admin_in")).await.unwrap();

    // Two changes the member may still read, left unread in its channel while the
    // bystander reads along.
    for _ in 0..2 {
        announce(
            &pool,
            w.fau.tenant_id,
            Change::Changed(Resource::Group(w.open)),
        )
        .await;
        assert_eq!(next(&mut bystander).await, Some(Resource::Group(w.open)));
    }

    *now.lock().unwrap() = "2027-09-01T10:00:00Z".parse().unwrap();
    announce(
        &pool,
        w.fau.tenant_id,
        Change::Changed(Resource::Group(w.open)),
    )
    .await;
    // The bystander hearing it means the dispatch has already found the member without
    // standing and closed it.
    assert_eq!(next(&mut bystander).await, Some(Resource::Group(w.open)));

    assert_eq!(
        next(&mut member).await,
        None,
        "lost standing discards whatever was already buffered, rather than delivering it first"
    );
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

/// Ruling R2, Minor 1: falling more than `SUBSCRIPTION_BUFFER` deliveries behind closes the
/// stream. Unlike a revocation (`revoking_a_role_discards_whatever_was_already_buffered`),
/// this is not hiding anything the viewer was not entitled to: it is exactly
/// `SUBSCRIPTION_BUFFER` Some, then None, because falling behind only ever drops the
/// sender (see `HubInner::retain`'s doc comment) rather than signalling the subscription's
/// `CloseSignal` -- so whatever tokio's mpsc channel already buffered still drains.
#[tokio::test]
async fn a_subscription_more_than_the_buffer_behind_is_closed() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let (hub, _) = hub(&db).await;
    // Never read until after every change is announced: it falls behind.
    let mut slow = hub.subscribe(w.viewer("member_out")).await.unwrap();
    let mut bystander = hub.subscribe(w.viewer("admin_out")).await.unwrap();
    assert_eq!(hub.subscriber_count(), 2);

    let total = SUBSCRIPTION_BUFFER + 1;
    for _ in 0..total {
        announce(
            &pool,
            w.fau.tenant_id,
            Change::Changed(Resource::Group(w.open)),
        )
        .await;
    }
    for n in 0..total {
        assert_eq!(
            next(&mut bystander).await,
            Some(Resource::Group(w.open)),
            "the bystander keeps reading and sees every change ({n})"
        );
    }

    // By the time the bystander has seen all `total`, the single-threaded dispatch loop
    // has already processed the slow subscriber for every one of those same
    // notifications too (both are handled inside the same `dispatch` call, in order), so
    // its removal -- on the buffer's 65th notification -- has already happened.
    for n in 0..SUBSCRIPTION_BUFFER {
        assert_eq!(
            next(&mut slow).await,
            Some(Resource::Group(w.open)),
            "buffered items still drain once a lagging stream is closed ({n})"
        );
    }
    assert_eq!(
        next(&mut slow).await,
        None,
        "more than SUBSCRIPTION_BUFFER deliveries behind closes the stream"
    );
    assert_eq!(
        hub.subscriber_count(),
        1,
        "the lagging subscription is gone"
    );
}

/// I3: a listener failure must not reconnect silently. When the LISTEN connection itself
/// dies, every existing stream must close -- whether the fix's `Err` retry loop or sqlx's
/// own eager reconnect handles the recovery -- and the hub must keep working afterward.
///
/// The hub gets its own pool tagged with a unique `application_name` (a query parameter on
/// the connection URL, the minimal way to identify it in `pg_stat_activity` without
/// touching production code), read back immediately after `Hub::start` -- before anything
/// else uses that pool -- so exactly one row can match.
#[tokio::test]
async fn a_killed_listener_connection_closes_every_stream_and_the_hub_keeps_working() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;

    let app_name = format!("fau_hub_test_{}", db.name);
    let hub_url = format!("{}?application_name={app_name}", db.url());
    let hub_pool = PgPoolOptions::new()
        .max_connections(3)
        .connect(&hub_url)
        .await
        .expect("connect the hub's dedicated pool");
    let now = Arc::new(Mutex::new(T0.parse::<Timestamp>().unwrap()));
    let read = now.clone();
    let clock: HubClock = Arc::new(move || *read.lock().unwrap());
    let hub = Hub::start(hub_pool, clock).await.expect("start the hub");

    let admin = db.admin_pool();
    let pid: i32 =
        sqlx::query_scalar("select pid from pg_stat_activity where application_name = $1")
            .bind(&app_name)
            .fetch_one(&admin)
            .await
            .expect("find the hub's listener backend by its application_name");

    let mut member = hub.subscribe(w.viewer("member_out")).await.unwrap();

    sqlx::query("select pg_terminate_backend($1)")
        .bind(pid)
        .execute(&admin)
        .await
        .expect("terminate the listener's backend");

    assert_eq!(
        next(&mut member).await,
        None,
        "a killed listener connection closes every existing stream"
    );

    let mut fresh = hub
        .subscribe(w.viewer("member_out"))
        .await
        .expect("the hub keeps refusing and admitting correctly after reconnecting");
    announce(
        &pool,
        w.fau.tenant_id,
        Change::Changed(Resource::Group(w.open)),
    )
    .await;
    assert_eq!(
        next(&mut fresh).await,
        Some(Resource::Group(w.open)),
        "a fresh subscription after the reconnect receives new changes"
    );
}
