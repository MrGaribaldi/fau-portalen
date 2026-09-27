//! Session-held data keys (ADR-003 decision 5a; docs/key-service-design.md §3.2). Memory
//! only. `DataKey` zeroises on drop, so removing an entry is releasing it. Only real user
//! requests count as activity.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use fau_crypto::{DataKey, Unit, WrappedKey};
use jiff::{SignedDuration, Timestamp};
use uuid::Uuid;

use crate::client::{KeyError, Keys};

pub type CacheClock = Arc<dyn Fn() -> Timestamp + Send + Sync>;

struct Entry {
    key: DataKey,
    last_activity: Timestamp,
}

pub struct KeyCache {
    keys: Arc<Keys>,
    clock: CacheClock,
    idle: SignedDuration,
    entries: Mutex<HashMap<(Uuid, Unit), Entry>>,
}

impl fmt::Debug for KeyCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyCache")
            .field("entries", &self.len())
            .field("idle", &self.idle)
            .finish()
    }
}

impl KeyCache {
    pub fn new(keys: Arc<Keys>, clock: CacheClock, idle: SignedDuration) -> Self {
        Self {
            keys,
            clock,
            idle,
            entries: Mutex::new(HashMap::new()),
        }
    }

    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<(Uuid, Unit), Entry>> {
        self.entries.lock().expect("key cache lock")
    }

    pub fn len(&self) -> usize {
        self.map().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub async fn get(
        &self,
        session: Uuid,
        unit: &Unit,
        wrapped: &WrappedKey,
    ) -> Result<DataKey, KeyError> {
        let now = (self.clock)();
        if let Some(e) = self.map().get_mut(&(session, *unit)) {
            e.last_activity = now;
            return Ok(e.key.clone());
        }
        let key = self.keys.unwrap(unit, wrapped).await?;
        self.put(session, *unit, key.clone());
        Ok(key)
    }

    pub fn put(&self, session: Uuid, unit: Unit, key: DataKey) {
        let now = (self.clock)();
        self.map().insert(
            (session, unit),
            Entry {
                key,
                last_activity: now,
            },
        );
    }

    pub fn touch_session(&self, session: Uuid) {
        let now = (self.clock)();
        for ((s, _), e) in self.map().iter_mut() {
            if *s == session {
                e.last_activity = now;
            }
        }
    }

    pub fn release_session(&self, session: Uuid) {
        self.map().retain(|(s, _), _| *s != session);
    }

    pub fn release_document(&self, session: Uuid, tenant: Uuid, document: Uuid) {
        let gone = Unit::Document { tenant, document };
        self.map()
            .retain(|(s, u), _| !(*s == session && *u == gone));
    }

    /// Reads `last_activity`, never writes it.
    pub fn sweep(&self) -> usize {
        let now = (self.clock)();
        let mut map = self.map();
        let before = map.len();
        map.retain(|_, e| now.duration_since(e.last_activity) < self.idle);
        before - map.len()
    }
}
