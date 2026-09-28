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
//!   - [`Change::AccessRevoked`] arrives for its membership;
//!   - its viewer has lost all standing (a role reached its end date), at its next delivery;
//!   - it falls more than [`SUBSCRIPTION_BUFFER`] deliveries behind;
//!   - the listener reconnects, which closes every subscription, since the gap cannot be
//!     replayed.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use fau_domain::authz::{Action, Decision, Denied};
use fau_domain::membership::access::Capability;
use fau_domain::time::Moment;
use jiff::Timestamp;
use serde_json::{json, Value};
use sqlx::postgres::PgListener;
use sqlx::{PgConnection, PgPool};
use tokio::sync::mpsc;
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

struct Subscriber {
    id: u64,
    membership_id: Uuid,
    sender: mpsc::Sender<Resource>,
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
        let mut listener = PgListener::connect_with(&pool).await?;
        listener.listen(EVENTS_CHANNEL).await?;
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
            });
        let subscription = Subscription {
            receiver,
            tenant_id: viewer.tenant_id,
            id,
            hub: Arc::downgrade(&self.inner),
        };
        let mut conn = self.inner.pool.acquire().await?;
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
}

impl Subscription {
    /// The next changed resource the viewer may read, or `None` once the stream is closed.
    pub async fn recv(&mut self) -> Option<Resource> {
        self.receiver.recv().await
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
    /// drops its sender, which closes its stream.
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

    fn close_all(&self) {
        self.subscribers().clear();
    }

    async fn dispatch(&self, tenant_id: Uuid, change: Change) {
        match change {
            Change::AccessRevoked { membership_id } => {
                self.retain(tenant_id, |s| s.membership_id != membership_id);
            }
            Change::Changed(resource) => {
                let targets: Vec<(u64, Uuid, mpsc::Sender<Resource>)> = self
                    .subscribers()
                    .get(&tenant_id)
                    .map(|subs| {
                        subs.iter()
                            .map(|s| (s.id, s.membership_id, s.sender.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                for (id, membership_id, sender) in targets {
                    let viewer = Viewer {
                        tenant_id,
                        membership_id,
                    };
                    match self.decide(viewer, resource).await {
                        Ok(Ok(())) => {
                            if sender.try_send(resource).is_err() {
                                // Full (too far behind) or already dropped.
                                self.remove(tenant_id, id);
                            }
                        }
                        Ok(Err(Denied::NoAccess)) => self.remove(tenant_id, id),
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

    async fn decide(
        &self,
        viewer: Viewer,
        resource: Resource,
    ) -> Result<Decision, MembershipError> {
        let mut conn = self.pool.acquire().await?;
        authorize(&mut conn, viewer, resource, Action::Read, self.now()).await
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
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
