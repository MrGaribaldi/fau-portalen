//! Change notifications and the per-FAU change stream (groups design §3.3 and §5.4;
//! ADR-003 decision 5a).
//!
//! - **Writing.** A mutation calls [`notify`] inside its own transaction. PostgreSQL
//!   delivers a NOTIFY only when the transaction commits, so a rolled-back change announces
//!   nothing.
//! - **Payload.** Ids and a kind code only ([`Change::payload`]): never a name, never content.
//! - **Listening.** Each process runs one [`Hub`]. It holds the process's only LISTEN
//!   connection (planning decision of 27 September 2026: one per pod, never one per viewer)
//!   and fans each notification out to that FAU's [`Subscription`]s.
//! - **Filtering.** Every delivery is authorized for its viewer through [`authorize`] at
//!   the moment of delivery, reading the database. A viewer never learns that something
//!   they cannot read has changed, so a guest beside a closed group hears nothing about it.
//! - **Closing** (Rulings R2, R3). Every closed subscription ends with
//!   [`Subscription::recv`] returning `None`, which ends the SSE body. The client
//!   reconnects and is authorized afresh. A subscription closes when:
//!   - [`Change::AccessRevoked`] arrives for its membership, or its viewer has lost all
//!     standing (a role reached its end date), at its next delivery. Both discard whatever
//!     was already buffered and unread (fix round 1, I1): a revoked viewer must not go on
//!     reading up to [`SUBSCRIPTION_BUFFER`] changes it saw while it still had access, so
//!     these two close a [`CloseSignal`] that [`Subscription::recv`] checks ahead of the
//!     channel;
//!   - it falls more than [`SUBSCRIPTION_BUFFER`] deliveries behind, or the listener
//!     reconnects (below): both merely drop the sender, so whatever is already buffered
//!     still drains before `None` -- there is nothing to hide from a viewer who could read
//!     it, only a gap in what it was told.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use fau_domain::authz::{Action, Decision, Denied};
use fau_domain::membership::access::Capability;
use fau_domain::time::Moment;
use jiff::Timestamp;
use serde_json::{json, Value};
use sqlx::postgres::PgListener;
use sqlx::{PgConnection, PgPool};
use tokio::sync::{mpsc, Notify};
use tokio::task::JoinHandle;
use uuid::Uuid;

use super::access::membership_access;
use super::authz::{authorize, Resource, Viewer};
use super::error::MembershipError;
use crate::pool::safe_error_kind;

/// The one channel every change is announced on.
pub const EVENTS_CHANNEL: &str = "fau_events";

/// How many deliveries a subscription may fall behind before it is closed.
pub const SUBSCRIPTION_BUFFER: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Something about the resource changed. Viewers who may read it are told its id, and
    /// fetch it themselves inside their session.
    Changed(Resource),
    /// A membership's access shrank: every live subscription it holds closes.
    AccessRevoked { membership_id: Uuid },
}

impl Change {
    /// The NOTIFY payload: the tenant, a kind code and ids.
    pub fn payload(self, tenant_id: Uuid) -> String {
        let v = match self {
            Change::Changed(Resource::Fau) => json!({ "t": tenant_id, "k": "fau" }),
            Change::Changed(Resource::Group(id)) => {
                json!({ "t": tenant_id, "k": "group", "id": id })
            }
            Change::Changed(Resource::GroupContent(id)) => {
                json!({ "t": tenant_id, "k": "group_content", "id": id })
            }
            Change::AccessRevoked { membership_id } => {
                json!({ "t": tenant_id, "k": "access", "m": membership_id })
            }
        };
        v.to_string()
    }

    /// The inverse of [`Change::payload`]. `None` for anything else.
    pub fn parse(payload: &str) -> Option<(Uuid, Change)> {
        let v: Value = serde_json::from_str(payload).ok()?;
        let uuid = |key: &str| -> Option<Uuid> { v.get(key)?.as_str()?.parse().ok() };
        let tenant = uuid("t")?;
        let change = match v.get("k")?.as_str()? {
            "fau" => Change::Changed(Resource::Fau),
            "group" => Change::Changed(Resource::Group(uuid("id")?)),
            "group_content" => Change::Changed(Resource::GroupContent(uuid("id")?)),
            "access" => Change::AccessRevoked {
                membership_id: uuid("m")?,
            },
            _ => return None,
        };
        Some((tenant, change))
    }
}

/// Announces `change` when the caller's transaction commits.
pub(crate) async fn notify(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    change: Change,
) -> Result<(), MembershipError> {
    sqlx::query("select pg_notify($1, $2)")
        .bind(EVENTS_CHANNEL)
        .bind(change.payload(tenant_id))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// The hub's notion of now. Production passes `Arc::new(jiff::Timestamp::now)`; tests pass
/// a clock they can move.
pub type HubClock = Arc<dyn Fn() -> Timestamp + Send + Sync>;

/// Tells a [`Subscription`] to stop, discarding whatever is already buffered in its
/// channel (fix round 1, I1). Dropping the `Sender` alone is not enough: tokio's
/// `Receiver::recv` still drains everything already queued before it returns `None`, and a
/// revoked viewer must not go on reading changes it saw while it still had access.
///
/// `close()` sets the flag before notifying. [`Subscription::recv`] first creates its
/// `Notified` future, then checks the flag, then races that future against
/// `receiver.recv()`, `biased` so a pending close always wins over a buffered item. The
/// order is what makes it race-free (see `tokio::sync::Notify`'s own documentation):
/// `notify_waiters` only wakes futures that already exist, so a future created after the
/// flag check could miss a `close()` landing in between; created before it, any `close()`
/// is either seen by the check or wakes the future.
struct CloseSignal {
    closed: AtomicBool,
    notify: Notify,
}

impl CloseSignal {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            closed: AtomicBool::new(false),
            notify: Notify::new(),
        })
    }

    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

struct Subscriber {
    id: u64,
    membership_id: Uuid,
    sender: mpsc::Sender<Resource>,
    close: Arc<CloseSignal>,
}

struct HubInner {
    pool: PgPool,
    clock: HubClock,
    subscribers: Mutex<HashMap<Uuid, Vec<Subscriber>>>,
    next_id: AtomicU64,
    listener: Mutex<Option<JoinHandle<()>>>,
}

/// One per process: the LISTEN connection and every live subscription.
#[derive(Clone)]
pub struct Hub {
    inner: Arc<HubInner>,
}

impl Hub {
    /// Connects the LISTEN connection (taken from `pool` for the hub's lifetime) and starts
    /// the listener task, which ends when the last clone of the hub is dropped.
    pub async fn start(pool: PgPool, clock: HubClock) -> Result<Self, MembershipError> {
        let listener = connect_listener(&pool).await?;
        let inner = Arc::new(HubInner {
            pool,
            clock,
            subscribers: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(0),
            listener: Mutex::new(None),
        });
        let task = tokio::spawn(listen(listener, Arc::downgrade(&inner)));
        *inner.listener.lock().expect("hub listener lock") = Some(task);
        Ok(Self { inner })
    }

    /// Opens a stream for `viewer`. Refused with `NotAuthorized` unless the viewer has
    /// standing in the FAU today; a guest has standing.
    pub async fn subscribe(&self, viewer: Viewer) -> Result<Subscription, MembershipError> {
        let (sender, receiver) = mpsc::channel(SUBSCRIPTION_BUFFER);
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let close = CloseSignal::new();
        // Registered before the standing check. A revocation that commits after the check
        // is announced after registration and closes this subscription; one that committed
        // before it is seen by the check.
        self.inner
            .subscribers()
            .entry(viewer.tenant_id)
            .or_default()
            .push(Subscriber {
                id,
                membership_id: viewer.membership_id,
                sender,
                close: close.clone(),
            });
        let subscription = Subscription {
            receiver,
            tenant_id: viewer.tenant_id,
            id,
            hub: Arc::downgrade(&self.inner),
            close,
        };
        let mut conn = self.inner.pool.acquire().await?;
        // Must stay equivalent to what `authorize` itself calls `Denied::NoAccess` (fix
        // round 1, minor 3): both read `evaluate_access` through `membership_access`, so a
        // viewer this admits but dispatch's `authorize` would refuse as `NoAccess` -- or
        // the reverse -- would be a bug, not a design choice.
        let access = membership_access(
            &mut conn,
            viewer.tenant_id,
            viewer.membership_id,
            self.inner.now().today(),
        )
        .await?;
        if access.capability == Capability::None {
            // Dropping `subscription` unregisters it.
            return Err(MembershipError::NotAuthorized);
        }
        Ok(subscription)
    }

    /// Live subscriptions across every FAU, for tests and metrics.
    pub fn subscriber_count(&self) -> usize {
        self.inner.subscribers().values().map(Vec::len).sum()
    }
}

/// One viewer's stream. Dropping it unregisters it.
pub struct Subscription {
    receiver: mpsc::Receiver<Resource>,
    tenant_id: Uuid,
    id: u64,
    hub: Weak<HubInner>,
    close: Arc<CloseSignal>,
}

impl Subscription {
    /// The next changed resource the viewer may read, or `None` once the stream is closed.
    ///
    /// The `Notified` future is created before the flag check, and the check happens before
    /// the `select!`, precisely so a `close()` landing at any point from here on is either
    /// seen by the flag or wakes the future -- never missed (see [`CloseSignal`]). `biased`
    /// makes a pending close win even when the channel already has something buffered, so
    /// an `AccessRevoked` or an ended role discards it rather than delivering it first.
    pub async fn recv(&mut self) -> Option<Resource> {
        let notified = self.close.notify.notified();
        if self.close.is_closed() {
            return None;
        }
        tokio::select! {
            biased;
            _ = notified => None,
            v = self.receiver.recv() => v,
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(hub) = self.hub.upgrade() {
            hub.remove(self.tenant_id, self.id);
        }
    }
}

impl fmt::Debug for Subscription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Subscription")
            .field("tenant_id", &self.tenant_id)
            .field("id", &self.id)
            .finish()
    }
}

impl Drop for HubInner {
    fn drop(&mut self) {
        if let Ok(slot) = self.listener.get_mut() {
            if let Some(task) = slot.take() {
                task.abort();
            }
        }
    }
}

impl HubInner {
    fn subscribers(&self) -> MutexGuard<'_, HashMap<Uuid, Vec<Subscriber>>> {
        self.subscribers.lock().expect("hub subscribers lock")
    }

    fn now(&self) -> Moment {
        Moment::at((self.clock)())
    }

    /// Keeps only the tenant's subscribers for which `keep` holds. Dropping a subscriber
    /// drops its sender: whatever it already buffered still drains before its stream ends
    /// (lag and reconnect closes: nothing here was hidden from a viewer who could read it).
    fn retain(&self, tenant_id: Uuid, keep: impl Fn(&Subscriber) -> bool) {
        let mut map = self.subscribers();
        let empty = match map.get_mut(&tenant_id) {
            Some(subs) => {
                subs.retain(&keep);
                subs.is_empty()
            }
            None => false,
        };
        if empty {
            map.remove(&tenant_id);
        }
    }

    fn remove(&self, tenant_id: Uuid, id: u64) {
        self.retain(tenant_id, |s| s.id != id);
    }

    /// As [`HubInner::retain`], but for a removal that must also discard whatever is
    /// already buffered (fix round 1, I1): [`Change::AccessRevoked`] and a viewer who has
    /// lost all standing. Signals every matching subscriber's [`CloseSignal`] before
    /// dropping it.
    fn close_and_remove(&self, tenant_id: Uuid, matches: impl Fn(&Subscriber) -> bool) {
        let mut map = self.subscribers();
        let empty = match map.get_mut(&tenant_id) {
            Some(subs) => {
                for s in subs.iter().filter(|s| matches(s)) {
                    s.close.close();
                }
                subs.retain(|s| !matches(s));
                subs.is_empty()
            }
            None => false,
        };
        if empty {
            map.remove(&tenant_id);
        }
    }

    fn close_all(&self) {
        self.subscribers().clear();
    }

    async fn dispatch(&self, tenant_id: Uuid, change: Change) {
        match change {
            Change::AccessRevoked { membership_id } => {
                // A stale `AccessRevoked` can close a newer stream of the same membership
                // (fix round 1, minor 2) if the membership was revoked and later re-added
                // -- `ensure_membership` reopens the same row -- and this notification is
                // only now being dispatched (e.g. after a listener reconnect re-plays
                // nothing, but ordinary delivery lag still could). Harmless: the client
                // just reconnects and is authorized afresh, the same as any other close.
                self.close_and_remove(tenant_id, |s| s.membership_id == membership_id);
            }
            Change::Changed(resource) => {
                let targets: Vec<(u64, Uuid, mpsc::Sender<Resource>, Arc<CloseSignal>)> = self
                    .subscribers()
                    .get(&tenant_id)
                    .map(|subs| {
                        subs.iter()
                            .map(|s| (s.id, s.membership_id, s.sender.clone(), s.close.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                if targets.is_empty() {
                    return;
                }
                // One connection and one `authorize` per distinct membership for the whole
                // dispatch (fix round 1, I2's cheap part): every subscription of the same
                // membership shares the same answer, and there is no reason to acquire a
                // fresh connection per subscriber. Concurrent dispatch across a tenant's
                // subscribers is deferred to the load test, #3508.
                let mut conn = match self.pool.acquire().await {
                    Ok(conn) => conn,
                    Err(e) => {
                        tracing::warn!(error = %e, "could not acquire a connection to authorize a change; closing every stream it would have reached");
                        for (id, _, _, _) in &targets {
                            self.remove(tenant_id, *id);
                        }
                        return;
                    }
                };
                let now = self.now();
                let mut decisions: HashMap<Uuid, Result<Decision, MembershipError>> =
                    HashMap::new();
                for (id, membership_id, sender, close) in targets {
                    if let Entry::Vacant(e) = decisions.entry(membership_id) {
                        let viewer = Viewer {
                            tenant_id,
                            membership_id,
                        };
                        let decision =
                            authorize(&mut conn, viewer, resource, Action::Read, now).await;
                        e.insert(decision);
                    }
                    match decisions
                        .get(&membership_id)
                        .expect("just computed or already present")
                    {
                        Ok(Ok(())) => {
                            if sender.try_send(resource).is_err() {
                                // Full (too far behind) or already dropped.
                                self.remove(tenant_id, id);
                            }
                        }
                        Ok(Err(Denied::NoAccess)) => {
                            close.close();
                            self.remove(tenant_id, id);
                        }
                        Ok(Err(Denied::Hidden | Denied::Forbidden)) => {}
                        Err(e) => {
                            tracing::warn!(error = %e, "could not authorize a change for a stream; closing it");
                            self.remove(tenant_id, id);
                        }
                    }
                }
            }
        }
    }
}

/// The hub's LISTEN connection: a fresh connection from `pool`, listening on
/// [`EVENTS_CHANNEL`] only.
async fn connect_listener(pool: &PgPool) -> Result<PgListener, sqlx::Error> {
    let mut listener = PgListener::connect_with(pool).await?;
    listener.listen(EVENTS_CHANNEL).await?;
    Ok(listener)
}

/// Replaces a listener whose `try_recv` failed with one built afresh by
/// [`connect_listener`], retrying every second until that succeeds. `None` once the hub is
/// gone.
///
/// Never re-listens on `failed` (final review I1). sqlx 0.8.6's `try_recv` drops a dead
/// connection itself only for `ConnectionAborted`, `UnexpectedEof`, `TimedOut` and
/// `BrokenPipe`; any other error (a `ConnectionReset`, a protocol error) comes back as
/// `Err` with the dead connection still held, and `PgListener::listen` reconnects only when
/// it holds none -- so retrying `listen` on the same listener would reuse the dead socket
/// forever, and the hub would go deaf. Rebuilding also avoids `listen` pushing the channel
/// onto the listener's list a second time on every recovery.
///
/// Holds only the `Weak` between attempts, and a pool handle only during one, so a hub
/// dropped meanwhile ends this at the next attempt; every await point is safe to cancel
/// (the hub aborts its listener task when dropped).
async fn rebuild_listener(failed: PgListener, weak: &Weak<HubInner>) -> Option<PgListener> {
    drop(failed);
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let pool = weak.upgrade()?.pool.clone();
        match connect_listener(&pool).await {
            Ok(listener) => return Some(listener),
            Err(e) => tracing::warn!(
                kind = %safe_error_kind(&e),
                "change listener could not reconnect; retrying"
            ),
        }
    }
}

async fn listen(mut listener: PgListener, weak: Weak<HubInner>) {
    loop {
        let next = listener.try_recv().await;
        let Some(hub) = weak.upgrade() else {
            return;
        };
        match next {
            Ok(Some(notification)) => match Change::parse(notification.payload()) {
                Some((tenant_id, change)) => hub.dispatch(tenant_id, change).await,
                None => tracing::warn!("ignored a malformed change notification"),
            },
            Ok(None) => {
                // The connection dropped and was re-established; anything sent meanwhile
                // is lost, so every stream closes and its client refetches.
                tracing::warn!("change listener reconnected; closing every stream");
                hub.close_all();
            }
            Err(e) => {
                tracing::warn!(kind = %safe_error_kind(&e), "change listener failed; closing every stream");
                hub.close_all();
                drop(hub);
                // Either sqlx's own eager reconnect already failed, or sqlx kept a dead
                // connection (see `rebuild_listener`). Left alone, the *next* `try_recv()`
                // would at best reconnect and re-listen silently inside itself, with no
                // return to this loop to close streams again (fix round 1, I3) -- so a
                // viewer that subscribed during the outage, or an `AccessRevoked` sent
                // while nobody here was listening, would never be told -- and at worst
                // never recover. Instead: drop this listener, build a fresh one on the same
                // backoff, and only once it listens, close every stream again -- whether
                // it predates the failure or was opened during it, none of them can be
                // trusted to have seen everything since the connection dropped.
                let Some(rebuilt) = rebuild_listener(listener, &weak).await else {
                    return;
                };
                listener = rebuilt;
                let Some(hub) = weak.upgrade() else {
                    return;
                };
                tracing::warn!(
                    "change listener reconnected after a failure; closing every stream again"
                );
                hub.close_all();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;

    /// A hub's inner state on the shared test database, without a listener task: enough
    /// to drive [`rebuild_listener`] directly.
    async fn inner() -> Arc<HubInner> {
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL is set");
        let pool = PgPoolOptions::new()
            .max_connections(3)
            .connect(&url)
            .await
            .expect("connect to the test database");
        Arc::new(HubInner {
            pool,
            clock: Arc::new(Timestamp::now),
            subscribers: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(0),
            listener: Mutex::new(None),
        })
    }

    /// Final review I1: sqlx drops a dead LISTEN connection itself only for four I/O error
    /// kinds; for any other error `try_recv` returns `Err` and keeps the dead connection,
    /// and `PgListener::listen` reconnects only when it holds none -- so re-listening on the
    /// same listener would reuse the dead socket forever. A listener whose backend was
    /// terminated, and which nobody has polled since, is in exactly that state: sqlx still
    /// holds the dead connection. The recovery must hand back a listener that hears new
    /// notifications.
    #[tokio::test]
    async fn a_failed_listener_is_rebuilt_on_a_fresh_connection() {
        let inner = inner().await;
        let pool = inner.pool.clone();
        let mut failed = connect_listener(&pool).await.expect("connect the listener");
        let pid: i32 = sqlx::query_scalar("select pg_backend_pid()")
            .fetch_one(&mut failed)
            .await
            .expect("the listener's backend pid");
        let terminated: bool = sqlx::query_scalar("select pg_terminate_backend($1, 5000)")
            .bind(pid)
            .fetch_one(&pool)
            .await
            .expect("terminate the listener's backend");
        assert!(terminated, "the listener's backend is gone");

        let mut rebuilt = tokio::time::timeout(
            Duration::from_secs(10),
            rebuild_listener(failed, &Arc::downgrade(&inner)),
        )
        .await
        .expect("a rebuilt listener within ten seconds")
        .expect("the hub is still alive");

        let tenant = Uuid::now_v7();
        let mut conn = pool.acquire().await.unwrap();
        notify(&mut conn, tenant, Change::Changed(Resource::Fau))
            .await
            .unwrap();
        let heard = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let n = rebuilt.recv().await.expect("the rebuilt listener works");
                if let Some((t, change)) = Change::parse(n.payload()) {
                    if t == tenant {
                        return change;
                    }
                }
            }
        })
        .await
        .expect("the rebuilt listener hears a new notification within five seconds");
        assert_eq!(heard, Change::Changed(Resource::Fau));
    }

    /// The retry loop holds only a `Weak`: once the hub is gone it gives up rather than
    /// reconnecting for nobody.
    #[tokio::test]
    async fn rebuilding_stops_once_the_hub_is_gone() {
        let inner = inner().await;
        let failed = connect_listener(&inner.pool).await.unwrap();
        let weak = Arc::downgrade(&inner);
        drop(inner);
        let rebuilt = tokio::time::timeout(Duration::from_secs(5), rebuild_listener(failed, &weak))
            .await
            .expect("gives up within five seconds");
        assert!(rebuilt.is_none());
    }

    #[test]
    fn a_payload_carries_ids_and_a_kind_only() {
        let (t, g, m) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let group: Value =
            serde_json::from_str(&Change::Changed(Resource::Group(g)).payload(t)).unwrap();
        assert_eq!(group, json!({ "t": t, "k": "group", "id": g }));
        let access: Value =
            serde_json::from_str(&Change::AccessRevoked { membership_id: m }.payload(t)).unwrap();
        assert_eq!(access, json!({ "t": t, "k": "access", "m": m }));
    }

    #[test]
    fn every_change_round_trips() {
        let (t, g, m) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        for c in [
            Change::Changed(Resource::Fau),
            Change::Changed(Resource::Group(g)),
            Change::Changed(Resource::GroupContent(g)),
            Change::AccessRevoked { membership_id: m },
        ] {
            assert_eq!(Change::parse(&c.payload(t)), Some((t, c)));
        }
    }

    #[test]
    fn a_malformed_payload_is_ignored() {
        let unknown_kind = format!(r#"{{"t":"{}","k":"poll"}}"#, Uuid::now_v7());
        for bad in [
            "",
            "{}",
            "not json",
            r#"{"t":"x","k":"group","id":"y"}"#,
            unknown_kind.as_str(),
        ] {
            assert_eq!(Change::parse(bad), None, "{bad}");
        }
    }
}
